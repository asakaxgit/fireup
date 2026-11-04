//! `StructuredQuery` evaluation for `fireup local`.
//!
//! A query is compiled once ([`Query::compile`]) and then run against the
//! documents of one collection ([`Query::evaluate`]). Compilation is where
//! every unsupported shape is rejected, so an unsupported query fails before
//! any documents are read and the client gets one clear `UNIMPLEMENTED`
//! instead of a silently wrong result set.
//!
//! ## Why the engine is here and not in the store
//!
//! Filtering, ordering, and limiting run in the server process against
//! documents fetched by [`Store::scan_collection`](crate::local::store::Store::scan_collection),
//! rather than being pushed into SQL per backend. That is a deliberate
//! trade-off:
//!
//! - **One set of semantics.** Firestore's value ordering, its treatment of
//!   missing fields, and `NaN` handling are subtle. Expressed once in Rust,
//!   the in-memory and PostgreSQL stores cannot disagree about what a query
//!   means; expressed as SQL per backend, they would.
//! - **Cost.** A query reads the whole collection. For the local-development
//!   data volumes `fireup local` exists for that is fine, and it is recorded
//!   as a known limit in `docs/local/README.md` rather than hidden.
//!
//! Semantics come from the public Firestore query documentation and the
//! `googleapis` protos vendored under `proto/`. No Firebase emulator code or
//! behaviour was reverse-engineered.

use std::borrow::Cow;
use std::cmp::Ordering;

use tonic::Status;

use crate::protos::google::firestore::v1::structured_query::{
    composite_filter::Operator as ProtoCompositeOperator,
    field_filter::Operator as ProtoFieldOperator, filter::FilterType,
    unary_filter::Operator as ProtoUnaryOperator, unary_filter::OperandType, Direction,
    FieldReference,
};
use crate::protos::google::firestore::v1::value::ValueType;
use crate::protos::google::firestore::v1::{Document, StructuredQuery, Value};

/// Firestore's `__name__` pseudo-field, which stands for the document's own
/// resource name. SDKs order and filter on it, and it is the implicit tiebreak
/// of every query.
pub const DOCUMENT_NAME_FIELD: &str = "__name__";

fn unsupported(what: &str, why: &str) -> Status {
    Status::unimplemented(format!("fireup local does not support {what} yet ({why})"))
}

/// A field path a query can address.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Path {
    /// `__name__`: the document's resource name.
    Name,
    /// A top-level field of the document.
    Field(String),
}

impl Path {
    /// Parse a `FieldReference`.
    ///
    /// Nested (`a.b`) and quoted (`` `a&b` ``) paths are rejected rather than
    /// matched against a top-level field that happens to be spelled the same
    /// way: answering a query about `address.city` by looking at a field
    /// literally named `address.city` would return a confidently wrong result.
    fn parse(reference: &Option<FieldReference>) -> Result<Self, Status> {
        let path = reference
            .as_ref()
            .map(|reference| reference.field_path.as_str())
            .ok_or_else(|| Status::invalid_argument("a filter or order has no field"))?;

        match path {
            "" => Err(Status::invalid_argument("a field path must not be empty")),
            DOCUMENT_NAME_FIELD => Ok(Path::Name),
            path if path.contains('.') || path.contains('`') => Err(unsupported(
                &format!("the nested field path {path:?}"),
                "only top-level field names and __name__ are supported",
            )),
            path => Ok(Path::Field(path.to_string())),
        }
    }

    /// The value this path has in `document`, if any.
    fn resolve<'a>(&self, document: &'a Document) -> Option<Cow<'a, Value>> {
        match self {
            // `__name__` is always present, and compares as a reference, which
            // is how Firestore orders document names against each other.
            Path::Name => Some(Cow::Owned(Value {
                value_type: Some(ValueType::ReferenceValue(document.name.clone())),
            })),
            Path::Field(field) => document.fields.get(field).map(Cow::Borrowed),
        }
    }
}

/// A comparison a query can make against one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    In,
    NotIn,
    ArrayContains,
    ArrayContainsAny,
}

