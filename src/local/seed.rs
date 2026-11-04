//! `fireup local seed` — load a managed Firestore/Datastore export into the
//! local data plane's store.
//!
//! The export is read by the same parser `fireup import` uses
//! ([`FirestoreDocumentParser`]), so there is one implementation of the export
//! format in the project. Seeding then maps each parsed document onto a
//! Firestore resource name and writes it through the
//! [`Store`][crate::local::store::Store] trait, which means `--postgres-url`
//! seeds a durable store and omitting it seeds an in-memory one (useful only
//! in-process, e.g. in tests).
//!
//! Writes go through `put`, which upserts, so seeding the same export twice
//! leaves the store in the same state as seeding it once.
//!
//! ## Value types
//!
//! Most of a document's data arrives as plain JSON and infers its Firestore
//! value type from that JSON (string/number/bool/null/array/object). Four
//! Datastore types have no native JSON form, so the shared export parser decodes
//! them into a *tagged map* instead of erasing them — see
//! [`crate::types::DATASTORE_TYPE_TAG`] and its companions. Seeding reads those
//! same tags and restores the real value type:
//!
//! | Tag | Seeds as |
//! |---|---|
//! | `blob` | `bytesValue` |
//! | `timestamp` | `timestampValue` |
//! | `geo_point` | `geoPointValue` |
//! | `key` | `referenceValue` |
//!
//! which is the document-plane counterpart of `fireup import` reading the same
//! tags to produce `bytea`, `timestamptz`, `point`, and `text`.
//!
//! Dispatch is on the tag and never on a string's shape: a document holding the
//! literal text of a date is not the same as one holding a timestamp. A map that
//! merely happens to carry a `__fireup_type__` key whose value this module does
//! not know stays an ordinary `mapValue`.
//!
//! A tagged value whose payload cannot be restored — unparseable base64, a key
//! path in a non-default namespace — is reported as a skipped document rather
//! than seeded as something else, for the same reason the namespace refusal
//! below exists.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use prost_types::Timestamp;
use tracing::{debug, warn};

use crate::leveldb_parser::parser::{FirestoreDocumentParser, LevelDBParser};
use crate::local::store::{SharedStore, WriteOutcome};
use crate::local::value_json;
use crate::protos::google::firestore::v1::{value::ValueType, ArrayValue, Document, MapValue, Value};
use crate::types::{
    datastore_type_tag, DatastoreKey, DatastoreKeyId, DatastoreKeyPathElement, FirestoreDocument,
    DATASTORE_TYPE_BLOB, DATASTORE_TYPE_GEO_POINT, DATASTORE_TYPE_KEY, DATASTORE_TYPE_TIMESTAMP,
};

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
            // A seed only ever puts, so the delete and verify outcomes cannot
            // happen; counting them as updates keeps the match exhaustive
            // without inventing counters for impossible outcomes.
            Ok(
                WriteOutcome::Updated
                | WriteOutcome::Deleted { .. }
                | WriteOutcome::Verified,
            ) => report.updated += 1,
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

        match convert_fields(documents_root, &source.data) {
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
    documents_root: &str,
    data: &HashMap<String, serde_json::Value>,
) -> Result<HashMap<String, Value>, String> {
    let mut fields = HashMap::with_capacity(data.len());
    for (name, json) in data {
        let value = json_to_value(documents_root, json)
            .map_err(|reason| format!("field {name:?}: {reason}"))?;
        fields.insert(name.clone(), value);
    }
    Ok(fields)
}

/// Turn the JSON the export parser produced into a Firestore `Value`.
///
/// A map carrying a Datastore type tag becomes the value type that tag names
/// (see the module note). Everything else is inferred from its JSON shape:
/// integers stay integers, fractional numbers become doubles, and no string is
/// re-interpreted as a timestamp or as bytes.
///
/// `documents_root` is needed because a `referenceValue` is a full resource
/// name, while the tag carries a Datastore key path.
pub fn json_to_value(
    documents_root: &str,
    json: &serde_json::Value,
) -> Result<Value, String> {
    if let Some(tag) = datastore_type_tag(json) {
        if let Some(restored) = tagged_value(documents_root, tag, json) {
            return restored;
        }
        // An unknown tag value is somebody else's `__fireup_type__` key, not a
        // type this module can restore, so it keeps its JSON shape.
    }

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
                .map(|item| json_to_value(documents_root, item))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        serde_json::Value::Object(entries) => {
            let mut fields = HashMap::with_capacity(entries.len());
            for (key, value) in entries {
                fields.insert(key.clone(), json_to_value(documents_root, value)?);
            }
            ValueType::MapValue(MapValue { fields })
        }
    };

    Ok(Value {
        value_type: Some(value_type),
    })
}

