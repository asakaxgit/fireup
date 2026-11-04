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
    firestore_client::FirestoreClient, precondition::ConditionType, value::ValueType,
    write::Operation, BatchGetDocumentsRequest, CommitRequest, CreateDocumentRequest,
    DeleteDocumentRequest, Document, DocumentMask, GetDocumentRequest, Precondition, RunQueryRequest,
    Value, Write,
};
use tonic::transport::Channel;
use tonic::Code;

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
    let server = local::spawn("127.0.0.1:0".parse().expect("valid bind address"))
        .await
        .expect("server binds");
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

    assert!(server.store.is_empty());
    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn unserved_rpcs_answer_unimplemented_rather_than_hanging() {
    let (server, mut client) = start().await;

    let status = client
        .run_query(RunQueryRequest {
            parent: ROOT.to_string(),
            ..Default::default()
        })
        .await
        .expect_err("RunQuery is out of Phase 0 scope");

    assert_eq!(status.code(), Code::Unimplemented);
    assert!(
        status.message().contains("fireup local"),
        "the status should say who refused and why: {}",
        status.message()
    );

    server.shutdown().await.expect("clean shutdown");
}
