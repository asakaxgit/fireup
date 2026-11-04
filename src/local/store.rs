//! In-memory document store backing `fireup local`.
//!
//! Phase 0 scope: a single `BTreeMap` keyed by full document resource name.
//! `BTreeMap` rather than a hash map because every query shape Firestore needs
//! later (list a collection, scan a subtree) is a key-prefix range scan, and the
//! ordering makes those cheap when we get there.
//!
//! Durable storage is deliberately out of scope — see the Phase 0 contract doc.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use prost_types::Timestamp;

use crate::protos::google::firestore::v1::Document;

/// Thread-safe handle to the document store. Cloning shares the same data.
#[derive(Clone, Default)]
pub struct DocumentStore {
    documents: Arc<RwLock<BTreeMap<String, Document>>>,
}

/// Outcome of a write, so callers can report the right gRPC status without
/// re-reading the store.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Created,
    Updated,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a document by full resource name.
    pub fn get(&self, name: &str) -> Option<Document> {
        self.documents
            .read()
            .expect("document store lock poisoned")
            .get(name)
            .cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.documents
            .read()
            .expect("document store lock poisoned")
            .contains_key(name)
    }

    /// Insert or replace a document wholesale.
    ///
    /// `create_time` is preserved across overwrites of an existing document,
    /// matching the documented Firestore behaviour that `create_time` only
    /// advances when a document is deleted and recreated.
    pub fn put(&self, mut document: Document, now: Timestamp) -> WriteOutcome {
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
        outcome
    }

    /// Merge `fields` into an existing document, creating it if absent.
    /// Only the supplied field names are touched.
    pub fn merge(
        &self,
        name: &str,
        fields: impl IntoIterator<Item = (String, crate::protos::google::firestore::v1::Value)>,
        now: Timestamp,
    ) -> WriteOutcome {
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
        outcome
    }

    /// Remove a document. Returns whether it existed; Firestore deletes are
    /// idempotent, so a missing document is not an error.
    pub fn delete(&self, name: &str) -> bool {
        self.documents
            .write()
            .expect("document store lock poisoned")
            .remove(name)
            .is_some()
    }

    pub fn len(&self) -> usize {
        self.documents
            .read()
            .expect("document store lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protos::google::firestore::v1::{value::ValueType, Value};

    fn ts(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    fn string_value(text: &str) -> Value {
        Value {
            value_type: Some(ValueType::StringValue(text.to_string())),
        }
    }

    fn doc(name: &str, field: &str, text: &str) -> Document {
        Document {
            name: name.to_string(),
            fields: [(field.to_string(), string_value(text))].into_iter().collect(),
            create_time: None,
            update_time: None,
        }
    }

    #[test]
    fn put_then_get_round_trips() {
        let store = DocumentStore::new();
        assert_eq!(store.put(doc("a", "k", "v"), ts(10)), WriteOutcome::Created);

        let found = store.get("a").expect("document was stored");
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(10)));
        assert_eq!(found.fields["k"], string_value("v"));
    }

    #[test]
    fn overwrite_keeps_create_time_and_advances_update_time() {
        let store = DocumentStore::new();
        store.put(doc("a", "k", "v1"), ts(10));
        assert_eq!(store.put(doc("a", "k", "v2"), ts(20)), WriteOutcome::Updated);

        let found = store.get("a").expect("document still present");
        assert_eq!(found.create_time, Some(ts(10)), "create_time must not move");
        assert_eq!(found.update_time, Some(ts(20)));
        assert_eq!(found.fields["k"], string_value("v2"));
    }

    #[test]
    fn overwrite_drops_fields_absent_from_the_new_document() {
        let store = DocumentStore::new();
        store.put(doc("a", "keep", "v"), ts(10));
        store.put(doc("a", "other", "v"), ts(20));

        let found = store.get("a").expect("document still present");
        assert!(!found.fields.contains_key("keep"), "put replaces all fields");
    }

    #[test]
    fn merge_only_touches_named_fields() {
        let store = DocumentStore::new();
        store.put(doc("a", "keep", "v"), ts(10));
        store.merge("a", [("added".to_string(), string_value("x"))], ts(20));

        let found = store.get("a").expect("document still present");
        assert_eq!(found.fields["keep"], string_value("v"));
        assert_eq!(found.fields["added"], string_value("x"));
        assert_eq!(found.create_time, Some(ts(10)));
        assert_eq!(found.update_time, Some(ts(20)));
    }

    #[test]
    fn merge_creates_a_missing_document() {
        let store = DocumentStore::new();
        let outcome = store.merge("a", [("k".to_string(), string_value("v"))], ts(10));

        assert_eq!(outcome, WriteOutcome::Created);
        assert_eq!(store.get("a").expect("created").create_time, Some(ts(10)));
    }

    #[test]
    fn delete_reports_whether_the_document_existed() {
        let store = DocumentStore::new();
        store.put(doc("a", "k", "v"), ts(10));

        assert!(store.delete("a"));
        assert!(!store.delete("a"), "deletes are idempotent");
        assert!(store.is_empty());
    }
}