/// Restore the value type a Datastore type tag names.
///
/// `None` means the tag is not one of ours, so the caller should treat the map
/// as ordinary data. `Some(Err(..))` means the tag *is* ours but its payload is
/// unusable, which must not be papered over as a map.
///
/// Each tagged payload is rebuilt in the public REST value shape and handed to
/// [`value_json::decode`], the codec the store itself round-trips through. That
/// keeps one implementation of RFC 3339 and base64 parsing, and guarantees
/// anything this produces is storable.
fn tagged_value(
    documents_root: &str,
    tag: &str,
    json: &serde_json::Value,
) -> Option<Result<Value, String>> {
    let rest_shape = match tag {
        DATASTORE_TYPE_TIMESTAMP => tagged_payload(json, tag, "rfc3339")
            .map(|rfc3339| serde_json::json!({ "timestampValue": rfc3339 })),
        DATASTORE_TYPE_BLOB => tagged_payload(json, tag, "base64")
            .map(|base64| serde_json::json!({ "bytesValue": base64 })),
        DATASTORE_TYPE_GEO_POINT => tagged_payload(json, tag, "latitude").and_then(|latitude| {
            let longitude = tagged_payload(json, tag, "longitude")?;
            Ok(serde_json::json!({
                "geoPointValue": { "latitude": latitude, "longitude": longitude },
            }))
        }),
        DATASTORE_TYPE_KEY => tagged_payload(json, tag, "key_path").and_then(|key_path| {
            let key_path = key_path
                .as_str()
                .ok_or_else(|| format!("{tag:?} value has a non-string \"key_path\""))?;
            Ok(serde_json::json!({
                "referenceValue": reference_name(documents_root, key_path)?,
            }))
        }),
        _ => return None,
    };

    Some(
        rest_shape.and_then(|shape| {
            value_json::decode(&shape)
                .map_err(|error| format!("{tag:?} value cannot be restored: {error}"))
        }),
    )
}

fn tagged_payload(
    json: &serde_json::Value,
    tag: &str,
    key: &str,
) -> Result<serde_json::Value, String> {
    json.get(key)
        .cloned()
        .ok_or_else(|| format!("{tag:?} value has no {key:?} payload"))
}

/// Resource name a key reference points at.
///
/// A `referenceValue` is a full `projects/…/documents/…` name, so the key path
/// the tag carries goes through the same key-to-document-path mapping as a
/// seeded document's own name — including its namespace refusal, because a
/// reference into another namespace has no document name either.
fn reference_name(documents_root: &str, key_path: &str) -> Result<String, String> {
    let key = key_from_encoded_path(key_path)?;
    Ok(format!("{documents_root}/{}", key_segments(&key)?.join("/")))
}

