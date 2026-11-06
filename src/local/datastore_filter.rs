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
//! ## One comparator, not two that agree
//!
//! The obvious shape for this module is "translate the filter for the store,
//! keep the in-process evaluator as it was". That shape does not work here, and
//! the reason is worth writing down because it is what the design turns on.
//!
//! The Datastore evaluator compared two `datastore.v1.Value`s with a function
//! whose fall-through arm returned `Ordering::Equal` for **every type pair it
//! did not name**. A string property compared to an integer filter value was
//! therefore "equal", so `priority = 3` matched an entity whose `priority` was
//! `"high"`, a blob, or a list, and so did `priority <= 3` and `priority >= 3`.
//! Firestore's comparator — the one a `Predicate` is evaluated with — instead
//! requires the two values to share a type rank, which is also what Datastore's
//! published value ordering says.
//!
//! So the two comparators disagreed, and they disagreed in the direction that
//! makes a pushdown *unsound*: translating `priority = 3` into the obvious
//! `Predicate` would have asked the store to drop rows the in-process filter
//! was still keeping, and a pre-filter may only ever keep too many. The ops
//! that happen to be safe under the old comparator are just `<` and `>`, where
//! a fall-through `Equal` fails the test anyway — which would have left the
//! most common filter in Datastore, equality, reading the whole kind forever.
//!
//! The fix is not a second comparator that is careful to agree. It is to stop
//! having two: this module translates the filter, and
//! [`DatastoreApi::run_structured_query`] evaluates the *translated predicate*
//! in process, through [`Predicate::admits`], whenever the translation is
//! exact. The pushed-down filter and the in-process filter are then the same
//! object evaluated by the same code, so "the translation is exact" is true by
//! construction rather than by inspection of two functions that can drift.
//!
//! Entities whose properties have a different type from the filter's value stop
//! matching as a result. That is a bug fix — real Datastore does not return
//! them either — and it is the undocumented, untested behaviour of a
//! fall-through arm rather than anything this server promised.
//!
//! ## What translates
//!
//! | Filter | Translated |
//! | --- | --- |
//! | `=`, `<`, `<=`, `>`, `>=` on a plain property name | yes, exactly |
//! | `HAS_ANCESTOR` | already removed upstream by `strip_ancestor_filter`; it is *where* to read, not a predicate |
//! | `AND` of the above | yes, exactly |
//! | `AND` with one untranslatable child | the translatable children are pushed; the whole filter still decides in process |
//! | `OR` | no — the in-process evaluator does not serve `OR` either, and translating it would quietly start |
//! | `IN`, `NOT_IN`, `!=`, `NOT_EQUAL` | no — the in-process evaluator refuses them, and a pre-filter that implemented them would answer a query the surface rejects |
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
use crate::protos::google::datastore::v1::{Filter, PropertyFilter};

/// What a Datastore filter became.
pub(crate) struct Translated {
    /// A predicate sound to hand a store as a pre-filter: it keeps every entity
    /// the filter matches, and may keep more.
    ///
    /// `None` means nothing could be translated, so the read stays unfiltered.
    /// That is always a legal answer — the pre-filter contract is one-sided.
    pub prefilter: Option<Predicate>,
    /// Whether `prefilter` is *equivalent* to the filter rather than merely a
    /// superset of it, so the in-process pass can evaluate it and skip the
    /// Datastore evaluator entirely.
    ///
    /// False whenever any part of the filter did not translate: those parts
    /// still have to be decided by the original evaluator, which decides the
    /// whole filter rather than the leftover.
    pub exact: bool,
}

impl Translated {
    /// Nothing translated: read everything, decide everything in process.
    fn nothing() -> Self {
        Self {
            prefilter: None,
            exact: false,
        }
    }
}

