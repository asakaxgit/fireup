//! Differential test for the Datastore query pushdown.
//!
//! Companion to `local_query_pushdown_tests.rs`, which does this for the
//! Firestore surface. The property under test is the same one-sided contract —
//! a pre-filter may hand back extra rows, never drop a matching one — and it is
//! checked the only way that proves anything: every query shape is run **both**
//! ways against the same real database and the answers are compared.
//!
//! The two ways are two running servers over the same PostgreSQL table:
//!
//! 1. The ordinary stack, where `DatastoreApi::run_structured_query` translates
//!    the query's `Filter` into a `Predicate` and reads through
//!    `scan_kind_filtered` / `scan_collection_filtered`.
//! 2. The same stack with [`PushdownBlindStore`] spliced in — a pass-through
//!    decorator that forwards the unfiltered scans and not their filtered
//!    forms, so both fall back to the trait default and read everything. That
//!    is the full-scan path, and therefore the definition of a correct answer.
//!
//! Comparing a translation against itself proves nothing, which is why this
//! file also counts rows. [`CountingStore`] records how many documents each
//! scan actually returned, and every case asserts on that count as well as on
//! the answer:
//!
//! - A translatable filter must read **fewer** rows than the kind holds. Without
//!   this the agreement check would pass against a translation that returned
//!   `None` for everything.
//! - An **untranslatable** filter must read **all** of them. That is the other
//!   half of soundness and the direction that actually loses data: a filter this
//!   module does not understand must widen to "keep the row", so a narrowed
//!   read here would mean entities were dropped before anything evaluated them.
//!
//! ## Confirming the corpus can fail
//!
//! A green old-vs-new comparison is worth nothing until the new path has been
//! mutated and watched go red. These mutations of `datastore_filter` were
//! applied and confirmed to fail:
//!
//! | Mutation | What fails |
//! | --- | --- |
//! | `<` translated as `>` | the answer comparison, on `"priority < 3"`: the pushed path drops `a-low` |
//! | equality translated as `None` | the `Narrows` count on `"priority = 3"`, and `a_selective_datastore_query_reads_fewer_rows_than_the_kind_holds` (11 rows read instead of 1) |
//! | an `OR` keeps only its translatable branches | the answer comparison, on `"priority = 3 OR owner != ada"`: the pushed path drops every entity only the `!=` branch matched |
//! | `IN` translated as `NOT_IN` | the `Narrows` count on `"owner IN [ada]"`: the PostgreSQL pushdown declines `NOT_IN`, so all 11 rows were read |
//!
//! Worth repeating whenever a translation is added. Note that the first of
//! those three only fails because the Datastore data plane still answers with
//! *its own* evaluator rather than with the translated predicate — an earlier
//! version of this work evaluated the predicate in process, and that mutation
//! passed, because both paths were then the same filter agreeing on a wrong
//! answer. See the "What is deliberately not shared" section of
//! `src/local/datastore_filter.rs`.
//!
//! Needs a database and is skipped when `FIREUP_TEST_POSTGRES_URL` is unset:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL='postgresql://fireup@127.0.0.1:55432/fireup_test' \
//!   cargo test --test local_datastore_pushdown_tests
//! ```

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use fireup::local;
use fireup::local::postgres_store::{PostgresStore, DOCUMENTS_TABLE};
use fireup::local::query::Prefilter;
use fireup::local::store::{PendingWrite, Store, StoreResult, WriteEffect};
use fireup::protos::google::datastore::v1 as ds;
use fireup::protos::google::datastore::v1::datastore_client::DatastoreClient;
use fireup::protos::google::datastore::v1::value::ValueType as DsValueType;
use fireup::protos::google::firestore::v1::Document;
use prost_types::Timestamp;
use tonic::transport::Channel;

#[path = "postgres_test_env.rs"]
#[allow(dead_code)]
mod postgres_test_env;

fn test_url() -> Option<String> {
    let url = postgres_test_env::resolve_integration_postgres_url();
    if url.is_none() {
        postgres_test_env::skip_postgres_integration_test("local datastore pushdown tests");
    }
    url
}

