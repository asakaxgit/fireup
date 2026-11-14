//! `fireup local` Datastore data-plane tests.
//!
//! These drive the server over real gRPC using stubs generated from the public
//! `googleapis` protos. The separate SDK smoke test
//! (`scripts/smoke-local-datastore.sh`) proves an official client library can
//! talk to it; this file is the fast check that runs in `cargo test`.

use std::collections::HashMap;

use fireup::local;
use fireup::protos::google::datastore::v1::commit_request::Mode as CommitMode;
use fireup::protos::google::datastore::v1::composite_filter::Operator as CompositeOp;
use fireup::protos::google::datastore::v1::datastore_client::DatastoreClient;
use fireup::protos::google::datastore::v1::key::path_element::IdType;
use fireup::protos::google::datastore::v1::key::PathElement;
use fireup::protos::google::datastore::v1::mutation::Operation;
use fireup::protos::google::datastore::v1::property_filter::Operator as PropertyOp;
use fireup::protos::google::datastore::v1::property_order::Direction;
use fireup::protos::google::datastore::v1::property_transform::{ServerValue, TransformType};
use fireup::protos::google::datastore::v1::query_result_batch::MoreResultsType;
use fireup::protos::google::datastore::v1::run_query_request::QueryType;
use fireup::protos::google::datastore::v1::value::ValueType;
use fireup::protos::google::datastore::v1::{
    filter::FilterType, AllocateIdsRequest, ArrayValue, BeginTransactionRequest, CommitRequest,
    CompositeFilter, Entity, Filter, Key, KindExpression, LookupRequest, Mutation, PartitionId,
    PropertyFilter, PropertyMask, PropertyOrder, PropertyReference, PropertyTransform, Query,
    QueryResultBatch, RollbackRequest, RunAggregationQueryRequest, RunQueryRequest, Value,
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

fn property_transform(property: &str, transform_type: TransformType) -> PropertyTransform {
    PropertyTransform {
        property: property.to_string(),
        transform_type: Some(transform_type),
    }
}

/// An `upsert` of `properties` with `transforms` applied after it.
fn upsert_with_transforms(
    key: Key,
    properties: HashMap<String, Value>,
    transforms: Vec<PropertyTransform>,
) -> Mutation {
    Mutation {
        operation: Some(Operation::Upsert(entity(key, properties))),
        property_transforms: transforms,
        ..Default::default()
    }
}

/// An `upsert` whose write is restricted to `mask`, with `transforms` applied
/// after it — the shape `property_mask` exists for.
fn masked_upsert(
    key: Key,
    properties: HashMap<String, Value>,
    mask: &[&str],
    transforms: Vec<PropertyTransform>,
) -> Mutation {
    Mutation {
        operation: Some(Operation::Upsert(entity(key, properties))),
        property_mask: Some(PropertyMask {
            paths: mask.iter().map(|path| path.to_string()).collect(),
        }),
        property_transforms: transforms,
        ..Default::default()
    }
}

fn commit_one(mutation: Mutation) -> CommitRequest {
    CommitRequest {
        project_id: PROJECT.to_string(),
        mode: CommitMode::NonTransactional as i32,
        mutations: vec![mutation],
        ..Default::default()
    }
}

async fn lookup(client: &mut DatastoreClient<Channel>, key: &Key) -> Option<Entity> {
    client
        .lookup(LookupRequest {
            project_id: PROJECT.to_string(),
            keys: vec![key.clone()],
            ..Default::default()
        })
        .await
        .expect("Lookup")
        .into_inner()
        .found
        .into_iter()
        .next()
        .and_then(|result| result.entity)
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
async fn property_transforms_apply_after_the_entity_and_report_their_results() {
    let (server, mut client) = start().await;

    let key = named_key("Counter", "transforms");
    let response = client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [
                ("title".to_string(), string_value("first")),
                // The transform runs *after* the operation, so it wins over the
                // value the same mutation carries for the same property.
                ("hits".to_string(), integer_value(100)),
            ]
            .into_iter()
            .collect(),
            vec![
                property_transform(
                    "touched",
                    TransformType::SetToServerValue(ServerValue::RequestTime as i32),
                ),
                property_transform("hits", TransformType::Increment(integer_value(1))),
                property_transform(
                    "tags",
                    TransformType::AppendMissingElements(ArrayValue {
                        values: vec![string_value("a"), string_value("a")],
                    }),
                ),
            ],
        )))
        .await
        .expect("Commit with transforms")
        .into_inner();

    let results = &response.mutation_results[0].transform_results;
    assert_eq!(
        results.len(),
        3,
        "one result per transform, in request order"
    );
    assert!(
        matches!(results[0].value_type, Some(ValueType::TimestampValue(_))),
        "serverTimestamp reports the commit time: {:?}",
        results[0]
    );
    assert_eq!(
        results[1],
        integer_value(101),
        "increment reports the value it wrote, on top of the mutation's own 100"
    );
    assert_eq!(
        results[2].value_type,
        Some(ValueType::NullValue(0)),
        "the array transforms report the null value"
    );

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(stored.properties["title"], string_value("first"));
    assert_eq!(stored.properties["hits"], integer_value(101));
    assert_eq!(
        stored.properties["tags"].value_type,
        Some(ValueType::ArrayValue(ArrayValue {
            values: vec![string_value("a")],
        })),
        "a repeated input element is appended once"
    );
    assert!(matches!(
        stored.properties["touched"].value_type,
        Some(ValueType::TimestampValue(_))
    ));

    // A follow-up `update` that does not carry `hits` *resets* it: the proto
    // runs the transforms after the operation, and an unmasked update replaces
    // the entity, so the increment lands on a property that is no longer there
    // and sets it to the operand. Pinned because it is the surprising half of
    // "after the operation" and the fidelity matrix calls it out.
    let again = client
        .commit(commit_one(Mutation {
            operation: Some(Operation::Update(entity(
                key.clone(),
                [("title".to_string(), string_value("second"))]
                    .into_iter()
                    .collect(),
            ))),
            property_transforms: vec![property_transform(
                "hits",
                TransformType::Increment(integer_value(1)),
            )],
            ..Default::default()
        }))
        .await
        .expect("second Commit")
        .into_inner();
    assert_eq!(
        again.mutation_results[0].transform_results[0],
        integer_value(1),
        "the unmasked update dropped `hits` before the transform read it"
    );

    // Carrying the current value is therefore how a Datastore client
    // accumulates — the read-modify-write idiom inside a transaction.
    let accumulated = client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [
                ("title".to_string(), string_value("third")),
                ("hits".to_string(), integer_value(1)),
            ]
            .into_iter()
            .collect(),
            vec![property_transform(
                "hits",
                TransformType::Increment(integer_value(41)),
            )],
        )))
        .await
        .expect("third Commit")
        .into_inner();
    assert_eq!(
        accumulated.mutation_results[0].transform_results[0],
        integer_value(42)
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn increment_sets_a_missing_property_and_a_double_operand_makes_it_a_double() {
    let (server, mut client) = start().await;

    let key = named_key("Counter", "fresh");
    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            HashMap::new(),
            vec![
                property_transform("hits", TransformType::Increment(integer_value(7))),
                property_transform(
                    "ratio",
                    TransformType::Increment(Value {
                        value_type: Some(ValueType::DoubleValue(0.5)),
                        ..Default::default()
                    }),
                ),
            ],
        )))
        .await
        .expect("Commit onto a missing property");

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["hits"],
        integer_value(7),
        "a missing property is set to the operand, keeping its integer tag"
    );
    assert_eq!(
        stored.properties["ratio"].value_type,
        Some(ValueType::DoubleValue(0.5))
    );

    // An *integer* increment of a double property: the sum stays a double. The
    // entity carries the current value because an unmasked upsert replaces it.
    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [(
                "ratio".to_string(),
                Value {
                    value_type: Some(ValueType::DoubleValue(0.5)),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            vec![property_transform(
                "ratio",
                TransformType::Increment(integer_value(1)),
            )],
        )))
        .await
        .expect("Commit a mixed-type increment");
    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["ratio"].value_type,
        Some(ValueType::DoubleValue(1.5)),
        "a double on either side makes the sum a double"
    );

    // And a string property is overwritten rather than failing the write.
    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [("ratio".to_string(), string_value("not a number"))]
                .into_iter()
                .collect(),
            vec![property_transform(
                "ratio",
                TransformType::Increment(integer_value(3)),
            )],
        )))
        .await
        .expect("Commit onto a non-numeric property");
    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(stored.properties["ratio"], integer_value(3));

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn array_remove_matches_a_key_property_through_the_entity_encoding() {
    let (server, mut client) = start().await;

    // A key property is stored as a Firestore reference string, so the transform
    // operand has to cross the same conversion or it would never match.
    let referenced = named_key("Task", "sample");
    let key_value = Value {
        value_type: Some(ValueType::KeyValue(referenced.clone())),
        ..Default::default()
    };
    let key = named_key("Counter", "keys-in-arrays");

    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [(
                "related".to_string(),
                Value {
                    value_type: Some(ValueType::ArrayValue(ArrayValue {
                        values: vec![key_value.clone(), string_value("keep")],
                    })),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            vec![property_transform(
                "related",
                TransformType::RemoveAllFromArray(ArrayValue {
                    values: vec![key_value.clone()],
                }),
            )],
        )))
        .await
        .expect("Commit arrayRemove of a key");

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["related"].value_type,
        Some(ValueType::ArrayValue(ArrayValue {
            values: vec![string_value("keep")],
        })),
        "the key element was removed and the round-tripped key compared equal"
    );

    // And arrayUnion of the same key does not re-add it when it is present.
    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [(
                "related".to_string(),
                Value {
                    value_type: Some(ValueType::ArrayValue(ArrayValue {
                        values: vec![key_value.clone()],
                    })),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            vec![property_transform(
                "related",
                TransformType::AppendMissingElements(ArrayValue {
                    values: vec![key_value.clone()],
                }),
            )],
        )))
        .await
        .expect("Commit arrayUnion of a key");

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    let Some(ValueType::ArrayValue(array)) = &stored.properties["related"].value_type else {
        panic!("expected an array, got {:?}", stored.properties["related"]);
    };
    assert_eq!(
        array.values.len(),
        1,
        "an already-present key is not appended"
    );
    let Some(ValueType::KeyValue(round_tripped)) = &array.values[0].value_type else {
        panic!(
            "expected a key, not a reference string: {:?}",
            array.values[0]
        );
    };
    assert_eq!(round_tripped.path, referenced.path);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn unservable_property_transforms_are_refused_and_apply_nothing() {
    let (server, mut client) = start().await;

    let key = named_key("Counter", "refused");
    let cases: Vec<(&str, Mutation, Code)> = vec![
        (
            "maximum",
            upsert_with_transforms(
                key.clone(),
                HashMap::new(),
                vec![property_transform(
                    "hits",
                    TransformType::Maximum(integer_value(1)),
                )],
            ),
            Code::Unimplemented,
        ),
        (
            "minimum",
            upsert_with_transforms(
                key.clone(),
                HashMap::new(),
                vec![property_transform(
                    "hits",
                    TransformType::Minimum(integer_value(1)),
                )],
            ),
            Code::Unimplemented,
        ),
        (
            "a property path",
            upsert_with_transforms(
                key.clone(),
                HashMap::new(),
                vec![property_transform(
                    "nested.hits",
                    TransformType::Increment(integer_value(1)),
                )],
            ),
            Code::Unimplemented,
        ),
        (
            "a non-numeric increment operand",
            upsert_with_transforms(
                key.clone(),
                HashMap::new(),
                vec![property_transform(
                    "hits",
                    TransformType::Increment(string_value("1")),
                )],
            ),
            Code::InvalidArgument,
        ),
        (
            "a transform on a delete",
            Mutation {
                operation: Some(Operation::Delete(key.clone())),
                property_transforms: vec![property_transform(
                    "hits",
                    TransformType::Increment(integer_value(1)),
                )],
                ..Default::default()
            },
            Code::InvalidArgument,
        ),
    ];

    for (label, mutation, expected) in cases {
        let status = client.commit(commit_one(mutation)).await.expect_err(label);
        assert_eq!(status.code(), expected, "{label}: {}", status.message());
    }

    assert!(
        lookup(&mut client, &key).await.is_none(),
        "a refused batch applies none of its writes"
    );

    // The refusal fails the whole batch, not just the offending mutation.
    let served = named_key("Counter", "served-alongside");
    let status = client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations: vec![
                upsert_with_transforms(
                    served.clone(),
                    [("title".to_string(), string_value("fine"))]
                        .into_iter()
                        .collect(),
                    vec![property_transform(
                        "hits",
                        TransformType::Increment(integer_value(1)),
                    )],
                ),
                upsert_with_transforms(
                    key.clone(),
                    HashMap::new(),
                    vec![property_transform(
                        "hits",
                        TransformType::Maximum(integer_value(1)),
                    )],
                ),
            ],
            ..Default::default()
        })
        .await
        .expect_err("one bad transform fails the batch");
    assert_eq!(status.code(), Code::Unimplemented);
    assert!(
        lookup(&mut client, &served).await.is_none(),
        "the servable mutation in the same batch did not apply either"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_datastore_increment_and_a_firestore_increment_agree_on_the_same_property() {
    use fireup::protos::google::firestore::v1 as fs;

    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("addr"))
        .with_project_id(PROJECT);
    let server = local::spawn(config).await.expect("server");

    let mut datastore = DatastoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("datastore client");
    let mut firestore =
        fs::firestore_client::FirestoreClient::connect(format!("http://{}", server.addr))
            .await
            .expect("firestore client");

    let key = named_key("Shared", "counter");
    datastore
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            HashMap::new(),
            vec![property_transform(
                "hits",
                TransformType::Increment(integer_value(5)),
            )],
        )))
        .await
        .expect("datastore increment");

    // The Firestore surface increments the *same* stored field. Its masked
    // update is a merge, so it reads the 5 the Datastore write left behind —
    // which is only true if both surfaces tagged the value as an integer.
    let name = format!("projects/{PROJECT}/databases/(default)/documents/Shared/counter");
    let written = firestore
        .commit(fs::CommitRequest {
            database: format!("projects/{PROJECT}/databases/(default)"),
            writes: vec![fs::Write {
                update_mask: Some(fs::DocumentMask {
                    field_paths: Vec::new(),
                }),
                update_transforms: vec![fs::document_transform::FieldTransform {
                    field_path: "hits".to_string(),
                    transform_type: Some(
                        fs::document_transform::field_transform::TransformType::Increment(
                            fs::Value {
                                value_type: Some(fs::value::ValueType::IntegerValue(1)),
                            },
                        ),
                    ),
                }],
                operation: Some(fs::write::Operation::Update(fs::Document {
                    name: name.clone(),
                    fields: Default::default(),
                    create_time: None,
                    update_time: None,
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("firestore increment")
        .into_inner();
    assert_eq!(
        written.write_results[0].transform_results[0].value_type,
        Some(fs::value::ValueType::IntegerValue(6)),
        "the Firestore increment read the Datastore write's integer"
    );

    let stored = lookup(&mut datastore, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["hits"],
        integer_value(6),
        "and Datastore reads it back as an integer property"
    );

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

/// The point of the whole feature: a masked write plus a transform accumulates.
///
/// `property_transforms` run after the operation, so an *unmasked* update has
/// already replaced the entity by the time the increment reads it — pinned in
/// `property_transforms_apply_after_the_entity_and_report_their_results`, where
/// an update that omits `hits` resets it. A mask that does not name `hits` keeps
/// the write off it, so the increment reads what is stored and the counter
/// climbs across commits without the client ever sending its value. This is the
/// same pairing as Firestore's `update_mask` + `update_transforms`.
#[tokio::test]
async fn a_masked_upsert_lets_a_transform_accumulate_across_commits() {
    let (server, mut client) = start().await;
    let key = named_key("Page", "masked-counter");

    // Seed the whole entity unmasked, including a property no later mask names.
    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [
                ("title".to_string(), string_value("first")),
                ("hits".to_string(), integer_value(0)),
                ("author".to_string(), string_value("ada")),
            ]
            .into_iter()
            .collect(),
            Vec::new(),
        )))
        .await
        .expect("seed Commit");

    // Three masked commits that only ever carry `title`.
    for (title, expected_hits) in [("second", 1), ("third", 2), ("fourth", 3)] {
        let response = client
            .commit(commit_one(masked_upsert(
                key.clone(),
                [("title".to_string(), string_value(title))]
                    .into_iter()
                    .collect(),
                &["title"],
                vec![property_transform(
                    "hits",
                    TransformType::Increment(integer_value(1)),
                )],
            )))
            .await
            .expect("masked Commit")
            .into_inner();
        assert_eq!(
            response.mutation_results[0].transform_results[0],
            integer_value(expected_hits),
            "the increment read the stored `hits`, not one the client sent"
        );
    }

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(stored.properties["title"], string_value("fourth"));
    assert_eq!(stored.properties["hits"], integer_value(3));
    assert_eq!(
        stored.properties["author"],
        string_value("ada"),
        "a property the mask does not name is left untouched"
    );
    assert_eq!(
        stored.properties.len(),
        3,
        "no extra properties appeared: {:?}",
        stored.properties.keys().collect::<Vec<_>>()
    );

    server.shutdown().await.expect("clean shutdown");
}

/// The two halves of the proto's mask rule, which pull in opposite directions:
/// a property the mask names but the entity omits is **deleted**, while a
/// property the entity carries but the mask does not name is **left untouched**.
#[tokio::test]
async fn a_mask_deletes_what_it_names_and_ignores_what_it_does_not() {
    let (server, mut client) = start().await;
    let key = named_key("Post", "masked-delete");

    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [
                ("title".to_string(), string_value("draft title")),
                ("draft".to_string(), integer_value(1)),
                ("author".to_string(), string_value("ada")),
            ]
            .into_iter()
            .collect(),
            Vec::new(),
        )))
        .await
        .expect("seed Commit");

    client
        .commit(commit_one(Mutation {
            // The entity carries `title` and `author`; the mask names `title`
            // and `draft`. So: `title` is written, `draft` is deleted because
            // the mask names it and the entity does not carry it, and `author`
            // is left alone because the mask does not name it — even though the
            // entity carries a different value for it.
            operation: Some(Operation::Update(entity(
                key.clone(),
                [
                    ("title".to_string(), string_value("published title")),
                    ("author".to_string(), string_value("ignored")),
                ]
                .into_iter()
                .collect(),
            ))),
            property_mask: Some(PropertyMask {
                paths: vec!["title".to_string(), "draft".to_string()],
            }),
            ..Default::default()
        }))
        .await
        .expect("masked update Commit");

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(stored.properties["title"], string_value("published title"));
    assert_eq!(
        stored.properties["author"],
        string_value("ada"),
        "an unmasked property keeps its stored value even when the entity carries another"
    );
    assert!(
        !stored.properties.contains_key("draft"),
        "a masked property the entity omits is deleted: {:?}",
        stored.properties.keys().collect::<Vec<_>>()
    );

    server.shutdown().await.expect("clean shutdown");
}

/// The proto: `property_mask` "is ignored for `delete`". So a delete carrying
/// one removes the whole entity rather than some of its properties, and is not
/// an error either.
#[tokio::test]
async fn property_mask_is_ignored_on_a_delete() {
    let (server, mut client) = start().await;
    let key = named_key("Post", "masked-delete-op");

    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [
                ("title".to_string(), string_value("doomed")),
                ("author".to_string(), string_value("ada")),
            ]
            .into_iter()
            .collect(),
            Vec::new(),
        )))
        .await
        .expect("seed Commit");

    client
        .commit(commit_one(Mutation {
            operation: Some(Operation::Delete(key.clone())),
            property_mask: Some(PropertyMask {
                paths: vec!["title".to_string()],
            }),
            ..Default::default()
        }))
        .await
        .expect("a delete with a property_mask is accepted");

    assert!(
        lookup(&mut client, &key).await.is_none(),
        "the whole entity is gone, not just the masked property"
    );

    server.shutdown().await.expect("clean shutdown");
}