impl Comparison {
    fn parse(op: ProtoFieldOperator) -> Result<Self, Status> {
        match op {
            ProtoFieldOperator::Equal => Ok(Comparison::Equal),
            ProtoFieldOperator::NotEqual => Ok(Comparison::NotEqual),
            ProtoFieldOperator::LessThan => Ok(Comparison::LessThan),
            ProtoFieldOperator::LessThanOrEqual => Ok(Comparison::LessThanOrEqual),
            ProtoFieldOperator::GreaterThan => Ok(Comparison::GreaterThan),
            ProtoFieldOperator::GreaterThanOrEqual => Ok(Comparison::GreaterThanOrEqual),
            ProtoFieldOperator::In => Ok(Comparison::In),
            ProtoFieldOperator::NotIn => Ok(Comparison::NotIn),
            ProtoFieldOperator::ArrayContains => Ok(Comparison::ArrayContains),
            ProtoFieldOperator::ArrayContainsAny => Ok(Comparison::ArrayContainsAny),
            ProtoFieldOperator::Unspecified => Err(Status::invalid_argument(
                "a field filter has no operator set",
            )),
        }
    }

    /// Whether this operator takes a list of candidate values.
    fn takes_a_list(self) -> bool {
        matches!(
            self,
            Comparison::In | Comparison::NotIn | Comparison::ArrayContainsAny
        )
    }
}

/// A presence/`NaN` test, which takes no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    IsNull,
    IsNotNull,
    IsNan,
    IsNotNan,
}

impl Presence {
    fn parse(op: ProtoUnaryOperator) -> Result<Self, Status> {
        match op {
            ProtoUnaryOperator::IsNull => Ok(Presence::IsNull),
            ProtoUnaryOperator::IsNotNull => Ok(Presence::IsNotNull),
            ProtoUnaryOperator::IsNan => Ok(Presence::IsNan),
            ProtoUnaryOperator::IsNotNan => Ok(Presence::IsNotNan),
            ProtoUnaryOperator::Unspecified => Err(Status::invalid_argument(
                "a unary filter has no operator set",
            )),
        }
    }
}

/// A compiled filter tree.
#[derive(Debug, Clone)]
enum Predicate {
    /// Every child must match.
    All(Vec<Predicate>),
    /// At least one child must match.
    Any(Vec<Predicate>),
    Compare {
        path: Path,
        op: Comparison,
        value: Value,
    },
    Test {
        path: Path,
        op: Presence,
    },
}

impl Predicate {
    fn compile(filter: &crate::protos::google::firestore::v1::structured_query::Filter) -> Result<Self, Status> {
        let filter_type = filter
            .filter_type
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("a filter has no filter_type set"))?;

        match filter_type {
            FilterType::CompositeFilter(composite) => {
                let children = composite
                    .filters
                    .iter()
                    .map(Predicate::compile)
                    .collect::<Result<Vec<_>, Status>>()?;

                match composite.op() {
                    ProtoCompositeOperator::And => Ok(Predicate::All(children)),
                    ProtoCompositeOperator::Or => Ok(Predicate::Any(children)),
                    ProtoCompositeOperator::Unspecified => Err(Status::invalid_argument(
                        "a composite filter has no operator set",
                    )),
                }
            }
            FilterType::FieldFilter(field) => {
                let op = Comparison::parse(field.op())?;
                let value = field
                    .value
                    .clone()
                    .ok_or_else(|| Status::invalid_argument("a field filter has no value"))?;

                if op.takes_a_list() && list_values(&value).is_none() {
                    return Err(Status::invalid_argument(format!(
                        "the {op:?} operator needs an arrayValue of candidates"
                    )));
                }
                Ok(Predicate::Compare {
                    path: Path::parse(&field.field)?,
                    op,
                    value,
                })
            }
            FilterType::UnaryFilter(unary) => {
                let op = Presence::parse(unary.op())?;
                let field = match &unary.operand_type {
                    Some(OperandType::Field(reference)) => Some(reference.clone()),
                    None => None,
                };
                Ok(Predicate::Test {
                    path: Path::parse(&field)?,
                    op,
                })
            }
        }
    }

    /// Whether `document` matches.
    ///
    /// A document that does not carry the field never matches: Firestore
    /// queries are index lookups, and a document with no value for a field has
    /// no entry in that field's index. Returning it would be the one failure
    /// mode that looks like working code.
    fn matches(&self, document: &Document) -> bool {
        match self {
            Predicate::All(children) => children.iter().all(|child| child.matches(document)),
            Predicate::Any(children) => children.iter().any(|child| child.matches(document)),
            Predicate::Compare { path, op, value } => match path.resolve(document) {
                None => false,
                Some(actual) => compare_matches(*op, actual.as_ref(), value),
            },
            Predicate::Test { path, op } => match path.resolve(document) {
                None => false,
                Some(actual) => match op {
                    Presence::IsNull => is_null(actual.as_ref()),
                    Presence::IsNotNull => !is_null(actual.as_ref()),
                    Presence::IsNan => is_nan(actual.as_ref()),
                    Presence::IsNotNan => !is_nan(actual.as_ref()),
                },
            },
        }
    }
}

