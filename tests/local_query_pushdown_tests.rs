//! Differential test for the PostgreSQL query pushdown.
//!
//! The pushdown's contract is one-sided: SQL may return extra rows, never drop
//! a matching one. These tests check that property the only way that proves
//! anything — by running every query **both** ways against the same real
//! database and comparing the answers:
//!
//! 1. `scan_collection` then `Query::evaluate` — the whole collection filtered
//!    in-process, which is what the in-memory store does and therefore the
//!    definition of a correct answer.
//! 2. `scan_collection_filtered` then `Query::evaluate` — the same query with
//!    its predicates pushed into SQL.
//!
//! A unit test can only check the SQL text. Only a real server decides whether
//! `COLLATE "C"`, `@>`, and the numeric casts mean what this module assumed —
//! and the default collation of a typical cluster disagrees with Firestore's
//! byte ordering, so getting that wrong would silently drop rows.
//!
//! Needs a database and is skipped when `FIREUP_TEST_POSTGRES_URL` is unset:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql://fireup@127.0.0.1:55432/fireup_test' \
//!   cargo test --test local_query_pushdown_tests
//! ```

use std::collections::HashMap;

use fireup::local::postgres_store::{PostgresStore, DOCUMENTS_TABLE};
use fireup::local::query::{Query, DOCUMENT_NAME_FIELD};
use fireup::local::store::{now_timestamp, Store, StoreError};
use fireup::protos::google::firestore::v1::structured_query::{
    composite_filter::Operator as CompositeOperator, field_filter::Operator as FieldOperator,
    filter::FilterType, unary_filter::OperandType, unary_filter::Operator as UnaryOperator,
    CollectionSelector, CompositeFilter, Direction, FieldFilter, FieldReference, Filter, Order,
    UnaryFilter,
};
use fireup::protos::google::firestore::v1::value::ValueType;
use fireup::protos::google::firestore::v1::{
    ArrayValue, Document, MapValue, StructuredQuery, Value,
};
use fireup::protos::google::r#type::LatLng;
use prost_types::Timestamp;

#[path = "postgres_test_env.rs"]
#[allow(dead_code)]
mod postgres_test_env;

fn test_url() -> Option<String> {
    let url = postgres_test_env::resolve_integration_postgres_url();
    if url.is_none() {
        postgres_test_env::skip_postgres_integration_test("local query pushdown tests");
    }
    url
}

/// A project id unique to this run, so concurrent runs against one database do
/// not see each other's documents.
fn unique_project() -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("pushdown-{}", &suffix[..8])
}

fn value(value_type: ValueType) -> Value {
    Value {
        value_type: Some(value_type),
    }
}

fn text(contents: &str) -> Value {
    value(ValueType::StringValue(contents.to_string()))
}

fn integer(number: i64) -> Value {
    value(ValueType::IntegerValue(number))
}

fn double(number: f64) -> Value {
    value(ValueType::DoubleValue(number))
}

fn timestamp(seconds: i64, nanos: i32) -> Value {
    value(ValueType::TimestampValue(Timestamp { seconds, nanos }))
}

fn array(values: Vec<Value>) -> Value {
    value(ValueType::ArrayValue(ArrayValue { values }))
}

