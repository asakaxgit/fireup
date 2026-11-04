//! Map Datastore keys/entities onto the shared Firestore document store.
//!
//! Kind path elements become Firestore collection/document segments — the same
//! mapping [`seed`](crate::local::seed) uses when loading a managed export — so
//! a document written through either API is visible to the other. Namespaces
//! have no Firestore equivalent and are refused rather than silently merged.
//!
//! Value conversion is between the public `google.datastore.v1.Value` and
//! `google.firestore.v1.Value` shapes. Key values become Firestore reference
//! strings; entity values become maps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use tonic::Status;

use crate::local::names::parse_document_name;
use crate::protos::google::datastore::v1::key::path_element::IdType as DsIdType;
use crate::protos::google::datastore::v1::value::ValueType as DsValueType;
use crate::protos::google::datastore::v1::{
    key::PathElement as DsPathElement, ArrayValue as DsArrayValue, Entity as DsEntity,
    Key as DsKey, PartitionId, Value as DsValue,
};
use crate::protos::google::firestore::v1::value::ValueType as FsValueType;
use crate::protos::google::firestore::v1::{
    ArrayValue as FsArrayValue, Document, MapValue as FsMapValue, Value as FsValue,
};

/// Next numeric id for [`AllocateIds`](crate::local::datastore) and incomplete
/// keys on insert/upsert. Starts above typical fixture ids so tests that pick
/// small numbers do not collide with auto-allocated ones.
static NEXT_ID: AtomicI64 = AtomicI64::new(1_000_000_000_001);

/// Allocate one positive numeric Datastore id.
pub fn allocate_id() -> i64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// `projects/{project}/databases/{database}/documents` for this data plane.
pub fn documents_root(project_id: &str, database_id: &str) -> String {
    format!("projects/{project_id}/databases/{database_id}/documents")
}

/// Datastore's default database is the empty string; Firestore's is `(default)`.
pub fn normalize_database_id(database_id: &str) -> &str {
    if database_id.is_empty() {
        "(default)"
    } else {
        database_id
    }
}

/// Refuse a partition/database the process was not started for.
pub fn check_project_database(
    request_project: &str,
    request_database: &str,
    configured_project: &str,
    configured_database: &str,
) -> Result<(), Status> {
    if request_project != configured_project {
        return Err(Status::invalid_argument(format!(
            "project_id {request_project:?} does not match this server's project {configured_project:?}"
        )));
    }
    let request_database = normalize_database_id(request_database);
    let configured_database = normalize_database_id(configured_database);
    if request_database != configured_database {
        return Err(Status::invalid_argument(format!(
            "database_id {request_database:?} does not match this server's database {configured_database:?}"
        )));
    }
    Ok(())
}

/// Full Firestore resource name for a complete Datastore key.
pub fn key_to_document_name(
    key: &DsKey,
    project_id: &str,
    database_id: &str,
) -> Result<String, Status> {
    let segments = key_segments(key, project_id, database_id)?;
    Ok(format!(
        "{}/{}",
        documents_root(project_id, normalize_database_id(database_id)),
        segments.join("/")
    ))
}

/// Parse a stored document name back into a Datastore key.
pub fn document_name_to_key(
    name: &str,
    project_id: &str,
    database_id: &str,
) -> Result<DsKey, Status> {
    let parsed = parse_document_name(name)?;
    let expected_root = documents_root(project_id, normalize_database_id(database_id));
    if parsed.root != expected_root {
        return Err(Status::internal(format!(
            "stored document {name} is outside this server's database root"
        )));
    }

    let mut path = Vec::with_capacity(parsed.segments.len() / 2);
    for chunk in parsed.segments.chunks(2) {
        let kind = chunk[0].clone();
        let id_segment = &chunk[1];
        let id_type = match id_segment.parse::<i64>() {
            // Digit-only segments round-trip as numeric ids. A string name that
            // happens to be all digits is therefore indistinguishable after a
            // store round-trip — recorded in the Phase 3 fidelity notes.
            Ok(id) if !id_segment.is_empty() && id_segment.bytes().all(|b| b.is_ascii_digit()) => {
                DsIdType::Id(id)
            }
            _ => DsIdType::Name(id_segment.clone()),
        };
        path.push(DsPathElement {
            kind,
            id_type: Some(id_type),
        });
    }

    Ok(DsKey {
        partition_id: Some(PartitionId {
            project_id: project_id.to_string(),
            database_id: if normalize_database_id(database_id) == "(default)" {
                String::new()
            } else {
                normalize_database_id(database_id).to_string()
            },
            namespace_id: String::new(),
        }),
        path,
    })
}

