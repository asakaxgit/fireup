//! Firestore `Value` ⇄ JSON, in the shape the public Firestore REST API uses.
//!
//! The durable store keeps a document's fields as a JSONB payload, so this
//! mapping is the format of data at rest. Two properties matter more than
//! anything else here:
//!
//! 1. **Round-trip exactness.** `decode(encode(v)) == v` for every value an
//!    SDK can store. A lossy mapping would silently corrupt data on restart,
//!    which is the one failure mode a durable store must not have.
//! 2. **No silent fallbacks.** A value this module cannot represent is an
//!    error, never a dropped field or a coerced type.
//!
//! The two are in tension for a value `encode` can write but `decode` cannot
//! read, so the set of storable values is pinned down on the **encode** side:
//! `encode` refuses anything outside it (property 2) rather than writing a
//! payload the read path then rejects, which is what makes property 1 hold as
//! stated. `decode` is deliberately the wider of the two — it also reads back
//! the payloads a pre-SAK-205 build wrote, so a database that already holds
//! one stays readable; see [`decode_timestamp`].
//!
//! The wire shape is the documented REST encoding of `google.firestore.v1.Value`
//! (`{"stringValue": "x"}`, `{"integerValue": "7"}`, …), chosen over an opaque
//! protobuf blob so the JSONB is legible to SQL and usable by the query engine
//! that lands in a later phase.
//!
//! Field names are the JSON names from the public `document.proto`; see
//! `docs/licenses/FIREBASE_GCP_LOCAL.md`.

use std::collections::HashMap;

use prost_types::Timestamp;
use serde_json::{Map, Number, Value as Json};

use crate::leveldb_parser::base64;
use crate::protos::google::firestore::v1::{value::ValueType, ArrayValue, MapValue, Value};
use crate::protos::google::r#type::LatLng;

/// A value or payload that cannot be mapped in one direction or the other.
#[derive(Debug, thiserror::Error)]
pub enum ValueCodecError {
    /// A `Value` variant that documents cannot contain (pipeline expression
    /// values). Encoding these would invent a storage format for something the
    /// store should never be handed.
    #[error("{variant} cannot be stored in a document")]
    Unstorable { variant: &'static str },

    /// A timestamp outside years 0001–9999, which is the range both
    /// `google.protobuf.Timestamp` and Firestore are specified for.
    ///
    /// Its own variant rather than a [`ValueCodecError::Malformed`] because it
    /// is the one encode failure a client can cause with an otherwise
    /// well-formed value, and the message has to name the range rather than
    /// leave an SDK guessing why its write was refused.
    #[error(
        "timestampValue {seconds}s {nanos}ns is outside years 0001-9999, \
         the range a Firestore timestamp can hold"
    )]
    TimestampOutOfRange { seconds: i64, nanos: i32 },

    /// Structurally wrong JSON: not an object, unknown tag, wrong payload type.
    #[error("{0}")]
    Malformed(String),
}

type Result<T> = std::result::Result<T, ValueCodecError>;

fn malformed(message: impl Into<String>) -> ValueCodecError {
    ValueCodecError::Malformed(message.into())
}

/// JSON tag names, kept in one place so encode and decode cannot drift.
mod tag {
    pub const NULL: &str = "nullValue";
    pub const BOOLEAN: &str = "booleanValue";
    pub const INTEGER: &str = "integerValue";
    pub const DOUBLE: &str = "doubleValue";
    pub const TIMESTAMP: &str = "timestampValue";
    pub const STRING: &str = "stringValue";
    pub const BYTES: &str = "bytesValue";
    pub const REFERENCE: &str = "referenceValue";
    pub const GEO_POINT: &str = "geoPointValue";
    pub const ARRAY: &str = "arrayValue";
    pub const MAP: &str = "mapValue";
}

/// Encode a document's whole field map.
pub fn encode_fields(fields: &HashMap<String, Value>) -> Result<Json> {
    let mut object = Map::with_capacity(fields.len());
    for (name, value) in fields {
        object.insert(name.clone(), encode(value)?);
    }
    Ok(Json::Object(object))
}

/// Decode a document's whole field map.
pub fn decode_fields(json: &Json) -> Result<HashMap<String, Value>> {
    let object = json
        .as_object()
        .ok_or_else(|| malformed("a document's fields payload must be a JSON object"))?;

    let mut fields = HashMap::with_capacity(object.len());
    for (name, value) in object {
        fields.insert(name.clone(), decode(value)?);
    }
    Ok(fields)
}