/// Evaluate one field comparison.
fn compare_matches(op: Comparison, actual: &Value, expected: &Value) -> bool {
    match op {
        Comparison::Equal => values_equal(actual, expected),
        Comparison::NotEqual => !values_equal(actual, expected),
        Comparison::LessThan => ordered(actual, expected, &[Ordering::Less]),
        Comparison::LessThanOrEqual => ordered(actual, expected, &[Ordering::Less, Ordering::Equal]),
        Comparison::GreaterThan => ordered(actual, expected, &[Ordering::Greater]),
        Comparison::GreaterThanOrEqual => {
            ordered(actual, expected, &[Ordering::Greater, Ordering::Equal])
        }
        Comparison::In => list_values(expected)
            .is_some_and(|candidates| candidates.iter().any(|candidate| values_equal(actual, candidate))),
        Comparison::NotIn => list_values(expected)
            .is_some_and(|candidates| !candidates.iter().any(|candidate| values_equal(actual, candidate))),
        Comparison::ArrayContains => list_values(actual)
            .is_some_and(|elements| elements.iter().any(|element| values_equal(element, expected))),
        Comparison::ArrayContainsAny => match (list_values(actual), list_values(expected)) {
            (Some(elements), Some(candidates)) => elements
                .iter()
                .any(|element| candidates.iter().any(|candidate| values_equal(element, candidate))),
            _ => false,
        },
    }
}

/// Whether `actual` compares to `expected` in one of `wanted` orderings.
///
/// An inequality against a value of a different type never matches. Firestore
/// keeps one index per (field, type), so `where('age', '>', 5)` does not return
/// a document whose `age` is the string `"old"`, even though strings sort after
/// numbers in the global value order.
fn ordered(actual: &Value, expected: &Value, wanted: &[Ordering]) -> bool {
    if type_rank(actual) != type_rank(expected) {
        return false;
    }
    // `NaN` is not ordered against anything, including itself: `IS_NAN` is the
    // only way to select it.
    if is_nan(actual) || is_nan(expected) {
        return false;
    }
    wanted.contains(&compare_values(actual, expected))
}

/// Equality as a *filter* sees it.
///
/// Distinct from `compare_values(..) == Equal` in one place: ordering has to be
/// a total order, so it treats `NaN` as equal to itself, while an equality
/// filter on `NaN` must never match.
fn values_equal(left: &Value, right: &Value) -> bool {
    if is_nan(left) || is_nan(right) {
        return false;
    }
    type_rank(left) == type_rank(right) && compare_values(left, right) == Ordering::Equal
}

fn is_null(value: &Value) -> bool {
    matches!(value.value_type, Some(ValueType::NullValue(_)))
}

fn is_nan(value: &Value) -> bool {
    matches!(value.value_type, Some(ValueType::DoubleValue(number)) if number.is_nan())
}

/// The elements of an `arrayValue`, or `None` for anything else.
fn list_values(value: &Value) -> Option<&[Value]> {
    match &value.value_type {
        Some(ValueType::ArrayValue(array)) => Some(&array.values),
        _ => None,
    }
}

/// Firestore's canonical ordering *between* value types.
///
/// From the public "sort order of Firestore values" documentation. Integers and
/// doubles share a rank because they compare numerically with each other.
fn type_rank(value: &Value) -> u8 {
    match &value.value_type {
        None => 0,
        Some(ValueType::NullValue(_)) => 0,
        Some(ValueType::BooleanValue(_)) => 1,
        Some(ValueType::IntegerValue(_)) | Some(ValueType::DoubleValue(_)) => 2,
        Some(ValueType::TimestampValue(_)) => 3,
        Some(ValueType::StringValue(_)) => 4,
        Some(ValueType::BytesValue(_)) => 5,
        Some(ValueType::ReferenceValue(_)) => 6,
        Some(ValueType::GeoPointValue(_)) => 7,
        Some(ValueType::ArrayValue(_)) => 8,
        Some(ValueType::MapValue(_)) => 9,
        // Pipeline expression values cannot appear in a stored document (the
        // store rejects them); they are ranked last so a client that sends one
        // in a filter gets a defined answer rather than a panic.
        Some(_) => 10,
    }
}