/// A mask changes which properties are written, not whether the operation's own
/// precondition holds: a masked `insert` on an existing entity is still
/// ALREADY_EXISTS, and a masked `update` on a missing one is still NOT_FOUND.
#[tokio::test]
async fn a_mask_does_not_weaken_the_insert_and_update_preconditions() {
    let (server, mut client) = start().await;
    let key = named_key("Post", "masked-precondition");

    let masked_insert = |properties: HashMap<String, Value>| Mutation {
        operation: Some(Operation::Insert(entity(key.clone(), properties))),
        property_mask: Some(PropertyMask {
            paths: vec!["title".to_string()],
        }),
        ..Default::default()
    };

    let missing = client
        .commit(commit_one(Mutation {
            operation: Some(Operation::Update(entity(
                key.clone(),
                [("title".to_string(), string_value("nope"))]
                    .into_iter()
                    .collect(),
            ))),
            property_mask: Some(PropertyMask {
                paths: vec!["title".to_string()],
            }),
            ..Default::default()
        }))
        .await
        .expect_err("a masked update of a missing entity fails");
    assert_eq!(missing.code(), Code::NotFound);

    client
        .commit(commit_one(masked_insert(
            [("title".to_string(), string_value("first"))]
                .into_iter()
                .collect(),
        )))
        .await
        .expect("a masked insert of a new entity succeeds");

    let duplicate = client
        .commit(commit_one(masked_insert(
            [("title".to_string(), string_value("second"))]
                .into_iter()
                .collect(),
        )))
        .await
        .expect_err("a masked insert of an existing entity fails");
    assert_eq!(duplicate.code(), Code::AlreadyExists);

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["title"],
        string_value("first"),
        "the refused insert wrote nothing"
    );

    server.shutdown().await.expect("clean shutdown");
}

