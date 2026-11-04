//! `fireup local seed` — load a managed Firestore/Datastore export into the
//! local data plane's store.
//!
//! The export is read by the same parser `fireup import` uses
//! ([`FirestoreDocumentParser`]), so there is one implementation of the export
//! format in the project. Seeding then maps each parsed document onto a
//! Firestore resource name and writes it through the [`Store`] trait, which
//! means `--postgres-url` seeds a durable store and omitting it seeds an
//! in-memory one (useful only in-process, e.g. in tests).
//!
//! Writes go through `put`, which upserts, so seeding the same export twice
//! leaves the store in the same state as seeding it once.
//!
//! ## Known fidelity limit
//!
//! The shared export parser erases Firestore's value types on the way in: it
//! unwraps `{"timestampValue": "…"}` to a plain JSON string, `bytesValue` to
//! its base64 string, and so on. Seeding therefore infers types from JSON
//! (string/number/bool/null/array/object) and cannot restore `timestampValue`,
//! `bytesValue`, `geoPointValue` or `referenceValue`. Those fields seed as the
//! strings or maps the parser produced. This is recorded as a divergence rather
//! than papered over by guessing from a string's shape — a document holding the
//! literal text of a date is not the same as one holding a timestamp.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use prost_types::Timestamp;
use tracing::{debug, warn};

use crate::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser};
use crate::local::store::{SharedStore, WriteOutcome};
use crate::protos::google::firestore::v1::{value::ValueType, ArrayValue, Document, MapValue, Value};
use crate::types::{DatastoreKey, FirestoreDocument};

/// A document in the export that was not seeded, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSkip {
    /// Best available identification of the source document.
    pub source: String,
    pub reason: String,
}

/// What a seed run did.
#[derive(Debug, Default)]
pub struct SeedReport {
    /// Documents the export parser produced, including nested subcollections.
    pub parsed: usize,
    pub created: usize,
    pub updated: usize,
    /// Documents that could not be mapped to a Firestore document.
    pub skipped: Vec<SeedSkip>,
}

impl SeedReport {
    pub fn seeded(&self) -> usize {
        self.created + self.updated
    }

    /// A seed run is only complete when every parsed document was written.
    pub fn is_complete(&self) -> bool {
        self.skipped.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "seeded {} of {} documents ({} created, {} updated, {} skipped)",
            self.seeded(),
            self.parsed,
            self.created,
            self.updated,
            self.skipped.len()
        )
    }
}

/// Parse `backup_file` and write every document it holds into `store`.
///
/// `documents_root` is `projects/{project}/databases/{database}/documents`,
/// built from the `--project-id` / `--database-id` flags.
pub async fn seed_from_export(
    store: &SharedStore,
    documents_root: &str,
    backup_file: &Path,
) -> Result<SeedReport> {
    let path = backup_file
        .to_str()
        .with_context(|| format!("backup path {backup_file:?} is not valid UTF-8"))?;

    let parser = FirestoreDocumentParser::new(path.to_string());
    let parse_result = parser
        .parse_backup(path)
        .await
        .with_context(|| format!("failed to parse the export at {backup_file:?}"))?;

    let mut report = SeedReport::default();
    let mut mapped = Vec::new();
    collect_documents(
        documents_root,
        None,
        &parse_result.documents,
        &mut mapped,
        &mut report,
    );

    for (document, create_time) in mapped {
        let name = document.name.clone();
        match store.put(document, create_time).await {
            Ok(WriteOutcome::Created) => report.created += 1,
            // A seed only ever puts, so `Deleted` cannot happen; counting it
            // as an update keeps the match exhaustive without inventing a
            // fourth counter for an impossible outcome.
            Ok(WriteOutcome::Updated | WriteOutcome::Deleted { .. }) => report.updated += 1,
            Err(error) => {
                // One unstorable document must not abort the rest of the seed;
                // it is reported instead, and the caller treats a non-empty
                // skip list as an incomplete run.
                warn!(%name, %error, "failed to seed document");
                report.skipped.push(SeedSkip {
                    source: name,
                    reason: error.to_string(),
                });
            }
        }
    }

    Ok(report)
}

