//! The `factory` namespace: a convenient **type-inference** factory.
//!
//! `factory.scalar(value)`, `factory.dtype(value)` and `factory.field(name, value)`
//! infer the data type from a JS value and build the matching
//! `yggdryl.scalar` / `yggdryl.dtype` / `yggdryl.field` object, so a value crosses
//! without naming its type. The inference mirrors the model's available types: a
//! whole `number` / `bigint` → `int64`, a **fractional** `number` → `float64` (JS
//! has only the `f64` `number`, so `2.0` is a whole number and stays `int64`), a
//! `Buffer` → `binary`, `null` / `undefined` → `null`, a numeric array → an `int64`
//! serie when every element is whole or a `float64` serie as soon as one is
//! fractional, and a plain object → a `struct`, each member inferred the same way
//! (`scalar` builds the `RecordScalar` row, `dtype` its `StructType`, `field` a
//! `StructField`). The namespaces' own classes are inputs too: a `yggdryl.scalar`
//! handle re-wraps as the same class over the same value in `scalar` and classifies
//! as its data type in `dtype`; a `yggdryl.dtype` handle is the identity for `dtype`
//! and builds its default scalar in `scalar` (the null record for a `StructType` —
//! the scalar models nullness). A value the model has no type for (a `string`, a
//! `boolean`, or an array of anything but numbers) throws — build it through the
//! explicit per-type factories.

use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either23, Either4, Either9, Error, Object, Result,
};
use napi_derive::napi;
use yggdryl_dtype::arrow_schema;
use yggdryl_scalar::{AnyScalar, Scalar};

use crate::data_error;
use crate::dtype::{
    BinaryType, Float32SerieType, Float32Type, Float64SerieType, Float64Type, Int64SerieType,
    Int64Type, NullType, StructType,
};
use crate::field::{
    BinaryField, Float32Field, Float32SerieField, Float64Field, Float64SerieField, Int64Field,
    Int64SerieField, NullField, StructField,
};
use crate::scalar::{
    BinaryScalar, Float32Scalar, Float32Serie, Float64Scalar, Float64Serie, Int64Scalar,
    Int64Serie, NullScalar, RecordScalar,
};

