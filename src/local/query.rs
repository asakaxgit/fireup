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
use std::collections::HashMap;

use tonic::Status;

use crate::protos::google::firestore::v1::structured_query::{
    composite_filter::Operator as ProtoCompositeOperator,
    field_filter::Operator as ProtoFieldOperator, filter::FilterType,
    unary_filter::Operator as ProtoUnaryOperator, unary_filter::OperandType, Direction,
    FieldReference,
};
use crate::protos::google::firestore::v1::value::ValueType;
use crate::protos::google::firestore::v1::{Cursor, Document, MapValue, StructuredQuery, Value};

/// Firestore's `__name__` pseudo-field, which stands for the document's own
/// resource name. SDKs order and filter on it, and it is the implicit tiebreak
/// of every query.
pub const DOCUMENT_NAME_FIELD: &str = "__name__";

fn unsupported(what: &str, why: &str) -> Status {
    Status::unimplemented(format!("fireup local does not support {what} yet ({why})"))
}

/// A field path, split into the segments it addresses.
///
/// `a.b` addresses the `b` entry of the map under `a`, which is a different
/// thing from a top-level field literally named `a.b`. A field whose name
/// contains a dot (or any other character the unquoted syntax does not allow)
/// is addressed as a single backtick-quoted segment: `` `a.b` ``.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPath {
    /// Always non-empty.
    segments: Vec<String>,
}

impl FieldPath {
    /// Parse Firestore's field-path syntax: dot-separated segments, each
    /// either an unquoted identifier (`[A-Za-z_][A-Za-z_0-9]*`) or a
    /// backtick-quoted literal in which `` \` `` and `\\` are escapes.
    ///
    /// Anything outside that grammar is an `INVALID_ARGUMENT` rather than a
    /// best guess: a path the server read differently from the way the client
    /// wrote it would return a confidently wrong result set.
    pub fn parse(text: &str) -> Result<Self, Status> {
        let malformed = |reason: &str| {
            Status::invalid_argument(format!("the field path {text:?} is malformed: {reason}"))
        };
        if text.is_empty() {
            return Err(Status::invalid_argument("a field path must not be empty"));
        }

        let mut segments = Vec::new();
        let mut characters = text.chars().peekable();
        loop {
            let segment = if characters.peek() == Some(&'`') {
                characters.next();
                let mut segment = String::new();
                loop {
                    match characters.next() {
                        None => return Err(malformed("a ` quote is never closed")),
                        Some('`') => break,
                        // Only the two characters that would otherwise end or
                        // escape the segment can be escaped; anything else is
                        // a client mistake worth reporting.
                        Some('\\') => match characters.next() {
                            Some(escaped @ ('`' | '\\')) => segment.push(escaped),
                            _ => return Err(malformed("a \\ escapes only ` or \\")),
                        },
                        Some(character) => segment.push(character),
                    }
                }
                if segment.is_empty() {
                    return Err(malformed("a quoted segment must not be empty"));
                }
                segment
            } else {
                let mut segment = String::new();
                while let Some(&character) = characters.peek() {
                    if character == '.' {
                        break;
                    }
                    characters.next();
                    segment.push(character);
                }
                if segment.is_empty() {
                    return Err(malformed("a segment must not be empty"));
                }
                let identifier = segment.chars().enumerate().all(|(at, character)| {
                    character == '_'
                        || character.is_ascii_alphabetic()
                        || (at > 0 && character.is_ascii_digit())
                });
                if !identifier {
                    return Err(malformed(&format!(
                        "the segment {segment:?} must be quoted as `{segment}`"
                    )));
                }
                segment
            };
            segments.push(segment);

            match characters.next() {
                None => break,
                Some('.') => continue,
                // Only reachable after a quoted segment; the unquoted branch
                // stops at a dot or the end of the path.
                Some(character) => {
                    return Err(malformed(&format!(
                        "a quoted segment must be followed by . or the end, not {character:?}"
                    )))
                }
            }
        }
        Ok(Self { segments })
    }

    /// The value this path addresses in `fields`, if any.
    ///
    /// Only maps are walked into. Firestore field paths do not index into
    /// arrays, so `tags.0` addresses the `0` key of a map named `tags` and
    /// nothing at all when `tags` is a list.
    fn resolve<'a>(&self, fields: &'a HashMap<String, Value>) -> Option<&'a Value> {
        let (first, rest) = self.segments.split_first()?;
        let mut value = fields.get(first)?;
        for segment in rest {
            match &value.value_type {
                Some(ValueType::MapValue(map)) => value = map.fields.get(segment)?,
                _ => return None,
            }
        }
        Some(value)
    }
}

/// Keep only the values `paths` select, preserving the nesting around them.
///
/// This is what both a `select()` projection and a read `DocumentMask` do, so
/// they share one implementation: selecting `a.b` returns a document whose `a`
/// is a map holding only `b`, not a flattened field called `a.b` and not the
/// whole of `a`.
pub fn retain_field_paths(fields: &mut HashMap<String, Value>, paths: &[FieldPath]) {
    let mut kept: HashMap<String, Value> = HashMap::new();
    for path in paths {
        if let Some(value) = path.resolve(fields) {
            graft(&mut kept, &path.segments, value.clone());
        }
    }
    *fields = kept;
}

