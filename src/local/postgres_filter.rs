//! Firestore filter → PostgreSQL `WHERE` fragment.
//!
//! This is the pushdown half of the store's filtered reads
//! ([`Store::scan_collection_filtered`] and `scan_kind_filtered`): it turns a
//! compiled query's [`Prefilter`] into SQL the database can apply, so a query
//! over a large collection ships and decodes the rows it might want instead of
//! all of them.
//!
//! ## The one rule
//!
//! Everything here is **sound, not exact**. The fragment it produces must keep
//! every row the query could match; it is free to keep rows the query will
//! reject, because [`Query::evaluate`](crate::local::query::Query::evaluate)
//! runs afterwards and has the only vote on the result. Every function
//! therefore has two legal answers for a predicate it cannot translate
//! faithfully: translate it, or return `None` and let the row through.
//!
//! That asymmetry is what makes the feature safe to add to a semantics-heavy
//! query engine. `MemoryStore` pushes nothing down, so the conformance tests
//! and the whole in-process suite keep testing the Rust semantics; this module
//! can only change how many rows are read.
//!
//! ## What is pushed, and what stays in-process
//!
//! Pushed:
//!
//! - `==` and `IN` on `null`, booleans, numbers, timestamps, strings, bytes,
//!   references, geo points, and `__name__`.
//! - `<`, `<=`, `>`, `>=` on numbers, booleans, strings, references,
//!   timestamps, and `__name__`.
//! - `IS NULL` / `IS NAN` exactly, and `IS NOT NULL` / `IS NOT NAN` as the
//!   field-presence half of their meaning.
//! - The implicit "a document must carry every field it is ordered by".
//! - `AND` of any of the above, and `OR` when *every* branch is pushable.
//!
//! Each of those works on a nested path as well as a top-level field; see
//! *Nested field paths* below.
//!
//! Left in-process, deliberately:
//!
//! | Case | Why |
//! | --- | --- |
//! | `!=`, `NOT_IN` | A negation of a predicate that is only *approximately* translatable is not sound, and these two are not selective enough to be worth an exact translation. |
//! | `ARRAY_CONTAINS`, `ARRAY_CONTAINS_ANY` | Firestore compares elements by value, where an integer `1` equals a double `1.0`; JSONB containment compares them as written. |
//! | `==` on an array or map | Same reason: the numeric equivalence applies at every depth inside the value. |
//! | `<`/`>` on bytes | Values are stored base64-encoded, and base64 does not sort like the bytes it encodes. |
//! | `<`/`>` on null, geo points, arrays, maps | No ordering a SQL expression can express cheaply, and no real query uses them. |
//! | `OR` with one unpushable branch | That branch could match any row, so the whole `OR` could. |
//! | `offset` / `limit` / cursors | All three are positional, resolved against the ordered result the pre-filter has already fed, so there is no way to know which rows they would keep. |
//!
//! ## Why the comparisons look the way they do
//!
//! Three details are not interchangeable with the obvious spelling:
//!
//! - **`COLLATE "C"`.** Firestore orders strings, references, and `__name__` by
//!   UTF-8 bytes. A bare `<` would use the database's collation, which for most
//!   locales disagrees (it ignores case and punctuation differently), so rows
//!   that should match would be dropped. `"C"` is byte order and is built into
//!   every PostgreSQL install.
//! - **`@>` rather than `=` for equality.** Containment of a one-key object
//!   whose value has exactly one type tag is equality, and unlike `=` on
//!   `fields -> 'x'` it can be served by a GIN index on the `fields` column.
//! - **Two branches for numbers.** An integer and a double live under different
//!   JSON tags but are one type to Firestore. The casts mirror the in-process
//!   engine exactly: integer-to-integer compares as `numeric` (exact), and any
//!   comparison involving a double compares as `double precision`, including
//!   its rounding past 2^53.
//!
//! ## Nested field paths
//!
//! A filter or order can address `address.city`, which is stored as a map
//! value: `{"address": {"mapValue": {"fields": {"city": …}}}}`. Every
//! comparison therefore addresses its field through [`Binder::address`], which
//! walks that shape one `-> 'mapValue' -> 'fields'` hop per extra segment —
//! the same walk `FieldPath::resolve` does in the query engine.
//!
//! The thing not to do is re-join the segments with a `.` and look for a
//! top-level field of that name. It compiles, it reads naturally, and it
//! quietly matches nothing — which on a query means dropping documents that
//! should have been returned, the one failure this module exists to avoid.
//!
//! [`Store::scan_collection_filtered`]: crate::local::store::Store::scan_collection_filtered
//! [`Prefilter`]: crate::local::query::Prefilter
//! [`Path::Field`]: crate::local::query::Path::Field

