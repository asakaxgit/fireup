//! The change feed behind `google.firestore.v1.Firestore.Listen`.
//!
//! Two pieces, deliberately separated:
//!
//! - [`WatchedStore`] is a [`Store`] decorator. It forwards every call to the
//!   store underneath and, after a batch commits, publishes one
//!   [`ChangeEvent`] per document the batch touched. Because it sits *below*
//!   the data plane, every write notices — Firestore `Commit`, `BatchWrite`,
//!   the single-document RPCs, a Datastore commit, and a seed — without any of
//!   them knowing a listener exists.
//! - [`listen`] serves one `Listen` stream: it tracks the stream's targets,
//!   answers each `AddTarget` with the target's current state, and then
//!   forwards the feed.
//!
//! The feed is **in-process**. With the Postgres store, a document written by
//! *another* `fireup local` process (or by psql) does not reach these
//! listeners, because nothing polls the table. That limit is in the fidelity
//! matrix in `docs/local/README.md`; it is a gap, not a wrong answer, and the
//! single-process case is the one a local SDK test actually uses.
//!
//! This is an independent implementation of the published `Listen` contract.
//! No Firebase Emulator Suite code or behaviour was reverse-engineered; see
//! `.cursor/rules/firebase-gcp-compliance.mdc`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use prost_types::Timestamp;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Status, Streaming};
use tracing::debug;

use crate::local::names::{parse_document_name, parse_parent_name};
use crate::local::query::{Prefilter, Query};
use crate::local::store::{
    now_timestamp, Mutation, PendingWrite, SharedStore, Store, StoreResult, WriteEffect,
    WriteOutcome,
};
use crate::protos::google::firestore::v1::{
    listen_request::TargetChange as RequestedChange,
    listen_response::ResponseType,
    target::{query_target::QueryType, DocumentsTarget, QueryTarget, ResumeType, TargetType},
    target_change::TargetChangeType,
    Document, DocumentChange, DocumentDelete, DocumentRemove, ListenRequest, ListenResponse,
    Target, TargetChange,
};
use crate::protos::google::rpc::Status as RpcStatus;

/// How many change events the feed buffers for a listener that is behind.
///
/// A listener that falls further behind than this is told to `RESET` rather
/// than quietly skipping the events it missed.
const EVENT_CAPACITY: usize = 512;

/// Outbound buffer for one `Listen` stream.
const RESPONSE_CAPACITY: usize = 64;

/// Inbound buffer for one `Listen` stream's requests.
const REQUEST_CAPACITY: usize = 16;

/// One document's worth of change, as the store left it.
#[derive(Debug)]
pub enum Change {
    /// Created or updated. Carries the document as the store holds it, so a
    /// listener sees the same `create_time`/`update_time` a read would return.
    Written(Document),
    /// Deleted.
    Removed(String),
}

impl Change {
    /// Resource name of the document that changed.
    pub fn name(&self) -> &str {
        match self {
            Change::Written(document) => &document.name,
            Change::Removed(name) => name,
        }
    }
}

/// A change plus the time of the commit that produced it.
#[derive(Clone, Debug)]
pub struct ChangeEvent {
    /// Behind an `Arc` because the feed fans one event out to every listener
    /// and a `Document` is not cheap to clone.
    pub change: Arc<Change>,
    pub commit_time: Timestamp,
}

/// The change feed every listener on this process subscribes to.
pub struct WatchHub {
    sender: broadcast::Sender<ChangeEvent>,
    /// Set once, when the server is going away.
    ///
    /// Needed because a `Listen` stream is open-ended by design: tonic's
    /// graceful shutdown waits for in-flight requests, and a listener with
    /// nothing left to say would hold the connection — and therefore the whole
    /// server — open forever. Dropping the hub cannot be the signal, because
    /// the service that owns it is itself owned by the server task.
    closing: tokio::sync::watch::Sender<bool>,
}

impl Default for WatchHub {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchHub {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_CAPACITY);
        let (closing, _) = tokio::sync::watch::channel(false);
        Self { sender, closing }
    }

    /// Subscribe to every change published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.sender.subscribe()
    }

    /// End every live `Listen` stream.
    ///
    /// Called on shutdown so an open listener does not keep the server from
    /// stopping. Idempotent.
    pub fn close(&self) {
        let _ = self.closing.send(true);
    }

    fn closing(&self) -> tokio::sync::watch::Receiver<bool> {
        self.closing.subscribe()
    }

    /// Whether anyone is listening.
    ///
    /// The write path checks this so that with no listeners — the normal case —
    /// publishing costs nothing at all, not even the read-back below.
    pub fn watching(&self) -> bool {
        self.sender.receiver_count() > 0
    }

    /// Publish one change. A send with no receivers is not an error.
    pub fn publish(&self, event: ChangeEvent) {
        let _ = self.sender.send(event);
    }
}

