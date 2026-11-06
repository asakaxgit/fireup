//! The query pushdown is guarded at the data-plane boundary, not per `Store`.
//!
//! `Store::scan_collection_filtered` and `scan_kind_filtered` are **defaulted**
//! trait methods whose default throws the pre-filter away and reads everything.
//! That default is what makes pushdown opt-in per backend, and it is also a
//! silent trap: anything sitting between the data plane and the real store — a
//! decorator for change feeds, metrics, access control — that forwards
//! `scan_collection` but forgets `scan_collection_filtered` still **compiles**
//! and still returns **correct results**, because the in-process engine runs
//! afterwards and has the only vote on the answer. The pushdown is simply gone
//! for every request the server serves.
//!
//! No existing test can see that:
//!
//! - `tests/local_query_pushdown_tests.rs` drives a bare `PostgresStore`, below
//!   wherever a decorator would sit, and asks whether SQL and a full scan agree.
//!   They agree whether or not the server ever calls the filtered method.
//! - `tests/local_firestore_tests.rs` drives the real server, but only checks
//!   the documents that come back — which the trap does not change.
//!
//! So this file asserts the thing neither can: that **the data plane reads
//! through the filtered method**, with a pre-filter that actually carries the
//! query's predicates, against whatever store stack `local::spawn_with_store`
//! assembles. One assertion at the boundary covers every present and future
//! decorator; a test per decorator is a test that drifts.
//!
//! ## Why the negative control is not optional
//!
//! A test written the obvious way — "assert `scan_collection_filtered` was
//! called" — passes against a broken stack too, because the default
//! implementation *is* `scan_collection_filtered`; it just does nothing useful.
//! Every positive assertion here is therefore paired with the same query run
//! through [`PushdownBlindStore`], a pass-through decorator that deliberately
//! omits the two forwards. Those tests assert the recorder sees the *unfiltered*
//! method, which is the mutation check: it proves the recorder can tell the two
//! stacks apart, and so proves the positive assertions above would fail if the
//! forwarding were lost. `results_are_identical_with_and_without_pushdown` then
//! shows why that mutation is invisible to every other test in the tree.
//!
//! The same recorder also guards the *Datastore* data plane, which reads the
//! same store and translates its own filter language into the same compiled
//! `Predicate` — see `datastore_run_query_reads_the_kind_through_the_filtered_scan`
//! and its ancestor-query and negative-control siblings. Two data planes over
//! one store, both asserted at the same boundary.
//!
//! Needs no database and no network beyond loopback, so it runs in plain
//! `cargo test --test local_pushdown_boundary_tests`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fireup::local;
use fireup::local::query::Prefilter;
use fireup::local::store::{MemoryStore, PendingWrite, Store, StoreResult, WriteEffect};
use fireup::protos::google::datastore::v1 as datastore;
use fireup::protos::google::datastore::v1::datastore_client::DatastoreClient;
use fireup::protos::google::firestore::v1::{
    firestore_client::FirestoreClient, run_query_request::QueryType,
    structured_query::field_filter::Operator as FieldOperator,
    structured_query::filter::FilterType, structured_query::CollectionSelector,
    structured_query::Direction, structured_query::FieldFilter, structured_query::FieldReference,
    structured_query::Filter, structured_query::Order, value::ValueType, CreateDocumentRequest,
    Document, RunQueryRequest, StructuredQuery, Value,
};
use prost_types::Timestamp;
use tonic::transport::Channel;

const ROOT: &str = "projects/fireup-test/databases/(default)/documents";

// ---------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------

/// One read the data plane asked the store for.
///
/// Carries the arguments as well as the method, because "the filtered method
/// was called" is only half the claim: a caller that passes an empty pre-filter
/// has pushed nothing down while satisfying a method-name assertion.
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
/// Placed *underneath* whatever `spawn_with_store` wraps the store in, so what
/// it records is what the whole configured stack passed down — not what the
/// outermost layer was handed.
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

// ---------------------------------------------------------------------------
// The negative control
// ---------------------------------------------------------------------------

/// A pass-through `Store` decorator with exactly one thing wrong with it: it
/// forwards `scan_collection` and `scan_kind` and **does not** forward their
/// filtered forms, so both fall back to the trait default and read everything.
///
/// This is the regression, written down. It exists so the assertions above can
/// be shown to fail against a stack that has the bug — see the module docs on
/// why a positive-only assertion proves nothing about a defaulted method. Do not
/// "fix" it by adding the forwards; the tests below assert that it is broken.
struct PushdownBlindStore {
    inner: Arc<dyn Store>,
}