use serde_json::{Map, Value as Json};

use crate::local::query::{Comparison, FieldPath, Path, Predicate, Prefilter, Presence};
use crate::local::value_json;
use crate::protos::google::firestore::v1::value::ValueType;
use crate::protos::google::firestore::v1::Value;

/// A `WHERE` fragment and the parameters it binds, in placeholder order.
///
/// Every parameter is text: the SQL casts it to whatever it needs, so the
/// caller has one uniform list to bind and no value is ever interpolated into
/// the statement.
///
/// Where a cast is needed it is spelled `$n::text::jsonb` rather than
/// `$n::jsonb`. PostgreSQL infers a bare parameter's type from the cast
/// applied to it, so the short spelling would ask the client to send `jsonb`
/// and the bind would be rejected; the extra step pins the parameter to text.
#[derive(Debug)]
pub(crate) struct SqlPrefilter {
    pub sql: String,
    pub params: Vec<String>,
}

/// A predicate that matches nothing, for a comparison whose own types rule out
/// every document (`__name__ < 5`, `== NaN`).
const MATCHES_NOTHING: &str = "false";

/// Width of the canonical timestamp encoding, `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`.
///
/// Within that width the encoding is fixed-field, so its byte order *is*
/// chronological order. Taken from the encoder rather than restated, because
/// the whole comparison below is unsound the moment the two disagree.
const CANONICAL_TIMESTAMP_LEN: usize = value_json::ENCODED_TIMESTAMP_LEN;

/// Accumulates bound parameters and hands out their placeholders.
struct Binder {
    params: Vec<String>,
    /// Placeholder number of the first parameter this builder owns; the caller
    /// has already bound `first - 1` of its own.
    first: usize,
}

impl Binder {
    /// Bind `value` and return the placeholder that refers to it.
    fn bind(&mut self, value: impl Into<String>) -> String {
        self.params.push(value.into());
        format!("${}", self.first + self.params.len() - 1)
    }

    /// A JSONB expression addressing `path` inside the `fields` column.
    ///
    /// Each segment past the first steps through the stored shape of a map
    /// value, `{"mapValue": {"fields": {…}}}`, which is the same walk
    /// `FieldPath::resolve` does in the query engine. Segment names are bound
    /// as parameters, never interpolated.
    ///
    /// Yields SQL `NULL` whenever any hop is missing or is not a map — which
    /// is what makes every comparison built on it refuse a document that does
    /// not have the field, the way Firestore does.
    fn address(&mut self, path: &FieldPath) -> String {
        let mut segments = path.segments().iter();
        let first = segments
            .next()
            .expect("a FieldPath always has at least one segment");
        let mut expression = format!("fields -> {}", self.bind(first.clone()));
        for segment in segments {
            expression.push_str(" -> 'mapValue' -> 'fields' -> ");
            expression.push_str(&self.bind(segment.clone()));
        }
        expression
    }

    /// A `@>` operand asserting that `path` holds `encoded`.
    ///
    /// Containment rather than equality on the addressed value, because
    /// containment can be served by the GIN index on `fields`. Sound as
    /// *equality* only because the value at a leaf has exactly one type tag,
    /// so nothing can contain it without equalling it — which is why the
    /// caller must not pass an array or map value.
    fn containment(&mut self, path: &FieldPath, encoded: Json) -> Option<String> {
        // Built from the innermost segment outwards, wrapping a map layer for
        // each step back up.
        let mut wanted = encoded;
        for segment in path.segments().iter().skip(1).rev() {
            wanted = json_object([(segment.clone(), wanted)]);
            wanted = json_object([("fields".to_string(), wanted)]);
            wanted = json_object([("mapValue".to_string(), wanted)]);
        }
        let outermost = path.segments().first()?.clone();
        wanted = json_object([(outermost, wanted)]);

        let param = self.bind(serde_json::to_string(&wanted).ok()?);
        Some(format!("fields @> {param}::text::jsonb"))
    }
}