/// A [`Store`] that publishes what it writes.
///
/// A decorator rather than a method on each backend, so `MemoryStore` and
/// `PostgresStore` cannot drift on which writes produce an event.
pub struct WatchedStore {
    inner: SharedStore,
    hub: Arc<WatchHub>,
}

/// Wrap `store` in a change feed, returning the store to serve from and the hub
/// listeners subscribe to.
pub fn attach(store: SharedStore) -> (SharedStore, Arc<WatchHub>) {
    let hub = Arc::new(WatchHub::new());
    let watched = WatchedStore {
        inner: store,
        hub: hub.clone(),
    };
    (Arc::new(watched), hub)
}

#[async_trait::async_trait]
impl Store for WatchedStore {
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
        if !self.hub.watching() {
            return self.inner.commit(writes, now).await;
        }

        // Names are captured before the batch moves into the backend. A
        // `Verify` changes nothing — the backend short-circuits it before even
        // its transforms run — so it contributes no name.
        let touched: Vec<Option<String>> = writes
            .iter()
            .map(|write| match write.mutation {
                Mutation::Verify { .. } => None,
                _ => Some(write.mutation.name().to_string()),
            })
            .collect();

        let effects = self.inner.commit(writes, now.clone()).await?;

        // One event per *document*, not per write: a batch that touches the
        // same document twice is a single change as far as a listener is
        // concerned, and the last write is the one that decided its state.
        let mut final_state: Vec<(String, WriteOutcome)> = Vec::new();
        for (name, effect) in touched.into_iter().zip(&effects) {
            let Some(name) = name else { continue };
            match final_state.iter_mut().find(|(seen, _)| *seen == name) {
                Some(slot) => slot.1 = effect.outcome.clone(),
                None => final_state.push((name, effect.outcome.clone())),
            }
        }

        for (name, outcome) in final_state {
            let change = match outcome {
                WriteOutcome::Deleted { existed: true } => Change::Removed(name),
                // Nothing moved: a delete that found nothing, or a read-set
                // verification. Publishing either would make a listener report
                // a change that never happened.
                WriteOutcome::Deleted { existed: false } | WriteOutcome::Verified => continue,
                WriteOutcome::Created | WriteOutcome::Updated => {
                    // Read back rather than reconstructed from the mutation:
                    // `create_time` preservation and merge semantics live in
                    // the backend, and a listener handed a document the store
                    // does not actually hold is worse than a missed event.
                    match self.inner.get(&name).await? {
                        Some(document) => Change::Written(document),
                        // A concurrent writer deleted it already; their own
                        // delete event is the truthful one.
                        None => continue,
                    }
                }
            };
            self.hub.publish(ChangeEvent {
                change: Arc::new(change),
                commit_time: now.clone(),
            });
        }

        Ok(effects)
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

    // The two pushdown reads must be forwarded explicitly. Their trait defaults
    // are *correct* — they drop the prefilter and scan everything — so leaving
    // them out costs no answer and fails no test, it just silently turns the
    // Postgres pushdown off for the whole server, because the store the data
    // plane holds is this decorator rather than the backend underneath.
    async fn scan_collection_filtered(
        &self,
        parent: &str,
        collection_id: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        self.inner
            .scan_collection_filtered(parent, collection_id, prefilter)
            .await
    }

