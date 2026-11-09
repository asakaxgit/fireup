//! `fireup local` Firestore `Listen` tests.
//!
//! These drive the real `google.firestore.v1.Firestore.Listen` bidirectional
//! stream over a loopback gRPC port, using the stubs generated from the public
//! `googleapis` protos — the same wire path an official SDK takes.
//!
//! Every test consumes the stream the way a client SDK does: responses
//! accumulate until a `TargetChange` of type `NO_CHANGE` with **no** target ids
//! arrives, which is the contract's "the stream is at a new consistent
//! snapshot" marker and the point at which an SDK hands a snapshot to the
//! application. Asserting on those batches rather than on individual messages
//! is what keeps these tests about observable behaviour.

use std::collections::HashMap;
use std::time::Duration;

use fireup::local;
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient,
    listen_request::TargetChange as RequestedChange,
    listen_response::ResponseType,
    structured_query::field_filter::Operator as FieldOperator,
    structured_query::filter::FilterType,
    structured_query::{CollectionSelector, FieldFilter, FieldReference, Filter},
    target::{query_target::QueryType, DocumentsTarget, QueryTarget, ResumeType, TargetType},
    target_change::TargetChangeType,
    value::ValueType,
    write::Operation,
    CommitRequest, DeleteDocumentRequest, Document, ListenRequest, ListenResponse, StructuredQuery,
    Target, Value, Write,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Code, Streaming};

const DATABASE: &str = "projects/fireup-test/databases/(default)";
const ROOT: &str = "projects/fireup-test/databases/(default)/documents";

/// Generous enough that a loaded CI box does not flake, short enough that a
/// listener which never fires fails the test instead of hanging it.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

fn string_value(text: &str) -> Value {
    Value {
        value_type: Some(ValueType::StringValue(text.to_string())),
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
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"));
    let server = local::spawn(config).await.expect("server binds");
    let client = FirestoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects to the bound port");
    (server, client)
}

/// A live `Listen` stream.
///
/// The request sender is held for as long as the stream is in use: dropping it
/// half-closes the request side, which is what a client does when it is done
/// adding targets.
struct Listener {
    requests: mpsc::Sender<ListenRequest>,
    responses: Streaming<ListenResponse>,
}

impl Listener {
    async fn open(client: &mut FirestoreClient<Channel>) -> Self {
        let (requests, inbound) = mpsc::channel(8);
        let responses = client
            .listen(ReceiverStream::new(inbound))
            .await
            .expect("Listen stream opens")
            .into_inner();
        Self {
            requests,
            responses,
        }
    }

    async fn send(&self, change: RequestedChange) {
        self.requests
            .send(ListenRequest {
                database: DATABASE.to_string(),
                target_change: Some(change),
                ..Default::default()
            })
            .await
            .expect("the server is still reading requests");
    }

    async fn add(&self, target: Target) {
        self.send(RequestedChange::AddTarget(target)).await;
    }

    async fn remove(&self, target_id: i32) {
        self.send(RequestedChange::RemoveTarget(target_id)).await;
    }

    /// Read responses up to the next whole-stream snapshot marker, returning
    /// everything that came before it.
    async fn snapshot(&mut self) -> Vec<ResponseType> {
        let mut batch = Vec::new();
        loop {
            let response = tokio::time::timeout(RESPONSE_TIMEOUT, self.responses.message())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "no snapshot marker within {RESPONSE_TIMEOUT:?}; batch so far: {batch:?}"
                    )
                })
                .expect("the stream stays healthy")
                .expect("the server does not end the stream");
            let response_type = response
                .response_type
                .expect("every ListenResponse carries a response_type");

            if is_snapshot_marker(&response_type) {
                return batch;
            }
            batch.push(response_type);
        }
    }

    /// Assert that nothing arrives for a while.
    ///
    /// A listener that fires on an unrelated write is as wrong as one that
    /// never fires, and only a negative check catches it.
    async fn expect_quiet(&mut self) {
        let waited =
            tokio::time::timeout(Duration::from_millis(300), self.responses.message()).await;
        if let Ok(message) = waited {
            panic!("expected no further responses, got {message:?}");
        }
    }
}

/// `NO_CHANGE` with no target ids and a `read_time` is the contract's
/// "everything is consistent as of now" marker.
fn is_snapshot_marker(response_type: &ResponseType) -> bool {
    match response_type {
        ResponseType::TargetChange(change) => {
            change.target_change_type() == TargetChangeType::NoChange
                && change.target_ids.is_empty()
                && change.read_time.is_some()
        }
        _ => false,
    }
}

