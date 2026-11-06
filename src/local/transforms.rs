//! Server-computed field transforms for the local Firestore write path.
//!
//! A `FieldTransform` is the one part of a Firestore write whose value the
//! *server* decides: `serverTimestamp()` resolves to the commit time,
//! `increment()` reads the stored number and adds to it, and the array
//! transforms read the stored array and add to or remove from it. That read has
//! to happen inside the same store transaction as the write, or two concurrent
//! `increment(1)` calls lose one of the increments — so this module is split in
//! two halves:
//!
//! * [`compile`] translates the proto into the validated [`FieldTransform`]s
//!   this module applies. It is pure and runs in the data plane **before** the
//!   store is touched, so a transform we cannot serve fails the whole batch
//!   rather than applying half of it.
//! * [`apply`] runs inside [`Store::commit`](crate::local::store::Store::commit),
//!   against the document the write's base mutation left behind, and is
//!   infallible: everything that can be rejected was rejected at compile time.
//!
//! Semantics come from the comments on
//! `DocumentTransform.FieldTransform` in the vendored public
//! `proto/google/firestore/v1/write.proto` — in particular the parts that are
//! easy to get subtly wrong:
//!
//! * `increment` on a field that is missing, or holds something that is not a
//!   number, **sets** the field to the operand rather than failing.
//! * if either side is a double, both are interpreted as doubles.
//! * integer overflow resolves to the largest-magnitude integer of that sign,
//!   it does not wrap.
//! * the array transforms treat a missing or non-array field as the empty
//!   array, and compare elements with numbers-across-types equality where
//!   `NaN == NaN` and `null == null`.
//! * `REQUEST_TIME` has **millisecond** precision, and every field transformed
//!   in one commit gets the same timestamp.

use prost_types::Timestamp;
use tonic::Status;

use crate::local::firestore::unimplemented;
use crate::local::query::values_equivalent;
use crate::protos::google::firestore::v1::document_transform::field_transform::{
    ServerValue, TransformType,
};
use crate::protos::google::firestore::v1::document_transform::FieldTransform as ProtoFieldTransform;
use crate::protos::google::firestore::v1::{value::ValueType, ArrayValue, Document, Value};

/// One validated transform: which field, and what to do to it.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldTransform {
    /// Top-level field name. Nested paths are refused by [`compile`].
    pub field: String,
    pub kind: TransformKind,
}

/// The transforms `fireup local` serves.
///
/// `maximum` and `minimum` are deliberately absent: they are part of the same
/// proto oneof, but no SDK surface emits them, and a variant here would have to
/// be implemented rather than refused. [`compile`] rejects them explicitly.
#[derive(Debug, Clone, PartialEq)]
pub enum TransformKind {
    /// `FieldValue.serverTimestamp()` — the commit timestamp.
    RequestTime,
    /// `FieldValue.increment(n)`.
    Increment(Number),
    /// `FieldValue.arrayUnion(…)`.
    AppendMissingElements(Vec<Value>),
    /// `FieldValue.arrayRemove(…)`.
    RemoveAllFromArray(Vec<Value>),
}

/// An `increment` operand, kept in its original width so an integer increment
/// of an integer field stays exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Integer(i64),
    Double(f64),
}

impl Number {
    fn as_double(self) -> f64 {
        match self {
            Number::Integer(number) => number as f64,
            Number::Double(number) => number,
        }
    }

    fn value(self) -> Value {
        match self {
            Number::Integer(number) => integer_value(number),
            Number::Double(number) => double_value(number),
        }
    }
}

/// Translate a write's proto transforms, rejecting everything we cannot serve.
///
/// Pure: nothing here reads the store, so a `Commit` carrying one unsupported
/// transform fails before any of its writes apply.
pub fn compile(transforms: &[ProtoFieldTransform]) -> Result<Vec<FieldTransform>, Status> {
    transforms.iter().map(compile_one).collect()
}