/// The corpus, deliberately full of the cases the translation has to get right
/// rather than a tidy set of rows: mixed integer/double storage of equal
/// numbers, a field that is a string in one document and a number in another,
/// `NaN`, an explicit null next to a missing field, and names and strings whose
/// byte order differs from a locale collation (`Z` before `a`).
fn corpus(root: &str) -> Vec<Document> {
    let document = |id: &str, fields: Vec<(&str, Value)>| Document {
        name: format!("{root}/people/{id}"),
        fields: fields
            .into_iter()
            .map(|(field, value)| (field.to_string(), value))
            .collect(),
        create_time: None,
        update_time: None,
    };

    vec![
        document(
            "Zoe",
            vec![
                ("age", integer(31)),
                ("city", text("Tokyo")),
                ("joined", timestamp(1_700_000_000, 123_456_789)),
                ("tags", array(vec![text("x"), text("y")])),
                ("nickname", value(ValueType::NullValue(0))),
            ],
        ),
        document(
            "alice",
            vec![
                // Stored as a double, equal to Zoe's integer 31.
                ("age", double(31.0)),
                ("city", text("Kyoto")),
                ("joined", timestamp(1_600_000_000, 0)),
                ("active", value(ValueType::BooleanValue(true))),
            ],
        ),
        document(
            "bob",
            vec![
                ("age", integer(41)),
                ("city", text("Osaka")),
                ("joined", timestamp(1_800_000_000, 1)),
                ("active", value(ValueType::BooleanValue(false))),
                (
                    "home",
                    value(ValueType::GeoPointValue(LatLng {
                        latitude: 34.6937,
                        longitude: 135.5023,
                    })),
                ),
            ],
        ),
        document(
            "carol",
            vec![
                // A string where every other document has a number: an
                // inequality must not match across types.
                ("age", text("ancient")),
                ("city", text("Tokyo")),
            ],
        ),
        document(
            "dan",
            vec![
                ("score", double(f64::NAN)),
                ("city", text("Nara")),
                ("age", integer(-5)),
            ],
        ),
        document(
            "erin",
            vec![
                ("score", double(1.5)),
                // Past 2^53, where `f64` and `i64` stop agreeing.
                ("ticket", integer(9_007_199_254_740_993)),
                ("city", text("Zurich")),
            ],
        ),
        document(
            "frank",
            vec![
                ("city", text("tokyo")),
                (
                    "profile",
                    value(ValueType::MapValue(MapValue {
                        fields: HashMap::from([
                            ("plan".to_string(), text("pro")),
                            ("level".to_string(), integer(2)),
                        ]),
                    })),
                ),
                ("secret", value(ValueType::BytesValue(vec![1, 2, 255]))),
            ],
        ),
        // Nested-path material. `profile.plan` has to tell these three apart:
        // a different plan, a `profile` map with no `plan` at all, and a
        // `profile` that is not a map — field paths walk only into maps, so the
        // last one has no value at `profile.plan` rather than an odd one.
        document(
            "ivan",
            vec![
                ("city", text("Kobe")),
                (
                    "profile",
                    value(ValueType::MapValue(MapValue {
                        fields: HashMap::from([
                            ("plan".to_string(), text("free")),
                            // A double where frank has an integer.
                            ("level".to_string(), double(2.0)),
                        ]),
                    })),
                ),
            ],
        ),
        document(
            "judy",
            vec![
                ("city", text("Naha")),
                (
                    "profile",
                    value(ValueType::MapValue(MapValue {
                        fields: HashMap::from([("nickname".to_string(), text("jj"))]),
                    })),
                ),
            ],
        ),
        document(
            "ken",
            vec![("city", text("Nagoya")), ("profile", text("pro"))],
        ),
        // A timestamp outside years 0001–9999 is the one value whose encoding
        // is not the fixed width the byte-order comparison relies on. It is
        // not here because the store refuses to write one (SAK-205) and
        // `MemoryStore` shares this corpus; it is inserted as raw SQL by
        // `seed_legacy_wide_timestamp`, which is how such a row really gets
        // into a database.

        // No fields at all beyond the one: every filter and every order has to
        // leave this one out.
        document("grace", vec![("city", text("Sapporo"))]),
    ]
}

fn field(path: &str) -> Option<FieldReference> {
    Some(FieldReference {
        field_path: path.to_string(),
    })
}

fn field_filter(path: &str, op: FieldOperator, value: Value) -> Option<Filter> {
    Some(Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: field(path),
            op: op as i32,
            value: Some(value),
        })),
    })
}

fn unary_filter(path: &str, op: UnaryOperator) -> Option<Filter> {
    Some(Filter {
        filter_type: Some(FilterType::UnaryFilter(UnaryFilter {
            op: op as i32,
            operand_type: Some(OperandType::Field(FieldReference {
                field_path: path.to_string(),
            })),
        })),
    })
}