/// The mask paths fireup refuses, each a line of the proto, and the one
/// reserved name it accepts.
#[tokio::test]
async fn unservable_mask_paths_are_refused_and_write_nothing() {
    let (server, mut client) = start().await;
    let key = named_key("Post", "masked-refusals");

    client
        .commit(commit_one(upsert_with_transforms(
            key.clone(),
            [("title".to_string(), string_value("untouched"))]
                .into_iter()
                .collect(),
            Vec::new(),
        )))
        .await
        .expect("seed Commit");

    let refused = |paths: &[&str]| {
        commit_one(masked_upsert(
            key.clone(),
            [("title".to_string(), string_value("overwritten"))]
                .into_iter()
                .collect(),
            paths,
            Vec::new(),
        ))
    };

    // "A path is a list of property names separated by dots" — a nested path
    // names a property inside an entity value, which the Firestore update_mask
    // side refuses too rather than overwriting the whole parent property.
    let nested = client
        .commit(refused(&["author.name"]))
        .await
        .expect_err("a nested mask path is refused");
    assert_eq!(nested.code(), Code::Unimplemented);
    // The escape exists only to disambiguate that, so it is refused with it.
    let escaped = client
        .commit(refused(&["author\\.name"]))
        .await
        .expect_err("an escaped mask path is refused");
    assert_eq!(escaped.code(), Code::Unimplemented);

    // "A path must not be empty."
    let empty = client
        .commit(refused(&[""]))
        .await
        .expect_err("an empty mask path is refused");
    assert_eq!(empty.code(), Code::InvalidArgument);

    // "None of the properties in the mask may have a reserved name, except for
    // `__key__`."
    let reserved = client
        .commit(refused(&["title", "__scatter__"]))
        .await
        .expect_err("a reserved mask path is refused");
    assert_eq!(reserved.code(), Code::InvalidArgument);

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["title"],
        string_value("untouched"),
        "every refusal happened before the store was touched"
    );

    // `__key__` is the exception, and it names no field — the key travels in the
    // entity's key — so a mask of only `__key__` writes no properties. The
    // transforms still run: they apply "regardless of the property mask".
    let key_masked = client
        .commit(commit_one(masked_upsert(
            key.clone(),
            [("title".to_string(), string_value("ignored"))]
                .into_iter()
                .collect(),
            &["__key__"],
            vec![property_transform(
                "hits",
                TransformType::Increment(integer_value(7)),
            )],
        )))
        .await
        .expect("a __key__ mask is accepted")
        .into_inner();
    assert_eq!(
        key_masked.mutation_results[0].transform_results[0],
        integer_value(7)
    );

    let stored = lookup(&mut client, &key).await.expect("entity stored");
    assert_eq!(
        stored.properties["title"],
        string_value("untouched"),
        "a __key__-only mask wrote no properties"
    );
    assert_eq!(stored.properties["hits"], integer_value(7));

    server.shutdown().await.expect("clean shutdown");
}