fn target_changes(batch: &[ResponseType]) -> Vec<(TargetChangeType, Vec<i32>)> {
    batch
        .iter()
        .filter_map(|response| match response {
            ResponseType::TargetChange(change) => {
                Some((change.target_change_type(), change.target_ids.clone()))
            }
            _ => None,
        })
        .collect()
}

fn changed(batch: &[ResponseType]) -> Vec<Document> {
    batch
        .iter()
        .filter_map(|response| match response {
            ResponseType::DocumentChange(change) => change.document.clone(),
            _ => None,
        })
        .collect()
}

fn deleted(batch: &[ResponseType]) -> Vec<String> {
    batch
        .iter()
        .filter_map(|response| match response {
            ResponseType::DocumentDelete(delete) => Some(delete.document.clone()),
            _ => None,
        })
        .collect()
}

fn removed(batch: &[ResponseType]) -> Vec<String> {
    batch
        .iter()
        .filter_map(|response| match response {
            ResponseType::DocumentRemove(remove) => Some(remove.document.clone()),
            _ => None,
        })
        .collect()
}

fn documents_target(target_id: i32, names: &[&str]) -> Target {
    Target {
        target_id,
        target_type: Some(TargetType::Documents(DocumentsTarget {
            documents: names.iter().map(|name| name.to_string()).collect(),
        })),
        ..Default::default()
    }
}

fn query_target(target_id: i32, collection_id: &str, filter: Option<Filter>) -> Target {
    Target {
        target_id,
        target_type: Some(TargetType::Query(QueryTarget {
            parent: ROOT.to_string(),
            query_type: Some(QueryType::StructuredQuery(StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: collection_id.to_string(),
                    all_descendants: false,
                }],
                r#where: filter,
                ..Default::default()
            })),
        })),
        ..Default::default()
    }
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