/// A project id unique to this run, so concurrent runs against one database do
/// not see each other's entities.
fn unique_project() -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("ds-pushdown-{}", &suffix[..8])
}

// ---------------------------------------------------------------------------
// The row counter
// ---------------------------------------------------------------------------

/// How many documents one scan returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Returned {
    /// Whether the data plane used the filtered form of the scan.
    filtered: bool,
    rows: usize,
}

/// A `Store` decorator that records the size of every scan's result.
///
/// Row counts rather than method names: the method name only says the data
/// plane asked for narrowing, and this file's job is to check it got some.
struct CountingStore {
    inner: Arc<dyn Store>,
    scans: Arc<Mutex<Vec<Returned>>>,
}

impl CountingStore {
    fn new(inner: Arc<dyn Store>) -> Self {
        Self {
            inner,
            scans: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self) -> Arc<Mutex<Vec<Returned>>> {
        self.scans.clone()
    }

    fn record(&self, filtered: bool, rows: usize) {
        self.scans
            .lock()
            .expect("scan log poisoned")
            .push(Returned { filtered, rows });
    }
}

#[async_trait::async_trait]
impl Store for CountingStore {
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
        let documents = self.inner.scan_collection(parent, collection_id).await?;
        self.record(false, documents.len());
        Ok(documents)
    }

    async fn scan_collection_filtered(
        &self,
        parent: &str,
        collection_id: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        let documents = self
            .inner
            .scan_collection_filtered(parent, collection_id, prefilter)
            .await?;
        self.record(true, documents.len());
        Ok(documents)
    }

    async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>> {
        let documents = self.inner.scan_kind(documents_root, kind).await?;
        self.record(false, documents.len());
        Ok(documents)
    }

    async fn scan_kind_filtered(
        &self,
        documents_root: &str,
        kind: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        let documents = self
            .inner
            .scan_kind_filtered(documents_root, kind, prefilter)
            .await?;
        self.record(true, documents.len());
        Ok(documents)
    }

    async fn child_segments(&self, prefix: &str) -> StoreResult<Vec<String>> {
        self.inner.child_segments(prefix).await
    }

    async fn count(&self) -> StoreResult<usize> {
        self.inner.count().await
    }
}

// ---------------------------------------------------------------------------
// The full-scan path
// ---------------------------------------------------------------------------

