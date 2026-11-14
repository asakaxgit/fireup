//! Datastore `Filter` → compiled Firestore [`Predicate`].
//!
//! There are two data planes over one store and only one of them used to push
//! down. The Firestore path has had its predicates in SQL since the PostgreSQL
//! pushdown landed; `DatastoreApi::run_structured_query` read the whole kind —
//! or the whole ancestor collection — and filtered entities in process. Nothing
//! was being *dropped* on the way down, because a Datastore `Filter` is a
//! different type from the `Predicate` a [`Prefilter`] carries: there was
//! nothing to forward until this translation existed.
//!
//! [`Prefilter`]: crate::local::query::Prefilter
//!
//! ## The one rule
//!
//! Everything here is **sound, not exact**, the same contract
//! [`postgres_filter`](crate::local::postgres_filter) works under. The
//! predicate this module produces must keep every entity the filter matches; it
//! is free to keep entities the filter rejects, because the Datastore evaluator
//! runs afterwards and has the only vote on the answer. Every function
//! therefore has two legal answers for a filter it cannot translate faithfully:
//! translate it, or return `None` and let the row through.
//!
//! ## What had to be fixed before anything could be pushed
//!
//! The Datastore evaluator compared two `datastore.v1.Value`s with a function
//! whose fall-through arm returned `Ordering::Equal` for **every type pair it
//! did not name**. A string property compared to an integer filter value was
//! therefore "equal", so `priority = 3` matched an entity whose `priority` was
//! `"high"`, a blob, or a list, and so did `priority <= 3` and `priority >= 3`.
//! Firestore's comparator — the one a `Predicate` is evaluated with — instead
//! holds that two values of different types are not comparable at all, which is
//! also what Datastore's published value ordering says.
//!
//! That disagreement pointed the wrong way for a pushdown. Translating
//! `priority = 3` into the obvious `Predicate` would have asked the store to
//! drop rows the in-process filter was still keeping, and a pre-filter may only
//! ever keep too many. Under the old comparator the only ops safe to translate
//! were `<` and `>`, where a fall-through `Equal` fails the test anyway — which
//! would have left equality, the most common filter in Datastore, reading the
//! whole kind forever.
//!
//! So `property_matches` now compares its two values through
//! [`query::comparable`], the same partial order a `Predicate` is evaluated
//! with, after converting them to the Firestore values the store holds.
//! Entities whose property has a different type from the filter's value stop
//! matching as a result: a bug fix, and one that corrects the undocumented,
//! untested behaviour of a fall-through arm rather than anything this server
//! promised.
//!
//! ## What is deliberately *not* shared
//!
//! Only the value comparison is. Each surface keeps its own operator table and
//! its own field lookup, and [`DatastoreApi::run_structured_query`] still
//! answers with its own evaluator rather than with the predicate this module
//! produced.
//!
//! That separation is the point. Evaluating the translated predicate in process
//! would make "the translation is correct" true by construction — and also
//! untestable, because the pushed and full-scan paths would then be the same
//! filter, and would agree on a *wrong* answer exactly as readily as on a right
//! one. The first version of this work did that, and the mutation check in
//! `tests/local_datastore_pushdown_tests.rs` stayed green with `<` translated
//! as `>`. Two independent implementations compared against each other is what
//! gives that test something to find.
//!
//! [`query::comparable`]: crate::local::query
//!
//! ## What translates
//!
//! | Filter | Translated |
//! | --- | --- |
//! | `=`, `<`, `<=`, `>`, `>=` on a plain property name | yes, `OR` "the property is an array", whose elements only the evaluator can test |
//! | `HAS_ANCESTOR` | already removed upstream by `strip_ancestor_filter`; it is *where* to read, not a predicate |
//! | `AND` of the above | yes |
//! | `AND` with one untranslatable child | the translatable children are pushed; the whole filter still decides in process |
//! | `IN` with an array of candidates | yes, as the `Predicate`'s own `IN` (any candidate `=` the property), with the same array branch |
//! | `OR` whose every branch translates | yes |
//! | `OR` with one untranslatable branch | no — that branch could match any entity, so dropping it would drop them |
//! | `NOT_IN`, `!=` | no — a negation keeps nearly every entity, and the PostgreSQL pushdown declines the Firestore `!=`/`NOT_IN` anyway, so a translation could only add a way to be wrong |
//! | A property *path* (`a.b`), an escaped name, `__key__` | no |
//!
//! A property's name is a document *field* name under
//! [`entity_map`](crate::local::entity_map), and the operand crosses the same
//! [`ds_value_to_fs`] conversion the stored property did — so a `key_value`
//! compares against the reference string the key is stored as and a
//! `blob_value` against the stored bytes, rather than never matching.
//!
//! [`DatastoreApi::run_structured_query`]: crate::local::datastore::DatastoreApi
//! [`ds_value_to_fs`]: crate::local::entity_map::ds_value_to_fs