/// Encode a single `Value`.
pub fn encode(value: &Value) -> Result<Json> {
    let value_type = value
        .value_type
        .as_ref()
        .ok_or_else(|| malformed("Value has no value_type set"))?;

    let (tag, payload) = match value_type {
        // The proto carries the `google.protobuf.NullValue` enum, whose only
        // member is 0. Anything else is a value we did not produce.
        ValueType::NullValue(code) => {
            if *code != 0 {
                return Err(malformed(format!("nullValue has unexpected enum code {code}")));
            }
            (tag::NULL, Json::Null)
        }
        ValueType::BooleanValue(flag) => (tag::BOOLEAN, Json::Bool(*flag)),
        // int64 goes out as a string: JSON numbers lose precision past 2^53 and
        // the REST API encodes it the same way.
        ValueType::IntegerValue(number) => (tag::INTEGER, Json::String(number.to_string())),
        ValueType::DoubleValue(number) => (tag::DOUBLE, encode_double(*number)),
        ValueType::TimestampValue(timestamp) => {
            (tag::TIMESTAMP, Json::String(encode_timestamp(timestamp)?))
        }
        ValueType::StringValue(text) => (tag::STRING, Json::String(text.clone())),
        ValueType::BytesValue(bytes) => (tag::BYTES, Json::String(base64::encode(bytes))),
        ValueType::ReferenceValue(name) => (tag::REFERENCE, Json::String(name.clone())),
        ValueType::GeoPointValue(point) => (tag::GEO_POINT, encode_geo_point(point)),
        ValueType::ArrayValue(array) => {
            let values = array
                .values
                .iter()
                .map(encode)
                .collect::<Result<Vec<Json>>>()?;
            let mut object = Map::new();
            object.insert("values".to_string(), Json::Array(values));
            (tag::ARRAY, Json::Object(object))
        }
        ValueType::MapValue(map) => {
            let mut object = Map::new();
            object.insert("fields".to_string(), encode_fields(&map.fields)?);
            (tag::MAP, Json::Object(object))
        }
        ValueType::FieldReferenceValue(_) => {
            return Err(ValueCodecError::Unstorable {
                variant: "fieldReferenceValue",
            })
        }
        ValueType::VariableReferenceValue(_) => {
            return Err(ValueCodecError::Unstorable {
                variant: "variableReferenceValue",
            })
        }
        ValueType::FunctionValue(_) => {
            return Err(ValueCodecError::Unstorable {
                variant: "functionValue",
            })
        }
        ValueType::PipelineValue(_) => {
            return Err(ValueCodecError::Unstorable {
                variant: "pipelineValue",
            })
        }
    };

    let mut wrapper = Map::with_capacity(1);
    wrapper.insert(tag.to_string(), payload);
    Ok(Json::Object(wrapper))
}

