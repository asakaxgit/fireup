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
use fireup::protos::google::datastore::v1::property_transform::{ServerValue, TransformType};
use fireup::protos::google::datastore::v1::run_query_request::QueryType;
use fireup::protos::google::datastore::v1::value::ValueType;
use fireup::protos::google::datastore::v1::{
    filter::FilterType, AllocateIdsRequest, ArrayValue, BeginTransactionRequest, CommitRequest,
    Entity, Filter, Key, KindExpression, LookupRequest, Mutation, PropertyFilter,
    PropertyReference, PropertyTransform, Query, RollbackRequest, RunAggregationQueryRequest,
    RunQueryRequest, Value,
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