/// Write one document with a `set()`-shaped commit.
async fn write(
    client: &mut FirestoreClient<Channel>,
    name: &str,
    document_fields: HashMap<String, Value>,
) {
    client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            writes: vec![Write {
                operation: Some(Operation::Update(Document {
                    name: name.to_string(),
                    fields: document_fields,
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("Commit succeeds");
}

async fn delete(client: &mut FirestoreClient<Channel>, name: &str) {
    client
        .delete_document(DeleteDocumentRequest {
            name: name.to_string(),
            ..Default::default()
        })
        .await
        .expect("DeleteDocument succeeds");
}

#[tokio::test]
async fn a_document_watch_reports_create_update_and_delete() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(7, &[&name])).await;

    // The document does not exist yet, so the opening snapshot carries no
    // document change — only ADD and CURRENT. That is how a client learns
    // "watched, and currently absent".
    let opening = listener.snapshot().await;
    assert_eq!(
        target_changes(&opening),
        vec![
            (TargetChangeType::Add, vec![7]),
            (TargetChangeType::Current, vec![7]),
        ]
    );
    assert!(changed(&opening).is_empty(), "nothing is there yet");

    write(&mut client, &name, fields([("city", string_value("SF"))])).await;
    let created = listener.snapshot().await;
    let documents = changed(&created);
    assert_eq!(documents.len(), 1, "one change, got {created:?}");
    assert_eq!(documents[0].name, name);
    assert_eq!(documents[0].fields["city"], string_value("SF"));
    assert!(
        documents[0].update_time.is_some(),
        "a watcher sees the stored document, timestamps included"
    );

    write(&mut client, &name, fields([("city", string_value("LA"))])).await;
    let updated = changed(&listener.snapshot().await);
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].fields["city"], string_value("LA"));

    delete(&mut client, &name).await;
    let removed_batch = listener.snapshot().await;
    assert_eq!(
        deleted(&removed_batch),
        vec![name.clone()],
        "a deleted document is a DocumentDelete, not a DocumentRemove"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_document_watch_opens_with_the_state_already_in_the_store() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");
    write(&mut client, &name, fields([("city", string_value("SF"))])).await;

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(1, &[&name])).await;

    let opening = listener.snapshot().await;
    let documents = changed(&opening);
    assert_eq!(
        documents.len(),
        1,
        "the opening snapshot is the current state"
    );
    assert_eq!(documents[0].fields["city"], string_value("SF"));
    assert!(
        target_changes(&opening).contains(&(TargetChangeType::Current, vec![1])),
        "CURRENT comes after the state it describes: {opening:?}"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_document_watch_ignores_writes_to_other_documents() {
    let (server, mut client) = start().await;
    let watched = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(1, &[&watched])).await;
    listener.snapshot().await;

    write(
        &mut client,
        &format!("{ROOT}/users/bob"),
        fields([("city", string_value("SF"))]),
    )
    .await;
    listener.expect_quiet().await;

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_query_watch_reports_documents_entering_and_leaving_the_result_set() {
    let (server, mut client) = start().await;
    let alice = format!("{ROOT}/users/alice");
    let bob = format!("{ROOT}/users/bob");
    write(&mut client, &bob, fields([("city", string_value("SF"))])).await;

    let mut listener = Listener::open(&mut client).await;
    listener
        .add(query_target(3, "users", equals("city", string_value("SF"))))
        .await;

    let opening = listener.snapshot().await;
    assert_eq!(
        changed(&opening)
            .iter()
            .map(|document| document.name.clone())
            .collect::<Vec<_>>(),
        vec![bob.clone()],
        "the opening snapshot is the current result set"
    );

    // Enters the result set.
    write(&mut client, &alice, fields([("city", string_value("SF"))])).await;
    assert_eq!(
        changed(&listener.snapshot().await)
            .iter()
            .map(|document| document.name.clone())
            .collect::<Vec<_>>(),
        vec![alice.clone()]
    );

    // Still in the collection, no longer in the result set. The contract calls
    // that a DocumentRemove — the document itself is very much still there.
    write(&mut client, &alice, fields([("city", string_value("LA"))])).await;
    let left = listener.snapshot().await;
    assert_eq!(removed(&left), vec![alice.clone()]);
    assert!(
        deleted(&left).is_empty(),
        "the document was not deleted, only dropped from the target: {left:?}"
    );

    // Deleted out of the result set: a DocumentDelete this time.
    delete(&mut client, &bob).await;
    let gone = listener.snapshot().await;
    assert_eq!(deleted(&gone), vec![bob.clone()]);
    assert!(removed(&gone).is_empty(), "{gone:?}");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_query_watch_ignores_a_write_to_another_collection() {
    let (server, mut client) = start().await;

    let mut listener = Listener::open(&mut client).await;
    listener.add(query_target(1, "users", None)).await;
    listener.snapshot().await;

    write(
        &mut client,
        &format!("{ROOT}/cities/sf"),
        fields([("name", string_value("San Francisco"))]),
    )
    .await;
    // A subcollection document shares the watched collection's key prefix but
    // belongs to a different collection, which is the easy one to get wrong.
    write(
        &mut client,
        &format!("{ROOT}/users/alice/posts/p1"),
        fields([("title", string_value("hello"))]),
    )
    .await;
    listener.expect_quiet().await;

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_query_watch_respects_the_query_limit() {
    let (server, mut client) = start().await;
    let mut target = query_target(1, "users", None);
    if let Some(TargetType::Query(QueryTarget {
        query_type: Some(QueryType::StructuredQuery(query)),
        ..
    })) = target.target_type.as_mut()
    {
        query.limit = Some(1);
    }

    write(
        &mut client,
        &format!("{ROOT}/users/alice"),
        fields([("city", string_value("SF"))]),
    )
    .await;

    let mut listener = Listener::open(&mut client).await;
    listener.add(target).await;
    assert_eq!(changed(&listener.snapshot().await).len(), 1);

    // `bob` sorts after `alice`, so the limit keeps it out of the result set.
    // A naive implementation that matched each change against the filter alone
    // would report it; re-evaluating the query does not.
    write(
        &mut client,
        &format!("{ROOT}/users/bob"),
        fields([("city", string_value("SF"))]),
    )
    .await;
    let batch = listener.snapshot().await;
    assert!(
        changed(&batch).is_empty() && removed(&batch).is_empty(),
        "a write that cannot affect a limited result set changes nothing: {batch:?}"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_collection_group_watch_reports_a_nested_collection() {
    let (server, mut client) = start().await;
    let nested = format!("{ROOT}/users/alice/posts/p1");

    let mut group = query_target(1, "posts", None);
    if let Some(TargetType::Query(QueryTarget {
        query_type: Some(QueryType::StructuredQuery(query)),
        ..
    })) = group.target_type.as_mut()
    {
        query.from[0].all_descendants = true;
    }

    let mut listener = Listener::open(&mut client).await;
    listener.add(group).await;
    assert!(changed(&listener.snapshot().await).is_empty());

    // A plain collection watch on `posts` under the root would never see this
    // document; a collection group watch has to.
    write(
        &mut client,
        &nested,
        fields([("title", string_value("hello"))]),
    )
    .await;
    let documents = changed(&listener.snapshot().await);
    assert_eq!(documents.len(), 1, "a nested collection is in the group");
    assert_eq!(documents[0].name, nested);

    // A document in a different collection, at the same depth, is not.
    write(
        &mut client,
        &format!("{ROOT}/users/alice/notes/n1"),
        fields([("title", string_value("nope"))]),
    )
    .await;
    listener.expect_quiet().await;

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_write_that_did_not_come_through_the_rpc_still_notifies() {
    // Proves the change feed sits under the data plane rather than inside the
    // Firestore RPCs: a write through the shared store — which is also the
    // path the Datastore surface and the seeder take — reaches a listener.
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(1, &[&name])).await;
    listener.snapshot().await;

    server
        .store
        .put(
            Document {
                name: name.clone(),
                fields: fields([("city", string_value("SF"))]),
                ..Default::default()
            },
            fireup::local::store::now_timestamp(),
        )
        .await
        .expect("a direct store write");

    let documents = changed(&listener.snapshot().await);
    assert_eq!(documents.len(), 1, "a store write notifies the listener");
    assert_eq!(documents[0].name, name);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn removing_a_target_stops_its_notifications() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(4, &[&name])).await;
    listener.snapshot().await;

    listener.remove(4).await;
    let confirmation = tokio::time::timeout(RESPONSE_TIMEOUT, listener.responses.message())
        .await
        .expect("the removal is confirmed")
        .expect("healthy stream")
        .expect("open stream");
    assert_eq!(
        target_changes(&[confirmation.response_type.expect("a response_type")]),
        vec![(TargetChangeType::Remove, vec![4])]
    );

    write(&mut client, &name, fields([("city", string_value("SF"))])).await;
    listener.expect_quiet().await;

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_once_target_is_retired_as_soon_as_it_is_current() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");
    write(&mut client, &name, fields([("city", string_value("SF"))])).await;

    let mut listener = Listener::open(&mut client).await;
    listener
        .add(Target {
            once: true,
            ..documents_target(5, &[&name])
        })
        .await;

    let opening = listener.snapshot().await;
    assert_eq!(changed(&opening).len(), 1, "the snapshot still arrives");
    assert_eq!(
        target_changes(&opening),
        vec![
            (TargetChangeType::Add, vec![5]),
            (TargetChangeType::Current, vec![5]),
            (TargetChangeType::Remove, vec![5]),
        ],
        "`once` means the target is gone the moment it is consistent"
    );

    write(&mut client, &name, fields([("city", string_value("LA"))])).await;
    listener.expect_quiet().await;

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_unserved_target_is_refused_without_taking_the_stream_down() {
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;

    // A collection group query with no collection id is the kindless scan of
    // everything under the parent, which the query engine does not serve. The
    // refusal has to arrive as a per-target REMOVE with a cause, because
    // failing the whole call would take down every other listener the client
    // multiplexed onto this stream.
    let mut kindless = query_target(1, "", None);
    if let Some(TargetType::Query(QueryTarget {
        query_type: Some(QueryType::StructuredQuery(query)),
        ..
    })) = kindless.target_type.as_mut()
    {
        query.from[0].all_descendants = true;
    }
    listener.add(kindless).await;

    let refusal = tokio::time::timeout(RESPONSE_TIMEOUT, listener.responses.message())
        .await
        .expect("a refusal arrives")
        .expect("healthy stream")
        .expect("open stream")
        .response_type
        .expect("a response_type");
    match &refusal {
        ResponseType::TargetChange(change) => {
            assert_eq!(change.target_change_type(), TargetChangeType::Remove);
            assert_eq!(change.target_ids, vec![1]);
            let cause = change.cause.as_ref().expect("the refusal says why");
            assert_eq!(cause.code, Code::Unimplemented as i32);
            assert!(
                cause.message.contains("collection group"),
                "the cause should name the missing piece: {}",
                cause.message
            );
        }
        other => panic!("expected a target REMOVE, got {other:?}"),
    }

    // The stream is still usable, which is the point.
    listener.add(documents_target(2, &[&name])).await;
    listener.snapshot().await;
    write(&mut client, &name, fields([("city", string_value("SF"))])).await;
    assert_eq!(changed(&listener.snapshot().await).len(), 1);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_read_time_resume_is_refused_rather_than_approximated() {
    let (server, mut client) = start().await;
    let mut listener = Listener::open(&mut client).await;

    listener
        .add(Target {
            resume_type: Some(ResumeType::ReadTime(prost_types::Timestamp {
                seconds: 1,
                nanos: 0,
            })),
            ..query_target(1, "users", None)
        })
        .await;

    let refusal = tokio::time::timeout(RESPONSE_TIMEOUT, listener.responses.message())
        .await
        .expect("a refusal arrives")
        .expect("healthy stream")
        .expect("open stream")
        .response_type
        .expect("a response_type");
    match &refusal {
        ResponseType::TargetChange(change) => {
            let cause = change.cause.as_ref().expect("the refusal says why");
            assert_eq!(cause.code, Code::Unimplemented as i32);
            assert!(
                cause.message.contains("read_time"),
                "the cause should name the missing piece: {}",
                cause.message
            );
        }
        other => panic!("expected a target REMOVE, got {other:?}"),
    }

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_malformed_document_name_is_refused_per_target() {
    let (server, mut client) = start().await;
    let mut listener = Listener::open(&mut client).await;

    // An odd segment count names a collection, not a document.
    listener
        .add(documents_target(1, &[&format!("{ROOT}/users")]))
        .await;

    let refusal = tokio::time::timeout(RESPONSE_TIMEOUT, listener.responses.message())
        .await
        .expect("a refusal arrives")
        .expect("healthy stream")
        .expect("open stream")
        .response_type
        .expect("a response_type");
    match &refusal {
        ResponseType::TargetChange(change) => {
            assert_eq!(change.target_change_type(), TargetChangeType::Remove);
            let cause = change.cause.as_ref().expect("the refusal says why");
            assert_eq!(cause.code, Code::InvalidArgument as i32);
        }
        other => panic!("expected a target REMOVE, got {other:?}"),
    }

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn two_targets_on_one_stream_are_reported_independently() {
    let (server, mut client) = start().await;
    let alice = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(1, &[&alice])).await;
    listener.snapshot().await;
    listener.add(query_target(2, "users", None)).await;
    listener.snapshot().await;

    write(&mut client, &alice, fields([("city", string_value("SF"))])).await;
    let batch = listener.snapshot().await;

    // One write, two targets: the document change is reported once per target,
    // each tagged with its own target id.
    let tagged: Vec<Vec<i32>> = batch
        .iter()
        .filter_map(|response| match response {
            ResponseType::DocumentChange(change) => Some(change.target_ids.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tagged, vec![vec![1], vec![2]], "{batch:?}");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_half_closed_request_stream_finishes_the_listen_stream() {
    // This asserted the opposite until PR #81: that a half-closed stream kept
    // its targets live, on the reading that half-close means "done adding
    // targets" rather than "done with this stream". That is legal bidi gRPC and
    // it is not what the official SDKs do. `Watch.shutdown()` in
    // `@google-cloud/firestore` — the body of the function `onSnapshot()`
    // returns as `unsubscribe` — is exactly:
    //
    //     this.currentStream?.end();
    //     this.currentStream = null;
    //
    // It half-closes, drops the reference, and never reads the stream again.
    // `resetStream()` and the watch idle timeout do the same before opening a
    // replacement. So a server that keeps the stream open after a half-close is
    // holding something no client will ever read:
    //
    //   * `terminate()` closes the HTTP/2 session *gracefully*, which waits for
    //     open streams — so a Node process that ever called `onSnapshot()`
    //     never exits. `scripts/smoke-local-firestore.sh` hung exactly there.
    //   * every SDK idle cycle leaves another live session behind on the server.
    //
    // Neither showed up under tonic 0.11: hyper 0.14 dropped the whole
    // connection when the client sent GOAWAY, whatever the stream was doing.
    // hyper 1.x honours graceful shutdown, so the stream has to end itself.
    let (server, mut client) = start().await;
    let name = format!("{ROOT}/users/alice");

    let mut listener = Listener::open(&mut client).await;
    listener.add(documents_target(1, &[&name])).await;
    listener.snapshot().await;

    let Listener {
        requests,
        mut responses,
    } = listener;
    drop(requests); // half-close the request side

    // The stream ends on its own, without the client reading anything more and
    // without the server being shut down. `None` rather than an error: this is
    // an ordinary completion, so the SDK's `onError` must not fire.
    let ended = tokio::time::timeout(RESPONSE_TIMEOUT, responses.message())
        .await
        .expect("a half-closed Listen stream must finish, not hang")
        .expect("ending the stream is not an error");
    assert!(
        ended.is_none(),
        "expected the stream to complete; got another response: {ended:?}"
    );

    // And a write to a document the dead stream was watching is still served
    // normally — ending one Listen session must not disturb the store.
    write(&mut client, &name, fields([("city", string_value("SF"))])).await;

    server.shutdown().await.expect("clean shutdown");
}