/// Decode a single `Value`.
pub fn decode(json: &Json) -> Result<Value> {
    let object = json
        .as_object()
        .ok_or_else(|| malformed(format!("expected a tagged Value object, got {json}")))?;

    // Exactly one tag: a multi-tag object is ambiguous and a zero-tag object
    // carries no value, so neither can be decoded without guessing.
    if object.len() != 1 {
        return Err(malformed(format!(
            "a Value object must have exactly one type tag, found {}",
            object.len()
        )));
    }
    let (tag, payload) = object.iter().next().expect("length checked above");

    let value_type = match tag.as_str() {
        tag::NULL => {
            if !payload.is_null() {
                return Err(malformed(format!("{} must be null, got {payload}", tag::NULL)));
            }
            ValueType::NullValue(0)
        }
        tag::BOOLEAN => ValueType::BooleanValue(
            payload
                .as_bool()
                .ok_or_else(|| malformed(format!("{} must be a boolean", tag::BOOLEAN)))?,
        ),
        tag::INTEGER => ValueType::IntegerValue(decode_integer(payload)?),
        tag::DOUBLE => ValueType::DoubleValue(decode_double(payload)?),
        tag::TIMESTAMP => ValueType::TimestampValue(decode_timestamp(payload)?),
        tag::STRING => ValueType::StringValue(
            payload
                .as_str()
                .ok_or_else(|| malformed(format!("{} must be a string", tag::STRING)))?
                .to_string(),
        ),
        tag::BYTES => {
            let encoded = payload
                .as_str()
                .ok_or_else(|| malformed(format!("{} must be a base64 string", tag::BYTES)))?;
            ValueType::BytesValue(
                base64::decode(encoded)
                    .map_err(|error| malformed(format!("{} is not valid base64: {error}", tag::BYTES)))?,
            )
        }
        tag::REFERENCE => ValueType::ReferenceValue(
            payload
                .as_str()
                .ok_or_else(|| malformed(format!("{} must be a string", tag::REFERENCE)))?
                .to_string(),
        ),
        tag::GEO_POINT => ValueType::GeoPointValue(decode_geo_point(payload)?),
        tag::ARRAY => {
            let values = match payload.get("values") {
                // An empty array is encoded with an absent/null `values`, which
                // is how proto3 JSON drops empty repeated fields.
                None | Some(Json::Null) => Vec::new(),
                Some(Json::Array(items)) => items.iter().map(decode).collect::<Result<Vec<_>>>()?,
                Some(other) => {
                    return Err(malformed(format!(
                        "{}.values must be an array, got {other}",
                        tag::ARRAY
                    )))
                }
            };
            ValueType::ArrayValue(ArrayValue { values })
        }
        tag::MAP => {
            let fields = match payload.get("fields") {
                None | Some(Json::Null) => HashMap::new(),
                Some(object @ Json::Object(_)) => decode_fields(object)?,
                Some(other) => {
                    return Err(malformed(format!(
                        "{}.fields must be an object, got {other}",
                        tag::MAP
                    )))
                }
            };
            ValueType::MapValue(MapValue { fields })
        }
        unknown => {
            return Err(malformed(format!(
                "unknown Value type tag {unknown:?}; expected one of \
                 nullValue, booleanValue, integerValue, doubleValue, timestampValue, \
                 stringValue, bytesValue, referenceValue, geoPointValue, arrayValue, mapValue"
            )))
        }
    };

    Ok(Value {
        value_type: Some(value_type),
    })
}

/// Doubles go out as JSON numbers, except the three values JSON has no number
/// for. Proto3 JSON spells those as strings, so we do too.
fn encode_double(number: f64) -> Json {
    match Number::from_f64(number) {
        Some(finite) => Json::Number(finite),
        None if number.is_nan() => Json::String("NaN".to_string()),
        None if number.is_sign_positive() => Json::String("Infinity".to_string()),
        None => Json::String("-Infinity".to_string()),
    }
}

fn decode_double(payload: &Json) -> Result<f64> {
    match payload {
        Json::Number(number) => number
            .as_f64()
            .ok_or_else(|| malformed(format!("{} {number} is not representable as f64", tag::DOUBLE))),
        // Accept the string spellings both ways round, and also accept a plain
        // numeric string: some clients stringify every number.
        Json::String(text) => match text.as_str() {
            "NaN" => Ok(f64::NAN),
            "Infinity" => Ok(f64::INFINITY),
            "-Infinity" => Ok(f64::NEG_INFINITY),
            other => other
                .parse::<f64>()
                .map_err(|_| malformed(format!("{} {other:?} is not a number", tag::DOUBLE))),
        },
        other => Err(malformed(format!(
            "{} must be a number, got {other}",
            tag::DOUBLE
        ))),
    }
}

/// Integers are written as strings; accept a JSON number too, but only when it
/// is an exact integer, so a lossy float never becomes an int64 silently.
fn decode_integer(payload: &Json) -> Result<i64> {
    match payload {
        Json::String(text) => text
            .parse::<i64>()
            .map_err(|_| malformed(format!("{} {text:?} is not an int64", tag::INTEGER))),
        Json::Number(number) => number
            .as_i64()
            .ok_or_else(|| malformed(format!("{} {number} is not an exact int64", tag::INTEGER))),
        other => Err(malformed(format!(
            "{} must be a string or integer, got {other}",
            tag::INTEGER
        ))),
    }
}

/// `0001-01-01T00:00:00Z`, the first instant a Firestore timestamp can hold.
const TIMESTAMP_MIN_SECONDS: i64 = -62_135_596_800;

/// `9999-12-31T23:59:59Z`, the last whole second one can hold.
const TIMESTAMP_MAX_SECONDS: i64 = 253_402_300_799;