/// Insert `value` at `segments`, creating the maps above it as needed.
fn graft(fields: &mut HashMap<String, Value>, segments: &[String], value: Value) {
    match segments {
        [] => {}
        [last] => {
            fields.insert(last.clone(), value);
        }
        [first, rest @ ..] => {
            let parent = fields.entry(first.clone()).or_insert_with(|| Value {
                value_type: Some(ValueType::MapValue(MapValue {
                    fields: HashMap::new(),
                })),
            });
            // A non-map here means a broader path already selected the whole
            // of this field, which covers the narrower one too.
            if let Some(ValueType::MapValue(map)) = &mut parent.value_type {
                graft(&mut map.fields, rest, value);
            }
        }
    }
}

/// A field path a query can address.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Path {
    /// `__name__`: the document's resource name.
    Name,
    /// A field of the document, possibly nested.
    Field(FieldPath),
}

impl Path {
    /// Parse a `FieldReference`.
    fn parse(reference: &Option<FieldReference>) -> Result<Self, Status> {
        let path = reference
            .as_ref()
            .map(|reference| reference.field_path.as_str())
            .ok_or_else(|| Status::invalid_argument("a filter or order has no field"))?;

        match path {
            "" => Err(Status::invalid_argument("a field path must not be empty")),
            // Unquoted only: `` `__name__` `` is an ordinary field that merely
            // shares the pseudo-field's spelling.
            DOCUMENT_NAME_FIELD => Ok(Path::Name),
            path => Ok(Path::Field(FieldPath::parse(path)?)),
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
            Path::Field(path) => path.resolve(&document.fields).map(Cow::Borrowed),
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

    /// Whether this operator is an **inequality** in the sense that gives a
    /// query an implicit order.
    ///
    /// Firestore orders an inequality-filtered query by the filtered field, so
    /// that field joins the query's order whether the client asked for it or
    /// not — which is what makes a trailing explicit `__name__` illegal beside
    /// one. Enumerated one operator at a time against
    /// `cloud-firestore-emulator` v1.22.0 rather than inferred, because the
    /// split is not the obvious one: `NOT_IN` counts, `IN` does not, and
    /// neither array operator does.
    fn is_an_inequality(self) -> bool {
        match self {
            Comparison::NotEqual
            | Comparison::LessThan
            | Comparison::LessThanOrEqual
            | Comparison::GreaterThan
            | Comparison::GreaterThanOrEqual
            | Comparison::NotIn => true,
            Comparison::Equal
            | Comparison::In
            | Comparison::ArrayContains
            | Comparison::ArrayContainsAny => false,
        }
    }
}

/// A presence/`NaN` test, which takes no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    Null,
    NotNull,
    Nan,
    NotNan,
}

impl Presence {
    fn parse(op: ProtoUnaryOperator) -> Result<Self, Status> {
        match op {
            ProtoUnaryOperator::IsNull => Ok(Presence::Null),
            ProtoUnaryOperator::IsNotNull => Ok(Presence::NotNull),
            ProtoUnaryOperator::IsNan => Ok(Presence::Nan),
            ProtoUnaryOperator::IsNotNan => Ok(Presence::NotNan),
            ProtoUnaryOperator::Unspecified => Err(Status::invalid_argument(
                "a unary filter has no operator set",
            )),
        }
    }