    async fn scan_kind_filtered(
        &self,
        documents_root: &str,
        kind: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
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

/// A `Listen` target, compiled.
#[derive(Debug)]
enum Tracked {
    /// `Target.DocumentsTarget` — a fixed set of document names.
    Documents(BTreeSet<String>),
    /// `Target.QueryTarget` — a `StructuredQuery`, over one collection or, for
    /// a collection group query, every collection with that id beneath
    /// `parent`.
    Query {
        parent: String,
        /// The key prefix every document the query could return shares:
        /// `{parent}/{collection_id}/` for an ordinary query, `{parent}/` for a
        /// collection group one. Precomputed so a change can be tested against
        /// the target without recompiling anything.
        prefix: String,
        query: Query,
    },
}

impl Tracked {
    /// Whether a change to `name` could possibly affect this target.
    ///
    /// For a query target this is membership of the collection, not of the
    /// result set: a document that did not match before may match now, and
    /// only re-evaluating the query can tell.
    fn covers(&self, name: &str) -> bool {
        match self {
            Tracked::Documents(names) => names.contains(name),
            Tracked::Query { prefix, query, .. } => {
                let Some(rest) = name.strip_prefix(prefix.as_str()) else {
                    return false;
                };
                if !query.all_descendants() {
                    // Documents in a subcollection share the prefix but have a
                    // further `/` and belong to a different collection.
                    return !rest.is_empty() && !rest.contains('/');
                }
                // A collection group query reads every collection with this id
                // anywhere beneath the parent, so what matters is the document's
                // own collection id — the second-to-last path segment. Same rule
                // the group scan in the store uses, so the two cannot disagree
                // about which documents belong to the target.
                let segments: Vec<&str> = rest.split('/').collect();
                segments.len() >= 2
                    && segments.len() % 2 == 0
                    && segments[segments.len() - 2] == query.collection_id()
            }
        }
    }
}

/// One live target on one stream.
struct ActiveTarget {
    tracked: Tracked,
    /// What the client has been told, as document name → `update_time`. A
    /// re-evaluation reports only what actually moved, so a write elsewhere in
    /// a watched collection does not resend the whole result set.
    sent: BTreeMap<String, Option<Timestamp>>,
    /// `Target.once`: drop the target as soon as it is `CURRENT`.
    once: bool,
}

/// The state of one `Listen` stream.
struct Session {
    store: SharedStore,
    out: mpsc::Sender<Result<ListenResponse, Status>>,
    targets: BTreeMap<i32, ActiveTarget>,
}

/// Build one `TargetChange` response.
///
/// `resume_token` is never set. Fireup mints no resume tokens, so a client has
/// nothing to resume from and reconnects with a full snapshot instead — which
/// is correct, where a token we could not honour would make the client skip the
/// changes it missed.
fn target_change(
    change_type: TargetChangeType,
    target_ids: Vec<i32>,
    read_time: Option<Timestamp>,
    cause: Option<RpcStatus>,
) -> ListenResponse {
    ListenResponse {
        response_type: Some(ResponseType::TargetChange(TargetChange {
            target_change_type: change_type as i32,
            target_ids,
            cause,
            resume_token: Vec::new(),
            read_time,
        })),
    }
}

/// The response for a document that left a target.
///
/// `DocumentDelete` and `DocumentRemove` are not interchangeable in the
/// contract: delete means the document is gone from the database, remove means
/// it still exists but no longer belongs to this target. A client that merges
/// several listeners' views needs the difference.
fn left_target(name: &str, id: i32, read_time: &Timestamp, deleted: bool) -> ListenResponse {
    let response_type = if deleted {
        ResponseType::DocumentDelete(DocumentDelete {
            document: name.to_string(),
            removed_target_ids: vec![id],
            read_time: Some(read_time.clone()),
        })
    } else {
        ResponseType::DocumentRemove(DocumentRemove {
            document: name.to_string(),
            removed_target_ids: vec![id],
            read_time: Some(read_time.clone()),
        })
    };
    ListenResponse {
        response_type: Some(response_type),
    }
}

/// Compile an `AddTarget` into something this server can track, rejecting every
/// target shape it does not serve.
fn build_target(target: &Target) -> Result<Tracked, Status> {
    match &target.resume_type {
        None => {}
        Some(ResumeType::ResumeToken(token)) if token.is_empty() => {}
        Some(ResumeType::ResumeToken(_)) => {
            // Fireup issues no resume tokens, so any token that arrives came
            // from somewhere else. A full snapshot is the safe answer: the
            // client resets to it, where pretending to resume would silently
            // skip whatever changed while it was away.
            debug!("ignoring a resume_token fireup did not issue; sending a full snapshot");
        }
        Some(ResumeType::ReadTime(_)) => {
            return Err(Status::unimplemented(
                "fireup local does not serve a Listen target resumed from a read_time \
                 (the store only keeps the current version of a document). \
                 See docs/local/README.md for the full fidelity matrix.",
            ))
        }
    }

    match &target.target_type {
        Some(TargetType::Documents(DocumentsTarget { documents })) => {
            // A `BTreeSet` because the contract says duplicate names are
            // elided, and it gives the snapshot a deterministic order.
            let mut names = BTreeSet::new();
            for document in documents {
                names.insert(parse_document_name(document)?.full());
            }
            if names.is_empty() {
                return Err(Status::invalid_argument(
                    "a documents target must name at least one document",
                ));
            }
            Ok(Tracked::Documents(names))
        }
        Some(TargetType::Query(QueryTarget { parent, query_type })) => {
            let parent = parse_parent_name(parent)?.full();
            let Some(QueryType::StructuredQuery(structured)) = query_type else {
                return Err(Status::invalid_argument(
                    "a query target needs a structured_query",
                ));
            };
            // Compiled up front, so an unsupported query shape is refused when
            // the target is added rather than on the first change.
            let query = Query::compile(structured)?;
            let prefix = if query.all_descendants() {
                format!("{parent}/")
            } else {
                format!("{parent}/{}/", query.collection_id())
            };
            Ok(Tracked::Query {
                parent,
                prefix,
                query,
            })
        }
        None => Err(Status::invalid_argument(
            "a target must be a query or a set of documents",
        )),
    }
}

/// The documents a target matches right now, in the order the client should see
/// them.
async fn evaluate(store: &SharedStore, tracked: &Tracked) -> Result<Vec<Document>, Status> {
    match tracked {
        Tracked::Documents(names) => {
            let mut found = Vec::with_capacity(names.len());
            for name in names {
                // A watched document that does not exist contributes nothing:
                // the client learns it is absent from the `CURRENT` that
                // arrives with no change for it.
                if let Some(document) = store.get(name).await? {
                    found.push(document);
                }
            }
            Ok(found)
        }
        Tracked::Query { parent, query, .. } => {
            // The same two scans `RunQuery` picks between, so watching a query
            // and reading it once see the same documents.
            let documents = if query.all_descendants() {
                store.scan_kind(parent, query.collection_id()).await?
            } else {
                store.scan_collection(parent, query.collection_id()).await?
            };
            Ok(query.evaluate(documents).documents)
        }
    }
}

impl Session {
    /// Send `responses` in order. `false` means the client is gone and the
    /// stream should end.
    async fn flush(&self, responses: Vec<ListenResponse>) -> bool {
        for response in responses {
            if self.out.send(Ok(response)).await.is_err() {
                return false;
            }
        }
        true
    }

    /// Tell the client a target could not be served, and why.
    ///
    /// `REMOVE` with a `cause` is the contract's per-target error channel. Used
    /// instead of failing the whole stream so one bad target does not take a
    /// client's other listeners down with it.
    async fn refuse(&self, id: i32, status: Status) -> bool {
        debug!(target = id, %status, "Listen target refused");
        self.flush(vec![target_change(
            TargetChangeType::Remove,
            vec![id],
            None,
            Some(RpcStatus {
                code: status.code() as i32,
                message: status.message().to_string(),
                details: Vec::new(),
            }),
        )])
        .await
    }

    /// Smallest positive id not already in use on this stream.
    fn next_free_id(&self) -> i32 {
        (1..i32::MAX)
            .find(|id| !self.targets.contains_key(id))
            .unwrap_or(1)
    }

    async fn handle(&mut self, request: ListenRequest) -> bool {
        match request.target_change {
            Some(RequestedChange::AddTarget(target)) => self.add_target(target).await,
            Some(RequestedChange::RemoveTarget(id)) => self.remove_target(id).await,
            // The protocol allows a request that carries only labels. Nothing
            // to do, and not an error.
            None => true,
        }
    }

    async fn add_target(&mut self, target: Target) -> bool {
        // `target_id = 0` means "assign one". The contract says clients should
        // supply their own, and every official SDK does; assigning one is still
        // better than refusing a legal request.
        let id = if target.target_id == 0 {
            self.next_free_id()
        } else {
            target.target_id
        };
        if id < 0 {
            return self
                .refuse(id, Status::invalid_argument("target_id must be positive"))
                .await;
        }

        let tracked = match build_target(&target) {
            Ok(tracked) => tracked,
            Err(status) => return self.refuse(id, status).await,
        };

        // Re-adding a live id resets that target rather than erroring. It is
        // the state the client would reach by removing and re-adding it, and
        // the alternative leaves the two sides disagreeing about what the id
        // names.
        self.targets.insert(
            id,
            ActiveTarget {
                tracked,
                sent: BTreeMap::new(),
                once: target.once,
            },
        );

        if !self
            .flush(vec![target_change(
                TargetChangeType::Add,
                vec![id],
                None,
                None,
            )])
            .await
        {
            return false;
        }
        self.snapshot(id).await
    }

    async fn remove_target(&mut self, id: i32) -> bool {
        if self.targets.remove(&id).is_none() {
            return self
                .refuse(
                    id,
                    Status::not_found(format!("no active target {id} on this stream")),
                )
                .await;
        }
        self.flush(vec![target_change(
            TargetChangeType::Remove,
            vec![id],
            None,
            None,
        )])
        .await
    }

    /// Send a target's current state, then `CURRENT`.
    async fn snapshot(&mut self, id: i32) -> bool {
        let read_time = now_timestamp();
        let mut responses = match self.diff(id, &read_time, None).await {
            Ok(responses) => responses,
            Err(status) => {
                // The target cannot be served at all, so it does not stay on
                // the stream pretending to work.
                self.targets.remove(&id);
                return self.refuse(id, status).await;
            }
        };

        responses.push(target_change(
            TargetChangeType::Current,
            vec![id],
            Some(read_time.clone()),
            None,
        ));
        if self.targets.get(&id).is_some_and(|active| active.once) {
            self.targets.remove(&id);
            responses.push(target_change(
                TargetChangeType::Remove,
                vec![id],
                None,
                None,
            ));
        }
        responses.push(self.consistent_at(read_time));
        self.flush(responses).await
    }

    /// The whole-stream snapshot marker: `NO_CHANGE` with no target ids and a
    /// `read_time`.
    ///
    /// This is the message that makes a client *deliver* a snapshot — the
    /// official SDKs accumulate document changes and only hand them to the
    /// application when this arrives. Without it a listener looks like it
    /// silently never fires.
    fn consistent_at(&self, read_time: Timestamp) -> ListenResponse {
        target_change(
            TargetChangeType::NoChange,
            Vec::new(),
            Some(read_time),
            None,
        )
    }

    /// Re-evaluate a target and return only the changes the client has not
    /// already been told about.
    ///
    /// `deleted` names the document a delete event removed — the one case that
    /// reports `DocumentDelete` rather than `DocumentRemove`.
    async fn diff(
        &mut self,
        id: i32,
        read_time: &Timestamp,
        deleted: Option<&str>,
    ) -> Result<Vec<ListenResponse>, Status> {
        let Some(active) = self.targets.get(&id) else {
            return Ok(Vec::new());
        };
        let documents = evaluate(&self.store, &active.tracked).await?;

        let current: BTreeMap<String, Option<Timestamp>> = documents
            .iter()
            .map(|document| (document.name.clone(), document.update_time.clone()))
            .collect();

        let active = self
            .targets
            .get_mut(&id)
            .expect("the target was present a moment ago and only this task removes targets");

        let mut responses = Vec::new();
        for document in documents {
            if active.sent.get(&document.name) == Some(&document.update_time) {
                continue; // unchanged since the client last saw it
            }
            responses.push(ListenResponse {
                response_type: Some(ResponseType::DocumentChange(DocumentChange {
                    document: Some(document),
                    target_ids: vec![id],
                    removed_target_ids: Vec::new(),
                })),
            });
        }
        for name in active.sent.keys() {
            if current.contains_key(name) {
                continue;
            }
            responses.push(left_target(
                name,
                id,
                read_time,
                deleted == Some(name.as_str()),
            ));
        }

        active.sent = current;
        Ok(responses)
    }

    /// Apply one change to a documents target.
    ///
    /// No re-read: the event already carries exactly the document this target
    /// watches, so a watched-document listener costs nothing per change.
    fn apply_direct(
        &mut self,
        id: i32,
        event: &ChangeEvent,
        read_time: &Timestamp,
    ) -> Vec<ListenResponse> {
        let Some(active) = self.targets.get_mut(&id) else {
            return Vec::new();
        };
        match event.change.as_ref() {
            Change::Written(document) => {
                if active.sent.get(&document.name) == Some(&document.update_time) {
                    return Vec::new();
                }
                active
                    .sent
                    .insert(document.name.clone(), document.update_time.clone());
                vec![ListenResponse {
                    response_type: Some(ResponseType::DocumentChange(DocumentChange {
                        document: Some(document.clone()),
                        target_ids: vec![id],
                        removed_target_ids: Vec::new(),
                    })),
                }]
            }
            Change::Removed(name) => {
                // Never reported as present, so there is nothing to retract.
                if active.sent.remove(name).is_none() {
                    return Vec::new();
                }
                vec![left_target(name, id, read_time, true)]
            }
        }
    }

    /// Fan one change out to every target it affects.
    async fn apply(&mut self, event: &ChangeEvent) -> bool {
        let name = event.change.name();
        let affected: Vec<i32> = self
            .targets
            .iter()
            .filter(|(_, active)| active.tracked.covers(name))
            .map(|(id, _)| *id)
            .collect();
        if affected.is_empty() {
            return true;
        }

        let read_time = event.commit_time.clone();
        let deleted = match event.change.as_ref() {
            Change::Removed(name) => Some(name.as_str()),
            Change::Written(_) => None,
        };

        let mut responses = Vec::new();
        for id in affected {
            let direct = matches!(
                self.targets.get(&id).map(|active| &active.tracked),
                Some(Tracked::Documents(_))
            );
            if direct {
                responses.extend(self.apply_direct(id, event, &read_time));
                continue;
            }
            match self.diff(id, &read_time, deleted).await {
                Ok(changes) => responses.extend(changes),
                Err(status) => {
                    // Whatever was already computed still goes out, in order,
                    // before the target is retired.
                    if !self.flush(std::mem::take(&mut responses)).await {
                        return false;
                    }
                    self.targets.remove(&id);
                    if !self.refuse(id, status).await {
                        return false;
                    }
                }
            }
        }

        // Sent even when the diffs were empty: the client learns the stream
        // reached a new consistent snapshot, which is what lets it tell "no
        // change" apart from "not heard from".
        responses.push(self.consistent_at(read_time));
        self.flush(responses).await
    }

    /// The feed dropped events for this listener, so its targets no longer have
    /// a base state worth diffing against.
    ///
    /// `RESET` is the contract's answer: forget what you were told, this
    /// snapshot is the new truth.
    async fn reset_all(&mut self) -> bool {
        let ids: Vec<i32> = self.targets.keys().copied().collect();
        for id in ids {
            if let Some(active) = self.targets.get_mut(&id) {
                active.sent.clear();
            }
            if !self
                .flush(vec![target_change(
                    TargetChangeType::Reset,
                    vec![id],
                    None,
                    None,
                )])
                .await
            {
                return false;
            }
            if !self.snapshot(id).await {
                return false;
            }
        }
        true
    }
}

/// Serve one `Listen` stream.
///
/// Returns immediately with the response stream; the stream is driven by a
/// background task that ends when the client closes it, the feed shuts down, or
/// a response cannot be delivered.
pub fn listen(
    store: SharedStore,
    hub: &WatchHub,
    mut requests: Streaming<ListenRequest>,
) -> ReceiverStream<Result<ListenResponse, Status>> {
    // Subscribed before any target exists, so a write landing between a
    // target's snapshot read and its first event cannot be missed. It may be
    // seen twice instead, which the `update_time` check in `diff` absorbs.
    let mut events = hub.subscribe();
    let mut closing = hub.closing();
    let (out, receiver) = mpsc::channel(RESPONSE_CAPACITY);

    // Inbound requests are pumped through a channel rather than polled inside
    // the `select!` below: `Streaming::message` is not documented as
    // cancel-safe, and the `select!` drops whichever future did not win.
    let (request_tx, mut request_rx) =
        mpsc::channel::<Result<ListenRequest, Status>>(REQUEST_CAPACITY);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                // The session ended, so nothing will read another request.
                // Dropping the inbound body here is what lets the connection
                // close instead of lingering on a read that no longer matters.
                _ = request_tx.closed() => break,
                message = requests.message() => match message {
                    Ok(Some(request)) => {
                        if request_tx.send(Ok(request)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break, // the client half-closed
                    Err(status) => {
                        let _ = request_tx.send(Err(status)).await;
                        break;
                    }
                },
            }
        }
    });