fn property_filter(name: &str, op: PropertyOp, value: Value) -> Filter {
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

fn composite(op: CompositeOp, filters: Vec<Filter>) -> Filter {
    Filter {
        filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
            op: op as i32,
            filters,
        })),
    }
}

fn integers(values: &[i64]) -> Value {
    Value {
        value_type: Some(ValueType::ArrayValue(ArrayValue {
            values: values.iter().copied().map(integer_value).collect(),
        })),
        ..Default::default()
    }
}

/// The names a `RunQuery` over kind `kind` with `filter` answers, sorted.
async fn query_names(
    client: &mut DatastoreClient<Channel>,
    kind: &str,
    filter: Filter,
) -> Result<Vec<String>, tonic::Status> {
    let batch = client
        .run_query(RunQueryRequest {
            project_id: PROJECT.to_string(),
            query_type: Some(QueryType::Query(Query {
                kind: vec![KindExpression {
                    name: kind.to_string(),
                }],
                filter: Some(filter),
                ..Default::default()
            })),
            ..Default::default()
        })
        .await?
        .into_inner()
        .batch
        .expect("batch");
    let mut names: Vec<String> = batch
        .entity_results
        .into_iter()
        .filter_map(|result| result.entity?.key?.path.pop()?.id_type)
        .map(|id| match id {
            IdType::Name(name) => name,
            IdType::Id(id) => id.to_string(),
        })
        .collect();
    names.sort();
    Ok(names)
}

