//! `fireup local` Datastore data-plane tests.
//!
//! These drive the server over real gRPC using stubs generated from the public
//! `googleapis` protos. The separate SDK smoke test
//! (`scripts/smoke-local-datastore.sh`) proves an official client library can
//! talk to it; this file is the fast check that runs in `cargo test`.

use std::collections::HashMap;

use fireup::local;
use fireup::protos::google::datastore::v1::commit_request::Mode as CommitMode;
use fireup::protos::google::datastore::v1::datastore_client::DatastoreClient;
use fireup::protos::google::datastore::v1::key::path_element::IdType;
use fireup::protos::google::datastore::v1::key::PathElement;
use fireup::protos::google::datastore::v1::mutation::Operation;
use fireup::protos::google::datastore::v1::property_filter::Operator as PropertyOp;
use fireup::protos::google::datastore::v1::run_query_request::QueryType;
use fireup::protos::google::datastore::v1::value::ValueType;
use fireup::protos::google::datastore::v1::{
    filter::FilterType, AllocateIdsRequest, BeginTransactionRequest, CommitRequest, Entity,
    Filter, Key, KindExpression, LookupRequest, Mutation, PropertyFilter, PropertyReference,
    Query, RollbackRequest, RunAggregationQueryRequest, RunQueryRequest, Value,
};
use tonic::transport::Channel;
use tonic::Code;

const PROJECT: &str = "fireup-test";

fn string_value(text: &str) -> Value {
    Value {
        value_type: Some(ValueType::StringValue(text.to_string())),
        ..Default::default()
    }
}

fn integer_value(n: i64) -> Value {
    Value {
        value_type: Some(ValueType::IntegerValue(n)),
        ..Default::default()
    }
}

fn named_key(kind: &str, name: &str) -> Key {
    Key {
        partition_id: None,
        path: vec![PathElement {
            kind: kind.to_string(),
            id_type: Some(IdType::Name(name.to_string())),
        }],
    }
}

fn incomplete_key(kind: &str) -> Key {
    Key {
        partition_id: None,
        path: vec![PathElement {
            kind: kind.to_string(),
            id_type: None,
        }],
    }
}

fn entity(key: Key, properties: HashMap<String, Value>) -> Entity {
    Entity {
        key: Some(key),
        properties,
    }
}

async fn start() -> (local::RunningServer, DatastoreClient<Channel>) {
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_project_id(PROJECT)
        .with_only(local::Service::Datastore);
    let server = local::spawn(config).await.expect("server binds");
    let client = DatastoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects");
    (server, client)
}

