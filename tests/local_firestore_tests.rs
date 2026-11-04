//! `fireup local` Firestore data-plane tests.
//!
//! These drive the server over real gRPC on a loopback port using the stubs
//! generated from the public `googleapis` protos, so they exercise the same wire
//! path an official SDK uses. The separate SDK smoke test
//! (`scripts/smoke-local-firestore.mjs`) proves an actual client library can
//! talk to it; this file is the fast check that runs in `cargo test`.

use std::collections::HashMap;

use fireup::local;
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient, precondition::ConditionType,
    run_query_request::QueryType, structured_query::field_filter::Operator as FieldOperator,
    structured_query::filter::FilterType, structured_query::CollectionSelector,
    structured_query::Direction, structured_query::FieldFilter, structured_query::FieldReference,
    structured_query::Filter, structured_query::Order, value::ValueType, write::Operation,
    BatchGetDocumentsRequest, BatchWriteRequest, BeginTransactionRequest, CommitRequest,
    CreateDocumentRequest, DeleteDocumentRequest,
    Document, DocumentMask, GetDocumentRequest, ListCollectionIdsRequest, ListDocumentsRequest,
    ListDocumentsResponse, PartitionQueryRequest, Precondition, RollbackRequest,
    RunAggregationQueryRequest, RunQueryRequest, StructuredQuery, Value, Write,
};
use tonic::transport::Channel;
use tonic::Code;

const DATABASE: &str = "projects/fireup-test/databases/(default)";
const ROOT: &str = "projects/fireup-test/databases/(default)/documents";

fn string_value(text: &str) -> Value {
    Value {
        value_type: Some(ValueType::StringValue(text.to_string())),
    }
}

fn integer_value(n: i64) -> Value {
    Value {
        value_type: Some(ValueType::IntegerValue(n)),
    }
}

fn fields<const N: usize>(pairs: [(&str, Value); N]) -> HashMap<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Start a server on an OS-assigned port and return a connected client.
async fn start() -> (local::RunningServer, FirestoreClient<Channel>) {
    let config =
        local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"));
    let server = local::spawn(config).await.expect("server binds");
    let client = FirestoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects to the bound port");
    (server, client)
}