/// The fixed width of an encoded timestamp: `0000-00-00T00:00:00.000000000Z`.
///
/// Public because the SQL pushdown compares stored timestamps as text and
/// relies on every one of them being exactly this wide; see
/// [`postgres_filter`](crate::local::postgres_filter).
pub const ENCODED_TIMESTAMP_LEN: usize = 30;

/// RFC 3339 with nanosecond precision and a `Z` offset, as the REST API uses.
///
/// Refuses a timestamp outside years 0001–9999. That range is not ours: it is
/// what `google.protobuf.Timestamp` is specified for and what Firestore
/// documents, so a value outside it is one no SDK could have obtained from the
/// real service. Refusing it here fails the **write**, which is the error the
/// client can act on. The alternative — chrono's `+11476-08-15T…Z`, with a
/// signed five-digit year — is not RFC 3339, is not what the REST API would
/// ever emit, and is not the fixed width the pushdown's text comparison needs;
/// writing one would succeed and then break every later read of the whole
/// collection (SAK-205).
fn encode_timestamp(timestamp: &Timestamp) -> Result<String> {
    let out_of_range = || ValueCodecError::TimestampOutOfRange {
        seconds: timestamp.seconds,
        nanos: timestamp.nanos,
    };

    if !(TIMESTAMP_MIN_SECONDS..=TIMESTAMP_MAX_SECONDS).contains(&timestamp.seconds)
        // proto3 requires `0 <= nanos <= 999,999,999`; a negative one would
        // otherwise widen into a nonsense `u32` below.
        || !(0..=999_999_999).contains(&timestamp.nanos)
    {
        return Err(out_of_range());
    }

    let moment = chrono::DateTime::from_timestamp(timestamp.seconds, timestamp.nanos as u32)
        .ok_or_else(out_of_range)?;
    let text = moment.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    debug_assert_eq!(
        text.len(),
        ENCODED_TIMESTAMP_LEN,
        "an in-range timestamp must encode to the fixed width the pushdown assumes"
    );
    Ok(text)
}

/// Read a timestamp back, including the one shape `encode` no longer writes.
///
/// A build before SAK-205 encoded an out-of-range timestamp as chrono's signed
/// expanded year (`+11476-08-15T05:20:00.000000000Z`), which
/// `parse_from_rfc3339` rejects — and because a collection scan decodes every
/// row it reads, one such row made the entire collection unreadable. Those
/// rows are already in databases, so they are read back exactly rather than
/// left as permanent poison. This is the one place the read set is wider than
/// the write set, and it is wider by exactly the payloads this module used to
/// produce: nothing is guessed at or coerced, and a value that comes back this
/// way is refused the next time anything tries to store it.
fn decode_timestamp(payload: &Json) -> Result<Timestamp> {
    let text = payload
        .as_str()
        .ok_or_else(|| malformed(format!("{} must be an RFC 3339 string", tag::TIMESTAMP)))?;

    let moment = match chrono::DateTime::parse_from_rfc3339(text) {
        Ok(moment) => moment.with_timezone(&chrono::Utc),
        Err(error) => decode_expanded_year_timestamp(text).ok_or_else(|| {
            malformed(format!(
                "{} {text:?} is not RFC 3339: {error}",
                tag::TIMESTAMP
            ))
        })?,
    };

    Ok(Timestamp {
        seconds: moment.timestamp(),
        nanos: moment.timestamp_subsec_nanos() as i32,
    })
}

/// The pre-SAK-205 spelling only: a leading `+`/`-` sign on the year, a `Z`
/// offset, and otherwise the same fields. Anything else is not something this
/// module ever wrote, so it stays an error.
fn decode_expanded_year_timestamp(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if !text.starts_with('+') && !text.starts_with('-') {
        return None;
    }
    let naive = text.strip_suffix('Z')?;
    chrono::NaiveDateTime::parse_from_str(naive, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc())
}

fn encode_geo_point(point: &LatLng) -> Json {
    let mut object = Map::with_capacity(2);
    object.insert("latitude".to_string(), encode_double(point.latitude));
    object.insert("longitude".to_string(), encode_double(point.longitude));
    Json::Object(object)
}