/// `!=`, `IN`, `NOT_IN` and `OR` used to be accepted and answered with no
/// entities at all. Each is served now: a property of another type is "not
/// equal", a missing property matches none of them, and an array-valued
/// property matches when any one of its values does.
#[tokio::test]
async fn not_equal_in_not_in_and_or_filters_are_served() {
    let (server, mut client) = start().await;

    let kind = "Filtered";
    let seed = [
        (
            "a",
            vec![
                ("priority", integer_value(1)),
                ("owner", string_value("ada")),
            ],
        ),
        (
            "b",
            vec![
                ("priority", integer_value(2)),
                ("owner", string_value("bob")),
            ],
        ),
        (
            "c",
            vec![
                ("priority", integer_value(3)),
                ("owner", string_value("ada")),
            ],
        ),
        ("text", vec![("priority", string_value("2"))]),
        ("list", vec![("priority", integers(&[2, 5]))]),
        ("bare", vec![("owner", string_value("bob"))]),
    ];
    let mutations = seed
        .into_iter()
        .map(|(name, properties)| Mutation {
            operation: Some(Operation::Upsert(entity(
                named_key(kind, name),
                properties
                    .into_iter()
                    .map(|(property, value)| (property.to_string(), value))
                    .collect(),
            ))),
            ..Default::default()
        })
        .collect();
    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations,
            ..Default::default()
        })
        .await
        .expect("seed");

    for (filter, want) in [
        (
            // `[2, 5]` holds a value, 5, that is not 2.
            property_filter("priority", PropertyOp::NotEqual, integer_value(2)),
            vec!["a", "c", "list", "text"],
        ),
        (
            property_filter("priority", PropertyOp::In, integers(&[1, 3])),
            vec!["a", "c"],
        ),
        (
            property_filter("priority", PropertyOp::NotIn, integers(&[1, 3])),
            vec!["b", "list", "text"],
        ),
        (
            composite(
                CompositeOp::Or,
                vec![
                    property_filter("priority", PropertyOp::Equal, integer_value(1)),
                    property_filter("owner", PropertyOp::Equal, string_value("bob")),
                ],
            ),
            vec!["a", "b", "bare"],
        ),
        (
            // A translatable OR inside an AND with an untranslatable `!=`: the
            // pushed part narrows, the evaluator still decides.
            composite(
                CompositeOp::And,
                vec![
                    composite(
                        CompositeOp::Or,
                        vec![
                            property_filter("owner", PropertyOp::Equal, string_value("ada")),
                            property_filter("priority", PropertyOp::In, integers(&[2])),
                        ],
                    ),
                    property_filter("priority", PropertyOp::NotEqual, integer_value(3)),
                ],
            ),
            vec!["a", "b", "list"],
        ),
    ] {
        let got = query_names(&mut client, kind, filter.clone())
            .await
            .unwrap_or_else(|status| panic!("{filter:?} was refused: {status}"));
        assert_eq!(got, want, "{filter:?}");
    }

    server.shutdown().await.expect("clean shutdown");
}

