//! The document store behind `fireup local`.
//!
//! Phase 1 splits the Phase 0 map into a `Store` trait with two
//! implementations: [`MemoryStore`] (the default, nothing to set up) and
//! [`PostgresStore`](crate::local::postgres_store::PostgresStore) (durable, so
//! data survives a restart). The data plane only ever sees the trait, which is
//! what keeps the two honest: the same conformance tests run against both.
//!
//! Phase 2 adds exactly what the query engine and the write pipeline need, and
//! nothing beyond it:
//!
//! - [`Store::commit`] applies a batch of [`PendingWrite`]s **atomically**,
//!   with every precondition checked inside the same store transaction. It is
//!   the only required write method; `put`/`merge`/`delete` are now provided
//!   methods on top of it, so a backend cannot implement single writes and
//!   batches with different semantics.
//! - [`Store::scan_collection`] and [`Store::child_segments`] are the two
//!   collection reads `RunQuery`, `ListDocuments`, and `ListCollectionIds` are
//!   built from. Filtering, ordering, and limiting live in
//!   [`query`](crate::local::query) rather than in each backend, so both
//!   stores answer a query identically.
//!
//! [`Store::scan_collection_filtered`] is the one concession to backend
//! strengths: a backend may use a query's [`Prefilter`] to return fewer rows,
//! but only ever a superset of the matching ones, so it changes how much is
//! read and never what a query means. The default implementation ignores the
//! pre-filter, which is what [`MemoryStore`] uses.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use prost_types::Timestamp;
use tonic::Status;

use crate::local::query::Prefilter;
use crate::local::transforms::{self, FieldTransform};
use crate::local::value_json;
use crate::protos::google::firestore::v1::{Document, Value};