fn compile_one(transform: &ProtoFieldTransform) -> Result<FieldTransform, Status> {
    let field = transform.field_path.clone();
    if field.is_empty() {
        return Err(Status::invalid_argument(
            "a field transform needs a field_path",
        ));
    }
    // A transform is a *write*, so it is held to the write path's field-path
    // rule rather than the read path's: `masked_fields` refuses a nested
    // `update_mask` because the store writes whole top-level fields, and a
    // transform on `a.b` would have the same problem — it would have to merge
    // into a stored map. Read masks, filters, and orders do support nested
    // paths; this deliberately does not.
    if field.contains('.') || field.contains('`') {
        return Err(unimplemented(
            &format!("the nested field transform path {field:?}"),
            "a transform writes a whole top-level field, like an update mask",
        ));
    }

    let kind = match transform
        .transform_type
        .as_ref()
        .ok_or_else(|| Status::invalid_argument(format!("transform on {field:?} is empty")))?
    {
        TransformType::SetToServerValue(server_value) => {
            match ServerValue::try_from(*server_value) {
                Ok(ServerValue::RequestTime) => TransformKind::RequestTime,
                // The proto says the unspecified value must not be used, and a
                // future one is not something we can guess at.
                _ => {
                    return Err(Status::invalid_argument(format!(
                        "unknown set_to_server_value {server_value} on field {field:?}"
                    )))
                }
            }
        }
        TransformType::Increment(operand) => TransformKind::Increment(number(operand).ok_or_else(
            || {
                Status::invalid_argument(format!(
                    "increment on field {field:?} needs an integer or double operand"
                ))
            },
        )?),
        TransformType::AppendMissingElements(array) => {
            TransformKind::AppendMissingElements(array.values.clone())
        }
        TransformType::RemoveAllFromArray(array) => {
            TransformKind::RemoveAllFromArray(array.values.clone())
        }
        // Served would mean implemented; refused means an SDK that somehow
        // emits one sees a clear gap instead of a wrong number.
        TransformType::Maximum(_) => {
            return Err(unimplemented(
                &format!("the maximum field transform (on {field:?})"),
                "no SDK surface emits it; increment, serverTimestamp, and the array transforms are served",
            ))
        }
        TransformType::Minimum(_) => {
            return Err(unimplemented(
                &format!("the minimum field transform (on {field:?})"),
                "no SDK surface emits it; increment, serverTimestamp, and the array transforms are served",
            ))
        }
    };

    Ok(FieldTransform { field, kind })
}

/// Apply `transforms` to `document` in order, returning the
/// `WriteResult.transform_results` value for each.
///
/// Infallible on purpose — [`compile`] already rejected everything that can be
/// rejected, so there is no way for this to fail halfway through a store
/// transaction and leave a partly transformed document behind.
pub fn apply(
    transforms: &[FieldTransform],
    document: &mut Document,
    now: &Timestamp,
) -> Vec<Value> {
    transforms
        .iter()
        .map(|transform| {
            let current = document.fields.get(&transform.field);
            let (next, result) = match &transform.kind {
                TransformKind::RequestTime => {
                    let value = request_time(now);
                    (value.clone(), value)
                }
                TransformKind::Increment(operand) => {
                    let value = increment(current, *operand);
                    (value.clone(), value)
                }
                // The array transforms report a null result, per the proto.
                TransformKind::AppendMissingElements(elements) => {
                    (append_missing(current, elements), null_value())
                }
                TransformKind::RemoveAllFromArray(elements) => {
                    (remove_all(current, elements), null_value())
                }
            };
            document.fields.insert(transform.field.clone(), next);
            result
        })
        .collect()
}

/// The commit timestamp, truncated to milliseconds.
///
/// `REQUEST_TIME` is documented as millisecond precision, so the value a client
/// reads back is deliberately coarser than the document's `update_time` rather
/// than claiming a precision the contract does not promise.
fn request_time(now: &Timestamp) -> Value {
    Value {
        value_type: Some(ValueType::TimestampValue(Timestamp {
            seconds: now.seconds,
            nanos: now.nanos / 1_000_000 * 1_000_000,
        })),
    }
}