/// A number, kept in its original width so integers compare exactly.
enum Numeric {
    Integer(i64),
    Double(f64),
}

fn numeric(value: &Value) -> Option<Numeric> {
    match &value.value_type {
        Some(ValueType::IntegerValue(number)) => Some(Numeric::Integer(*number)),
        Some(ValueType::DoubleValue(number)) => Some(Numeric::Double(*number)),
        _ => None,
    }
}

/// Order two doubles, placing `NaN` before every other number.
fn compare_doubles(left: f64, right: f64) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        // Neither is NaN, so a partial comparison always succeeds.
        _ => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}

/// A total order over `Value`, matching Firestore's documented sort order.
///
/// Total on purpose: it is used to sort query results, and a comparator that
/// returns "incomparable" would make the sort order depend on the input order.
fn compare_values(left: &Value, right: &Value) -> Ordering {
    let by_type = type_rank(left).cmp(&type_rank(right));
    if by_type != Ordering::Equal {
        return by_type;
    }

    match (&left.value_type, &right.value_type) {
        (Some(ValueType::BooleanValue(a)), Some(ValueType::BooleanValue(b))) => a.cmp(b),
        (Some(ValueType::TimestampValue(a)), Some(ValueType::TimestampValue(b))) => {
            (a.seconds, a.nanos).cmp(&(b.seconds, b.nanos))
        }
        // Strings and references compare by UTF-8 bytes, which is what
        // Firestore orders them by (and what the store's scan order uses).
        (Some(ValueType::StringValue(a)), Some(ValueType::StringValue(b)))
        | (Some(ValueType::ReferenceValue(a)), Some(ValueType::ReferenceValue(b))) => {
            a.as_bytes().cmp(b.as_bytes())
        }
        (Some(ValueType::BytesValue(a)), Some(ValueType::BytesValue(b))) => a.cmp(b),
        (Some(ValueType::GeoPointValue(a)), Some(ValueType::GeoPointValue(b))) => {
            compare_doubles(a.latitude, b.latitude).then_with(|| compare_doubles(a.longitude, b.longitude))
        }
        (Some(ValueType::ArrayValue(a)), Some(ValueType::ArrayValue(b))) => {
            // Element-wise, then shorter first — ordinary lexicographic order.
            for (left, right) in a.values.iter().zip(b.values.iter()) {
                let ordering = compare_values(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            a.values.len().cmp(&b.values.len())
        }
        (Some(ValueType::MapValue(a)), Some(ValueType::MapValue(b))) => {
            // Keys in sorted order first, then the values of equal keys.
            let sorted = |map: &std::collections::HashMap<String, Value>| {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Vec<(String, Value)>>()
            };
            let (left, right) = (sorted(&a.fields), sorted(&b.fields));

            for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right.iter()) {
                let by_key = left_key.as_bytes().cmp(right_key.as_bytes());
                if by_key != Ordering::Equal {
                    return by_key;
                }
                let by_value = compare_values(left_value, right_value);
                if by_value != Ordering::Equal {
                    return by_value;
                }
            }
            left.len().cmp(&right.len())
        }
        // Same rank, so either both are numbers or both are a type with only
        // one inhabitant (null) or an unstorable pipeline value.
        _ => match (numeric(left), numeric(right)) {
            (Some(Numeric::Integer(a)), Some(Numeric::Integer(b))) => a.cmp(&b),
            (Some(Numeric::Double(a)), Some(Numeric::Double(b))) => compare_doubles(a, b),
            // Mixed int/double. Past 2^53 the conversion is lossy, which is a
            // documented limit rather than a silent one.
            (Some(Numeric::Integer(a)), Some(Numeric::Double(b))) => compare_doubles(a as f64, b),
            (Some(Numeric::Double(a)), Some(Numeric::Integer(b))) => compare_doubles(a, b as f64),
            _ => Ordering::Equal,
        },
    }
}

/// One `ORDER BY` term.
#[derive(Debug, Clone)]
struct SortKey {
    path: Path,
    descending: bool,
}

/// What a query returned.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// The matching documents, in query order, after offset, limit, and
    /// projection.
    pub documents: Vec<Document>,
    /// How many matching documents `offset` skipped. `RunQueryResponse` has a
    /// field for this, and a client that pages with an offset uses it to tell
    /// "no more results" apart from "all results were skipped".
    pub skipped: usize,
}