fn composite(op: CompositeOperator, filters: Vec<Filter>) -> Option<Filter> {
    Some(Filter {
        filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
            op: op as i32,
            filters,
        })),
    })
}

fn query(mutate: impl FnOnce(&mut StructuredQuery)) -> StructuredQuery {
    let mut structured = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "people".to_string(),
            all_descendants: false,
        }],
        ..Default::default()
    };
    mutate(&mut structured);
    structured
}

/// Every query shape worth checking, named so a failure says which one broke.
///
/// `root` is needed because a `__name__` filter compares against a full
/// resource name.
fn cases(root: &str) -> Vec<(&'static str, StructuredQuery)> {
    let reference = |id: &str| value(ValueType::ReferenceValue(format!("{root}/people/{id}")));

    vec![
        ("no filter at all", query(|_| {})),
        (
            "equality on a string",
            query(|q| q.r#where = field_filter("city", FieldOperator::Equal, text("Tokyo"))),
        ),
        (
            "equality on a string that only differs by case",
            query(|q| q.r#where = field_filter("city", FieldOperator::Equal, text("tokyo"))),
        ),
        (
            "an integer bound matches a stored double of the same value",
            query(|q| q.r#where = field_filter("age", FieldOperator::Equal, integer(31))),
        ),
        (
            "a double bound matches a stored integer of the same value",
            query(|q| q.r#where = field_filter("age", FieldOperator::Equal, double(31.0))),
        ),
        (
            "a numeric inequality ignores a string of the same field",
            query(|q| q.r#where = field_filter("age", FieldOperator::GreaterThan, integer(31))),
        ),
        (
            "a numeric inequality over a negative bound",
            query(|q| {
                q.r#where = field_filter("age", FieldOperator::GreaterThanOrEqual, integer(-5))
            }),
        ),
        (
            "an integer bound past 2^53",
            query(|q| {
                q.r#where = field_filter(
                    "ticket",
                    FieldOperator::GreaterThan,
                    integer(9_007_199_254_740_992),
                )
            }),
        ),
        (
            "a string range",
            query(|q| q.r#where = field_filter("city", FieldOperator::LessThan, text("Nara"))),
        ),
        (
            "a string range over the whole alphabet",
            query(|q| {
                q.r#where = field_filter("city", FieldOperator::GreaterThanOrEqual, text("T"))
            }),
        ),
        // A lowercase bound is where byte order and a locale collation part
        // company: `"C"` puts every capitalised city before `"b"`, while
        // `en_US.UTF-8` compares letters first and puts none of them there. A
        // comparison that forgot `COLLATE "C"` passes every other case in this
        // list and fails this one.
        (
            "a string range whose bound sorts differently under a locale collation",
            query(|q| q.r#where = field_filter("city", FieldOperator::LessThan, text("b"))),
        ),
        (
            "a string range whose bound is above every lowercase city",
            query(|q| q.r#where = field_filter("city", FieldOperator::GreaterThan, text("Z"))),
        ),
        (
            "a timestamp range",
            query(|q| {
                q.r#where = field_filter(
                    "joined",
                    FieldOperator::GreaterThan,
                    timestamp(1_650_000_000, 0),
                )
            }),
        ),
        (
            "a timestamp range that splits a nanosecond",
            query(|q| {
                q.r#where = field_filter(
                    "joined",
                    FieldOperator::GreaterThanOrEqual,
                    timestamp(1_800_000_000, 1),
                )
            }),
        ),
        (
            "a boolean equality",
            query(|q| {
                q.r#where = field_filter(
                    "active",
                    FieldOperator::Equal,
                    value(ValueType::BooleanValue(false)),
                )
            }),
        ),
        (
            "a boolean range",
            query(|q| {
                q.r#where = field_filter(
                    "active",
                    FieldOperator::GreaterThan,
                    value(ValueType::BooleanValue(false)),
                )
            }),
        ),
        (
            "bytes equality",
            query(|q| {
                q.r#where = field_filter(
                    "secret",
                    FieldOperator::Equal,
                    value(ValueType::BytesValue(vec![1, 2, 255])),
                )
            }),
        ),
        (
            "geo point equality",
            query(|q| {
                q.r#where = field_filter(
                    "home",
                    FieldOperator::Equal,
                    value(ValueType::GeoPointValue(LatLng {
                        latitude: 34.6937,
                        longitude: 135.5023,
                    })),
                )
            }),
        ),
        (
            "IN over strings",
            query(|q| {
                q.r#where = field_filter(
                    "city",
                    FieldOperator::In,
                    array(vec![text("Osaka"), text("Nara")]),
                )
            }),
        ),
        (
            "IN over numbers stored under both tags",
            query(|q| {
                q.r#where = field_filter(
                    "age",
                    FieldOperator::In,
                    array(vec![integer(31), integer(41)]),
                )
            }),
        ),
        (
            "IN over an empty candidate list",
            query(|q| q.r#where = field_filter("city", FieldOperator::In, array(vec![]))),
        ),
        (
            "equality against NaN, which never matches",
            query(|q| q.r#where = field_filter("score", FieldOperator::Equal, double(f64::NAN))),
        ),
        (
            "a range against NaN, which is ordered against nothing",
            query(|q| q.r#where = field_filter("score", FieldOperator::LessThan, double(f64::NAN))),
        ),
        (
            "IS NAN",
            query(|q| q.r#where = unary_filter("score", UnaryOperator::IsNan)),
        ),
        (
            "IS NOT NAN",
            query(|q| q.r#where = unary_filter("score", UnaryOperator::IsNotNan)),
        ),
        (
            "IS NULL distinguishes an explicit null from a missing field",
            query(|q| q.r#where = unary_filter("nickname", UnaryOperator::IsNull)),
        ),
        (
            "IS NOT NULL",
            query(|q| q.r#where = unary_filter("city", UnaryOperator::IsNotNull)),
        ),
        (
            "__name__ equality",
            query(|q| {
                q.r#where =
                    field_filter(DOCUMENT_NAME_FIELD, FieldOperator::Equal, reference("bob"))
            }),
        ),
        (
            "__name__ range upwards",
            query(|q| {
                q.r#where = field_filter(
                    DOCUMENT_NAME_FIELD,
                    FieldOperator::GreaterThan,
                    reference("bob"),
                )
            }),
        ),
        // The direction that needs `COLLATE "C"` on the name column: byte
        // order puts `Zoe` before `bob`, a locale collation puts it after, and
        // only the first of those keeps `Zoe` in the result.
        (
            "__name__ range downwards, where byte order puts Zoe before bob",
            query(|q| {
                q.r#where = field_filter(
                    DOCUMENT_NAME_FIELD,
                    FieldOperator::LessThan,
                    reference("bob"),
                )
            }),
        ),
        (
            "array-contains, which is not pushed down",
            query(|q| q.r#where = field_filter("tags", FieldOperator::ArrayContains, text("x"))),
        ),
        (
            "an array equality, which is not pushed down",
            query(|q| {
                q.r#where = field_filter(
                    "tags",
                    FieldOperator::Equal,
                    array(vec![text("x"), text("y")]),
                )
            }),
        ),
        (
            "a map equality, which is not pushed down",
            query(|q| {
                q.r#where = field_filter(
                    "profile",
                    FieldOperator::Equal,
                    value(ValueType::MapValue(MapValue {
                        fields: HashMap::from([("plan".to_string(), text("pro"))]),
                    })),
                )
            }),
        ),
        (
            "not-equal, which is not pushed down",
            query(|q| q.r#where = field_filter("city", FieldOperator::NotEqual, text("Tokyo"))),
        ),
        (
            "NOT IN, which is not pushed down",
            query(|q| {
                q.r#where = field_filter("city", FieldOperator::NotIn, array(vec![text("Tokyo")]))
            }),
        ),
        (
            "AND of two pushable predicates",
            query(|q| {
                q.r#where = composite(
                    CompositeOperator::And,
                    vec![
                        field_filter("city", FieldOperator::Equal, text("Tokyo")).expect("filter"),
                        field_filter("age", FieldOperator::GreaterThanOrEqual, integer(31))
                            .expect("filter"),
                    ],
                )
            }),
        ),
        (
            "AND where only one side can be pushed",
            query(|q| {
                q.r#where = composite(
                    CompositeOperator::And,
                    vec![
                        field_filter("city", FieldOperator::Equal, text("Tokyo")).expect("filter"),
                        field_filter("age", FieldOperator::NotEqual, integer(31)).expect("filter"),
                    ],
                )
            }),
        ),
        (
            "OR of two pushable predicates",
            query(|q| {
                q.r#where = composite(
                    CompositeOperator::Or,
                    vec![
                        field_filter("city", FieldOperator::Equal, text("Nara")).expect("filter"),
                        field_filter("age", FieldOperator::Equal, integer(41)).expect("filter"),
                    ],
                )
            }),
        ),
        (
            "OR with one unpushable branch keeps every row a candidate",
            query(|q| {
                q.r#where = composite(
                    CompositeOperator::Or,
                    vec![
                        field_filter("city", FieldOperator::Equal, text("Nara")).expect("filter"),
                        field_filter("tags", FieldOperator::ArrayContains, text("x"))
                            .expect("filter"),
                    ],
                )
            }),
        ),
        (
            "ordering by a field excludes documents that lack it",
            query(|q| {
                q.order_by = vec![Order {
                    field: field("age"),
                    direction: Direction::Ascending as i32,
                }]
            }),
        ),
        (
            "ordering descending with a filter",
            query(|q| {
                q.r#where = field_filter("city", FieldOperator::GreaterThan, text("A"));
                q.order_by = vec![Order {
                    field: field("city"),
                    direction: Direction::Descending as i32,
                }];
            }),
        ),
        // Nested field paths, served since SAK-191. These are the cases a
        // translation that re-joined the segments into `"profile.plan"` would
        // get wrong — it would match nothing and drop every row.
        (
            "equality on a nested path",
            query(|q| q.r#where = field_filter("profile.plan", FieldOperator::Equal, text("pro"))),
        ),
        (
            "a range on a nested path",
            query(|q| q.r#where = field_filter("profile.plan", FieldOperator::LessThan, text("g"))),
        ),
        (
            "a numeric range on a nested path, across both storage tags",
            query(|q| {
                q.r#where = field_filter(
                    "profile.level",
                    FieldOperator::GreaterThanOrEqual,
                    integer(2),
                )
            }),
        ),
        (
            "IS NULL on a nested path",
            query(|q| q.r#where = unary_filter("profile.plan", UnaryOperator::IsNull)),
        ),
        (
            "IS NOT NULL on a nested path",
            query(|q| q.r#where = unary_filter("profile.plan", UnaryOperator::IsNotNull)),
        ),
        (
            "ordering by a nested path excludes documents that lack it",
            query(|q| {
                q.order_by = vec![Order {
                    field: field("profile.plan"),
                    direction: Direction::Ascending as i32,
                }]
            }),
        ),
        (
            "a nested path combined with a top-level filter",
            query(|q| {
                q.r#where = composite(
                    CompositeOperator::And,
                    vec![
                        field_filter("profile.plan", FieldOperator::Equal, text("pro"))
                            .expect("filter"),
                        field_filter("city", FieldOperator::GreaterThan, text("A"))
                            .expect("filter"),
                    ],
                )
            }),
        ),
        (
            "limit and offset over a filtered, ordered query",
            query(|q| {
                q.r#where = field_filter("age", FieldOperator::LessThan, integer(100));
                q.order_by = vec![Order {
                    field: field("age"),
                    direction: Direction::Ascending as i32,
                }];
                q.offset = 1;
                q.limit = Some(2);
            }),
        ),
    ]
}

async fn connect(url: &str) -> PostgresStore {
    PostgresStore::connect(url, 4)
        .await
        .expect("connect to the test database and create the schema")
}

async fn remove_project(url: &str, project: &str) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("connect for cleanup");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            &format!("DELETE FROM {DOCUMENTS_TABLE} WHERE project_id = $1"),
            &[&project],
        )
        .await
        .expect("delete this test's rows");
}