fn decode_geo_point(payload: &Json) -> Result<LatLng> {
    let object = payload.as_object().ok_or_else(|| {
        malformed(format!(
            "{} must be an object with latitude and longitude",
            tag::GEO_POINT
        ))
    })?;

    // A missing coordinate means 0 in proto3 JSON, which for a geo point is a
    // real location (null island) rather than "unset" — so it round-trips.
    let coordinate = |key: &str| -> Result<f64> {
        match object.get(key) {
            None | Some(Json::Null) => Ok(0.0),
            Some(value) => decode_double(value),
        }
    };

    Ok(LatLng {
        latitude: coordinate("latitude")?,
        longitude: coordinate("longitude")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(value_type: ValueType) -> Value {
        Value {
            value_type: Some(value_type),
        }
    }

    /// Every value type an SDK can store, in one array, so the round-trip test
    /// covers the whole surface rather than a chosen-by-hand subset.
    fn every_storable_value() -> Vec<Value> {
        vec![
            value(ValueType::NullValue(0)),
            value(ValueType::BooleanValue(true)),
            value(ValueType::BooleanValue(false)),
            value(ValueType::IntegerValue(0)),
            value(ValueType::IntegerValue(-1)),
            value(ValueType::IntegerValue(i64::MAX)),
            value(ValueType::IntegerValue(i64::MIN)),
            value(ValueType::DoubleValue(0.0)),
            value(ValueType::DoubleValue(-0.5)),
            value(ValueType::DoubleValue(f64::MAX)),
            value(ValueType::DoubleValue(f64::MIN_POSITIVE)),
            value(ValueType::TimestampValue(Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            })),
            value(ValueType::TimestampValue(Timestamp {
                seconds: 0,
                nanos: 0,
            })),
            value(ValueType::TimestampValue(Timestamp {
                seconds: -1,
                nanos: 1,
            })),
            // Both ends of the range, which is where an off-by-one in the
            // check added for SAK-205 would show up.
            value(ValueType::TimestampValue(Timestamp {
                seconds: TIMESTAMP_MIN_SECONDS,
                nanos: 0,
            })),
            value(ValueType::TimestampValue(Timestamp {
                seconds: TIMESTAMP_MAX_SECONDS,
                nanos: 999_999_999,
            })),
            value(ValueType::StringValue(String::new())),
            value(ValueType::StringValue("hello".to_string())),
            value(ValueType::StringValue("日本語 🎏 \" \\ \n".to_string())),
            value(ValueType::BytesValue(Vec::new())),
            value(ValueType::BytesValue(vec![0, 1, 2, 253, 254, 255])),
            value(ValueType::ReferenceValue(
                "projects/p/databases/(default)/documents/users/alice".to_string(),
            )),
            value(ValueType::GeoPointValue(LatLng {
                latitude: 35.6812,
                longitude: 139.7671,
            })),
            value(ValueType::GeoPointValue(LatLng {
                latitude: 0.0,
                longitude: 0.0,
            })),
            value(ValueType::ArrayValue(ArrayValue { values: Vec::new() })),
            value(ValueType::ArrayValue(ArrayValue {
                values: vec![
                    value(ValueType::IntegerValue(1)),
                    value(ValueType::StringValue("two".to_string())),
                    value(ValueType::NullValue(0)),
                ],
            })),
            value(ValueType::MapValue(MapValue {
                fields: HashMap::new(),
            })),
            value(ValueType::MapValue(MapValue {
                fields: [
                    ("nested".to_string(), value(ValueType::BooleanValue(true))),
                    (
                        "deep".to_string(),
                        value(ValueType::MapValue(MapValue {
                            fields: [(
                                "array".to_string(),
                                value(ValueType::ArrayValue(ArrayValue {
                                    values: vec![value(ValueType::DoubleValue(1.5))],
                                })),
                            )]
                            .into_iter()
                            .collect(),
                        })),
                    ),
                ]
                .into_iter()
                .collect(),
            })),
        ]
    }

    #[test]
    fn every_storable_value_round_trips() {
        for original in every_storable_value() {
            let json = encode(&original).expect("storable values encode");
            let decoded = decode(&json).expect("encoded values decode");
            assert_eq!(decoded, original, "round trip changed the value via {json}");
        }
    }

    #[test]
    fn round_trips_through_serialised_text() {
        // The real path goes through Postgres as JSONB text, not as an
        // in-memory serde_json tree.
        for original in every_storable_value() {
            let text = serde_json::to_string(&encode(&original).expect("encode")).expect("serialise");
            let json: Json = serde_json::from_str(&text).expect("parse");
            assert_eq!(decode(&json).expect("decode"), original, "via {text}");
        }
    }

    #[test]
    fn non_finite_doubles_survive_as_the_proto3_json_spellings() {
        for original in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let json = encode(&value(ValueType::DoubleValue(original))).expect("encode");
            let Some(ValueType::DoubleValue(decoded)) =
                decode(&json).expect("decode").value_type
            else {
                panic!("expected a double back, got {json}");
            };
            if original.is_nan() {
                assert!(decoded.is_nan(), "{json}");
            } else {
                assert_eq!(decoded, original, "{json}");
            }
        }
    }

    #[test]
    fn large_integers_keep_every_digit() {
        let json = encode(&value(ValueType::IntegerValue(9_007_199_254_740_993))).expect("encode");
        assert_eq!(json["integerValue"], Json::String("9007199254740993".to_string()));
    }

    #[test]
    fn integer_json_numbers_are_accepted_but_lossy_floats_are_not() {
        let from_number = decode(&serde_json::json!({"integerValue": 7})).expect("exact integer");
        assert_eq!(from_number, value(ValueType::IntegerValue(7)));

        decode(&serde_json::json!({"integerValue": 7.5})).expect_err("7.5 is not an int64");
    }

    #[test]
    fn timestamps_use_rfc3339_in_the_payload() {
        let json = encode(&value(ValueType::TimestampValue(Timestamp {
            seconds: 1_700_000_000,
            nanos: 123_456_789,
        })))
        .expect("encode");
        assert_eq!(
            json["timestampValue"],
            Json::String("2023-11-14T22:13:20.123456789Z".to_string())
        );
    }

    /// Every timestamp `encode` must refuse, as a list, so each case is named.
    fn every_unstorable_timestamp() -> Vec<(&'static str, Timestamp)> {
        vec![
            (
                "the ticket's repro, year 11476",
                Timestamp {
                    seconds: 300_000_000_000,
                    nanos: 0,
                },
            ),
            (
                "one second past 9999-12-31T23:59:59Z",
                Timestamp {
                    seconds: TIMESTAMP_MAX_SECONDS + 1,
                    nanos: 0,
                },
            ),
            (
                "one second before 0001-01-01T00:00:00Z",
                Timestamp {
                    seconds: TIMESTAMP_MIN_SECONDS - 1,
                    nanos: 0,
                },
            ),
            (
                // Inside chrono's range and so encodable as `0000-01-01…`, but
                // outside the proto's: year 0 is not a year 0001-9999.
                "year 0000",
                Timestamp {
                    seconds: -62_167_219_200,
                    nanos: 0,
                },
            ),
            ("i64::MAX seconds", Timestamp {
                seconds: i64::MAX,
                nanos: 0,
            }),
            ("i64::MIN seconds", Timestamp {
                seconds: i64::MIN,
                nanos: 0,
            }),
            (
                "negative nanos, which proto3 forbids",
                Timestamp {
                    seconds: 0,
                    nanos: -1,
                },
            ),
            (
                "a whole second's worth of nanos",
                Timestamp {
                    seconds: 0,
                    nanos: 1_000_000_000,
                },
            ),
        ]
    }

    /// The bug SAK-205 reported, stated as the property that was violated:
    /// a write either fails or produces a payload the read path accepts.
    #[test]
    fn a_timestamp_outside_years_0001_9999_fails_the_write() {
        for (case, timestamp) in every_unstorable_timestamp() {
            let error = encode(&value(ValueType::TimestampValue(timestamp)))
                .expect_err(&format!("{case} must not encode"));
            assert!(
                matches!(error, ValueCodecError::TimestampOutOfRange { .. }),
                "{case}: wrong error {error}"
            );
            // The message has to say what is wrong; it is what reaches the SDK.
            assert!(
                error.to_string().contains("0001-9999"),
                "{case}: unhelpful message {error}"
            );
        }
    }

    /// The property the SQL pushdown depends on: every timestamp this module
    /// writes is the same number of bytes, so byte order is chronological
    /// order. The old encoder broke this as well as the round trip.
    #[test]
    fn every_encoded_timestamp_is_the_same_fixed_width() {
        let timestamps: Vec<String> = every_storable_value()
            .iter()
            .filter_map(|original| {
                let json = encode(original).expect("storable values encode");
                Some(json.get(tag::TIMESTAMP)?.as_str()?.to_string())
            })
            .collect();

        assert!(timestamps.len() >= 5, "the corpus lost its timestamps");
        for text in timestamps {
            assert_eq!(
                text.len(),
                ENCODED_TIMESTAMP_LEN,
                "{text} is not fixed width"
            );
            assert!(text.ends_with('Z'), "{text} is not a UTC spelling");
            assert!(
                text.starts_with(|first: char| first.is_ascii_digit()),
                "{text} has a signed year"
            );
        }
    }

    /// A row a pre-SAK-205 build wrote stays readable, because refusing it
    /// fails every scan of the collection holding it, not just that document.
    #[test]
    fn a_legacy_expanded_year_payload_is_still_readable() {
        let cases = [
            ("+11476-08-15T05:20:00.000000000Z", 300_000_000_000, 0),
            ("+10000-01-01T00:00:00.000000000Z", 253_402_300_800, 0),
            ("-0001-01-01T00:00:00.000000001Z", -62_198_755_200, 1),
        ];

        for (payload, seconds, nanos) in cases {
            let decoded = decode(&serde_json::json!({ "timestampValue": payload }))
                .unwrap_or_else(|error| panic!("{payload} must still decode: {error}"));
            assert_eq!(
                decoded,
                value(ValueType::TimestampValue(Timestamp { seconds, nanos })),
                "{payload} decoded to the wrong instant"
            );

            // And it is refused the next time anything tries to store it, so
            // the legacy spelling cannot be written back.
            encode(&decoded).expect_err("re-encoding a legacy timestamp must fail");
        }
    }

    #[test]
    fn a_timestamp_that_is_neither_rfc_3339_nor_the_legacy_spelling_is_rejected() {
        for payload in [
            "2023-11-14",
            "+not-a-date",
            "+11476-08-15T05:20:00.000000000",
            "+11476-08-15T05:20:00.000000000+09:00",
            "",
        ] {
            decode(&serde_json::json!({ "timestampValue": payload }))
                .expect_err(&format!("{payload:?} is not a timestamp"));
        }
    }

    #[test]
    fn pipeline_values_are_refused_rather_than_stored_wrong() {
        let error = encode(&value(ValueType::FieldReferenceValue("a".to_string())))
            .expect_err("field references are not document data");
        assert!(matches!(error, ValueCodecError::Unstorable { .. }), "{error}");
    }

    #[test]
    fn unknown_and_ambiguous_payloads_are_rejected() {
        decode(&serde_json::json!({"unknownValue": 1})).expect_err("unknown tag");
        decode(&serde_json::json!({})).expect_err("no tag");
        decode(&serde_json::json!({"stringValue": "a", "integerValue": "1"}))
            .expect_err("two tags is ambiguous");
        decode(&serde_json::json!("bare")).expect_err("not an object");
        decode(&serde_json::json!({"stringValue": 1})).expect_err("wrong payload type");
        decode(&serde_json::json!({"nullValue": 3})).expect_err("nullValue must be null");
    }

    #[test]
    fn empty_repeated_payloads_decode_the_way_proto3_json_writes_them() {
        // proto3 JSON omits empty repeated/map fields, so `{"arrayValue":{}}`
        // has to mean the empty array rather than an error.
        assert_eq!(
            decode(&serde_json::json!({"arrayValue": {}})).expect("empty array"),
            value(ValueType::ArrayValue(ArrayValue { values: Vec::new() }))
        );
        assert_eq!(
            decode(&serde_json::json!({"mapValue": {}})).expect("empty map"),
            value(ValueType::MapValue(MapValue {
                fields: HashMap::new()
            }))
        );
    }

    #[test]
    fn field_maps_round_trip_as_a_whole() {
        let fields: HashMap<String, Value> = [
            ("a".to_string(), value(ValueType::IntegerValue(1))),
            ("b".to_string(), value(ValueType::StringValue("x".into()))),
        ]
        .into_iter()
        .collect();

        let json = encode_fields(&fields).expect("encode");
        assert_eq!(decode_fields(&json).expect("decode"), fields);
    }

    #[test]
    fn a_value_with_no_type_set_is_an_error() {
        encode(&Value { value_type: None }).expect_err("Value must have a type");
    }
}