fn json_object<const N: usize>(entries: [(String, Json); N]) -> Json {
    let mut object = Map::with_capacity(N);
    for (key, value) in entries {
        object.insert(key, value);
    }
    Json::Object(object)
}

/// Translate `node`, discarding any parameters it bound if it cannot be
/// translated.
///
/// The rollback is the point: a half-built subtree that bound two parameters
/// and then gave up would leave them in the list with nothing referring to
/// them, and PostgreSQL rejects a bind with more parameters than the statement
/// uses. Every recursive call goes through here.
fn push(binder: &mut Binder, node: &Predicate) -> Option<String> {
    let mark = binder.params.len();
    match translate(node, binder) {
        Some(sql) => Some(sql),
        None => {
            binder.params.truncate(mark);
            None
        }
    }
}

/// Compile `prefilter` into a `WHERE` fragment, or `None` when nothing can be
/// pushed down and the caller should scan the collection as it always did.
///
/// `first_placeholder` is the number the fragment's first `$n` should take, so
/// it can be appended to a statement that already has parameters of its own.
pub(crate) fn compile(prefilter: Prefilter<'_>, first_placeholder: usize) -> Option<SqlPrefilter> {
    let mut binder = Binder {
        params: Vec::new(),
        first: first_placeholder,
    };
    let mut conjuncts: Vec<String> = Vec::new();

    // A document with no value for a sort field cannot be ordered, so Firestore
    // leaves it out of the result entirely. That is a filter, and a cheap one.
    for path in prefilter.required_fields {
        conjuncts.push(presence(path, &mut binder));
    }

    if let Some(filter) = prefilter.filter {
        if let Some(sql) = push(&mut binder, filter) {
            conjuncts.push(sql);
        }
    }

    if conjuncts.is_empty() {
        return None;
    }
    Some(SqlPrefilter {
        sql: conjuncts.join(" AND "),
        params: binder.params,
    })
}

fn translate(node: &Predicate, binder: &mut Binder) -> Option<String> {
    match node {
        Predicate::All(children) => {
            // Dropping a conjunct we cannot translate only widens the result,
            // which the superset contract allows — so an AND is pushable as
            // soon as *any* of its children is.
            let mut pushed = Vec::with_capacity(children.len());
            for child in children {
                if let Some(sql) = push(binder, child) {
                    pushed.push(sql);
                }
            }
            if pushed.is_empty() {
                None
            } else {
                Some(format!("({})", pushed.join(" AND ")))
            }
        }
        Predicate::Any(children) => {
            // An OR is only narrower than "keep everything" when every branch
            // is translatable: a branch we had to drop could match any row, and
            // dropping it from an OR would drop those rows.
            let mut pushed = Vec::with_capacity(children.len());
            for child in children {
                pushed.push(push(binder, child)?);
            }
            if pushed.is_empty() {
                // An empty OR matches nothing, which is what the in-process
                // engine's `iter().any()` over no children also says.
                return Some(MATCHES_NOTHING.to_string());
            }
            Some(format!("({})", pushed.join(" OR ")))
        }
        Predicate::Compare { path, op, value } => compare(path, *op, value, binder),
        Predicate::Test { path, op } => test(path, *op, binder),
    }
}

fn compare(path: &Path, op: Comparison, value: &Value, binder: &mut Binder) -> Option<String> {
    match op {
        Comparison::Equal => equality(path, value, binder),
        Comparison::LessThan => range(path, "<", value, binder),
        Comparison::LessThanOrEqual => range(path, "<=", value, binder),
        Comparison::GreaterThan => range(path, ">", value, binder),
        Comparison::GreaterThanOrEqual => range(path, ">=", value, binder),
        Comparison::In => {
            let candidates = match &value.value_type {
                Some(ValueType::ArrayValue(array)) => &array.values,
                // Compilation already rejected this; nothing to push.
                _ => return None,
            };
            if candidates.is_empty() {
                return Some(MATCHES_NOTHING.to_string());
            }
            let mut branches = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                branches.push(equality(path, candidate, binder)?);
            }
            Some(format!("({})", branches.join(" OR ")))
        }
        // See the table in the module docs for why each of these stays in the
        // query engine.
        Comparison::NotEqual
        | Comparison::NotIn
        | Comparison::ArrayContains
        | Comparison::ArrayContainsAny => None,
    }
}