use crate::local::entity_map::ds_value_to_fs;
use crate::local::query::{Comparison, FieldPath, Path, Predicate};
use crate::protos::google::datastore::v1::composite_filter::Operator as CompositeOp;
use crate::protos::google::datastore::v1::filter::FilterType;
use crate::protos::google::datastore::v1::property_filter::Operator as PropertyOp;
use crate::protos::google::datastore::v1::value::ValueType as DsValueType;
use crate::protos::google::datastore::v1::{Filter, PropertyFilter};
use crate::protos::google::firestore::v1::value::ValueType as FsValueType;
use crate::protos::google::firestore::v1::{ArrayValue as FsArrayValue, Value as FsValue};

/// Translate the property half of a Datastore query's filter into a pre-filter.
///
/// `None` means nothing could be translated, so the read stays unfiltered. That
/// is always a legal answer — the pre-filter contract is one-sided.
///
/// `filter` is what `strip_ancestor_filter` left behind, so the `__key__
/// HAS_ANCESTOR` term is already gone: it chose which collection to read and is
/// not a predicate over properties.
pub(crate) fn translate(
    filter: Option<&Filter>,
    project_id: &str,
    database_id: &str,
) -> Option<Predicate> {
    match filter?.filter_type.as_ref() {
        Some(FilterType::PropertyFilter(property)) => {
            property_predicate(property, project_id, database_id)
        }
        Some(FilterType::CompositeFilter(composite)) => {
            match CompositeOp::try_from(composite.op).ok()? {
                CompositeOp::And => {}
                // An `OR` keeps every entity any branch keeps, so it is only
                // narrower than "read everything" when *every* branch
                // translated: a dropped branch could match any entity.
                CompositeOp::Or => {
                    let children = composite
                        .filters
                        .iter()
                        .map(|child| translate(Some(child), project_id, database_id))
                        .collect::<Option<Vec<_>>>()?;
                    return (!children.is_empty()).then_some(Predicate::Any(children));
                }
                CompositeOp::Unspecified => return None,
            }

            // Dropping an untranslatable conjunct widens the predicate, which
            // is the safe direction: an `AND` matches no more entities than any
            // one of its terms, so the terms that did translate still keep
            // every entity the whole filter could match.
            let mut children: Vec<Predicate> = composite
                .filters
                .iter()
                .filter_map(|child| translate(Some(child), project_id, database_id))
                .collect();

            match children.len() {
                0 => None,
                1 => Some(children.remove(0)),
                _ => Some(Predicate::All(children)),
            }
        }
        // An empty filter matches everything, which no pre-filter can improve on.
        None => None,
    }
}