/// What went wrong in the storage layer.
///
/// Separate from `tonic::Status` so the store does not have opinions about gRPC
/// codes; the conversion at the bottom of this file is the single place that
/// decides how a storage failure is reported to a client.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The backend itself failed: connection lost, query rejected, pool empty.
    #[error("document store backend failed: {0:#}")]
    Backend(#[from] anyhow::Error),

    /// Data already in the store could not be read back. This is a data
    /// integrity problem, so it is never swallowed or treated as "not found".
    #[error("stored document {name} could not be decoded: {reason}")]
    Corrupt { name: String, reason: String },

    /// A document the caller handed us cannot be represented in this backend.
    #[error("{0}")]
    Unsupported(String),

    /// A document the caller handed us carries a value no Firestore document
    /// can hold, so the write is refused.
    ///
    /// Separate from [`StoreError::Unsupported`] because the two mean opposite
    /// things to a client: `Unsupported` is "this build cannot do that yet",
    /// which an SDK may surface as a missing feature, while this is "that
    /// value is not legal", which is the caller's to fix and pointless to
    /// retry.
    #[error("{0}")]
    InvalidDocument(String),

    /// A write precondition did not hold, so the whole batch was abandoned.
    #[error("{0}")]
    Precondition(#[from] PreconditionFailure),
}

/// Which precondition failed, and on what document.
///
/// Carried out of the store rather than turned into a `tonic::Status` on the
/// spot so the gRPC mapping stays in the one place at the bottom of this file;
/// the three cases map to three different status codes and getting that wrong
/// makes an SDK retry something it should surface.
#[derive(Debug, thiserror::Error)]
pub enum PreconditionFailure {
    /// `exists: true` — the document had to be there and was not.
    #[error("no document to update at {0}")]
    Missing(String),

    /// `exists: false` — the document had to be absent and was not.
    #[error("document already exists: {0}")]
    AlreadyExists(String),

    /// `update_time` did not match, i.e. somebody else wrote in between.
    #[error("update_time precondition failed for {0}")]
    UpdateTime(String),

    /// A document a transaction read changed before the transaction committed.
    ///
    /// Reported separately from the preconditions a client asked for, because
    /// the client did not ask for this one: the right answer is ABORTED, which
    /// is what tells an SDK to retry the whole transaction.
    #[error("transaction conflict: {0} changed after the transaction read it")]
    Conflict(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl From<StoreError> for Status {
    fn from(error: StoreError) -> Self {
        match &error {
            // `internal` for both backend and corruption failures: from the
            // client's side the server failed, and the detail is in the
            // message plus the server-side log.
            StoreError::Backend(_) | StoreError::Corrupt { .. } => {
                Status::internal(error.to_string())
            }
            StoreError::Unsupported(_) => Status::unimplemented(error.to_string()),
            StoreError::InvalidDocument(_) => Status::invalid_argument(error.to_string()),
            // These three codes are what the SDKs branch on: `create()` on an
            // existing document must be ALREADY_EXISTS and `update()` on a
            // missing one must be NOT_FOUND, or client-side error handling
            // takes the wrong path.
            StoreError::Precondition(failure) => match failure {
                PreconditionFailure::Missing(_) => Status::not_found(failure.to_string()),
                PreconditionFailure::AlreadyExists(_) => {
                    Status::already_exists(failure.to_string())
                }
                PreconditionFailure::UpdateTime(_) => {
                    Status::failed_precondition(failure.to_string())
                }
                PreconditionFailure::Conflict(_) => Status::aborted(failure.to_string()),
            },
        }
    }
}

/// What must hold of a document before a mutation is allowed to apply.
///
/// The store checks these rather than the data plane, because a check in the
/// data plane followed by a write is two round trips with a gap in between: two
/// concurrent `create()` calls can both see "absent" and both succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    /// The document must (or must not) exist.
    Exists(bool),
    /// The document's `update_time` must equal this exactly.
    UpdateTime(Timestamp),
}

/// One change to one document.
#[derive(Debug, Clone)]
pub enum Mutation {
    /// Replace the document wholesale — the `set()` path. Fields absent from
    /// `document` are dropped.
    Put(Document),
    /// Replace only the named fields, creating the document if absent — the
    /// `update()` / `set(…, {merge: true})` path.
    Merge {
        name: String,
        fields: Vec<(String, Value)>,
    },
    /// Remove the document. Idempotent: deleting a missing document succeeds.
    Delete { name: String },
    /// Change nothing; only enforce the precondition.
    ///
    /// This is how a transaction's **read set** is carried into its commit: one
    /// `Verify` per document the transaction read, guarded by the
    /// `update_time` that was read. Because the whole batch is one store
    /// transaction, a document that someone else wrote in the meantime fails
    /// the batch instead of being silently overwritten.
    Verify { name: String },
}

impl Mutation {
    /// Resource name of the document this mutation touches.
    pub fn name(&self) -> &str {
        match self {
            Mutation::Put(document) => &document.name,
            Mutation::Merge { name, .. }
            | Mutation::Delete { name }
            | Mutation::Verify { name } => name,
        }
    }
}

/// A mutation plus the precondition guarding it, as submitted to
/// [`Store::commit`].
#[derive(Debug, Clone)]
pub struct PendingWrite {
    pub mutation: Mutation,
    pub precondition: Option<Precondition>,
    /// Server-computed field transforms, applied **after** `mutation` to the
    /// document it left behind, in order.
    ///
    /// They live next to the mutation rather than inside it because that is the
    /// shape of the contract — a `Write` carries an operation *and* its
    /// `update_transforms` — and because a transform has to read the stored
    /// value, which only the store can do inside the write's own transaction.
    pub transforms: Vec<FieldTransform>,
}

impl PendingWrite {
    /// A write with no precondition and no transforms.
    pub fn new(mutation: Mutation) -> Self {
        Self::guarded(mutation, None)
    }

    /// A write guarded by `precondition`, with no transforms.
    pub fn guarded(mutation: Mutation, precondition: Option<Precondition>) -> Self {
        Self {
            mutation,
            precondition,
            transforms: Vec::new(),
        }
    }

    /// A write guarded by `precondition` that applies `transforms` afterwards.
    pub fn transformed(
        mutation: Mutation,
        precondition: Option<Precondition>,
        transforms: Vec<FieldTransform>,
    ) -> Self {
        Self {
            mutation,
            precondition,
            transforms,
        }
    }
}

/// Encode a document's fields for storage, refusing one that carries a value
/// the mapping cannot represent.
///
/// Both backends go through here, for different reasons that have to produce
/// the same answer. `PostgresStore` needs the payload. `MemoryStore` throws it
/// away — it keeps `Value`s exactly as they arrived and so has no encode step
/// of its own to fail, which without this would let it accept a document the
/// durable store refuses. A divergence there is the worst kind: it appears
/// only when somebody adds `--postgres-url` to a project that already works.
///
/// Validating by encoding, rather than with a cheaper check, is deliberate: a
/// separate validator would be a second definition of "storable" to keep in
/// step with [`value_json`](crate::local::value_json).
pub(crate) fn encode_storable_fields(document: &Document) -> StoreResult<serde_json::Value> {
    value_json::encode_fields(&document.fields).map_err(|error| {
        StoreError::InvalidDocument(format!(
            "document {} cannot be stored: {error}",
            document.name
        ))
    })
}

/// Check `precondition` against the document currently stored under `name`.
///
/// Shared by both backends so they cannot drift on, for example, whether a
/// document with no `update_time` satisfies an `update_time` precondition.
pub(crate) fn check_precondition(
    name: &str,
    stored: Option<&Document>,
    precondition: Option<&Precondition>,
) -> Result<(), PreconditionFailure> {
    match precondition {
        None => Ok(()),
        Some(Precondition::Exists(true)) if stored.is_none() => {
            Err(PreconditionFailure::Missing(name.to_string()))
        }
        Some(Precondition::Exists(false)) if stored.is_some() => {
            Err(PreconditionFailure::AlreadyExists(name.to_string()))
        }
        Some(Precondition::Exists(_)) => Ok(()),
        Some(Precondition::UpdateTime(expected)) => {
            let actual = stored.and_then(|document| document.update_time.as_ref());
            if actual == Some(expected) {
                Ok(())
            } else {
                Err(PreconditionFailure::UpdateTime(name.to_string()))
            }
        }
    }
}

/// Apply one pending write to `stored`, returning the document to write and
/// what the write did, or `None` for a delete.
///
/// This is the one definition of what `put`, `merge`, and a field transform
/// mean — `create_time` preservation, which fields survive a merge, and the
/// fact that transforms read the document the mutation *just* produced — so a
/// backend that reimplements the storage can still not reimplement the
/// semantics.
///
/// The precondition is checked by the caller (inside its own store
/// transaction), not here; see [`check_write`].
pub(crate) fn mutate(
    write: PendingWrite,
    stored: Option<Document>,
    now: &Timestamp,
) -> (Option<Document>, WriteEffect) {
    let PendingWrite {
        mutation,
        transforms,
        ..
    } = write;

    let existed = stored.is_some();
    let outcome = if existed {
        WriteOutcome::Updated
    } else {
        WriteOutcome::Created
    };
    if let Mutation::Delete { .. } = mutation {
        return (None, WriteEffect::plain(WriteOutcome::Deleted { existed }));
    }
    if let Mutation::Verify { .. } = mutation {
        return (stored, WriteEffect::plain(WriteOutcome::Verified));
    }
    // `create_time` only advances when a document is deleted and recreated, so
    // an overwrite keeps the time the document first appeared.
    let create_time = stored
        .as_ref()
        .and_then(|document| document.create_time.clone())
        .unwrap_or_else(|| now.clone());

    let mut document = match mutation {
        Mutation::Put(mut document) => {
            document.create_time = Some(create_time);
            document
        }
        Mutation::Merge { name, fields } => {
            let mut document = stored.unwrap_or_else(|| Document {
                name,
                fields: HashMap::new(),
                create_time: Some(create_time),
                update_time: None,
            });
            for (field, value) in fields {
                document.fields.insert(field, value);
            }
            document
        }
        // Both returned above, before `stored` was consumed.
        Mutation::Delete { .. } | Mutation::Verify { .. } => unreachable!(),
    };

    // After the mutation, per the contract: a write carrying both an update and
    // transforms is "equivalent to performing `update` and `transform` to the
    // same document atomically and in order".
    let transform_results = transforms::apply(&transforms, &mut document, now);
    document.update_time = Some(now.clone());

    (
        Some(document),
        WriteEffect {
            outcome,
            transform_results,
        },
    )
}

/// Check the precondition guarding one pending write.
///
/// Wraps [`check_precondition`] with the one rule that depends on *why* the
/// precondition is there: a failure on a read-set [`Mutation::Verify`] is
/// contention, not a client mistake, and has to reach the client as ABORTED.
pub(crate) fn check_write(
    write: &PendingWrite,
    stored: Option<&Document>,
) -> Result<(), PreconditionFailure> {
    let name = write.mutation.name();
    check_precondition(name, stored, write.precondition.as_ref()).map_err(|failure| {
        match &write.mutation {
            Mutation::Verify { .. } => PreconditionFailure::Conflict(name.to_string()),
            _ => failure,
        }
    })
}

/// Outcome of a write, so callers can report the right gRPC status without
/// re-reading the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Created,
    Updated,
    /// A delete, and whether it actually removed anything. Firestore deletes
    /// are idempotent, so "nothing to delete" is a successful outcome and not
    /// an error.
    Deleted {
        existed: bool,
    },
    /// A [`Mutation::Verify`] whose precondition held. Nothing was written.
    Verified,
}

/// What one applied write did: its outcome, plus one value per field transform.
///
/// The transform results travel with the outcome because they are only knowable
/// inside the write — `increment` reports the *new* number, which depends on
/// the stored one. Reading them back after the commit would report whatever a
/// later writer left behind instead.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteEffect {
    pub outcome: WriteOutcome,
    /// `WriteResult.transform_results`: one value per transform on the write,
    /// in order. Empty for a write with no transforms.
    pub transform_results: Vec<Value>,
}