/// The inferred type, carrying the extracted native value.
pub(crate) enum Inferred {
    Null,
    Int64(i64),
    Float64(f64),
    Binary(Vec<u8>),
    Serie(Vec<i64>),
    SerieFloat64(Vec<f64>),
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

/// A native JS value a type is inferred from. napi tries each variant in order — a
/// `Buffer` first, then an array (`Vec<f64>`), then a `bigint`, then a `number` —
/// and `None` is JS `null` / `undefined`.
pub(crate) type NativeValue = Option<Either4<Buffer, Vec<f64>, BigInt, f64>>;

/// Infer the data type from `value`, extracting the native value.
pub(crate) fn infer(value: NativeValue) -> Result<Inferred> {
    Ok(match value {
        None => Inferred::Null,
        Some(Either4::A(buffer)) => Inferred::Binary(buffer.to_vec()),
        Some(Either4::B(numbers)) => {
            // A numeric array → an int64 serie when every element is whole, a
            // float64 serie as soon as one carries a fractional part (JS numbers
            // are all f64); an empty array defaults to the int64 serie.
            if numbers.iter().any(|number| number.fract() != 0.0) {
                Inferred::SerieFloat64(numbers)
            } else {
                let mut values = Vec::with_capacity(numbers.len());
                for number in numbers {
                    values.push(number_to_i64(number)?);
                }
                Inferred::Serie(values)
            }
        }
        Some(Either4::C(big)) => Inferred::Int64(crate::bigint_to_i64(big)?),
        Some(Either4::D(number)) => {
            // A whole `number` infers int64 (backward-compatible; `2.0` is whole);
            // a fractional one is a float64 (JS has only the f64 `number`).
            if number.fract() == 0.0 {
                Inferred::Int64(number_to_i64(number)?)
            } else {
                Inferred::Float64(number)
            }
        }
    })
}

/// One inferred struct member: the Arrow field named `name` (nullable, like every
/// factory-built field) and the atomic scalar holding the value — the field's type
/// is read off the built scalar, so the two always agree.
fn inferred_member(name: String, inferred: Inferred) -> (arrow_schema::Field, AnyScalar) {
    let scalar: AnyScalar = match inferred {
        Inferred::Null => {
            AnyScalar::from_arrow(yggdryl_scalar::NullScalar::default().to_arrow_scalar())
        }
        Inferred::Int64(integer) => AnyScalar::from(yggdryl_scalar::Int64Scalar::new(integer)),
        Inferred::Float64(value) => AnyScalar::from(yggdryl_scalar::Float64Scalar::new(value)),
        Inferred::Binary(bytes) => {
            AnyScalar::from_arrow(yggdryl_scalar::BinaryScalar::new(bytes).to_arrow_scalar())
        }
        Inferred::Serie(values) => {
            AnyScalar::from_arrow(yggdryl_scalar::Int64Serie::from(values).to_arrow_scalar())
        }
        Inferred::SerieFloat64(values) => {
            AnyScalar::from_arrow(yggdryl_scalar::Float64Serie::from(values).to_arrow_scalar())
        }
    };
    let field = arrow_schema::Field::new(name, scalar.data_type(), true);
    (field, scalar)
}

/// A core record row from a plain JS object: each member's value runs through
/// [`infer`] and lands as a one-element child column of the shared struct type.
pub(crate) fn record_from_object(object: &Object) -> Result<yggdryl_scalar::RecordScalar> {
    let names = Object::keys(object)?;
    let mut fields = Vec::with_capacity(names.len());
    let mut scalars = Vec::with_capacity(names.len());
    for name in names {
        let value = object.get::<_, NativeValue>(&name).map_err(|error| {
            Error::from_reason(format!(
                "cannot infer the type of member \"{name}\": {}",
                error.reason
            ))
        })?;
        let (field, scalar) = inferred_member(name, infer(value.flatten())?);
        fields.push(field);
        scalars.push(scalar);
    }
    yggdryl_scalar::RecordScalar::new(
        yggdryl_dtype::StructType::new(arrow_schema::Fields::from(fields)),
        scalars,
    )
    .map_err(data_error)
}

/// A core struct type from a plain JS object of example values — the record row's
/// data type, without keeping the row.
pub(crate) fn struct_type_from_object(object: &Object) -> Result<yggdryl_dtype::StructType> {
    Ok(record_from_object(object)?.data_type().clone())
}

/// The JS value a factory function infers from. napi tries each variant in order:
/// the native values first (a `Buffer`, then an array (`Vec<f64>`), a `bigint`, a
/// `number`), then the namespaces' own class handles (`instanceof`-checked), and a
/// plain object **last**, so it cannot shadow the buffers, arrays and class
/// handles that are also `typeof "object"`; `None` is JS `null` / `undefined`.
type Value = Option<
    Either23<
        Buffer,
        Vec<f64>,
        BigInt,
        f64,
        ClassInstance<NullScalar>,
        ClassInstance<Int64Scalar>,
        ClassInstance<BinaryScalar>,
        ClassInstance<Int64Serie>,
        ClassInstance<RecordScalar>,
        ClassInstance<Float32Scalar>,
        ClassInstance<Float64Scalar>,
        ClassInstance<Float32Serie>,
        ClassInstance<Float64Serie>,
        ClassInstance<NullType>,
        ClassInstance<Int64Type>,
        ClassInstance<BinaryType>,
        ClassInstance<Int64SerieType>,
        ClassInstance<StructType>,
        ClassInstance<Float32Type>,
        ClassInstance<Float64Type>,
        ClassInstance<Float32SerieType>,
        ClassInstance<Float64SerieType>,
        Object,
    >,
>;

/// The classified factory input: a native value's inference, one of the
/// `yggdryl.scalar` handles (its core value cloned out), or one of the
/// `yggdryl.dtype` handles.
enum Classified {
    Value(Inferred),
    NullScalar(yggdryl_scalar::NullScalar),
    Int64Scalar(yggdryl_scalar::Int64Scalar),
    BinaryScalar(yggdryl_scalar::BinaryScalar),
    Int64Serie(yggdryl_scalar::Int64Serie),
    Record(yggdryl_scalar::RecordScalar),
    Float32Scalar(yggdryl_scalar::Float32Scalar),
    Float64Scalar(yggdryl_scalar::Float64Scalar),
    Float32Serie(yggdryl_scalar::Float32Serie),
    Float64Serie(yggdryl_scalar::Float64Serie),
    NullType,
    Int64Type,
    BinaryType,
    Int64SerieType,
    StructType(yggdryl_dtype::StructType),
    Float32Type,
    Float64Type,
    Float32SerieType,
    Float64SerieType,
}

/// Classify `value`: native values run through [`infer`], class handles clone the
/// core value out, and a plain object builds the record row.
fn classify(value: Value) -> Result<Classified> {
    Ok(match value {
        None => Classified::Value(Inferred::Null),
        Some(Either23::A(buffer)) => Classified::Value(infer(Some(Either4::A(buffer)))?),
        Some(Either23::B(numbers)) => Classified::Value(infer(Some(Either4::B(numbers)))?),
        Some(Either23::C(big)) => Classified::Value(infer(Some(Either4::C(big)))?),
        Some(Either23::D(number)) => Classified::Value(infer(Some(Either4::D(number)))?),
        Some(Either23::E(scalar)) => Classified::NullScalar(scalar.inner),
        Some(Either23::F(scalar)) => Classified::Int64Scalar(scalar.inner),
        Some(Either23::G(scalar)) => Classified::BinaryScalar(scalar.inner.clone()),
        Some(Either23::H(serie)) => Classified::Int64Serie(serie.inner.clone()),
        Some(Either23::I(record)) => Classified::Record(record.inner.clone()),
        Some(Either23::J(scalar)) => Classified::Float32Scalar(scalar.inner),
        Some(Either23::K(scalar)) => Classified::Float64Scalar(scalar.inner),
        Some(Either23::L(serie)) => Classified::Float32Serie(serie.inner.clone()),
        Some(Either23::M(serie)) => Classified::Float64Serie(serie.inner.clone()),
        Some(Either23::N(_)) => Classified::NullType,
        Some(Either23::O(_)) => Classified::Int64Type,
        Some(Either23::P(_)) => Classified::BinaryType,
        Some(Either23::Q(_)) => Classified::Int64SerieType,
        Some(Either23::R(data_type)) => Classified::StructType(data_type.inner.clone()),
        Some(Either23::S(_)) => Classified::Float32Type,
        Some(Either23::T(_)) => Classified::Float64Type,
        Some(Either23::U(_)) => Classified::Float32SerieType,
        Some(Either23::V(_)) => Classified::Float64SerieType,
        Some(Either23::W(object)) => {
            // An array only reaches the object fallback when an element failed the
            // native readings above — keep the actionable serie error rather than
            // treating its indices as record fields.
            if object.is_array()? {
                return Err(Error::from_reason(
                    "cannot infer a serie from the array: expected every element to be a number",
                ));
            }
            Classified::Record(record_from_object(&object)?)
        }
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.scalar` — an
/// existing scalar handle re-wraps as the same class, a data type builds its
/// default scalar, a plain object builds the `RecordScalar` row.
#[napi(namespace = "factory")]
// The full return union is spelled inline so napi generates the precise TypeScript
// type (a type alias would erase to `any`); the nine arms trip the complexity lint.
#[allow(clippy::type_complexity)]
pub fn scalar(
    value: Value,
) -> Result<
    Either9<
        NullScalar,
        Int64Scalar,
        BinaryScalar,
        Int64Serie,
        RecordScalar,
        Float32Scalar,
        Float64Scalar,
        Float32Serie,
        Float64Serie,
    >,
> {
    Ok(match classify(value)? {
        Classified::Value(Inferred::Null) | Classified::NullType => {
            Either9::A(NullScalar::default())
        }
        Classified::Value(Inferred::Int64(integer)) => Either9::B(Int64Scalar {
            inner: yggdryl_scalar::Int64Scalar::new(integer),
        }),
        Classified::Value(Inferred::Binary(bytes)) => Either9::C(BinaryScalar {
            inner: yggdryl_scalar::BinaryScalar::new(bytes),
        }),
        Classified::Value(Inferred::Serie(values)) => Either9::D(Int64Serie {
            inner: yggdryl_scalar::Int64Serie::from(values),
        }),
        Classified::Value(Inferred::Float64(value)) => Either9::G(Float64Scalar {
            inner: yggdryl_scalar::Float64Scalar::new(value),
        }),
        Classified::Value(Inferred::SerieFloat64(values)) => Either9::I(Float64Serie {
            inner: yggdryl_scalar::Float64Serie::from(values),
        }),
        Classified::NullScalar(inner) => Either9::A(NullScalar { inner }),
        Classified::Int64Scalar(inner) => Either9::B(Int64Scalar { inner }),
        Classified::BinaryScalar(inner) => Either9::C(BinaryScalar { inner }),
        Classified::Int64Serie(inner) => Either9::D(Int64Serie { inner }),
        Classified::Record(inner) => Either9::E(RecordScalar { inner }),
        Classified::Float32Scalar(inner) => Either9::F(Float32Scalar { inner }),
        Classified::Float64Scalar(inner) => Either9::G(Float64Scalar { inner }),
        Classified::Float32Serie(inner) => Either9::H(Float32Serie { inner }),
        Classified::Float64Serie(inner) => Either9::I(Float64Serie { inner }),
        Classified::Int64Type => Either9::B(Int64Type::default().default_scalar()),
        Classified::BinaryType => Either9::C(BinaryType::default().default_scalar()),
        Classified::Int64SerieType => Either9::D(Int64SerieType::default().default_scalar()),
        // A struct type's default scalar is the null record: the scalar models
        // nullness, and the fields carry no default row.
        Classified::StructType(data_type) => Either9::E(RecordScalar {
            inner: yggdryl_scalar::RecordScalar::null(data_type),
        }),
        Classified::Float32Type => Either9::F(Float32Type::default().default_scalar()),
        Classified::Float64Type => Either9::G(Float64Type::default().default_scalar()),
        Classified::Float32SerieType => Either9::H(Float32SerieType::default().default_scalar()),
        Classified::Float64SerieType => Either9::I(Float64SerieType::default().default_scalar()),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.dtype` — an
/// existing data type handle is the identity, a scalar handle classifies as its
/// data type, a plain object builds the `StructType`.
#[napi(namespace = "factory")]
#[allow(clippy::type_complexity)]
pub fn dtype(
    value: Value,
) -> Result<
    Either9<
        NullType,
        Int64Type,
        BinaryType,
        Int64SerieType,
        StructType,
        Float32Type,
        Float64Type,
        Float32SerieType,
        Float64SerieType,
    >,
> {
    Ok(match classify(value)? {
        Classified::Value(Inferred::Null) | Classified::NullScalar(_) | Classified::NullType => {
            Either9::A(NullType::default())
        }
        Classified::Value(Inferred::Int64(_))
        | Classified::Int64Scalar(_)
        | Classified::Int64Type => Either9::B(Int64Type::default()),
        Classified::Value(Inferred::Binary(_))
        | Classified::BinaryScalar(_)
        | Classified::BinaryType => Either9::C(BinaryType::default()),
        Classified::Value(Inferred::Serie(_))
        | Classified::Int64Serie(_)
        | Classified::Int64SerieType => Either9::D(Int64SerieType::default()),
        Classified::Record(record) => Either9::E(StructType {
            inner: record.data_type().clone(),
        }),
        Classified::StructType(inner) => Either9::E(StructType { inner }),
        Classified::Value(Inferred::Float64(_))
        | Classified::Float64Scalar(_)
        | Classified::Float64Type => Either9::G(Float64Type::default()),
        Classified::Float32Scalar(_) | Classified::Float32Type => {
            Either9::F(Float32Type::default())
        }
        Classified::Value(Inferred::SerieFloat64(_))
        | Classified::Float64Serie(_)
        | Classified::Float64SerieType => Either9::I(Float64SerieType::default()),
        Classified::Float32Serie(_) | Classified::Float32SerieType => {
            Either9::H(Float32SerieType::default())
        }
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.field` named
/// `name` (nullable by default) — class handles and plain objects classify like
/// `dtype`.
#[napi(namespace = "factory")]
#[allow(clippy::type_complexity)]
pub fn field(
    name: String,
    value: Value,
    nullable: Option<bool>,
) -> Result<
    Either9<
        NullField,
        Int64Field,
        BinaryField,
        Int64SerieField,
        StructField,
        Float32Field,
        Float64Field,
        Float32SerieField,
        Float64SerieField,
    >,
> {
    let nullable = nullable.unwrap_or(true);
    Ok(match classify(value)? {
        Classified::Value(Inferred::Null) | Classified::NullScalar(_) | Classified::NullType => {
            Either9::A(NullField {
                inner: yggdryl_field::NullField::new(name, nullable),
            })
        }
        Classified::Value(Inferred::Int64(_))
        | Classified::Int64Scalar(_)
        | Classified::Int64Type => Either9::B(Int64Field {
            inner: yggdryl_field::Int64Field::new(name, nullable),
        }),
        Classified::Value(Inferred::Binary(_))
        | Classified::BinaryScalar(_)
        | Classified::BinaryType => Either9::C(BinaryField {
            inner: yggdryl_field::BinaryField::new(name, nullable),
        }),
        Classified::Value(Inferred::Serie(_))
        | Classified::Int64Serie(_)
        | Classified::Int64SerieType => Either9::D(Int64SerieField {
            inner: yggdryl_field::TypedSerieField::new(name, nullable),
        }),
        Classified::Record(record) => Either9::E(StructField {
            inner: yggdryl_field::StructField::new(name, record.data_type().clone(), nullable),
        }),
        Classified::StructType(data_type) => Either9::E(StructField {
            inner: yggdryl_field::StructField::new(name, data_type, nullable),
        }),
        Classified::Value(Inferred::Float64(_))
        | Classified::Float64Scalar(_)
        | Classified::Float64Type => Either9::G(Float64Field {
            inner: yggdryl_field::Float64Field::new(name, nullable),
        }),
        Classified::Float32Scalar(_) | Classified::Float32Type => Either9::F(Float32Field {
            inner: yggdryl_field::Float32Field::new(name, nullable),
        }),
        Classified::Value(Inferred::SerieFloat64(_))
        | Classified::Float64Serie(_)
        | Classified::Float64SerieType => Either9::I(Float64SerieField {
            inner: yggdryl_field::TypedSerieField::new(name, nullable),
        }),
        Classified::Float32Serie(_) | Classified::Float32SerieType => {
            Either9::H(Float32SerieField {
                inner: yggdryl_field::TypedSerieField::new(name, nullable),
            })
        }
    })
}
