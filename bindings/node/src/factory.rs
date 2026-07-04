//! The `factory` namespace: a convenient **type-inference** factory.
//!
//! `factory.scalar(value)`, `factory.dtype(value)` and `factory.field(name, value)`
//! infer the data type from a native JS value and build the matching
//! `yggdryl.scalar` / `yggdryl.dtype` / `yggdryl.field` object, so a value crosses
//! without naming its type. The inference mirrors the model's available types: an
//! integer `number` / `bigint` → `int64`, a `Buffer` → `binary`, `null` /
//! `undefined` → `null`, and an array of integers → an `int64` serie. A value the
//! model has no type for (a non-integer `number`, a `string`, a `boolean`, a plain
//! object, or an array of anything but integers) throws — build it through the
//! explicit per-type factories.

use napi::bindgen_prelude::{BigInt, Buffer, Either4, Error, Result};
use napi_derive::napi;

use crate::dtype::{BinaryType, Int64SerieType, Int64Type, NullType};
use crate::field::{BinaryField, Int64Field, Int64SerieField, NullField};
use crate::scalar::{BinaryScalar, Int64Scalar, Int64Serie, NullScalar};

/// The inferred type, carrying the extracted native value.
enum Inferred {
    Null,
    Int64(i64),
    Binary(Vec<u8>),
    Serie(Vec<i64>),
}

/// A JS integer `number` as an `i64`, or an actionable error when it is fractional
/// or out of range.
fn number_to_i64(number: f64) -> Result<i64> {
    if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
        Ok(number as i64)
    } else {
        Err(Error::from_reason(format!(
            "cannot infer an int64 from {number}: expected an integer in the int64 range"
        )))
    }
}

/// The JS value a factory function infers from. napi tries each variant in order — a
/// `Buffer` first, then an array (`Vec<f64>`), then a `bigint`, then a `number` — and
/// `None` is JS `null` / `undefined`.
type Value = Option<Either4<Buffer, Vec<f64>, BigInt, f64>>;

/// Infer the data type from `value`, extracting the native value.
fn infer(value: Value) -> Result<Inferred> {
    Ok(match value {
        None => Inferred::Null,
        Some(Either4::A(buffer)) => Inferred::Binary(buffer.to_vec()),
        Some(Either4::B(numbers)) => {
            // A homogeneous array of integers → an int64 serie (the model's only
            // bindable serie); an empty array defaults to it too.
            let mut values = Vec::with_capacity(numbers.len());
            for number in numbers {
                values.push(number_to_i64(number)?);
            }
            Inferred::Serie(values)
        }
        Some(Either4::C(big)) => Inferred::Int64(crate::bigint_to_i64(big)?),
        Some(Either4::D(number)) => Inferred::Int64(number_to_i64(number)?),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.scalar`.
#[napi(namespace = "factory")]
pub fn scalar(value: Value) -> Result<Either4<NullScalar, Int64Scalar, BinaryScalar, Int64Serie>> {
    Ok(match infer(value)? {
        Inferred::Null => Either4::A(NullScalar::default()),
        Inferred::Int64(integer) => Either4::B(Int64Scalar {
            inner: yggdryl_scalar::Int64Scalar::new(integer),
        }),
        Inferred::Binary(bytes) => Either4::C(BinaryScalar {
            inner: yggdryl_scalar::BinaryScalar::new(bytes),
        }),
        Inferred::Serie(values) => Either4::D(Int64Serie {
            inner: yggdryl_scalar::Int64Serie::from(values),
        }),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.dtype`.
#[napi(namespace = "factory")]
pub fn dtype(value: Value) -> Result<Either4<NullType, Int64Type, BinaryType, Int64SerieType>> {
    Ok(match infer(value)? {
        Inferred::Null => Either4::A(NullType::default()),
        Inferred::Int64(_) => Either4::B(Int64Type::default()),
        Inferred::Binary(_) => Either4::C(BinaryType::default()),
        Inferred::Serie(_) => Either4::D(Int64SerieType::default()),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.field` named
/// `name` (nullable by default).
#[napi(namespace = "factory")]
pub fn field(
    name: String,
    value: Value,
    nullable: Option<bool>,
) -> Result<Either4<NullField, Int64Field, BinaryField, Int64SerieField>> {
    let nullable = nullable.unwrap_or(true);
    Ok(match infer(value)? {
        Inferred::Null => Either4::A(NullField {
            inner: yggdryl_field::NullField::new(name, nullable),
        }),
        Inferred::Int64(_) => Either4::B(Int64Field {
            inner: yggdryl_field::Int64Field::new(name, nullable),
        }),
        Inferred::Binary(_) => Either4::C(BinaryField {
            inner: yggdryl_field::BinaryField::new(name, nullable),
        }),
        Inferred::Serie(_) => Either4::D(Int64SerieField {
            inner: yggdryl_field::TypedSerieField::new(name, nullable),
        }),
    })
}