#[tokio::test]
async fn commit_lookup_and_query_round_trip() {
    let (server, mut client) = start().await;

    let key = named_key("Task", "sample");
    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations: vec![Mutation {
                operation: Some(Operation::Upsert(entity(
                    key.clone(),
                    [
                        ("title".to_string(), string_value("write tests")),
                        ("done".to_string(), Value {
                            value_type: Some(ValueType::BooleanValue(false)),
                            ..Default::default()
                        }),
                        ("priority".to_string(), integer_value(3)),
                    ]
                    .into_iter()
                    .collect(),
                ))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("Commit upsert");

    let looked_up = client
        .lookup(LookupRequest {
            project_id: PROJECT.to_string(),
            keys: vec![key.clone()],
            ..Default::default()
        })
        .await
        .expect("Lookup")
        .into_inner();

    assert_eq!(looked_up.found.len(), 1);
    assert!(looked_up.missing.is_empty());
    let found = looked_up.found[0].entity.as_ref().expect("entity");
    assert_eq!(
        found.properties["title"],
        string_value("write tests")
    );

    let queried = client
        .run_query(RunQueryRequest {
            project_id: PROJECT.to_string(),
            query_type: Some(QueryType::Query(Query {
                kind: vec![KindExpression {
                    name: "Task".to_string(),
                }],
                filter: Some(Filter {
                    filter_type: Some(FilterType::PropertyFilter(PropertyFilter {
                        property: Some(PropertyReference {
                            name: "priority".to_string(),
                        }),
                        op: PropertyOp::Equal as i32,
                        value: Some(integer_value(3)),
                    })),
                }),
                ..Default::default()
            })),
            ..Default::default()
        })
        .await
        .expect("RunQuery")
        .into_inner();

    let batch = queried.batch.expect("batch");
    assert_eq!(batch.entity_results.len(), 1);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn allocate_ids_completes_incomplete_keys() {
    let (server, mut client) = start().await;

    let response = client
        .allocate_ids(AllocateIdsRequest {
            project_id: PROJECT.to_string(),
            keys: vec![incomplete_key("Task"), incomplete_key("Task")],
            ..Default::default()
        })
        .await
        .expect("AllocateIds")
        .into_inner();

    assert_eq!(response.keys.len(), 2);
    let first = response.keys[0].path[0].id_type.as_ref().expect("id");
    let second = response.keys[1].path[0].id_type.as_ref().expect("id");
    assert_ne!(first, second);
    assert!(matches!(first, IdType::Id(id) if *id > 0));

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn begin_transaction_commit_and_rollback() {
    let (server, mut client) = start().await;

    let begun = client
        .begin_transaction(BeginTransactionRequest {
            project_id: PROJECT.to_string(),
            ..Default::default()
        })
        .await
        .expect("BeginTransaction")
        .into_inner();
    assert!(!begun.transaction.is_empty());

    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::Transactional as i32,
            transaction_selector: Some(
                fireup::protos::google::datastore::v1::commit_request::TransactionSelector::Transaction(
                    begun.transaction.clone(),
                ),
            ),
            mutations: vec![Mutation {
                operation: Some(Operation::Insert(entity(
                    named_key("Task", "tx-1"),
                    [("title".to_string(), string_value("in tx"))]
                        .into_iter()
                        .collect(),
                ))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("transactional Commit");

    let other = client
        .begin_transaction(BeginTransactionRequest {
            project_id: PROJECT.to_string(),
            ..Default::default()
        })
        .await
        .expect("second BeginTransaction")
        .into_inner();
    client
        .rollback(RollbackRequest {
            project_id: PROJECT.to_string(),
            transaction: other.transaction,
            ..Default::default()
        })
        .await
        .expect("Rollback");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn insert_with_incomplete_key_allocates_an_id() {
    let (server, mut client) = start().await;

    let response = client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations: vec![Mutation {
                operation: Some(Operation::Insert(entity(
                    incomplete_key("Task"),
                    [("title".to_string(), string_value("auto id"))]
                        .into_iter()
                        .collect(),
                ))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("insert incomplete")
        .into_inner();

    let key = response.mutation_results[0]
        .key
        .as_ref()
        .expect("allocated key returned");
    assert!(matches!(
        key.path[0].id_type.as_ref(),
        Some(IdType::Id(id)) if *id > 0
    ));

    let looked_up = client
        .lookup(LookupRequest {
            project_id: PROJECT.to_string(),
            keys: vec![key.clone()],
            ..Default::default()
        })
        .await
        .expect("Lookup")
        .into_inner();
    assert_eq!(looked_up.found.len(), 1);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn aggregation_queries_are_unimplemented() {
    let (server, mut client) = start().await;

    let status = client
        .run_aggregation_query(RunAggregationQueryRequest {
            project_id: PROJECT.to_string(),
            ..Default::default()
        })
        .await
        .expect_err("aggregations are not Phase 3");
    assert_eq!(status.code(), Code::Unimplemented);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn firestore_and_datastore_share_the_store() {
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("addr"))
        .with_project_id(PROJECT);
    let server = local::spawn(config).await.expect("server");

    let mut datastore = DatastoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("datastore client");
    let mut firestore =
        fireup::protos::google::firestore::v1::firestore_client::FirestoreClient::connect(
            format!("http://{}", server.addr),
        )
        .await
        .expect("firestore client");

    datastore
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations: vec![Mutation {
                operation: Some(Operation::Upsert(entity(
                    named_key("Shared", "one"),
                    [("via".to_string(), string_value("datastore"))]
                        .into_iter()
                        .collect(),
                ))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("datastore write");

    let document = firestore
        .get_document(
            fireup::protos::google::firestore::v1::GetDocumentRequest {
                name: format!(
                    "projects/{PROJECT}/databases/(default)/documents/Shared/one"
                ),
                ..Default::default()
            },
        )
        .await
        .expect("firestore read of datastore write")
        .into_inner();
    assert_eq!(
        document.fields["via"].value_type,
        Some(
            fireup::protos::google::firestore::v1::value::ValueType::StringValue(
                "datastore".to_string()
            )
        )
    );

    server.shutdown().await.expect("clean shutdown");
}