/// Document id of the row [`seed_legacy_wide_timestamp`] writes.
const LEGACY_ROW_ID: &str = "mallory";

/// Seconds of the timestamp that row carries: `+11476-08-15T05:20:00Z`.
const LEGACY_ROW_SECONDS: i64 = 300_000_000_000;

/// Add the one row the store can read but no longer write: a `joined` outside
/// years 0001–9999, spelled the way a pre-SAK-205 build spelled it.
///
/// Inserted as raw SQL because `value_json::encode` now refuses that value —
/// which is also its real provenance, a database an older build wrote. It is
/// what makes `postgres_filter`'s width guard a live case instead of a
/// margin: `+11476-…` sorts *before* `2023-…` byte-wise, so a timestamp range
/// that has to include year 11476 silently loses this row unless the guard
/// keeps it out of the text comparison and lets the in-process engine decide.
async fn seed_legacy_wide_timestamp(url: &str, project: &str, root: &str) {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("connect to insert the legacy row");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let name = format!("{root}/people/{LEGACY_ROW_ID}");
    let fields = r#"{
        "city": {"stringValue": "Legacy"},
        "joined": {"timestampValue": "+11476-08-15T05:20:00.000000000Z"}
    }"#;

    client
        .execute(
            &format!(
                "INSERT INTO {DOCUMENTS_TABLE} (
                     name, project_id, database_id, parent, collection_id, document_id,
                     fields, create_time_seconds, create_time_nanos,
                     update_time_seconds, update_time_nanos
                 ) VALUES ($1, $2, '(default)', $3, 'people', $4, $5::text::jsonb, 0, 0, 0, 0)"
            ),
            &[&name, &project, &root, &LEGACY_ROW_ID, &fields],
        )
        .await
        .expect("insert the legacy row");
}