fn equality(path: &Path, value: &Value, binder: &mut Binder) -> Option<String> {
    let path = match path {
        // `__name__` resolves to a reference to the document's own name, so only
        // a reference can equal it — and the name is its own column.
        Path::Name => {
            return match &value.value_type {
                Some(ValueType::ReferenceValue(name)) => {
                    let param = binder.bind(name.clone());
                    Some(format!("name = {param}"))
                }
                _ => Some(MATCHES_NOTHING.to_string()),
            }
        }
        Path::Field(path) => path,
    };

    match &value.value_type {
        // `== NaN` never matches, which is why `IS NAN` exists.
        Some(ValueType::DoubleValue(number)) if number.is_nan() => {
            Some(MATCHES_NOTHING.to_string())
        }
        // One type to Firestore, two JSON tags in storage.
        Some(ValueType::IntegerValue(_)) | Some(ValueType::DoubleValue(_)) => {
            numeric_comparison(path, "=", value, binder)
        }
        // A number nested inside these still compares across int/double, and
        // JSONB containment does not know that.
        Some(ValueType::ArrayValue(_)) | Some(ValueType::MapValue(_)) | None => None,
        // Everything else has exactly one canonical encoding, which both the
        // stored value and this bound value go through, so JSONB containment
        // of the value at this path is Firestore equality.
        Some(_) => binder.containment(path, value_json::encode(value).ok()?),
    }
}

fn range(path: &Path, op: &str, value: &Value, binder: &mut Binder) -> Option<String> {
    // `NaN` is ordered against nothing at all, including itself.
    if matches!(&value.value_type, Some(ValueType::DoubleValue(number)) if number.is_nan()) {
        return Some(MATCHES_NOTHING.to_string());
    }

    let path = match path {
        Path::Name => {
            return match &value.value_type {
                Some(ValueType::ReferenceValue(name)) => {
                    let param = binder.bind(name.clone());
                    Some(format!("name COLLATE \"C\" {op} {param}"))
                }
                // An inequality against a value of another type matches
                // nothing: Firestore indexes each type separately.
                _ => Some(MATCHES_NOTHING.to_string()),
            };
        }
        Path::Field(path) => path,
    };

    match &value.value_type {
        Some(ValueType::IntegerValue(_)) | Some(ValueType::DoubleValue(_)) => {
            numeric_comparison(path, op, value, binder)
        }
        Some(ValueType::StringValue(text)) => {
            Some(text_comparison(path, "stringValue", op, text, binder))
        }
        Some(ValueType::ReferenceValue(name)) => {
            Some(text_comparison(path, "referenceValue", op, name, binder))
        }
        // `false` sorts before `true`, and so do the two spellings' bytes.
        Some(ValueType::BooleanValue(flag)) => Some(text_comparison(
            path,
            "booleanValue",
            op,
            if *flag { "true" } else { "false" },
            binder,
        )),
        Some(ValueType::TimestampValue(_)) => timestamp_range(path, op, value, binder),
        _ => None,
    }
}

/// Compare a field stored under one JSON tag as text, in UTF-8 byte order.
fn text_comparison(
    path: &FieldPath,
    tag: &'static str,
    op: &str,
    text: &str,
    binder: &mut Binder,
) -> String {
    let address = binder.address(path);
    let param = binder.bind(text.to_string());
    // A field holding another type yields SQL NULL here, so the comparison is
    // NULL and the row is not kept — which is what Firestore does too, since an
    // inequality only looks at values of the bound's own type.
    format!("({address} ->> '{tag}') COLLATE \"C\" {op} {param}")
}

/// Compare a numeric field, matching the in-process engine's arithmetic.
///
/// Two branches, because an integer and a double are stored under different
/// tags but are one Firestore type. An absent field or a field of another type
/// yields SQL NULL from `->>`, and `NULL::numeric` is NULL rather than an
/// error, so such a row is simply not kept.
fn numeric_comparison(
    path: &FieldPath,
    op: &str,
    expected: &Value,
    binder: &mut Binder,
) -> Option<String> {
    let address = binder.address(path);

    match &expected.value_type {
        Some(ValueType::IntegerValue(number)) => {
            // Integer against integer is exact in the engine (`i64::cmp`), so
            // it has to be exact here; `numeric` is, and `double precision`
            // would not be past 2^53.
            let exact = binder.bind(number.to_string());
            let widened = binder.bind(render_double(*number as f64));
            Some(format!(
                "(({address} ->> 'integerValue')::numeric {op} {exact}::text::numeric \
                 OR ({address} ->> 'doubleValue')::double precision \
                    {op} {widened}::text::double precision)"
            ))
        }
        Some(ValueType::DoubleValue(number)) => {
            // The engine compares an integer against a double by widening the
            // integer to `f64` first, rounding included, so both branches are
            // `double precision`.
            let widened = binder.bind(render_double(*number));
            Some(format!(
                "(({address} ->> 'integerValue')::double precision \
                    {op} {widened}::text::double precision \
                 OR ({address} ->> 'doubleValue')::double precision \
                    {op} {widened}::text::double precision)"
            ))
        }
        _ => None,
    }
}

