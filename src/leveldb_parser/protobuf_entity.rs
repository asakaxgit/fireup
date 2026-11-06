//! Map Google Datastore Entity protobuf messages into `FirestoreDocument`.
//!
//! This handles `google.datastore.v1.Entity` — the *API* proto. Managed exports
//! carry the legacy storage proto instead; see
//! [`crate::leveldb_parser::entity_proto`]. Both are supported, and
//! `parser::parse_protobuf_document` picks per record.

use crate::error::{ErrorContext, FireupError};
use crate::protos::google::datastore::v1::key::path_element::IdType;
use crate::protos::google::datastore::v1::value::ValueType;
use crate::protos::google::datastore::v1::{Entity, EntityResult, Key, Value};
use crate::types::{
    tagged_blob, tagged_geo_point, tagged_key_reference, tagged_non_finite_double,
    tagged_timestamp, DatastoreKey, DatastoreKeyId, DatastoreKeyPathElement, DocumentMetadata,
    FirestoreDocument,
};
use bytes::Bytes;
use prost::Message;
use serde_json::{Map, Number, Value as JsonValue};
use std::collections::HashMap;
use tracing::debug;

/// Decode a LevelDB record payload as a Datastore Entity (or EntityResult wrapper).
pub fn try_decode_entity_document(
    record_data: &Bytes,
    record_index: usize,
) -> Result<Option<FirestoreDocument>, FireupError> {
    if record_data.is_empty() {
        return Ok(None);
    }

    // Prefer EntityResult when present — its wire shape can also decode as a
    // partial Entity without a usable key.
    if let Ok(result) = EntityResult::decode(record_data.as_ref()) {
        if let Some(entity) = result.entity.as_ref() {
            if entity.key.is_some() || !entity.properties.is_empty() {
                return Ok(Some(entity_to_document(entity, record_data.len())?));
            }
        }
    }

    if let Ok(entity) = Entity::decode(record_data.as_ref()) {
        if entity.key.is_some() || !entity.properties.is_empty() {
            return Ok(Some(entity_to_document(&entity, record_data.len())?));
        }
    }

    debug!(
        "Record {} ({} bytes) is not a recognizable Datastore Entity protobuf",
        record_index,
        record_data.len()
    );
    Ok(None)
}