/// A compiled, runnable query.
#[derive(Debug, Clone)]
pub struct Query {
    /// The collection to read, as named by the single `from` selector.
    collection_id: String,
    filter: Option<Predicate>,
    /// Always non-empty and always ends with `__name__`, which is Firestore's
    /// implicit tiebreak and what makes the result order deterministic.
    order: Vec<SortKey>,
    offset: usize,
    limit: Option<usize>,
    /// `select` projection: the top-level fields to return, if restricted.
    projection: Option<Vec<String>>,
}

impl Query {
    /// Compile a `StructuredQuery`, rejecting every shape this phase does not
    /// serve.
    pub fn compile(query: &StructuredQuery) -> Result<Self, Status> {
        if query.find_nearest.is_some() {
            return Err(unsupported("vector search (find_nearest)", "out of scope"));
        }
        if query.start_at.is_some() || query.end_at.is_some() {
            return Err(unsupported(
                "query cursors (start_at/end_at)",
                "use limit and offset for now",
            ));
        }

        let selector = match query.from.as_slice() {
            [selector] => selector,
            [] => {
                return Err(Status::invalid_argument(
                    "a query must name a collection in `from`",
                ))
            }
            _ => {
                return Err(unsupported(
                    "a query over more than one collection",
                    "`from` must name exactly one collection",
                ))
            }
        };
        if selector.all_descendants {
            return Err(unsupported(
                "collection group queries (all_descendants)",
                "a query reads one collection under one parent",
            ));
        }
        if selector.collection_id.is_empty() {
            return Err(Status::invalid_argument(
                "`from` must name a collection id",
            ));
        }
        if selector.collection_id.contains('/') {
            return Err(Status::invalid_argument(
                "a collection id must be a single path segment",
            ));
        }

        let filter = query.r#where.as_ref().map(Predicate::compile).transpose()?;

        let mut order = Vec::with_capacity(query.order_by.len() + 1);
        for term in &query.order_by {
            order.push(SortKey {
                path: Path::parse(&term.field)?,
                descending: term.direction() == Direction::Descending,
            });
        }
        // Firestore appends `__name__` in the direction of the last explicit
        // order (ascending when there is none), so that every query has a
        // total order and equal documents do not come back in a random order.
        if !order.iter().any(|key| key.path == Path::Name) {
            let descending = order.last().is_some_and(|key| key.descending);
            order.push(SortKey {
                path: Path::Name,
                descending,
            });
        }

        let offset = usize::try_from(query.offset)
            .map_err(|_| Status::invalid_argument("offset must not be negative"))?;
        let limit = match query.limit {
            None => None,
            Some(limit) => Some(
                usize::try_from(limit)
                    .map_err(|_| Status::invalid_argument("limit must not be negative"))?,
            ),
        };

        let projection = match &query.select {
            None => None,
            Some(projection) => {
                let mut fields = Vec::with_capacity(projection.fields.len());
                for reference in &projection.fields {
                    match Path::parse(&Some(reference.clone()))? {
                        // A keys-only query selects `__name__`, which is the
                        // document's name and not one of its fields.
                        Path::Name => {}
                        Path::Field(field) => fields.push(field),
                    }
                }
                Some(fields)
            }
        };

        Ok(Self {
            collection_id: selector.collection_id.clone(),
            filter,
            order,
            offset,
            limit,
            projection,
        })
    }

    /// The collection this query reads, relative to the request's parent.
    pub fn collection_id(&self) -> &str {
        &self.collection_id
    }

    /// Filter, order, page, and project `documents`.
    pub fn evaluate(&self, documents: Vec<Document>) -> Results {
        let mut matched: Vec<Document> = documents
            .into_iter()
            .filter(|document| self.admits(document))
            .collect();

        matched.sort_by(|left, right| self.compare_documents(left, right));

        let skipped = self.offset.min(matched.len());
        Results {
            documents: matched
                .into_iter()
                .skip(self.offset)
                .take(self.limit.unwrap_or(usize::MAX))
                .map(|document| self.project(document))
                .collect(),
            skipped,
        }
    }

