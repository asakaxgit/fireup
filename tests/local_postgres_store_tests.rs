//! `fireup local` against a real PostgreSQL store.
//!
//! These cover the Phase 1 exit criterion the in-memory tests cannot: that a
//! document written through the Firestore API is still there after the process
//! that wrote it has gone away, byte for byte.
//!
//! The tests need a reachable database and are skipped when
//! `FIREUP_TEST_POSTGRES_URL` is unset, for example:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql://fireup:pw@127.0.0.1:5433/fireup_dev' \
//!   cargo test --test local_postgres_store_tests
//! ```
//!
//! The URL goes through the shared `postgres_test_env` guard, which refuses the
//! host Paperclip role/database outright and never falls back to a default
//! connection. See `PROJECT_STATUS.md` for the throwaway-Postgres setup.
//!
//! Each test uses its own generated project id, so its documents are disjoint
//! from every other test's and concurrent runs against one database do not
//! collide. Rows are deleted afterwards.

use std::io::Write;
use std::sync::Arc;

use fireup::local::postgres_store::{PostgresStore, DOCUMENTS_TABLE};
use fireup::local::store::{conformance, SharedStore};
use fireup::local::{self, LocalConfig};
use fireup::protos::google::firestore::v1::run_query_request::QueryType;
use fireup::protos::google::firestore::v1::structured_query::{
    field_filter::Operator as FieldOperator, filter::FilterType, CollectionSelector, Direction,
    FieldFilter, FieldReference, Filter, Order,
};
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient, value::ValueType, ArrayValue, CreateDocumentRequest,
    Document, GetDocumentRequest, MapValue, RunQueryRequest, StructuredQuery, Value,
};
use tonic::transport::Channel;

#[path = "postgres_test_env.rs"]
#[allow(dead_code)]
mod postgres_test_env;

fn test_url() -> Option<String> {
    let url = postgres_test_env::resolve_integration_postgres_url();
    if url.is_none() {
        postgres_test_env::skip_postgres_integration_test("local PostgreSQL store tests");
    }
    url
}

/// A project id unique to this test, so documents cannot collide.
fn unique_project(prefix: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

async fn store(url: &str) -> SharedStore {
    Arc::new(
        PostgresStore::connect(url, 4)
            .await
            .expect("connect to the test database and create the schema"),
    )
}

/// Remove every document belonging to `project_id`.
async fn cleanup(url: &str, project_id: &str) {
    let config = fireup::data_importer::parse_connection_url(url, 1, 10).expect("parse test URL");
    let mut pg_config = deadpool_postgres::Config::new();
    pg_config.host = Some(config.host);
    pg_config.port = Some(config.port);
    pg_config.dbname = Some(config.database);
    pg_config.user = Some(config.user.clone());
    if !config.password.is_empty() {
        pg_config.password = Some(config.password);
    }

    let pool = pg_config
        .create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls)
        .expect("create cleanup pool");
    let client = pool.get().await.expect("cleanup connection");
    client
        .execute(
            &format!("DELETE FROM {DOCUMENTS_TABLE} WHERE project_id = $1"),
            &[&project_id],
        )
        .await
        .expect("delete this test's documents");
}

/// Start a server backed by `url` and return a connected Firestore client.
async fn start(url: &str, project_id: &str) -> (local::RunningServer, FirestoreClient<Channel>) {
    let config = LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_postgres(url)
        .with_project_id(project_id);
    let server = local::spawn(config).await.expect("server binds and connects");
    let client = FirestoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects");
    (server, client)
}