    /// See [`Comparison::is_an_inequality`].
    ///
    /// `IS_NOT_NULL` and `IS_NOT_NAN` are how `!= null` and `!= NaN` reach the
    /// wire, so they inherit `!=`'s implicit order; `IS_NULL` and `IS_NAN` come
    /// from `==` and do not. Measured, like the rest.
    fn is_an_inequality(self) -> bool {
        match self {
            Presence::NotNull | Presence::NotNan => true,
            Presence::Null | Presence::Nan => false,
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
                let field = unary
                    .operand_type
                    .as_ref()
                    .map(|OperandType::Field(reference)| reference.clone());
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
                    Presence::Null => is_null(actual.as_ref()),
                    Presence::NotNull => !is_null(actual.as_ref()),
                    Presence::Nan => is_nan(actual.as_ref()),
                    Presence::NotNan => !is_nan(actual.as_ref()),
                },
            },
        }
    }

    /// Collect every path filtered with an inequality, in first-seen order.
    ///
    /// Both composite operators contribute: an `OR` branch's inequality counts
    /// towards the query's implicit order exactly as an `AND`'s does, which is
    /// measurable and was measured. Duplicates are not removed — callers only
    /// ask whether any of these is missing from the explicit order.
    fn inequality_paths(&self, found: &mut Vec<Path>) {
        match self {
            Predicate::All(children) | Predicate::Any(children) => {
                for child in children {
                    child.inequality_paths(found);
                }
            }
            Predicate::Compare { path, op, .. } if op.is_an_inequality() => {
                found.push(path.clone())
            }
            Predicate::Test { path, op } if op.is_an_inequality() => found.push(path.clone()),
            Predicate::Compare { .. } | Predicate::Test { .. } => {}
        }
    }

    /// Whether anything here filters on a document **field**.
    ///
    /// `__name__` is the document's name rather than one of its fields, so a
    /// predicate on it does not count. The distinction decides whether a
    /// descending key scan has an index behind it; see `Query::compile`.
    fn filters_a_field(&self) -> bool {
        match self {
            Predicate::All(children) | Predicate::Any(children) => {
                children.iter().any(Predicate::filters_a_field)
            }
            Predicate::Compare { path, .. } | Predicate::Test { path, .. } => {
                matches!(path, Path::Field(_))
            }
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
    values_equivalent(left, right)
}

/// Equality as the **array field transforms** see it.
///
/// `arrayUnion`/`arrayRemove` are documented to treat equivalent numbers of
/// different types (`3` and `3.0`) as equal, and — unlike an equality filter —
/// `NaN` as equal to `NaN` and `null` as equal to `null`. That is exactly the
/// sort order's notion of equality, so it is defined here, beside it, rather
/// than as a second value comparator in
/// [`transforms`](crate::local::transforms) that could drift from this one.
pub(crate) fn values_equivalent(left: &Value, right: &Value) -> bool {
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

/// Which end of the result range a cursor bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    End,
}

/// One end of the result range, compiled from a `start_at`/`end_at` cursor.
///
/// A cursor is a *position* in the query's sort order, not a filter on a
/// field: its values line up positionally with the leading `order_by` terms,
/// and may be shorter than the full order.
#[derive(Debug, Clone)]
struct Bound {
    values: Vec<Value>,
    /// Whether a document sitting exactly at `values` is inside the range.
    inclusive: bool,
}

impl Bound {
    /// Compile a cursor against the query's compiled `order`.
    ///
    /// The proto carries `before` — whether the position is just before or
    /// just after the values — which the SDK methods map onto like this:
    ///
    /// | SDK call | end | `before` | equal documents |
    /// |---|---|---|---|
    /// | `startAt` | start | `true` | included |
    /// | `startAfter` | start | `false` | excluded |
    /// | `endBefore` | end | `true` | excluded |
    /// | `endAt` | end | `false` | included |
    fn compile(cursor: &Cursor, order: &[SortKey], end: End) -> Result<Self, Status> {
        let at = match end {
            End::Start => "start_at",
            End::End => "end_at",
        };
        if cursor.values.is_empty() {
            return Err(Status::invalid_argument(format!(
                "the {at} cursor has no values"
            )));
        }
        if cursor.values.len() > order.len() {
            return Err(Status::invalid_argument(format!(
                "the {at} cursor has {} values but the query orders by {} field(s)",
                cursor.values.len(),
                order.len()
            )));
        }
        // A `__name__` position is a document, so it has to arrive as a
        // reference. A string of the same text sorts in a different type
        // bucket, which would silently move the cursor somewhere else.
        for (key, value) in order.iter().zip(cursor.values.iter()) {
            if key.path == Path::Name
                && !matches!(value.value_type, Some(ValueType::ReferenceValue(_)))
            {
                return Err(Status::invalid_argument(format!(
                    "the {at} cursor's {DOCUMENT_NAME_FIELD} value must be a referenceValue"
                )));
            }
        }

        Ok(Self {
            values: cursor.values.clone(),
            inclusive: match end {
                End::Start => cursor.before,
                End::End => !cursor.before,
            },
        })
    }
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
    /// Whether that selector asked for every collection with this id anywhere
    /// beneath the request's parent, rather than the one directly under it.
    all_descendants: bool,
    filter: Option<Predicate>,
    /// Always non-empty and always ends with `__name__`, which is Firestore's
    /// implicit tiebreak and what makes the result order deterministic.
    order: Vec<SortKey>,
    /// The `start_at` cursor, if the query is bounded below.
    start: Option<Bound>,
    /// The `end_at` cursor, if the query is bounded above.
    end: Option<Bound>,
    offset: usize,
    limit: Option<usize>,
    /// `select` projection: the field paths to return, if restricted.
    projection: Option<Vec<FieldPath>>,
}

impl Query {
    /// Compile a `StructuredQuery`, rejecting every shape this phase does not
    /// serve.
    pub fn compile(query: &StructuredQuery) -> Result<Self, Status> {
        if query.find_nearest.is_some() {
            return Err(unsupported("vector search (find_nearest)", "out of scope"));
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
        if selector.collection_id.is_empty() {
            // With `all_descendants` this is the kindless scan of every
            // document under the parent, which the API allows and this server
            // does not serve; without it, it is simply a malformed query.
            return Err(if selector.all_descendants {
                unsupported(
                    "a collection group query with no collection id",
                    "name the collection id to scan",
                )
            } else {
                Status::invalid_argument("`from` must name a collection id")
            });
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

        // Nothing may follow the key. `__name__` is the tie-breaker that makes
        // an order total, so a term after it can never change a result, and
        // Firestore refuses the shape rather than quietly ignoring the dead
        // term.
        //
        // The refusal is checked against the order Firestore *uses*, which is
        // not the one on the wire: it also orders by every inequality-filtered
        // field the client did not order by, appended after the explicit terms.
        // So `.where('v', '!=', null).orderBy('__name__')` asks for
        // `[__name__, v]` while sending only `[__name__]`. That matters because
        // `@google-cloud/firestore` builds exactly that on its own — the shape
        // is what a user writes on the first day, not a corner case, and
        // serving it here would mean their query dies in production.
        let unordered_inequality = filter.as_ref().is_some_and(|filter| {
            let mut paths = Vec::new();
            filter.inequality_paths(&mut paths);
            paths
                .iter()
                .any(|path| !order.iter().any(|key| &key.path == path))
        });
        if let Some(key) = order.iter().position(|key| key.path == Path::Name) {
            if key + 1 < order.len() || unordered_inequality {
                return Err(Status::invalid_argument(
                    "order by clause cannot contain more fields after the key",
                ));
            }
        }

        // A descending scan of nothing but the key. Firestore reads key order
        // out of the index that stores the documents, and only forwards, so it
        // refuses this with `FAILED_PRECONDITION` rather than
        // `INVALID_ARGUMENT`: the shape is legal, there is just no index that
        // serves it.
        //
        // Anything else to sort or filter by lifts the refusal, because that
        // field's index ends in the key and *can* be read backwards. An
        // explicit term before the key is one — which is why
        // `orderBy('v', 'desc')` keeps working — and so is any predicate on a
        // field, down to `IS_NULL`. A predicate on `__name__` is not: it is a
        // range over the same forward-only store. Measured case by case
        // against `cloud-firestore-emulator` v1.22.0 (SAK-223); fireup does
        // not model indexes, so this is the one place their shape shows
        // through, and it is written as the rule rather than derived from one.
        if matches!(order.as_slice(), [key] if key.path == Path::Name && key.descending)
            && !filter.as_ref().is_some_and(Predicate::filters_a_field)
        {
            return Err(Status::failed_precondition(
                "Firestore does not support descending key scans",
            ));
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

        // Compiled after `order`, because a cursor's values are positions in
        // that order rather than named fields.
        let start = query
            .start_at
            .as_ref()
            .map(|cursor| Bound::compile(cursor, &order, End::Start))
            .transpose()?;
        let end = query
            .end_at
            .as_ref()
            .map(|cursor| Bound::compile(cursor, &order, End::End))
            .transpose()?;

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
            all_descendants: selector.all_descendants,
            filter,
            order,
            start,
            end,
            offset,
            limit,
            projection,
        })
    }

    /// The collection this query reads, relative to the request's parent.
    pub fn collection_id(&self) -> &str {
        &self.collection_id
    }

    /// Whether this is a collection group query: every collection called
    /// [`collection_id`](Self::collection_id) anywhere beneath the request's
    /// parent, rather than only the one directly under it.
    pub fn all_descendants(&self) -> bool {
        self.all_descendants
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

    /// Whether a document is a candidate at all: it must match the filter,
    /// carry every field the query orders by, and lie between the cursors.
    ///
    /// The second condition is easy to miss and visible to users: Firestore
    /// cannot order a document that has no value for the sort field, so such
    /// documents are absent from the result rather than sorted to one end.
    fn admits(&self, document: &Document) -> bool {
        if !self
            .order
            .iter()
            .all(|key| key.path.resolve(document).is_some())
        {
            return false;
        }
        if !self.within_bounds(document) {
            return false;
        }
        match &self.filter {
            None => true,
            Some(filter) => filter.matches(document),
        }
    }

    /// Whether a document lies inside the `start_at`/`end_at` range.
    ///
    /// A cursor describes a position in the sort order, but because that order
    /// is total and known per document, membership is decided per document
    /// rather than by slicing the sorted sequence.
    fn within_bounds(&self, document: &Document) -> bool {
        if let Some(start) = &self.start {
            match self.position(document, start) {
                Ordering::Less => return false,
                Ordering::Equal if !start.inclusive => return false,
                _ => {}
            }
        }
        if let Some(end) = &self.end {
            match self.position(document, end) {
                Ordering::Greater => return false,
                Ordering::Equal if !end.inclusive => return false,
                _ => {}
            }
        }
        true
    }

    /// Where `document` sits relative to a cursor, in the query's sort order.
    ///
    /// Only the leading order terms the cursor supplies values for are
    /// compared, so a short cursor positions on a whole group of documents
    /// that share that prefix.
    fn position(&self, document: &Document, bound: &Bound) -> Ordering {
        for (key, value) in self.order.iter().zip(bound.values.iter()) {
            // `admits` checks the ordered fields before this runs, so a
            // missing value is impossible; skipping the term keeps the
            // comparison total either way.
            let Some(actual) = key.path.resolve(document) else {
                continue;
            };
            let ordering = compare_values(actual.as_ref(), value);
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
        if let Some(paths) = &self.projection {
            retain_field_paths(&mut document.fields, paths);
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

    fn map_value(fields: Vec<(&str, Value)>) -> Value {
        Value {
            value_type: Some(ValueType::MapValue(MapValue {
                fields: fields
                    .into_iter()
                    .map(|(field, value)| (field.to_string(), value))
                    .collect(),
            })),
        }
    }

    fn reference_value(name: &str) -> Value {
        Value {
            value_type: Some(ValueType::ReferenceValue(name.to_string())),
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
            compile(|q| q.find_nearest = Some(Default::default())).code(),
            tonic::Code::Unimplemented,
            "vector search is not served"
        );
        assert_eq!(
            compile(|q| {
                q.from[0].all_descendants = true;
                q.from[0].collection_id = String::new();
            })
            .code(),
            tonic::Code::Unimplemented,
            "a kindless collection group scan is not served"
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

    // --- `order_by` shapes Firestore refuses --------------------------------
    //
    // Every expectation in this section was measured against
    // `cloud-firestore-emulator` v1.22.0 over raw gRPC (SAK-223), one case at a
    // time, rather than read off the documentation. Both rules are narrower
    // than they first look, so each test carries the shapes production *serves*
    // beside the ones it refuses: over-refusing here would break working code,
    // which is a worse failure than the bug being fixed.

    /// Build `(order_by, where)` and compile it.
    fn compile_order(shape: &[(&str, bool)], filter: Option<Filter>) -> Result<Query, Status> {
        let mut structured = StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "users".to_string(),
                all_descendants: false,
            }],
            ..Default::default()
        };
        structured.order_by = shape
            .iter()
            .map(|(path, descending)| Order {
                field: Some(FieldReference {
                    field_path: path.to_string(),
                }),
                direction: if *descending {
                    Direction::Descending as i32
                } else {
                    Direction::Ascending as i32
                },
            })
            .collect();
        structured.r#where = filter;
        Query::compile(&structured)
    }

    fn unary_filter(path: &str, op: ProtoUnaryOperator) -> Option<Filter> {
        Some(Filter {
            filter_type: Some(FilterType::UnaryFilter(UnaryFilter {
                operand_type: Some(OperandType::Field(FieldReference {
                    field_path: path.to_string(),
                })),
                op: op as i32,
            })),
        })
    }

    fn refused_after_the_key(shape: &[(&str, bool)], filter: Option<Filter>, why: &str) {
        let error = compile_order(shape, filter).expect_err(why);
        assert_eq!(error.code(), tonic::Code::InvalidArgument, "{why}");
        assert_eq!(
            error.message(),
            "order by clause cannot contain more fields after the key",
            "the refusal has to name the key, so {why} is distinguishable from \
             every other invalid argument"
        );
    }

    fn served(shape: &[(&str, bool)], filter: Option<Filter>, why: &str) {
        compile_order(shape, filter)
            .unwrap_or_else(|error| panic!("Firestore serves {why}, and it was refused: {error}"));
    }

    #[test]
    fn nothing_may_follow_the_key_in_an_order() {
        for (shape, why) in [
            (
                &[("__name__", false), ("__name__", false)][..],
                "a duplicate key",
            ),
            (&[("__name__", false), ("age", false)][..], "a field after"),
            (
                &[("__name__", true), ("age", true)][..],
                "a field after, descending",
            ),
            (
                &[("age", false), ("__name__", false), ("__name__", false)][..],
                "a duplicate key, not first",
            ),
            (
                &[("age", false), ("__name__", false), ("city", false)][..],
                "a field after, not first",
            ),
        ] {
            refused_after_the_key(shape, None, why);
        }

        served(&[][..], None, "the default order");
        served(&[("__name__", false)][..], None, "the key alone");
        served(
            &[("age", false), ("__name__", false)][..],
            None,
            "the key last",
        );
        served(
            &[("age", true), ("__name__", false)][..],
            None,
            "the key last, in its own direction",
        );
    }

    /// The rule is about the order Firestore *uses*, not the one on the wire.
    ///
    /// This is the case that matters most: the wire carries a legal
    /// `[__name__]`, and it is illegal anyway because the inequality field
    /// joins the order behind it. `@google-cloud/firestore` builds it from
    /// `.where('v', '!=', null).orderBy('__name__')` without being asked, so
    /// serving it locally breaks a first-day query in production.
    #[test]
    fn an_unordered_inequality_field_counts_as_a_term_after_the_key() {
        for (op, name) in [
            (ProtoFieldOperator::NotEqual, "!="),
            (ProtoFieldOperator::LessThan, "<"),
            (ProtoFieldOperator::LessThanOrEqual, "<="),
            (ProtoFieldOperator::GreaterThan, ">"),
            (ProtoFieldOperator::GreaterThanOrEqual, ">="),
        ] {
            refused_after_the_key(
                &[("__name__", false)],
                field_filter("age", op, integer_value(31)),
                &format!("an unordered `age {name}` behind an explicit key"),
            );
            // Ordering by the inequality field first is the fix, and is served.
            served(
                &[("age", false), ("__name__", false)],
                field_filter("age", op, integer_value(31)),
                &format!("`age {name}` with `age` ordered first"),
            );
        }
        refused_after_the_key(
            &[("__name__", false)],
            field_filter(
                "age",
                ProtoFieldOperator::NotIn,
                array_value(vec![integer_value(31)]),
            ),
            "NOT_IN is an inequality",
        );
        refused_after_the_key(
            &[("__name__", false)],
            unary_filter("age", ProtoUnaryOperator::IsNotNull),
            "IS_NOT_NULL is how `!= null` reaches the wire",
        );
        refused_after_the_key(
            &[("__name__", false)],
            unary_filter("age", ProtoUnaryOperator::IsNotNan),
            "IS_NOT_NAN is how `!= NaN` reaches the wire",
        );
        refused_after_the_key(
            &[("__name__", false)],
            Some(Filter {
                filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                    op: ProtoCompositeOperator::Or as i32,
                    filters: vec![
                        field_filter("city", ProtoFieldOperator::Equal, string_value("Tokyo"))
                            .expect("a filter"),
                        field_filter("age", ProtoFieldOperator::LessThan, integer_value(40))
                            .expect("a filter"),
                    ],
                })),
            }),
            "an inequality inside an OR branch still joins the order",
        );

        // The operators that are *not* inequalities get no implicit order, so
        // an explicit trailing key stays legal beside them. Getting this half
        // wrong would refuse working queries.
        for (op, value, name) in [
            (ProtoFieldOperator::Equal, integer_value(31), "=="),
            (
                ProtoFieldOperator::In,
                array_value(vec![integer_value(31)]),
                "in",
            ),
            (
                ProtoFieldOperator::ArrayContains,
                integer_value(31),
                "array-contains",
            ),
            (
                ProtoFieldOperator::ArrayContainsAny,
                array_value(vec![integer_value(31)]),
                "array-contains-any",
            ),
        ] {
            served(
                &[("__name__", false)],
                field_filter("age", op, value),
                &format!("`age {name}` beside an explicit key"),
            );
        }
        served(
            &[("__name__", false)],
            unary_filter("age", ProtoUnaryOperator::IsNull),
            "IS_NULL is an equality",
        );
        served(
            &[("__name__", false)],
            unary_filter("age", ProtoUnaryOperator::IsNan),
            "IS_NAN is an equality",
        );
        // An inequality on the key itself is already ordered by the key.
        served(
            &[("__name__", false)],
            field_filter(
                "__name__",
                ProtoFieldOperator::GreaterThanOrEqual,
                reference_value(&format!("{ROOT}/users/bob")),
            ),
            "`__name__ >=` beside an explicit key",
        );
    }

    #[test]
    fn a_descending_key_scan_with_no_index_behind_it_is_refused() {
        let error = compile_order(&[("__name__", true)], None).expect_err("a bare descending key");
        assert_eq!(
            error.code(),
            tonic::Code::FailedPrecondition,
            "the shape is legal and no index serves it, which is not the same \
             thing as a malformed request"
        );
        assert_eq!(
            error.message(),
            "Firestore does not support descending key scans"
        );

        // A filter on the key is a range over the same forward-only store, so
        // it does not lift the refusal.
        let error = compile_order(
            &[("__name__", true)],
            field_filter(
                "__name__",
                ProtoFieldOperator::GreaterThanOrEqual,
                reference_value(&format!("{ROOT}/users/bob")),
            ),
        )
        .expect_err("a descending key scan over a key range");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);

        // Anything else to sort or filter by does lift it: that field's index
        // ends in the key and can be read backwards.
        served(
            &[("age", true), ("__name__", true)],
            None,
            "a descending key as the tie-breaker — what orderBy(f, 'desc') compiles to",
        );
        served(
            &[("age", false), ("__name__", true)],
            None,
            "a descending key behind an ascending field",
        );
        for (op, value, name) in [
            (ProtoFieldOperator::Equal, integer_value(31), "=="),
            (
                ProtoFieldOperator::In,
                array_value(vec![integer_value(31)]),
                "in",
            ),
            (
                ProtoFieldOperator::ArrayContains,
                integer_value(31),
                "array-contains",
            ),
        ] {
            served(
                &[("__name__", true)],
                field_filter("age", op, value),
                &format!("a descending key scan filtered by `age {name}`"),
            );
        }
        served(
            &[("__name__", true)],
            unary_filter("age", ProtoUnaryOperator::IsNull),
            "a descending key scan filtered by IS_NULL",
        );
    }

    /// The explicit direction of a trailing `__name__` survives compilation.
    ///
    /// Guards the seam between the two halves above: it would be easy to make
    /// the descending-key refusal pass by normalising every trailing key to
    /// ascending, which would silently reorder a served query's ties.
    #[test]
    fn an_explicit_trailing_name_keeps_its_own_direction() {
        let found = query(|q| {
            q.order_by = vec![
                Order {
                    field: Some(FieldReference {
                        field_path: "age".to_string(),
                    }),
                    direction: Direction::Ascending as i32,
                },
                Order {
                    field: Some(FieldReference {
                        field_path: "__name__".to_string(),
                    }),
                    direction: Direction::Descending as i32,
                },
            ];
        })
        .evaluate(people());

        assert_eq!(
            ids(&found),
            ["bob", "alice", "carol"],
            "alice and bob tie on age 31, so the descending key puts bob first"
        );
    }

    // --- Cursors -----------------------------------------------------------

    fn cursor(values: Vec<Value>, before: bool) -> Option<Cursor> {
        Some(Cursor { values, before })
    }

    fn order_by(path: &str, descending: bool) -> Vec<Order> {
        vec![Order {
            field: Some(FieldReference {
                field_path: path.to_string(),
            }),
            direction: if descending {
                Direction::Descending as i32
            } else {
                Direction::Ascending as i32
            },
        }]
    }

    /// `__name__`-only cursors, which is what a default-ordered
    /// `startAt`/`endAt` over document ids compiles to.
    #[test]
    fn cursors_bound_the_default_name_order() {
        let at = |values, before, end: End| {
            query(|q| match end {
                End::Start => q.start_at = cursor(values, before),
                End::End => q.end_at = cursor(values, before),
            })
            .evaluate(people())
        };
        let bob = vec![reference_value(&format!("{ROOT}/users/bob"))];

        assert_eq!(
            ids(&at(bob.clone(), true, End::Start)),
            ["bob", "carol", "dan"],
            "startAt includes the cursor document"
        );
        assert_eq!(
            ids(&at(bob.clone(), false, End::Start)),
            ["carol", "dan"],
            "startAfter excludes it"
        );
        assert_eq!(
            ids(&at(bob.clone(), false, End::End)),
            ["alice", "bob"],
            "endAt includes it"
        );
        assert_eq!(
            ids(&at(bob, true, End::End)),
            ["alice"],
            "endBefore excludes it"
        );
    }

    #[test]
    fn a_start_and_an_end_cursor_make_a_range() {
        let found = query(|q| {
            q.start_at = cursor(vec![reference_value(&format!("{ROOT}/users/bob"))], true);
            q.end_at = cursor(vec![reference_value(&format!("{ROOT}/users/dan"))], true);
        })
        .evaluate(people());

        assert_eq!(ids(&found), ["bob", "carol"]);
    }

    /// The case that matters for paging: `startAfter(lastDocumentSnapshot)`
    /// sends one value per order term, ending in the document's own name.
    #[test]
    fn a_full_snapshot_cursor_pages_past_a_tie() {
        let page = |after: Option<&str>| {
            query(|q| {
                q.order_by = order_by("age", false);
                q.limit = Some(2);
                if let Some(id) = after {
                    let age = if id == "carol" { 41 } else { 31 };
                    q.start_at = cursor(
                        vec![
                            integer_value(age),
                            reference_value(&format!("{ROOT}/users/{id}")),
                        ],
                        false,
                    );
                }
            })
            .evaluate(people())
        };

        // alice and bob are both 31, so only the name breaks the tie.
        assert_eq!(ids(&page(None)), ["alice", "bob"]);
        assert_eq!(
            ids(&page(Some("bob"))),
            ["carol"],
            "paging after a tied document must not repeat its partner"
        );
        assert_eq!(ids(&page(Some("carol"))), [] as [&str; 0]);
    }

    #[test]
    fn a_short_cursor_positions_on_every_document_sharing_the_prefix() {
        let found = query(|q| {
            q.order_by = order_by("age", false);
            q.start_at = cursor(vec![integer_value(31)], false);
        })
        .evaluate(people());

        assert_eq!(
            ids(&found),
            ["carol"],
            "startAfter(31) skips the whole group of 31s, not just one of them"
        );
    }

    #[test]
    fn a_cursor_follows_a_descending_order() {
        let found = query(|q| {
            q.order_by = order_by("age", true);
            q.start_at = cursor(vec![integer_value(41)], false);
        })
        .evaluate(people());

        assert_eq!(
            ids(&found),
            ["bob", "alice"],
            "descending, 'after 41' means the smaller ages"
        );
    }

    #[test]
    fn a_cursor_applies_before_offset_and_limit() {
        let found = query(|q| {
            q.start_at = cursor(vec![reference_value(&format!("{ROOT}/users/alice"))], false);
            q.offset = 1;
            q.limit = Some(1);
        })
        .evaluate(people());

        // After alice: bob, carol, dan. Offset 1, limit 1 of that.
        assert_eq!(ids(&found), ["carol"]);
        assert_eq!(found.skipped, 1);
    }

    #[test]
    fn a_cursor_the_query_cannot_position_is_rejected() {
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
            compile(|q| q.start_at = cursor(vec![], true)).code(),
            tonic::Code::InvalidArgument,
            "a cursor with no values has no position"
        );
        assert_eq!(
            compile(|q| {
                // The implicit order is `__name__` alone, so two values cannot
                // line up with it.
                q.start_at = cursor(vec![integer_value(1), integer_value(2)], true)
            })
            .code(),
            tonic::Code::InvalidArgument,
            "a cursor must not be longer than the order"
        );
        assert_eq!(
            compile(|q| q.end_at = cursor(vec![string_value("bob")], false)).code(),
            tonic::Code::InvalidArgument,
            "a __name__ position must arrive as a reference, not a string"
        );
    }

    // --- Collection group queries ------------------------------------------

    #[test]
    fn a_collection_group_query_is_compiled_and_flagged() {
        let group = query(|q| q.from[0].all_descendants = true);
        assert!(group.all_descendants(), "the scan has to widen");
        assert_eq!(group.collection_id(), "users");

        assert!(
            !query(|_| {}).all_descendants(),
            "an ordinary query still reads one collection"
        );
    }

    #[test]
    fn a_collection_group_query_orders_documents_from_different_parents() {
        let at = |name: &str, age: i64| Document {
            name: format!("{ROOT}/{name}"),
            fields: [("age".to_string(), integer_value(age))]
                .into_iter()
                .collect(),
            create_time: None,
            update_time: None,
        };
        let documents = vec![
            at("teams/b/members/zoe", 20),
            at("members/root", 30),
            at("teams/a/members/amy", 20),
        ];

        let found = query(|q| {
            q.from[0].collection_id = "members".to_string();
            q.from[0].all_descendants = true;
            q.order_by = order_by("age", false);
        })
        .evaluate(documents);

        let names: Vec<&str> = found
            .documents
            .iter()
            .map(|document| {
                document
                    .name
                    .strip_prefix(&format!("{ROOT}/"))
                    .expect("under the root")
            })
            .collect();
        assert_eq!(
            names,
            ["teams/a/members/amy", "teams/b/members/zoe", "members/root"],
            "age first, then the full resource name as the tiebreak"
        );
    }

    // --- Nested field paths ------------------------------------------------

    fn addressed() -> Vec<Document> {
        vec![
            document(
                "alice",
                vec![(
                    "address",
                    map_value(vec![
                        ("city", string_value("Tokyo")),
                        ("zip", string_value("100-0001")),
                    ]),
                )],
            ),
            document(
                "bob",
                vec![("address", map_value(vec![("city", string_value("Kyoto"))]))],
            ),
            // A document whose `address` is not a map at all.
            document("carol", vec![("address", string_value("Osaka"))]),
            // A document with a top-level field literally spelled "address.city".
            document("dan", vec![("address.city", string_value("Tokyo"))]),
        ]
    }

    #[test]
    fn a_filter_on_a_nested_path_reads_inside_the_map() {
        let found = query(|q| {
            q.r#where = field_filter(
                "address.city",
                ProtoFieldOperator::Equal,
                string_value("Tokyo"),
            );
        })
        .evaluate(addressed());

        assert_eq!(
            ids(&found),
            ["alice"],
            "carol's address is a string and dan's field is only spelled that way"
        );
    }

    #[test]
    fn a_quoted_segment_addresses_a_field_whose_name_has_a_dot() {
        let found = query(|q| {
            q.r#where = field_filter(
                "`address.city`",
                ProtoFieldOperator::Equal,
                string_value("Tokyo"),
            );
        })
        .evaluate(addressed());

        assert_eq!(
            ids(&found),
            ["dan"],
            "the backticks make it one segment, not a walk into a map"
        );
    }

    #[test]
    fn ordering_by_a_nested_path_drops_documents_that_lack_it() {
        let found = query(|q| q.order_by = order_by("address.city", false)).evaluate(addressed());

        assert_eq!(
            ids(&found),
            ["bob", "alice"],
            "Kyoto before Tokyo; carol and dan have no address.city to order by"
        );
    }

    #[test]
    fn a_projection_of_a_nested_path_keeps_the_nesting() {
        let found = query(|q| {
            q.select = Some(Projection {
                fields: vec![FieldReference {
                    field_path: "address.city".to_string(),
                }],
            });
            q.r#where = field_filter(
                "address.city",
                ProtoFieldOperator::Equal,
                string_value("Tokyo"),
            );
        })
        .evaluate(addressed());

        let only = &found.documents[0];
        assert!(
            !only.fields.contains_key("address.city"),
            "the path must not be flattened into a field name"
        );
        let address = only.fields.get("address").expect("address survives");
        let Some(ValueType::MapValue(map)) = &address.value_type else {
            panic!("address stays a map: {address:?}");
        };
        assert!(map.fields.contains_key("city"));
        assert!(
            !map.fields.contains_key("zip"),
            "an unselected sibling inside the map is dropped"
        );
    }

    #[test]
    fn field_paths_parse_to_their_segments() {
        let segments = |path: &str| {
            FieldPath::parse(path)
                .unwrap_or_else(|error| panic!("{path:?} should parse: {error}"))
                .segments
        };

        assert_eq!(segments("a"), ["a"]);
        assert_eq!(segments("a.b.c"), ["a", "b", "c"]);
        assert_eq!(segments("a_1.b2"), ["a_1", "b2"]);
        assert_eq!(segments("`a.b`"), ["a.b"], "a quoted dot is literal");
        assert_eq!(segments("`a.b`.c"), ["a.b", "c"]);
        assert_eq!(segments("`a b`"), ["a b"]);
        assert_eq!(segments(r"`a\`b`"), ["a`b"], "an escaped backtick");
        assert_eq!(segments(r"`a\\b`"), [r"a\b"], "an escaped backslash");
    }

    #[test]
    fn a_malformed_field_path_is_an_invalid_argument() {
        for path in [
            "", ".", "a.", ".a", "a..b", "`a", "``", "1abc", "a b", "a-b", r"`a\nb`", "`a`b",
        ] {
            let error = FieldPath::parse(path).expect_err(&format!("{path:?} is malformed"));
            assert_eq!(
                error.code(),
                tonic::Code::InvalidArgument,
                "{path:?} should be an invalid argument, not {error}"
            );
        }
    }

    #[test]
    fn a_quoted_name_field_is_an_ordinary_field() {
        let documents = vec![
            document("alice", vec![(DOCUMENT_NAME_FIELD, string_value("mine"))]),
            document("bob", vec![("age", integer_value(1))]),
        ];

        let found = query(|q| {
            q.r#where = field_filter(
                &format!("`{DOCUMENT_NAME_FIELD}`"),
                ProtoFieldOperator::Equal,
                string_value("mine"),
            );
        })
        .evaluate(documents);

        assert_eq!(
            ids(&found),
            ["alice"],
            "quoted, it is a stored field rather than the resource name"
        );
    }
}