/// Translate the property half of a Datastore query's filter.
///
/// `filter` is what `strip_ancestor_filter` left behind, so the `__key__
/// HAS_ANCESTOR` term is already gone — it chose which collection to read and
/// is not a predicate over properties.
pub(crate) fn translate(
    filter: Option<&Filter>,
    project_id: &str,
    database_id: &str,
) -> Translated {
    let Some(filter) = filter else {
        // An unfiltered query is exactly the empty predicate, but there is no
        // point pushing one down: say "nothing to narrow on" and let the store
        // take its unfiltered path.
        return Translated {
            prefilter: None,
            exact: true,
        };
    };

    match filter.filter_type.as_ref() {
        Some(FilterType::PropertyFilter(property)) => {
            match property_predicate(property, project_id, database_id) {
                Some(predicate) => Translated {
                    prefilter: Some(predicate),
                    exact: true,
                },
                None => Translated::nothing(),
            }
        }
        Some(FilterType::CompositeFilter(composite)) => {
            // Only `AND` is translated. `OR` is not an oversight: the
            // in-process evaluator serves `AND` alone and answers an `OR`
            // filter with "no match", so translating `OR` here would change
            // what a query *returns* under the cover of an optimisation.
            if CompositeOp::try_from(composite.op) != Ok(CompositeOp::And) {
                return Translated::nothing();
            }

            let mut children = Vec::with_capacity(composite.filters.len());
            let mut exact = true;
            for child in &composite.filters {
                let translated = translate(Some(child), project_id, database_id);
                exact &= translated.exact;
                if let Some(predicate) = translated.prefilter {
                    children.push(predicate);
                }
            }

            // Dropping an untranslatable conjunct widens the predicate, which
            // is the safe direction: an `AND` matches fewer entities than any
            // one of its terms, so the terms that did translate still narrow
            // the read correctly.
            match children.len() {
                0 => Translated {
                    prefilter: None,
                    exact,
                },
                1 => Translated {
                    prefilter: Some(children.remove(0)),
                    exact,
                },
                _ => Translated {
                    prefilter: Some(Predicate::All(children)),
                    exact,
                },
            }
        }
        // An empty filter matches everything on the Datastore side, which no
        // pre-filter can improve on.
        None => Translated {
            prefilter: None,
            exact: true,
        },
    }
}