/// Compare a timestamp field as text.
///
/// Sound because of the encoding: [`value_json`] writes every timestamp as
/// RFC 3339 with four year digits, nine fractional digits and a `Z`, which is
/// fixed-field, so byte order is chronological order.
///
/// The guard on the stored side is for rows a build predating #69 wrote, whose
/// year sits outside 0001–9999 and whose encoding is therefore both wider and
/// signed — `+11476-…` sorts *before* `2023-…` byte-wise, so comparing one
/// would drop a row that matches. Such a row is kept as a candidate instead
/// and the in-process engine decides it. `value_json` no longer writes one,
/// but it still reads one back, so this is a live case and not a margin; see
/// `tests/local_query_pushdown_tests.rs`.
///
/// The guard on the query side is for the same value arriving as the operand:
/// `value_json::encode` refuses it, so there is no text to compare against and
/// the filter is simply not pushed down.
fn timestamp_range(
    path: &FieldPath,
    op: &str,
    expected: &Value,
    binder: &mut Binder,
) -> Option<String> {
    let encoded = value_json::encode(expected).ok()?;
    let text = encoded.get("timestampValue")?.as_str()?.to_string();
    if text.len() != CANONICAL_TIMESTAMP_LEN {
        return None;
    }

    let address = binder.address(path);
    let param = binder.bind(text);
    Some(format!(
        "(length({address} ->> 'timestampValue') <> {CANONICAL_TIMESTAMP_LEN} \
         OR ({address} ->> 'timestampValue') COLLATE \"C\" {op} {param})"
    ))
}

/// "A document carries a value at `path`", the narrowing every sort field and
/// every presence test shares.
///
/// A top-level field uses `?`, which the GIN index can serve. A nested path
/// cannot — containment is rooted at the column — so it tests the addressed
/// value for SQL `NULL` instead, which `->` yields whenever any hop is missing
/// or is not a map.
fn presence(path: &FieldPath, binder: &mut Binder) -> String {
    if let [only] = path.segments() {
        let key = binder.bind(only.clone());
        return format!("fields ? {key}");
    }
    let address = binder.address(path);
    format!("{address} IS NOT NULL")
}

fn test(path: &Path, op: Presence, binder: &mut Binder) -> Option<String> {
    // `__name__` is always present and always a reference, so all four tests
    // have a constant answer there; not worth translating.
    let Path::Field(path) = path else {
        return None;
    };

    match op {
        // Exact: the encoder writes a null field as this and nothing else.
        Presence::Null => binder.containment(path, serde_json::json!({"nullValue": null})),
        // Exact: a non-finite double is stored as its proto3 JSON spelling.
        Presence::Nan => {
            let address = binder.address(path);
            Some(format!("{address} ->> 'doubleValue' = 'NaN'"))
        }
        // The presence half of their meaning. A document with no value for the
        // field matches neither, so this is sound; the `not null` / `not NaN`
        // half is left to the engine, where it costs one comparison.
        Presence::NotNull | Presence::NotNan => Some(presence(path, binder)),
    }
}