/// One property filter as a `Predicate`, or `None` if it does not translate.
fn property_predicate(
    filter: &PropertyFilter,
    project_id: &str,
    database_id: &str,
) -> Option<Predicate> {
    let name = filter.property.as_ref().map(|r| r.name.as_str())?;
    // Declined, though the in-process evaluator serves `__key__` filters. A key
    // is *not* `Path::Name`: it compares as a Datastore key, by path element
    // with numeric ids before names and ids ordered as numbers, while the
    // document's resource string orders `10` before `9` and a name such as `5`
    // between them. Pushing it would drop entities the filter keeps.
    if name == "__key__" {
        return None;
    }
    // A property path names a value inside an entity value. Refused for the
    // same reason `compile_property_transforms` refuses one: `a.b` and the
    // property literally named `a.b` are different values, and Datastore
    // escapes both `.` and `\` in a name, so a name carrying either is
    // ambiguous here.
    if name.contains('.') || name.contains('\\') {
        return None;
    }

    let op = match PropertyOp::try_from(filter.op).ok()? {
        PropertyOp::Equal => Comparison::Equal,
        PropertyOp::LessThan => Comparison::LessThan,
        PropertyOp::LessThanOrEqual => Comparison::LessThanOrEqual,
        PropertyOp::GreaterThan => Comparison::GreaterThan,
        PropertyOp::GreaterThanOrEqual => Comparison::GreaterThanOrEqual,
        // The candidates cross `ds_value_to_fs` below as one array, element by
        // element, exactly as `=` converts its single operand. Only an array
        // is pushed; anything else was refused before the read.
        PropertyOp::In => match filter.value.as_ref()?.value_type {
            Some(DsValueType::ArrayValue(_)) => Comparison::In,
            _ => return None,
        },
        // `!=` and `NOT_IN` are served in process only: a negation keeps
        // nearly every entity, and `postgres_filter` declines to push the
        // Firestore spellings of both, so translating them would narrow
        // nothing while adding a way to be wrong. `HAS_ANCESTOR` was stripped
        // upstream.
        PropertyOp::NotIn
        | PropertyOp::NotEqual
        | PropertyOp::HasAncestor
        | PropertyOp::Unspecified => return None,
    };

    // The operand has to cross the same conversion the stored property did, or
    // a `key_value` would be compared against a reference string it cannot
    // equal. A conversion that fails (a key outside this database) is simply
    // not translated, rather than turned into an error the unfiltered path
    // would not have raised.
    let value = ds_value_to_fs(filter.value.as_ref()?, project_id, database_id).ok()?;
    let path = Path::Field(FieldPath::literal(name).ok()?);

    // The Datastore evaluator matches an array-valued property element by
    // element, and a `Predicate` compares the whole array: `tags = 'y'` would
    // drop `['x', 'y']`. So the pushed predicate also keeps every entity whose
    // property is an array — `>= []`, since the empty array is the least one —
    // and leaves the elements to the evaluator.
    Some(Predicate::Any(vec![
        Predicate::Compare {
            path: path.clone(),
            op,
            value,
        },
        Predicate::Compare {
            path,
            op: Comparison::GreaterThanOrEqual,
            value: FsValue {
                value_type: Some(FsValueType::ArrayValue(FsArrayValue::default())),
            },
        },
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::datastore::entity_matches;
    use crate::local::entity_map::{document_to_entity, entity_to_document};
    use crate::protos::google::datastore::v1::key::path_element::IdType;
    use crate::protos::google::datastore::v1::key::PathElement;
    use crate::protos::google::datastore::v1::{
        ArrayValue as DsArrayValue, CompositeFilter, Entity, Key, PropertyReference,
        Value as DsValue,
    };
    use crate::protos::google::firestore::v1::value::ValueType as FsValueType;
    use crate::protos::google::firestore::v1::{Document, Value as FsValue};
    use std::collections::HashMap;

    const PROJECT: &str = "demo";

    fn ds_value(value_type: DsValueType) -> DsValue {
        DsValue {
            value_type: Some(value_type),
            ..Default::default()
        }
    }

    fn integer(number: i64) -> DsValue {
        ds_value(DsValueType::IntegerValue(number))
    }

    fn double(number: f64) -> DsValue {
        ds_value(DsValueType::DoubleValue(number))
    }

    fn text(contents: &str) -> DsValue {
        ds_value(DsValueType::StringValue(contents.to_string()))
    }

    fn user_key(name: &str) -> DsValue {
        ds_value(DsValueType::KeyValue(Key {
            partition_id: None,
            path: vec![PathElement {
                kind: "User".to_string(),
                id_type: Some(IdType::Name(name.to_string())),
            }],
        }))
    }

    fn property(name: &str, op: PropertyOp, value: DsValue) -> Filter {
        Filter {
            filter_type: Some(FilterType::PropertyFilter(PropertyFilter {
                property: Some(PropertyReference {
                    name: name.to_string(),
                }),
                op: op as i32,
                value: Some(value),
            })),
        }
    }

    fn conjunction(filters: Vec<Filter>) -> Filter {
        Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::And as i32,
                filters,
            })),
        }
    }

    fn disjunction(filters: Vec<Filter>) -> Filter {
        Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::Or as i32,
                filters,
            })),
        }
    }

    fn array(values: Vec<DsValue>) -> DsValue {
        ds_value(DsValueType::ArrayValue(DsArrayValue { values }))
    }

    fn translated(filter: &Filter) -> Option<Predicate> {
        translate(Some(filter), PROJECT, "")
    }

    /// An entity of kind `Task`, named `id`, with the given properties.
    fn entity(id: &str, properties: Vec<(&str, DsValue)>) -> Entity {
        Entity {
            key: Some(Key {
                partition_id: None,
                path: vec![PathElement {
                    kind: "Task".to_string(),
                    id_type: Some(IdType::Name(id.to_string())),
                }],
            }),
            properties: properties
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        }
    }

    /// The document an entity is stored as, which is what a pre-filter sees.
    fn stored(entity: &Entity) -> Document {
        entity_to_document(entity, PROJECT, "").expect("the entity converts")
    }

    fn document(fields: Vec<(&str, FsValue)>) -> Document {
        Document {
            name: "projects/demo/databases/(default)/documents/Task/sample".to_string(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect::<HashMap<_, _>>(),
            create_time: None,
            update_time: None,
        }
    }

    fn fs_integer(number: i64) -> FsValue {
        FsValue {
            value_type: Some(FsValueType::IntegerValue(number)),
        }
    }

    fn fs_text(contents: &str) -> FsValue {
        FsValue {
            value_type: Some(FsValueType::StringValue(contents.to_string())),
        }
    }

    // -----------------------------------------------------------------------
    // The soundness invariant
    // -----------------------------------------------------------------------

    /// Entities spanning the type and presence cases a comparison can trip on.
    fn corpus() -> Vec<Entity> {
        vec![
            entity(
                "low",
                vec![("priority", integer(1)), ("owner", text("ada"))],
            ),
            entity(
                "mid",
                vec![("priority", integer(3)), ("owner", text("ada"))],
            ),
            entity(
                "high",
                vec![("priority", integer(9)), ("owner", text("bob"))],
            ),
            // Numerically equal to `mid` under a different JSON tag.
            entity("double", vec![("priority", double(3.0))]),
            // The types the old fall-through comparator called "equal" to every
            // number, and the ones this invariant exists to pin.
            entity("string", vec![("priority", text("urgent"))]),
            entity(
                "list",
                vec![(
                    "priority",
                    ds_value(DsValueType::ArrayValue(DsArrayValue {
                        values: vec![integer(3)],
                    })),
                )],
            ),
            // Matched element by element: `= 9` and `< 3` match it, and so
            // does `!= 1`, but `> 2 AND < 5` does not.
            entity(
                "pair",
                vec![("priority", array(vec![integer(1), integer(9)]))],
            ),
            entity("empty", vec![("priority", array(Vec::new()))]),
            entity(
                "blob",
                vec![("priority", ds_value(DsValueType::BlobValue(vec![3])))],
            ),
            entity("nan", vec![("priority", double(f64::NAN))]),
            entity(
                "null",
                vec![("priority", ds_value(DsValueType::NullValue(0)))],
            ),
            entity("keyed", vec![("assignee", user_key("ada"))]),
            // No `priority` at all.
            entity("bare", vec![("owner", text("dan"))]),
        ]
    }

    /// Every filter shape, translatable or not.
    fn filters() -> Vec<Filter> {
        let mut filters = Vec::new();
        for op in [
            PropertyOp::Equal,
            PropertyOp::LessThan,
            PropertyOp::LessThanOrEqual,
            PropertyOp::GreaterThan,
            PropertyOp::GreaterThanOrEqual,
            PropertyOp::NotEqual,
        ] {
            for value in [
                integer(3),
                double(3.0),
                text("urgent"),
                ds_value(DsValueType::NullValue(0)),
            ] {
                filters.push(property("priority", op, value));
            }
            filters.push(property("assignee", op, user_key("ada")));
            filters.push(property("missing", op, integer(3)));
            filters.push(property("profile.plan", op, text("pro")));
            filters.push(property("__key__", op, integer(3)));
        }
        for op in [PropertyOp::In, PropertyOp::NotIn] {
            for candidates in [
                vec![integer(3)],
                vec![integer(1), double(3.0), text("urgent")],
                vec![ds_value(DsValueType::NullValue(0)), integer(9)],
            ] {
                filters.push(property("priority", op, array(candidates)));
            }
            filters.push(property("assignee", op, array(vec![user_key("ada")])));
        }
        filters.push(conjunction(vec![
            property("priority", PropertyOp::GreaterThanOrEqual, integer(3)),
            property("owner", PropertyOp::Equal, text("ada")),
        ]));
        // Two range filters on one property, which one element has to satisfy
        // together.
        filters.push(conjunction(vec![
            property("priority", PropertyOp::GreaterThan, integer(2)),
            property("priority", PropertyOp::LessThan, integer(5)),
        ]));
        // An OR every branch of which translates, one with a branch that does
        // not, and an OR nested in an AND.
        filters.push(disjunction(vec![
            property(
                "priority",
                PropertyOp::In,
                array(vec![integer(1), integer(9)]),
            ),
            property("owner", PropertyOp::Equal, text("dan")),
        ]));
        filters.push(disjunction(vec![
            property("priority", PropertyOp::Equal, integer(3)),
            property("owner", PropertyOp::NotEqual, text("ada")),
        ]));
        filters.push(conjunction(vec![
            disjunction(vec![
                property("owner", PropertyOp::Equal, text("ada")),
                property("priority", PropertyOp::GreaterThan, integer(5)),
            ]),
            property("priority", PropertyOp::NotEqual, integer(1)),
        ]));
        filters.push(conjunction(vec![
            property("priority", PropertyOp::Equal, integer(3)),
            property("owner", PropertyOp::In, text("ada")),
        ]));
        filters.push(Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::Or as i32,
                filters: vec![
                    property("priority", PropertyOp::Equal, integer(3)),
                    property("owner", PropertyOp::Equal, text("bob")),
                ],
            })),
        });
        filters
    }

    /// The property a pre-filter has to have, over every filter × entity pair.
    ///
    /// A pre-filter may keep an entity the filter rejects; it must never drop
    /// one the filter matches. This is the whole contract, and it is checked
    /// against the Datastore evaluator — a separate implementation with its own
    /// operator table and its own property lookup — rather than against the
    /// predicate itself, which would make it a tautology.
    ///
    /// Cheap, database-free, and the reason the translation can be trusted
    /// before `tests/local_datastore_pushdown_tests.rs` runs it for real.
    #[test]
    fn a_translated_prefilter_never_drops_an_entity_the_filter_matches() {
        let mut dropped_by_prefilter = 0usize;

        for filter in filters() {
            let predicate = translated(&filter);
            for entity in corpus() {
                let document = stored(&entity);
                // Round-tripped so the evaluator sees the properties as a query
                // would, after the store's own conversion.
                let read_back = document_to_entity(&document, PROJECT, "")
                    .expect("the stored document converts back");
                let matched = entity_matches(&read_back, &filter, PROJECT, "");

                match predicate.as_ref() {
                    None => {
                        // Nothing translated: the read is unfiltered, so no
                        // entity can be dropped. Trivially sound.
                    }
                    Some(predicate) => {
                        let kept = predicate.admits(&document);
                        assert!(
                            !matched || kept,
                            "the pre-filter dropped an entity the filter \
                             matches, which is the one thing it may never do.\n\
                             filter: {filter:?}\nentity: {entity:?}"
                        );
                        if !kept {
                            dropped_by_prefilter += 1;
                        }
                    }
                }
            }
        }

        assert!(
            dropped_by_prefilter > 0,
            "no entity was narrowed away by any pre-filter, so this test \
             would pass against a translation that returned None for \
             everything — the corpus is not exercising the feature"
        );
    }

    // -----------------------------------------------------------------------
    // What translates, and what it means
    // -----------------------------------------------------------------------

    #[test]
    fn an_equality_on_a_property_translates() {
        let predicate = translated(&property("priority", PropertyOp::Equal, integer(3)))
            .expect("an equality translates");

        assert!(predicate.admits(&document(vec![("priority", fs_integer(3))])));
        assert!(!predicate.admits(&document(vec![("priority", fs_integer(4))])));
        assert!(
            !predicate.admits(&document(vec![("other", fs_integer(3))])),
            "an entity without the property does not match, as on both surfaces"
        );
    }

    /// The behaviour change this work carried with it, stated as a test.
    #[test]
    fn an_equality_does_not_match_a_property_of_another_type() {
        let predicate =
            translated(&property("priority", PropertyOp::Equal, integer(3))).expect("a predicate");

        assert!(
            !predicate.admits(&document(vec![("priority", fs_text("urgent"))])),
            "a string property is not equal to an integer filter value. The \
             Datastore comparator this replaced returned Ordering::Equal for \
             every type pair it did not name, so it matched — which is also \
             what made pushing down `=` unsound"
        );

        // And the evaluator agrees, which is what makes the pushdown sound
        // rather than merely narrower.
        let entity = entity("string", vec![("priority", text("urgent"))]);
        assert!(
            !entity_matches(
                &entity,
                &property("priority", PropertyOp::Equal, integer(3)),
                PROJECT,
                ""
            ),
            "the Datastore evaluator does not match it either"
        );
    }

    #[test]
    fn a_range_filter_translates() {
        for (op, matching, missing) in [
            (PropertyOp::LessThan, 2, 3),
            (PropertyOp::LessThanOrEqual, 3, 4),
            (PropertyOp::GreaterThan, 4, 3),
            (PropertyOp::GreaterThanOrEqual, 3, 2),
        ] {
            let predicate =
                translated(&property("priority", op, integer(3))).expect("a range translates");
            assert!(
                predicate.admits(&document(vec![("priority", fs_integer(matching))])),
                "{op:?} keeps {matching}"
            );
            assert!(
                !predicate.admits(&document(vec![("priority", fs_integer(missing))])),
                "{op:?} drops {missing}"
            );
        }
    }

    #[test]
    fn a_key_operand_compares_as_the_reference_string_it_is_stored_as() {
        let predicate = translated(&property("owner", PropertyOp::Equal, user_key("ada")))
            .expect("a key operand translates");

        assert!(
            predicate.admits(&document(vec![(
                "owner",
                FsValue {
                    value_type: Some(FsValueType::ReferenceValue(
                        "projects/demo/databases/(default)/documents/User/ada".to_string()
                    )),
                },
            )])),
            "the operand crossed the same conversion the stored property did; \
             comparing the unconverted key would never match"
        );
    }

    #[test]
    fn an_and_of_translatable_filters_becomes_one_conjunction() {
        let predicate = translated(&conjunction(vec![
            property("priority", PropertyOp::GreaterThanOrEqual, integer(3)),
            property("owner", PropertyOp::Equal, text("ada")),
        ]))
        .expect("a conjunction translates");
        assert!(matches!(&predicate, Predicate::All(children) if children.len() == 2));

        assert!(predicate.admits(&document(vec![
            ("priority", fs_integer(5)),
            ("owner", fs_text("ada")),
        ])));
        assert!(
            !predicate.admits(&document(vec![
                ("priority", fs_integer(5)),
                ("owner", fs_text("bob")),
            ])),
            "both terms have to hold"
        );
    }

    /// The partial case: push what translates, keep deciding the whole filter.
    #[test]
    fn an_and_with_an_untranslatable_child_still_pushes_the_rest() {
        let predicate = translated(&conjunction(vec![
            property("priority", PropertyOp::Equal, integer(3)),
            // `!=` is served in process only, so it is not translated.
            property("owner", PropertyOp::NotEqual, text("ada")),
        ]))
        .expect("the translatable conjunct is still worth pushing");

        assert!(
            predicate.admits(&document(vec![("priority", fs_integer(3))])),
            "the surviving conjunct keeps every entity the whole filter could \
             match — dropping a term only ever widens an AND"
        );
        assert!(!predicate.admits(&document(vec![("priority", fs_integer(9))])));
    }

    #[test]
    fn an_in_filter_translates_to_any_candidate() {
        let predicate = translated(&property(
            "priority",
            PropertyOp::In,
            array(vec![integer(1), integer(3)]),
        ))
        .expect("an IN of candidates translates");

        assert!(predicate.admits(&document(vec![("priority", fs_integer(3))])));
        assert!(!predicate.admits(&document(vec![("priority", fs_integer(2))])));
    }

    #[test]
    fn an_or_translates_only_when_every_branch_does() {
        let predicate = translated(&disjunction(vec![
            property("priority", PropertyOp::Equal, integer(3)),
            property("owner", PropertyOp::Equal, text("bob")),
        ]))
        .expect("an OR of translatable branches translates");
        assert!(matches!(&predicate, Predicate::Any(children) if children.len() == 2));
        assert!(predicate.admits(&document(vec![("owner", fs_text("bob"))])));
        assert!(!predicate.admits(&document(vec![("owner", fs_text("ada"))])));

        assert!(
            translated(&disjunction(vec![
                property("priority", PropertyOp::Equal, integer(3)),
                property("owner", PropertyOp::NotEqual, text("bob")),
            ]))
            .is_none(),
            "the `!=` branch could match any entity, so the OR narrows nothing"
        );
    }

    #[test]
    fn untranslatable_filters_narrow_nothing() {
        for filter in [
            property("__key__", PropertyOp::Equal, text("x")),
            property("profile.plan", PropertyOp::Equal, text("pro")),
            property("priority", PropertyOp::NotEqual, integer(3)),
            property("priority", PropertyOp::NotIn, array(vec![integer(3)])),
            property("owner", PropertyOp::In, text("ada")),
        ] {
            assert!(
                translated(&filter).is_none(),
                "{filter:?} must narrow nothing and leave the answer to the \
                 in-process evaluator"
            );
        }
    }

    #[test]
    fn an_absent_filter_narrows_nothing() {
        assert!(translate(None, PROJECT, "").is_none());
    }
}