impl WriteEffect {
    /// The effect of a write with no transforms.
    pub fn plain(outcome: WriteOutcome) -> Self {
        Self {
            outcome,
            transform_results: Vec::new(),
        }
    }
}

/// A shared handle to whichever store the process was configured with.
pub type SharedStore = Arc<dyn Store>;

/// Document storage for the local Firestore data plane.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Short backend name, for logs and the discovery endpoint.
    fn backend(&self) -> &'static str;

    /// Read a document by full resource name.
    async fn get(&self, name: &str) -> StoreResult<Option<Document>>;

    /// Whether a document exists. Separate from `get` because the Postgres
    /// implementation can answer it without fetching and decoding the payload.
    async fn contains(&self, name: &str) -> StoreResult<bool>;

    /// Apply `writes` as one atomic batch, in order.
    ///
    /// Either every write applies or none does: a precondition that fails on
    /// the last write leaves the store exactly as it was. Preconditions are
    /// checked inside the same transaction as the writes, so a concurrent
    /// writer cannot slip in between the check and the write.
    ///
    /// Writes are applied in order and a later write sees the effect of an
    /// earlier one in the same batch, which is what makes a transaction that
    /// reads its own writes behave sensibly.
    ///
    /// Field transforms on a write are computed here, against the document the
    /// write's own mutation just produced and inside the same transaction, so
    /// two concurrent `increment(1)` calls cannot both read the same starting
    /// value. `now` is the single timestamp every `serverTimestamp()` in the
    /// batch resolves to.
    async fn commit(
        &self,
        writes: Vec<PendingWrite>,
        now: Timestamp,
    ) -> StoreResult<Vec<WriteEffect>>;

    /// Every document directly inside `parent`/`collection_id`, ordered by
    /// document id in UTF-8 byte order (the order Firestore sorts `__name__`
    /// in).
    ///
    /// "Directly inside" excludes subcollections: a document at
    /// `…/users/alice/posts/p1` is not in `users`.
    async fn scan_collection(
        &self,
        parent: &str,
        collection_id: &str,
    ) -> StoreResult<Vec<Document>>;

    /// Like [`Store::scan_collection`], but allowed to leave out documents the
    /// query cannot match.
    ///
    /// The contract is deliberately one-sided: the result must contain **every**
    /// document [`scan_collection`](Store::scan_collection) would return that
    /// `prefilter` can match, and may contain any number of documents that it
    /// cannot. A backend that drops a matching row produces a wrong query
    /// result; a backend that keeps a non-matching row only costs one in-process
    /// comparison, which the caller was going to do anyway.
    ///
    /// The default implementation returns the whole collection. That is always
    /// correct, so a backend opts into pushdown rather than having to implement
    /// it — and `MemoryStore` deliberately does not, which keeps the in-process
    /// engine the thing CI tests.
    async fn scan_collection_filtered(
        &self,
        parent: &str,
        collection_id: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        let _ = prefilter;
        self.scan_collection(parent, collection_id).await
    }

    /// Every document whose collection id is `kind` anywhere under
    /// `documents_root`, ordered by resource name.
    ///
    /// Datastore kind queries return entities of that kind regardless of
    /// ancestry; Firestore collection reads do not. This is the store shape
    /// that makes the two APIs share one table without lying about either.
    async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>>;

    /// Like [`Store::scan_kind`], under the same one-sided contract as
    /// [`Store::scan_collection_filtered`]: the result may omit documents
    /// `prefilter` cannot match, and must omit none that it can.
    ///
    /// Worth having separately from the collection form because this is the
    /// wider read — a Firestore collection group query, or a Datastore kind
    /// query, spans every matching collection under the root rather than one.
    async fn scan_kind_filtered(
        &self,
        documents_root: &str,
        kind: &str,
        prefilter: Prefilter<'_>,
    ) -> StoreResult<Vec<Document>> {
        let _ = prefilter;
        self.scan_kind(documents_root, kind).await
    }

    /// The distinct first path segments of every document name beneath
    /// `prefix`, sorted in UTF-8 byte order.
    ///
    /// Two RPCs are built from this, because Firestore paths alternate
    /// collection and document:
    ///
    /// - with a document (or the database root) as `prefix`, the result is the
    ///   **collection ids** under it, which is `ListCollectionIds`;
    /// - with a collection as `prefix`, the result is the **document ids** in
    ///   it, which is what `ListDocuments(show_missing: true)` needs.
    ///
    /// A segment counts even when the document between it and `prefix` was
    /// never written: Firestore has no empty collections and no required
    /// parent documents, so `…/users/alice/posts/p1` puts `users` under the
    /// database root whether or not `users/alice` itself exists.
    async fn child_segments(&self, prefix: &str) -> StoreResult<Vec<String>>;

    /// Total number of stored documents. Used by the seed command's summary and
    /// by tests; not on any request path.
    async fn count(&self) -> StoreResult<usize>;

    /// Insert or replace a document wholesale.
    ///
    /// `create_time` is preserved across overwrites of an existing document,
    /// matching the documented Firestore behaviour that `create_time` only
    /// advances when a document is deleted and recreated.
    async fn put(&self, document: Document, now: Timestamp) -> StoreResult<WriteOutcome> {
        single(
            self.commit(vec![PendingWrite::new(Mutation::Put(document))], now)
                .await?,
        )
    }

    /// Merge `fields` into an existing document, creating it if absent. Only
    /// the supplied field names are touched.
    async fn merge(
        &self,
        name: &str,
        fields: Vec<(String, Value)>,
        now: Timestamp,
    ) -> StoreResult<WriteOutcome> {
        let mutation = Mutation::Merge {
            name: name.to_string(),
            fields,
        };
        single(self.commit(vec![PendingWrite::new(mutation)], now).await?)
    }

    /// Remove a document. Returns whether it existed; Firestore deletes are
    /// idempotent, so a missing document is not an error.
    async fn delete(&self, name: &str) -> StoreResult<bool> {
        let mutation = Mutation::Delete {
            name: name.to_string(),
        };
        // `now` is unused by a delete, but the batch API takes one; passing the
        // real clock keeps it honest if that ever changes.
        let outcome = single(
            self.commit(vec![PendingWrite::new(mutation)], now_timestamp())
                .await?,
        )?;
        match outcome {
            WriteOutcome::Deleted { existed } => Ok(existed),
            other => Err(StoreError::Backend(anyhow::anyhow!(
                "a delete reported {other:?} instead of a delete outcome"
            ))),
        }
    }
}