/// Render a double as text PostgreSQL will read back as the same `f64`.
///
/// `{:?}` is Rust's round-trip spelling; the three non-finite values get
/// PostgreSQL's names for them.
fn render_double(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_string()
    } else if number == f64::INFINITY {
        "Infinity".to_string()
    } else if number == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        format!("{number:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::Timestamp;

    fn integer(number: i64) -> Value {
        Value {
            value_type: Some(ValueType::IntegerValue(number)),
        }
    }

    fn text(value: &str) -> Value {
        Value {
            value_type: Some(ValueType::StringValue(value.to_string())),
        }
    }

    fn double(number: f64) -> Value {
        Value {
            value_type: Some(ValueType::DoubleValue(number)),
        }
    }

    /// A field path, parsed the same way a client's `field_path` is.
    fn path(text: &str) -> FieldPath {
        FieldPath::parse(text).expect("a well-formed field path")
    }

    /// Compile one predicate with no required fields, as the store would.
    fn compiled(filter: &Predicate) -> SqlPrefilter {
        compile(
            Prefilter {
                filter: Some(filter),
                required_fields: &[],
            },
            3,
        )
        .expect("the predicate is pushable")
    }

    fn compare_node(field: &str, op: Comparison, value: Value) -> Predicate {
        Predicate::Compare {
            path: Path::Field(path(field)),
            op,
            value,
        }
    }

    /// The property the whole module rests on: placeholder numbers run from the
    /// caller's offset with no gaps, so what is bound is what is referenced.
    fn assert_placeholders_are_dense(compiled: &SqlPrefilter) {
        for (index, _) in compiled.params.iter().enumerate() {
            let placeholder = format!("${}", index + 3);
            assert!(
                compiled.sql.contains(&placeholder),
                "{placeholder} is bound but never used in {}",
                compiled.sql
            );
        }
        let orphan = format!("${}", compiled.params.len() + 3);
        assert!(
            !compiled.sql.contains(&orphan),
            "{orphan} is used but never bound in {}",
            compiled.sql
        );
    }

    #[test]
    fn an_unfiltered_query_pushes_nothing() {
        assert!(
            compile(
                Prefilter {
                    filter: None,
                    required_fields: &[]
                },
                3
            )
            .is_none(),
            "there is nothing to narrow a scan with"
        );
    }

    #[test]
    fn ordering_by_a_field_requires_the_field_to_be_present() {
        let fields = vec![path("age")];
        let pushed = compile(
            Prefilter {
                filter: None,
                required_fields: &fields,
            },
            3,
        )
        .expect("a sort field is a filter");

        assert_eq!(pushed.sql, "fields ? $3");
        assert_eq!(pushed.params, vec!["age".to_string()]);
    }

    #[test]
    fn scalar_equality_becomes_containment_so_an_index_can_serve_it() {
        let pushed = compiled(&compare_node("city", Comparison::Equal, text("Tokyo")));

        assert_eq!(pushed.sql, "fields @> $3::text::jsonb");
        assert_eq!(
            pushed.params,
            vec![r#"{"city":{"stringValue":"Tokyo"}}"#.to_string()]
        );
    }

    #[test]
    fn numeric_equality_covers_both_storage_tags() {
        let pushed = compiled(&compare_node("age", Comparison::Equal, integer(31)));

        assert!(pushed
            .sql
            .contains("'integerValue')::numeric = $4::text::numeric"));
        assert!(
            pushed
                .sql
                .contains("'doubleValue')::double precision = $5::text::double precision"),
            "a stored 31.0 has to match an integer 31: {}",
            pushed.sql
        );
        assert_eq!(
            pushed.params,
            vec!["age".to_string(), "31".to_string(), "31.0".to_string()]
        );
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn a_double_bound_widens_both_branches_to_f64() {
        let pushed = compiled(&compare_node("age", Comparison::GreaterThan, double(1.5)));

        assert_eq!(
            pushed.sql.matches("::double precision").count(),
            4,
            "the engine widens a stored integer before comparing it: {}",
            pushed.sql
        );
        assert!(!pushed.sql.contains("numeric"), "{}", pushed.sql);
        assert_eq!(
            pushed.params,
            vec!["age".to_string(), "1.5".to_string()],
            "both branches compare against the one bound double"
        );
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn string_ranges_compare_in_byte_order_not_the_database_collation() {
        let pushed = compiled(&compare_node("city", Comparison::LessThan, text("Tokyo")));

        assert_eq!(
            pushed.sql,
            "(fields -> $3 ->> 'stringValue') COLLATE \"C\" < $4"
        );
        assert_eq!(pushed.params, vec!["city".to_string(), "Tokyo".to_string()]);
    }

    #[test]
    fn nan_bounds_match_nothing_rather_than_sorting_somewhere() {
        for op in [
            Comparison::Equal,
            Comparison::LessThan,
            Comparison::GreaterThanOrEqual,
        ] {
            let pushed = compiled(&compare_node("score", op, double(f64::NAN)));
            assert_eq!(pushed.sql, MATCHES_NOTHING, "{op:?}");
            assert!(pushed.params.is_empty(), "{op:?} bound a parameter");
        }
    }

    #[test]
    fn timestamp_ranges_rely_on_the_fixed_width_encoding() {
        let expected = Value {
            value_type: Some(ValueType::TimestampValue(Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            })),
        };
        let encoded = value_json::encode(&expected).expect("encode");
        assert_eq!(
            encoded["timestampValue"].as_str().expect("a string").len(),
            CANONICAL_TIMESTAMP_LEN,
            "the byte-order comparison is only chronological at this width"
        );

        let pushed = compiled(&compare_node("at", Comparison::GreaterThan, expected));
        assert!(
            pushed
                .sql
                .starts_with("(length(fields -> $3 ->> 'timestampValue') <> 30 OR "),
            "a row encoded at another width has to be kept, not compared: {}",
            pushed.sql
        );
        assert_eq!(
            pushed.params,
            vec![
                "at".to_string(),
                "2023-11-14T22:13:20.123456789Z".to_string()
            ]
        );
    }

    #[test]
    fn document_name_filters_use_the_name_column() {
        let reference = Value {
            value_type: Some(ValueType::ReferenceValue(
                "projects/p/databases/(default)/documents/users/bob".to_string(),
            )),
        };
        let pushed = compiled(&Predicate::Compare {
            path: Path::Name,
            op: Comparison::GreaterThan,
            value: reference,
        });

        assert_eq!(pushed.sql, "name COLLATE \"C\" > $3");
    }

    #[test]
    fn a_name_filter_against_another_type_matches_nothing() {
        let pushed = compiled(&Predicate::Compare {
            path: Path::Name,
            op: Comparison::GreaterThan,
            value: text("bob"),
        });
        assert_eq!(pushed.sql, MATCHES_NOTHING);
    }

    #[test]
    fn in_becomes_one_branch_per_candidate() {
        let candidates = Value {
            value_type: Some(ValueType::ArrayValue(
                crate::protos::google::firestore::v1::ArrayValue {
                    values: vec![text("Kyoto"), text("Osaka")],
                },
            )),
        };
        let pushed = compiled(&compare_node("city", Comparison::In, candidates));

        assert_eq!(
            pushed.sql,
            "(fields @> $3::text::jsonb OR fields @> $4::text::jsonb)"
        );
        assert_eq!(pushed.params.len(), 2);
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn an_and_pushes_the_children_it_understands_and_drops_the_rest() {
        let node = Predicate::All(vec![
            compare_node("city", Comparison::Equal, text("Tokyo")),
            // Not pushable, and must not stop the sibling from being pushed.
            compare_node("age", Comparison::NotEqual, integer(31)),
        ]);
        let pushed = compiled(&node);

        assert_eq!(pushed.sql, "(fields @> $3::text::jsonb)");
        assert_eq!(pushed.params.len(), 1, "the dropped child bound nothing");
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn an_or_with_an_unpushable_branch_is_not_pushed_at_all() {
        let node = Predicate::All(vec![
            compare_node("city", Comparison::Equal, text("Tokyo")),
            Predicate::Any(vec![
                compare_node("age", Comparison::Equal, integer(31)),
                compare_node("age", Comparison::NotEqual, integer(0)),
            ]),
        ]);
        let pushed = compiled(&node);

        assert_eq!(
            pushed.sql, "(fields @> $3::text::jsonb)",
            "the OR could match any row, so only the AND's other child survives"
        );
        assert_eq!(
            pushed.params.len(),
            1,
            "the abandoned OR branch must not leave a parameter behind: {:?}",
            pushed.params
        );
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn an_or_of_pushable_branches_is_pushed() {
        let node = Predicate::Any(vec![
            compare_node("city", Comparison::Equal, text("Tokyo")),
            compare_node("city", Comparison::Equal, text("Kyoto")),
        ]);
        let pushed = compiled(&node);

        assert_eq!(
            pushed.sql,
            "(fields @> $3::text::jsonb OR fields @> $4::text::jsonb)"
        );
    }

    #[test]
    fn array_and_map_equality_stay_in_process() {
        let array = Value {
            value_type: Some(ValueType::ArrayValue(
                crate::protos::google::firestore::v1::ArrayValue {
                    values: vec![integer(1)],
                },
            )),
        };
        assert!(
            compile(
                Prefilter {
                    filter: Some(&compare_node("tags", Comparison::Equal, array)),
                    required_fields: &[],
                },
                3
            )
            .is_none(),
            "an integer inside the array must still match a double"
        );
    }

    #[test]
    fn presence_tests_narrow_without_deciding() {
        let null = Predicate::Test {
            path: Path::Field(path("nickname")),
            op: Presence::Null,
        };
        let pushed = compiled(&null);
        assert_eq!(pushed.sql, "fields @> $3::text::jsonb");
        assert_eq!(
            pushed.params,
            vec![r#"{"nickname":{"nullValue":null}}"#.to_string()],
            "an explicit null is a value like any other"
        );

        let not_null = Predicate::Test {
            path: Path::Field(path("nickname")),
            op: Presence::NotNull,
        };
        assert_eq!(compiled(&not_null).sql, "fields ? $3");

        let nan = Predicate::Test {
            path: Path::Field(path("score")),
            op: Presence::Nan,
        };
        assert_eq!(compiled(&nan).sql, "fields -> $3 ->> 'doubleValue' = 'NaN'");
    }

    /// A nested path addresses the stored *map* shape, not a top-level field
    /// whose name happens to contain a dot.
    #[test]
    fn a_nested_path_walks_the_stored_map_shape() {
        let pushed = compiled(&Predicate::Compare {
            path: Path::Field(path("address.city")),
            op: Comparison::LessThan,
            value: text("Tokyo"),
        });

        assert_eq!(
            pushed.sql,
            "(fields -> $3 -> 'mapValue' -> 'fields' -> $4 ->> 'stringValue') COLLATE \"C\" < $5"
        );
        assert_eq!(
            pushed.params,
            vec![
                "address".to_string(),
                "city".to_string(),
                "Tokyo".to_string()
            ],
            "each segment is bound separately; nothing is re-joined with a dot"
        );
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn nested_equality_nests_the_containment_operand_too() {
        let pushed = compiled(&Predicate::Compare {
            path: Path::Field(path("address.city")),
            op: Comparison::Equal,
            value: text("Tokyo"),
        });

        assert_eq!(pushed.sql, "fields @> $3::text::jsonb");
        assert_eq!(
            pushed.params,
            vec![
                r#"{"address":{"mapValue":{"fields":{"city":{"stringValue":"Tokyo"}}}}}"#
                    .to_string()
            ],
            "containment has to describe the whole path down from the column"
        );
    }

    #[test]
    fn ordering_by_a_nested_path_tests_the_addressed_value_for_null() {
        // `?` only works on a top level key, so the nested form asks whether
        // the walk arrived anywhere instead.
        let fields = vec![path("address.city")];
        let pushed = compile(
            Prefilter {
                filter: None,
                required_fields: &fields,
            },
            3,
        )
        .expect("a sort field is a filter");

        assert_eq!(
            pushed.sql,
            "fields -> $3 -> 'mapValue' -> 'fields' -> $4 IS NOT NULL"
        );
        assert_placeholders_are_dense(&pushed);
    }

    #[test]
    fn placeholders_continue_from_the_callers_own_parameters() {
        let node = Predicate::All(vec![
            compare_node("city", Comparison::Equal, text("Tokyo")),
            compare_node("age", Comparison::GreaterThan, integer(30)),
        ]);
        let fields = vec![path("age")];
        let pushed = compile(
            Prefilter {
                filter: Some(&node),
                required_fields: &fields,
            },
            3,
        )
        .expect("pushable");

        assert_eq!(pushed.params.len(), 5);
        assert_placeholders_are_dense(&pushed);
        assert!(
            pushed.sql.starts_with("fields ? $3 AND ("),
            "{}",
            pushed.sql
        );
    }

    #[test]
    fn doubles_are_rendered_so_postgresql_reads_back_the_same_value() {
        for number in [
            0.0,
            -0.5,
            1.5,
            f64::MAX,
            f64::MIN_POSITIVE,
            9_007_199_254_740_993.0,
        ] {
            let rendered = render_double(number);
            assert_eq!(
                rendered.parse::<f64>().expect("parses as a double"),
                number,
                "{rendered} does not round-trip"
            );
        }
        assert_eq!(render_double(f64::INFINITY), "Infinity");
        assert_eq!(render_double(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(render_double(f64::NAN), "NaN");
    }
}