/// A pass-through decorator that forwards the unfiltered scans and **not** their
/// filtered forms, so both fall back to the trait default and read everything.
///
/// Same device as the one in `local_pushdown_boundary_tests.rs`: it is how this
/// file gets a real server that takes the full-scan path, without a second code
/// path in the data plane that only tests use. Do not add the missing forwards.
struct PushdownBlindStore {
    inner: Arc<dyn Store>,
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
// Values and the corpus
// ---------------------------------------------------------------------------

fn ds_value(value_type: DsValueType) -> ds::Value {
    ds::Value {
        value_type: Some(value_type),
        ..Default::default()
    }
}

fn integer(number: i64) -> ds::Value {
    ds_value(DsValueType::IntegerValue(number))
}

fn double(number: f64) -> ds::Value {
    ds_value(DsValueType::DoubleValue(number))
}

fn text(contents: &str) -> ds::Value {
    ds_value(DsValueType::StringValue(contents.to_string()))
}

fn boolean(flag: bool) -> ds::Value {
    ds_value(DsValueType::BooleanValue(flag))
}

fn timestamp(seconds: i64, nanos: i32) -> ds::Value {
    ds_value(DsValueType::TimestampValue(Timestamp { seconds, nanos }))
}

fn named(kind: &str, name: &str) -> ds::key::PathElement {
    ds::key::PathElement {
        kind: kind.to_string(),
        id_type: Some(ds::key::path_element::IdType::Name(name.to_string())),
    }
}

fn key(path: Vec<ds::key::PathElement>) -> ds::Key {
    ds::Key {
        partition_id: None,
        path,
    }
}

fn key_value(path: Vec<ds::key::PathElement>) -> ds::Value {
    ds_value(DsValueType::KeyValue(key(path)))
}

/// The entities every case runs against.
///
/// Chosen for the comparisons that can go wrong rather than for realism. In
/// particular the corpus deliberately mixes *types* within one property, which
/// is where the Datastore comparator used to disagree with the Firestore one
/// the pushdown translates into: `priority` is an integer on most entities, a
/// double on one, a string on one, and a list on one.
fn corpus() -> Vec<(Vec<ds::key::PathElement>, Vec<(&'static str, ds::Value)>)> {
    vec![
        (
            vec![named("Task", "a-low")],
            vec![
                ("priority", integer(1)),
                ("owner", text("ada")),
                ("due", timestamp(1_600_000_000, 0)),
                ("done", boolean(false)),
            ],
        ),
        (
            vec![named("Task", "b-mid")],
            vec![
                ("priority", integer(3)),
                ("owner", text("ada")),
                ("due", timestamp(1_700_000_000, 123_456_789)),
                ("done", boolean(true)),
            ],
        ),
        (
            vec![named("Task", "c-high")],
            vec![
                ("priority", integer(9)),
                ("owner", text("bob")),
                ("due", timestamp(1_800_000_000, 1)),
            ],
        ),
        (
            // A double where the others hold an integer, numerically equal to
            // `b-mid`'s 3: one type to both comparators, two JSON tags in the
            // store, so the SQL has two branches to get right.
            vec![named("Task", "d-double")],
            vec![("priority", double(3.0)), ("owner", text("bob"))],
        ),
        (
            // A string where the others hold a number. The old Datastore
            // comparator called this "equal" to every integer; neither
            // comparator should match it now.
            vec![named("Task", "e-string")],
            vec![("priority", text("urgent")), ("owner", text("carol"))],
        ),
        (
            // A list, which is a type neither comparator orders against a
            // number — and the other value the old fall-through arm matched.
            vec![named("Task", "f-list")],
            vec![(
                "priority",
                ds_value(DsValueType::ArrayValue(ds::ArrayValue {
                    values: vec![integer(1), integer(3)],
                })),
            )],
        ),
        (
            // Past 2^53, where `f64` and `i64` stop agreeing.
            vec![named("Task", "g-huge")],
            vec![
                ("priority", integer(9_007_199_254_740_993)),
                ("owner", text("carol")),
            ],
        ),
        (
            // A key property, stored as a reference string: the operand has to
            // cross the same conversion or it could never match.
            vec![named("Task", "h-keyed")],
            vec![
                ("priority", integer(3)),
                ("assignee", key_value(vec![named("User", "ada")])),
            ],
        ),
        (
            // No `priority` at all: every filter on it has to leave this out,
            // on both paths.
            vec![named("Task", "i-bare")],
            vec![("owner", text("dan"))],
        ),
        (
            // Under an ancestor, so the ancestor cases have something to find
            // and the kind cases have one more entity to exclude or include.
            vec![named("Owner", "ada"), named("Task", "j-owned")],
            vec![("priority", integer(3)), ("owner", text("ada"))],
        ),
        (
            vec![named("Owner", "ada"), named("Task", "k-owned-high")],
            vec![("priority", integer(9)), ("owner", text("ada"))],
        ),
    ]
}

/// How many entities of kind `Task` exist anywhere, which a kind query spans.
fn kind_size() -> usize {
    corpus().len()
}

// ---------------------------------------------------------------------------
// The query shapes
// ---------------------------------------------------------------------------

fn property_filter(name: &str, op: ds::property_filter::Operator, value: ds::Value) -> ds::Filter {
    ds::Filter {
        filter_type: Some(ds::filter::FilterType::PropertyFilter(ds::PropertyFilter {
            property: Some(ds::PropertyReference {
                name: name.to_string(),
            }),
            op: op as i32,
            value: Some(value),
        })),
    }
}

fn conjunction(filters: Vec<ds::Filter>) -> ds::Filter {
    ds::Filter {
        filter_type: Some(ds::filter::FilterType::CompositeFilter(
            ds::CompositeFilter {
                op: ds::composite_filter::Operator::And as i32,
                filters,
            },
        )),
    }
}

fn disjunction(filters: Vec<ds::Filter>) -> ds::Filter {
    ds::Filter {
        filter_type: Some(ds::filter::FilterType::CompositeFilter(
            ds::CompositeFilter {
                op: ds::composite_filter::Operator::Or as i32,
                filters,
            },
        )),
    }
}

fn array(values: Vec<ds::Value>) -> ds::Value {
    ds_value(DsValueType::ArrayValue(ds::ArrayValue { values }))
}

fn has_ancestor(path: Vec<ds::key::PathElement>) -> ds::Filter {
    property_filter(
        "__key__",
        ds::property_filter::Operator::HasAncestor,
        key_value(path),
    )
}

fn task_query(filter: Option<ds::Filter>) -> ds::Query {
    ds::Query {
        kind: vec![ds::KindExpression {
            name: "Task".to_string(),
        }],
        filter,
        ..Default::default()
    }
}

/// Whether a case's filter is one the translation is expected to narrow on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The pushed read must return fewer rows than the full read did.
    Narrows,
    /// The pushed read must return every row, because nothing translated. This
    /// is the fallback-to-"keep the row" half of the contract.
    ReadsEverything,
}

/// Every query shape, with what the pushdown is expected to do to its read.
fn cases() -> Vec<(&'static str, ds::Query, Expect)> {
    use ds::property_filter::Operator as Op;
    use Expect::{Narrows, ReadsEverything};

    vec![
        (
            "no filter",
            task_query(None),
            // Nothing to narrow on; the read is the whole kind either way.
            ReadsEverything,
        ),
        (
            "priority = 3",
            task_query(Some(property_filter("priority", Op::Equal, integer(3)))),
            Narrows,
        ),
        (
            // The double-vs-integer pair, from the other side.
            "priority = 3.0",
            task_query(Some(property_filter("priority", Op::Equal, double(3.0)))),
            Narrows,
        ),
        (
            "priority = a string",
            task_query(Some(property_filter("priority", Op::Equal, text("urgent")))),
            Narrows,
        ),
        (
            "priority < 3",
            task_query(Some(property_filter("priority", Op::LessThan, integer(3)))),
            Narrows,
        ),
        (
            "priority <= 3",
            task_query(Some(property_filter(
                "priority",
                Op::LessThanOrEqual,
                integer(3),
            ))),
            Narrows,
        ),
        (
            "priority > 3",
            task_query(Some(property_filter(
                "priority",
                Op::GreaterThan,
                integer(3),
            ))),
            Narrows,
        ),
        (
            "priority >= 3",
            task_query(Some(property_filter(
                "priority",
                Op::GreaterThanOrEqual,
                integer(3),
            ))),
            Narrows,
        ),
        (
            // Past 2^53: the integer branch of the SQL has to compare as an
            // exact numeric rather than rounding through a double.
            "priority >= 2^53+1",
            task_query(Some(property_filter(
                "priority",
                Op::GreaterThanOrEqual,
                integer(9_007_199_254_740_993),
            ))),
            Narrows,
        ),
        (
            "owner = ada",
            task_query(Some(property_filter("owner", Op::Equal, text("ada")))),
            Narrows,
        ),
        (
            "done = false",
            task_query(Some(property_filter("done", Op::Equal, boolean(false)))),
            Narrows,
        ),
        (
            "due >= a timestamp",
            task_query(Some(property_filter(
                "due",
                Op::GreaterThanOrEqual,
                timestamp(1_700_000_000, 123_456_789),
            ))),
            Narrows,
        ),
        (
            "assignee = a key",
            task_query(Some(property_filter(
                "assignee",
                Op::Equal,
                key_value(vec![named("User", "ada")]),
            ))),
            Narrows,
        ),
        (
            "priority >= 3 AND owner = ada",
            task_query(Some(conjunction(vec![
                property_filter("priority", Op::GreaterThanOrEqual, integer(3)),
                property_filter("owner", Op::Equal, text("ada")),
            ]))),
            Narrows,
        ),
        (
            // A range on both sides of one property, which is the shape an
            // index serves and the one worth reading least for.
            "1 < priority < 9",
            task_query(Some(conjunction(vec![
                property_filter("priority", Op::GreaterThan, integer(1)),
                property_filter("priority", Op::LessThan, integer(9)),
            ]))),
            Narrows,
        ),
        (
            // `HAS_ANCESTOR` chose the collection and was stripped out of the
            // predicate; the property filter still has to narrow it.
            "ancestor AND priority = 3",
            task_query(Some(conjunction(vec![
                has_ancestor(vec![named("Owner", "ada")]),
                property_filter("priority", Op::Equal, integer(3)),
            ]))),
            Narrows,
        ),
        (
            // An ancestor on its own: no property predicate survives, so the
            // ancestor collection is read whole. The answer still has to be the
            // entities under it and nothing else.
            "ancestor only",
            task_query(Some(has_ancestor(vec![named("Owner", "ada")]))),
            ReadsEverything,
        ),
        (
            "owner IN [ada]",
            task_query(Some(property_filter(
                "owner",
                Op::In,
                array(vec![text("ada")]),
            ))),
            Narrows,
        ),
        (
            // Candidates of mixed types, including the double that equals an
            // integer and the null that only matches a null.
            "priority IN [1, 3.0, null]",
            task_query(Some(property_filter(
                "priority",
                Op::In,
                array(vec![
                    integer(1),
                    double(3.0),
                    ds_value(DsValueType::NullValue(0)),
                ]),
            ))),
            Narrows,
        ),
        (
            "an OR of two terms",
            task_query(Some(disjunction(vec![
                property_filter("priority", Op::Equal, integer(3)),
                property_filter("owner", Op::Equal, text("bob")),
            ]))),
            Narrows,
        ),
        (
            // A pushed OR nested in an AND beside a conjunct that is not.
            "(owner = ada OR priority IN [9]) AND priority != 1",
            task_query(Some(conjunction(vec![
                disjunction(vec![
                    property_filter("owner", Op::Equal, text("ada")),
                    property_filter("priority", Op::In, array(vec![integer(9)])),
                ]),
                property_filter("priority", Op::NotEqual, integer(1)),
            ]))),
            Narrows,
        ),
        // --- the untranslatable shapes -------------------------------------
        //
        // Each must read everything. A narrowed read here is the failure this
        // half of the test exists for: entities dropped by a pre-filter the
        // translation had no business producing.
        (
            "priority != 3",
            task_query(Some(property_filter("priority", Op::NotEqual, integer(3)))),
            ReadsEverything,
        ),
        (
            "priority NOT_IN [1, 3]",
            task_query(Some(property_filter(
                "priority",
                Op::NotIn,
                array(vec![integer(1), integer(3)]),
            ))),
            ReadsEverything,
        ),
        (
            // One branch is `!=`, which could match any entity, so the OR as a
            // whole cannot narrow.
            "priority = 3 OR owner != ada",
            task_query(Some(disjunction(vec![
                property_filter("priority", Op::Equal, integer(3)),
                property_filter("owner", Op::NotEqual, text("ada")),
            ]))),
            ReadsEverything,
        ),
        (
            "a property path",
            task_query(Some(property_filter(
                "profile.plan",
                Op::Equal,
                text("pro"),
            ))),
            ReadsEverything,
        ),
        (
            // A one-term composite of *either* operator is flattened into that
            // term by `strip_ancestor_filter` before anything sees the
            // operator, so this is an equality by the time it is translated and
            // by the time it is evaluated. Pinned because the first version of
            // this corpus expected it to read everything, and finding out
            // otherwise is the only reason anyone knows the flattening is
            // there. Both paths agree either way, which is the part that
            // matters.
            "an OR of one term",
            task_query(Some(ds::Filter {
                filter_type: Some(ds::filter::FilterType::CompositeFilter(
                    ds::CompositeFilter {
                        op: ds::composite_filter::Operator::Or as i32,
                        filters: vec![property_filter("priority", Op::Equal, integer(3))],
                    },
                )),
            })),
            Narrows,
        ),
        (
            // One translatable conjunct and one that is not. The translatable
            // one is still pushed, so this narrows — and the whole filter is
            // still decided in process, which is what keeps it correct.
            "priority = 3 AND owner != bob",
            task_query(Some(conjunction(vec![
                property_filter("priority", Op::Equal, integer(3)),
                property_filter("owner", Op::NotEqual, text("bob")),
            ]))),
            Narrows,
        ),
    ]
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A running Datastore server over the same table, with the scan-size log.
struct Surface {
    server: local::RunningServer,
    client: DatastoreClient<Channel>,
    scans: Arc<Mutex<Vec<Returned>>>,
}

impl Surface {
    /// Start a server whose store is `wrap` applied over the counter, which
    /// sits over a real `PostgresStore`.
    async fn start(
        url: &str,
        project: &str,
        wrap: impl FnOnce(Arc<dyn Store>) -> Arc<dyn Store>,
    ) -> Self {
        let postgres = PostgresStore::connect(url, 4)
            .await
            .expect("connect to the test database and create the schema");
        let counter = CountingStore::new(Arc::new(postgres));
        let scans = counter.log();

        let config =
            local::LocalConfig::in_memory("127.0.0.1:0".parse().expect("valid bind address"))
                .with_project_id(project)
                .with_only(local::Service::Datastore);
        let server = local::spawn_with_store(config, wrap(Arc::new(counter)))
            .await
            .expect("server binds");
        let client = DatastoreClient::connect(format!("http://{}", server.addr))
            .await
            .expect("client connects to the bound port");

        Surface {
            server,
            client,
            scans,
        }
    }

    /// Run `query` and return the answer in a comparable shape, plus the sizes
    /// of the scans it did.
    async fn answer(&mut self, project: &str, query: ds::Query) -> (Vec<String>, Vec<Returned>) {
        self.scans.lock().expect("scan log poisoned").clear();

        let batch = self
            .client
            .run_query(ds::RunQueryRequest {
                project_id: project.to_string(),
                query_type: Some(ds::run_query_request::QueryType::Query(query)),
                ..Default::default()
            })
            .await
            .expect("RunQuery is accepted")
            .into_inner()
            .batch
            .expect("a batch");

        let answer = batch.entity_results.iter().map(describe).collect();
        let scans = self.scans.lock().expect("scan log poisoned").clone();
        (answer, scans)
    }
}

/// One entity result as a string: its key path and every property.
///
/// Properties go through a `BTreeMap` so two runs of the same query describe
/// them in the same order, and the whole property set is included rather than
/// just the key — a pushdown that returned the right entities with the wrong
/// contents would be just as broken.
fn describe(result: &ds::EntityResult) -> String {
    let entity = result.entity.as_ref().expect("a full entity result");
    let path = entity
        .key
        .as_ref()
        .expect("an entity has a key")
        .path
        .iter()
        .map(|element| {
            let id = match &element.id_type {
                Some(ds::key::path_element::IdType::Name(name)) => name.clone(),
                Some(ds::key::path_element::IdType::Id(id)) => id.to_string(),
                None => "<incomplete>".to_string(),
            };
            format!("{}/{}", element.kind, id)
        })
        .collect::<Vec<_>>()
        .join("/");

    let properties: BTreeMap<&String, String> = entity
        .properties
        .iter()
        .map(|(name, value)| (name, format!("{:?}", value.value_type)))
        .collect();
    format!("{path} {properties:?}")
}

/// Write the corpus through the public Commit RPC, so the rows land the way the
/// server lands them.
async fn seed(surface: &mut Surface, project: &str) {
    for (path, properties) in corpus() {
        surface
            .client
            .commit(ds::CommitRequest {
                project_id: project.to_string(),
                mode: ds::commit_request::Mode::NonTransactional as i32,
                mutations: vec![ds::Mutation {
                    operation: Some(ds::mutation::Operation::Upsert(ds::Entity {
                        key: Some(key(path)),
                        properties: properties
                            .into_iter()
                            .map(|(name, value)| (name.to_string(), value))
                            .collect(),
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .expect("Commit upsert");
    }
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

/// The rows the one scan of a query returned.
///
/// Every shape here reads exactly once; asserting that rather than summing
/// keeps a future change that reads twice from hiding inside a total.
fn rows_read(scans: &[Returned], case: &str) -> Returned {
    match scans {
        [single] => *single,
        other => panic!("{case:?} read the store {} times: {other:?}", other.len()),
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// Translating a Datastore filter into a pre-filter must not change a single
/// answer.
///
/// This is the test the feature rests on. The two surfaces differ only in
/// whether the store narrows the read, so any disagreement is the translation
/// claiming something about a value comparison that the evaluator does not
/// agree with.
#[tokio::test]
async fn pushdown_answers_every_datastore_query_as_a_full_scan_does() {
    let Some(url) = test_url() else { return };
    let project = unique_project();

    let mut pushed = Surface::start(&url, &project, |store| store).await;
    let mut scanned = Surface::start(&url, &project, |store| {
        Arc::new(PushdownBlindStore { inner: store })
    })
    .await;
    seed(&mut pushed, &project).await;

    for (case, query, expect) in cases() {
        let (pushed_answer, pushed_scans) = pushed.answer(&project, query.clone()).await;
        let (scanned_answer, scanned_scans) = scanned.answer(&project, query).await;

        assert_eq!(
            pushed_answer, scanned_answer,
            "the pushdown changed the answer of {case:?}"
        );

        let full = rows_read(&scanned_scans, case);
        let narrowed = rows_read(&pushed_scans, case);
        assert!(
            !full.filtered,
            "{case:?}: the blind stack must take the unfiltered path, or this \
             is not a comparison against a full scan"
        );
        assert!(
            narrowed.filtered,
            "{case:?}: the ordinary stack must reach the store through the \
             filtered scan"
        );

        match expect {
            Expect::Narrows => assert!(
                narrowed.rows < full.rows,
                "{case:?}: the filter was not pushed down — SQL returned all \
                 {} rows the full scan did. The agreement above would pass \
                 against a translation that narrowed nothing, which is why \
                 this assertion is here",
                full.rows
            ),
            Expect::ReadsEverything => assert_eq!(
                narrowed.rows, full.rows,
                "{case:?}: an untranslatable filter narrowed the read. A \
                 pre-filter must keep every row the query could match, so this \
                 means entities were dropped before anything evaluated them"
            ),
        }
    }

    pushed.server.shutdown().await.expect("server stops");
    scanned.server.shutdown().await.expect("server stops");
    remove_project(&url, &project).await;
}

/// The point of the feature: a selective Datastore query has to read less.
///
/// Stated once on its own, against the kind's real size, because the comparison
/// above is relative to whatever the full scan happened to return.
#[tokio::test]
async fn a_selective_datastore_query_reads_fewer_rows_than_the_kind_holds() {
    let Some(url) = test_url() else { return };
    let project = unique_project();

    let mut surface = Surface::start(&url, &project, |store| store).await;
    seed(&mut surface, &project).await;

    let (answer, scans) = surface
        .answer(
            &project,
            task_query(Some(property_filter(
                "owner",
                ds::property_filter::Operator::Equal,
                text("dan"),
            ))),
        )
        .await;

    let read = rows_read(&scans, "owner = dan");
    assert_eq!(
        read.rows,
        1,
        "one entity of {} has owner=dan, and a plain equality is exactly the \
         shape `fields @> …` and the GIN index exist for — SQL should hand back \
         just that one",
        kind_size()
    );
    assert_eq!(
        answer.len(),
        1,
        "and the query still answers with that entity"
    );

    surface.server.shutdown().await.expect("server stops");
    remove_project(&url, &project).await;
}
