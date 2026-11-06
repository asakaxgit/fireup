//! Decoder for legacy `storage_onestore_v3.EntityProto` payloads.
//!
//! A Firestore/Datastore **managed export** stores the *storage* proto, not the
//! `google.datastore.v1` API proto: records are proto2
//! `storage_onestore_v3.EntityProto` messages whose field numbering is
//! unrelated to the API shape (see `tests/fixtures/README.md`).
//!
//! ```text
//! EntityProto   key = 13 (Reference), property = 14 (repeated Property),
//!               raw_property = 15, entity_group = 16 (Path)
//! Reference     app = 13, path = 14 (Path), name_space = 20, database_id = 21
//! Path          repeated group Element = 1 { type = 2, id = 3, name = 4 }
//! Property      meaning = 1, meaning_uri = 2, name = 3, multiple = 4, value = 5
//! PropertyValue int64Value = 1, booleanValue = 2, stringValue = 3, doubleValue = 4,
//!               group PointValue = 5 { x = 6, y = 7 },
//!               group UserValue = 8 { email = 9, auth_domain = 10, ... },
//!               group ReferenceValue = 12 { app = 13,
//!                   repeated group PathElement = 14 { type = 15, id = 16, name = 17 },
//!                   name_space = 20, database_id = 23 }
//! ```
//!
//! Two properties of that schema force a hand-written wire decoder rather than a
//! `prost` message:
//!
//! 1. `Path.Element`, `PropertyValue.PointValue`, `PropertyValue.UserValue` and
//!    `PropertyValue.ReferenceValue` are proto2 **groups** (wire types 3/4),
//!    which `prost` does not support at all.
//! 2. A value's *type* lives in `Property.meaning`, not in the value field: a
//!    timestamp is `int64Value` + `GD_WHEN` in microseconds, a blob is
//!    `stringValue` + `BLOB`, a nested map is `stringValue` holding another
//!    serialized `EntityProto` + `ENTITY_PROTO`. Ignoring `meaning` corrupts
//!    data silently instead of failing.

use crate::leveldb_parser::base64::encode as base64_encode;
use crate::types::{
    tagged_blob, tagged_geo_point, tagged_key_reference, tagged_non_finite_double,
    tagged_timestamp, DatastoreKey, DatastoreKeyId, DatastoreKeyPathElement, DocumentMetadata,
    FirestoreDocument,
};
use serde_json::{Map, Number, Value as JsonValue};
use std::collections::HashMap;
use std::fmt;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// protobuf wire primitives
// ---------------------------------------------------------------------------

const WIRE_VARINT: u8 = 0;
const WIRE_FIXED64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_START_GROUP: u8 = 3;
const WIRE_END_GROUP: u8 = 4;
const WIRE_FIXED32: u8 = 5;

/// Guard against pathological nesting in `ENTITY_PROTO` values.
const MAX_NESTING_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

type Result<T> = std::result::Result<T, DecodeError>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(DecodeError(message.into()))
}