/// `increment`: add to a number, or set the field if it is not one.
fn increment(current: Option<&Value>, operand: Number) -> Value {
    match current.and_then(number) {
        // Missing, or holding a string/bool/array/…: the proto says set it.
        None => operand.value(),
        Some(Number::Integer(stored)) => match operand {
            // Saturating, not wrapping: the proto resolves an overflow to the
            // largest-magnitude integer of that sign.
            Number::Integer(by) => integer_value(stored.saturating_add(by)),
            Number::Double(_) => double_value(stored as f64 + operand.as_double()),
        },
        // A double on either side makes the whole sum IEEE-754 double maths.
        Some(stored) => double_value(stored.as_double() + operand.as_double()),
    }
}

/// `arrayUnion`: append the elements that are not already present.
///
/// Appending as we go is also what implements "if the input contains multiple
/// equivalent values, only the first will be considered": the second one finds
/// the first already in the result.
fn append_missing(current: Option<&Value>, elements: &[Value]) -> Value {
    let mut values = array_or_empty(current);
    for element in elements {
        if !values
            .iter()
            .any(|present| values_equivalent(present, element))
        {
            values.push(element.clone());
        }
    }
    array_value(values)
}

/// `arrayRemove`: drop every element equivalent to any of `elements`.
fn remove_all(current: Option<&Value>, elements: &[Value]) -> Value {
    let mut values = array_or_empty(current);
    values.retain(|present| {
        !elements
            .iter()
            .any(|removed| values_equivalent(present, removed))
    });
    array_value(values)
}

/// The field's current elements, or the empty array for a missing or non-array
/// field — which is what the proto says both array transforms do.
fn array_or_empty(current: Option<&Value>) -> Vec<Value> {
    match current.map(|value| &value.value_type) {
        Some(Some(ValueType::ArrayValue(array))) => array.values.clone(),
        _ => Vec::new(),
    }
}

fn number(value: &Value) -> Option<Number> {
    match &value.value_type {
        Some(ValueType::IntegerValue(number)) => Some(Number::Integer(*number)),
        Some(ValueType::DoubleValue(number)) => Some(Number::Double(*number)),
        _ => None,
    }
}

fn integer_value(number: i64) -> Value {
    Value {
        value_type: Some(ValueType::IntegerValue(number)),
    }
}

fn double_value(number: f64) -> Value {
    Value {
        value_type: Some(ValueType::DoubleValue(number)),
    }
}

fn null_value() -> Value {
    Value {
        value_type: Some(ValueType::NullValue(0)),
    }
}