/// Read a Datastore encoded key path (`Users/ada`, `[tenant-b]Users/ada`) back
/// into the key it was written from by [`DatastoreKey::encoded_path`].
///
/// Every identifier comes back as a string name. The distinction between a name
/// and a numeric id is not recoverable from the encoded form and does not
/// matter here: both render as the same path segment.
fn key_from_encoded_path(encoded: &str) -> Result<DatastoreKey, String> {
    let (namespace, path) = match encoded.strip_prefix('[') {
        Some(rest) => {
            let (namespace, path) = rest.split_once(']').ok_or_else(|| {
                format!("key path {encoded:?} has an unterminated namespace prefix")
            })?;
            (namespace.to_string(), path)
        }
        None => (String::new(), encoded),
    };

    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() % 2 != 0 {
        return Err(format!(
            "key path {encoded:?} is not a sequence of kind/id pairs"
        ));
    }

    Ok(DatastoreKey {
        namespace,
        path: segments
            .chunks(2)
            .map(|pair| DatastoreKeyPathElement {
                kind: pair[0].to_string(),
                id: DatastoreKeyId::Name(pair[1].to_string()),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DocumentMetadata;

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

        let fields = convert_fields(ROOT, &data).expect("all values are representable");

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
    fn tagged_maps_restore_the_datastore_value_types_json_cannot_express() {
        let data: HashMap<String, serde_json::Value> = [
            (
                "created_at".to_string(),
                crate::types::tagged_timestamp("2026-03-14T09:26:53.589793Z".to_string()),
            ),
            (
                "avatar".to_string(),
                crate::types::tagged_blob("AAEC/f7/".to_string()),
            ),
            (
                "location".to_string(),
                crate::types::tagged_geo_point(51.5074, -0.1278),
            ),
            (
                "manager".to_string(),
                crate::types::tagged_key_reference("Users/ada".to_string()),
            ),
        ]
        .into_iter()
        .collect();

        let fields = convert_fields(ROOT, &data).expect("every tag is restorable");

        assert_eq!(
            fields["created_at"].value_type,
            Some(ValueType::TimestampValue(Timestamp {
                seconds: 1_773_480_413,
                nanos: 589_793_000,
            })),
            "the tag carries RFC 3339, so nanoseconds must survive"
        );
        assert_eq!(
            fields["avatar"].value_type,
            Some(ValueType::BytesValue(vec![0, 1, 2, 253, 254, 255]))
        );
        assert_eq!(
            fields["location"].value_type,
            Some(ValueType::GeoPointValue(
                crate::protos::google::r#type::LatLng {
                    latitude: 51.5074,
                    longitude: -0.1278,
                }
            ))
        );
        // A key path is a Datastore path; a referenceValue is a resource name.
        assert_eq!(
            fields["manager"].value_type,
            Some(ValueType::ReferenceValue(format!("{ROOT}/Users/ada"))),
        );

        // And everything the restore produced is storable as it stands.
        let encoded = value_json::encode_fields(&fields).expect("encodes");
        assert_eq!(
            value_json::decode_fields(&encoded).expect("decodes"),
            fields
        );
    }

    #[test]
    fn a_multi_element_key_reference_keeps_its_whole_path() {
        let data: HashMap<String, serde_json::Value> = [(
            "order".to_string(),
            crate::types::tagged_key_reference("Users/ada/Orders/1001".to_string()),
        )]
        .into_iter()
        .collect();

        let fields = convert_fields(ROOT, &data).expect("restorable");
        assert_eq!(
            fields["order"].value_type,
            Some(ValueType::ReferenceValue(format!(
                "{ROOT}/Users/ada/Orders/1001"
            ))),
        );
        // The reference has to name a document the data plane can serve.
        let Some(ValueType::ReferenceValue(name)) = &fields["order"].value_type else {
            panic!("expected a reference");
        };
        crate::local::names::parse_document_name(name).expect("valid document name");
    }

    #[test]
    fn tagged_values_are_restored_inside_arrays_and_maps_too() {
        let data: HashMap<String, serde_json::Value> = [
            (
                "history".to_string(),
                serde_json::json!([crate::types::tagged_timestamp(
                    "2024-05-01T12:00:00Z".to_string()
                )]),
            ),
            (
                "profile".to_string(),
                serde_json::json!({"home": crate::types::tagged_geo_point(0.0, 0.0)}),
            ),
        ]
        .into_iter()
        .collect();

        let fields = convert_fields(ROOT, &data).expect("restorable");

        let Some(ValueType::ArrayValue(array)) = &fields["history"].value_type else {
            panic!("expected an array");
        };
        assert!(matches!(
            array.values[0].value_type,
            Some(ValueType::TimestampValue(_))
        ));

        let Some(ValueType::MapValue(map)) = &fields["profile"].value_type else {
            panic!("expected a map");
        };
        assert!(matches!(
            map.fields["home"].value_type,
            Some(ValueType::GeoPointValue(_))
        ));
    }

    #[test]
    fn only_the_tag_decides_the_type_never_a_strings_shape() {
        // A document that holds the literal text of a date holds a string.
        let data: HashMap<String, serde_json::Value> = [
            ("when".to_string(), serde_json::json!("2026-03-14T09:26:53Z")),
            ("bytes_ish".to_string(), serde_json::json!("AAEC/f7/")),
            ("path_ish".to_string(), serde_json::json!("Users/ada")),
        ]
        .into_iter()
        .collect();

        let fields = convert_fields(ROOT, &data).expect("strings are representable");
        for name in ["when", "bytes_ish", "path_ish"] {
            assert!(
                matches!(fields[name].value_type, Some(ValueType::StringValue(_))),
                "{name} must stay a string, got {:?}",
                fields[name].value_type
            );
        }
    }

    #[test]
    fn a_map_with_an_unknown_type_tag_stays_a_map() {
        let data: HashMap<String, serde_json::Value> = [(
            "theirs".to_string(),
            serde_json::json!({
                crate::types::DATASTORE_TYPE_TAG: "something-else",
                "payload": 1,
            }),
        )]
        .into_iter()
        .collect();

        let fields = convert_fields(ROOT, &data).expect("an unknown tag is ordinary data");
        let Some(ValueType::MapValue(map)) = &fields["theirs"].value_type else {
            panic!("expected a map, got {:?}", fields["theirs"].value_type);
        };
        assert_eq!(
            map.fields[crate::types::DATASTORE_TYPE_TAG].value_type,
            Some(ValueType::StringValue("something-else".to_string()))
        );
        assert_eq!(map.fields["payload"].value_type, Some(ValueType::IntegerValue(1)));
    }

    #[test]
    fn a_tagged_value_with_an_unusable_payload_is_refused_rather_than_seeded_as_a_map() {
        let cases = [
            ("not RFC 3339", crate::types::tagged_timestamp("yesterday".to_string())),
            ("not base64", crate::types::tagged_blob("not base64!!".to_string())),
            (
                "a namespaced key has no document name",
                crate::types::tagged_key_reference("[tenant-b]Users/ada".to_string()),
            ),
            (
                "an odd key path is not kind/id pairs",
                crate::types::tagged_key_reference("Users".to_string()),
            ),
            (
                "a missing payload key",
                serde_json::json!({ crate::types::DATASTORE_TYPE_TAG: "timestamp" }),
            ),
        ];

        for (what, json) in cases {
            let data: HashMap<String, serde_json::Value> =
                [("field".to_string(), json)].into_iter().collect();
            let reason = convert_fields(ROOT, &data)
                .expect_err(&format!("{what} must not seed as a map"));
            assert!(reason.contains("field"), "{what}: {reason}");
        }
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

        let fields = convert_fields(ROOT, &data).expect("representable");
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

        convert_fields(ROOT, &data).expect_err("u64::MAX cannot be stored exactly");
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