/// Minimal reader over a protobuf wire-format buffer, including proto2 groups.
struct WireReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> WireReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_byte(&mut self) -> Result<u8> {
        let Some(byte) = self.data.get(self.pos) else {
            return err("unexpected end of buffer");
        };
        self.pos += 1;
        Ok(*byte)
    }

    fn read_varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in 0..10 {
            let byte = self.read_byte()?;
            value |= u64::from(byte & 0x7F) << (shift * 7);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        err("varint longer than 10 bytes")
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| DecodeError("length overflow".to_string()))?;
        if end > self.data.len() {
            return err(format!(
                "length-delimited field of {len} bytes exceeds the remaining {} bytes",
                self.data.len() - self.pos
            ));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_length_delimited(&mut self) -> Result<&'a [u8]> {
        let len = self.read_varint()? as usize;
        self.read_slice(len)
    }

    fn read_fixed64(&mut self) -> Result<u64> {
        let slice = self.read_slice(8)?;
        Ok(u64::from_le_bytes(slice.try_into().expect("8 bytes")))
    }

    fn read_double(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_fixed64()?))
    }

    /// Read a `(field_number, wire_type)` tag.
    fn read_tag(&mut self) -> Result<(u32, u8)> {
        let tag = self.read_varint()?;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        if field == 0 {
            return err("field number 0 is not valid");
        }
        Ok((field, wire))
    }

    /// Read the body of a proto2 group opened by `field`, consuming its
    /// `END_GROUP` tag. Nested groups are skipped as a unit.
    fn read_group(&mut self, field: u32) -> Result<&'a [u8]> {
        let start = self.pos;
        loop {
            if self.is_empty() {
                return err(format!("group {field} is not terminated"));
            }
            let tag_start = self.pos;
            let (inner_field, inner_wire) = self.read_tag()?;
            if inner_wire == WIRE_END_GROUP {
                if inner_field != field {
                    return err(format!(
                        "group {field} closed by END_GROUP for field {inner_field}"
                    ));
                }
                return Ok(&self.data[start..tag_start]);
            }
            self.skip_field(inner_field, inner_wire)?;
        }
    }

    fn skip_field(&mut self, field: u32, wire: u8) -> Result<()> {
        match wire {
            WIRE_VARINT => {
                self.read_varint()?;
            }
            WIRE_FIXED64 => {
                self.read_slice(8)?;
            }
            WIRE_LEN => {
                self.read_length_delimited()?;
            }
            WIRE_START_GROUP => {
                self.read_group(field)?;
            }
            WIRE_FIXED32 => {
                self.read_slice(4)?;
            }
            other => return err(format!("unsupported wire type {other}")),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// message shapes
// ---------------------------------------------------------------------------

/// `Path.Element` / `ReferenceValue.PathElement`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathElement {
    /// `type` in the proto — the Datastore kind.
    pub kind: String,
    pub id: Option<i64>,
    pub name: Option<String>,
}

impl PathElement {
    /// The key component as text: the name when present, else the numeric id.
    pub fn identifier(&self) -> Option<String> {
        match (&self.name, self.id) {
            (Some(name), _) => Some(name.clone()),
            (None, Some(id)) => Some(id.to_string()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    pub app: String,
    pub name_space: String,
    pub database_id: String,
    pub path: Vec<PathElement>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointValue {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserValue {
    pub email: String,
    pub auth_domain: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyValue {
    pub int64_value: Option<i64>,
    pub boolean_value: Option<bool>,
    /// Proto `stringValue`, which also carries blobs and nested entities, so it
    /// is kept as raw bytes rather than a `String`.
    pub string_value: Option<Vec<u8>>,
    pub double_value: Option<f64>,
    pub point_value: Option<PointValue>,
    pub user_value: Option<UserValue>,
    pub reference_value: Option<Reference>,
}

impl PropertyValue {
    /// A `PropertyValue` with no field set means null.
    pub fn is_null(&self) -> bool {
        self.int64_value.is_none()
            && self.boolean_value.is_none()
            && self.string_value.is_none()
            && self.double_value.is_none()
            && self.point_value.is_none()
            && self.user_value.is_none()
            && self.reference_value.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Property {
    pub meaning: i32,
    pub meaning_uri: String,
    pub name: String,
    /// Set on every element of a multi-valued (array) property.
    pub multiple: bool,
    pub value: PropertyValue,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityProto {
    pub key: Option<Reference>,
    pub entity_group: Vec<PathElement>,
    pub properties: Vec<Property>,
    /// Unindexed properties. Semantically identical to `properties`.
    pub raw_properties: Vec<Property>,
}

// `Property.Meaning` values that change how a value must be read.
const MEANING_NO_MEANING: i32 = 0;
const MEANING_GD_WHEN: i32 = 7;
const MEANING_GEORSS_POINT: i32 = 9;
const MEANING_BLOB: i32 = 14;
const MEANING_TEXT: i32 = 15;
const MEANING_BYTESTRING: i32 = 16;
const MEANING_ENTITY_PROTO: i32 = 19;
const MEANING_EMPTY_LIST: i32 = 24;

// ---------------------------------------------------------------------------
// decoding
// ---------------------------------------------------------------------------

/// Decode an `EntityProto`. Fails when `data` is not this wire shape, which is
/// how callers tell a legacy export payload from a `datastore.v1` one.
pub fn decode_entity_proto(data: &[u8]) -> Result<EntityProto> {
    let mut reader = WireReader::new(data);
    let mut entity = EntityProto::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (13, WIRE_LEN) => entity.key = Some(decode_reference(reader.read_length_delimited()?)?),
            (14, WIRE_LEN) => entity
                .properties
                .push(decode_property(reader.read_length_delimited()?)?),
            (15, WIRE_LEN) => entity
                .raw_properties
                .push(decode_property(reader.read_length_delimited()?)?),
            (16, WIRE_LEN) => entity.entity_group = decode_path(reader.read_length_delimited()?)?,
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(entity)
}

/// `Path { repeated group Element = 1 { type = 2, id = 3, name = 4 } }`
fn decode_path(data: &[u8]) -> Result<Vec<PathElement>> {
    let mut reader = WireReader::new(data);
    let mut elements = Vec::new();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (1, WIRE_START_GROUP) => {
                elements.push(decode_path_element(reader.read_group(1)?, 2, 3, 4)?)
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(elements)
}

/// Shared by `Path.Element` (2/3/4) and `ReferenceValue.PathElement` (15/16/17).
fn decode_path_element(
    data: &[u8],
    kind_field: u32,
    id_field: u32,
    name_field: u32,
) -> Result<PathElement> {
    let mut reader = WireReader::new(data);
    let mut element = PathElement::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        if field == kind_field && wire == WIRE_LEN {
            element.kind = decode_utf8(reader.read_length_delimited()?, "path element type")?;
        } else if field == id_field && wire == WIRE_VARINT {
            element.id = Some(reader.read_varint()? as i64);
        } else if field == name_field && wire == WIRE_LEN {
            element.name = Some(decode_utf8(
                reader.read_length_delimited()?,
                "path element name",
            )?);
        } else {
            reader.skip_field(field, wire)?;
        }
    }

    if element.kind.is_empty() {
        return err("path element without a kind");
    }

    Ok(element)
}

/// `Reference { app = 13, path = 14, name_space = 20, database_id = 21 }`
fn decode_reference(data: &[u8]) -> Result<Reference> {
    let mut reader = WireReader::new(data);
    let mut reference = Reference::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (13, WIRE_LEN) => {
                reference.app = decode_utf8(reader.read_length_delimited()?, "reference app")?
            }
            (14, WIRE_LEN) => reference.path = decode_path(reader.read_length_delimited()?)?,
            (20, WIRE_LEN) => {
                reference.name_space =
                    decode_utf8(reader.read_length_delimited()?, "reference namespace")?
            }
            (21, WIRE_LEN) => {
                reference.database_id =
                    decode_utf8(reader.read_length_delimited()?, "reference database id")?
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(reference)
}

/// `Property { meaning = 1, meaning_uri = 2, name = 3, multiple = 4, value = 5 }`
fn decode_property(data: &[u8]) -> Result<Property> {
    let mut reader = WireReader::new(data);
    let mut property = Property::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (1, WIRE_VARINT) => property.meaning = reader.read_varint()? as i32,
            (2, WIRE_LEN) => {
                property.meaning_uri =
                    decode_utf8(reader.read_length_delimited()?, "property meaning_uri")?
            }
            (3, WIRE_LEN) => {
                property.name = decode_utf8(reader.read_length_delimited()?, "property name")?
            }
            (4, WIRE_VARINT) => property.multiple = reader.read_varint()? != 0,
            (5, WIRE_LEN) => {
                property.value = decode_property_value(reader.read_length_delimited()?)?
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    if property.name.is_empty() {
        return err("property without a name");
    }

    Ok(property)
}

fn decode_property_value(data: &[u8]) -> Result<PropertyValue> {
    let mut reader = WireReader::new(data);
    let mut value = PropertyValue::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (1, WIRE_VARINT) => value.int64_value = Some(reader.read_varint()? as i64),
            (2, WIRE_VARINT) => value.boolean_value = Some(reader.read_varint()? != 0),
            (3, WIRE_LEN) => value.string_value = Some(reader.read_length_delimited()?.to_vec()),
            (4, WIRE_FIXED64) => value.double_value = Some(reader.read_double()?),
            (5, WIRE_START_GROUP) => value.point_value = Some(decode_point(reader.read_group(5)?)?),
            (8, WIRE_START_GROUP) => value.user_value = Some(decode_user(reader.read_group(8)?)?),
            (12, WIRE_START_GROUP) => {
                value.reference_value = Some(decode_reference_value(reader.read_group(12)?)?)
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(value)
}

/// `group PointValue = 5 { x = 6, y = 7 }`
fn decode_point(data: &[u8]) -> Result<PointValue> {
    let mut reader = WireReader::new(data);
    let mut point = PointValue::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (6, WIRE_FIXED64) => point.x = reader.read_double()?,
            (7, WIRE_FIXED64) => point.y = reader.read_double()?,
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(point)
}

/// `group UserValue = 8 { email = 9, auth_domain = 10, ... }`
fn decode_user(data: &[u8]) -> Result<UserValue> {
    let mut reader = WireReader::new(data);
    let mut user = UserValue::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (9, WIRE_LEN) => {
                user.email = decode_utf8(reader.read_length_delimited()?, "user email")?
            }
            (10, WIRE_LEN) => {
                user.auth_domain =
                    decode_utf8(reader.read_length_delimited()?, "user auth domain")?
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(user)
}

/// `group ReferenceValue = 12 { app = 13, group PathElement = 14 {...},
/// name_space = 20, database_id = 23 }`
fn decode_reference_value(data: &[u8]) -> Result<Reference> {
    let mut reader = WireReader::new(data);
    let mut reference = Reference::default();

    while !reader.is_empty() {
        let (field, wire) = reader.read_tag()?;
        match (field, wire) {
            (13, WIRE_LEN) => {
                reference.app = decode_utf8(reader.read_length_delimited()?, "reference app")?
            }
            (14, WIRE_START_GROUP) => reference
                .path
                .push(decode_path_element(reader.read_group(14)?, 15, 16, 17)?),
            (20, WIRE_LEN) => {
                reference.name_space =
                    decode_utf8(reader.read_length_delimited()?, "reference namespace")?
            }
            (23, WIRE_LEN) => {
                reference.database_id =
                    decode_utf8(reader.read_length_delimited()?, "reference database id")?
            }
            _ => reader.skip_field(field, wire)?,
        }
    }

    Ok(reference)
}

fn decode_utf8(data: &[u8], what: &str) -> Result<String> {
    String::from_utf8(data.to_vec()).map_err(|e| DecodeError(format!("invalid UTF-8 in {what}: {e}")))
}

// ---------------------------------------------------------------------------
// mapping to FirestoreDocument
// ---------------------------------------------------------------------------

/// Decode a record payload as a legacy `EntityProto` and map it to a document.
///
/// Returns `None` when the payload is not an `EntityProto` — either it does not
/// decode, or it carries no key — which lets the caller fall through to the
/// `datastore.v1.Entity` decoder.
pub fn try_decode_entity_proto_document(
    record_data: &[u8],
    record_index: usize,
) -> Option<FirestoreDocument> {
    let entity = match decode_entity_proto(record_data) {
        Ok(entity) => entity,
        Err(e) => {
            debug!(
                "Record {} ({} bytes) is not a legacy EntityProto: {}",
                record_index,
                record_data.len(),
                e
            );
            return None;
        }
    };

    // A real export always carries the key. Requiring it keeps `datastore.v1`
    // payloads (which use entirely different field numbers) from being
    // mistaken for sparse `EntityProto`s.
    let reference = entity.key.as_ref()?;
    if reference.path.is_empty() {
        debug!(
            "Record {} decoded as EntityProto but has an empty key path",
            record_index
        );
        return None;
    }

    Some(entity_proto_to_document(
        &entity,
        reference,
        record_data.len(),
        record_index,
    ))
}

fn entity_proto_to_document(
    entity: &EntityProto,
    reference: &Reference,
    size_bytes: usize,
    record_index: usize,
) -> FirestoreDocument {
    let key = datastore_key(reference);

    let collection = key
        .kind()
        .filter(|kind| !kind.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let id = match key.path.last() {
        Some(element) => element.id.as_path_segment(),
        None => format!("doc-{}", uuid::Uuid::new_v4()),
    };
    let path = key.path_string();

    FirestoreDocument {
        id,
        collection,
        data: properties_to_json(entity, record_index),
        subcollections: Vec::new(),
        metadata: DocumentMetadata {
            created_at: None,
            updated_at: None,
            path,
            size_bytes: Some(size_bytes as u64),
        },
        key: Some(key),
    }
}

/// Lift a legacy `Reference` into the shared `DatastoreKey` identity, keeping
/// the namespace and every ancestor element.
fn datastore_key(reference: &Reference) -> DatastoreKey {
    DatastoreKey {
        namespace: reference.name_space.clone(),
        path: reference
            .path
            .iter()
            .map(|element| DatastoreKeyPathElement {
                kind: element.kind.clone(),
                id: match (&element.name, element.id) {
                    (Some(name), _) => DatastoreKeyId::Name(name.clone()),
                    (None, Some(id)) => DatastoreKeyId::Id(id),
                    (None, None) => DatastoreKeyId::Name(String::new()),
                },
            })
            .collect(),
    }
}

/// Collapse `property` + `raw_property` into document fields.
///
/// Arrays are *repeated* `Property` entries that share a name and carry
/// `multiple = true`; taking the last one would silently drop every other
/// element, so same-named entries are collected in wire order instead.
fn properties_to_json(entity: &EntityProto, record_index: usize) -> HashMap<String, JsonValue> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<&Property>> = HashMap::new();

    for property in entity.properties.iter().chain(entity.raw_properties.iter()) {
        let entry = grouped.entry(property.name.clone()).or_insert_with(|| {
            order.push(property.name.clone());
            Vec::new()
        });
        entry.push(property);
    }

    let mut data = HashMap::with_capacity(order.len());
    for name in order {
        let properties = grouped.remove(&name).unwrap_or_default();
        let is_array = properties.len() > 1 || properties.iter().any(|p| p.multiple);
        let value = if is_array {
            JsonValue::Array(
                properties
                    .iter()
                    .map(|p| property_to_json(p, record_index, 0))
                    .collect(),
            )
        } else {
            properties
                .first()
                .map(|p| property_to_json(p, record_index, 0))
                .unwrap_or(JsonValue::Null)
        };
        data.insert(name, value);
    }

    data
}

/// Convert one property, honouring `meaning` over the raw value field.
fn property_to_json(property: &Property, record_index: usize, depth: usize) -> JsonValue {
    let value = &property.value;

    match property.meaning {
        // An empty array is encoded as a meaning with no value at all.
        MEANING_EMPTY_LIST => return JsonValue::Array(Vec::new()),
        MEANING_GD_WHEN => {
            if let Some(micros) = value.int64_value {
                return micros_to_rfc3339(micros, &property.name, record_index);
            }
        }
        MEANING_GEORSS_POINT => {
            if let Some(point) = value.point_value.as_ref() {
                return geo_point_to_json(point);
            }
        }
        MEANING_BLOB | MEANING_BYTESTRING => {
            if let Some(bytes) = value.string_value.as_ref() {
                // Blobs are raw bytes, not UTF-8 — base64 keeps them intact.
                return tagged_blob(base64_encode(bytes));
            }
        }
        MEANING_TEXT => {
            if let Some(bytes) = value.string_value.as_ref() {
                return JsonValue::String(String::from_utf8_lossy(bytes).into_owned());
            }
        }
        MEANING_ENTITY_PROTO => {
            if let Some(bytes) = value.string_value.as_ref() {
                return nested_entity_to_json(bytes, &property.name, record_index, depth);
            }
        }
        _ => {}
    }

    scalar_to_json(value, &property.name, property.meaning, record_index)
}

/// Fall back to the value field when `meaning` does not dictate a reading.
fn scalar_to_json(
    value: &PropertyValue,
    property_name: &str,
    meaning: i32,
    record_index: usize,
) -> JsonValue {
    if let Some(int_value) = value.int64_value {
        return JsonValue::Number(int_value.into());
    }
    if let Some(bool_value) = value.boolean_value {
        return JsonValue::Bool(bool_value);
    }
    if let Some(bytes) = value.string_value.as_ref() {
        return match std::str::from_utf8(bytes) {
            Ok(text) => JsonValue::String(text.to_string()),
            // Non-UTF-8 without a BLOB meaning: keep the bytes rather than
            // producing mojibake.
            Err(_) => tagged_blob(base64_encode(bytes)),
        };
    }
    if let Some(double_value) = value.double_value {
        return match Number::from_f64(double_value) {
            Some(number) => JsonValue::Number(number),
            // JSON has no literal for NaN/±Infinity, but Datastore stores them
            // and PostgreSQL accepts them, so tag rather than drop.
            None => {
                debug!(
                    "Record {}: property '{}' holds the non-finite double {}; tagging it",
                    record_index, property_name, double_value
                );
                tagged_non_finite_double(double_value).unwrap_or(JsonValue::Null)
            }
        };
    }
    if let Some(point) = value.point_value.as_ref() {
        return geo_point_to_json(point);
    }
    if let Some(user) = value.user_value.as_ref() {
        let mut map = Map::new();
        map.insert("email".to_string(), JsonValue::String(user.email.clone()));
        map.insert(
            "auth_domain".to_string(),
            JsonValue::String(user.auth_domain.clone()),
        );
        return JsonValue::Object(map);
    }
    if let Some(reference) = value.reference_value.as_ref() {
        return tagged_key_reference(datastore_key(reference).encoded_path());
    }

    if meaning != MEANING_NO_MEANING && meaning != MEANING_EMPTY_LIST {
        debug!(
            "Record {}: property '{}' has meaning {} but no value; storing null",
            record_index, property_name, meaning
        );
    }
    JsonValue::Null
}

/// A `PointValue` is a geo point: `x` is latitude, `y` is longitude.
fn geo_point_to_json(point: &PointValue) -> JsonValue {
    tagged_geo_point(point.x, point.y)
}

/// `GD_WHEN` is microseconds since the Unix epoch, not seconds.
fn micros_to_rfc3339(micros: i64, property_name: &str, record_index: usize) -> JsonValue {
    let seconds = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1_000) as u32;
    match chrono::DateTime::from_timestamp(seconds, nanos) {
        Some(datetime) => tagged_timestamp(datetime.to_rfc3339()),
        None => {
            warn!(
                "Record {}: property '{}' has an out-of-range GD_WHEN value ({} µs); keeping the raw integer",
                record_index, property_name, micros
            );
            JsonValue::Number(micros.into())
        }
    }
}

/// `ENTITY_PROTO` values hold another serialized `EntityProto` — a nested map.
fn nested_entity_to_json(
    bytes: &[u8],
    property_name: &str,
    record_index: usize,
    depth: usize,
) -> JsonValue {
    if depth >= MAX_NESTING_DEPTH {
        warn!(
            "Record {}: property '{}' nests ENTITY_PROTO beyond {} levels; storing base64",
            record_index, property_name, MAX_NESTING_DEPTH
        );
        return tagged_blob(base64_encode(bytes));
    }

    let nested = match decode_entity_proto(bytes) {
        Ok(nested) => nested,
        Err(e) => {
            warn!(
                "Record {}: property '{}' is marked ENTITY_PROTO but did not decode ({}); storing base64",
                record_index, property_name, e
            );
            return tagged_blob(base64_encode(bytes));
        }
    };

    let mut map = Map::new();
    for (name, value) in nested_properties_to_json(&nested, record_index, depth + 1) {
        map.insert(name, value);
    }
    JsonValue::Object(map)
}

fn nested_properties_to_json(
    entity: &EntityProto,
    record_index: usize,
    depth: usize,
) -> Vec<(String, JsonValue)> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<&Property>> = HashMap::new();

    for property in entity.properties.iter().chain(entity.raw_properties.iter()) {
        let entry = grouped.entry(property.name.clone()).or_insert_with(|| {
            order.push(property.name.clone());
            Vec::new()
        });
        entry.push(property);
    }

    order
        .into_iter()
        .map(|name| {
            let properties = grouped.remove(&name).unwrap_or_default();
            let is_array = properties.len() > 1 || properties.iter().any(|p| p.multiple);
            let value = if is_array {
                JsonValue::Array(
                    properties
                        .iter()
                        .map(|p| property_to_json(p, record_index, depth))
                        .collect(),
                )
            } else {
                properties
                    .first()
                    .map(|p| property_to_json(p, record_index, depth))
                    .unwrap_or(JsonValue::Null)
            };
            (name, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- wire-format builders (mirror tests/fixtures/generate_datastore_export.py)

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn tag(field: u32, wire: u8) -> Vec<u8> {
        varint(((field as u64) << 3) | wire as u64)
    }

    fn f_varint(field: u32, value: i64) -> Vec<u8> {
        let mut out = tag(field, WIRE_VARINT);
        out.extend(varint(value as u64));
        out
    }

    fn f_double(field: u32, value: f64) -> Vec<u8> {
        let mut out = tag(field, WIRE_FIXED64);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn f_bytes(field: u32, value: &[u8]) -> Vec<u8> {
        let mut out = tag(field, WIRE_LEN);
        out.extend(varint(value.len() as u64));
        out.extend_from_slice(value);
        out
    }

    fn f_string(field: u32, value: &str) -> Vec<u8> {
        f_bytes(field, value.as_bytes())
    }

    fn f_group(field: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = tag(field, WIRE_START_GROUP);
        out.extend_from_slice(payload);
        out.extend(tag(field, WIRE_END_GROUP));
        out
    }

    enum Id {
        Name(&'static str),
        Num(i64),
    }

    fn path(elements: &[(&str, &Id)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, id) in elements {
            let mut element = f_string(2, kind);
            match id {
                Id::Name(name) => element.extend(f_string(4, name)),
                Id::Num(value) => element.extend(f_varint(3, *value)),
            }
            out.extend(f_group(1, &element));
        }
        out
    }

    fn reference(elements: &[(&str, &Id)]) -> Vec<u8> {
        let mut out = f_string(13, "s~fireup-test");
        out.extend(f_bytes(14, &path(elements)));
        out
    }

    fn property(name: &str, value: &[u8], meaning: i32, multiple: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if meaning != MEANING_NO_MEANING {
            out.extend(f_varint(1, meaning as i64));
        }
        out.extend(f_string(3, name));
        out.extend(f_varint(4, if multiple { 1 } else { 0 }));
        out.extend(f_bytes(5, value));
        out
    }

    fn entity(elements: &[(&str, &Id)], properties: &[Vec<u8>]) -> Vec<u8> {
        let mut out = f_bytes(13, &reference(elements));
        for prop in properties {
            out.extend(f_bytes(14, prop));
        }
        out
    }

    #[test]
    fn decodes_a_key_with_name_and_numeric_elements() {
        let bytes = entity(
            &[("Users", &Id::Name("ada")), ("Orders", &Id::Num(1001))],
            &[],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");

        assert_eq!(document.collection, "Orders");
        assert_eq!(document.id, "1001");
        assert_eq!(document.metadata.path, "Users/ada/Orders/1001");

        // The ancestor path is the entity's identity, so it must survive whole.
        let key = document.key.expect("datastore key");
        assert_eq!(key.encoded_path(), "Users/ada/Orders/1001");
        assert_eq!(key.parent_encoded_path().as_deref(), Some("Users/ada"));
        assert_eq!(key.numeric_id(), Some(1001));
    }

    #[test]
    fn a_namespaced_key_keeps_its_namespace() {
        let mut bytes = {
            let mut reference_bytes = f_string(13, "s~fireup-test");
            reference_bytes.extend(f_bytes(14, &path(&[("Users", &Id::Name("ada"))])));
            reference_bytes.extend(f_string(20, "tenant-7"));
            f_bytes(13, &reference_bytes)
        };
        bytes.extend(f_bytes(14, &property("age", &f_varint(1, 36), 0, false)));

        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        let key = document.key.expect("datastore key");
        assert_eq!(key.namespace, "tenant-7");
        assert_eq!(key.encoded_path(), "[tenant-7]Users/ada");
    }

    #[test]
    fn decodes_a_numeric_key_beyond_int32() {
        let bytes = entity(&[("Users", &Id::Num(4_503_599_627_370_496))], &[]);
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(document.id, "4503599627370496");
    }

    #[test]
    fn gd_when_becomes_a_timestamp_in_microseconds() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property(
                "created_at",
                &f_varint(1, 1_773_480_413_589_793),
                MEANING_GD_WHEN,
                false,
            )],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        // Read as seconds this would land in the year 58 million.
        assert_eq!(
            document.data.get("created_at"),
            Some(&tagged_timestamp("2026-03-14T09:26:53.589793+00:00".to_string()))
        );
    }

    #[test]
    fn gd_when_handles_pre_epoch_microseconds() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property(
                "created_at",
                &f_varint(1, -1_500_000),
                MEANING_GD_WHEN,
                false,
            )],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("created_at"),
            Some(&tagged_timestamp("1969-12-31T23:59:58.500+00:00".to_string()))
        );
    }

    #[test]
    fn blob_values_keep_their_bytes() {
        let raw: Vec<u8> = (0u8..=255).collect();
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("avatar", &f_bytes(3, &raw), MEANING_BLOB, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");

        let value = document.data.get("avatar").expect("avatar");
        assert_eq!(value, &tagged_blob(base64_encode(&raw)));
        // A lossy UTF-8 read would have replaced the non-UTF-8 bytes.
        let encoded = value.get("base64").and_then(JsonValue::as_str).expect("base64");
        assert!(!encoded.contains('\u{FFFD}'));
    }

    #[test]
    fn non_utf8_strings_without_a_blob_meaning_keep_their_bytes() {
        let raw = vec![0xFF, 0xFE, 0x00, 0x41];
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("legacy", &f_bytes(3, &raw), MEANING_NO_MEANING, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("legacy"),
            Some(&tagged_blob(base64_encode(&raw)))
        );
    }

    #[test]
    fn text_values_decode_as_utf8() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property(
                "bio",
                &f_bytes(3, "絵文字 🔥 included".as_bytes()),
                MEANING_TEXT,
                false,
            )],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("bio"),
            Some(&JsonValue::String("絵文字 🔥 included".to_string()))
        );
    }

    #[test]
    fn repeated_properties_sharing_a_name_become_an_array() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[
                property("tags", &f_bytes(3, b"math"), MEANING_TEXT, true),
                property("tags", &f_bytes(3, b"computing"), MEANING_TEXT, true),
            ],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("tags"),
            Some(&JsonValue::Array(vec![
                JsonValue::String("math".to_string()),
                JsonValue::String("computing".to_string()),
            ]))
        );
    }

    #[test]
    fn a_single_multiple_property_is_still_an_array() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("tags", &f_bytes(3, b"solo"), MEANING_TEXT, true)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("tags"),
            Some(&JsonValue::Array(vec![JsonValue::String("solo".to_string())]))
        );
    }

    #[test]
    fn empty_list_meaning_becomes_an_empty_array() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("tags", &[], MEANING_EMPTY_LIST, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(document.data.get("tags"), Some(&JsonValue::Array(Vec::new())));
    }

    #[test]
    fn a_value_with_no_field_set_is_null() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("nickname", &[], MEANING_NO_MEANING, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(document.data.get("nickname"), Some(&JsonValue::Null));
    }

    #[test]
    fn nested_entity_proto_becomes_a_map() {
        let nested = entity(
            &[("Users", &Id::Name("ada")), ("Address", &Id::Num(1))],
            &[
                property("city", &f_bytes(3, "London".as_bytes()), MEANING_TEXT, false),
                property("verified", &f_varint(2, 1), MEANING_NO_MEANING, false),
            ],
        );
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property(
                "address",
                &f_bytes(3, &nested),
                MEANING_ENTITY_PROTO,
                false,
            )],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");

        let JsonValue::Object(map) = document.data.get("address").expect("address") else {
            panic!("ENTITY_PROTO must decode to an object");
        };
        assert_eq!(map.get("city"), Some(&JsonValue::String("London".to_string())));
        assert_eq!(map.get("verified"), Some(&JsonValue::Bool(true)));
    }

    #[test]
    fn point_groups_decode_as_lat_lng() {
        let point = {
            let mut body = f_double(6, 51.5074);
            body.extend(f_double(7, -0.1278));
            f_group(5, &body)
        };
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[property("location", &point, MEANING_GEORSS_POINT, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");

        let JsonValue::Object(map) = document.data.get("location").expect("location") else {
            panic!("GEORSS_POINT must decode to an object");
        };
        assert_eq!(map.get("latitude").and_then(JsonValue::as_f64), Some(51.5074));
        assert_eq!(map.get("longitude").and_then(JsonValue::as_f64), Some(-0.1278));
    }

    #[test]
    fn reference_value_groups_decode_as_a_key_path() {
        let reference_value = {
            let mut body = f_string(13, "s~fireup-test");
            let mut element = f_string(15, "Users");
            element.extend(f_string(17, "ada"));
            body.extend(f_group(14, &element));
            f_group(12, &body)
        };
        let bytes = entity(
            &[("Users", &Id::Name("grace"))],
            &[property("manager", &reference_value, MEANING_NO_MEANING, false)],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("manager"),
            Some(&tagged_key_reference("Users/ada".to_string()))
        );
    }

    #[test]
    fn negative_scalars_round_trip() {
        let bytes = entity(
            &[("Users", &Id::Name("ada"))],
            &[
                property("age", &f_varint(1, -1), MEANING_NO_MEANING, false),
                property("score", &f_double(4, -12.5), MEANING_NO_MEANING, false),
            ],
        );
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(
            document.data.get("age").and_then(JsonValue::as_i64),
            Some(-1)
        );
        assert_eq!(
            document.data.get("score").and_then(JsonValue::as_f64),
            Some(-12.5)
        );
    }

    #[test]
    fn unknown_fields_and_groups_are_skipped() {
        let mut bytes = entity(&[("Users", &Id::Name("ada"))], &[]);
        bytes.extend(f_group(99, &f_string(1, "future field")));
        bytes.extend(f_varint(98, 7));
        let document = try_decode_entity_proto_document(&bytes, 0).expect("document");
        assert_eq!(document.id, "ada");
    }

    #[test]
    fn a_datastore_v1_entity_is_not_mistaken_for_an_entity_proto() {
        // datastore.v1.Entity: key = 1, properties = 3. No field 13, so there is
        // no EntityProto key and the caller must fall through to prost.
        let mut v1 = f_bytes(1, &f_bytes(2, &f_string(1, "Users")));
        v1.extend(f_bytes(3, &f_string(1, "name")));
        assert!(try_decode_entity_proto_document(&v1, 0).is_none());
    }

    #[test]
    fn non_protobuf_bytes_are_rejected() {
        assert!(try_decode_entity_proto_document(b"{not-protobuf", 0).is_none());
        assert!(try_decode_entity_proto_document(&[], 0).is_none());
    }

    #[test]
    fn an_unterminated_group_is_an_error() {
        let mut bytes = f_bytes(13, &reference(&[("Users", &Id::Name("ada"))]));
        bytes.extend(tag(1, WIRE_START_GROUP)); // opened, never closed
        assert!(decode_entity_proto(&bytes).is_err() || try_decode_entity_proto_document(&bytes, 0).is_none());
    }

    #[test]
    fn truncated_length_delimited_fields_are_an_error() {
        let bytes = [tag(13, WIRE_LEN), varint(50), vec![0u8; 3]].concat();
        assert!(decode_entity_proto(&bytes).is_err());
    }
}