/// Walk the parsed export, turning each document (and its subcollections) into
/// a Firestore `Document` plus the timestamp to stamp it with.
fn collect_documents(
    documents_root: &str,
    parent_name: Option<&str>,
    documents: &[FirestoreDocument],
    out: &mut Vec<(Document, Timestamp)>,
    report: &mut SeedReport,
) {
    for source in documents {
        report.parsed += 1;

        let name = match document_name(documents_root, parent_name, source) {
            Ok(name) => name,
            Err(reason) => {
                report.skipped.push(SeedSkip {
                    source: source_label(source),
                    reason,
                });
                continue;
            }
        };

        match convert_fields(&source.data) {
            Ok(fields) => {
                debug!(%name, fields = fields.len(), "seeding document");
                out.push((
                    Document {
                        name: name.clone(),
                        fields,
                        create_time: None,
                        update_time: None,
                    },
                    seed_timestamp(source),
                ));
            }
            Err(reason) => {
                report.skipped.push(SeedSkip {
                    source: name.clone(),
                    reason,
                });
                // Still walk the subcollections: a child document is its own
                // row and does not depend on its parent being storable.
            }
        }

        if !source.subcollections.is_empty() {
            collect_documents(
                documents_root,
                Some(&name),
                &source.subcollections,
                out,
                report,
            );
        }
    }
}

fn source_label(source: &FirestoreDocument) -> String {
    if source.metadata.path != "unknown" && !source.metadata.path.is_empty() {
        source.metadata.path.clone()
    } else {
        format!("{}/{}", source.collection, source.id)
    }
}

/// Resource name for a parsed document.
///
/// A nested document's name hangs off its parent's. A top-level document uses
/// its Datastore key path when the export carried one — that path is the
/// document's real identity, including ancestors — and falls back to
/// `collection/id`.
fn document_name(
    documents_root: &str,
    parent_name: Option<&str>,
    source: &FirestoreDocument,
) -> Result<String, String> {
    let prefix = parent_name.unwrap_or(documents_root);

    let segments = match (parent_name, source.key.as_ref()) {
        (None, Some(key)) => key_segments(key)?,
        _ => {
            let collection = non_empty(&source.collection, "collection")?;
            let id = non_empty(&source.id, "document id")?;
            vec![collection, id]
        }
    };

    Ok(format!("{prefix}/{}", segments.join("/")))
}

/// `kind/id` pairs of a Datastore key become `collection/document` segments.
fn key_segments(key: &DatastoreKey) -> Result<Vec<String>, String> {
    // Datastore namespaces have no Firestore equivalent. Two entities with the
    // same key path in different namespaces would map to one document name and
    // silently overwrite each other, so a non-default namespace is refused.
    if !key.namespace.is_empty() {
        return Err(format!(
            "entity is in Datastore namespace {:?}, which has no Firestore document name; \
             seeding it would collide with the default namespace",
            key.namespace
        ));
    }
    if key.path.is_empty() {
        return Err("Datastore key has an empty path".to_string());
    }

    let mut segments = Vec::with_capacity(key.path.len() * 2);
    for element in &key.path {
        segments.push(non_empty(&element.kind, "key kind")?);
        segments.push(non_empty(&element.id.as_path_segment(), "key id")?);
    }
    Ok(segments)
}

/// Path segments must be non-empty and must not contain `/`, or the resulting
/// resource name would parse as a different document.
fn non_empty(segment: &str, what: &str) -> Result<String, String> {
    if segment.is_empty() {
        return Err(format!("{what} is empty"));
    }
    if segment.contains('/') {
        return Err(format!("{what} {segment:?} contains '/'"));
    }
    Ok(segment.to_string())
}

/// `create_time`/`update_time` for a seeded document: the export's own
/// timestamp when it carried one, otherwise now.
fn seed_timestamp(source: &FirestoreDocument) -> Timestamp {
    let moment = source
        .metadata
        .updated_at
        .or(source.metadata.created_at)
        .unwrap_or_else(chrono::Utc::now);
    Timestamp {
        seconds: moment.timestamp(),
        nanos: moment.timestamp_subsec_nanos() as i32,
    }
}

fn convert_fields(
    data: &HashMap<String, serde_json::Value>,
) -> Result<HashMap<String, Value>, String> {
    let mut fields = HashMap::with_capacity(data.len());
    for (name, json) in data {
        let value = json_to_value(json).map_err(|reason| format!("field {name:?}: {reason}"))?;
        fields.insert(name.clone(), value);
    }
    Ok(fields)
}