/// Current wall-clock time as a protobuf `Timestamp`.
pub fn now_timestamp() -> Timestamp {
    let now = chrono::Utc::now();
    Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

/// Unwrap the single outcome of a one-write batch.
///
/// The provided `put`/`merge`/`delete` helpers never carry transforms, so the
/// effect's transform results are empty and only the outcome is of interest.
fn single(effects: Vec<WriteEffect>) -> StoreResult<WriteOutcome> {
    effects
        .into_iter()
        .next()
        .map(|effect| effect.outcome)
        .ok_or_else(|| {
            StoreError::Backend(anyhow::anyhow!(
                "a one-write batch returned no write outcome"
            ))
        })
}

/// In-memory store: a single `BTreeMap` keyed by full document resource name.
///
/// `BTreeMap` rather than a hash map because every query shape Firestore needs
/// later (list a collection, scan a subtree) is a key-prefix range scan, and the
/// ordering makes those cheap when we get there.
///
/// Nothing is persisted. This is the default because it needs no setup; pass
/// `--postgres-url` for a store that survives a restart.
#[derive(Clone, Default)]
pub struct MemoryStore {
    documents: Arc<RwLock<BTreeMap<String, Document>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    fn backend(&self) -> &'static str {
        "memory"
    }

    async fn get(&self, name: &str) -> StoreResult<Option<Document>> {
        Ok(self
            .documents
            .read()
            .expect("document store lock poisoned")
            .get(name)
            .cloned())
    }

    async fn contains(&self, name: &str) -> StoreResult<bool> {
        Ok(self
            .documents
            .read()
            .expect("document store lock poisoned")
            .contains_key(name))
    }

    async fn commit(
        &self,
        writes: Vec<PendingWrite>,
        now: Timestamp,
    ) -> StoreResult<Vec<WriteEffect>> {
        // One lock for the whole batch is what makes it atomic against other
        // writers — including against another batch's transforms reading the
        // same document. Nothing in this loop awaits, so the guard never
        // crosses a suspension point.
        let mut documents = self
            .documents
            .write()
            .expect("document store lock poisoned");

        // Staged rather than applied as we go: a precondition that fails on the
        // last write has to leave the store exactly as it was.
        let mut staged: HashMap<String, Option<Document>> = HashMap::new();
        let mut outcomes = Vec::with_capacity(writes.len());

        for write in writes {
            let name = write.mutation.name().to_string();
            // A later write in the batch sees the effect of an earlier one.
            let stored = match staged.get(&name) {
                Some(pending) => pending.clone(),
                None => documents.get(&name).cloned(),
            };

            check_write(&write, stored.as_ref())?;
            if matches!(write.mutation, Mutation::Verify { .. }) {
                // Nothing to stage: a verification changes no document.
                outcomes.push(WriteEffect::plain(WriteOutcome::Verified));
                continue;
            }

            let (next, effect) = mutate(write, stored, &now);
            // Checked here rather than before the mutation because a merge
            // produces a document neither side held, and because this is
            // where `PostgresStore` encodes too.
            if let Some(document) = next.as_ref() {
                encode_storable_fields(document)?;
            }
            staged.insert(name, next);
            outcomes.push(effect);
        }

        for (name, state) in staged {
            match state {
                Some(document) => documents.insert(name, document),
                None => documents.remove(&name),
            };
        }
        Ok(outcomes)
    }

    async fn scan_collection(
        &self,
        parent: &str,
        collection_id: &str,
    ) -> StoreResult<Vec<Document>> {
        let prefix = format!("{parent}/{collection_id}/");
        let documents = self.documents.read().expect("document store lock poisoned");

        // A `BTreeMap` range is why this store uses one: a collection read is a
        // key-prefix scan, already in the order Firestore wants.
        Ok(documents
            .range(prefix.clone()..)
            .take_while(|(name, _)| name.starts_with(&prefix))
            // Subcollection documents share the prefix but have more segments.
            .filter(|(name, _)| !name[prefix.len()..].contains('/'))
            .map(|(_, document)| document.clone())
            .collect())
    }

    async fn scan_kind(&self, documents_root: &str, kind: &str) -> StoreResult<Vec<Document>> {
        let prefix = format!("{documents_root}/");
        let documents = self.documents.read().expect("document store lock poisoned");

        let mut found: Vec<Document> = documents
            .range(prefix.clone()..)
            .take_while(|(name, _)| name.starts_with(&prefix))
            .filter_map(|(name, document)| {
                // Collection id is the second-to-last path segment after the
                // documents root — the same rule `parse_document_name` uses.
                let rest = name.strip_prefix(&prefix)?;
                let segments: Vec<&str> = rest.split('/').collect();
                if segments.len() >= 2
                    && segments.len() % 2 == 0
                    && segments[segments.len() - 2] == kind
                {
                    Some(document.clone())
                } else {
                    None
                }
            })
            .collect();
        found.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
        Ok(found)
    }

    async fn child_segments(&self, parent: &str) -> StoreResult<Vec<String>> {
        let prefix = format!("{parent}/");
        let documents = self.documents.read().expect("document store lock poisoned");

        // `BTreeSet` to dedupe and sort in one pass.
        Ok(documents
            .range(prefix.clone()..)
            .take_while(|(name, _)| name.starts_with(&prefix))
            .filter_map(|(name, _)| name[prefix.len()..].split('/').next())
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect())
    }

    async fn count(&self) -> StoreResult<usize> {
        Ok(self
            .documents
            .read()
            .expect("document store lock poisoned")
            .len())
    }
}