    /// Whether a document is a candidate at all: it must match the filter and
    /// carry every field the query orders by.
    ///
    /// The second half is easy to miss and visible to users: Firestore cannot
    /// order a document that has no value for the sort field, so such documents
    /// are absent from the result rather than sorted to one end.
    fn admits(&self, document: &Document) -> bool {
        if !self
            .order
            .iter()
            .all(|key| key.path.resolve(document).is_some())
        {
            return false;
        }
        match &self.filter {
            None => true,
            Some(filter) => filter.matches(document),
        }
    }

    fn compare_documents(&self, left: &Document, right: &Document) -> Ordering {
        for key in &self.order {
            // `admits` already established that both documents carry every
            // sort field, so a missing value here is impossible; treating it
            // as equal keeps the comparator total either way.
            let ordering = match (key.path.resolve(left), key.path.resolve(right)) {
                (Some(left), Some(right)) => compare_values(left.as_ref(), right.as_ref()),
                _ => Ordering::Equal,
            };
            let ordering = if key.descending {
                ordering.reverse()
            } else {
                ordering
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }

    /// Drop the fields a `select` projection did not ask for.
    fn project(&self, mut document: Document) -> Document {
        if let Some(fields) = &self.projection {
            document
                .fields
                .retain(|field, _| fields.iter().any(|wanted| wanted == field));
        }
        document
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protos::google::firestore::v1::structured_query::{
        CollectionSelector, CompositeFilter, FieldFilter, Filter, Order, Projection, UnaryFilter,
    };
    use crate::protos::google::firestore::v1::ArrayValue;

    const ROOT: &str = "projects/demo/databases/(default)/documents";

    fn string_value(text: &str) -> Value {
        Value {
            value_type: Some(ValueType::StringValue(text.to_string())),
        }
    }

    fn integer_value(number: i64) -> Value {
        Value {
            value_type: Some(ValueType::IntegerValue(number)),
        }
    }

    fn double_value(number: f64) -> Value {
        Value {
            value_type: Some(ValueType::DoubleValue(number)),
        }
    }

    fn null_value() -> Value {
        Value {
            value_type: Some(ValueType::NullValue(0)),
        }
    }

    fn array_value(values: Vec<Value>) -> Value {
        Value {
            value_type: Some(ValueType::ArrayValue(ArrayValue { values })),
        }
    }

    fn document(id: &str, fields: Vec<(&str, Value)>) -> Document {
        Document {
            name: format!("{ROOT}/users/{id}"),
            fields: fields
                .into_iter()
                .map(|(field, value)| (field.to_string(), value))
                .collect(),
            create_time: None,
            update_time: None,
        }
    }

    /// A query over `users` with whatever the caller wants to set.
    fn query(mutate: impl FnOnce(&mut StructuredQuery)) -> Query {
        let mut structured = StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "users".to_string(),
                all_descendants: false,
            }],
            ..Default::default()
        };
        mutate(&mut structured);
        Query::compile(&structured).expect("the query compiles")
    }