fn ids(documents: &[Document]) -> Vec<String> {
    documents
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

/// Pushing predicates into SQL must not change a single answer.
///
/// This is the test the whole feature rests on: for every query shape, the
/// pushed-down path and the scan-everything path have to agree exactly,
/// including order, paging, and which documents are absent.
#[tokio::test]
async fn pushdown_answers_every_query_exactly_as_a_full_scan_does() {
    let Some(url) = test_url() else { return };
    let project = unique_project();
    let root = format!("projects/{project}/databases/(default)/documents");
    let store = connect(&url).await;

    for document in corpus(&root) {
        store
            .put(document, now_timestamp())
            .await
            .expect("seed a document");
    }
    // Every query shape also has to agree in the presence of a row whose
    // timestamp is not the canonical width, which is the case the guard in
    // `postgres_filter::timestamp_range` exists for.
    seed_legacy_wide_timestamp(&url, &project, &root).await;

    for (name, structured) in cases(&root) {
        let compiled = Query::compile(&structured).expect("the case compiles");

        let scanned = store
            .scan_collection(&root, compiled.collection_id())
            .await
            .expect("full scan");
        let pushed = store
            .scan_collection_filtered(&root, compiled.collection_id(), compiled.prefilter())
            .await
            .expect("pushed-down scan");

        // The soundness property, stated directly on the candidate sets: SQL
        // may return fewer rows, but never one the in-process filter wanted.
        let scanned_results = compiled.evaluate(scanned);
        let pushed_results = compiled.evaluate(pushed);

        assert_eq!(
            ids(&pushed_results.documents),
            ids(&scanned_results.documents),
            "pushdown changed the result of {name:?}"
        );
        assert_eq!(
            pushed_results.skipped, scanned_results.skipped,
            "pushdown changed the skipped count of {name:?}"
        );
    }

    remove_project(&url, &project).await;
}

/// A row a pre-SAK-205 build wrote stays readable, and stays out of the text
/// comparison rather than being compared wrongly.
///
/// Three separate things, all of which that one row used to break:
///
/// 1. It decodes, so scanning the collection it sits in works at all. One
///    such row used to fail every read of the whole collection.
/// 2. The store refuses to write the same value back, as `INVALID_ARGUMENT` —
///    the write is where an unstorable timestamp has to fail.
/// 3. A timestamp range that has to include it does. This is the live case
///    for the width guard: `+11476-…` is byte-wise *below* `2023-…`, so a
///    `>=` against 2022 would drop the row if SQL compared it as text.
#[tokio::test]
async fn a_row_a_pre_sak_205_build_wrote_reads_back_and_is_never_compared_as_text() {
    let Some(url) = test_url() else { return };
    let project = unique_project();
    let root = format!("projects/{project}/databases/(default)/documents");
    let store = connect(&url).await;

    for document in corpus(&root) {
        store
            .put(document, now_timestamp())
            .await
            .expect("seed a document");
    }
    seed_legacy_wide_timestamp(&url, &project, &root).await;

    // 1. The collection reads, and the value comes back exactly.
    let scanned = store
        .scan_collection(&root, "people")
        .await
        .expect("a collection holding a legacy timestamp still scans");
    let legacy = scanned
        .iter()
        .find(|document| document.name.ends_with(LEGACY_ROW_ID))
        .expect("the legacy row is in the scan")
        .clone();
    assert_eq!(
        legacy.fields.get("joined"),
        Some(&timestamp(LEGACY_ROW_SECONDS, 0)),
        "the legacy timestamp did not survive the read"
    );

    // 2. Writing that same document back is refused, not stored.
    let error = store
        .put(legacy.clone(), now_timestamp())
        .await
        .expect_err("an out-of-range timestamp must fail the write");
    assert!(
        matches!(error, StoreError::InvalidDocument(_)),
        "wrong error for an out-of-range timestamp: {error}"
    );
    assert_eq!(
        tonic::Status::from(error).code(),
        tonic::Code::InvalidArgument,
        "an SDK has to see this as its own bad argument"
    );

    // 3. The guard keeps the row as a candidate for a range in either
    //    direction, and the in-process engine is what decides it.
    for (name, operator, expected) in [
        (
            "a range the legacy row matches",
            FieldOperator::GreaterThanOrEqual,
            true,
        ),
        (
            "a range the legacy row misses",
            FieldOperator::LessThan,
            false,
        ),
    ] {
        let structured =
            query(|q| q.r#where = field_filter("joined", operator, timestamp(1_650_000_000, 0)));
        let compiled = Query::compile(&structured).expect("compiles");

        let candidates = store
            .scan_collection_filtered(&root, compiled.collection_id(), compiled.prefilter())
            .await
            .expect("pushed-down scan");
        assert!(
            ids(&candidates).contains(&LEGACY_ROW_ID.to_string()),
            "{name}: the width guard let SQL compare a wide timestamp as text"
        );

        let matched =
            ids(&compiled.evaluate(candidates).documents).contains(&LEGACY_ROW_ID.to_string());
        assert_eq!(matched, expected, "{name}: wrong answer for the legacy row");
    }

    remove_project(&url, &project).await;
}

/// The point of the feature: a selective query has to actually read less.
///
/// Without this the test above would still pass against a pushdown that
/// translated nothing, which is the failure mode a correctness-only test
/// cannot see.
#[tokio::test]
async fn a_selective_query_reads_fewer_rows_than_the_collection_holds() {
    let Some(url) = test_url() else { return };
    let project = unique_project();
    let root = format!("projects/{project}/databases/(default)/documents");
    let store = connect(&url).await;

    let corpus = corpus(&root);
    let collection_size = corpus.len();
    for document in corpus {
        store
            .put(document, now_timestamp())
            .await
            .expect("seed a document");
    }

    // One row of eight matches, and the filter is a plain equality — exactly
    // the shape `fields @> …` and the GIN index exist for.
    let structured =
        query(|q| q.r#where = field_filter("city", FieldOperator::Equal, text("Nara")));
    let compiled = Query::compile(&structured).expect("compiles");

    let candidates = store
        .scan_collection_filtered(&root, compiled.collection_id(), compiled.prefilter())
        .await
        .expect("pushed-down scan");

    assert_eq!(
        ids(&candidates),
        ["dan"],
        "an equality filter should come back from SQL already narrowed"
    );
    assert!(
        candidates.len() < collection_size,
        "the filter was not pushed down at all"
    );

    remove_project(&url, &project).await;
}

/// The collection group read gets the same treatment, over the same filter.
///
/// `scan_kind_filtered` is a second SQL statement with the same appended
/// fragment but a wider row selector, so it needs its own agreement check —
/// and it is the read that benefits most, spanning every matching collection
/// under the root rather than one.
#[tokio::test]
async fn a_collection_group_read_agrees_with_its_full_scan_too() {
    let Some(url) = test_url() else { return };
    let project = unique_project();
    let root = format!("projects/{project}/databases/(default)/documents");
    let store = connect(&url).await;

    // The same documents, but scattered across `people` collections at three
    // different depths, which is what a collection group query spans.
    for (index, document) in corpus(&root).into_iter().enumerate() {
        let id = document
            .name
            .rsplit('/')
            .next()
            .expect("a name has segments")
            .to_string();
        let name = match index % 3 {
            0 => format!("{root}/people/{id}"),
            1 => format!("{root}/orgs/acme/people/{id}"),
            _ => format!("{root}/orgs/acme/teams/core/people/{id}"),
        };
        store
            .put(Document { name, ..document }, now_timestamp())
            .await
            .expect("seed a document");
    }

    for (name, mut structured) in cases(&root) {
        structured.from[0].all_descendants = true;
        let Ok(compiled) = Query::compile(&structured) else {
            // A `__name__` case compiles against the flat layout only; skip
            // rather than assert on a shape this test did not set up.
            continue;
        };

        let scanned = store
            .scan_kind(&root, compiled.collection_id())
            .await
            .expect("full kind scan");
        let pushed = store
            .scan_kind_filtered(&root, compiled.collection_id(), compiled.prefilter())
            .await
            .expect("pushed-down kind scan");

        assert_eq!(
            ids(&compiled.evaluate(pushed).documents),
            ids(&compiled.evaluate(scanned).documents),
            "pushdown changed the collection group result of {name:?}"
        );
    }

    remove_project(&url, &project).await;
}

/// `MemoryStore` must keep ignoring the pre-filter, because the in-process
/// engine is what CI tests and what the two backends are held to.
#[tokio::test]
async fn the_in_memory_store_ignores_the_prefilter() {
    use fireup::local::store::MemoryStore;

    let root = "projects/demo/databases/(default)/documents";
    let store = MemoryStore::new();
    for document in corpus(root) {
        store
            .put(document, now_timestamp())
            .await
            .expect("seed a document");
    }

    let structured =
        query(|q| q.r#where = field_filter("city", FieldOperator::Equal, text("Nara")));
    let compiled = Query::compile(&structured).expect("compiles");

    let candidates = store
        .scan_collection_filtered(root, compiled.collection_id(), compiled.prefilter())
        .await
        .expect("scan");

    assert_eq!(
        candidates.len(),
        corpus(root).len(),
        "the default implementation hands back the whole collection"
    );
    assert_eq!(
        ids(&compiled.evaluate(candidates).documents),
        ["dan"],
        "and the engine still answers the query"
    );
}