/// One property filter as a `Predicate`, or `None` if it does not translate.
fn property_predicate(
    filter: &PropertyFilter,
    project_id: &str,
    database_id: &str,
) -> Option<Predicate> {
    let name = filter.property.as_ref().map(|r| r.name.as_str())?;
    // `__key__` filters other than the `HAS_ANCESTOR` stripped upstream are not
    // served by the in-process evaluator, so there is nothing to be equivalent
    // to. Note this is *not* `Path::Name`: a key compares as a Datastore key,
    // whose ordering is by path element, not as the document's resource string.
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
        // `IN`, `NOT_IN`, `!=` and `HAS_ANCESTOR` all reach the in-process
        // evaluator as "no match". Translating them would make a pre-filter
        // answer a query the surface does not serve.
        PropertyOp::In
        | PropertyOp::NotIn
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

    Some(Predicate::Compare {
        path: Path::Field(FieldPath::literal(name).ok()?),
        op,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protos::google::datastore::v1::value::ValueType as DsValueType;
    use crate::protos::google::datastore::v1::{
        CompositeFilter, PropertyReference, Value as DsValue,
    };
    use crate::protos::google::firestore::v1::value::ValueType as FsValueType;
    use crate::protos::google::firestore::v1::{Document, Value as FsValue};
    use std::collections::HashMap;

    fn ds_value(value_type: DsValueType) -> DsValue {
        DsValue {
            value_type: Some(value_type),
            ..Default::default()
        }
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

    fn translated(filter: &Filter) -> Translated {
        translate(Some(filter), "demo", "")
    }

    /// A document with the given top-level fields, under a plausible name.
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

    #[test]
    fn an_equality_on_a_property_translates_exactly() {
        let translated = translated(&property(
            "priority",
            PropertyOp::Equal,
            ds_value(DsValueType::IntegerValue(3)),
        ));
        assert!(translated.exact, "an equality is fully translatable");
        let predicate = translated.prefilter.expect("a predicate");

        assert!(predicate.admits(&document(vec![("priority", fs_integer(3))])));
        assert!(!predicate.admits(&document(vec![("priority", fs_integer(4))])));
        assert!(
            !predicate.admits(&document(vec![("other", fs_integer(3))])),
            "an entity without the property does not match, as on both surfaces"
        );
    }

    /// The behaviour change this module is responsible for, stated as a test.
    #[test]
    fn an_equality_does_not_match_a_property_of_another_type() {
        let predicate = translated(&property(
            "priority",
            PropertyOp::Equal,
            ds_value(DsValueType::IntegerValue(3)),
        ))
        .prefilter
        .expect("a predicate");

        assert!(
            !predicate.admits(&document(vec![("priority", fs_text("high"))])),
            "a string property is not equal to an integer filter value. The \
             Datastore comparator this replaced returned Ordering::Equal for \
             every type pair it did not name, so it matched — which is also \
             what made an exact pushdown of `=` impossible"
        );
    }

    #[test]
    fn a_range_filter_translates_exactly() {
        for (op, matching, missing) in [
            (PropertyOp::LessThan, 2, 3),
            (PropertyOp::LessThanOrEqual, 3, 4),
            (PropertyOp::GreaterThan, 4, 3),
            (PropertyOp::GreaterThanOrEqual, 3, 2),
        ] {
            let translated = translated(&property(
                "priority",
                op,
                ds_value(DsValueType::IntegerValue(3)),
            ));
            assert!(translated.exact, "{op:?} is fully translatable");
            let predicate = translated.prefilter.expect("a predicate");
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
        use crate::protos::google::datastore::v1::key::path_element::IdType;
        use crate::protos::google::datastore::v1::key::PathElement;
        use crate::protos::google::datastore::v1::Key;

        let predicate = translated(&property(
            "owner",
            PropertyOp::Equal,
            ds_value(DsValueType::KeyValue(Key {
                partition_id: None,
                path: vec![PathElement {
                    kind: "User".to_string(),
                    id_type: Some(IdType::Name("ada".to_string())),
                }],
            })),
        ))
        .prefilter
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
        let translated = translated(&conjunction(vec![
            property(
                "priority",
                PropertyOp::GreaterThanOrEqual,
                ds_value(DsValueType::IntegerValue(3)),
            ),
            property(
                "owner",
                PropertyOp::Equal,
                ds_value(DsValueType::StringValue("ada".to_string())),
            ),
        ]));
        assert!(translated.exact);
        let predicate = translated.prefilter.expect("a predicate");
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
        let translated = translated(&conjunction(vec![
            property(
                "priority",
                PropertyOp::Equal,
                ds_value(DsValueType::IntegerValue(3)),
            ),
            // `IN` is not served in process, so it cannot be translated.
            property(
                "owner",
                PropertyOp::In,
                ds_value(DsValueType::StringValue("ada".to_string())),
            ),
        ]));

        assert!(
            !translated.exact,
            "a dropped conjunct means the predicate is a superset, so the \
             Datastore evaluator still has to decide the answer"
        );
        let predicate = translated
            .prefilter
            .expect("the translatable conjunct is still worth pushing");
        assert!(
            predicate.admits(&document(vec![("priority", fs_integer(3))])),
            "the surviving conjunct keeps every entity the whole filter could \
             match — dropping a term only ever widens an AND"
        );
        assert!(!predicate.admits(&document(vec![("priority", fs_integer(9))])));
    }

    #[test]
    fn untranslatable_filters_narrow_nothing_and_stay_inexact() {
        let key_filter = property(
            "__key__",
            PropertyOp::Equal,
            ds_value(DsValueType::StringValue("x".to_string())),
        );
        let path_filter = property(
            "profile.plan",
            PropertyOp::Equal,
            ds_value(DsValueType::StringValue("pro".to_string())),
        );
        let not_equal = property(
            "priority",
            PropertyOp::NotEqual,
            ds_value(DsValueType::IntegerValue(3)),
        );
        let disjunction = Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::Or as i32,
                filters: vec![property(
                    "priority",
                    PropertyOp::Equal,
                    ds_value(DsValueType::IntegerValue(3)),
                )],
            })),
        };

        for filter in [key_filter, path_filter, not_equal, disjunction] {
            let translated = translated(&filter);
            assert!(
                translated.prefilter.is_none() && !translated.exact,
                "{filter:?} must narrow nothing and leave the answer to the \
                 in-process evaluator"
            );
        }
    }

    #[test]
    fn an_absent_filter_is_exact_and_narrows_nothing() {
        let translated = translate(None, "demo", "");
        assert!(translated.prefilter.is_none());
        assert!(
            translated.exact,
            "an unfiltered query has no predicate to disagree about, so the \
             in-process pass has nothing to run"
        );
    }
}