/// An array-valued property is matched one value at a time, for every
/// operator, and several range filters on it must all hold for one value. Both
/// rules, and their examples, are from the public Datastore queries page.
#[tokio::test]
async fn an_array_property_is_matched_element_by_element() {
    let (server, mut client) = start().await;

    let kind = "Tagged";
    let strings = |values: &[&str]| Value {
        value_type: Some(ValueType::ArrayValue(ArrayValue {
            values: values.iter().copied().map(string_value).collect(),
        })),
        ..Default::default()
    };
    let seed = [
        ("both", strings(&["fun", "programming"])),
        ("one", strings(&["learn"])),
        ("scalar", string_value("lisp")),
        ("empty", strings(&[])),
    ];
    let mutations = seed
        .into_iter()
        .map(|(name, tags)| Mutation {
            operation: Some(Operation::Upsert(entity(
                named_key(kind, name),
                [("tag".to_string(), tags)].into_iter().collect(),
            ))),
            ..Default::default()
        })
        .collect();
    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations,
            ..Default::default()
        })
        .await
        .expect("seed");

    let tag = |op, value| property_filter("tag", op, value);
    for (filter, want) in [
        (
            tag(PropertyOp::Equal, string_value("programming")),
            vec!["both"],
        ),
        (
            // Neither value satisfies both, but each satisfies one.
            composite(
                CompositeOp::And,
                vec![
                    tag(PropertyOp::Equal, string_value("fun")),
                    tag(PropertyOp::Equal, string_value("programming")),
                ],
            ),
            vec!["both"],
        ),
        (
            // Each value satisfies one range filter, and neither satisfies
            // both, so `both` does not match.
            composite(
                CompositeOp::And,
                vec![
                    tag(PropertyOp::GreaterThan, string_value("learn")),
                    tag(PropertyOp::LessThan, string_value("math")),
                ],
            ),
            vec!["scalar"],
        ),
        (tag(PropertyOp::LessThan, string_value("g")), vec!["both"]),
        (
            tag(PropertyOp::In, strings(&["learn", "fun"])),
            vec!["both", "one"],
        ),
        (
            tag(PropertyOp::NotEqual, string_value("fun")),
            vec!["both", "one", "scalar"],
        ),
        (
            tag(PropertyOp::NotIn, strings(&["fun", "learn"])),
            vec!["both", "scalar"],
        ),
    ] {
        let got = query_names(&mut client, kind, filter.clone())
            .await
            .unwrap_or_else(|status| panic!("{filter:?} was refused: {status}"));
        assert_eq!(got, want, "{filter:?}");
    }

    server.shutdown().await.expect("clean shutdown");
}

/// Where the proto's "Requires" list rules a filter out, the answer is
/// `INVALID_ARGUMENT`, not an empty result.
#[tokio::test]
async fn filters_the_proto_rules_out_are_invalid_arguments() {
    let (server, mut client) = start().await;

    let not_equal = || property_filter("priority", PropertyOp::NotEqual, integer_value(2));
    let not_in = || property_filter("priority", PropertyOp::NotIn, integers(&[1]));
    let is_in = || property_filter("priority", PropertyOp::In, integers(&[1]));

    for (what, filter) in [
        (
            "two NOT_EQUAL",
            composite(CompositeOp::And, vec![not_equal(), not_equal()]),
        ),
        (
            "NOT_EQUAL and NOT_IN",
            composite(CompositeOp::And, vec![not_equal(), not_in()]),
        ),
        (
            "NOT_IN and IN",
            composite(CompositeOp::And, vec![not_in(), is_in()]),
        ),
        (
            "NOT_IN under OR",
            composite(CompositeOp::Or, vec![not_in(), is_in()]),
        ),
        (
            "IN on a scalar",
            property_filter("priority", PropertyOp::In, integer_value(1)),
        ),
        (
            "IN on an empty array",
            property_filter("priority", PropertyOp::In, integers(&[])),
        ),
        (
            "NOT_IN of 11 values",
            property_filter("priority", PropertyOp::NotIn, integers(&[0; 11])),
        ),
        (
            "no operator",
            property_filter("priority", PropertyOp::Unspecified, integer_value(1)),
        ),
        (
            "no composite operator",
            composite(CompositeOp::Unspecified, vec![is_in()]),
        ),
    ] {
        let status = query_names(&mut client, "Refused", filter)
            .await
            .expect_err(what);
        assert_eq!(status.code(), Code::InvalidArgument, "{what}: {status}");
    }

    server.shutdown().await.expect("clean shutdown");
}

fn key_value(key: Key) -> Value {
    Value {
        value_type: Some(ValueType::KeyValue(key)),
        ..Default::default()
    }
}

fn path_key(path: &[(&str, IdType)]) -> Key {
    Key {
        partition_id: None,
        path: path
            .iter()
            .map(|(kind, id)| PathElement {
                kind: kind.to_string(),
                id_type: Some(id.clone()),
            })
            .collect(),
    }
}

/// Seeds kind `Keyed` with two numeric ids, two names, and a child of id 9,
/// so the public key order differs from the document name's string order in
/// two places: `10` after `9`, and the child between them.
async fn seed_keyed(client: &mut DatastoreClient<Channel>) {
    let kind = "Keyed";
    let keys = [
        path_key(&[(kind, IdType::Id(10))]),
        path_key(&[(kind, IdType::Name("b".into()))]),
        path_key(&[(kind, IdType::Id(9)), (kind, IdType::Name("child".into()))]),
        path_key(&[(kind, IdType::Name("a".into()))]),
        path_key(&[(kind, IdType::Id(9))]),
    ];
    let mutations = keys
        .into_iter()
        .map(|key| Mutation {
            operation: Some(Operation::Upsert(entity(key, HashMap::new()))),
            ..Default::default()
        })
        .collect();
    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations,
            ..Default::default()
        })
        .await
        .expect("seed");
}