    tokio::spawn(async move {
        // `changed()` below only reports a *new* value, so a stream opened
        // after the server already started closing has to notice on its own.
        if *closing.borrow() {
            return;
        }

        let mut session = Session {
            store,
            out,
            targets: BTreeMap::new(),
        };
        // A clone so the abandoned-stream branch below does not borrow
        // `session` while the other branches' handlers mutate it. `closed()`
        // resolves on the *receiver* going away, which a clone does not affect.
        let hangup = session.out.clone();
        // Once the client half-closes it can add no more targets, but the ones
        // it already added stay live — so the loop keeps running with that
        // branch disabled rather than ending the stream.
        let mut half_closed = false;

        loop {
            tokio::select! {
                request = request_rx.recv(), if !half_closed => match request {
                    None => half_closed = true,
                    Some(Err(status)) => {
                        let _ = session.out.send(Err(status)).await;
                        break;
                    }
                    Some(Ok(request)) => {
                        if !session.handle(request).await {
                            break;
                        }
                    }
                },
                _ = hangup.closed() => break,
                // The server is stopping. Ending the stream here is what lets
                // its graceful shutdown finish. `changed()` rather than
                // `wait_for`, whose guard is not `Send` and so cannot be held
                // across the other branches' awaits.
                _ = closing.changed() => break,
                event = events.recv() => match event {
                    Ok(event) => {
                        if !session.apply(&event).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        debug!(
                            missed,
                            "a Listen stream fell behind the change feed; resetting its targets"
                        );
                        if !session.reset_all().await {
                            break;
                        }
                    }
                    // The hub is gone, i.e. the server is shutting down.
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        debug!("Listen stream closed");
    });

    ReceiverStream::new(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::store::{conformance, MemoryStore};
    use crate::protos::google::firestore::v1::structured_query::CollectionSelector;
    use crate::protos::google::firestore::v1::StructuredQuery;

    const ROOT: &str = "projects/demo/databases/(default)/documents";

    fn documents_target(names: &[&str]) -> Target {
        Target {
            target_id: 1,
            target_type: Some(TargetType::Documents(DocumentsTarget {
                documents: names.iter().map(|name| name.to_string()).collect(),
            })),
            ..Default::default()
        }
    }

    fn group_query_target(collection_id: &str, all_descendants: bool) -> Target {
        Target {
            target_id: 2,
            target_type: Some(TargetType::Query(QueryTarget {
                parent: ROOT.to_string(),
                query_type: Some(QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection_id.to_string(),
                        all_descendants,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        }
    }

    fn query_target(collection_id: &str) -> Target {
        group_query_target(collection_id, false)
    }

    #[test]
    fn a_documents_target_covers_exactly_the_names_it_lists() {
        let tracked =
            build_target(&documents_target(&[&format!("{ROOT}/users/alice")])).expect("compiles");

        assert!(tracked.covers(&format!("{ROOT}/users/alice")));
        assert!(!tracked.covers(&format!("{ROOT}/users/bob")));
    }

    #[test]
    fn a_query_target_covers_its_collection_but_not_subcollections() {
        let tracked = build_target(&query_target("users")).expect("compiles");

        assert!(tracked.covers(&format!("{ROOT}/users/alice")));
        assert!(
            !tracked.covers(&format!("{ROOT}/users/alice/posts/p1")),
            "a subcollection document shares the prefix but is a different collection"
        );
        assert!(!tracked.covers(&format!("{ROOT}/cities/sf")));
    }

    #[test]
    fn a_read_time_resume_is_refused_rather_than_approximated() {
        let target = Target {
            resume_type: Some(ResumeType::ReadTime(conformance::ts(10))),
            ..query_target("users")
        };

        let status = build_target(&target).expect_err("read_time resume is out of scope");
        assert_eq!(status.code(), tonic::Code::Unimplemented);
    }

    #[test]
    fn a_resume_token_we_never_issued_falls_back_to_a_full_snapshot() {
        let target = Target {
            resume_type: Some(ResumeType::ResumeToken(b"not-ours".to_vec())),
            ..query_target("users")
        };

        build_target(&target).expect("an unknown token is ignored, not an error");
    }

    #[test]
    fn a_collection_group_target_covers_the_collection_at_any_depth() {
        let tracked = build_target(&group_query_target("posts", true)).expect("compiles");

        assert!(tracked.covers(&format!("{ROOT}/posts/p1")));
        assert!(
            tracked.covers(&format!("{ROOT}/users/alice/posts/p1")),
            "a collection group target reaches a nested collection of the same id"
        );
        assert!(
            !tracked.covers(&format!("{ROOT}/users/alice")),
            "a document in a different collection is not covered, however deep"
        );
        assert!(!tracked.covers(&format!("{ROOT}/posts/p1/comments/c1")));
    }

    #[test]
    fn an_unsupported_query_shape_is_refused_when_the_target_is_added() {
        // A collection group query with no collection id is the kindless scan
        // of everything under the parent, which the query engine does not
        // serve. Refused when the target is added, not on the first change.
        let status = build_target(&group_query_target("", true))
            .expect_err("a kindless collection group scan is out of scope");
        assert_eq!(status.code(), tonic::Code::Unimplemented);
    }

    #[tokio::test]
    async fn a_commit_publishes_one_event_per_document_it_touched() {
        let (store, hub) = attach(Arc::new(MemoryStore::new()));
        let mut events = hub.subscribe();
        let name = format!("{ROOT}/users/alice");

        store
            .commit(
                vec![
                    PendingWrite::new(Mutation::Put(conformance::doc(&name, "k", "v1"))),
                    // A second write to the same document in the same batch.
                    PendingWrite::new(Mutation::Put(conformance::doc(&name, "k", "v2"))),
                ],
                conformance::ts(10),
            )
            .await
            .expect("commit");

        let event = events.recv().await.expect("one event");
        match event.change.as_ref() {
            Change::Written(document) => {
                assert_eq!(document.name, name);
                assert_eq!(
                    document.fields["k"],
                    conformance::string_value("v2"),
                    "the event carries the state the batch left behind"
                );
            }
            other => panic!("expected a write, got {other:?}"),
        }
        assert!(
            events.try_recv().is_err(),
            "two writes to one document are one change"
        );
    }

    #[tokio::test]
    async fn a_delete_that_removed_nothing_publishes_nothing() {
        let (store, hub) = attach(Arc::new(MemoryStore::new()));
        let mut events = hub.subscribe();

        store
            .delete(&format!("{ROOT}/users/nobody"))
            .await
            .expect("delete");

        assert!(
            events.try_recv().is_err(),
            "an idempotent delete that found nothing changed nothing"
        );
    }

    #[tokio::test]
    async fn a_delete_publishes_the_removal() {
        let (store, hub) = attach(Arc::new(MemoryStore::new()));
        let name = format!("{ROOT}/users/alice");
        store
            .put(conformance::doc(&name, "k", "v"), conformance::ts(10))
            .await
            .expect("put");

        let mut events = hub.subscribe();
        store.delete(&name).await.expect("delete");

        let event = events.recv().await.expect("one event");
        assert!(matches!(event.change.as_ref(), Change::Removed(removed) if *removed == name));
    }

    #[tokio::test]
    async fn a_read_set_verification_publishes_nothing() {
        let (store, hub) = attach(Arc::new(MemoryStore::new()));
        let name = format!("{ROOT}/users/alice");
        store
            .put(conformance::doc(&name, "k", "v"), conformance::ts(10))
            .await
            .expect("put");

        let mut events = hub.subscribe();
        store
            .commit(
                vec![PendingWrite::new(Mutation::Verify { name })],
                conformance::ts(20),
            )
            .await
            .expect("verification holds");

        assert!(
            events.try_recv().is_err(),
            "a verification changes no document"
        );
    }

    #[tokio::test]
    async fn the_watched_store_still_satisfies_the_store_contract() {
        // The decorator must not change what the store underneath promises —
        // including with a live subscriber, which turns on the read-back path.
        let (store, hub) = attach(Arc::new(MemoryStore::new()));
        let _subscriber = hub.subscribe();

        conformance::assert_store_contract(store.as_ref(), ROOT).await;
    }

    /// A store that answers like `MemoryStore` but records which of the two
    /// scan spellings the caller actually reached.
    struct ScanRecorder {
        inner: SharedStore,
        filtered: Arc<std::sync::atomic::AtomicUsize>,
        unfiltered: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Store for ScanRecorder {
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
            self.unfiltered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.scan_collection(parent, collection_id).await
        }

        async fn scan_collection_filtered(
            &self,
            parent: &str,
            collection_id: &str,
            prefilter: Prefilter<'_>,
        ) -> StoreResult<Vec<Document>> {
            self.filtered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner
                .scan_collection_filtered(parent, collection_id, prefilter)
                .await
        }

        async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>> {
            self.unfiltered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.scan_kind(documents_root, kind).await
        }

        async fn scan_kind_filtered(
            &self,
            documents_root: &str,
            kind: &str,
            prefilter: Prefilter<'_>,
        ) -> StoreResult<Vec<Document>> {
            self.filtered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

    /// The decorator has to forward the *pushdown* reads, not just the plain
    /// ones.
    ///
    /// `scan_collection_filtered`/`scan_kind_filtered` have trait defaults that
    /// discard the prefilter and scan the whole collection. That is a correct
    /// answer, so if the decorator omits them nothing fails — the Postgres
    /// pushdown is simply never reached, because the store the data plane holds
    /// is this decorator and not the backend. Delete either forwarding method in
    /// `impl Store for WatchedStore` and this test fails.
    #[tokio::test]
    async fn the_watched_store_forwards_the_pushdown_reads() {
        let filtered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let unfiltered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorder = Arc::new(ScanRecorder {
            inner: Arc::new(MemoryStore::new()),
            filtered: filtered.clone(),
            unfiltered: unfiltered.clone(),
        });
        let (store, _hub) = attach(recorder);

        let empty: Vec<crate::local::query::FieldPath> = Vec::new();
        let prefilter = || Prefilter {
            filter: None,
            required_fields: &empty,
        };

        store
            .scan_collection_filtered(ROOT, "users", prefilter())
            .await
            .expect("filtered collection scan");
        store
            .scan_kind_filtered(ROOT, "users", prefilter())
            .await
            .expect("filtered kind scan");

        let load =
            |c: &Arc<std::sync::atomic::AtomicUsize>| c.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            load(&filtered),
            2,
            "both pushdown reads must reach the backend; got {} filtered and {} unfiltered, \
             which means a trait default scanned everything instead",
            load(&filtered),
            load(&unfiltered)
        );
        assert_eq!(
            load(&unfiltered),
            0,
            "a pushdown read must not fall back to the unfiltered scan"
        );
    }
}
