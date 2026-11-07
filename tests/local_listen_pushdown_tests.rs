//! The `Listen` read path pushes down too — and it is the path that most needs
//! to.
//!
//! [`tests/local_pushdown_boundary_tests.rs`] asserts that `RunQuery` reaches
//! the store through the filtered scans, once per request. This file asserts
//! the same thing for a `Listen` query target, where the read happens on a
//! completely different schedule: a query target is re-evaluated **per change
//! event, per active target**. An unfiltered read there is not one wide scan per
//! request, it is one wide scan per write per listener — a full collection scan,
//! or a full *kind* scan for a collection group listener, every time anything in
//! the collection changes.
//!
//! That is why the watch path needs its own guard rather than inheriting
//! `RunQuery`'s. The two paths pick their scan independently:
//! `FirestoreApi::run_structured_query` picks one pair of methods and
//! `watch::evaluate` picks another, so the boundary test for one says nothing
//! about the other. The watch path originally called `scan_collection` /
//! `scan_kind` with a comment claiming they were "the same two scans `RunQuery`
//! picks between"; that was true when it was written and stopped being true
//! when the pushdown landed, with nothing to notice.
//!
//! ## What makes these real assertions
//!
//! Both halves of the claim are checked, because either alone is satisfiable by
//! a broken implementation:
//!
//! - **The filtered method.** `Store::scan_collection_filtered` and
//!   `scan_kind_filtered` are *defaulted* trait methods, so a stack that has
//!   lost the pushdown still "calls the filtered method" — the default just
//!   reads everything. `tests/local_pushdown_boundary_tests.rs` carries the
//!   negative control for that (a decorator that omits the forwards), and the
//!   forward it guards is the same one this path uses, since both read through
//!   `WatchedStore`.
//! - **A pre-filter that carries something.** A caller can satisfy a
//!   method-name assertion while passing an empty pre-filter, which pushes
//!   nothing down. Every read below is therefore recorded with whether its
//!   pre-filter was *narrowing*, and
//!   [`an_unfiltered_query_target_reads_with_an_empty_prefilter`] pins the
//!   non-narrowing case so `narrowing: true` above is a distinction the
//!   recorder can actually draw rather than a constant.
//!
//! The remaining mutation — reverting one arm of `watch::evaluate` to the
//! unfiltered scan — is what these tests exist to fail on, and was run by hand
//! against both arms before they were committed.
//!
//! Needs no database and no network beyond loopback, so it runs in plain
//! `cargo test --test local_listen_pushdown_tests`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fireup::local;
use fireup::local::query::Prefilter;
use fireup::local::store::{MemoryStore, PendingWrite, Store, StoreResult, WriteEffect};
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient,
    listen_request::TargetChange as RequestedChange,
    listen_response::ResponseType,
    structured_query::field_filter::Operator as FieldOperator,
    structured_query::filter::FilterType,
    structured_query::{CollectionSelector, Direction, FieldFilter, FieldReference, Filter, Order},
    target::{query_target::QueryType, QueryTarget, TargetType},
    target_change::TargetChangeType,
    value::ValueType,
    write::Operation,
    CommitRequest, Document, ListenRequest, ListenResponse, StructuredQuery, Target, Value, Write,
};
use prost_types::Timestamp;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Streaming;

const DATABASE: &str = "projects/fireup-test/databases/(default)";
const ROOT: &str = "projects/fireup-test/databases/(default)/documents";

/// Generous enough that a loaded CI box does not flake, short enough that a
/// listener which never fires fails the test instead of hanging it.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------

/// One read the server asked the store for.
///
/// Only the four scans are recorded. `get` is deliberately not: the write path
/// reads a document back after every commit to publish it, and recording that
/// would bury the scan this file is about.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Read {
    Collection {
        parent: String,
        collection_id: String,
    },
    CollectionFiltered {
        parent: String,
        collection_id: String,
        /// Whether the pre-filter carried anything a backend could narrow on.
        narrowing: bool,
    },
    Kind {
        documents_root: String,
        kind: String,
    },
    KindFiltered {
        documents_root: String,
        kind: String,
        narrowing: bool,
    },
}

/// Whether a backend receiving this pre-filter has been given anything to work
/// with. Both halves count: the `where` clause, and the sort fields a matching
/// document must carry.
fn narrowing(prefilter: Prefilter<'_>) -> bool {
    prefilter.filter.is_some() || !prefilter.required_fields.is_empty()
}

/// A `Store` decorator that records which read methods were called on it, then
/// forwards faithfully.
///
/// Handed to `local::spawn_with_store`, which wraps it in the change feed's own
/// decorator — so this sits *below* `WatchedStore` and records what the whole
/// configured stack passed down, not what the data plane handed to the top of
/// it.
struct RecordingStore {
    inner: Arc<dyn Store>,
    reads: Arc<Mutex<Vec<Read>>>,
}