    fn field_filter(path: &str, op: ProtoFieldOperator, value: Value) -> Option<Filter> {
        Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(FieldReference {
                    field_path: path.to_string(),
                }),
                op: op as i32,
                value: Some(value),
            })),
        })
    }

    fn ids(results: &Results) -> Vec<String> {
        results
            .documents
            .iter()
            .map(|document| {
                document
                    .name
                    .rsplit('/')
                    .next()
                    .expect("a name has segments")
                    .to_string()
            })
            .collect()
    }

    fn people() -> Vec<Document> {
        vec![
            document("carol", vec![("age", integer_value(41)), ("city", string_value("Osaka"))]),
            document("alice", vec![("age", integer_value(31)), ("city", string_value("Tokyo"))]),
            document("bob", vec![("age", integer_value(31)), ("city", string_value("Kyoto"))]),
            document("dan", vec![("city", string_value("Tokyo"))]),
        ]
    }

    #[test]
    fn an_unfiltered_query_returns_everything_ordered_by_name() {
        let found = query(|_| {}).evaluate(people());
        assert_eq!(ids(&found), ["alice", "bob", "carol", "dan"]);
    }

    #[test]
    fn equality_selects_matching_documents() {
        let found = query(|q| {
            q.r#where = field_filter("age", ProtoFieldOperator::Equal, integer_value(31));
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["alice", "bob"]);
    }

    #[test]
    fn an_integer_and_a_double_of_the_same_value_are_equal() {
        let found = query(|q| {
            q.r#where = field_filter("age", ProtoFieldOperator::Equal, double_value(31.0));
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["alice", "bob"], "31 and 31.0 are one value");
    }

    #[test]
    fn in_matches_any_candidate() {
        let found = query(|q| {
            q.r#where = field_filter(
                "city",
                ProtoFieldOperator::In,
                array_value(vec![string_value("Kyoto"), string_value("Osaka")]),
            );
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["bob", "carol"]);
    }

    #[test]
    fn an_inequality_ignores_values_of_another_type() {
        let mut people = people();
        people.push(document("eve", vec![("age", string_value("ancient"))]));

        let found = query(|q| {
            q.r#where = field_filter("age", ProtoFieldOperator::GreaterThan, integer_value(31));
        })
        .evaluate(people);

        assert_eq!(
            ids(&found),
            ["carol"],
            "a string age must not satisfy a numeric inequality"
        );
    }

    #[test]
    fn a_document_without_the_filtered_field_never_matches() {
        for op in [
            ProtoFieldOperator::Equal,
            ProtoFieldOperator::NotEqual,
            ProtoFieldOperator::LessThan,
            ProtoFieldOperator::NotIn,
        ] {
            let value = if op == ProtoFieldOperator::NotIn {
                array_value(vec![integer_value(31)])
            } else {
                integer_value(31)
            };
            let found = query(|q| q.r#where = field_filter("age", op, value)).evaluate(people());
            assert!(
                !ids(&found).contains(&"dan".to_string()),
                "dan has no age, so {op:?} must not return it"
            );
        }
    }

    #[test]
    fn a_document_without_the_ordered_field_is_absent_from_the_results() {
        let found = query(|q| {
            q.order_by = vec![Order {
                field: Some(FieldReference {
                    field_path: "age".to_string(),
                }),
                direction: Direction::Ascending as i32,
            }];
        })
        .evaluate(people());

        assert_eq!(
            ids(&found),
            ["alice", "bob", "carol"],
            "dan has no age to order by"
        );
    }

    #[test]
    fn order_by_falls_back_to_the_document_name_for_ties() {
        let found = query(|q| {
            q.order_by = vec![Order {
                field: Some(FieldReference {
                    field_path: "age".to_string(),
                }),
                direction: Direction::Descending as i32,
            }];
        })
        .evaluate(people());

        // age descending, then name descending — the direction Firestore gives
        // the implicit `__name__` tiebreak.
        assert_eq!(ids(&found), ["carol", "bob", "alice"]);
    }

    #[test]
    fn limit_and_offset_page_through_the_ordered_results() {
        let found = query(|q| {
            q.offset = 1;
            q.limit = Some(2);
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["bob", "carol"]);
    }

    #[test]
    fn a_projection_keeps_only_the_selected_fields() {
        let found = query(|q| {
            q.select = Some(Projection {
                fields: vec![
                    FieldReference {
                        field_path: "city".to_string(),
                    },
                    FieldReference {
                        field_path: DOCUMENT_NAME_FIELD.to_string(),
                    },
                ],
            });
            q.limit = Some(1);
        })
        .evaluate(people());

        assert_eq!(found.documents.len(), 1);
        let only = &found.documents[0];
        assert!(only.fields.contains_key("city"));
        assert!(!only.fields.contains_key("age"), "age was not selected");
        assert!(
            only.name.ends_with("/alice"),
            "__name__ is the document's name, not a field"
        );
    }

    #[test]
    fn nan_is_only_selectable_with_is_nan() {
        let documents = vec![
            document("nan", vec![("score", double_value(f64::NAN))]),
            document("one", vec![("score", double_value(1.0))]),
        ];

        let by_equality = query(|q| {
            q.r#where = field_filter(
                "score",
                ProtoFieldOperator::Equal,
                double_value(f64::NAN),
            );
        })
        .evaluate(documents.clone());
        assert!(by_equality.documents.is_empty(), "== NaN never matches");

        let by_unary = query(|q| {
            q.r#where = Some(Filter {
                filter_type: Some(FilterType::UnaryFilter(UnaryFilter {
                    op: ProtoUnaryOperator::IsNan as i32,
                    operand_type: Some(OperandType::Field(FieldReference {
                        field_path: "score".to_string(),
                    })),
                })),
            });
        })
        .evaluate(documents);
        assert_eq!(ids(&by_unary), ["nan"]);
    }

    #[test]
    fn is_null_distinguishes_a_null_field_from_a_missing_one() {
        let documents = vec![
            document("null", vec![("nickname", null_value())]),
            document("missing", vec![("age", integer_value(1))]),
        ];

        let found = query(|q| {
            q.r#where = Some(Filter {
                filter_type: Some(FilterType::UnaryFilter(UnaryFilter {
                    op: ProtoUnaryOperator::IsNull as i32,
                    operand_type: Some(OperandType::Field(FieldReference {
                        field_path: "nickname".to_string(),
                    })),
                })),
            });
        })
        .evaluate(documents);

        assert_eq!(ids(&found), ["null"]);
    }

    #[test]
    fn composite_filters_combine_with_and_and_or() {
        let and = query(|q| {
            q.r#where = Some(Filter {
                filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                    op: ProtoCompositeOperator::And as i32,
                    filters: vec![
                        field_filter("age", ProtoFieldOperator::Equal, integer_value(31))
                            .expect("filter"),
                        field_filter("city", ProtoFieldOperator::Equal, string_value("Tokyo"))
                            .expect("filter"),
                    ],
                })),
            });
        })
        .evaluate(people());
        assert_eq!(ids(&and), ["alice"]);

        let or = query(|q| {
            q.r#where = Some(Filter {
                filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                    op: ProtoCompositeOperator::Or as i32,
                    filters: vec![
                        field_filter("age", ProtoFieldOperator::Equal, integer_value(41))
                            .expect("filter"),
                        field_filter("city", ProtoFieldOperator::Equal, string_value("Kyoto"))
                            .expect("filter"),
                    ],
                })),
            });
        })
        .evaluate(people());
        assert_eq!(ids(&or), ["bob", "carol"]);
    }

    #[test]
    fn array_contains_looks_inside_an_array_field() {
        let documents = vec![
            document("a", vec![("tags", array_value(vec![string_value("x"), string_value("y")]))]),
            document("b", vec![("tags", array_value(vec![string_value("z")]))]),
            document("c", vec![("tags", string_value("x"))]),
        ];

        let found = query(|q| {
            q.r#where = field_filter("tags", ProtoFieldOperator::ArrayContains, string_value("x"));
        })
        .evaluate(documents);

        assert_eq!(
            ids(&found),
            ["a"],
            "a non-array field must not satisfy array-contains"
        );
    }

    #[test]
    fn document_name_is_orderable_and_filterable() {
        let found = query(|q| {
            q.r#where = field_filter(
                DOCUMENT_NAME_FIELD,
                ProtoFieldOperator::GreaterThan,
                Value {
                    value_type: Some(ValueType::ReferenceValue(format!("{ROOT}/users/bob"))),
                },
            );
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["carol", "dan"]);
    }

    #[test]
    fn unsupported_query_shapes_are_rejected_at_compile_time() {
        let compile = |mutate: fn(&mut StructuredQuery)| {
            let mut structured = StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: "users".to_string(),
                    all_descendants: false,
                }],
                ..Default::default()
            };
            mutate(&mut structured);
            Query::compile(&structured).expect_err("should be rejected")
        };

        assert_eq!(
            compile(|q| q.from[0].all_descendants = true).code(),
            tonic::Code::Unimplemented,
            "collection group queries are not served"
        );
        assert_eq!(
            compile(|q| q.start_at = Some(Default::default())).code(),
            tonic::Code::Unimplemented,
            "cursors are not served"
        );
        assert_eq!(
            compile(|q| {
                q.order_by = vec![Order {
                    field: Some(FieldReference {
                        field_path: "address.city".to_string(),
                    }),
                    direction: Direction::Ascending as i32,
                }]
            })
            .code(),
            tonic::Code::Unimplemented,
            "nested field paths are not served"
        );
        assert_eq!(
            compile(|q| q.from.clear()).code(),
            tonic::Code::InvalidArgument,
            "a query has to name a collection"
        );
        assert_eq!(
            compile(|q| {
                q.r#where = field_filter("age", ProtoFieldOperator::In, integer_value(1))
            })
            .code(),
            tonic::Code::InvalidArgument,
            "IN needs a list of candidates"
        );
    }
}