#[tokio::test]
async fn postgres_store_satisfies_the_same_contract_as_the_memory_store() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("contract");

    let store = store(&url).await;
    assert_eq!(store.backend(), "postgres");
    conformance::assert_store_contract(
        store.as_ref(),
        &format!("projects/{project_id}/databases/(default)/documents"),
    )
    .await;

    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn documents_survive_a_restart() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("restart");
    let root = format!("projects/{project_id}/databases/(default)/documents");

    // Write through the Firestore API, then stop the server entirely.
    let (first, mut client) = start(&url, &project_id).await;
    let created = client
        .create_document(CreateDocumentRequest {
            parent: root.clone(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                fields: [("email".to_string(), string_value("alice@example.com"))]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create succeeds")
        .into_inner();
    first.shutdown().await.expect("clean shutdown");

    // A brand-new process, pointed at the same database, must see it.
    let (second, mut client) = start(&url, &project_id).await;
    let found = client
        .get_document(GetDocumentRequest {
            name: created.name.clone(),
            ..Default::default()
        })
        .await
        .expect("the document is still there after a restart")
        .into_inner();

    assert_eq!(found.name, created.name);
    assert_eq!(found.fields, created.fields);
    assert_eq!(
        found.create_time, created.create_time,
        "create_time must survive a restart exactly: it is a client-visible value"
    );
    assert_eq!(
        found.update_time, created.update_time,
        "update_time must survive exactly — clients use it as a write precondition"
    );

    second.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn every_value_type_survives_a_restart_unchanged() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("values");
    let root = format!("projects/{project_id}/databases/(default)/documents");

    let fields: std::collections::HashMap<String, Value> = [
        ("null".to_string(), value(ValueType::NullValue(0))),
        ("bool".to_string(), value(ValueType::BooleanValue(true))),
        // Past 2^53: this is the value a JSON-number encoding would corrupt.
        (
            "big_int".to_string(),
            value(ValueType::IntegerValue(9_007_199_254_740_993)),
        ),
        ("double".to_string(), value(ValueType::DoubleValue(-0.125))),
        (
            "timestamp".to_string(),
            value(ValueType::TimestampValue(prost_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            })),
        ),
        ("string".to_string(), string_value("日本語 🎏")),
        (
            "bytes".to_string(),
            value(ValueType::BytesValue(vec![0, 1, 254, 255])),
        ),
        (
            "reference".to_string(),
            value(ValueType::ReferenceValue(format!("{root}/users/bob"))),
        ),
        (
            "geo".to_string(),
            value(ValueType::GeoPointValue(
                fireup::protos::google::r#type::LatLng {
                    latitude: 35.6812,
                    longitude: 139.7671,
                },
            )),
        ),
        (
            "array".to_string(),
            value(ValueType::ArrayValue(ArrayValue {
                values: vec![string_value("a"), value(ValueType::IntegerValue(2))],
            })),
        ),
        (
            "map".to_string(),
            value(ValueType::MapValue(MapValue {
                fields: [("inner".to_string(), string_value("x"))]
                    .into_iter()
                    .collect(),
            })),
        ),
    ]
    .into_iter()
    .collect();

    let (first, mut client) = start(&url, &project_id).await;
    let created = client
        .create_document(CreateDocumentRequest {
            parent: root.clone(),
            collection_id: "kitchen".to_string(),
            document_id: "sink".to_string(),
            document: Some(Document {
                fields: fields.clone(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create succeeds")
        .into_inner();
    assert_eq!(created.fields, fields, "the response must echo what was stored");
    first.shutdown().await.expect("clean shutdown");

    let (second, mut client) = start(&url, &project_id).await;
    let found = client
        .get_document(GetDocumentRequest {
            name: created.name.clone(),
            ..Default::default()
        })
        .await
        .expect("read back after restart")
        .into_inner();

    assert_eq!(
        found.fields, fields,
        "every value type must come back from PostgreSQL unchanged"
    );

    second.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn documents_are_stored_in_the_documented_jsonb_shape() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("shape");
    let root = format!("projects/{project_id}/databases/(default)/documents");

    let store = store(&url).await;
    store
        .put(
            Document {
                name: format!("{root}/users/alice/posts/p1"),
                fields: [
                    ("title".to_string(), string_value("hello")),
                    ("rank".to_string(), value(ValueType::IntegerValue(7))),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            prost_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 42,
            },
        )
        .await
        .expect("put");

    // Read the row with plain SQL: the point is that the data at rest is the
    // documented shape, which is what the query engine will later rely on.
    let config = fireup::data_importer::parse_connection_url(&url, 1, 10).expect("parse URL");
    let mut pg_config = deadpool_postgres::Config::new();
    pg_config.host = Some(config.host);
    pg_config.port = Some(config.port);
    pg_config.dbname = Some(config.database);
    pg_config.user = Some(config.user.clone());
    if !config.password.is_empty() {
        pg_config.password = Some(config.password);
    }
    let pool = pg_config
        .create_pool(Some(deadpool_postgres::Runtime::Tokio1), tokio_postgres::NoTls)
        .expect("pool");
    let client = pool.get().await.expect("connection");

    let row = client
        .query_one(
            &format!(
                "SELECT project_id, database_id, parent, collection_id, document_id,
                        fields, create_time_seconds, create_time_nanos,
                        update_time_seconds, update_time_nanos
                   FROM {DOCUMENTS_TABLE} WHERE name = $1"
            ),
            &[&format!("{root}/users/alice/posts/p1")],
        )
        .await
        .expect("the row exists");

    assert_eq!(row.get::<_, String>("project_id"), project_id);
    assert_eq!(row.get::<_, String>("database_id"), "(default)");
    assert_eq!(row.get::<_, String>("parent"), format!("{root}/users/alice"));
    assert_eq!(row.get::<_, String>("collection_id"), "posts");
    assert_eq!(row.get::<_, String>("document_id"), "p1");
    assert_eq!(row.get::<_, i64>("create_time_seconds"), 1_700_000_000);
    assert_eq!(row.get::<_, i32>("create_time_nanos"), 42, "nanos must not be rounded");
    assert_eq!(row.get::<_, i64>("update_time_seconds"), 1_700_000_000);
    assert_eq!(row.get::<_, i32>("update_time_nanos"), 42);

    let payload: serde_json::Value = row.get("fields");
    assert_eq!(
        payload,
        serde_json::json!({
            "title": {"stringValue": "hello"},
            "rank": {"integerValue": "7"},
        }),
        "the JSONB payload must be the documented Firestore REST value shape"
    );

    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn seed_loads_an_export_and_is_idempotent() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("seed");
    let root = format!("projects/{project_id}/databases/(default)/documents");

    let export = write_export(3);
    let store = store(&url).await;

    let first = local::seed::seed_from_export(&store, &root, export.path())
        .await
        .expect("seed succeeds");
    assert_eq!(first.parsed, 3, "{}", first.summary());
    assert_eq!(first.created, 3, "{}", first.summary());
    assert_eq!(first.updated, 0);
    assert!(first.is_complete(), "{:?}", first.skipped);

    // Re-seeding the same export upserts rather than duplicating or failing.
    let second = local::seed::seed_from_export(&store, &root, export.path())
        .await
        .expect("re-seed succeeds");
    assert_eq!(second.created, 0, "{}", second.summary());
    assert_eq!(second.updated, 3, "{}", second.summary());

    // And a server over the same database serves the seeded documents.
    let (server, mut client) = start(&url, &project_id).await;
    let found = client
        .get_document(GetDocumentRequest {
            name: format!("{root}/seeded/doc1"),
            ..Default::default()
        })
        .await
        .expect("a seeded document is readable through the API")
        .into_inner();

    assert_eq!(found.fields["email"], string_value("user1@example.com"));
    assert_eq!(
        found.fields["rank"].value_type,
        Some(ValueType::IntegerValue(1))
    );

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

/// `RunQuery` over the Postgres store, end to end through the gRPC server.
///
/// `local_query_pushdown_tests` proves the translation agrees with a full scan
/// for every query shape; this proves the server is actually wired to it and an
/// SDK gets the right documents back. Ordering by a field is included because
/// that is the half of the pre-filter that is not part of the `where` clause.
#[tokio::test]
async fn run_query_over_postgres_answers_a_filtered_ordered_query() {
    let Some(url) = test_url() else { return };
    let project_id = unique_project("runquery");
    let root = format!("projects/{project_id}/databases/(default)/documents");

    let (server, mut client) = start(&url, &project_id).await;
    for (id, city, age) in [
        ("alice", "Tokyo", 31),
        ("bob", "Kyoto", 41),
        ("carol", "Tokyo", 24),
        // No age, so a query ordering by age must leave this one out.
        ("dan", "Tokyo", -1),
    ] {
        let mut fields: std::collections::HashMap<String, Value> =
            [("city".to_string(), string_value(city))].into_iter().collect();
        if id != "dan" {
            fields.insert("age".to_string(), value(ValueType::IntegerValue(age)));
        }
        client
            .create_document(CreateDocumentRequest {
                parent: root.clone(),
                collection_id: "people".to_string(),
                document_id: id.to_string(),
                document: Some(Document {
                    fields,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .expect("create a document");
    }

    let query = StructuredQuery {
        from: vec![CollectionSelector {
            collection_id: "people".to_string(),
            all_descendants: false,
        }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(FieldReference {
                    field_path: "city".to_string(),
                }),
                op: FieldOperator::Equal as i32,
                value: Some(string_value("Tokyo")),
            })),
        }),
        order_by: vec![Order {
            field: Some(FieldReference {
                field_path: "age".to_string(),
            }),
            direction: Direction::Ascending as i32,
        }],
        ..Default::default()
    };

    let mut stream = client
        .run_query(RunQueryRequest {
            parent: root.clone(),
            query_type: Some(QueryType::StructuredQuery(query)),
            ..Default::default()
        })
        .await
        .expect("the query runs")
        .into_inner();

    let mut found = Vec::new();
    while let Some(response) = stream.message().await.expect("a response") {
        if let Some(document) = response.document {
            found.push(
                document
                    .name
                    .rsplit('/')
                    .next()
                    .expect("a name has segments")
                    .to_string(),
            );
        }
    }

    assert_eq!(
        found,
        ["carol", "alice"],
        "Tokyo only, youngest first, and dan has no age to order by"
    );

    server.shutdown().await.expect("clean shutdown");
    cleanup(&url, &project_id).await;
}

#[tokio::test]
async fn an_unreachable_database_fails_startup_instead_of_falling_back_to_memory() {
    // No test database needed: the point is that a bad URL is a hard error.
    // Port 1 on loopback has nothing listening on it.
    let config = LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid address"))
        .with_postgres("postgresql://fireup:nope@127.0.0.1:1/fireup_nonexistent");

    let rendered = match local::spawn(config).await {
        Ok(_) => panic!("an unreachable store must not start a server"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        rendered.contains("PostgreSQL"),
        "the error should name the storage backend: {rendered}"
    );
}

fn value(value_type: ValueType) -> Value {
    Value {
        value_type: Some(value_type),
    }
}

fn string_value(text: &str) -> Value {
    value(ValueType::StringValue(text.to_string()))
}

/// Write a Firestore-style JSON Lines export the shared parser understands.
fn write_export(count: usize) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create export file");
    for index in 0..count {
        writeln!(
            file,
            r#"{{"name":"projects/source/databases/(default)/documents/seeded/doc{index}","fields":{{"email":{{"stringValue":"user{index}@example.com"}},"rank":{{"integerValue":"{index}"}}}}}}"#
        )
        .expect("write document");
    }
    file.flush().expect("flush export file");
    file
}