impl RecordingStore {
    fn new(inner: Arc<dyn Store>) -> Self {
        Self {
            inner,
            reads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A handle to the log that outlives handing the store to the server.
    fn log(&self) -> Arc<Mutex<Vec<Read>>> {
        self.reads.clone()
    }

    fn record(&self, read: Read) {
        self.reads.lock().expect("read log poisoned").push(read);
    }
}

#[async_trait::async_trait]
impl Store for RecordingStore {
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }

    async fn get(&self, name: &str) -> StoreResult<Option<Document>> {
        self.inner.get(name).await
    }

    async fn contains(&self, name: &str) -> StoreResult<bool> {
        self.inner.contains(name).await
    }

    async fn commit(
        &self,
        writes: Vec<PendingWrite>,
        now: Timestamp,
    ) -> StoreResult<Vec<WriteEffect>> {
        self.inner.commit(writes, now).await
    }

    async fn scan_collection(
        &self,
        parent: &str,
        collection_id: &str,
    ) -> StoreResult<Vec<Document>> {
        self.record(Read::Collection {
            parent: parent.to_string(),
            collection_id: collection_id.to_string(),
        });
        self.inner.scan_collection(parent, collection_id).await
    }

    async fn scan_collection_filtered(
        &self,
        parent: &str,
        collection_id: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        self.record(Read::CollectionFiltered {
            parent: parent.to_string(),
            collection_id: collection_id.to_string(),
            narrowing: narrowing(prefilter),
        });
        self.inner
            .scan_collection_filtered(parent, collection_id, prefilter)
            .await
    }

    async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>> {
        self.record(Read::Kind {
            documents_root: documents_root.to_string(),
            kind: kind.to_string(),
        });
        self.inner.scan_kind(documents_root, kind).await
    }

    async fn scan_kind_filtered(
        &self,
        documents_root: &str,
        kind: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        self.record(Read::KindFiltered {
            documents_root: documents_root.to_string(),
            kind: kind.to_string(),
            narrowing: narrowing(prefilter),
        });
        self.inner
            .scan_kind_filtered(documents_root, kind, prefilter)
            .await
    }

    async fn child_segments(&self, prefix: &str) -> StoreResult<Vec<String>> {
        self.inner.child_segments(prefix).await
    }

    async fn count(&self) -> StoreResult<usize> {
        self.inner.count().await
    }
}

/// Reads recorded since the log was last drained.
fn drain(log: &Arc<Mutex<Vec<Read>>>) -> Vec<Read> {
    std::mem::take(&mut *log.lock().expect("read log poisoned"))
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Start a server over the recorder and return it, a connected client, and the
/// recorder's log.
async fn start() -> (
    local::RunningServer,
    FirestoreClient<Channel>,
    Arc<Mutex<Vec<Read>>>,
) {
    let recorder = RecordingStore::new(Arc::new(MemoryStore::new()));
    let log = recorder.log();
    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"));
    let server = local::spawn_with_store(config, Arc::new(recorder))
        .await
        .expect("server binds");
    let client = FirestoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects to the bound port");
    (server, client, log)
}

/// A live `Listen` stream, consumed the way a client SDK consumes one: up to the
/// whole-stream `NO_CHANGE` marker that makes an SDK deliver a snapshot.
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

    async fn add(&self, target: Target) {
        self.requests
            .send(ListenRequest {
                database: DATABASE.to_string(),
                target_change: Some(RequestedChange::AddTarget(target)),
                ..Default::default()
            })
            .await
            .expect("the server is still reading requests");
    }

    /// Read responses up to the next whole-stream snapshot marker, returning
    /// the document names the batch reported as changed.
    async fn snapshot(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        loop {
            let response = tokio::time::timeout(RESPONSE_TIMEOUT, self.responses.message())
                .await
                .unwrap_or_else(|_| panic!("no snapshot marker within {RESPONSE_TIMEOUT:?}"))
                .expect("the stream stays healthy")
                .expect("the server does not end the stream");
            match response
                .response_type
                .expect("every ListenResponse carries a response_type")
            {
                ResponseType::TargetChange(change)
                    if change.target_change_type() == TargetChangeType::NoChange
                        && change.target_ids.is_empty()
                        && change.read_time.is_some() =>
                {
                    return changed
                }
                ResponseType::DocumentChange(change) => {
                    changed.push(change.document.expect("a changed document").name)
                }
                _ => {}
            }
        }
    }
}

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

fn fields<const N: usize>(pairs: [(&str, Value); N]) -> HashMap<String, Value> {
    pairs
        .into_iter()
        .map(|(field, value)| (field.to_string(), value))
        .collect()
}

/// Write one document with a `set()`-shaped commit, which is what publishes a
/// change event to the listener.
async fn write(client: &mut FirestoreClient<Channel>, name: &str, city: &str, age: i64) {
    client
        .commit(CommitRequest {
            database: DATABASE.to_string(),
            writes: vec![Write {
                operation: Some(Operation::Update(Document {
                    name: name.to_string(),
                    fields: fields([("city", string_value(city)), ("age", integer_value(age))]),
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .expect("Commit succeeds");
}

/// A query target whose predicates the pushdown is documented to translate: an
/// equality on a string plus an `orderBy`, which also makes the pre-filter's
/// `required_fields` non-empty.
fn pushable_target(target_id: i32, all_descendants: bool) -> Target {
    query_target(
        target_id,
        StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "people".to_string(),
                all_descendants,
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
        },
    )
}

fn query_target(target_id: i32, query: StructuredQuery) -> Target {
    Target {
        target_id,
        target_type: Some(TargetType::Query(QueryTarget {
            parent: ROOT.to_string(),
            query_type: Some(QueryType::StructuredQuery(query)),
        })),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_change_re_evaluates_a_collection_target_through_the_filtered_scan() {
    let (server, mut client, log) = start().await;
    let mut listener = Listener::open(&mut client).await;

    // Adding the target evaluates it once, for the initial snapshot.
    listener.add(pushable_target(1, false)).await;
    assert!(
        listener.snapshot().await.is_empty(),
        "nothing is written yet, so the first snapshot is empty"
    );
    assert_eq!(
        drain(&log),
        vec![Read::CollectionFiltered {
            parent: ROOT.to_string(),
            collection_id: "people".to_string(),
            narrowing: true,
        }],
        "adding a query target must read through the filtered scan too"
    );

    // The read this test is really about: one write, one re-evaluation.
    write(&mut client, &format!("{ROOT}/people/alice"), "Tokyo", 31).await;
    assert_eq!(
        listener.snapshot().await,
        vec![format!("{ROOT}/people/alice")],
        "the listener fires for the matching write, so the re-evaluation \
         actually happened"
    );

    assert_eq!(
        drain(&log),
        vec![Read::CollectionFiltered {
            parent: ROOT.to_string(),
            collection_id: "people".to_string(),
            narrowing: true,
        }],
        "a change must re-evaluate the target through the filtered scan \
         carrying the target query's pre-filter. An unfiltered `scan_collection` \
         here is a whole-collection read on every write, for every listener \
         watching the collection — the cost no other test in this tree can see, \
         because the in-process engine answers identically either way"
    );

    server.shutdown().await.expect("server stops");
}

#[tokio::test]
async fn a_change_re_evaluates_a_collection_group_target_through_the_filtered_kind_scan() {
    let (server, mut client, log) = start().await;
    let mut listener = Listener::open(&mut client).await;

    listener.add(pushable_target(1, true)).await;
    assert!(
        listener.snapshot().await.is_empty(),
        "nothing is written yet"
    );
    drain(&log);

    // Nested below a document, so only the collection group form finds it.
    let nested = format!("{ROOT}/offices/kyoto/people/dan");
    write(&mut client, &nested, "Tokyo", 55).await;
    assert_eq!(
        listener.snapshot().await,
        vec![nested],
        "a collection group listener sees a write to a nested collection with \
         the same id"
    );

    assert_eq!(
        drain(&log),
        vec![Read::KindFiltered {
            documents_root: ROOT.to_string(),
            kind: "people".to_string(),
            narrowing: true,
        }],
        "the collection group arm is the one that matters most: unfiltered, \
         every write re-reads every `people` collection anywhere under the root"
    );

    server.shutdown().await.expect("server stops");
}

// ---------------------------------------------------------------------------
// The control: `narrowing` is a distinction, not a constant
// ---------------------------------------------------------------------------

/// A query with no `where` and no `orderBy` has nothing to push down, and the
/// recorder says so.
///
/// Without this, `narrowing: true` above would be unfalsifiable from inside
/// this file — a `narrowing` that were always `true` would satisfy both
/// assertions while proving nothing about what the pre-filter carried.
#[tokio::test]
async fn an_unfiltered_query_target_reads_with_an_empty_prefilter() {
    let (server, mut client, log) = start().await;
    let mut listener = Listener::open(&mut client).await;

    listener
        .add(query_target(
            1,
            StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: "people".to_string(),
                    all_descendants: false,
                }],
                ..Default::default()
            },
        ))
        .await;
    assert!(
        listener.snapshot().await.is_empty(),
        "nothing is written yet"
    );
    drain(&log);

    write(&mut client, &format!("{ROOT}/people/alice"), "Tokyo", 31).await;
    assert_eq!(
        listener.snapshot().await,
        vec![format!("{ROOT}/people/alice")],
        "an unfiltered target matches every write to the collection"
    );

    assert_eq!(
        drain(&log),
        vec![Read::CollectionFiltered {
            parent: ROOT.to_string(),
            collection_id: "people".to_string(),
            narrowing: false,
        }],
        "an unfiltered query still goes through the filtered method — there is \
         simply nothing in the pre-filter for a backend to narrow on, which is \
         what makes `narrowing: true` in the tests above a real claim"
    );

    server.shutdown().await.expect("server stops");
}