fn array_value(values: Vec<Value>) -> Value {
    Value {
        value_type: Some(ValueType::ArrayValue(ArrayValue { values })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn string_value(text: &str) -> Value {
        Value {
            value_type: Some(ValueType::StringValue(text.to_string())),
        }
    }

    fn document(fields: Vec<(&str, Value)>) -> Document {
        Document {
            name: "projects/p/databases/(default)/documents/c/d".to_string(),
            fields: fields
                .into_iter()
                .map(|(field, value)| (field.to_string(), value))
                .collect::<HashMap<String, Value>>(),
            create_time: None,
            update_time: None,
        }
    }

    /// Apply one transform kind to a document holding `current` under `n`.
    fn transformed(current: Option<Value>, kind: TransformKind) -> (Value, Value) {
        let mut doc = document(match current {
            Some(value) => vec![("n", value)],
            None => vec![],
        });
        let results = apply(
            &[FieldTransform {
                field: "n".to_string(),
                kind,
            }],
            &mut doc,
            &Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            },
        );
        (
            doc.fields["n"].clone(),
            results.into_iter().next().expect("one result"),
        )
    }

    fn proto(field_path: &str, transform_type: TransformType) -> ProtoFieldTransform {
        ProtoFieldTransform {
            field_path: field_path.to_string(),
            transform_type: Some(transform_type),
        }
    }

    #[test]
    fn server_timestamp_is_the_commit_time_truncated_to_milliseconds() {
        let (stored, result) = transformed(None, TransformKind::RequestTime);
        let expected = Value {
            value_type: Some(ValueType::TimestampValue(Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_000_000,
            })),
        };
        assert_eq!(stored, expected);
        assert_eq!(result, expected, "the result reports the value written");
    }

    #[test]
    fn increment_adds_to_an_integer_exactly() {
        let (stored, result) = transformed(
            Some(integer_value(41)),
            TransformKind::Increment(Number::Integer(1)),
        );
        assert_eq!(stored, integer_value(42));
        assert_eq!(result, integer_value(42));
    }

    #[test]
    fn increment_sets_a_missing_or_non_numeric_field_to_the_operand() {
        let (stored, _) = transformed(None, TransformKind::Increment(Number::Integer(7)));
        assert_eq!(stored, integer_value(7), "missing field");

        let (stored, _) = transformed(
            Some(string_value("not a number")),
            TransformKind::Increment(Number::Double(2.5)),
        );
        assert_eq!(stored, double_value(2.5), "non-numeric field");
    }

    #[test]
    fn a_double_on_either_side_makes_the_sum_a_double() {
        let (stored, _) = transformed(
            Some(integer_value(1)),
            TransformKind::Increment(Number::Double(0.5)),
        );
        assert_eq!(stored, double_value(1.5), "integer field, double operand");

        let (stored, _) = transformed(
            Some(double_value(1.5)),
            TransformKind::Increment(Number::Integer(1)),
        );
        assert_eq!(stored, double_value(2.5), "double field, integer operand");
    }

    #[test]
    fn integer_overflow_saturates_rather_than_wrapping() {
        let (stored, _) = transformed(
            Some(integer_value(i64::MAX)),
            TransformKind::Increment(Number::Integer(5)),
        );
        assert_eq!(stored, integer_value(i64::MAX));

        let (stored, _) = transformed(
            Some(integer_value(i64::MIN)),
            TransformKind::Increment(Number::Integer(-5)),
        );
        assert_eq!(stored, integer_value(i64::MIN));
    }

    #[test]
    fn array_union_appends_only_missing_elements_and_reports_null() {
        let (stored, result) = transformed(
            Some(array_value(vec![string_value("a")])),
            TransformKind::AppendMissingElements(vec![
                string_value("a"),
                string_value("b"),
                string_value("b"),
            ]),
        );
        assert_eq!(
            stored,
            array_value(vec![string_value("a"), string_value("b")]),
            "an already-present element and a repeated input are both appended once"
        );
        assert_eq!(
            result,
            null_value(),
            "array transforms report a null result"
        );
    }

    #[test]
    fn array_union_treats_a_missing_or_non_array_field_as_empty() {
        let (stored, _) = transformed(
            None,
            TransformKind::AppendMissingElements(vec![integer_value(1)]),
        );
        assert_eq!(stored, array_value(vec![integer_value(1)]), "missing field");

        let (stored, _) = transformed(
            Some(string_value("scalar")),
            TransformKind::AppendMissingElements(vec![integer_value(1)]),
        );
        assert_eq!(
            stored,
            array_value(vec![integer_value(1)]),
            "a non-array field is first set to the empty array"
        );
    }

    #[test]
    fn array_elements_compare_across_numeric_types_and_include_nan_and_null() {
        let nan = double_value(f64::NAN);
        let before = vec![integer_value(3), nan.clone(), null_value()];
        let (stored, _) = transformed(
            Some(array_value(before.clone())),
            TransformKind::AppendMissingElements(vec![
                double_value(3.0),
                nan.clone(),
                null_value(),
            ]),
        );

        // Compared element-wise rather than with `assert_eq!`, because `NaN`
        // is not `PartialEq`-equal to itself — which is the whole reason the
        // transforms need their own equality in the first place.
        let after = array_or_empty(Some(&stored));
        assert_eq!(
            after.len(),
            before.len(),
            "3 == 3.0, NaN == NaN, and null == null, so nothing is appended: {after:?}"
        );
        for (left, right) in after.iter().zip(before.iter()) {
            assert!(values_equivalent(left, right), "{left:?} vs {right:?}");
        }
    }

    #[test]
    fn array_remove_drops_every_equivalent_element() {
        let (stored, result) = transformed(
            Some(array_value(vec![
                integer_value(1),
                double_value(1.0),
                integer_value(2),
            ])),
            TransformKind::RemoveAllFromArray(vec![integer_value(1)]),
        );
        assert_eq!(
            stored,
            array_value(vec![integer_value(2)]),
            "duplicates and equivalent numbers of another type both go"
        );
        assert_eq!(result, null_value());
    }

    #[test]
    fn array_remove_on_a_non_array_field_leaves_an_empty_array() {
        let (stored, _) = transformed(
            Some(string_value("scalar")),
            TransformKind::RemoveAllFromArray(vec![integer_value(1)]),
        );
        assert_eq!(stored, array_value(Vec::new()));
    }

    #[test]
    fn transforms_apply_in_order_and_see_each_other() {
        let mut doc = document(vec![]);
        let results = apply(
            &[
                FieldTransform {
                    field: "n".to_string(),
                    kind: TransformKind::Increment(Number::Integer(5)),
                },
                FieldTransform {
                    field: "n".to_string(),
                    kind: TransformKind::Increment(Number::Integer(2)),
                },
            ],
            &mut doc,
            &Timestamp::default(),
        );

        assert_eq!(doc.fields["n"], integer_value(7));
        assert_eq!(results, vec![integer_value(5), integer_value(7)]);
    }

    #[test]
    fn compile_accepts_the_four_served_transforms() {
        let compiled = compile(&[
            proto(
                "t",
                TransformType::SetToServerValue(ServerValue::RequestTime as i32),
            ),
            proto("n", TransformType::Increment(integer_value(2))),
            proto(
                "tags",
                TransformType::AppendMissingElements(ArrayValue {
                    values: vec![string_value("x")],
                }),
            ),
            proto(
                "tags",
                TransformType::RemoveAllFromArray(ArrayValue {
                    values: vec![string_value("y")],
                }),
            ),
        ])
        .expect("the served transforms compile");

        assert_eq!(
            compiled
                .iter()
                .map(|t| t.field.as_str())
                .collect::<Vec<_>>(),
            vec!["t", "n", "tags", "tags"]
        );
        assert_eq!(compiled[0].kind, TransformKind::RequestTime);
        assert_eq!(
            compiled[1].kind,
            TransformKind::Increment(Number::Integer(2))
        );
    }

    #[test]
    fn compile_rejects_a_non_numeric_increment_operand() {
        let error = compile(&[proto("n", TransformType::Increment(string_value("1")))])
            .expect_err("a string increment");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn compile_rejects_max_min_nested_paths_and_empty_transforms() {
        for transform in [
            proto("n", TransformType::Maximum(integer_value(1))),
            proto("n", TransformType::Minimum(integer_value(1))),
        ] {
            let error = compile(&[transform]).expect_err("max/min are not served");
            assert_eq!(error.code(), tonic::Code::Unimplemented);
        }

        let error = compile(&[proto(
            "a.b",
            TransformType::SetToServerValue(ServerValue::RequestTime as i32),
        )])
        .expect_err("nested field path");
        assert_eq!(error.code(), tonic::Code::Unimplemented);

        let error = compile(&[ProtoFieldTransform {
            field_path: "n".to_string(),
            transform_type: None,
        }])
        .expect_err("no transform_type");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);

        let error = compile(&[proto(
            "",
            TransformType::SetToServerValue(ServerValue::RequestTime as i32),
        )])
        .expect_err("empty field path");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);

        let error = compile(&[proto("n", TransformType::SetToServerValue(0))])
            .expect_err("SERVER_VALUE_UNSPECIFIED");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
}