fn entity_to_document(entity: &Entity, size_bytes: usize) -> Result<FirestoreDocument, FireupError> {
    let datastore_key = datastore_key_from_proto(entity.key.as_ref());

    let collection = datastore_key
        .as_ref()
        .and_then(|key| key.kind())
        .filter(|kind| !kind.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let id = match datastore_key.as_ref().and_then(|key| key.path.last()) {
        Some(element) => element.id.as_path_segment(),
        None => format!("doc-{}", uuid::Uuid::new_v4()),
    };
    let path = datastore_key
        .as_ref()
        .map(|key| key.path_string())
        .unwrap_or_default();

    let mut data = HashMap::new();
    for (name, value) in &entity.properties {
        data.insert(name.clone(), datastore_value_to_json(value)?);
    }

    Ok(FirestoreDocument {
        id,
        collection,
        data,
        subcollections: Vec::new(),
        metadata: DocumentMetadata {
            created_at: None,
            updated_at: None,
            path,
            size_bytes: Some(size_bytes as u64),
        },
        key: datastore_key,
    })
}

/// Lift a protobuf `Key` into the full `DatastoreKey` identity, keeping the
/// namespace and every ancestor path element.
fn datastore_key_from_proto(key: Option<&Key>) -> Option<DatastoreKey> {
    let key = key?;
    if key.path.is_empty() {
        return None;
    }

    let namespace = key
        .partition_id
        .as_ref()
        .map(|partition| partition.namespace_id.clone())
        .unwrap_or_default();

    let path = key
        .path
        .iter()
        .map(|element| DatastoreKeyPathElement {
            kind: element.kind.clone(),
            id: match &element.id_type {
                Some(IdType::Name(name)) => DatastoreKeyId::Name(name.clone()),
                Some(IdType::Id(id)) => DatastoreKeyId::Id(*id),
                None => DatastoreKeyId::Name(String::new()),
            },
        })
        .collect();

    Some(DatastoreKey { namespace, path })
}

fn datastore_value_to_json(value: &Value) -> Result<JsonValue, FireupError> {
    let context = || ErrorContext {
        operation: "datastore_value_to_json".to_string(),
        metadata: HashMap::new(),
        timestamp: chrono::Utc::now(),
        call_path: vec!["leveldb_parser::protobuf_entity".to_string()],
    };

    match value.value_type.as_ref() {
        None | Some(ValueType::NullValue(_)) => Ok(JsonValue::Null),
        Some(ValueType::BooleanValue(v)) => Ok(JsonValue::Bool(*v)),
        Some(ValueType::IntegerValue(v)) => Ok(JsonValue::Number((*v).into())),
        // A NaN/±Infinity double used to fail the whole record, which dropped
        // the entity from the import entirely. Both are legal Datastore values
        // and PostgreSQL can store them, so tag them the way the legacy
        // `EntityProto` decoder does and keep the entity.
        Some(ValueType::DoubleValue(v)) => Ok(Number::from_f64(*v)
            .map(JsonValue::Number)
            .or_else(|| tagged_non_finite_double(*v))
            .unwrap_or(JsonValue::Null)),
        Some(ValueType::TimestampValue(ts)) => {
            let datetime = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .ok_or_else(|| {
                    FireupError::leveldb_parse(
                        format!(
                            "Invalid timestamp in protobuf Entity: seconds={}, nanos={}",
                            ts.seconds, ts.nanos
                        ),
                        context(),
                    )
                })?;
            Ok(tagged_timestamp(datetime.to_rfc3339()))
        }
        Some(ValueType::StringValue(v)) => Ok(JsonValue::String(v.clone())),
        // Blobs, geo points and key references are tagged rather than flattened
        // into strings: the analyzer needs the source type to pick BYTEA/POINT
        // instead of inferring TEXT or JSONB from the stringified form.
        Some(ValueType::BlobValue(v)) => Ok(tagged_blob(base64_encode(v))),
        Some(ValueType::GeoPointValue(point)) => {
            Ok(tagged_geo_point(point.latitude, point.longitude))
        }
        Some(ValueType::KeyValue(key)) => Ok(tagged_key_reference(format_key(key))),
        Some(ValueType::EntityValue(entity)) => {
            let mut map = Map::new();
            for (name, nested) in &entity.properties {
                map.insert(name.clone(), datastore_value_to_json(nested)?);
            }
            Ok(JsonValue::Object(map))
        }
        Some(ValueType::ArrayValue(array)) => {
            let mut items = Vec::with_capacity(array.values.len());
            for item in &array.values {
                items.push(datastore_value_to_json(item)?);
            }
            Ok(JsonValue::Array(items))
        }
    }
}

fn format_key(key: &Key) -> String {
    key.path
        .iter()
        .map(|element| {
            let id = match &element.id_type {
                Some(IdType::Name(name)) => name.clone(),
                Some(IdType::Id(id)) => id.to_string(),
                None => "unknown".to_string(),
            };
            format!("{}/{}", element.kind, id)
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protos::google::datastore::v1::key::PathElement;
    use crate::protos::google::datastore::v1::{key, ArrayValue};

    fn sample_entity() -> Entity {
        let mut properties = HashMap::new();
        properties.insert(
            "name".to_string(),
            Value {
                value_type: Some(ValueType::StringValue("Ada".to_string())),
                ..Default::default()
            },
        );
        properties.insert(
            "age".to_string(),
            Value {
                value_type: Some(ValueType::IntegerValue(36)),
                ..Default::default()
            },
        );
        properties.insert(
            "active".to_string(),
            Value {
                value_type: Some(ValueType::BooleanValue(true)),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_string(),
            Value {
                value_type: Some(ValueType::ArrayValue(ArrayValue {
                    values: vec![
                        Value {
                            value_type: Some(ValueType::StringValue("rust".to_string())),
                            ..Default::default()
                        },
                        Value {
                            value_type: Some(ValueType::StringValue("db".to_string())),
                            ..Default::default()
                        },
                    ],
                })),
                ..Default::default()
            },
        );

        Entity {
            key: Some(Key {
                partition_id: None,
                path: vec![PathElement {
                    kind: "users".to_string(),
                    id_type: Some(key::path_element::IdType::Name("ada".to_string())),
                }],
            }),
            properties,
        }
    }

    #[test]
    fn decodes_entity_bytes_into_firestore_document() {
        let entity = sample_entity();
        let mut buf = Vec::new();
        entity.encode(&mut buf).expect("encode entity");

        let document = try_decode_entity_document(&Bytes::from(buf), 0)
            .expect("decode")
            .expect("document present");

        assert_eq!(document.collection, "users");
        assert_eq!(document.id, "ada");
        assert_eq!(document.metadata.path, "users/ada");
        assert_eq!(
            document.data.get("name"),
            Some(&JsonValue::String("Ada".to_string()))
        );
        assert_eq!(document.data.get("age"), Some(&JsonValue::Number(36.into())));
        assert_eq!(document.data.get("active"), Some(&JsonValue::Bool(true)));
        assert_eq!(
            document.data.get("tags"),
            Some(&JsonValue::Array(vec![
                JsonValue::String("rust".to_string()),
                JsonValue::String("db".to_string()),
            ]))
        );
    }

    #[test]
    fn decodes_entity_result_wrapper() {
        let entity = sample_entity();
        let wrapped = EntityResult {
            entity: Some(entity),
            ..Default::default()
        };
        let mut buf = Vec::new();
        wrapped.encode(&mut buf).expect("encode entity result");

        let document = try_decode_entity_document(&Bytes::from(buf), 1)
            .expect("decode")
            .expect("document present");
        assert_eq!(document.id, "ada");
    }

    #[test]
    fn returns_none_for_non_entity_bytes() {
        let result = try_decode_entity_document(&Bytes::from_static(b"{not-protobuf"), 2)
            .expect("should not error");
        assert!(result.is_none());
    }
}