fn key_segments(
    key: &DsKey,
    project_id: &str,
    database_id: &str,
) -> Result<Vec<String>, Status> {
    if let Some(partition) = &key.partition_id {
        if !partition.namespace_id.is_empty() {
            return Err(Status::unimplemented(format!(
                "fireup local does not serve Datastore namespaces yet \
                 (got {:?}); entities stay in the default namespace only",
                partition.namespace_id
            )));
        }
        if !partition.project_id.is_empty() && partition.project_id != project_id {
            return Err(Status::invalid_argument(format!(
                "key project_id {:?} does not match request project {project_id:?}",
                partition.project_id
            )));
        }
        let key_db = normalize_database_id(&partition.database_id);
        let want_db = normalize_database_id(database_id);
        if key_db != want_db {
            return Err(Status::invalid_argument(format!(
                "key database_id {key_db:?} does not match request database {want_db:?}"
            )));
        }
    }
    if key.path.is_empty() {
        return Err(Status::invalid_argument("Datastore key has an empty path"));
    }

    let mut segments = Vec::with_capacity(key.path.len() * 2);
    for (index, element) in key.path.iter().enumerate() {
        if element.kind.is_empty() {
            return Err(Status::invalid_argument("key path element has an empty kind"));
        }
        if element.kind.contains('/') {
            return Err(Status::invalid_argument(format!(
                "key kind {:?} contains '/'",
                element.kind
            )));
        }
        segments.push(element.kind.clone());

        let id_segment = match &element.id_type {
            Some(DsIdType::Name(name)) => {
                if name.is_empty() {
                    return Err(Status::invalid_argument("key path element has an empty name"));
                }
                if name.contains('/') {
                    return Err(Status::invalid_argument(format!(
                        "key name {name:?} contains '/'"
                    )));
                }
                name.clone()
            }
            Some(DsIdType::Id(id)) => {
                if *id == 0 {
                    return Err(Status::invalid_argument(
                        "numeric key id 0 is incomplete; use AllocateIds or leave id unset",
                    ));
                }
                id.to_string()
            }
            None => {
                // Incomplete keys are only legal on the final element of an
                // insert/upsert; callers that need a complete key must allocate
                // first. Reporting which element failed keeps the error usable.
                return Err(Status::invalid_argument(format!(
                    "key path element {index} (kind {:?}) is incomplete",
                    element.kind
                )));
            }
        };
        segments.push(id_segment);
    }
    Ok(segments)
}

/// Whether the key's final path element still needs an id allocated.
pub fn key_is_incomplete(key: &DsKey) -> bool {
    key.path
        .last()
        .map(|element| element.id_type.is_none())
        .unwrap_or(true)
}

/// Fill in a missing final numeric id. Ancestor elements must already be complete.
pub fn complete_key(mut key: DsKey) -> Result<DsKey, Status> {
    if key.path.is_empty() {
        return Err(Status::invalid_argument("cannot allocate an id for an empty key"));
    }
    if key.path.last().and_then(|element| element.id_type.as_ref()).is_some() {
        return Ok(key);
    }
    if let Some(index) = key
        .path
        .iter()
        .enumerate()
        .rev()
        .skip(1)
        .find_map(|(index, element)| element.id_type.is_none().then_some(index))
    {
        return Err(Status::invalid_argument(format!(
            "ancestor path element {index} is incomplete; only the final element may be"
        )));
    }
    let last = key.path.last_mut().expect("path non-empty");
    last.id_type = Some(DsIdType::Id(allocate_id()));
    Ok(key)
}