#[tokio::test]
async fn create_then_get_round_trips_a_document() {
    let (server, mut client) = start().await;

    let created = client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                fields: fields([("name", string_value("Alice")), ("age", integer_value(31))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("CreateDocument succeeds")
        .into_inner();

    assert_eq!(created.name, format!("{ROOT}/users/alice"));
    assert!(created.create_time.is_some(), "server stamps create_time");
    assert_eq!(created.create_time, created.update_time);

    let fetched = client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/users/alice"),
            ..Default::default()
        })
        .await
        .expect("GetDocument succeeds")
        .into_inner();

    assert_eq!(fetched.fields["name"], string_value("Alice"));
    assert_eq!(fetched.fields["age"], integer_value(31));
    assert_eq!(fetched.create_time, created.create_time);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn create_assigns_an_id_when_the_client_omits_one() {
    let (server, mut client) = start().await;

    let created = client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: String::new(),
            document: Some(Document {
                fields: fields([("name", string_value("Anon"))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("CreateDocument succeeds without a document_id")
        .into_inner();

    let prefix = format!("{ROOT}/users/");
    let id = created
        .name
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("auto id should be under {prefix}, got {}", created.name));
    assert_eq!(id.len(), 20, "Firestore auto ids are 20 characters: {id}");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn get_on_a_missing_document_is_not_found() {
    let (server, mut client) = start().await;

    let status = client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/users/nobody"),
            ..Default::default()
        })
        .await
        .expect_err("missing document");

    assert_eq!(status.code(), Code::NotFound);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn create_on_an_existing_document_already_exists() {
    let (server, mut client) = start().await;

    let request = CreateDocumentRequest {
        parent: ROOT.to_string(),
        collection_id: "users".to_string(),
        document_id: "alice".to_string(),
        document: Some(Document {
            fields: fields([("name", string_value("Alice"))]),
            ..Default::default()
        }),
        ..Default::default()
    };

    client
        .create_document(request.clone())
        .await
        .expect("first create succeeds");
    let status = client
        .create_document(request)
        .await
        .expect_err("second create on the same id");

    assert_eq!(status.code(), Code::AlreadyExists);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_collection_path_is_rejected_as_a_document_name() {
    let (server, mut client) = start().await;

    let status = client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/users"),
            ..Default::default()
        })
        .await
        .expect_err("odd segment count names a collection");

    assert_eq!(status.code(), Code::InvalidArgument);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_response_mask_limits_returned_fields() {
    let (server, mut client) = start().await;

    client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                fields: fields([("name", string_value("Alice")), ("age", integer_value(31))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create succeeds");

    let fetched = client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/users/alice"),
            mask: Some(DocumentMask {
                field_paths: vec!["name".to_string()],
            }),
            ..Default::default()
        })
        .await
        .expect("masked get succeeds")
        .into_inner();

    assert!(fetched.fields.contains_key("name"));
    assert!(!fetched.fields.contains_key("age"), "mask excludes age");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn batch_get_reports_found_and_missing_documents() {
    let (server, mut client) = start().await;

    client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                fields: fields([("name", string_value("Alice"))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create succeeds");

    let mut stream = client
        .batch_get_documents(BatchGetDocumentsRequest {
            database: "projects/fireup-test/databases/(default)".to_string(),
            documents: vec![
                format!("{ROOT}/users/alice"),
                format!("{ROOT}/users/nobody"),
            ],
            ..Default::default()
        })
        .await
        .expect("BatchGetDocuments succeeds")
        .into_inner();

    let mut found = Vec::new();
    let mut missing = Vec::new();
    while let Some(response) = stream.message().await.expect("stream item") {
        use fireup::protos::google::firestore::v1::batch_get_documents_response::Result as BatchResult;
        match response.result.expect("result is set") {
            BatchResult::Found(document) => found.push(document.name),
            BatchResult::Missing(name) => missing.push(name),
        }
    }

    assert_eq!(found, vec![format!("{ROOT}/users/alice")]);
    assert_eq!(missing, vec![format!("{ROOT}/users/nobody")]);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn commit_applies_set_merge_and_delete_writes() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    // Full set: no update_mask, so all fields are replaced.
    client
        .commit(CommitRequest {
            database: "projects/fireup-test/databases/(default)".to_string(),
            writes: vec![Write {
                operation: Some(Operation::Update(Document {
                    name: name.clone(),
                    fields: fields([("name", string_value("Alice")), ("age", integer_value(31))]),
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("set commit succeeds");

    // Merge: update_mask names one field, the rest must survive.
    client
        .commit(CommitRequest {
            database: "projects/fireup-test/databases/(default)".to_string(),
            writes: vec![Write {
                operation: Some(Operation::Update(Document {
                    name: name.clone(),
                    fields: fields([("age", integer_value(32))]),
                    ..Default::default()
                })),
                update_mask: Some(DocumentMask {
                    field_paths: vec!["age".to_string()],
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("merge commit succeeds");

    let fetched = client
        .get_document(GetDocumentRequest {
            name: name.clone(),
            ..Default::default()
        })
        .await
        .expect("get after merge")
        .into_inner();
    assert_eq!(fetched.fields["age"], integer_value(32));
    assert_eq!(
        fetched.fields["name"],
        string_value("Alice"),
        "merge must not drop unmasked fields"
    );

    // Delete.
    let response = client
        .commit(CommitRequest {
            database: "projects/fireup-test/databases/(default)".to_string(),
            writes: vec![Write {
                operation: Some(Operation::Delete(name.clone())),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("delete commit succeeds")
        .into_inner();
    assert_eq!(response.write_results.len(), 1);
    assert!(response.commit_time.is_some());

    let status = client
        .get_document(GetDocumentRequest {
            name,
            ..Default::default()
        })
        .await
        .expect_err("document is gone");
    assert_eq!(status.code(), Code::NotFound);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn commit_enforces_the_exists_precondition_both_ways() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let write_with = |exists: bool| Write {
        operation: Some(Operation::Update(Document {
            name: name.clone(),
            fields: fields([("name", string_value("Alice"))]),
            ..Default::default()
        })),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::Exists(exists)),
        }),
        ..Default::default()
    };
    let commit = |write: Write| CommitRequest {
        database: "projects/fireup-test/databases/(default)".to_string(),
        writes: vec![write],
        ..Default::default()
    };

    // `update()` on a missing document → NOT_FOUND.
    let status = client
        .commit(commit(write_with(true)))
        .await
        .expect_err("exists=true on a missing document");
    assert_eq!(status.code(), Code::NotFound);

    // `create()` on a missing document → ok.
    client
        .commit(commit(write_with(false)))
        .await
        .expect("exists=false on a missing document");

    // `create()` again → ALREADY_EXISTS.
    let status = client
        .commit(commit(write_with(false)))
        .await
        .expect_err("exists=false on an existing document");
    assert_eq!(status.code(), Code::AlreadyExists);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn delete_document_is_idempotent() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document::default()),
            ..Default::default()
        })
        .await
        .expect("create succeeds");

    for attempt in 1..=2 {
        client
            .delete_document(DeleteDocumentRequest {
                name: name.clone(),
                ..Default::default()
            })
            .await
            .unwrap_or_else(|e| panic!("delete attempt {attempt} should succeed: {e}"));
    }

    assert_eq!(server.store.count().await.expect("count"), 0);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn unserved_rpcs_answer_unimplemented_rather_than_hanging() {
    let (server, mut client) = start().await;

    // One assertion per RPC that is still out of scope. As each one lands, this
    // test fails until it is moved out of the list — which is the checklist.
    let aggregation = client
        .run_aggregation_query(RunAggregationQueryRequest {
            parent: ROOT.to_string(),
            ..Default::default()
        })
        .await
        .expect_err("RunAggregationQuery is out of scope");
    let partition = client
        .partition_query(PartitionQueryRequest {
            parent: ROOT.to_string(),
            ..Default::default()
        })
        .await
        .expect_err("PartitionQuery is out of scope");

    for status in [aggregation, partition] {
        assert_eq!(status.code(), Code::Unimplemented);
        assert!(
            status.message().contains("fireup local"),
            "the status should say who refused and why: {}",
            status.message()
        );
    }

    server.shutdown().await.expect("clean shutdown");
}

/// Write `documents` with one commit, so a test can set up a collection in a
/// single call.
async fn seed(client: &mut FirestoreClient<Channel>, documents: Vec<Document>) {
    client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            writes: documents
                .into_iter()
                .map(|document| Write {
                    operation: Some(Operation::Update(document)),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .await
        .expect("seeding should succeed");
}

fn person(id: &str, city: &str, age: i64) -> Document {
    Document {
        name: format!("{ROOT}/people/{id}"),
        fields: fields([("city", string_value(city)), ("age", integer_value(age))]),
        ..Default::default()
    }
}

fn from(collection_id: &str) -> Vec<CollectionSelector> {
    vec![CollectionSelector {
        collection_id: collection_id.to_string(),
        all_descendants: false,
    }]
}

fn equals(field_path: &str, value: Value) -> Option<Filter> {
    Some(Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference {
                field_path: field_path.to_string(),
            }),
            op: FieldOperator::Equal as i32,
            value: Some(value),
        })),
    })
}

/// Run a query and collect the document ids it returned, in order.
async fn run_query(
    client: &mut FirestoreClient<Channel>,
    query: StructuredQuery,
) -> Result<Vec<String>, tonic::Status> {
    let mut stream = client
        .run_query(RunQueryRequest {
            parent: ROOT.to_string(),
            query_type: Some(QueryType::StructuredQuery(query)),
            ..Default::default()
        })
        .await?
        .into_inner();

    let mut ids = Vec::new();
    while let Some(response) = stream.message().await? {
        assert!(
            response.read_time.is_some(),
            "every response carries the read time"
        );
        if let Some(document) = response.document {
            ids.push(
                document
                    .name
                    .rsplit('/')
                    .next()
                    .expect("a name has segments")
                    .to_string(),
            );
        }
    }
    Ok(ids)
}

#[tokio::test]
async fn run_query_filters_orders_and_limits() {
    let (server, mut client) = start().await;
    seed(
        &mut client,
        vec![
            person("alice", "Tokyo", 31),
            person("bob", "Kyoto", 41),
            person("carol", "Tokyo", 25),
            person("dan", "Tokyo", 55),
        ],
    )
    .await;

    let found = run_query(
        &mut client,
        StructuredQuery {
            from: from("people"),
            r#where: equals("city", string_value("Tokyo")),
            order_by: vec![Order {
                field: Some(FieldReference {
                    field_path: "age".to_string(),
                }),
                direction: Direction::Descending as i32,
            }],
            limit: Some(2),
            ..Default::default()
        },
    )
    .await
    .expect("the query runs");

    assert_eq!(found, vec!["dan", "alice"], "Tokyo, oldest first, top two");
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn run_query_does_not_look_inside_subcollections() {
    let (server, mut client) = start().await;
    seed(
        &mut client,
        vec![
            person("alice", "Tokyo", 31),
            Document {
                name: format!("{ROOT}/people/alice/pets/rex"),
                fields: fields([("city", string_value("Tokyo")), ("age", integer_value(3))]),
                ..Default::default()
            },
        ],
    )
    .await;

    let found = run_query(
        &mut client,
        StructuredQuery {
            from: from("people"),
            r#where: equals("city", string_value("Tokyo")),
            ..Default::default()
        },
    )
    .await
    .expect("the query runs");

    assert_eq!(found, vec!["alice"], "a pet is not a person");
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn run_query_with_no_matches_still_answers() {
    let (server, mut client) = start().await;
    seed(&mut client, vec![person("alice", "Tokyo", 31)]).await;

    let found = run_query(
        &mut client,
        StructuredQuery {
            from: from("people"),
            r#where: equals("city", string_value("Nowhere")),
            ..Default::default()
        },
    )
    .await
    .expect("an empty result is not an error");

    // `run_query` asserts that every response carries a read_time, so arriving
    // here with no ids means the server said "ran it, found nothing".
    assert!(found.is_empty());
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn run_query_rejects_a_collection_group_query_clearly() {
    let (server, mut client) = start().await;

    let status = run_query(
        &mut client,
        StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "people".to_string(),
                all_descendants: true,
            }],
            ..Default::default()
        },
    )
    .await
    .expect_err("collection group queries are not served");

    assert_eq!(status.code(), Code::Unimplemented);
    assert!(
        status.message().contains("all_descendants"),
        "the message should name the unsupported feature: {}",
        status.message()
    );
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn list_documents_pages_with_a_resume_token() {
    let (server, mut client) = start().await;
    seed(
        &mut client,
        vec![
            person("alice", "Tokyo", 31),
            person("bob", "Kyoto", 41),
            person("carol", "Tokyo", 25),
        ],
    )
    .await;

    let mut seen = Vec::new();
    let mut page_token = String::new();
    loop {
        let page = client
            .list_documents(ListDocumentsRequest {
                parent: ROOT.to_string(),
                collection_id: "people".to_string(),
                page_size: 2,
                page_token: page_token.clone(),
                ..Default::default()
            })
            .await
            .expect("ListDocuments succeeds")
            .into_inner();

        seen.extend(page.documents.iter().map(|document| document.name.clone()));
        page_token = page.next_page_token;
        if page_token.is_empty() {
            break;
        }
        assert!(seen.len() <= 3, "paging must terminate");
    }

    assert_eq!(
        seen,
        vec![
            format!("{ROOT}/people/alice"),
            format!("{ROOT}/people/bob"),
            format!("{ROOT}/people/carol"),
        ]
    );
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn list_documents_reports_documents_that_exist_only_as_parents() {
    let (server, mut client) = start().await;
    // `ghost` itself is never written — only a document in its subcollection
    // is. An SDK's listDocuments() has to see it anyway.
    seed(
        &mut client,
        vec![
            person("alice", "Tokyo", 31),
            Document {
                name: format!("{ROOT}/people/ghost/posts/p1"),
                fields: fields([("title", string_value("Boo"))]),
                ..Default::default()
            },
        ],
    )
    .await;

    let request = |show_missing: bool| ListDocumentsRequest {
        parent: ROOT.to_string(),
        collection_id: "people".to_string(),
        show_missing,
        ..Default::default()
    };
    let names = |response: ListDocumentsResponse| {
        response
            .documents
            .iter()
            .map(|document| document.name.clone())
            .collect::<Vec<String>>()
    };

    let without = client
        .list_documents(request(false))
        .await
        .expect("ListDocuments succeeds")
        .into_inner();
    assert_eq!(names(without), vec![format!("{ROOT}/people/alice")]);

    let with = client
        .list_documents(request(true))
        .await
        .expect("ListDocuments succeeds")
        .into_inner();
    let ghost = with
        .documents
        .iter()
        .find(|document| document.name.ends_with("/ghost"))
        .expect("the missing document is listed")
        .clone();
    assert_eq!(
        names(with),
        vec![
            format!("{ROOT}/people/alice"),
            format!("{ROOT}/people/ghost"),
        ]
    );
    assert!(ghost.fields.is_empty(), "a missing document has no fields");
    assert!(
        ghost.create_time.is_none() && ghost.update_time.is_none(),
        "a missing document has no timestamps"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn list_collection_ids_sees_root_and_nested_collections() {
    let (server, mut client) = start().await;
    seed(
        &mut client,
        vec![
            person("alice", "Tokyo", 31),
            Document {
                name: format!("{ROOT}/people/alice/posts/p1"),
                ..Default::default()
            },
            Document {
                name: format!("{ROOT}/places/tokyo"),
                ..Default::default()
            },
        ],
    )
    .await;

    let root = client
        .list_collection_ids(ListCollectionIdsRequest {
            parent: ROOT.to_string(),
            ..Default::default()
        })
        .await
        .expect("ListCollectionIds succeeds")
        .into_inner();
    assert_eq!(root.collection_ids, vec!["people", "places"]);

    let nested = client
        .list_collection_ids(ListCollectionIdsRequest {
            parent: format!("{ROOT}/people/alice"),
            ..Default::default()
        })
        .await
        .expect("ListCollectionIds succeeds")
        .into_inner();
    assert_eq!(nested.collection_ids, vec!["posts"]);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn commit_applies_the_whole_batch_or_none_of_it() {
    let (server, mut client) = start().await;

    let status = client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            writes: vec![
                Write {
                    operation: Some(Operation::Update(person("alice", "Tokyo", 31))),
                    ..Default::default()
                },
                Write {
                    operation: Some(Operation::Update(person("ghost", "Nowhere", 0))),
                    // Nothing was ever written here, so this cannot hold.
                    current_document: Some(Precondition {
                        condition_type: Some(ConditionType::Exists(true)),
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .await
        .expect_err("the batch must fail");

    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(
        server.store.count().await.expect("count"),
        0,
        "a failed batch must not leave its first write applied"
    );
    server.shutdown().await.expect("clean shutdown");
}

/// Begin a transaction and return its token.
async fn begin(client: &mut FirestoreClient<Channel>) -> Vec<u8> {
    client
        .begin_transaction(BeginTransactionRequest {
            database: DATABASE.to_string(),
            ..Default::default()
        })
        .await
        .expect("BeginTransaction succeeds")
        .into_inner()
        .transaction
}

/// Read one document inside `transaction`, so it joins the read set.
async fn get_in_transaction(
    client: &mut FirestoreClient<Channel>,
    name: &str,
    transaction: &[u8],
) -> Result<Document, tonic::Status> {
    use fireup::protos::google::firestore::v1::get_document_request::ConsistencySelector;
    client
        .get_document(GetDocumentRequest {
            name: name.to_string(),
            consistency_selector: Some(ConsistencySelector::Transaction(transaction.to_vec())),
            ..Default::default()
        })
        .await
        .map(|response| response.into_inner())
}

#[tokio::test]
async fn a_transaction_reads_then_commits_its_writes() {
    let (server, mut client) = start().await;
    seed(&mut client, vec![person("alice", "Tokyo", 31)]).await;

    let transaction = begin(&mut client).await;
    let read = get_in_transaction(&mut client, &format!("{ROOT}/people/alice"), &transaction)
        .await
        .expect("a transactional read succeeds");
    assert_eq!(read.fields["age"], integer_value(31));

    client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            transaction: transaction.clone(),
            writes: vec![Write {
                operation: Some(Operation::Update(person("alice", "Tokyo", 32))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("the transaction commits");

    let after = client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/people/alice"),
            ..Default::default()
        })
        .await
        .expect("get after commit")
        .into_inner();
    assert_eq!(after.fields["age"], integer_value(32));

    // The token is spent, so replaying the same commit must not write again.
    let replay = client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            transaction,
            writes: Vec::new(),
            ..Default::default()
        })
        .await
        .expect_err("a transaction token works once");
    assert_eq!(replay.code(), Code::InvalidArgument);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_transaction_is_aborted_when_a_document_it_read_changed() {
    let (server, mut client) = start().await;
    seed(&mut client, vec![person("alice", "Tokyo", 31)]).await;
    let name = format!("{ROOT}/people/alice");

    let transaction = begin(&mut client).await;
    get_in_transaction(&mut client, &name, &transaction)
        .await
        .expect("the transactional read succeeds");

    // Somebody else writes the document the transaction is working from.
    seed(&mut client, vec![person("alice", "Osaka", 99)]).await;

    let status = client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            transaction,
            writes: vec![Write {
                operation: Some(Operation::Update(person("alice", "Tokyo", 32))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect_err("the transaction must not commit on stale data");

    assert_eq!(
        status.code(),
        Code::Aborted,
        "ABORTED is what makes an SDK retry the transaction: {}",
        status.message()
    );

    let after = client
        .get_document(GetDocumentRequest {
            name,
            ..Default::default()
        })
        .await
        .expect("get after the aborted commit")
        .into_inner();
    assert_eq!(
        after.fields["age"],
        integer_value(99),
        "the aborted transaction must not have overwritten the other writer"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_transaction_that_created_a_document_aborts_if_someone_else_creates_it_first() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/people/alice");

    let transaction = begin(&mut client).await;
    // Read as absent — the fact this transaction is about to act on.
    let missing = get_in_transaction(&mut client, &name, &transaction)
        .await
        .expect_err("alice does not exist yet");
    assert_eq!(missing.code(), Code::NotFound);

    seed(&mut client, vec![person("alice", "Osaka", 99)]).await;

    let status = client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            transaction,
            writes: vec![Write {
                operation: Some(Operation::Update(person("alice", "Tokyo", 31))),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect_err("the document appeared after it was read as absent");

    assert_eq!(status.code(), Code::Aborted);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn rollback_spends_the_token_without_writing_anything() {
    let (server, mut client) = start().await;
    let transaction = begin(&mut client).await;

    client
        .rollback(RollbackRequest {
            database: DATABASE.to_string(),
            transaction: transaction.clone(),
            ..Default::default()
        })
        .await
        .expect("Rollback succeeds");

    let status = client
        .rollback(RollbackRequest {
            database: DATABASE.to_string(),
            transaction,
            ..Default::default()
        })
        .await
        .expect_err("the token is already spent");
    assert_eq!(status.code(), Code::InvalidArgument);

    assert_eq!(server.store.count().await.expect("count"), 0);
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn batch_write_reports_a_status_for_every_write() {
    let (server, mut client) = start().await;

    let response = client
        .batch_write(BatchWriteRequest {
            database: DATABASE.to_string(),
            writes: vec![
                Write {
                    operation: Some(Operation::Update(person("alice", "Tokyo", 31))),
                    ..Default::default()
                },
                Write {
                    operation: Some(Operation::Update(person("ghost", "Nowhere", 0))),
                    current_document: Some(Precondition {
                        condition_type: Some(ConditionType::Exists(true)),
                    }),
                    ..Default::default()
                },
                Write {
                    operation: Some(Operation::Update(person("bob", "Kyoto", 41))),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .await
        .expect("BatchWrite itself succeeds even when a write fails")
        .into_inner();

    assert_eq!(response.status.len(), 3);
    assert_eq!(response.write_results.len(), 3);
    assert_eq!(response.status[0].code, Code::Ok as i32);
    assert_eq!(
        response.status[1].code,
        Code::NotFound as i32,
        "the failing write reports its own status: {:?}",
        response.status[1]
    );
    assert_eq!(response.status[2].code, Code::Ok as i32);

    // Unlike Commit, a failed write does not take the others with it.
    assert_eq!(
        server.store.count().await.expect("count"),
        2,
        "BatchWrite is not atomic: the two good writes stay"
    );
    server.shutdown().await.expect("clean shutdown");
}

/// Send a raw HTTP/1.1 request to the server's port and return (status line, body).
async fn http_get(addr: std::net::SocketAddr, path: &str) -> (String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to the server port");
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .await
        .expect("write request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read response");

    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed response: {response:?}"));
    let status_line = head.lines().next().expect("status line").to_string();
    (status_line, body.to_string())
}

#[tokio::test]
async fn one_port_serves_both_grpc_and_the_http_surface() {
    let (server, mut client) = start().await;

    // gRPC first, so the HTTP request below is not just talking to an idle port.
    client
        .create_document(CreateDocumentRequest {
            parent: ROOT.to_string(),
            collection_id: "users".to_string(),
            document_id: "alice".to_string(),
            document: Some(Document {
                fields: fields([("email", string_value("alice@example.com"))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create over gRPC succeeds");

    let (status, body) = http_get(server.addr, "/health").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let health: serde_json::Value = serde_json::from_str(&body).expect("health body is JSON");
    assert_eq!(health["status"], "ok");
    assert_eq!(
        health["storage"]["documents"], 1,
        "health must report what the store actually holds: {body}"
    );

    let (status, body) = http_get(server.addr, "/").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let discovery: serde_json::Value = serde_json::from_str(&body).expect("discovery body is JSON");
    assert_eq!(discovery["services"][0]["id"], "firestore");
    assert_eq!(
        discovery["services"][0]["api"], "google.firestore.v1.Firestore",
        "discovery must name the API a client can call: {body}"
    );
    assert_eq!(discovery["storage"]["backend"], "memory");
    assert_eq!(discovery["storage"]["durable"], false);

    // The same connection path must still serve gRPC afterwards.
    client
        .get_document(GetDocumentRequest {
            name: format!("{ROOT}/users/alice"),
            ..Default::default()
        })
        .await
        .expect("gRPC still works after HTTP requests");

    let (status, _) = http_get(server.addr, "/not-a-route").await;
    assert!(status.starts_with("HTTP/1.1 404"), "{status}");

    server.shutdown().await.expect("clean shutdown");
}