/// Conformance tests every `Store` implementation must pass.
///
/// Public (behind `cfg(test)` for unit tests, and compiled into the library for
/// integration tests) so the Postgres integration test can run exactly the same
/// assertions as the in-memory unit test. A durable backend that disagrees with
/// the in-memory one about `create_time` or merge semantics is a bug that only
/// shows up as weird SDK behaviour much later.
pub mod conformance {
    use super::*;
    use crate::protos::google::firestore::v1::value::ValueType;

    pub fn ts(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    pub fn string_value(text: &str) -> Value {
        Value {
            value_type: Some(ValueType::StringValue(text.to_string())),
        }
    }

    pub fn integer_value(number: i64) -> Value {
        Value {
            value_type: Some(ValueType::IntegerValue(number)),
        }
    }

    /// Just the outcomes of a batch, for the assertions that do not care what
    /// any transform on it resolved to.
    fn outcomes(effects: Vec<WriteEffect>) -> Vec<WriteOutcome> {
        effects.into_iter().map(|effect| effect.outcome).collect()
    }

    pub fn doc(name: &str, field: &str, text: &str) -> Document {
        Document {
            name: name.to_string(),
            fields: [(field.to_string(), string_value(text))]
                .into_iter()
                .collect(),
            create_time: None,
            update_time: None,
        }
    }

    /// Run every contract assertion against `store`.
    ///
    /// `root` scopes the document names so concurrent runs against a shared
    /// database do not collide.
    pub async fn assert_store_contract(store: &dyn Store, root: &str) {
        let name = |suffix: &str| format!("{root}/contract/{suffix}");

        put_then_get_round_trips(store, &name("round-trip")).await;
        overwrite_keeps_create_time_and_advances_update_time(store, &name("create-time")).await;
        overwrite_drops_fields_absent_from_the_new_document(store, &name("replace")).await;
        merge_only_touches_named_fields(store, &name("merge")).await;
        merge_creates_a_missing_document(store, &name("merge-create")).await;
        delete_reports_whether_the_document_existed(store, &name("delete")).await;
        missing_documents_read_as_none(store, &name("absent")).await;
        writes_and_deletes_change_what_a_read_returns(store, &name("count")).await;
        preconditions_are_enforced_by_the_store(store, &name("precondition")).await;
        a_failed_precondition_abandons_the_whole_batch(store, &name("atomic")).await;
        a_batch_reads_its_own_earlier_writes(store, &name("ordered")).await;
        a_verification_changes_nothing_but_can_fail_the_batch(store, &name("verify")).await;
        transforms_are_computed_inside_the_write(store, &name("transform")).await;
        transforms_apply_after_the_mutation_they_ride_on(store, &name("transform-order")).await;
        collection_reads_return_only_direct_children(store, &name("scan")).await;
        collection_ids_include_collections_under_an_unwritten_parent(store, &name("ids")).await;
        a_timestamp_no_document_can_hold_fails_the_write(store, &name("timestamp-range")).await;
    }

    /// A timestamp outside years 0001–9999 fails the **write**, in both
    /// backends.
    ///
    /// It is in the shared contract precisely because the two reach the answer
    /// differently: `PostgresStore` encodes to JSON and `MemoryStore` keeps the
    /// `Value` as it arrived, so this is a rule they could easily drift on —
    /// and a value one accepts and the other refuses only surfaces when
    /// somebody adds `--postgres-url` to a project that already works.
    ///
    /// Before SAK-205 the write succeeded and then broke every later read of
    /// the entire collection, because the encoding it produced was one the
    /// read path refused.
    async fn a_timestamp_no_document_can_hold_fails_the_write(store: &dyn Store, name: &str) {
        let document = Document {
            name: name.to_string(),
            fields: [(
                "joined".to_string(),
                Value {
                    value_type: Some(ValueType::TimestampValue(Timestamp {
                        // Year 11476.
                        seconds: 300_000_000_000,
                        nanos: 0,
                    })),
                },
            )]
            .into_iter()
            .collect(),
            create_time: None,
            update_time: None,
        };

        let error = store
            .put(document, ts(10))
            .await
            .expect_err("a timestamp past year 9999 must not be stored");
        assert!(
            matches!(error, StoreError::InvalidDocument(_)),
            "wrong error for an out-of-range timestamp: {error}"
        );
        assert_eq!(
            Status::from(error).code(),
            tonic::Code::InvalidArgument,
            "an SDK has to see this as its own bad argument, not a server fault"
        );

        // Refused means refused: nothing was left behind to read back.
        assert!(
            store.get(name).await.expect("get").is_none(),
            "a refused write must not store a partial document"
        );
    }

    async fn put_then_get_round_trips(store: &dyn Store, name: &str) {
        assert_eq!(
            store.put(doc(name, "k", "v"), ts(10)).await.expect("put"),
            WriteOutcome::Created
        );

        let found = store
            .get(name)
            .await
            .expect("get")
            .expect("document stored");
        assert_eq!(found.name, name);
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(10)));
        assert_eq!(found.fields["k"], string_value("v"));
        assert!(store.contains(name).await.expect("contains"));
    }

    async fn overwrite_keeps_create_time_and_advances_update_time(store: &dyn Store, name: &str) {
        store.put(doc(name, "k", "v1"), ts(10)).await.expect("put");
        assert_eq!(
            store
                .put(doc(name, "k", "v2"), ts(20))
                .await
                .expect("overwrite"),
            WriteOutcome::Updated
        );

        let found = store.get(name).await.expect("get").expect("still present");
        assert_eq!(found.create_time, Some(ts(10)), "create_time must not move");
        assert_eq!(found.update_time, Some(ts(20)));
        assert_eq!(found.fields["k"], string_value("v2"));
    }

    async fn overwrite_drops_fields_absent_from_the_new_document(store: &dyn Store, name: &str) {
        store
            .put(doc(name, "keep", "v"), ts(10))
            .await
            .expect("put");
        store
            .put(doc(name, "other", "v"), ts(20))
            .await
            .expect("overwrite");

        let found = store.get(name).await.expect("get").expect("still present");
        assert!(
            !found.fields.contains_key("keep"),
            "put replaces all fields"
        );
        assert!(found.fields.contains_key("other"));
    }

    async fn merge_only_touches_named_fields(store: &dyn Store, name: &str) {
        store
            .put(doc(name, "keep", "v"), ts(10))
            .await
            .expect("put");
        assert_eq!(
            store
                .merge(name, vec![("added".to_string(), string_value("x"))], ts(20))
                .await
                .expect("merge"),
            WriteOutcome::Updated
        );

        let found = store.get(name).await.expect("get").expect("still present");
        assert_eq!(found.fields["keep"], string_value("v"));
        assert_eq!(found.fields["added"], string_value("x"));
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(20)));
    }

    async fn merge_creates_a_missing_document(store: &dyn Store, name: &str) {
        let outcome = store
            .merge(name, vec![("k".to_string(), string_value("v"))], ts(10))
            .await
            .expect("merge");

        assert_eq!(outcome, WriteOutcome::Created);
        let found = store.get(name).await.expect("get").expect("created");
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(10)));
        assert_eq!(found.fields["k"], string_value("v"));
    }

    async fn delete_reports_whether_the_document_existed(store: &dyn Store, name: &str) {
        store.put(doc(name, "k", "v"), ts(10)).await.expect("put");

        assert!(store.delete(name).await.expect("delete"));
        assert!(
            !store.delete(name).await.expect("second delete"),
            "deletes are idempotent"
        );
        assert!(!store.contains(name).await.expect("contains"));
        assert!(store.get(name).await.expect("get").is_none());
    }

    async fn missing_documents_read_as_none(store: &dyn Store, name: &str) {
        assert!(store.get(name).await.expect("get").is_none());
        assert!(!store.contains(name).await.expect("contains"));
    }

    /// A write becomes visible to a collection read, and a delete stops being
    /// visible.
    ///
    /// This is the race-free half of what used to be a [`Store::count`]
    /// assertion. `count` is **store-wide**, so an absolute total cannot be
    /// asserted from here: the PostgreSQL backend shares one table across every
    /// test in its binary, and a sibling test writing in between made
    /// `before + 1` fail for a reason that has nothing to do with counting
    /// (~1 run in 8 once a second document-writing test joined that binary,
    /// SAK-192). Asserting the *direction* of the change is no better — a
    /// concurrent insert masks the delete, giving `11 -> 11`.
    ///
    /// Scoping the read to this test's own collection makes it exact again.
    /// The store-wide behaviour of `count` itself is asserted in
    /// [`assert_exclusive_count_contract`], from a test that owns its store.
    async fn writes_and_deletes_change_what_a_read_returns(store: &dyn Store, name: &str) {
        let present = |documents: &[Document]| documents.iter().any(|d| d.name == name);
        // `name` is `…/contract/count`, so this reads the collection holding it.
        let (parent, collection) = name
            .rsplit_once('/')
            .map(|(parent, _)| parent.rsplit_once('/').expect("a rooted name"))
            .expect("a rooted name");

        store.put(doc(name, "k", "v"), ts(10)).await.expect("put");
        let after_put = store
            .scan_collection(parent, collection)
            .await
            .expect("scan");
        assert!(present(&after_put), "a stored document has to be readable");
        assert!(store.contains(name).await.expect("contains"));

        store.delete(name).await.expect("delete");
        let after_delete = store
            .scan_collection(parent, collection)
            .await
            .expect("scan");
        assert!(
            !present(&after_delete),
            "a deleted document must not come back from a read"
        );
        assert!(!store.contains(name).await.expect("contains"));
    }

    /// That [`Store::count`] tracks writes and deletes exactly.
    ///
    /// **Requires exclusive use of `store`**, because `count` is store-wide.
    /// Kept out of [`assert_store_contract`] for that reason: the PostgreSQL
    /// conformance test shares its table with every other test in its binary,
    /// so an exact total there is a coin flip rather than a check. Call this
    /// only from a test that owns the whole store.
    pub async fn assert_exclusive_count_contract(store: &dyn Store, root: &str) {
        let name = format!("{root}/exclusive-count/doc");
        let before = store.count().await.expect("count");

        store.put(doc(&name, "k", "v"), ts(10)).await.expect("put");
        assert_eq!(
            store.count().await.expect("count"),
            before + 1,
            "a stored document has to be counted"
        );

        store.delete(&name).await.expect("delete");
        assert_eq!(
            store.count().await.expect("count"),
            before,
            "a deleted document has to stop being counted"
        );
    }

    /// All three precondition kinds, with the exact failure each must report —
    /// the data plane turns these into NOT_FOUND / ALREADY_EXISTS /
    /// FAILED_PRECONDITION, and an SDK branches on the difference.
    async fn preconditions_are_enforced_by_the_store(store: &dyn Store, name: &str) {
        let put = |exists: bool| {
            PendingWrite::guarded(
                Mutation::Put(doc(name, "k", "v")),
                Some(Precondition::Exists(exists)),
            )
        };

        let failure = store
            .commit(vec![put(true)], ts(10))
            .await
            .expect_err("exists=true on a missing document");
        assert!(
            matches!(
                failure,
                StoreError::Precondition(PreconditionFailure::Missing(_))
            ),
            "{failure}"
        );

        store
            .commit(vec![put(false)], ts(10))
            .await
            .expect("exists=false creates");
        let failure = store
            .commit(vec![put(false)], ts(20))
            .await
            .expect_err("exists=false on an existing document");
        assert!(
            matches!(
                failure,
                StoreError::Precondition(PreconditionFailure::AlreadyExists(_))
            ),
            "{failure}"
        );

        // The real update_time is ts(10) from the create above.
        let guarded = |expected: Timestamp| {
            PendingWrite::guarded(
                Mutation::Put(doc(name, "k", "v2")),
                Some(Precondition::UpdateTime(expected)),
            )
        };
        let failure = store
            .commit(vec![guarded(ts(999))], ts(30))
            .await
            .expect_err("stale update_time");
        assert!(
            matches!(
                failure,
                StoreError::Precondition(PreconditionFailure::UpdateTime(_))
            ),
            "{failure}"
        );
        store
            .commit(vec![guarded(ts(10))], ts(30))
            .await
            .expect("matching update_time");
    }

    /// The property that makes `Commit` atomic: a precondition that fails on the
    /// last write must leave the earlier writes unapplied.
    async fn a_failed_precondition_abandons_the_whole_batch(store: &dyn Store, name: &str) {
        let first = format!("{name}-first");
        let doomed = format!("{name}-doomed");

        let failure = store
            .commit(
                vec![
                    PendingWrite::new(Mutation::Put(doc(&first, "k", "v"))),
                    PendingWrite::guarded(
                        Mutation::Put(doc(&doomed, "k", "v")),
                        // Nothing was ever written here, so this cannot hold.
                        Some(Precondition::Exists(true)),
                    ),
                ],
                ts(10),
            )
            .await
            .expect_err("the batch must fail");

        assert!(
            matches!(failure, StoreError::Precondition(_)),
            "the batch should fail on the precondition, not something else: {failure}"
        );
        assert!(
            store.get(&first).await.expect("get").is_none(),
            "a failed batch must not leave its earlier writes behind"
        );
        assert!(store.get(&doomed).await.expect("get").is_none());
    }

    /// Writes apply in order, and a later write sees the earlier one — what
    /// makes `create` followed by `update` in one transaction behave sensibly.
    async fn a_batch_reads_its_own_earlier_writes(store: &dyn Store, name: &str) {
        let effects = store
            .commit(
                vec![
                    PendingWrite::guarded(
                        Mutation::Put(doc(name, "first", "v")),
                        Some(Precondition::Exists(false)),
                    ),
                    PendingWrite::guarded(
                        Mutation::Merge {
                            name: name.to_string(),
                            fields: vec![("second".to_string(), string_value("v"))],
                        },
                        // Only holds if the put above is already visible.
                        Some(Precondition::Exists(true)),
                    ),
                ],
                ts(10),
            )
            .await
            .expect("ordered batch");

        assert_eq!(
            outcomes(effects),
            vec![WriteOutcome::Created, WriteOutcome::Updated]
        );
        let found = store.get(name).await.expect("get").expect("written");
        assert!(found.fields.contains_key("first"));
        assert!(found.fields.contains_key("second"));
    }

    /// A read-set verification writes nothing, and fails the batch — as a
    /// conflict, not as a client error — when the document moved on.
    async fn a_verification_changes_nothing_but_can_fail_the_batch(store: &dyn Store, name: &str) {
        let read = format!("{name}-read");
        let written = format!("{name}-written");
        store.put(doc(&read, "k", "v"), ts(10)).await.expect("put");

        let batch = |expected: Timestamp, target: &str| {
            vec![
                PendingWrite::guarded(
                    Mutation::Verify { name: read.clone() },
                    Some(Precondition::UpdateTime(expected)),
                ),
                PendingWrite::new(Mutation::Put(doc(target, "k", "v"))),
            ]
        };

        let effects = store
            .commit(batch(ts(10), &written), ts(20))
            .await
            .expect("the verification holds");
        assert_eq!(
            outcomes(effects),
            vec![WriteOutcome::Verified, WriteOutcome::Created]
        );

        let unchanged = store.get(&read).await.expect("get").expect("still there");
        assert_eq!(
            unchanged.update_time,
            Some(ts(10)),
            "a verification must not touch the document it checks"
        );

        let doomed = format!("{name}-doomed");
        let failure = store
            .commit(batch(ts(999), &doomed), ts(30))
            .await
            .expect_err("the document moved on");
        assert!(
            matches!(
                failure,
                StoreError::Precondition(PreconditionFailure::Conflict(_))
            ),
            "a stale read set is contention, not a bad request: {failure}"
        );
        assert!(
            store.get(&doomed).await.expect("get").is_none(),
            "the batch's writes must not survive a failed verification"
        );
    }

    /// A field transform reads the stored value and reports the value it wrote.
    ///
    /// The point of the assertion is *where* the read happens: the second
    /// commit's `increment` has to see the first commit's result, so a backend
    /// that computed transforms before opening its transaction would fail here
    /// as soon as anything else wrote in between.
    async fn transforms_are_computed_inside_the_write(store: &dyn Store, name: &str) {
        let transform = |kind| {
            vec![PendingWrite::transformed(
                Mutation::Merge {
                    name: name.to_string(),
                    fields: Vec::new(),
                },
                None,
                vec![FieldTransform {
                    field: "hits".to_string(),
                    kind,
                }],
            )]
        };

        let effects = store
            .commit(
                transform(transforms::TransformKind::Increment(
                    transforms::Number::Integer(1),
                )),
                ts(10),
            )
            .await
            .expect("increment a missing field");
        assert_eq!(
            effects,
            vec![WriteEffect {
                outcome: WriteOutcome::Created,
                // A missing field is set to the operand, and the result reports
                // the number that was written.
                transform_results: vec![integer_value(1)],
            }]
        );

        let effects = store
            .commit(
                transform(transforms::TransformKind::Increment(
                    transforms::Number::Integer(41),
                )),
                ts(20),
            )
            .await
            .expect("increment the stored value");
        assert_eq!(
            effects,
            vec![WriteEffect {
                outcome: WriteOutcome::Updated,
                transform_results: vec![integer_value(42)],
            }]
        );

        let found = store.get(name).await.expect("get").expect("written");
        assert_eq!(found.fields["hits"], integer_value(42));
        assert_eq!(found.create_time, Some(ts(10)), "create_time must not move");
        assert_eq!(found.update_time, Some(ts(20)));
    }

    /// A write carrying both a mutation and transforms applies them in that
    /// order, which is what makes `set()` with a `serverTimestamp()` field
    /// behave: the replace lands first, then the transform fills its field in.
    async fn transforms_apply_after_the_mutation_they_ride_on(store: &dyn Store, name: &str) {
        store
            .put(doc(name, "hits", "stale"), ts(10))
            .await
            .expect("put");

        store
            .commit(
                vec![PendingWrite::transformed(
                    // A full replace, so the stale string is gone before the
                    // transform looks at the field.
                    Mutation::Put(doc(name, "kept", "v")),
                    None,
                    vec![FieldTransform {
                        field: "hits".to_string(),
                        kind: transforms::TransformKind::Increment(transforms::Number::Integer(7)),
                    }],
                )],
                ts(20),
            )
            .await
            .expect("put plus transform");

        let found = store.get(name).await.expect("get").expect("written");
        assert_eq!(found.fields["kept"], string_value("v"));
        assert_eq!(
            found.fields["hits"],
            integer_value(7),
            "the transform sees the document the put produced, not the one it replaced"
        );
    }

    /// A collection read is the documents *directly* in it: subcollection
    /// documents share the key prefix but belong to a different collection.
    async fn collection_reads_return_only_direct_children(store: &dyn Store, name: &str) {
        for suffix in ["items/b", "items/a", "items/a/sub/deep", "other/z"] {
            store
                .put(doc(&format!("{name}/{suffix}"), "k", "v"), ts(10))
                .await
                .expect("put");
        }

        let found = store.scan_collection(name, "items").await.expect("scan");
        let names: Vec<&str> = found
            .iter()
            .map(|document| document.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![format!("{name}/items/a"), format!("{name}/items/b")],
            "a collection read returns its direct children, sorted by document id"
        );
        assert_eq!(
            found[0].fields["k"],
            string_value("v"),
            "documents come back whole"
        );

        assert!(
            store
                .scan_collection(name, "nothing-here")
                .await
                .expect("scan")
                .is_empty(),
            "an unknown collection reads as empty, not an error"
        );
    }

    /// Firestore has no empty collections and no required parent documents, so
    /// a collection whose only path to `parent` runs through an unwritten
    /// document still has to be listed.
    async fn collection_ids_include_collections_under_an_unwritten_parent(
        store: &dyn Store,
        name: &str,
    ) {
        store
            .put(doc(&format!("{name}/direct/d1"), "k", "v"), ts(10))
            .await
            .expect("put");
        store
            .put(
                doc(&format!("{name}/ghostly/never-written/deep/d1"), "k", "v"),
                ts(10),
            )
            .await
            .expect("put");

        assert_eq!(
            store.child_segments(name).await.expect("collection ids"),
            vec!["direct".to_string(), "ghostly".to_string()]
        );
        assert_eq!(
            store
                .child_segments(&format!("{name}/ghostly"))
                .await
                .expect("document ids"),
            vec!["never-written".to_string()],
            "a collection prefix yields document ids, including documents that \
             exist only as the parent of something deeper"
        );
        assert!(
            store
                .child_segments(&format!("{name}/direct/d1"))
                .await
                .expect("collection ids")
                .is_empty(),
            "a document with no subcollections has no collection ids"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::conformance::*;
    use super::*;

    #[tokio::test]
    async fn memory_store_satisfies_the_store_contract() {
        let store = MemoryStore::new();
        assert_store_contract(&store, "projects/demo/databases/(default)/documents").await;
    }

    /// `count` is store-wide, so its exact contract is asserted here rather
    /// than in `assert_store_contract`: this store is this test's alone, while
    /// the PostgreSQL conformance test shares its table with its siblings.
    #[tokio::test]
    async fn an_exclusively_owned_store_counts_exactly() {
        let store = MemoryStore::new();
        assert_exclusive_count_contract(&store, "projects/demo/databases/(default)/documents")
            .await;
    }

    #[tokio::test]
    async fn clones_share_one_set_of_documents() {
        let store = MemoryStore::new();
        let clone = store.clone();
        store.put(doc("a", "k", "v"), ts(10)).await.expect("put");

        assert_eq!(clone.count().await.expect("count"), 1);
        assert!(clone.get("a").await.expect("get").is_some());
    }
}