/// Convert a Datastore entity into a Firestore document for the shared store.
pub fn entity_to_document(
    entity: &DsEntity,
    project_id: &str,
    database_id: &str,
) -> Result<Document, Status> {
    let key = entity
        .key
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("entity is missing a key"))?;
    let name = key_to_document_name(key, project_id, database_id)?;
    let mut fields = HashMap::with_capacity(entity.properties.len());
    for (name, value) in &entity.properties {
        fields.insert(name.clone(), ds_value_to_fs(value, project_id, database_id)?);
    }
    Ok(Document {
        name,
        fields,
        create_time: None,
        update_time: None,
    })
}

/// Convert a stored Firestore document back into a Datastore entity.
pub fn document_to_entity(
    document: &Document,
    project_id: &str,
    database_id: &str,
) -> Result<DsEntity, Status> {
    let key = document_name_to_key(&document.name, project_id, database_id)?;
    let mut properties = HashMap::with_capacity(document.fields.len());
    for (name, value) in &document.fields {
        properties.insert(name.clone(), fs_value_to_ds(value, project_id, database_id)?);
    }
    Ok(DsEntity {
        key: Some(key),
        properties,
    })
}

/// Parent resource name and collection id for an ancestor-scoped kind query.
pub fn ancestor_collection(
    ancestor: &DsKey,
    kind: &str,
    project_id: &str,
    database_id: &str,
) -> Result<(String, String), Status> {
    let parent = key_to_document_name(ancestor, project_id, database_id)?;
    // Confirm the ancestor name is a document, not a collection.
    let _ = parse_document_name(&parent)?;
    Ok((parent, kind.to_string()))
}

/// Document name helpers used when applying a property mask.
pub fn apply_property_mask(entity: &mut DsEntity, paths: &[String]) -> Result<(), Status> {
    if paths.is_empty() {
        return Ok(());
    }
    for path in paths {
        if path.contains('.') || path.contains('`') {
            return Err(Status::unimplemented(format!(
                "fireup local does not support nested property masks yet (got {path:?})"
            )));
        }
    }
    entity
        .properties
        .retain(|name, _| paths.iter().any(|path| path == name));
    Ok(())
}

fn ds_value_to_fs(
    value: &DsValue,
    project_id: &str,
    database_id: &str,
) -> Result<FsValue, Status> {
    let value_type = match value.value_type.as_ref() {
        None | Some(DsValueType::NullValue(_)) => FsValueType::NullValue(0),
        Some(DsValueType::BooleanValue(v)) => FsValueType::BooleanValue(*v),
        Some(DsValueType::IntegerValue(v)) => FsValueType::IntegerValue(*v),
        Some(DsValueType::DoubleValue(v)) => FsValueType::DoubleValue(*v),
        Some(DsValueType::TimestampValue(v)) => FsValueType::TimestampValue(v.clone()),
        Some(DsValueType::StringValue(v)) => FsValueType::StringValue(v.clone()),
        Some(DsValueType::BlobValue(v)) => FsValueType::BytesValue(v.clone()),
        Some(DsValueType::GeoPointValue(v)) => FsValueType::GeoPointValue(v.clone()),
        Some(DsValueType::KeyValue(key)) => {
            FsValueType::ReferenceValue(key_to_document_name(key, project_id, database_id)?)
        }
        Some(DsValueType::EntityValue(entity)) => {
            let mut fields = HashMap::with_capacity(entity.properties.len());
            for (name, nested) in &entity.properties {
                fields.insert(name.clone(), ds_value_to_fs(nested, project_id, database_id)?);
            }
            FsValueType::MapValue(FsMapValue { fields })
        }
        Some(DsValueType::ArrayValue(array)) => {
            let mut values = Vec::with_capacity(array.values.len());
            for item in &array.values {
                values.push(ds_value_to_fs(item, project_id, database_id)?);
            }
            FsValueType::ArrayValue(FsArrayValue { values })
        }
    };
    Ok(FsValue {
        value_type: Some(value_type),
    })
}

