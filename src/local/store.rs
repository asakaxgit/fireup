//! The document store behind `fireup local`.
//!
//! Phase 1 splits the Phase 0 map into a `Store` trait with two
//! implementations: [`MemoryStore`] (the default, nothing to set up) and
//! [`PostgresStore`](crate::local::postgres_store::PostgresStore) (durable, so
//! data survives a restart). The data plane only ever sees the trait, which is
//! what keeps the two honest: the same conformance tests run against both.
//!
//! The trait is deliberately narrow — single-document reads and writes, plus a
//! count. Collection reads and queries land with the query engine in a later
//! phase, and adding speculative methods now would just freeze guesses about
//! the shape they need.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use prost_types::Timestamp;
use tonic::Status;

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
        }
    }
}

/// Outcome of a write, so callers can report the right gRPC status without
/// re-reading the store.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Created,
    Updated,
}

/// A shared handle to whichever store the process was configured with.
pub type SharedStore = Arc<dyn Store>;

/// Single-document storage for the local Firestore data plane.
#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    /// Short backend name, for logs and the discovery endpoint.
    fn backend(&self) -> &'static str;

    /// Read a document by full resource name.
    async fn get(&self, name: &str) -> StoreResult<Option<Document>>;

    /// Whether a document exists. Separate from `get` because the Postgres
    /// implementation can answer it without fetching and decoding the payload.
    async fn contains(&self, name: &str) -> StoreResult<bool>;

    /// Insert or replace a document wholesale.
    ///
    /// `create_time` is preserved across overwrites of an existing document,
    /// matching the documented Firestore behaviour that `create_time` only
    /// advances when a document is deleted and recreated.
    async fn put(&self, document: Document, now: Timestamp) -> StoreResult<WriteOutcome>;

    /// Merge `fields` into an existing document, creating it if absent. Only
    /// the supplied field names are touched.
    async fn merge(
        &self,
        name: &str,
        fields: Vec<(String, Value)>,
        now: Timestamp,
    ) -> StoreResult<WriteOutcome>;

    /// Remove a document. Returns whether it existed; Firestore deletes are
    /// idempotent, so a missing document is not an error.
    async fn delete(&self, name: &str) -> StoreResult<bool>;

    /// Total number of stored documents. Used by the seed command's summary and
    /// by tests; not on any request path.
    async fn count(&self) -> StoreResult<usize>;
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

    async fn put(&self, mut document: Document, now: Timestamp) -> StoreResult<WriteOutcome> {
        let mut documents = self
            .documents
            .write()
            .expect("document store lock poisoned");

        let existing_create_time = documents
            .get(&document.name)
            .and_then(|existing| existing.create_time.clone());

        document.create_time = Some(existing_create_time.clone().unwrap_or(now.clone()));
        document.update_time = Some(now);

        let outcome = if existing_create_time.is_some() {
            WriteOutcome::Updated
        } else {
            WriteOutcome::Created
        };
        documents.insert(document.name.clone(), document);
        Ok(outcome)
    }

    async fn merge(
        &self,
        name: &str,
        fields: Vec<(String, Value)>,
        now: Timestamp,
    ) -> StoreResult<WriteOutcome> {
        let mut documents = self
            .documents
            .write()
            .expect("document store lock poisoned");

        let entry = documents.entry(name.to_string()).or_insert_with(|| Document {
            name: name.to_string(),
            fields: Default::default(),
            create_time: Some(now.clone()),
            update_time: None,
        });
        let outcome = if entry.update_time.is_none() {
            WriteOutcome::Created
        } else {
            WriteOutcome::Updated
        };

        for (field, value) in fields {
            entry.fields.insert(field, value);
        }
        entry.update_time = Some(now);
        Ok(outcome)
    }

    async fn delete(&self, name: &str) -> StoreResult<bool> {
        Ok(self
            .documents
            .write()
            .expect("document store lock poisoned")
            .remove(name)
            .is_some())
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

    pub fn doc(name: &str, field: &str, text: &str) -> Document {
        Document {
            name: name.to_string(),
            fields: [(field.to_string(), string_value(text))].into_iter().collect(),
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
        counts_reflect_writes_and_deletes(store, &name("count")).await;
    }

    async fn put_then_get_round_trips(store: &dyn Store, name: &str) {
        assert_eq!(
            store.put(doc(name, "k", "v"), ts(10)).await.expect("put"),
            WriteOutcome::Created
        );

        let found = store.get(name).await.expect("get").expect("document stored");
        assert_eq!(found.name, name);
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(10)));
        assert_eq!(found.fields["k"], string_value("v"));
        assert!(store.contains(name).await.expect("contains"));
    }

    async fn overwrite_keeps_create_time_and_advances_update_time(store: &dyn Store, name: &str) {
        store.put(doc(name, "k", "v1"), ts(10)).await.expect("put");
        assert_eq!(
            store.put(doc(name, "k", "v2"), ts(20)).await.expect("overwrite"),
            WriteOutcome::Updated
        );

        let found = store.get(name).await.expect("get").expect("still present");
        assert_eq!(found.create_time, Some(ts(10)), "create_time must not move");
        assert_eq!(found.update_time, Some(ts(20)));
        assert_eq!(found.fields["k"], string_value("v2"));
    }

    async fn overwrite_drops_fields_absent_from_the_new_document(store: &dyn Store, name: &str) {
        store.put(doc(name, "keep", "v"), ts(10)).await.expect("put");
        store.put(doc(name, "other", "v"), ts(20)).await.expect("overwrite");

        let found = store.get(name).await.expect("get").expect("still present");
        assert!(!found.fields.contains_key("keep"), "put replaces all fields");
        assert!(found.fields.contains_key("other"));
    }

    async fn merge_only_touches_named_fields(store: &dyn Store, name: &str) {
        store.put(doc(name, "keep", "v"), ts(10)).await.expect("put");
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
        assert!(!store.delete(name).await.expect("second delete"), "deletes are idempotent");
        assert!(!store.contains(name).await.expect("contains"));
        assert!(store.get(name).await.expect("get").is_none());
    }

    async fn missing_documents_read_as_none(store: &dyn Store, name: &str) {
        assert!(store.get(name).await.expect("get").is_none());
        assert!(!store.contains(name).await.expect("contains"));
    }

    async fn counts_reflect_writes_and_deletes(store: &dyn Store, name: &str) {
        let before = store.count().await.expect("count");
        store.put(doc(name, "k", "v"), ts(10)).await.expect("put");
        assert_eq!(store.count().await.expect("count"), before + 1);
        store.delete(name).await.expect("delete");
        assert_eq!(store.count().await.expect("count"), before);
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

    #[tokio::test]
    async fn clones_share_one_set_of_documents() {
        let store = MemoryStore::new();
        let clone = store.clone();
        store.put(doc("a", "k", "v"), ts(10)).await.expect("put");

        assert_eq!(clone.count().await.expect("count"), 1);
        assert!(clone.get("a").await.expect("get").is_some());
    }
}