/// Infer a Firestore `Value` from the plain JSON the export parser produced.
///
/// Integers stay integers and fractional numbers become doubles; no string is
/// re-interpreted as a timestamp or as bytes (see the module note).
pub fn json_to_value(json: &serde_json::Value) -> Result<Value, String> {
    let value_type = match json {
        serde_json::Value::Null => ValueType::NullValue(0),
        serde_json::Value::Bool(flag) => ValueType::BooleanValue(*flag),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                ValueType::IntegerValue(integer)
            } else if number.is_u64() {
                // Above i64::MAX. Firestore's integer type is int64, and
                // widening to a double would change the number, so refuse it
                // rather than store a different value than the export held.
                return Err(format!("{number} is larger than an int64 can hold"));
            } else if let Some(double) = number.as_f64() {
                ValueType::DoubleValue(double)
            } else {
                return Err(format!("{number} is not representable as an int64 or a double"));
            }
        }
        serde_json::Value::String(text) => ValueType::StringValue(text.clone()),
        serde_json::Value::Array(items) => ValueType::ArrayValue(ArrayValue {
            values: items
                .iter()
                .map(json_to_value)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        serde_json::Value::Object(entries) => {
            let mut fields = HashMap::with_capacity(entries.len());
            for (key, value) in entries {
                fields.insert(key.clone(), json_to_value(value)?);
            }
            ValueType::MapValue(MapValue { fields })
        }
    };

    Ok(Value {
        value_type: Some(value_type),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DatastoreKeyId, DatastoreKeyPathElement, DocumentMetadata};

    const ROOT: &str = "projects/demo/databases/(default)/documents";

    fn metadata(path: &str) -> DocumentMetadata {
        DocumentMetadata {
            created_at: None,
            updated_at: None,
            path: path.to_string(),
            size_bytes: None,
        }
    }

    fn document(collection: &str, id: &str) -> FirestoreDocument {
        FirestoreDocument {
            id: id.to_string(),
            collection: collection.to_string(),
            data: HashMap::new(),
            subcollections: Vec::new(),
            metadata: metadata(&format!("{collection}/{id}")),
            key: None,
        }
    }

    fn collect(documents: &[FirestoreDocument]) -> (Vec<(Document, Timestamp)>, SeedReport) {
        let mut out = Vec::new();
        let mut report = SeedReport::default();
        collect_documents(ROOT, None, documents, &mut out, &mut report);
        (out, report)
    }

    #[test]
    fn a_top_level_document_gets_a_rooted_resource_name() {
        let (mapped, report) = collect(&[document("users", "alice")]);

        assert_eq!(report.parsed, 1);
        assert!(report.is_complete(), "{:?}", report.skipped);
        assert_eq!(mapped[0].0.name, format!("{ROOT}/users/alice"));
        // And the data plane must agree that the name is a document.
        crate::local::names::parse_document_name(&mapped[0].0.name).expect("valid document name");
    }

    #[test]
    fn subcollections_nest_under_their_parent() {
        let mut parent = document("users", "alice");
        parent.subcollections = vec![document("posts", "p1")];

        let (mapped, report) = collect(&[parent]);
        assert_eq!(report.parsed, 2);
        assert!(report.is_complete());
        let names: Vec<&str> = mapped.iter().map(|(doc, _)| doc.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                format!("{ROOT}/users/alice"),
                format!("{ROOT}/users/alice/posts/p1")
            ]
        );
    }

    #[test]
    fn a_datastore_key_path_becomes_the_document_path() {
        let mut source = document("Post", "123");
        source.key = Some(DatastoreKey {
            namespace: String::new(),
            path: vec![
                DatastoreKeyPathElement {
                    kind: "User".to_string(),
                    id: DatastoreKeyId::Name("alice".to_string()),
                },
                DatastoreKeyPathElement {
                    kind: "Post".to_string(),
                    id: DatastoreKeyId::Id(123),
                },
            ],
        });

        let (mapped, report) = collect(&[source]);
        assert!(report.is_complete());
        assert_eq!(mapped[0].0.name, format!("{ROOT}/User/alice/Post/123"));
    }

    #[test]
    fn a_namespaced_entity_is_skipped_rather_than_merged_into_the_default_namespace() {
        let mut source = document("Post", "1");
        source.key = Some(DatastoreKey {
            namespace: "tenant-b".to_string(),
            path: vec![DatastoreKeyPathElement {
                kind: "Post".to_string(),
                id: DatastoreKeyId::Id(1),
            }],
        });

        let (mapped, report) = collect(&[source]);
        assert!(mapped.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("namespace"), "{:?}", report.skipped);
        assert!(!report.is_complete());
    }

    #[test]
    fn documents_with_unusable_paths_are_skipped_with_a_reason() {
        let (mapped, report) = collect(&[
            document("", "alice"),
            document("users", ""),
            document("users/extra", "alice"),
        ]);

        assert!(mapped.is_empty());
        assert_eq!(report.skipped.len(), 3, "{:?}", report.skipped);
        assert_eq!(report.parsed, 3);
    }

    #[test]
    fn json_fields_map_onto_firestore_value_types() {
        let data: HashMap<String, serde_json::Value> = [
            ("text".to_string(), serde_json::json!("hello")),
            ("count".to_string(), serde_json::json!(7)),
            ("ratio".to_string(), serde_json::json!(1.5)),
            ("flag".to_string(), serde_json::json!(true)),
            ("nothing".to_string(), serde_json::json!(null)),
            ("list".to_string(), serde_json::json!([1, "two"])),
            ("nested".to_string(), serde_json::json!({"a": 1})),
        ]
        .into_iter()
        .collect();

        let fields = convert_fields(&data).expect("all values are representable");

        assert_eq!(
            fields["text"].value_type,
            Some(ValueType::StringValue("hello".to_string()))
        );
        assert_eq!(fields["count"].value_type, Some(ValueType::IntegerValue(7)));
        assert_eq!(fields["ratio"].value_type, Some(ValueType::DoubleValue(1.5)));
        assert_eq!(fields["flag"].value_type, Some(ValueType::BooleanValue(true)));
        assert_eq!(fields["nothing"].value_type, Some(ValueType::NullValue(0)));

        let Some(ValueType::ArrayValue(array)) = &fields["list"].value_type else {
            panic!("expected an array");
        };
        assert_eq!(array.values.len(), 2);

        let Some(ValueType::MapValue(map)) = &fields["nested"].value_type else {
            panic!("expected a map");
        };
        assert_eq!(map.fields["a"].value_type, Some(ValueType::IntegerValue(1)));
    }

    #[test]
    fn seeded_values_survive_the_storage_codec() {
        // Seeding writes through the same JSONB codec the store uses, so every
        // value the mapper can produce has to be storable.
        let data: HashMap<String, serde_json::Value> = [(
            "everything".to_string(),
            serde_json::json!({"n": null, "b": false, "i": -3, "f": 2.5, "s": "x", "a": [{"deep": 1}]}),
        )]
        .into_iter()
        .collect();

        let fields = convert_fields(&data).expect("representable");
        let encoded = crate::local::value_json::encode_fields(&fields).expect("encodes");
        assert_eq!(
            crate::local::value_json::decode_fields(&encoded).expect("decodes"),
            fields
        );
    }

    #[test]
    fn numbers_too_large_for_an_int64_are_refused_rather_than_rounded() {
        let data: HashMap<String, serde_json::Value> = [(
            "huge".to_string(),
            serde_json::Value::Number(serde_json::Number::from(u64::MAX)),
        )]
        .into_iter()
        .collect();

        convert_fields(&data).expect_err("u64::MAX cannot be stored exactly");
    }

    #[test]
    fn the_export_timestamp_is_used_when_the_export_carried_one() {
        let mut source = document("users", "alice");
        source.metadata.updated_at = Some(
            chrono::DateTime::parse_from_rfc3339("2024-05-01T12:00:00Z")
                .expect("valid")
                .with_timezone(&chrono::Utc),
        );

        let (mapped, _) = collect(&[source]);
        assert_eq!(mapped[0].1.seconds, 1_714_564_800);
    }

    #[test]
    fn the_report_only_reads_complete_when_nothing_was_skipped() {
        let mut report = SeedReport {
            parsed: 2,
            created: 1,
            updated: 1,
            skipped: Vec::new(),
        };
        assert!(report.is_complete());
        assert_eq!(report.seeded(), 2);

        report.skipped.push(SeedSkip {
            source: "x".to_string(),
            reason: "y".to_string(),
        });
        assert!(!report.is_complete());
        assert!(report.summary().contains("1 skipped"), "{}", report.summary());
    }
}