fn fs_value_to_ds(
    value: &FsValue,
    project_id: &str,
    database_id: &str,
) -> Result<DsValue, Status> {
    let value_type = match value.value_type.as_ref() {
        None | Some(FsValueType::NullValue(_)) => DsValueType::NullValue(0),
        Some(FsValueType::BooleanValue(v)) => DsValueType::BooleanValue(*v),
        Some(FsValueType::IntegerValue(v)) => DsValueType::IntegerValue(*v),
        Some(FsValueType::DoubleValue(v)) => DsValueType::DoubleValue(*v),
        Some(FsValueType::TimestampValue(v)) => DsValueType::TimestampValue(v.clone()),
        Some(FsValueType::StringValue(v)) => DsValueType::StringValue(v.clone()),
        Some(FsValueType::BytesValue(v)) => DsValueType::BlobValue(v.clone()),
        Some(FsValueType::GeoPointValue(v)) => DsValueType::GeoPointValue(v.clone()),
        Some(FsValueType::ReferenceValue(name)) => {
            DsValueType::KeyValue(document_name_to_key(name, project_id, database_id)?)
        }
        Some(FsValueType::MapValue(map)) => {
            let mut properties = HashMap::with_capacity(map.fields.len());
            for (name, nested) in &map.fields {
                properties.insert(name.clone(), fs_value_to_ds(nested, project_id, database_id)?);
            }
            DsValueType::EntityValue(DsEntity {
                key: None,
                properties,
            })
        }
        Some(FsValueType::ArrayValue(array)) => {
            let mut values = Vec::with_capacity(array.values.len());
            for item in &array.values {
                values.push(fs_value_to_ds(item, project_id, database_id)?);
            }
            DsValueType::ArrayValue(DsArrayValue { values })
        }
        Some(
            FsValueType::FieldReferenceValue(_)
            | FsValueType::FunctionValue(_)
            | FsValueType::PipelineValue(_)
            | FsValueType::VariableReferenceValue(_),
        ) => {
            return Err(Status::internal(
                "stored document held a pipeline expression value that cannot map to Datastore",
            ));
        }
    };
    Ok(DsValue {
        meaning: 0,
        exclude_from_indexes: false,
        value_type: Some(value_type),
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_key_maps_to_a_document_resource_name() {
        let key = DsKey {
            partition_id: None,
            path: vec![DsPathElement {
                kind: "Task".to_string(),
                id_type: Some(DsIdType::Name("sample".to_string())),
            }],
        };
        let name = key_to_document_name(&key, "demo", "").expect("complete key");
        assert_eq!(name, "projects/demo/databases/(default)/documents/Task/sample");
        let round = document_name_to_key(&name, "demo", "").expect("round trip");
        assert_eq!(round.path[0].kind, "Task");
        assert_eq!(
            round.path[0].id_type,
            Some(DsIdType::Name("sample".to_string()))
        );
    }

    #[test]
    fn a_numeric_key_round_trips_as_an_id() {
        let key = DsKey {
            partition_id: None,
            path: vec![DsPathElement {
                kind: "Task".to_string(),
                id_type: Some(DsIdType::Id(42)),
            }],
        };
        let name = key_to_document_name(&key, "demo", "(default)").expect("complete key");
        assert_eq!(name, "projects/demo/databases/(default)/documents/Task/42");
        let round = document_name_to_key(&name, "demo", "(default)").expect("round trip");
        assert_eq!(round.path[0].id_type, Some(DsIdType::Id(42)));
    }

    #[test]
    fn a_namespaced_key_is_refused() {
        let key = DsKey {
            partition_id: Some(PartitionId {
                project_id: "demo".to_string(),
                database_id: String::new(),
                namespace_id: "tenant".to_string(),
            }),
            path: vec![DsPathElement {
                kind: "Task".to_string(),
                id_type: Some(DsIdType::Name("a".to_string())),
            }],
        };
        let err = key_to_document_name(&key, "demo", "").expect_err("namespace");
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