impl PushdownBlindStore {
    fn new(inner: Arc<dyn Store>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl Store for PushdownBlindStore {
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
        self.inner.scan_collection(parent, collection_id).await
    }

    async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>> {
        self.inner.scan_kind(documents_root, kind).await
    }

    async fn child_segments(&self, prefix: &str) -> StoreResult<Vec<String>> {
        self.inner.child_segments(prefix).await
    }

    async fn count(&self) -> StoreResult<usize> {
        self.inner.count().await
    }

    // Deliberately no `scan_collection_filtered` / `scan_kind_filtered`.
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Start a server over the store stack `wrap` builds on top of the recorder,
/// and return the running server, a connected client, and the recorder's log.
///
/// `wrap` is where a real decorator would go. The recorder goes *below* it, so
/// the log reflects what reached the store rather than what the data plane
/// handed to the top of the stack.
async fn start_with(
    wrap: impl FnOnce(Arc<dyn Store>) -> Arc<dyn Store>,
) -> (
    local::RunningServer,
    FirestoreClient<Channel>,
    Arc<Mutex<Vec<Read>>>,
) {
    let recorder = RecordingStore::new(Arc::new(MemoryStore::new()));
    let log = recorder.log();
    let store = wrap(Arc::new(recorder));

    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"));
    let server = local::spawn_with_store(config, store)
        .await
        .expect("server binds");
    let client = FirestoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects to the bound port");
    (server, client, log)
}

/// The production stack: whatever `spawn_with_store` configures, nothing extra.
async fn start() -> (
    local::RunningServer,
    FirestoreClient<Channel>,
    Arc<Mutex<Vec<Read>>>,
) {
    start_with(|store| store).await
}

/// The same stack with the pushdown-dropping decorator spliced in.
async fn start_pushdown_blind() -> (
    local::RunningServer,
    FirestoreClient<Channel>,
    Arc<Mutex<Vec<Read>>>,
) {
    start_with(|store| Arc::new(PushdownBlindStore::new(store))).await
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

/// Write one document under `parent`/`collection_id` through the public API, so
/// the store is populated the way the server populates it.
async fn create(
    client: &mut FirestoreClient<Channel>,
    parent: &str,
    collection_id: &str,
    document_id: &str,
    city: &str,
    age: i64,
) {
    client
        .create_document(CreateDocumentRequest {
            parent: parent.to_string(),
            collection_id: collection_id.to_string(),
            document_id: document_id.to_string(),
            document: Some(Document {
                fields: fields([("city", string_value(city)), ("age", integer_value(age))]),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("create succeeds");
}

/// Seed one collection directly under the root plus a same-named collection
/// nested below a document, so a collection group query has more to find than
/// the collection query does.
async fn seed(client: &mut FirestoreClient<Channel>) {
    create(client, ROOT, "people", "alice", "Tokyo", 31).await;
    create(client, ROOT, "people", "bob", "Kyoto", 41).await;
    create(client, ROOT, "people", "carol", "Tokyo", 25).await;
    create(
        client,
        &format!("{ROOT}/offices/kyoto"),
        "people",
        "dan",
        "Tokyo",
        55,
    )
    .await;
}

/// A query whose predicates the pushdown is documented to translate: an
/// equality on a string plus an `orderBy`, which also makes the pre-filter's
/// `required_fields` non-empty.
fn pushable_query(all_descendants: bool) -> StructuredQuery {
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
    }
}

/// Run a query and return `(name, fields)` for each document, in query order.
///
/// Names and fields rather than whole documents because two servers stamp two
/// different `update_time`s on the same content, and the comparison below is
/// about the answer, not the clock.
async fn run_query(
    client: &mut FirestoreClient<Channel>,
    query: StructuredQuery,
) -> Vec<(String, HashMap<String, Value>)> {
    let mut stream = client
        .run_query(RunQueryRequest {
            parent: ROOT.to_string(),
            query_type: Some(QueryType::StructuredQuery(query)),
            ..Default::default()
        })
        .await
        .expect("RunQuery is accepted")
        .into_inner();

    let mut results = Vec::new();
    while let Some(response) = stream.message().await.expect("the stream does not fail") {
        if let Some(document) = response.document {
            results.push((document.name, document.fields));
        }
    }
    results
}

/// Reads recorded since the log was last drained. Seeding writes are not reads
/// and never appear here, but draining keeps a later change to the write path
/// from leaking into a query's assertion.
fn drain(log: &Arc<Mutex<Vec<Read>>>) -> Vec<Read> {
    std::mem::take(&mut *log.lock().expect("read log poisoned"))
}

// ---------------------------------------------------------------------------
// The guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_query_reads_a_collection_through_the_filtered_scan() {
    let (server, mut client, log) = start().await;
    seed(&mut client).await;
    drain(&log);

    let results = run_query(&mut client, pushable_query(false)).await;
    assert_eq!(
        results.len(),
        2,
        "the query matches alice and carol, so the read actually happened"
    );

    assert_eq!(
        drain(&log),
        vec![Read::CollectionFiltered {
            parent: ROOT.to_string(),
            collection_id: "people".to_string(),
            narrowing: true,
        }],
        "a collection query must reach the store as exactly one filtered scan \
         carrying the query's pre-filter — an unfiltered `scan_collection` here \
         means some layer between the data plane and the store dropped the \
         pushdown, and no other test in this tree would notice"
    );

    server.shutdown().await.expect("server stops");
}

#[tokio::test]
async fn run_query_reads_a_collection_group_through_the_filtered_kind_scan() {
    let (server, mut client, log) = start().await;
    seed(&mut client).await;
    drain(&log);

    let results = run_query(&mut client, pushable_query(true)).await;
    assert_eq!(
        results.len(),
        3,
        "the collection group query also finds dan under offices/kyoto"
    );

    assert_eq!(
        drain(&log),
        vec![Read::KindFiltered {
            documents_root: ROOT.to_string(),
            kind: "people".to_string(),
            narrowing: true,
        }],
        "a collection group query must reach the store as exactly one filtered \
         kind scan carrying the query's pre-filter; this is the widest read the \
         server does and therefore the one that most needs narrowing"
    );

    server.shutdown().await.expect("server stops");
}

// ---------------------------------------------------------------------------
// The mutation check: the guard above fails when the forwarding is lost
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_decorator_that_forgets_the_filtered_collection_scan_is_detected() {
    let (server, mut client, log) = start_pushdown_blind().await;
    seed(&mut client).await;
    drain(&log);

    run_query(&mut client, pushable_query(false)).await;

    assert_eq!(
        drain(&log),
        vec![Read::Collection {
            parent: ROOT.to_string(),
            collection_id: "people".to_string(),
        }],
        "with the forwarding removed the store sees an unfiltered scan, which is \
         what makes `run_query_reads_a_collection_through_the_filtered_scan` a \
         real assertion rather than one the trait default satisfies for free"
    );

    server.shutdown().await.expect("server stops");
}

#[tokio::test]
async fn a_decorator_that_forgets_the_filtered_kind_scan_is_detected() {
    let (server, mut client, log) = start_pushdown_blind().await;
    seed(&mut client).await;
    drain(&log);

    run_query(&mut client, pushable_query(true)).await;

    assert_eq!(
        drain(&log),
        vec![Read::Kind {
            documents_root: ROOT.to_string(),
            kind: "people".to_string(),
        }],
        "same mutation check for the collection group read"
    );

    server.shutdown().await.expect("server stops");
}

// ---------------------------------------------------------------------------
// The other data plane
// ---------------------------------------------------------------------------

/// Start a Datastore-only server over the store stack `wrap` builds on top of
/// the recorder, seeded with one `Task` entity under an `Owner` ancestor.
///
/// Same arrangement as [`start_with`], and for the same reason: the recorder
/// goes *below* whatever `spawn_with_store` and `wrap` assemble, so the log says
/// what reached the store.
async fn start_datastore_with(
    wrap: impl FnOnce(Arc<dyn Store>) -> Arc<dyn Store>,
) -> (
    local::RunningServer,
    DatastoreClient<Channel>,
    Arc<Mutex<Vec<Read>>>,
) {
    let recorder = RecordingStore::new(Arc::new(MemoryStore::new()));
    let log = recorder.log();
    let store = wrap(Arc::new(recorder));

    let config = local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
        .with_project_id("fireup-test")
        .with_only(local::Service::Datastore);
    let server = local::spawn_with_store(config, store)
        .await
        .expect("server binds");
    let mut client = DatastoreClient::connect(format!("http://{}", server.addr))
        .await
        .expect("client connects to the bound port");

    for path in [
        vec![task_element("sample")],
        vec![owner_element("ada"), task_element("owned")],
    ] {
        client
            .commit(datastore::CommitRequest {
                project_id: "fireup-test".to_string(),
                mode: datastore::commit_request::Mode::NonTransactional as i32,
                mutations: vec![datastore::Mutation {
                    operation: Some(datastore::mutation::Operation::Upsert(datastore::Entity {
                        key: Some(datastore::Key {
                            partition_id: None,
                            path,
                        }),
                        properties: [("priority".to_string(), ds_integer(3))]
                            .into_iter()
                            .collect(),
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .expect("Commit upsert");
    }
    drain(&log);
    (server, client, log)
}

fn task_element(name: &str) -> datastore::key::PathElement {
    datastore::key::PathElement {
        kind: "Task".to_string(),
        id_type: Some(datastore::key::path_element::IdType::Name(name.to_string())),
    }
}

fn owner_element(name: &str) -> datastore::key::PathElement {
    datastore::key::PathElement {
        kind: "Owner".to_string(),
        id_type: Some(datastore::key::path_element::IdType::Name(name.to_string())),
    }
}

fn ds_integer(number: i64) -> datastore::Value {
    datastore::Value {
        value_type: Some(datastore::value::ValueType::IntegerValue(number)),
        ..Default::default()
    }
}

/// A Datastore query over `Task` whose `priority = 3` predicate is one the
/// translation handles, optionally scoped to the `Owner/ada` ancestor.
fn datastore_pushable_query(ancestor: Option<datastore::Key>) -> datastore::Query {
    let property = datastore::Filter {
        filter_type: Some(datastore::filter::FilterType::PropertyFilter(
            datastore::PropertyFilter {
                property: Some(datastore::PropertyReference {
                    name: "priority".to_string(),
                }),
                op: datastore::property_filter::Operator::Equal as i32,
                value: Some(ds_integer(3)),
            },
        )),
    };
    let filter = match ancestor {
        None => property,
        Some(key) => datastore::Filter {
            filter_type: Some(datastore::filter::FilterType::CompositeFilter(
                datastore::CompositeFilter {
                    op: datastore::composite_filter::Operator::And as i32,
                    filters: vec![
                        property,
                        datastore::Filter {
                            filter_type: Some(datastore::filter::FilterType::PropertyFilter(
                                datastore::PropertyFilter {
                                    property: Some(datastore::PropertyReference {
                                        name: "__key__".to_string(),
                                    }),
                                    op: datastore::property_filter::Operator::HasAncestor as i32,
                                    value: Some(datastore::Value {
                                        value_type: Some(
                                            datastore::value::ValueType::KeyValue(key),
                                        ),
                                        ..Default::default()
                                    }),
                                },
                            )),
                        },
                    ],
                },
            )),
        },
    };

    datastore::Query {
        kind: vec![datastore::KindExpression {
            name: "Task".to_string(),
        }],
        filter: Some(filter),
        ..Default::default()
    }
}

/// Run a Datastore query and return how many entities came back.
async fn run_datastore_query(
    client: &mut DatastoreClient<Channel>,
    query: datastore::Query,
) -> usize {
    client
        .run_query(datastore::RunQueryRequest {
            project_id: "fireup-test".to_string(),
            query_type: Some(datastore::run_query_request::QueryType::Query(query)),
            ..Default::default()
        })
        .await
        .expect("RunQuery is accepted")
        .into_inner()
        .batch
        .expect("a batch")
        .entity_results
        .len()
}

/// The Datastore surface reads the same store, and now reaches it through the
/// filtered kind scan carrying the translation of its own filter language.
///
/// This test used to assert the opposite — it pinned the unfiltered `scan_kind`
/// so the gap was a checked fact rather than folklore, with a message saying to
/// flip it to the filtered form when the translation landed. This is that flip.
#[tokio::test]
async fn datastore_run_query_reads_the_kind_through_the_filtered_scan() {
    let (server, mut client, log) = start_datastore_with(|store| store).await;

    assert_eq!(
        run_datastore_query(&mut client, datastore_pushable_query(None)).await,
        2,
        "the query matches both Tasks, so the read actually happened"
    );

    assert_eq!(
        drain(&log),
        vec![Read::KindFiltered {
            documents_root: ROOT.to_string(),
            kind: "Task".to_string(),
            narrowing: true,
        }],
        "a Datastore kind query must reach the store as exactly one filtered \
         kind scan carrying the translated predicate; an unfiltered `scan_kind` \
         here means the whole kind is on the wire for every Datastore query, \
         even against `--postgres-url`"
    );

    server.shutdown().await.expect("server stops");
}

/// An ancestor query narrows the *collection* read the same way.
///
/// Worth its own assertion because it takes the other branch of the match on
/// `(kind, ancestor)`: `HAS_ANCESTOR` chooses which collection to read and is
/// stripped out of the predicate, so this is the one shape where the term that
/// selected the read and the term that was pushed down are different terms.
#[tokio::test]
async fn datastore_ancestor_query_reads_a_collection_through_the_filtered_scan() {
    let (server, mut client, log) = start_datastore_with(|store| store).await;
    let ancestor = datastore::Key {
        partition_id: None,
        path: vec![owner_element("ada")],
    };

    assert_eq!(
        run_datastore_query(&mut client, datastore_pushable_query(Some(ancestor))).await,
        1,
        "only the Task under Owner/ada is in scope"
    );

    assert_eq!(
        drain(&log),
        vec![Read::CollectionFiltered {
            parent: format!("{ROOT}/Owner/ada"),
            collection_id: "Task".to_string(),
            narrowing: true,
        }],
        "the ancestor chose the collection and the property filter still \
         narrowed it — a `narrowing: false` here would mean the predicate was \
         thrown away along with the stripped `HAS_ANCESTOR` term"
    );

    server.shutdown().await.expect("server stops");
}

/// The same mutation check the Firestore assertions get, for the same reason:
/// `scan_kind_filtered` is a defaulted method, so "it was called" is satisfied
/// for free and only the negative control makes the assertion above real.
#[tokio::test]
async fn a_decorator_that_forgets_the_filtered_scan_is_detected_on_the_datastore_surface() {
    let (server, mut client, log) =
        start_datastore_with(|store| Arc::new(PushdownBlindStore::new(store))).await;

    let matched = run_datastore_query(&mut client, datastore_pushable_query(None)).await;

    assert_eq!(
        drain(&log),
        vec![Read::Kind {
            documents_root: ROOT.to_string(),
            kind: "Task".to_string(),
        }],
        "with the forwarding removed the store sees an unfiltered scan, which \
         is what makes the assertion above a real one rather than one the trait \
         default satisfies"
    );
    assert_eq!(
        matched, 2,
        "and the answer is unchanged: the pre-filter is an optimisation, so \
         losing it costs reads and nothing a result comparison can see"
    );

    server.shutdown().await.expect("server stops");
}

/// Why the mutation above is invisible to every other test: it does not change
/// a single answer.
///
/// This is the whole reason the ticket exists. The pushdown is an optimisation
/// under a one-sided contract — a backend may return extra rows and the
/// in-process engine filters them — so losing it costs performance and nothing
/// else that a result comparison can see. Any test that checks query results,
/// including the differential pushdown suite, passes either way.
#[tokio::test]
async fn results_are_identical_with_and_without_pushdown() {
    let (intact, mut intact_client, intact_log) = start().await;
    seed(&mut intact_client).await;
    let (blind, mut blind_client, blind_log) = start_pushdown_blind().await;
    seed(&mut blind_client).await;
    drain(&intact_log);
    drain(&blind_log);

    for all_descendants in [false, true] {
        let from_intact = run_query(&mut intact_client, pushable_query(all_descendants)).await;
        let from_blind = run_query(&mut blind_client, pushable_query(all_descendants)).await;
        assert_eq!(
            from_intact, from_blind,
            "the two stacks answer identically (all_descendants={all_descendants}), \
             so correctness tests cannot tell them apart"
        );
        assert!(
            !from_intact.is_empty(),
            "the comparison is only meaningful over a non-empty result"
        );
    }

    // And the reads really did differ, so the equality above is about the
    // answers rather than about two stacks that behaved the same way.
    let intact_reads = drain(&intact_log);
    let blind_reads = drain(&blind_log);
    assert_ne!(
        intact_reads, blind_reads,
        "the two stacks read the store differently even though they agree on \
         every answer: {intact_reads:?} vs {blind_reads:?}"
    );

    intact.shutdown().await.expect("server stops");
    blind.shutdown().await.expect("server stops");
}