/// A `__key__` order sorts in the public key order: element by element, a
/// numeric id before a name, ids as numbers, a key before its descendants.
#[tokio::test]
async fn a_query_orders_by_key_in_the_public_key_order() {
    let (server, mut client) = start().await;
    seed_keyed(&mut client).await;

    let ascending = ["9", "child", "10", "a", "b"];
    for (direction, want) in [
        (Direction::Ascending, ascending.to_vec()),
        (
            Direction::Descending,
            ascending.iter().rev().copied().collect(),
        ),
    ] {
        let batch = client
            .run_query(RunQueryRequest {
                project_id: PROJECT.to_string(),
                query_type: Some(QueryType::Query(Query {
                    kind: vec![KindExpression {
                        name: "Keyed".to_string(),
                    }],
                    order: vec![PropertyOrder {
                        property: Some(PropertyReference {
                            name: "__key__".to_string(),
                        }),
                        direction: direction as i32,
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            })
            .await
            .expect("query")
            .into_inner()
            .batch
            .expect("batch");
        let got: Vec<String> = batch
            .entity_results
            .into_iter()
            .filter_map(|result| result.entity?.key?.path.pop()?.id_type)
            .map(|id| match id {
                IdType::Name(name) => name,
                IdType::Id(id) => id.to_string(),
            })
            .collect();
        assert_eq!(got, want, "{direction:?}");
    }

    server.shutdown().await.expect("clean shutdown");
}

/// Every operator a property filter takes is served on `__key__`, under the
/// same key order.
#[tokio::test]
async fn key_filters_are_served_in_the_public_key_order() {
    let (server, mut client) = start().await;
    seed_keyed(&mut client).await;

    let id = |id| key_value(path_key(&[("Keyed", IdType::Id(id))]));
    let name = |name: &str| key_value(path_key(&[("Keyed", IdType::Name(name.into()))]));
    let by_key = |op, value| property_filter("__key__", op, value);
    let keys = |values: Vec<Value>| Value {
        value_type: Some(ValueType::ArrayValue(ArrayValue { values })),
        ..Default::default()
    };

    for (filter, want) in [
        (by_key(PropertyOp::Equal, id(10)), vec!["10"]),
        (by_key(PropertyOp::Equal, name("a")), vec!["a"]),
        // As strings, "10" < "9"; as ids, 10 > 9. The child of 9 is above 9.
        (
            by_key(PropertyOp::GreaterThan, id(9)),
            vec!["10", "a", "b", "child"],
        ),
        // Every id precedes every name.
        (
            by_key(PropertyOp::LessThan, name("a")),
            vec!["10", "9", "child"],
        ),
        (by_key(PropertyOp::LessThanOrEqual, id(9)), vec!["9"]),
        (
            by_key(PropertyOp::NotEqual, name("a")),
            vec!["10", "9", "b", "child"],
        ),
        (
            by_key(PropertyOp::In, keys(vec![id(9), name("b")])),
            vec!["9", "b"],
        ),
        (
            by_key(PropertyOp::NotIn, keys(vec![id(9), name("b")])),
            vec!["10", "a", "child"],
        ),
        // Two ranges on `__key__` under AND, which the per-element grouping of
        // property ranges must leave alone.
        (
            composite(
                CompositeOp::And,
                vec![
                    by_key(PropertyOp::GreaterThanOrEqual, id(10)),
                    by_key(PropertyOp::LessThan, name("b")),
                ],
            ),
            vec!["10", "a"],
        ),
        (
            composite(
                CompositeOp::Or,
                vec![
                    by_key(PropertyOp::Equal, id(9)),
                    by_key(PropertyOp::Equal, name("b")),
                ],
            ),
            vec!["9", "b"],
        ),
    ] {
        let got = query_names(&mut client, "Keyed", filter.clone())
            .await
            .unwrap_or_else(|status| panic!("{filter:?} was refused: {status}"));
        assert_eq!(got, want, "{filter:?}");
    }

    server.shutdown().await.expect("clean shutdown");
}

/// A `__key__` filter whose value is not a complete key in this database is
/// refused, rather than matching nothing.
#[tokio::test]
async fn a_key_filter_needs_a_complete_key_in_this_database() {
    let (server, mut client) = start().await;

    let mut elsewhere = named_key("Keyed", "a");
    elsewhere.partition_id = Some(PartitionId {
        project_id: "another-project".to_string(),
        ..Default::default()
    });
    for (what, filter) in [
        (
            "a string",
            property_filter("__key__", PropertyOp::Equal, string_value("a")),
        ),
        (
            "an IN of strings",
            property_filter(
                "__key__",
                PropertyOp::In,
                Value {
                    value_type: Some(ValueType::ArrayValue(ArrayValue {
                        values: vec![string_value("a")],
                    })),
                    ..Default::default()
                },
            ),
        ),
        (
            "an incomplete key",
            property_filter(
                "__key__",
                PropertyOp::GreaterThan,
                key_value(incomplete_key("Keyed")),
            ),
        ),
        (
            "a key in another project",
            property_filter("__key__", PropertyOp::Equal, key_value(elsewhere)),
        ),
    ] {
        let status = query_names(&mut client, "Keyed", filter)
            .await
            .expect_err(what);
        assert_eq!(status.code(), Code::InvalidArgument, "{what}: {status}");
    }

    server.shutdown().await.expect("clean shutdown");
}

/// One `RunQuery` over kind `Paged`, ordered by `priority` when `by_priority`.
async fn run_paged(
    client: &mut DatastoreClient<Channel>,
    by_priority: bool,
    shape: impl FnOnce(&mut Query),
) -> Result<QueryResultBatch, tonic::Status> {
    let mut query = Query {
        kind: vec![KindExpression {
            name: "Paged".to_string(),
        }],
        ..Default::default()
    };
    if by_priority {
        query.order.push(PropertyOrder {
            property: Some(PropertyReference {
                name: "priority".to_string(),
            }),
            direction: Direction::Ascending as i32,
        });
    }
    shape(&mut query);
    Ok(client
        .run_query(RunQueryRequest {
            project_id: PROJECT.to_string(),
            query_type: Some(QueryType::Query(query)),
            ..Default::default()
        })
        .await?
        .into_inner()
        .batch
        .expect("batch"))
}

/// The names a batch returned, in the order it returned them.
fn batch_names(batch: &QueryResultBatch) -> Vec<String> {
    batch
        .entity_results
        .iter()
        .filter_map(|result| {
            result
                .entity
                .as_ref()?
                .key
                .as_ref()?
                .path
                .last()?
                .id_type
                .clone()
        })
        .map(|id| match id {
            IdType::Name(name) => name,
            IdType::Id(id) => id.to_string(),
        })
        .collect()
}

/// Seeds kind `Paged` with `a`..`e`, three of them tied on `priority`, so a
/// page boundary falls inside the tie.
async fn seed_paged(client: &mut DatastoreClient<Channel>) {
    let mutations = [("e", 3), ("d", 2), ("c", 2), ("b", 2), ("a", 1)]
        .into_iter()
        .map(|(name, priority)| Mutation {
            operation: Some(Operation::Upsert(entity(
                named_key("Paged", name),
                HashMap::from([("priority".to_string(), integer_value(priority))]),
            ))),
            ..Default::default()
        })
        .collect();
    client
        .commit(CommitRequest {
            project_id: PROJECT.to_string(),
            mode: CommitMode::NonTransactional as i32,
            mutations,
            ..Default::default()
        })
        .await
        .expect("seed");
}

/// A limited query used to answer an empty `end_cursor` and
/// `NO_MORE_RESULTS`, so a client asking for page 2 got page 1 again. Each
/// page now ends with a cursor that resumes just after it, ties broken by
/// key, and says whether the limit cut the answer short.
#[tokio::test]
async fn end_cursors_page_through_a_limited_query() {
    let (server, mut client) = start().await;
    seed_paged(&mut client).await;

    for by_priority in [true, false] {
        let mut pages = Vec::new();
        let mut cursor = Vec::new();
        loop {
            let start = cursor.clone();
            let batch = run_paged(&mut client, by_priority, |query| {
                query.limit = Some(2);
                query.start_cursor = start;
            })
            .await
            .expect("page");
            assert!(!batch.end_cursor.is_empty(), "an end cursor is never empty");
            pages.push((batch_names(&batch), batch.more_results));
            if batch.more_results == MoreResultsType::NoMoreResults as i32 {
                break;
            }
            cursor = batch.end_cursor;
        }
        let after_limit = MoreResultsType::MoreResultsAfterLimit as i32;
        let none = MoreResultsType::NoMoreResults as i32;
        assert_eq!(
            pages,
            vec![
                (vec!["a".to_string(), "b".to_string()], after_limit),
                (vec!["c".to_string(), "d".to_string()], after_limit),
                (vec!["e".to_string()], none),
            ],
            "by_priority={by_priority}"
        );
    }

    server.shutdown().await.expect("clean shutdown");
}

/// A cursor is a position, so it still resumes after its entity is deleted
/// or moved, and an `end_cursor` stops the answer just after its entity.
#[tokio::test]
async fn a_cursor_is_a_position_not_an_entity() {
    let (server, mut client) = start().await;
    seed_paged(&mut client).await;

    let first = run_paged(&mut client, true, |query| query.limit = Some(2))
        .await
        .expect("first page");
    assert_eq!(batch_names(&first), ["a", "b"]);
    client
        .commit(commit_one(Mutation {
            operation: Some(Operation::Delete(named_key("Paged", "b"))),
            ..Default::default()
        }))
        .await
        .expect("delete b");
    let rest = run_paged(&mut client, true, |query| {
        query.start_cursor = first.end_cursor.clone();
    })
    .await
    .expect("rest");
    assert_eq!(batch_names(&rest), ["c", "d", "e"]);
    assert_eq!(rest.more_results, MoreResultsType::NoMoreResults as i32);

    // Each result carries the cursor just after it.
    let after_c = rest.entity_results[0].cursor.clone();
    let upto_c = run_paged(&mut client, true, |query| {
        query.end_cursor = after_c.clone()
    })
    .await
    .expect("up to c");
    assert_eq!(batch_names(&upto_c), ["a", "c"]);
    assert_eq!(
        upto_c.more_results,
        MoreResultsType::MoreResultsAfterCursor as i32
    );

    let between = run_paged(&mut client, true, |query| {
        query.start_cursor = first.end_cursor.clone();
        query.end_cursor = after_c.clone();
    })
    .await
    .expect("between");
    assert_eq!(batch_names(&between), ["c"]);

    server.shutdown().await.expect("clean shutdown");
}

/// An offset that skips everything still answers a cursor to resume from,
/// and counts only what it actually skipped.
#[tokio::test]
async fn an_offset_reports_what_it_skipped_and_where_it_stopped() {
    let (server, mut client) = start().await;
    seed_paged(&mut client).await;

    let skipped = run_paged(&mut client, true, |query| query.offset = 3)
        .await
        .expect("offset 3");
    assert_eq!(batch_names(&skipped), ["d", "e"]);
    assert_eq!(skipped.skipped_results, 3);
    let after_skip = run_paged(&mut client, true, |query| {
        query.start_cursor = skipped.skipped_cursor.clone();
    })
    .await
    .expect("after the skipped cursor");
    assert_eq!(batch_names(&after_skip), ["d", "e"]);

    let past_the_end = run_paged(&mut client, true, |query| query.offset = 10)
        .await
        .expect("offset 10");
    assert!(past_the_end.entity_results.is_empty());
    assert_eq!(past_the_end.skipped_results, 5);
    let resumed = run_paged(&mut client, true, |query| {
        query.start_cursor = past_the_end.end_cursor.clone();
    })
    .await
    .expect("resume past the end");
    assert!(resumed.entity_results.is_empty());

    let empty = run_paged(&mut client, true, |query| query.limit = Some(0))
        .await
        .expect("limit 0");
    assert!(!empty.end_cursor.is_empty());
    let from_start = run_paged(&mut client, true, |query| {
        query.start_cursor = empty.end_cursor.clone();
    })
    .await
    .expect("from the start cursor");
    assert_eq!(batch_names(&from_start), ["a", "b", "c", "d", "e"]);

    server.shutdown().await.expect("clean shutdown");
}

/// A cursor Fireup did not write is refused, never read as "no cursor".
#[tokio::test]
async fn a_foreign_cursor_is_an_invalid_argument() {
    let (server, mut client) = start().await;
    seed_paged(&mut client).await;

    for (which, bytes) in [
        ("start", b"a-cursor".to_vec()),
        ("end", b"a-cursor".to_vec()),
    ] {
        let error = run_paged(&mut client, true, |query| match which {
            "start" => query.start_cursor = bytes,
            _ => query.end_cursor = bytes,
        })
        .await
        .expect_err("a foreign cursor");
        assert_eq!(error.code(), Code::InvalidArgument, "{which}: {error:?}");
        assert!(error.message().contains(which), "{which}: {error:?}");
    }

    server.shutdown().await.expect("clean shutdown");
}
