//! The `yggdryl.typed` namespace — the **typed column surface** grown on the byte contract.
//!
//! Mirrors `yggdryl_core::typed`'s column surface: a [`Serie`] (a typed column — many elements of
//! one [`DataTypeId`](crate::datatype_id::DataTypeId) over a data buffer, plus an optional validity
//! bit buffer for nulls) and its [`Field`] (the column's `name` / element type / `nullable`, carried
//! in a [`Headers`](crate::headers::Headers) map). Every method is a thin delegation to
//! `yggdryl_core::typed::FixedSerie` / `HeaderField` — no logic lives here.
//!
//! The core `FixedSerie<T>` is generic over its element type `T`; napi cannot carry that generic, so
//! [`Serie`] holds an **erased enum** ([`SerieInner`]) over the concrete `FixedSerie<T>` for every
//! element type, and each method dispatches across the variants (through the [`dispatch!`] /
//! [`dispatch_numeric!`] / [`dispatch_rebuild!`] macros, so the 13-way match is written once).
//!
//! Value marshalling matches the `memory` namespace: a **wide integer** (`i64` / `u64` / `i128` /
//! `u128`) crosses as a JS `BigInt`, every **narrow integer** (`i8`…`u32`) and **float**
//! (`f32` / `f64`) as a JS `number`, and a **bool** as a JS `boolean`; a null element is `null`.
//! Integer reductions (`sum`) return a `BigInt` (a wide accumulator never wraps); a float `sum`, and
//! every `mean`, return a `number`. A boolean column does not reduce, so its `sum` / `min` / `max` /
//! `mean` throw the core's guided `Error`.
//!
//! A **byte column** ([`ByteSerie`]) is the variable-length / fixed-size counterpart of [`Serie`],
//! type-erased over the six byte carriers: variable-length `Binary` / `Utf8` (an `i32` offsets +
//! data buffer), their **large** twins `LargeBinary` / `LargeUtf8` (`i64` offsets, for data past the
//! `i32` offset range), and fixed-size `FixedBinary` / `FixedUtf8` (a fixed byte stride, short values
//! zero-padded and long ones truncated). A binary element crosses as a JS `Buffer`, a UTF-8 element
//! as a JS `string`; it shares the same null-aware `get` / `toList` / `field` surface.
//!
//! A **decimal** column carries a signed *unscaled integer* whose width matches the four native
//! integers: `Decimal32` crosses as a `number` (`i32`), `Decimal64` / `Decimal128` as a `BigInt`
//! (`i64` / `i128`), and `Decimal256` as an arbitrary-precision `BigInt` (its 256-bit [`I256`]
//! bridges to napi's word-based `BigInt`). Decimals are not `Reduce`, so `sum` / `min` / `max` /
//! `mean` throw the guided `Error`; instead read the raw unscaled value with `get` / `values`, place
//! the point with `toDecimalString`, and read `precision` / `scale` from the [`Field`].

use napi::bindgen_prelude::{
    BigInt, Buffer, Either, Either3, Either5, FromNapiValue, Null, ToNapiValue,
};
use napi::{Env, JsUnknown};
use napi_derive::napi;

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;

use yggdryl_core::datatype_id::DataTypeId as DtId;
use yggdryl_core::io::memory::{Heap as CoreHeap, IOBase};
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{
    Decimal128, Decimal256, Decimal32, Decimal64, Float32, Float64, Int128, Int16, Int32, Int64,
    Int8, UInt128, UInt16, UInt32, UInt64, UInt8, I256,
};
use yggdryl_core::typed::{
    Binary, Column as CoreColumn, ColumnField as CoreColumnField, Decoder, Encoder,
    Field as FieldTrait, FixedBinary, FixedSerie, FixedSizeSerie, FixedUtf8, HeaderField,
    LargeBinary, LargeUtf8, ListSerie as CoreListSerie, MapSerie as CoreMapSerie, Scalar,
    Serie as SerieTrait, StructField as CoreStructField, StructSerie as CoreStructSerie, Utf8,
    Value as CoreValue, VarLenType, VarSerie, VarType,
};

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The guided error a boolean column raises for a numeric reduction — `Bit` is not a reducible
/// type, so `sum`/`min`/`max`/`mean`/`std`/`var`/`median`/`countGe` are undefined for it. Shared by
/// `dispatch_numeric!` and `count_ge`, so every reduction refuses a bool column with one message.
fn bool_reduce_error() -> napi::Error {
    to_error(
        "a boolean column does not reduce: sum/min/max/mean need a numeric element type — \
         build a numeric Serie (e.g. DataTypeId.I64())",
    )
}

/// The guided error a decimal column raises for a numeric reduction — the fixed-point decimal types
/// are not reducible. Shared by `dispatch_numeric!` and `count_ge`.
fn decimal_reduce_error() -> napi::Error {
    to_error(
        "a decimal column does not reduce: sum/min/max/mean are not defined for fixed-point \
         decimals — read the raw unscaled values with get/values or format with toDecimalString",
    )
}

/// One JS-side element value — a `number`, a `BigInt`, or a `boolean` (the three shapes a typed
/// element crosses as). Used only inside the binding; the `#[napi]` signatures spell it out so the
/// generated `.d.ts` renders the union.
type JsValue = Either3<f64, BigInt, bool>;

// ---- value marshalling: a native element -> its JS shape ------------------------------

/// A native element type rendered as its JS value (`number` / `BigInt` / `boolean`).
trait ToJs {
    /// This value as its JS shape.
    fn to_js(self) -> JsValue;
}

macro_rules! to_js_number {
    ($($native:ty),*) => {
        $(impl ToJs for $native {
            fn to_js(self) -> JsValue {
                Either3::A(self as f64)
            }
        })*
    };
}
to_js_number!(i8, u8, i16, u16, i32, u32, f32);

impl ToJs for f64 {
    fn to_js(self) -> JsValue {
        Either3::A(self)
    }
}

macro_rules! to_js_bigint {
    ($($native:ty),*) => {
        $(impl ToJs for $native {
            fn to_js(self) -> JsValue {
                Either3::B(BigInt::from(self))
            }
        })*
    };
}
to_js_bigint!(i64, u64, i128, u128);

impl ToJs for bool {
    fn to_js(self) -> JsValue {
        Either3::C(self)
    }
}

/// A 256-bit unscaled decimal value as a napi [`BigInt`] — its 32 little-endian two's-complement
/// bytes re-packed into napi's `(sign_bit, magnitude words)` shape (`words` little-endian `u64`).
/// Shared by [`ToJs`] (numeric columns) and the nested [`value_to_unknown`] marshaller.
fn i256_to_bigint(value: I256) -> BigInt {
    let sign_bit = value.is_negative();
    let mut bytes = value.to_le_bytes();
    if sign_bit {
        negate_le_32(&mut bytes); // two's-complement magnitude for the sign-plus-magnitude form
    }
    let words = bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
        .collect();
    BigInt { sign_bit, words }
}

/// A 256-bit unscaled decimal value as a JS `BigInt` — see [`i256_to_bigint`].
impl ToJs for I256 {
    fn to_js(self) -> JsValue {
        Either3::B(i256_to_bigint(self))
    }
}

/// Two's-complement negate a 256-bit little-endian byte buffer in place (bitwise-not, then `+1`) —
/// the bridge between napi's sign-plus-magnitude `BigInt` and the signed [`I256`] byte form.
fn negate_le_32(bytes: &mut [u8; 32]) {
    let mut carry = 1u16;
    for byte in bytes.iter_mut() {
        let sum = (!*byte) as u16 + carry;
        *byte = sum as u8;
        carry = sum >> 8;
    }
}

/// A reduction accumulator (`sum`) rendered as its JS value — an integer sum as a `BigInt`, a float
/// sum as a `number`.
trait ToJsSum {
    /// This accumulator as its JS shape.
    fn to_js_sum(self) -> Either<BigInt, f64>;
}

impl ToJsSum for i64 {
    fn to_js_sum(self) -> Either<BigInt, f64> {
        Either::A(BigInt::from(self))
    }
}
impl ToJsSum for i128 {
    fn to_js_sum(self) -> Either<BigInt, f64> {
        Either::A(BigInt::from(self))
    }
}
impl ToJsSum for u128 {
    fn to_js_sum(self) -> Either<BigInt, f64> {
        Either::A(BigInt::from(self))
    }
}
impl ToJsSum for f64 {
    fn to_js_sum(self) -> Either<BigInt, f64> {
        Either::B(self)
    }
}

// ---- value marshalling: a JS element -> its native ------------------------------------

/// Extracts a JS `number` element, or throws a guided `Error` (the caller passed the wrong shape).
fn as_number(value: JsValue) -> napi::Result<f64> {
    match value {
        Either3::A(number) => Ok(number),
        _ => Err(to_error(
            "expected a JS number for this element type: pass a number (a bigint suits only \
             i64/u64/i128/u128, a boolean only bool)",
        )),
    }
}

/// Extracts a JS `BigInt` element, or throws a guided `Error` naming the fix.
fn as_bigint(value: JsValue) -> napi::Result<BigInt> {
    match value {
        Either3::B(big) => Ok(big),
        _ => Err(to_error(
            "expected a JS bigint for a 64/128-bit integer element: pass a bigint literal (e.g. 42n)",
        )),
    }
}

/// Extracts a JS `boolean` element, or throws a guided `Error` naming the fix.
fn as_bool(value: JsValue) -> napi::Result<bool> {
    match value {
        Either3::C(flag) => Ok(flag),
        _ => Err(to_error(
            "expected a JS boolean for a bool element: pass true or false",
        )),
    }
}

fn to_i8(value: JsValue) -> napi::Result<i8> {
    Ok(as_number(value)? as i8)
}
fn to_u8(value: JsValue) -> napi::Result<u8> {
    Ok(as_number(value)? as u8)
}
fn to_i16(value: JsValue) -> napi::Result<i16> {
    Ok(as_number(value)? as i16)
}
fn to_u16(value: JsValue) -> napi::Result<u16> {
    Ok(as_number(value)? as u16)
}
fn to_i32(value: JsValue) -> napi::Result<i32> {
    Ok(as_number(value)? as i32)
}
fn to_u32(value: JsValue) -> napi::Result<u32> {
    Ok(as_number(value)? as u32)
}
fn to_i64(value: JsValue) -> napi::Result<i64> {
    Ok(as_bigint(value)?.get_i64().0)
}
fn to_u64(value: JsValue) -> napi::Result<u64> {
    Ok(as_bigint(value)?.get_u64().1)
}
fn to_i128(value: JsValue) -> napi::Result<i128> {
    Ok(as_bigint(value)?.get_i128().0)
}
fn to_u128(value: JsValue) -> napi::Result<u128> {
    Ok(as_bigint(value)?.get_u128().1)
}
/// Extracts a JS `BigInt` as a signed 256-bit [`I256`] — the `Decimal256` unscaled value. A value
/// that fits `i128` takes the fast [`I256::from_i128`] path; a wider one is packed from the
/// `BigInt`'s little-endian `u64` magnitude words (with the sign applied) into 32 two's-complement
/// bytes; a `BigInt` past 256 bits throws the guided `Error`.
fn to_i256(value: JsValue) -> napi::Result<I256> {
    let big = as_bigint(value)?;
    let (as_i128, lossless) = big.get_i128();
    if lossless {
        return Ok(I256::from_i128(as_i128));
    }
    if big.words.len() > 4 {
        return Err(to_error(
            "decimal256 value out of range: a Decimal256 holds a signed 256-bit integer — pass a \
             bigint within the 256-bit range",
        ));
    }
    let mut bytes = [0u8; 32];
    for (index, word) in big.words.iter().enumerate() {
        bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    if big.sign_bit {
        negate_le_32(&mut bytes); // sign-plus-magnitude -> signed two's-complement bytes
    }
    Ok(I256::from_le_bytes(bytes))
}
fn to_f32(value: JsValue) -> napi::Result<f32> {
    Ok(as_number(value)? as f32)
}
fn to_f64(value: JsValue) -> napi::Result<f64> {
    as_number(value)
}
fn to_bool_native(value: JsValue) -> napi::Result<bool> {
    as_bool(value)
}

/// Builds a non-nullable `FixedSerie<T>` from a JS value list, converting each element.
fn build_values<T, F>(values: Vec<JsValue>, convert: F) -> napi::Result<FixedSerie<T>>
where
    T: Encoder + Decoder,
    F: Fn(JsValue) -> napi::Result<T::Native>,
{
    let mut natives = Vec::with_capacity(values.len());
    for value in values {
        natives.push(convert(value)?);
    }
    Ok(FixedSerie::from_values(&natives))
}

/// Builds a nullable `FixedSerie<T>` from a JS option list (a `null` entry becomes a null element).
fn build_options<T, F>(values: Vec<Option<JsValue>>, convert: F) -> napi::Result<FixedSerie<T>>
where
    T: Encoder + Decoder,
    F: Fn(JsValue) -> napi::Result<T::Native>,
{
    let mut natives = Vec::with_capacity(values.len());
    for value in values {
        natives.push(match value {
            Some(value) => Some(convert(value)?),
            None => None,
        });
    }
    Ok(FixedSerie::from_options(&natives))
}

/// Converts a JS value list into a pre-sized native `Vec` for a bulk in-place write — the range twin
/// of [`build_values`] (which wraps its natives in a fresh `FixedSerie`). Reuses the same per-arm
/// converter, so the conversion doubles as the runtime type check.
fn convert_values<N, F>(values: Vec<JsValue>, convert: F) -> napi::Result<Vec<N>>
where
    F: Fn(JsValue) -> napi::Result<N>,
{
    let mut natives = Vec::with_capacity(values.len());
    for value in values {
        natives.push(convert(value)?);
    }
    Ok(natives)
}

/// Clones the borrowed column (the binding holds `&self`, so it cannot consume into the core's
/// `with_*` builders) — carrying its `name` and any decimal `precision` / `scale` metadata across, so
/// a rebuild for one field never drops the others.
fn clone_serie<T: Encoder + Decoder>(serie: &FixedSerie<T>) -> FixedSerie<T> {
    let field = serie.field();
    let mut out =
        FixedSerie::from_data(serie.data().clone(), serie.validity().cloned(), serie.len());
    if let Some(name) = field.name() {
        out = out.with_name(name);
    }
    if let (Some(precision), Some(scale)) = (field.precision(), field.scale()) {
        out = out.with_precision_scale(precision, scale);
    }
    out
}

/// Reconstructs a `FixedSerie<T>` with a fresh `name`, preserving its decimal precision/scale.
fn rename<T: Encoder + Decoder>(serie: &FixedSerie<T>, name: &str) -> FixedSerie<T> {
    clone_serie(serie).with_name(name)
}

/// Reconstructs a `FixedSerie<T>` with the decimal `precision` / `scale` set, preserving its `name`.
fn reprecision<T: Encoder + Decoder>(
    serie: &FixedSerie<T>,
    precision: u32,
    scale: i32,
) -> FixedSerie<T> {
    clone_serie(serie).with_precision_scale(precision, scale)
}

/// Distinct-value count for a 32-bit float column by IEEE-754 bit pattern — the core `n_unique`
/// needs `Eq + Hash`, which `f32` lacks, so the binding counts distinct bit patterns (equal bits ==
/// equal value). DESIGN: `-0.0`/`+0.0` and distinct NaN payloads count as separate values.
fn n_unique_f32(serie: &FixedSerie<Float32>) -> usize {
    (0..serie.len())
        .filter_map(|index| serie.get(index))
        .map(f32::to_bits)
        .collect::<std::collections::HashSet<u32>>()
        .len()
}

/// Distinct-value count for a 64-bit float column by IEEE-754 bit pattern — the `f64` counterpart of
/// `n_unique_f32`.
fn n_unique_f64(serie: &FixedSerie<Float64>) -> usize {
    (0..serie.len())
        .filter_map(|index| serie.get(index))
        .map(f64::to_bits)
        .collect::<std::collections::HashSet<u64>>()
        .len()
}

// ---- the erased column + its dispatch --------------------------------------------------

/// A `FixedSerie<T>` for every concrete element type — the type-erased backing of [`Serie`].
enum SerieInner {
    I8(FixedSerie<Int8>),
    U8(FixedSerie<UInt8>),
    I16(FixedSerie<Int16>),
    U16(FixedSerie<UInt16>),
    I32(FixedSerie<Int32>),
    U32(FixedSerie<UInt32>),
    I64(FixedSerie<Int64>),
    U64(FixedSerie<UInt64>),
    I128(FixedSerie<Int128>),
    U128(FixedSerie<UInt128>),
    F32(FixedSerie<Float32>),
    F64(FixedSerie<Float64>),
    Bool(FixedSerie<Bit>),
    Decimal32(FixedSerie<Decimal32>),
    Decimal64(FixedSerie<Decimal64>),
    Decimal128(FixedSerie<Decimal128>),
    Decimal256(FixedSerie<Decimal256>),
}

/// Runs `$body` against the inner `FixedSerie` (`$serie`) of whichever variant is present — the
/// 17-way match, written once. Every arm must yield the same type.
macro_rules! dispatch {
    ($self:expr, $serie:ident => $body:expr) => {
        match &$self.inner {
            SerieInner::I8($serie) => $body,
            SerieInner::U8($serie) => $body,
            SerieInner::I16($serie) => $body,
            SerieInner::U16($serie) => $body,
            SerieInner::I32($serie) => $body,
            SerieInner::U32($serie) => $body,
            SerieInner::I64($serie) => $body,
            SerieInner::U64($serie) => $body,
            SerieInner::I128($serie) => $body,
            SerieInner::U128($serie) => $body,
            SerieInner::F32($serie) => $body,
            SerieInner::F64($serie) => $body,
            SerieInner::Bool($serie) => $body,
            SerieInner::Decimal32($serie) => $body,
            SerieInner::Decimal64($serie) => $body,
            SerieInner::Decimal128($serie) => $body,
            SerieInner::Decimal256($serie) => $body,
        }
    };
}

/// Like [`dispatch!`], but the boolean arm throws the guided reduction error — for the numeric
/// aggregations (`Bit` is not `Reduce`), whose `$body` must be a `napi::Result`.
macro_rules! dispatch_numeric {
    ($self:expr, $serie:ident => $body:expr) => {
        match &$self.inner {
            SerieInner::I8($serie) => $body,
            SerieInner::U8($serie) => $body,
            SerieInner::I16($serie) => $body,
            SerieInner::U16($serie) => $body,
            SerieInner::I32($serie) => $body,
            SerieInner::U32($serie) => $body,
            SerieInner::I64($serie) => $body,
            SerieInner::U64($serie) => $body,
            SerieInner::I128($serie) => $body,
            SerieInner::U128($serie) => $body,
            SerieInner::F32($serie) => $body,
            SerieInner::F64($serie) => $body,
            SerieInner::Bool(_) => Err(bool_reduce_error()),
            SerieInner::Decimal32(_)
            | SerieInner::Decimal64(_)
            | SerieInner::Decimal128(_)
            | SerieInner::Decimal256(_) => Err(decimal_reduce_error()),
        }
    };
}

/// Like [`dispatch!`], but rewraps `$build` (a fresh `FixedSerie<T>` of the current element type)
/// back into the matching [`SerieInner`] variant — for transforms that return a new column.
macro_rules! dispatch_rebuild {
    ($self:expr, $serie:ident => $build:expr) => {
        match &$self.inner {
            SerieInner::I8($serie) => SerieInner::I8($build),
            SerieInner::U8($serie) => SerieInner::U8($build),
            SerieInner::I16($serie) => SerieInner::I16($build),
            SerieInner::U16($serie) => SerieInner::U16($build),
            SerieInner::I32($serie) => SerieInner::I32($build),
            SerieInner::U32($serie) => SerieInner::U32($build),
            SerieInner::I64($serie) => SerieInner::I64($build),
            SerieInner::U64($serie) => SerieInner::U64($build),
            SerieInner::I128($serie) => SerieInner::I128($build),
            SerieInner::U128($serie) => SerieInner::U128($build),
            SerieInner::F32($serie) => SerieInner::F32($build),
            SerieInner::F64($serie) => SerieInner::F64($build),
            SerieInner::Bool($serie) => SerieInner::Bool($build),
            SerieInner::Decimal32($serie) => SerieInner::Decimal32($build),
            SerieInner::Decimal64($serie) => SerieInner::Decimal64($build),
            SerieInner::Decimal128($serie) => SerieInner::Decimal128($build),
            SerieInner::Decimal256($serie) => SerieInner::Decimal256($build),
        }
    };
}

/// Like [`dispatch!`], but binds `$serie` **mutably** — for the uniform in-place mutators that take
/// no per-element conversion (`setNull`). Every arm must yield the same type.
macro_rules! dispatch_mut {
    ($self:expr, $serie:ident => $body:expr) => {
        match &mut $self.inner {
            SerieInner::I8($serie) => $body,
            SerieInner::U8($serie) => $body,
            SerieInner::I16($serie) => $body,
            SerieInner::U16($serie) => $body,
            SerieInner::I32($serie) => $body,
            SerieInner::U32($serie) => $body,
            SerieInner::I64($serie) => $body,
            SerieInner::U64($serie) => $body,
            SerieInner::I128($serie) => $body,
            SerieInner::U128($serie) => $body,
            SerieInner::F32($serie) => $body,
            SerieInner::F64($serie) => $body,
            SerieInner::Bool($serie) => $body,
            SerieInner::Decimal32($serie) => $body,
            SerieInner::Decimal64($serie) => $body,
            SerieInner::Decimal128($serie) => $body,
            SerieInner::Decimal256($serie) => $body,
        }
    };
}

/// Like [`dispatch_mut!`], but also binds `$conv` to the active arm's JS-value→native converter (the
/// same per-arm converters [`fromValues`](Serie::from_values) uses — a decimal arm binds its unscaled
/// integer converter). The conversion is the runtime type check: the wrong JS shape throws the guided
/// `Error`. Shared by every value-taking in-place mutator (`set` / `setChecked` / `setRange` /
/// `setRangeChecked`), so the 17-way match + converter table is written once. Every arm must yield the
/// same type.
macro_rules! set_dispatch {
    ($self:expr, $serie:ident, $conv:ident => $body:expr) => {
        match &mut $self.inner {
            SerieInner::I8($serie) => {
                let $conv = to_i8;
                $body
            }
            SerieInner::U8($serie) => {
                let $conv = to_u8;
                $body
            }
            SerieInner::I16($serie) => {
                let $conv = to_i16;
                $body
            }
            SerieInner::U16($serie) => {
                let $conv = to_u16;
                $body
            }
            SerieInner::I32($serie) => {
                let $conv = to_i32;
                $body
            }
            SerieInner::U32($serie) => {
                let $conv = to_u32;
                $body
            }
            SerieInner::I64($serie) => {
                let $conv = to_i64;
                $body
            }
            SerieInner::U64($serie) => {
                let $conv = to_u64;
                $body
            }
            SerieInner::I128($serie) => {
                let $conv = to_i128;
                $body
            }
            SerieInner::U128($serie) => {
                let $conv = to_u128;
                $body
            }
            SerieInner::F32($serie) => {
                let $conv = to_f32;
                $body
            }
            SerieInner::F64($serie) => {
                let $conv = to_f64;
                $body
            }
            SerieInner::Bool($serie) => {
                let $conv = to_bool_native;
                $body
            }
            SerieInner::Decimal32($serie) => {
                let $conv = to_i32;
                $body
            }
            SerieInner::Decimal64($serie) => {
                let $conv = to_i64;
                $body
            }
            SerieInner::Decimal128($serie) => {
                let $conv = to_i128;
                $body
            }
            SerieInner::Decimal256($serie) => {
                let $conv = to_i256;
                $body
            }
        }
    };
}

/// Like [`dispatch!`], but only the four **decimal** variants bind `$serie` — every other element
/// type throws the guided `Error` (the method is decimal-only). `$body` must be a `napi::Result`.
macro_rules! dispatch_decimal {
    ($self:expr, $serie:ident => $body:expr) => {
        match &$self.inner {
            SerieInner::Decimal32($serie) => $body,
            SerieInner::Decimal64($serie) => $body,
            SerieInner::Decimal128($serie) => $body,
            SerieInner::Decimal256($serie) => $body,
            _ => Err(to_error(
                "this Serie is not a decimal column: toDecimalString / decimalPrecision / \
                 decimalScale need a decimal element type — build one with DataTypeId.Decimal128() \
                 and set its scale with withPrecisionScale(precision, scale)",
            )),
        }
    };
}

/// A **typed column** — many elements of one [`DataTypeId`](crate::datatype_id::DataTypeId) over a
/// data buffer, with an optional validity bit buffer for nulls. Built from a value list
/// ([`fromValues`](Serie::from_values)) or an option list ([`fromOptions`](Serie::from_options));
/// reads and reductions forward to the byte layer's vectorized kernels.
#[napi(namespace = "typed")]
pub struct Serie {
    inner: SerieInner,
}

#[napi(namespace = "typed")]
impl Serie {
    /// A **non-null** column of `values`, each an element of `dtype`. Narrow integers and floats
    /// arrive as JS `number`s, wide integers (`i64`/`u64`/`i128`/`u128`) as `BigInt`s, booleans as
    /// `boolean`s; a `DataTypeId.Unknown()` (raw bytes) has no typed column and throws.
    #[napi(factory)]
    pub fn from_values(
        values: Vec<Either3<f64, BigInt, bool>>,
        dtype: &DataTypeId,
    ) -> napi::Result<Serie> {
        let inner =
            match dtype.inner {
                DtId::I8 => SerieInner::I8(build_values(values, to_i8)?),
                DtId::U8 => SerieInner::U8(build_values(values, to_u8)?),
                DtId::I16 => SerieInner::I16(build_values(values, to_i16)?),
                DtId::U16 => SerieInner::U16(build_values(values, to_u16)?),
                DtId::I32 => SerieInner::I32(build_values(values, to_i32)?),
                DtId::U32 => SerieInner::U32(build_values(values, to_u32)?),
                DtId::I64 => SerieInner::I64(build_values(values, to_i64)?),
                DtId::U64 => SerieInner::U64(build_values(values, to_u64)?),
                DtId::I128 => SerieInner::I128(build_values(values, to_i128)?),
                DtId::U128 => SerieInner::U128(build_values(values, to_u128)?),
                DtId::F32 => SerieInner::F32(build_values(values, to_f32)?),
                DtId::F64 => SerieInner::F64(build_values(values, to_f64)?),
                DtId::Bool => SerieInner::Bool(build_values(values, to_bool_native)?),
                DtId::Decimal32 => SerieInner::Decimal32(build_values(values, to_i32)?),
                DtId::Decimal64 => SerieInner::Decimal64(build_values(values, to_i64)?),
                DtId::Decimal128 => SerieInner::Decimal128(build_values(values, to_i128)?),
                DtId::Decimal256 => SerieInner::Decimal256(build_values(values, to_i256)?),
                _ => return Err(to_error(
                    "this DataTypeId has no typed Serie: pass a concrete fixed-width element type \
                     (e.g. DataTypeId.I64())",
                )),
            };
        Ok(Serie { inner })
    }

    /// A **nullable** column of `values` — a `null` entry becomes a null element (building the
    /// validity bitmap). Non-null entries follow the same per-`dtype` shapes as
    /// [`fromValues`](Serie::from_values).
    #[napi(factory)]
    pub fn from_options(
        values: Vec<Option<Either3<f64, BigInt, bool>>>,
        dtype: &DataTypeId,
    ) -> napi::Result<Serie> {
        let inner =
            match dtype.inner {
                DtId::I8 => SerieInner::I8(build_options(values, to_i8)?),
                DtId::U8 => SerieInner::U8(build_options(values, to_u8)?),
                DtId::I16 => SerieInner::I16(build_options(values, to_i16)?),
                DtId::U16 => SerieInner::U16(build_options(values, to_u16)?),
                DtId::I32 => SerieInner::I32(build_options(values, to_i32)?),
                DtId::U32 => SerieInner::U32(build_options(values, to_u32)?),
                DtId::I64 => SerieInner::I64(build_options(values, to_i64)?),
                DtId::U64 => SerieInner::U64(build_options(values, to_u64)?),
                DtId::I128 => SerieInner::I128(build_options(values, to_i128)?),
                DtId::U128 => SerieInner::U128(build_options(values, to_u128)?),
                DtId::F32 => SerieInner::F32(build_options(values, to_f32)?),
                DtId::F64 => SerieInner::F64(build_options(values, to_f64)?),
                DtId::Bool => SerieInner::Bool(build_options(values, to_bool_native)?),
                DtId::Decimal32 => SerieInner::Decimal32(build_options(values, to_i32)?),
                DtId::Decimal64 => SerieInner::Decimal64(build_options(values, to_i64)?),
                DtId::Decimal128 => SerieInner::Decimal128(build_options(values, to_i128)?),
                DtId::Decimal256 => SerieInner::Decimal256(build_options(values, to_i256)?),
                _ => return Err(to_error(
                    "this DataTypeId has no typed Serie: pass a concrete fixed-width element type \
                     (e.g. DataTypeId.I64())",
                )),
            };
        Ok(Serie { inner })
    }

    /// A **non-null** column built by **flexibly** parsing each string in `strings` as an element of
    /// `dtype` — tolerant of thousands separators (`1,000` / `1_000`), a leading `+`, scientific
    /// notation (`1e3`), a radix prefix (`0xFF` / `0b1010` / `0o17`), and `inf` / `nan`. A string that
    /// does not parse throws the guided `Error`. `DataTypeId.Decimal256()` has **no** string parse
    /// (its 256-bit native does not parse from text) and throws — build it from its unscaled integer
    /// with `Serie.fromValues(..., DataTypeId.Decimal256())`. A non-fixed-width / byte dtype throws the
    /// "no typed Serie" error.
    #[napi(factory)]
    pub fn parse(strings: Vec<String>, dtype: &DataTypeId) -> napi::Result<Serie> {
        let refs = strings.iter().map(String::as_str).collect::<Vec<_>>();
        let inner = match dtype.inner {
            DtId::I8 => SerieInner::I8(FixedSerie::<Int8>::from_strings(&refs).map_err(to_error)?),
            DtId::U8 => SerieInner::U8(FixedSerie::<UInt8>::from_strings(&refs).map_err(to_error)?),
            DtId::I16 => {
                SerieInner::I16(FixedSerie::<Int16>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::U16 => {
                SerieInner::U16(FixedSerie::<UInt16>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::I32 => {
                SerieInner::I32(FixedSerie::<Int32>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::U32 => {
                SerieInner::U32(FixedSerie::<UInt32>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::I64 => {
                SerieInner::I64(FixedSerie::<Int64>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::U64 => {
                SerieInner::U64(FixedSerie::<UInt64>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::I128 => {
                SerieInner::I128(FixedSerie::<Int128>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::U128 => {
                SerieInner::U128(FixedSerie::<UInt128>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::F32 => {
                SerieInner::F32(FixedSerie::<Float32>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::F64 => {
                SerieInner::F64(FixedSerie::<Float64>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::Bool => {
                SerieInner::Bool(FixedSerie::<Bit>::from_strings(&refs).map_err(to_error)?)
            }
            DtId::Decimal32 => SerieInner::Decimal32(
                FixedSerie::<Decimal32>::from_strings(&refs).map_err(to_error)?,
            ),
            DtId::Decimal64 => SerieInner::Decimal64(
                FixedSerie::<Decimal64>::from_strings(&refs).map_err(to_error)?,
            ),
            DtId::Decimal128 => SerieInner::Decimal128(
                FixedSerie::<Decimal128>::from_strings(&refs).map_err(to_error)?,
            ),
            DtId::Decimal256 => {
                return Err(to_error(
                    "decimal256 has no string parse: build it from its unscaled integer with \
                     Serie.fromValues(..., DataTypeId.Decimal256())",
                ))
            }
            _ => {
                return Err(to_error(
                    "this DataTypeId has no typed Serie: pass a concrete fixed-width element type \
                     (e.g. DataTypeId.I64())",
                ))
            }
        };
        Ok(Serie { inner })
    }

    /// The **strict** twin of [`parse`](Serie::parse): builds a non-null column by parsing each
    /// string exactly (no thousands separators, no leading `+`, no radix prefix — only the canonical
    /// form the element type formats to). A string that does not parse strictly throws the guided
    /// `Error`. `DataTypeId.Decimal256()` throws the same "no string parse" error; a non-fixed-width /
    /// byte dtype throws "no typed Serie".
    #[napi(factory)]
    pub fn parse_exact(strings: Vec<String>, dtype: &DataTypeId) -> napi::Result<Serie> {
        let refs = strings.iter().map(String::as_str).collect::<Vec<_>>();
        let inner =
            match dtype.inner {
                DtId::I8 => {
                    SerieInner::I8(FixedSerie::<Int8>::from_strings_exact(&refs).map_err(to_error)?)
                }
                DtId::U8 => SerieInner::U8(
                    FixedSerie::<UInt8>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::I16 => SerieInner::I16(
                    FixedSerie::<Int16>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::U16 => SerieInner::U16(
                    FixedSerie::<UInt16>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::I32 => SerieInner::I32(
                    FixedSerie::<Int32>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::U32 => SerieInner::U32(
                    FixedSerie::<UInt32>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::I64 => SerieInner::I64(
                    FixedSerie::<Int64>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::U64 => SerieInner::U64(
                    FixedSerie::<UInt64>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::I128 => SerieInner::I128(
                    FixedSerie::<Int128>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::U128 => SerieInner::U128(
                    FixedSerie::<UInt128>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::F32 => SerieInner::F32(
                    FixedSerie::<Float32>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::F64 => SerieInner::F64(
                    FixedSerie::<Float64>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::Bool => SerieInner::Bool(
                    FixedSerie::<Bit>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::Decimal32 => SerieInner::Decimal32(
                    FixedSerie::<Decimal32>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::Decimal64 => SerieInner::Decimal64(
                    FixedSerie::<Decimal64>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::Decimal128 => SerieInner::Decimal128(
                    FixedSerie::<Decimal128>::from_strings_exact(&refs).map_err(to_error)?,
                ),
                DtId::Decimal256 => {
                    return Err(to_error(
                        "decimal256 has no string parse: build it from its unscaled integer with \
                     Serie.fromValues(..., DataTypeId.Decimal256())",
                    ))
                }
                _ => return Err(to_error(
                    "this DataTypeId has no typed Serie: pass a concrete fixed-width element type \
                     (e.g. DataTypeId.I64())",
                )),
            };
        Ok(Serie { inner })
    }

    /// The number of elements.
    #[napi]
    pub fn len(&self) -> u32 {
        dispatch!(self, serie => serie.len() as u32)
    }

    /// Whether the column has no elements.
    #[napi]
    pub fn is_empty(&self) -> bool {
        dispatch!(self, serie => serie.is_empty())
    }

    /// The element at `index` as a `number` / `BigInt` / `boolean`, or `null` when it is null or
    /// out of range.
    #[napi]
    pub fn get(&self, index: u32) -> Option<Either3<f64, BigInt, bool>> {
        let index = index as usize;
        dispatch!(self, serie => serie.get(index).map(|value| value.to_js()))
    }

    /// Every element as an optional value (a null element is `null`) — the null-aware list.
    #[napi]
    pub fn to_list(&self) -> Vec<Option<Either3<f64, BigInt, bool>>> {
        dispatch!(self, serie => serie
            .to_options()
            .into_iter()
            .map(|value| value.map(|value| value.to_js()))
            .collect())
    }

    /// The **raw** values (validity ignored — a null slot surfaces its stored default). Pair with
    /// [`isValid`](Serie::is_valid) for null-awareness.
    #[napi]
    pub fn values(&self) -> Vec<Either3<f64, BigInt, bool>> {
        dispatch!(self, serie => serie.values().into_iter().map(|value| value.to_js()).collect())
    }

    /// Every element formatted as a string (validity ignored — a null slot renders its stored
    /// default; pair with [`toStringOptions`](Serie::to_string_options) for null-awareness). Uses
    /// each element type's canonical format. For a **decimal** column this renders the raw
    /// **unscaled** integer (use [`toDecimalString`](Serie::to_decimal_string) for the scaled value).
    /// A `Decimal256` column has **no** string format and throws the guided `Error`.
    #[napi]
    pub fn to_strings(&self) -> napi::Result<Vec<String>> {
        match &self.inner {
            SerieInner::I8(s) => s.to_strings().map_err(to_error),
            SerieInner::U8(s) => s.to_strings().map_err(to_error),
            SerieInner::I16(s) => s.to_strings().map_err(to_error),
            SerieInner::U16(s) => s.to_strings().map_err(to_error),
            SerieInner::I32(s) => s.to_strings().map_err(to_error),
            SerieInner::U32(s) => s.to_strings().map_err(to_error),
            SerieInner::I64(s) => s.to_strings().map_err(to_error),
            SerieInner::U64(s) => s.to_strings().map_err(to_error),
            SerieInner::I128(s) => s.to_strings().map_err(to_error),
            SerieInner::U128(s) => s.to_strings().map_err(to_error),
            SerieInner::F32(s) => s.to_strings().map_err(to_error),
            SerieInner::F64(s) => s.to_strings().map_err(to_error),
            SerieInner::Bool(s) => s.to_strings().map_err(to_error),
            SerieInner::Decimal32(s) => s.to_strings().map_err(to_error),
            SerieInner::Decimal64(s) => s.to_strings().map_err(to_error),
            SerieInner::Decimal128(s) => s.to_strings().map_err(to_error),
            SerieInner::Decimal256(_) => Err(to_error(
                "decimal256 has no string format: read the unscaled value with get/values, or use \
                 toDecimalString for the scaled decimal",
            )),
        }
    }

    /// Every element as an optional string — a **null** element is `null`, a non-null element its
    /// canonical string. The null-aware twin of [`toStrings`](Serie::to_strings) (which renders the
    /// stored default in null slots). For a **decimal** column this renders the raw **unscaled**
    /// integer. A `Decimal256` column has **no** string format and throws the guided `Error`.
    #[napi]
    pub fn to_string_options(&self) -> napi::Result<Vec<Option<String>>> {
        match &self.inner {
            SerieInner::I8(s) => s.to_string_options().map_err(to_error),
            SerieInner::U8(s) => s.to_string_options().map_err(to_error),
            SerieInner::I16(s) => s.to_string_options().map_err(to_error),
            SerieInner::U16(s) => s.to_string_options().map_err(to_error),
            SerieInner::I32(s) => s.to_string_options().map_err(to_error),
            SerieInner::U32(s) => s.to_string_options().map_err(to_error),
            SerieInner::I64(s) => s.to_string_options().map_err(to_error),
            SerieInner::U64(s) => s.to_string_options().map_err(to_error),
            SerieInner::I128(s) => s.to_string_options().map_err(to_error),
            SerieInner::U128(s) => s.to_string_options().map_err(to_error),
            SerieInner::F32(s) => s.to_string_options().map_err(to_error),
            SerieInner::F64(s) => s.to_string_options().map_err(to_error),
            SerieInner::Bool(s) => s.to_string_options().map_err(to_error),
            SerieInner::Decimal32(s) => s.to_string_options().map_err(to_error),
            SerieInner::Decimal64(s) => s.to_string_options().map_err(to_error),
            SerieInner::Decimal128(s) => s.to_string_options().map_err(to_error),
            SerieInner::Decimal256(_) => Err(to_error(
                "decimal256 has no string format: read the unscaled value with get/values, or use \
                 toDecimalString for the scaled decimal",
            )),
        }
    }

    /// How many elements are null.
    #[napi]
    pub fn null_count(&self) -> u32 {
        dispatch!(self, serie => serie.null_count() as u32)
    }

    /// Whether the element at `index` is **null** (absent). Out of range is `false`.
    #[napi]
    pub fn is_null(&self, index: u32) -> bool {
        let index = index as usize;
        dispatch!(self, serie => serie.is_null(index))
    }

    /// Whether the element at `index` is **valid** (non-null). Out of range is `false`.
    #[napi]
    pub fn is_valid(&self, index: u32) -> bool {
        let index = index as usize;
        dispatch!(self, serie => serie.is_valid(index))
    }

    /// A copy of this column with its **name** set (the metadata its [`field`](Serie::field)
    /// reports).
    #[napi]
    pub fn with_name(&self, name: String) -> Serie {
        Serie {
            inner: dispatch_rebuild!(self, serie => rename(serie, &name)),
        }
    }

    /// A copy of this column with its decimal **precision** (max significant digits) and **scale**
    /// (decimal places) set — the metadata its [`field`](Serie::field) reports and
    /// [`toDecimalString`](Serie::to_decimal_string) uses to place the decimal point. Apply it to a
    /// decimal column (`DataTypeId.Decimal128()`, …).
    #[napi]
    pub fn with_precision_scale(&self, precision: u32, scale: i32) -> Serie {
        Serie {
            inner: dispatch_rebuild!(self, serie => reprecision(serie, precision, scale)),
        }
    }

    /// The element [`DataTypeId`](crate::datatype_id::DataTypeId) of this column.
    #[napi]
    pub fn dtype(&self) -> DataTypeId {
        dispatch!(self, serie => DataTypeId { inner: serie.data_type_id() })
    }

    /// This column's [`Field`] metadata — its `name`, element type, and `nullable` flag (a column
    /// with a validity buffer is nullable), plus its `precision` / `scale` for a decimal column.
    #[napi]
    pub fn field(&self) -> Field {
        dispatch!(self, serie => Field { inner: serie.field() })
    }

    /// A **field-driven cast** — reshapes this column to match `field`, returning a fresh column:
    ///
    /// - **Same dtype** as `field`'s: a pure metadata reshape — applies the target **nullability**
    ///   (non-nullable → nullable adds an all-valid validity buffer; nullable → non-nullable
    ///   requires zero nulls, else the guided `Error`), the target **name**, and any free-form
    ///   **annotations**, reusing the data backing (no element copy).
    /// - **Different fixed-width numeric / bool / decimal dtype**: reinterprets the data buffer at
    ///   the new element width (via the byte layer's `resize_dtype`, converting across families
    ///   through `f64` — a narrowing integer saturates), then applies `field`.
    /// - **A byte / string target** (`Binary` / `Utf8` / `FixedBinary` / `FixedUtf8`, or `Unknown`):
    ///   the guided `Error` — a numeric `Serie` cannot become a byte column (build a [`ByteSerie`]).
    #[napi]
    pub fn cast_field(&self, field: &Field) -> napi::Result<Serie> {
        let target = field.inner.data_type_id();
        let current = dispatch!(self, s => s.data_type_id());

        // Same dtype — a per-variant metadata reshape (nullability / name / annotations), no data copy.
        if target == current {
            let inner = dispatch_rebuild!(self, s => s.cast_field(&field.inner).map_err(to_error)?);
            return Ok(Serie { inner });
        }

        // A byte / string (or Unknown) target has no numeric column — refuse before touching the data.
        if !target.is_fixed_width() {
            return Err(to_error(format!(
                "cannot cast a numeric column to the {target} dtype: a byte/string target needs a \
                 ByteSerie, not a numeric Serie"
            )));
        }

        // A dtype **change** touching a `Bool` (bit-packed data) or `Decimal256` (256-bit, exceeds
        // the f64 carrier) does not survive the numeric `resize_dtype` — refuse it rather than
        // produce garbage. A same-dtype cast (bool→bool, decimal256→decimal256) is handled above.
        let unresizable = |dtype: DtId| dtype == DtId::Bool || dtype == DtId::Decimal256;
        if unresizable(current) || unresizable(target) {
            return Err(to_error(format!(
                "cannot cast between {current} and {target}: a bool (bit-packed) or decimal256 \
                 (256-bit) representation does not convert through the numeric resize — build the \
                 {target} column directly"
            )));
        }

        // A different fixed-width numeric / decimal (32/64/128) target: reinterpret the data buffer
        // at the new element width (through `resize_dtype`), then rebuild the matching column and
        // apply `field`. `resized` / `validity` / `len` move into whichever single arm the target
        // selects. The `Bool` / `Decimal256` arms are now unreachable for a dtype change (guarded
        // above) but stay in the match for soundness (`DataTypeId` is `#[non_exhaustive]`).
        let mut buf = dispatch!(self, s => s.data().clone());
        buf.set_dtype(current);
        let resized = buf.resize_dtype(target).map_err(to_error)?;
        let validity = dispatch!(self, s => s.validity().cloned());
        let len = dispatch!(self, s => s.len());
        let inner = match target {
            DtId::I8 => SerieInner::I8(
                FixedSerie::<Int8>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::U8 => SerieInner::U8(
                FixedSerie::<UInt8>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::I16 => SerieInner::I16(
                FixedSerie::<Int16>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::U16 => SerieInner::U16(
                FixedSerie::<UInt16>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::I32 => SerieInner::I32(
                FixedSerie::<Int32>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::U32 => SerieInner::U32(
                FixedSerie::<UInt32>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::I64 => SerieInner::I64(
                FixedSerie::<Int64>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::U64 => SerieInner::U64(
                FixedSerie::<UInt64>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::I128 => SerieInner::I128(
                FixedSerie::<Int128>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::U128 => SerieInner::U128(
                FixedSerie::<UInt128>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::F32 => SerieInner::F32(
                FixedSerie::<Float32>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::F64 => SerieInner::F64(
                FixedSerie::<Float64>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::Bool => SerieInner::Bool(
                FixedSerie::<Bit>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::Decimal32 => SerieInner::Decimal32(
                FixedSerie::<Decimal32>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::Decimal64 => SerieInner::Decimal64(
                FixedSerie::<Decimal64>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::Decimal128 => SerieInner::Decimal128(
                FixedSerie::<Decimal128>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            DtId::Decimal256 => SerieInner::Decimal256(
                FixedSerie::<Decimal256>::from_data(resized, validity, len)
                    .cast_field(&field.inner)
                    .map_err(to_error)?,
            ),
            _ => {
                unreachable!("target.is_fixed_width() guarantees one of the fixed-width arms above")
            }
        };
        Ok(Serie { inner })
    }

    /// The **unscaled** decimal value at `index` formatted with the column's scale (e.g. `"123.45"`
    /// at scale 2), or `null` when the element is null or out of range. Throws the guided `Error` on
    /// a non-decimal column.
    #[napi]
    pub fn to_decimal_string(&self, index: u32) -> napi::Result<Option<String>> {
        let index = index as usize;
        dispatch_decimal!(self, serie => Ok(serie.to_decimal_string(index)))
    }

    /// The decimal **precision** (max significant digits) — the set value, else the type's max.
    /// Throws the guided `Error` on a non-decimal column.
    #[napi]
    pub fn decimal_precision(&self) -> napi::Result<u32> {
        dispatch_decimal!(self, serie => Ok(serie.decimal_precision()))
    }

    /// The decimal **scale** (decimal places) — the set value, else `0`. Throws the guided `Error`
    /// on a non-decimal column.
    #[napi]
    pub fn decimal_scale(&self) -> napi::Result<i32> {
        dispatch_decimal!(self, serie => Ok(serie.decimal_scale()))
    }

    /// The **sum** of every element — a `BigInt` for an integer column (a wide accumulator never
    /// wraps), a `number` for a float column. Throws the guided `Error` on a boolean column.
    #[napi]
    pub fn sum(&self) -> napi::Result<Either<BigInt, f64>> {
        dispatch_numeric!(self, serie => serie.sum().map(|value| value.to_js_sum()).map_err(to_error))
    }

    /// The **minimum** element (a float min ignores NaN), or `null` when empty. Throws the guided
    /// `Error` on a boolean column.
    #[napi]
    pub fn min(&self) -> napi::Result<Option<Either3<f64, BigInt, bool>>> {
        dispatch_numeric!(self, serie => serie
            .min()
            .map(|value| value.map(|value| value.to_js()))
            .map_err(to_error))
    }

    /// The **maximum** element (a float max ignores NaN), or `null` when empty. Throws the guided
    /// `Error` on a boolean column.
    #[napi]
    pub fn max(&self) -> napi::Result<Option<Either3<f64, BigInt, bool>>> {
        dispatch_numeric!(self, serie => serie
            .max()
            .map(|value| value.map(|value| value.to_js()))
            .map_err(to_error))
    }

    /// The **mean** as a `number`, or `null` when empty. Throws the guided `Error` on a boolean
    /// column.
    #[napi]
    pub fn mean(&self) -> napi::Result<Option<f64>> {
        dispatch_numeric!(self, serie => serie.mean().map_err(to_error))
    }

    /// The **population standard deviation** as a `number` (the `sqrt` of the variance), or `null`
    /// when empty. Throws the guided `Error` on a boolean or decimal column.
    #[napi]
    pub fn std(&self) -> napi::Result<Option<f64>> {
        dispatch_numeric!(self, serie => serie.std().map_err(to_error))
    }

    /// The **population variance** as a `number` (`std²`), or `null` when empty. Throws the guided
    /// `Error` on a boolean or decimal column.
    #[napi]
    pub fn var(&self) -> napi::Result<Option<f64>> {
        dispatch_numeric!(self, serie => serie.var().map_err(to_error))
    }

    /// The **median** as a `number`, or `null` when empty. Throws the guided `Error` on a boolean or
    /// decimal column.
    #[napi]
    pub fn median(&self) -> napi::Result<Option<f64>> {
        dispatch_numeric!(self, serie => serie.median().map_err(to_error))
    }

    /// How many elements are **`>= threshold`** — the threshold crosses in the same shape the
    /// column's elements do (a `number` for a narrow integer or float, a `BigInt` for a wide
    /// integer), converted to the column's native element type. Throws the guided `Error` on a
    /// boolean or decimal column.
    #[napi]
    pub fn count_ge(&self, threshold: Either3<f64, BigInt, bool>) -> napi::Result<u32> {
        let count = match &self.inner {
            SerieInner::I8(serie) => serie.count_ge(to_i8(threshold)?),
            SerieInner::U8(serie) => serie.count_ge(to_u8(threshold)?),
            SerieInner::I16(serie) => serie.count_ge(to_i16(threshold)?),
            SerieInner::U16(serie) => serie.count_ge(to_u16(threshold)?),
            SerieInner::I32(serie) => serie.count_ge(to_i32(threshold)?),
            SerieInner::U32(serie) => serie.count_ge(to_u32(threshold)?),
            SerieInner::I64(serie) => serie.count_ge(to_i64(threshold)?),
            SerieInner::U64(serie) => serie.count_ge(to_u64(threshold)?),
            SerieInner::I128(serie) => serie.count_ge(to_i128(threshold)?),
            SerieInner::U128(serie) => serie.count_ge(to_u128(threshold)?),
            SerieInner::F32(serie) => serie.count_ge(to_f32(threshold)?),
            SerieInner::F64(serie) => serie.count_ge(to_f64(threshold)?),
            SerieInner::Bool(_) => return Err(bool_reduce_error()),
            SerieInner::Decimal32(_)
            | SerieInner::Decimal64(_)
            | SerieInner::Decimal128(_)
            | SerieInner::Decimal256(_) => return Err(decimal_reduce_error()),
        };
        count.map(|value| value as u32).map_err(to_error)
    }

    /// The **total** element count (nulls included) — an alias of `len`. Works for every element
    /// type (no throw).
    #[napi]
    pub fn count(&self) -> u32 {
        dispatch!(self, serie => serie.count() as u32)
    }

    /// The count of **non-null** elements (`len - nullCount`). Works for every element type.
    #[napi]
    pub fn valid_count(&self) -> u32 {
        dispatch!(self, serie => serie.valid_count() as u32)
    }

    /// The count of **distinct non-null** values. Works for every element type; a float column
    /// counts distinct by IEEE-754 bit pattern (the core `n_unique` needs `Eq + Hash`, which
    /// `f32`/`f64` lack).
    #[napi]
    pub fn n_unique(&self) -> u32 {
        let count = match &self.inner {
            SerieInner::I8(serie) => serie.n_unique(),
            SerieInner::U8(serie) => serie.n_unique(),
            SerieInner::I16(serie) => serie.n_unique(),
            SerieInner::U16(serie) => serie.n_unique(),
            SerieInner::I32(serie) => serie.n_unique(),
            SerieInner::U32(serie) => serie.n_unique(),
            SerieInner::I64(serie) => serie.n_unique(),
            SerieInner::U64(serie) => serie.n_unique(),
            SerieInner::I128(serie) => serie.n_unique(),
            SerieInner::U128(serie) => serie.n_unique(),
            SerieInner::F32(serie) => n_unique_f32(serie),
            SerieInner::F64(serie) => n_unique_f64(serie),
            SerieInner::Bool(serie) => serie.n_unique(),
            SerieInner::Decimal32(serie) => serie.n_unique(),
            SerieInner::Decimal64(serie) => serie.n_unique(),
            SerieInner::Decimal128(serie) => serie.n_unique(),
            SerieInner::Decimal256(serie) => serie.n_unique(),
        };
        count as u32
    }

    /// The **first** element value (null-aware, at index 0) as a `number` / `BigInt` / `boolean`, or
    /// `null` when empty or the element is null. Works for every element type.
    #[napi]
    pub fn first_value(&self) -> Option<Either3<f64, BigInt, bool>> {
        dispatch!(self, serie => serie.first_value().map(|value| value.to_js()))
    }

    /// The **last** element value (null-aware, at `len - 1`) as a `number` / `BigInt` / `boolean`,
    /// or `null` when empty or the element is null. Works for every element type.
    #[napi]
    pub fn last_value(&self) -> Option<Either3<f64, BigInt, bool>> {
        dispatch!(self, serie => serie.last_value().map(|value| value.to_js()))
    }

    /// A fresh column keeping only the elements where `mask` is truthy — `mask` is either an array
    /// of booleans (`1` = keep) or a boolean [`Serie`]. Throws when a `Serie` mask is not a boolean
    /// column.
    #[napi]
    pub fn filter(&self, mask: Either<Vec<bool>, &Serie>) -> napi::Result<Serie> {
        let mask_heap = build_mask(mask)?;
        Ok(Serie {
            inner: dispatch_rebuild!(self, serie => serie.filter(&mask_heap)),
        })
    }

    /// Replaces the element at `index` **in place** (marking the slot valid on a nullable column) —
    /// `value` crosses in the same shape the column's elements do (a `number` for a narrow integer or
    /// float, a `BigInt` for a wide integer, a `boolean` for a bool; a decimal's *unscaled* integer),
    /// converted to the column's native type. Throws the guided `Error` on the wrong JS shape, or on an
    /// `index` past the end (set within `0..len`, or rebuild to grow).
    #[napi]
    pub fn set(&mut self, index: u32, value: Either3<f64, BigInt, bool>) -> napi::Result<()> {
        let index = index as usize;
        set_dispatch!(self, serie, conv => serie.set(index, conv(value)?)).map_err(to_error)
    }

    /// The **unchecked fast path** of [`set`](Serie::set): the same conversion and slot write with
    /// **no bounds check** (the caller guarantees `index < len`). An out-of-range `index` is a silent
    /// logic error that writes past the column — use [`set`](Serie::set) unless the index is validated.
    #[napi]
    pub fn set_checked(
        &mut self,
        index: u32,
        value: Either3<f64, BigInt, bool>,
    ) -> napi::Result<()> {
        let index = index as usize;
        set_dispatch!(self, serie, conv => serie.set_checked(index, conv(value)?));
        Ok(())
    }

    /// **Nulls** the element at `index` (ensuring a validity buffer exists, back-filling existing
    /// elements as valid on the first null). Throws the guided `Error` on an `index` past the end.
    #[napi]
    pub fn set_null(&mut self, index: u32) -> napi::Result<()> {
        let index = index as usize;
        dispatch_mut!(self, serie => serie.set_null(index)).map_err(to_error)
    }

    /// A fresh sub-column copying elements `[start, start + len)` — the window is **clamped** to the
    /// column's length (an out-of-range window yields a shorter or empty column, never an error).
    #[napi]
    pub fn slice(&self, start: u32, len: u32) -> Serie {
        let (start, len) = (start as usize, len as usize);
        Serie {
            inner: dispatch_rebuild!(self, serie => serie.slice(start, len)),
        }
    }

    /// **Bulk in-place replace** of `values.len()` elements starting at `start` (marking the range
    /// valid on a nullable column) — each element crosses in the column's element shape, converted to
    /// its native type. Throws the guided `Error` on the wrong JS shape, or when the window
    /// `start + values.len()` runs past the end.
    #[napi]
    pub fn set_range(
        &mut self,
        start: u32,
        values: Vec<Either3<f64, BigInt, bool>>,
    ) -> napi::Result<()> {
        let start = start as usize;
        set_dispatch!(self, serie, conv => serie.set_range(start, &convert_values(values, conv)?))
            .map_err(to_error)
    }

    /// The **unchecked bulk twin** of [`setRange`](Serie::set_range): the same dense vectorized write
    /// with **no bounds check** (the caller guarantees `start + values.len() <= len`). An out-of-range
    /// window is a silent logic error that writes past the column.
    #[napi]
    pub fn set_range_checked(
        &mut self,
        start: u32,
        values: Vec<Either3<f64, BigInt, bool>>,
    ) -> napi::Result<()> {
        let start = start as usize;
        set_dispatch!(self, serie, conv => serie.set_range_checked(start, &convert_values(values, conv)?));
        Ok(())
    }

    /// Sets the range `[start, start + other.len())` from **another column's values and validity** —
    /// `other` must have the same element type as this column (else the guided dtype-mismatch
    /// `Error`). Throws when the window `start + other.len()` runs past the end.
    #[napi]
    pub fn set_range_serie(&mut self, start: u32, other: &Serie) -> napi::Result<()> {
        let start = start as usize;
        let self_dtype = dispatch!(self, serie => serie.data_type_id());
        let other_dtype = dispatch!(other, serie => serie.data_type_id());
        match (&mut self.inner, &other.inner) {
            (SerieInner::I8(dst), SerieInner::I8(src)) => dst.set_range_serie(start, src),
            (SerieInner::U8(dst), SerieInner::U8(src)) => dst.set_range_serie(start, src),
            (SerieInner::I16(dst), SerieInner::I16(src)) => dst.set_range_serie(start, src),
            (SerieInner::U16(dst), SerieInner::U16(src)) => dst.set_range_serie(start, src),
            (SerieInner::I32(dst), SerieInner::I32(src)) => dst.set_range_serie(start, src),
            (SerieInner::U32(dst), SerieInner::U32(src)) => dst.set_range_serie(start, src),
            (SerieInner::I64(dst), SerieInner::I64(src)) => dst.set_range_serie(start, src),
            (SerieInner::U64(dst), SerieInner::U64(src)) => dst.set_range_serie(start, src),
            (SerieInner::I128(dst), SerieInner::I128(src)) => dst.set_range_serie(start, src),
            (SerieInner::U128(dst), SerieInner::U128(src)) => dst.set_range_serie(start, src),
            (SerieInner::F32(dst), SerieInner::F32(src)) => dst.set_range_serie(start, src),
            (SerieInner::F64(dst), SerieInner::F64(src)) => dst.set_range_serie(start, src),
            (SerieInner::Bool(dst), SerieInner::Bool(src)) => dst.set_range_serie(start, src),
            (SerieInner::Decimal32(dst), SerieInner::Decimal32(src)) => {
                dst.set_range_serie(start, src)
            }
            (SerieInner::Decimal64(dst), SerieInner::Decimal64(src)) => {
                dst.set_range_serie(start, src)
            }
            (SerieInner::Decimal128(dst), SerieInner::Decimal128(src)) => {
                dst.set_range_serie(start, src)
            }
            (SerieInner::Decimal256(dst), SerieInner::Decimal256(src)) => {
                dst.set_range_serie(start, src)
            }
            _ => {
                return Err(to_error(format!(
                    "dtype mismatch: cannot set an {self_dtype} range from a {other_dtype} column"
                )))
            }
        }
        .map_err(to_error)
    }

    /// A short debug string — the column's name, element type, length, and null count.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        dispatch!(self, serie => format!(
            "Serie(name={:?}, dtype={}, len={}, nullCount={})",
            serie.field().name(),
            serie.data_type_id(),
            serie.len(),
            serie.null_count()
        ))
    }
}

/// Builds the bit-packed mask buffer a core `filter` reads: an array of booleans packs into a fresh
/// heap (LSB-first), a boolean [`Serie`] reuses its already-bit-packed data buffer.
fn build_mask(mask: Either<Vec<bool>, &Serie>) -> napi::Result<CoreHeap> {
    match mask {
        Either::A(bits) => {
            let mut heap = CoreHeap::new();
            for (index, &keep) in bits.iter().enumerate() {
                heap.pwrite_bit(index as u64, keep).map_err(to_error)?;
            }
            Ok(heap)
        }
        Either::B(serie) => match &serie.inner {
            SerieInner::Bool(mask) => Ok(mask.data().clone()),
            _ => Err(to_error(
                "filter mask Serie must be a boolean column: pass an array of booleans or build \
                 the mask with DataTypeId.Bool()",
            )),
        },
    }
}

/// A **typed column's metadata** — its `name`, element [`DataTypeId`](crate::datatype_id::DataTypeId),
/// and `nullable` flag, carried in a [`Headers`](crate::headers::Headers) map. Mirrors
/// `yggdryl_core::typed::HeaderField`.
#[napi(namespace = "typed")]
pub struct Field {
    inner: HeaderField,
}

#[napi(namespace = "typed")]
impl Field {
    /// A field from its `name` (optional — `null` for an unnamed field), element `dtype`, and
    /// `nullable` flag.
    #[napi(constructor)]
    pub fn new(name: Option<String>, dtype: &DataTypeId, nullable: bool) -> Field {
        Field {
            inner: HeaderField::new(name.as_deref(), dtype.inner, nullable),
        }
    }

    /// The column name, or `null` when unset.
    #[napi]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The element [`DataTypeId`](crate::datatype_id::DataTypeId).
    #[napi]
    pub fn dtype(&self) -> DataTypeId {
        DataTypeId {
            inner: self.inner.data_type_id(),
        }
    }

    /// Whether the column admits nulls.
    #[napi]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The decimal **precision** (max significant digits) this field carries, or `null` when it does
    /// not describe a decimal column.
    #[napi]
    pub fn precision(&self) -> Option<u32> {
        self.inner.precision()
    }

    /// The decimal **scale** (decimal places) this field carries, or `null` when it does not
    /// describe a decimal column.
    #[napi]
    pub fn scale(&self) -> Option<i32> {
        self.inner.scale()
    }

    /// The fixed element **byte width** this field carries (for a fixed-size byte column —
    /// `FixedBinary` / `FixedUtf8`), or `null` when it does not describe one.
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width()
    }

    /// The backing metadata [`Headers`](crate::headers::Headers) — **a copy** (the name / type /
    /// nullable live here, alongside any extra annotations).
    #[napi]
    pub fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// The value of one arbitrary metadata annotation `key`, or `null` when absent — reads the
    /// backing [`Headers`](crate::headers::Headers) map (the promoted `name` / `type` / `nullable`
    /// entries are visible through it too).
    #[napi]
    pub fn metadata(&self, key: String) -> Option<String> {
        self.inner.metadata_value(&key)
    }

    /// Sets one arbitrary metadata annotation `key` to `value` **in place** (replace semantics).
    #[napi]
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.inner.set_metadata(&key, &value);
    }

    /// A copy of this field with the annotation `key` set to `value` — the chainable, non-mutating
    /// counterpart of [`setMetadata`](Field::set_metadata).
    #[napi]
    pub fn with_metadata(&self, key: String, value: String) -> Field {
        Field {
            inner: self.inner.clone().with_metadata(&key, &value),
        }
    }

    /// Sets the field **name** in place (the promoted `name` metadata entry).
    #[napi]
    pub fn set_name(&mut self, name: String) {
        self.inner.set_name(&name);
    }

    /// Sets whether the field admits nulls **in place** (the promoted `nullable` metadata entry).
    #[napi]
    pub fn set_nullable(&mut self, nullable: bool) {
        self.inner.set_nullable(nullable);
    }

    /// A copy of this field with its **nullable** flag set — the chainable, non-mutating counterpart
    /// of [`setNullable`](Field::set_nullable).
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Field {
        Field {
            inner: self.inner.clone().with_nullable(nullable),
        }
    }

    /// Content equality — equal iff the backing metadata maps are equal.
    #[napi]
    pub fn equals(&self, other: &Field) -> bool {
        self.inner == other.inner
    }

    /// A short debug string — `Field(name=…, dtype=…, nullable=…)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "Field(name={:?}, dtype={}, nullable={})",
            self.inner.name(),
            self.inner.data_type_id(),
            self.inner.nullable()
        )
    }
}

// ---- the byte column: value marshalling ------------------------------------------------

/// A native byte-column element rendered as its JS value — a `Buffer` for a binary element
/// (`Vec<u8>`), a `string` for a UTF-8 element (`String`).
trait ToJsElement {
    /// This owned element as its JS shape (`Buffer` / `string`).
    fn to_js_element(self) -> Either<Buffer, String>;
}

impl ToJsElement for Vec<u8> {
    fn to_js_element(self) -> Either<Buffer, String> {
        Either::A(Buffer::from(self))
    }
}

impl ToJsElement for String {
    fn to_js_element(self) -> Either<Buffer, String> {
        Either::B(self)
    }
}

/// Extracts a JS `Buffer` element as raw bytes, or throws a guided `Error` (a binary column takes a
/// `Buffer`, not a string).
fn as_binary_element(value: Either<Buffer, String>) -> napi::Result<Vec<u8>> {
    match value {
        Either::A(buffer) => Ok(buffer.to_vec()),
        Either::B(_) => Err(to_error(
            "expected a Buffer element for a binary column: pass a Buffer (a string suits only \
             utf8 / fixed_utf8)",
        )),
    }
}

/// Extracts a JS `string` element, or throws a guided `Error` (a utf8 column takes a string, not a
/// `Buffer`).
fn as_utf8_element(value: Either<Buffer, String>) -> napi::Result<String> {
    match value {
        Either::B(text) => Ok(text),
        Either::A(_) => Err(to_error(
            "expected a string element for a utf8 column: pass a string (a Buffer suits only \
             binary / fixed_binary)",
        )),
    }
}

/// Every element as raw bytes for a binary column (each must be a `Buffer`).
fn binary_values(values: Vec<Either<Buffer, String>>) -> napi::Result<Vec<Vec<u8>>> {
    values.into_iter().map(as_binary_element).collect()
}

/// Every element as a string for a utf8 column (each must be a `string`).
fn utf8_values(values: Vec<Either<Buffer, String>>) -> napi::Result<Vec<String>> {
    values.into_iter().map(as_utf8_element).collect()
}

/// Like [`binary_values`], null-aware — a `null` entry becomes a null element.
fn binary_options(
    values: Vec<Option<Either<Buffer, String>>>,
) -> napi::Result<Vec<Option<Vec<u8>>>> {
    values
        .into_iter()
        .map(|value| value.map(as_binary_element).transpose())
        .collect()
}

/// Like [`utf8_values`], null-aware — a `null` entry becomes a null element.
fn utf8_options(values: Vec<Option<Either<Buffer, String>>>) -> napi::Result<Vec<Option<String>>> {
    values
        .into_iter()
        .map(|value| value.map(as_utf8_element).transpose())
        .collect()
}

/// The fixed element width for a fixed-size column, or a guided `Error` when it is absent.
fn require_width(width: Option<u32>) -> napi::Result<usize> {
    match width {
        Some(width) => Ok(width as usize),
        None => Err(to_error(
            "a fixed-size column needs a width: pass the fixed element byte length for a \
             fixed_binary / fixed_utf8 column",
        )),
    }
}

/// Refuses a `width` on a variable-length column with a guided `Error`.
fn reject_width(width: Option<u32>) -> napi::Result<()> {
    match width {
        None => Ok(()),
        Some(_) => Err(to_error(
            "a variable-length column takes no width: drop the width argument for a binary / utf8 \
             column",
        )),
    }
}

/// The guided `Error` a non-byte [`DataTypeId`](crate::datatype_id::DataTypeId) raises when passed to
/// a [`ByteSerie`] builder.
fn byte_dtype_error() -> napi::Error {
    to_error(
        "this DataTypeId is not a byte column: expected Binary, Utf8, FixedBinary, or FixedUtf8 — \
         use Serie for numeric/decimal/bool columns",
    )
}

/// The guided `Error` a **variable-length** byte column (`Binary` / `Utf8`) raises for an in-place
/// `set` — replacing one element with a different-length value would rewrite the whole tail, so the
/// variable-length carrier is append-only.
fn var_set_error() -> napi::Error {
    to_error(
        "a variable-length column is append-only: in-place set needs a fixed_binary / fixed_utf8 \
         column (a variable element would rewrite the tail)",
    )
}

/// A byte carrier that clones its bytes into a fresh column carrying a new `name` — the shared body
/// of [`ByteSerie::with_name`], written once per carrier layout (offsets+data for the variable-length
/// carrier, a single data buffer for the fixed-size one).
trait RebuildNamed {
    /// A fresh column over clones of this one's buffers, with `name` set.
    fn rebuild_named(&self, name: &str) -> Self;
}

impl<T: VarLenType> RebuildNamed for VarSerie<T> {
    fn rebuild_named(&self, name: &str) -> Self {
        VarSerie::from_parts(
            self.offsets().clone(),
            self.data().clone(),
            self.validity().cloned(),
            self.len(),
        )
        .with_name(name)
    }
}

impl<T: VarType> RebuildNamed for FixedSizeSerie<T> {
    fn rebuild_named(&self, name: &str) -> Self {
        FixedSizeSerie::from_parts(
            self.data().clone(),
            self.validity().cloned(),
            self.len(),
            self.width(),
        )
        .with_name(name)
    }
}

// ---- the erased byte column + its dispatch ---------------------------------------------

/// A byte column for each of the six byte carriers — the type-erased backing of [`ByteSerie`]. The
/// variable-length carriers come in two offset widths: `Binary` / `Utf8` (`i32` offsets) and their
/// **large** twins `LargeBinary` / `LargeUtf8` (`i64` offsets, for data past the `i32` offset range);
/// the fixed-size carriers (`FixedBinary` / `FixedUtf8`) have no offsets.
enum ByteInner {
    Binary(VarSerie<Binary>),
    Utf8(VarSerie<Utf8>),
    LargeBinary(VarSerie<LargeBinary>),
    LargeUtf8(VarSerie<LargeUtf8>),
    FixedBinary(FixedSizeSerie<FixedBinary>),
    FixedUtf8(FixedSizeSerie<FixedUtf8>),
}

/// Runs `$body` against the inner byte column (`$serie`) of whichever variant is present — the
/// six-way match, written once. Every arm must yield the same type.
macro_rules! byte_dispatch {
    ($self:expr, $serie:ident => $body:expr) => {
        match &$self.inner {
            ByteInner::Binary($serie) => $body,
            ByteInner::Utf8($serie) => $body,
            ByteInner::LargeBinary($serie) => $body,
            ByteInner::LargeUtf8($serie) => $body,
            ByteInner::FixedBinary($serie) => $body,
            ByteInner::FixedUtf8($serie) => $body,
        }
    };
}

/// Like [`byte_dispatch!`], but rewraps `$build` (a fresh column of the current carrier) back into
/// the matching [`ByteInner`] variant — for transforms that return a new column.
macro_rules! byte_rebuild {
    ($self:expr, $serie:ident => $build:expr) => {
        match &$self.inner {
            ByteInner::Binary($serie) => ByteInner::Binary($build),
            ByteInner::Utf8($serie) => ByteInner::Utf8($build),
            ByteInner::LargeBinary($serie) => ByteInner::LargeBinary($build),
            ByteInner::LargeUtf8($serie) => ByteInner::LargeUtf8($build),
            ByteInner::FixedBinary($serie) => ByteInner::FixedBinary($build),
            ByteInner::FixedUtf8($serie) => ByteInner::FixedUtf8($build),
        }
    };
}

/// A **byte-blob typed column** — the variable-length / fixed-size counterpart of [`Serie`], over the
/// six byte carriers: variable-length `Binary` (an `i32` offsets + data buffer) / `Utf8`, their
/// **large** twins `LargeBinary` / `LargeUtf8` (`i64` offsets, for data past the `i32` offset range),
/// and fixed-size `FixedBinary` / `FixedUtf8` (a fixed byte stride). A binary element crosses as a JS
/// `Buffer`, a UTF-8 element as a JS `string`; a null element is `null`. Built from a value list
/// ([`fromValues`](ByteSerie::from_values)) or an option list
/// ([`fromOptions`](ByteSerie::from_options)).
#[napi(namespace = "typed")]
pub struct ByteSerie {
    inner: ByteInner,
}

#[napi(namespace = "typed")]
impl ByteSerie {
    /// A **non-null** byte column of `values`, each an element of `dtype` (`DataTypeId.Binary()`,
    /// `Utf8()`, `LargeBinary()`, `LargeUtf8()`, `FixedBinary()`, `FixedUtf8()`). Binary elements
    /// arrive as `Buffer`s, UTF-8 elements as `string`s. A fixed-size `dtype` requires `width` (the
    /// fixed element byte length); a variable-length one (including the large twins) takes no `width`.
    #[napi(factory)]
    pub fn from_values(
        values: Vec<Either<Buffer, String>>,
        dtype: &DataTypeId,
        width: Option<u32>,
    ) -> napi::Result<ByteSerie> {
        let inner = match dtype.inner {
            DtId::Binary => {
                reject_width(width)?;
                ByteInner::Binary(VarSerie::<Binary>::from_values(&binary_values(values)?))
            }
            DtId::Utf8 => {
                reject_width(width)?;
                ByteInner::Utf8(VarSerie::<Utf8>::from_values(&utf8_values(values)?))
            }
            DtId::LargeBinary => {
                reject_width(width)?;
                ByteInner::LargeBinary(VarSerie::<LargeBinary>::from_values(&binary_values(
                    values,
                )?))
            }
            DtId::LargeUtf8 => {
                reject_width(width)?;
                ByteInner::LargeUtf8(VarSerie::<LargeUtf8>::from_values(&utf8_values(values)?))
            }
            DtId::FixedBinary => {
                let width = require_width(width)?;
                ByteInner::FixedBinary(FixedSizeSerie::<FixedBinary>::from_values(
                    width,
                    &binary_values(values)?,
                ))
            }
            DtId::FixedUtf8 => {
                let width = require_width(width)?;
                ByteInner::FixedUtf8(FixedSizeSerie::<FixedUtf8>::from_values(
                    width,
                    &utf8_values(values)?,
                ))
            }
            _ => return Err(byte_dtype_error()),
        };
        Ok(ByteSerie { inner })
    }

    /// A **nullable** byte column of `values` — a `null` entry becomes a null element. Non-null
    /// entries follow the same per-`dtype` shapes as [`fromValues`](ByteSerie::from_values).
    #[napi(factory)]
    pub fn from_options(
        values: Vec<Option<Either<Buffer, String>>>,
        dtype: &DataTypeId,
        width: Option<u32>,
    ) -> napi::Result<ByteSerie> {
        let inner = match dtype.inner {
            DtId::Binary => {
                reject_width(width)?;
                ByteInner::Binary(VarSerie::<Binary>::from_options(&binary_options(values)?))
            }
            DtId::Utf8 => {
                reject_width(width)?;
                ByteInner::Utf8(VarSerie::<Utf8>::from_options(&utf8_options(values)?))
            }
            DtId::LargeBinary => {
                reject_width(width)?;
                ByteInner::LargeBinary(VarSerie::<LargeBinary>::from_options(&binary_options(
                    values,
                )?))
            }
            DtId::LargeUtf8 => {
                reject_width(width)?;
                ByteInner::LargeUtf8(VarSerie::<LargeUtf8>::from_options(&utf8_options(values)?))
            }
            DtId::FixedBinary => {
                let width = require_width(width)?;
                ByteInner::FixedBinary(FixedSizeSerie::<FixedBinary>::from_options(
                    width,
                    &binary_options(values)?,
                ))
            }
            DtId::FixedUtf8 => {
                let width = require_width(width)?;
                ByteInner::FixedUtf8(FixedSizeSerie::<FixedUtf8>::from_options(
                    width,
                    &utf8_options(values)?,
                ))
            }
            _ => return Err(byte_dtype_error()),
        };
        Ok(ByteSerie { inner })
    }

    /// The number of elements.
    #[napi]
    pub fn len(&self) -> u32 {
        byte_dispatch!(self, serie => serie.len() as u32)
    }

    /// Whether the column has no elements.
    #[napi]
    pub fn is_empty(&self) -> bool {
        byte_dispatch!(self, serie => serie.is_empty())
    }

    /// The element at `index` as a `Buffer` (binary) or `string` (utf8), or `null` when it is null
    /// or out of range.
    #[napi]
    pub fn get(&self, index: u32) -> Option<Either<Buffer, String>> {
        let index = index as usize;
        byte_dispatch!(self, serie => serie.get(index).map(|value| value.to_js_element()))
    }

    /// Every element as an optional value (a null element is `null`) — the null-aware list.
    #[napi]
    pub fn to_list(&self) -> Vec<Option<Either<Buffer, String>>> {
        byte_dispatch!(self, serie => serie
            .to_options()
            .into_iter()
            .map(|value| value.map(|value| value.to_js_element()))
            .collect())
    }

    /// The **raw** values (validity ignored — a null slot surfaces its stored bytes). Pair with
    /// [`isValid`](ByteSerie::is_valid) for null-awareness.
    #[napi]
    pub fn values(&self) -> Vec<Either<Buffer, String>> {
        byte_dispatch!(self, serie => serie
            .values()
            .into_iter()
            .map(|value| value.to_js_element())
            .collect())
    }

    /// How many elements are null.
    #[napi]
    pub fn null_count(&self) -> u32 {
        byte_dispatch!(self, serie => serie.null_count() as u32)
    }

    /// Whether the element at `index` is **null** (absent). Out of range is `false`.
    #[napi]
    pub fn is_null(&self, index: u32) -> bool {
        let index = index as usize;
        byte_dispatch!(self, serie => serie.is_null(index))
    }

    /// Whether the element at `index` is **valid** (non-null). Out of range is `false`.
    #[napi]
    pub fn is_valid(&self, index: u32) -> bool {
        let index = index as usize;
        byte_dispatch!(self, serie => serie.is_valid(index))
    }

    /// The **total** element count (nulls included) — an alias of `len`.
    #[napi]
    pub fn count(&self) -> u32 {
        byte_dispatch!(self, serie => serie.count() as u32)
    }

    /// The count of **non-null** elements (`len - nullCount`).
    #[napi]
    pub fn valid_count(&self) -> u32 {
        byte_dispatch!(self, serie => serie.valid_count() as u32)
    }

    /// The count of **distinct non-null** values (a binary element by its bytes, a utf8 element by
    /// its string).
    #[napi]
    pub fn n_unique(&self) -> u32 {
        byte_dispatch!(self, serie => serie.n_unique() as u32)
    }

    /// The **first** element (null-aware, at index 0) as a `Buffer` (binary) / `string` (utf8), or
    /// `null` when empty or the element is null.
    #[napi]
    pub fn first_value(&self) -> Option<Either<Buffer, String>> {
        byte_dispatch!(self, serie => serie.first_value().map(|value| value.to_js_element()))
    }

    /// The **last** element (null-aware, at `len - 1`) as a `Buffer` / `string`, or `null` when
    /// empty or the element is null.
    #[napi]
    pub fn last_value(&self) -> Option<Either<Buffer, String>> {
        byte_dispatch!(self, serie => serie.last_value().map(|value| value.to_js_element()))
    }

    /// The **lexicographic minimum** over non-null values (a binary element ordered by its bytes, a
    /// utf8 element by its string), or `null` when there are no non-null values.
    #[napi]
    pub fn min_value(&self) -> Option<Either<Buffer, String>> {
        byte_dispatch!(self, serie => serie.min_value().map(|value| value.to_js_element()))
    }

    /// The **lexicographic maximum** over non-null values, or `null` when there are no non-null
    /// values.
    #[napi]
    pub fn max_value(&self) -> Option<Either<Buffer, String>> {
        byte_dispatch!(self, serie => serie.max_value().map(|value| value.to_js_element()))
    }

    /// The element [`DataTypeId`](crate::datatype_id::DataTypeId) of this column.
    #[napi]
    pub fn dtype(&self) -> DataTypeId {
        byte_dispatch!(self, serie => DataTypeId { inner: serie.data_type_id() })
    }

    /// The fixed element **byte width** for a fixed-size column (`FixedBinary` / `FixedUtf8`), or
    /// `null` for a variable-length one (`Binary` / `Utf8` / `LargeBinary` / `LargeUtf8`).
    #[napi]
    pub fn width(&self) -> Option<u32> {
        match &self.inner {
            ByteInner::Binary(_)
            | ByteInner::Utf8(_)
            | ByteInner::LargeBinary(_)
            | ByteInner::LargeUtf8(_) => None,
            ByteInner::FixedBinary(serie) => Some(serie.width() as u32),
            ByteInner::FixedUtf8(serie) => Some(serie.width() as u32),
        }
    }

    /// A copy of this column with its **name** set — a fresh column sharing the same bytes (the
    /// metadata its [`field`](ByteSerie::field) reports).
    #[napi]
    pub fn with_name(&self, name: String) -> ByteSerie {
        ByteSerie {
            inner: byte_rebuild!(self, serie => serie.rebuild_named(&name)),
        }
    }

    /// A copy of this **variable-length** byte column (`Binary` / `Utf8` / `LargeBinary` /
    /// `LargeUtf8`) with its optional **max
    /// element byte width** set — a fresh column sharing the same bytes, **validating** every
    /// existing element against `maxWidth` (an element already wider throws the guided `Error`
    /// naming its index, width, the max, and the fix). The max is recorded as the field's
    /// [`byteWidth`](Field::byte_width) and enforced on the checked appends. Throws the guided
    /// `Error` on a **fixed-size** column (`FixedBinary` / `FixedUtf8`), whose width is already
    /// fixed and exact — read its stride with [`width`](ByteSerie::width).
    #[napi]
    pub fn with_max_width(&self, max_width: u32) -> napi::Result<ByteSerie> {
        let max_width = max_width as usize;
        let inner =
            match &self.inner {
                ByteInner::Binary(serie) => ByteInner::Binary(
                    VarSerie::<Binary>::from_parts(
                        serie.offsets().clone(),
                        serie.data().clone(),
                        serie.validity().cloned(),
                        serie.len(),
                    )
                    .with_max_width(max_width)
                    .map_err(to_error)?,
                ),
                ByteInner::Utf8(serie) => ByteInner::Utf8(
                    VarSerie::<Utf8>::from_parts(
                        serie.offsets().clone(),
                        serie.data().clone(),
                        serie.validity().cloned(),
                        serie.len(),
                    )
                    .with_max_width(max_width)
                    .map_err(to_error)?,
                ),
                ByteInner::LargeBinary(serie) => ByteInner::LargeBinary(
                    VarSerie::<LargeBinary>::from_parts(
                        serie.offsets().clone(),
                        serie.data().clone(),
                        serie.validity().cloned(),
                        serie.len(),
                    )
                    .with_max_width(max_width)
                    .map_err(to_error)?,
                ),
                ByteInner::LargeUtf8(serie) => ByteInner::LargeUtf8(
                    VarSerie::<LargeUtf8>::from_parts(
                        serie.offsets().clone(),
                        serie.data().clone(),
                        serie.validity().cloned(),
                        serie.len(),
                    )
                    .with_max_width(max_width)
                    .map_err(to_error)?,
                ),
                ByteInner::FixedBinary(_) | ByteInner::FixedUtf8(_) => return Err(to_error(
                    "a fixed-size column already has a fixed width: max_width applies only to a \
                     variable binary / utf8 column (its width() is the fixed stride)",
                )),
            };
        Ok(ByteSerie { inner })
    }

    /// The optional **max element byte width** for a **variable-length** column (`Binary` / `Utf8` /
    /// `LargeBinary` / `LargeUtf8`) — the value set by [`withMaxWidth`](ByteSerie::with_max_width), or
    /// `null` when unbounded. Always `null` for a **fixed-size** column (`FixedBinary` / `FixedUtf8`),
    /// whose width is exact — read its stride with [`width`](ByteSerie::width).
    #[napi]
    pub fn max_width(&self) -> Option<u32> {
        match &self.inner {
            ByteInner::Binary(serie) => serie.max_width().map(|max| max as u32),
            ByteInner::Utf8(serie) => serie.max_width().map(|max| max as u32),
            ByteInner::LargeBinary(serie) => serie.max_width().map(|max| max as u32),
            ByteInner::LargeUtf8(serie) => serie.max_width().map(|max| max as u32),
            ByteInner::FixedBinary(_) | ByteInner::FixedUtf8(_) => None,
        }
    }

    /// This column's [`Field`] metadata — its `name`, element type, `nullable` flag, and (for a
    /// fixed-size column) its fixed byte `width`.
    #[napi]
    pub fn field(&self) -> Field {
        byte_dispatch!(self, serie => Field { inner: serie.field() })
    }

    /// A fresh sub-column copying elements `[start, start + len)` — the window is **clamped** to the
    /// column's length (an out-of-range window yields a shorter or empty column, never an error).
    #[napi]
    pub fn slice(&self, start: u32, len: u32) -> ByteSerie {
        let (start, len) = (start as usize, len as usize);
        ByteSerie {
            inner: byte_rebuild!(self, serie => serie.slice(start, len)),
        }
    }

    /// Replaces the element at `index` **in place** on a **fixed-size** column (`FixedBinary` /
    /// `FixedUtf8`) — a binary element is a `Buffer`, a utf8 element a `string`, zero-padded or
    /// truncated to the fixed width (marking the slot valid on a nullable column). Throws the guided
    /// `Error` on the wrong JS shape, on an `index` past the end, or on a **variable-length** column
    /// (`Binary` / `Utf8`), which is append-only.
    #[napi]
    pub fn set(&mut self, index: u32, value: Either<Buffer, String>) -> napi::Result<()> {
        let index = index as usize;
        match &mut self.inner {
            ByteInner::FixedBinary(serie) => serie.set(index, &as_binary_element(value)?),
            ByteInner::FixedUtf8(serie) => serie.set(index, as_utf8_element(value)?.as_bytes()),
            ByteInner::Binary(_)
            | ByteInner::Utf8(_)
            | ByteInner::LargeBinary(_)
            | ByteInner::LargeUtf8(_) => return Err(var_set_error()),
        }
        .map_err(to_error)
    }

    /// The **unchecked fast path** of [`set`](ByteSerie::set): the same fixed-size slot write with
    /// **no bounds check** (the caller guarantees `index < len`). An out-of-range `index` is a silent
    /// logic error that writes past the column; a variable-length column still throws the append-only
    /// `Error`.
    #[napi]
    pub fn set_checked(&mut self, index: u32, value: Either<Buffer, String>) -> napi::Result<()> {
        let index = index as usize;
        match &mut self.inner {
            ByteInner::FixedBinary(serie) => serie.set_checked(index, &as_binary_element(value)?),
            ByteInner::FixedUtf8(serie) => {
                serie.set_checked(index, as_utf8_element(value)?.as_bytes())
            }
            ByteInner::Binary(_)
            | ByteInner::Utf8(_)
            | ByteInner::LargeBinary(_)
            | ByteInner::LargeUtf8(_) => return Err(var_set_error()),
        };
        Ok(())
    }

    /// A short debug string — the column's name, element type, length, null count, and (for a
    /// fixed-size column) its fixed byte width.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        let field = byte_dispatch!(self, serie => serie.field());
        match field.byte_width() {
            Some(width) => format!(
                "ByteSerie(name={:?}, dtype={}, len={}, nullCount={}, width={})",
                field.name(),
                field.data_type_id(),
                self.len(),
                self.null_count(),
                width
            ),
            None => format!(
                "ByteSerie(name={:?}, dtype={}, len={}, nullCount={})",
                field.name(),
                field.data_type_id(),
                self.len(),
                self.null_count()
            ),
        }
    }
}

// =======================================================================================
// The nested typed layer — struct / list / map columns grown on the erased core `Column`.
// =======================================================================================
//
// The core `yggdryl_core::typed::nested` layer composes columns of *different* types: the erased
// [`CoreColumn`] tags every concrete data column, and the [`CoreStructSerie`] / [`CoreListSerie`] /
// [`CoreMapSerie`] carriers nest inside each other. These four napi classes ([`StructSerie`],
// [`StructField`], [`ListSerie`], [`MapSerie`]) mirror them, each a thin owner of its core value.
//
// The crux is the boundary conversion in **both** directions, because napi hands classes across the
// FFI **by copy** (never a live `&mut`):
//
//   - reading a child — the core hands back a borrowed `&CoreColumn`, so it is **cloned** and wrapped
//     into whichever binding class the variant selects ([`column_to_union`]);
//   - writing a child — the binding gets a borrowed wrapper (`&Serie` / `&ByteSerie` / a nested
//     class), so its `inner` is **cloned** into a fresh owned `CoreColumn` ([`column_from_binding`]).
//
// Because there is no live `&mut` across the boundary, the core's deep-mut accessors
// (`column_by_name_mut`, `values_mut`, …) surface as a **copy-out + replace** pair here:
// `column(name)` returns a copy and `setColumn(name, serie)` replaces the whole column.

// Private in-module constructors wrapping an erased backing — used by the column conversions to
// build the leaf wrappers / a flat field back out of a core `Column` / `ColumnField`.
impl Serie {
    fn from_inner(inner: SerieInner) -> Serie {
        Serie { inner }
    }
}

impl ByteSerie {
    fn from_inner(inner: ByteInner) -> ByteSerie {
        ByteSerie { inner }
    }
}

impl Field {
    fn from_inner(inner: HeaderField) -> Field {
        Field { inner }
    }
}

/// The heterogeneous column union crossing to JS — a [`Serie`] (numeric / bool / decimal leaf), a
/// [`ByteSerie`] (byte leaf), or a nested [`StructSerie`] / [`ListSerie`] / [`MapSerie`]. napi returns
/// the concrete instance directly (no discriminant wrapper), so JS calls its methods as usual.
type ColumnUnion = Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie>;

/// The heterogeneous column union crossing **from** JS — a borrowed binding wrapper (`&Serie` /
/// `&ByteSerie` / a nested class), the input of every column-taking factory / setter. napi
/// discriminates it by class (an `instanceof`-style check per arm).
type ColumnRef<'a> =
    Either5<&'a Serie, &'a ByteSerie, &'a StructSerie, &'a ListSerie, &'a MapSerie>;

// ---- cloning a borrowed core column (napi returns copies) ------------------------------

/// Clones a borrowed [`VarSerie`] (variable-length byte carrier), preserving its `name` and optional
/// `max_width` — the byte-carrier twin of [`clone_serie`]. `VarSerie` is not `Clone`, so it is
/// re-wrapped from clones of its offsets / data / validity buffers.
fn clone_varserie<T: VarLenType>(serie: &VarSerie<T>) -> VarSerie<T> {
    let out = VarSerie::<T>::from_parts_bounded(
        serie.offsets().clone(),
        serie.data().clone(),
        serie.validity().cloned(),
        serie.len(),
        serie.max_width(),
    )
    .expect("re-wrapping already-valid buffers with their own max_width never fails");
    match serie.name() {
        Some(name) => out.with_name(name),
        None => out,
    }
}

/// Clones a borrowed [`FixedSizeSerie`] (fixed-stride byte carrier), preserving its `name` and fixed
/// `width` — the fixed-size twin of [`clone_serie`].
fn clone_fixedsize<T: VarType>(serie: &FixedSizeSerie<T>) -> FixedSizeSerie<T> {
    let out = FixedSizeSerie::<T>::from_parts(
        serie.data().clone(),
        serie.validity().cloned(),
        serie.len(),
        serie.width(),
    );
    match serie.name() {
        Some(name) => out.with_name(name),
        None => out,
    }
}

/// Rebuilds a fresh row-level / element-level validity [`CoreHeap`] bitmap (LSB-first, `1` = valid)
/// from a `nullable` flag, a `len`, and a per-index validity probe — the shared clone step for a
/// nested carrier's `validity` buffer (which the core does not expose directly).
fn rebuild_validity(
    nullable: bool,
    len: usize,
    is_valid: impl Fn(usize) -> bool,
) -> Option<CoreHeap> {
    if !nullable {
        return None;
    }
    let mut bits = CoreHeap::new();
    for index in 0..len {
        bits.pwrite_bit(index as u64, is_valid(index))
            .expect("bit write into a fresh heap never fails");
    }
    Some(bits)
}

/// Rebuilds a fresh `len + 1` little-endian `i32` offsets [`CoreHeap`] from a per-list span probe —
/// the shared clone step for a [`CoreListSerie`] / [`CoreMapSerie`] offsets buffer. `offsets[0]` is
/// the first span's start (`0` for an empty column) and `offsets[i + 1]` its end.
fn rebuild_offsets(len: usize, span: impl Fn(usize) -> (usize, usize)) -> CoreHeap {
    let mut offsets = CoreHeap::new();
    let first = if len == 0 { 0 } else { span(0).0 as i32 };
    offsets
        .pwrite_i32(0, first)
        .expect("offset write into a fresh heap never fails");
    for index in 0..len {
        offsets
            .pwrite_i32((index as u64 + 1) * 4, span(index).1 as i32)
            .expect("offset write into a heap never fails");
    }
    offsets
}

/// Deep-clones a borrowed [`CoreStructSerie`] — rebuilds it from clones of its children, its `name`,
/// its row-level validity (reconstructed from the per-row probe), and its metadata.
fn clone_struct(serie: &CoreStructSerie) -> CoreStructSerie {
    let children: Vec<CoreColumn> = serie.columns().iter().map(clone_column).collect();
    let mut out = CoreStructSerie::from_columns(children)
        .expect("cloned children keep the source's shared length");
    if let Some(name) = serie.name() {
        out = out.with_name(name);
    }
    out = out.with_validity(rebuild_validity(
        serie.field().nullable(),
        serie.len(),
        |index| serie.is_valid(index),
    ));
    *out.metadata_mut() = serie.metadata().clone();
    out
}

/// Deep-clones a borrowed [`CoreListSerie`] — rebuilds it from a clone of the flattened child column,
/// its reconstructed offsets, its `name`, and its element-level validity.
fn clone_list(serie: &CoreListSerie) -> CoreListSerie {
    let values = clone_column(serie.values());
    let offsets = rebuild_offsets(serie.len(), |index| {
        serie.list_at(index).expect("index within the list count")
    });
    let validity = rebuild_validity(serie.field().nullable(), serie.len(), |index| {
        serie.is_valid(index)
    });
    CoreListSerie::from_offsets(
        serie.name().unwrap_or(""),
        offsets,
        values,
        validity,
        serie.len(),
    )
}

/// Deep-clones a borrowed [`CoreMapSerie`] — rebuilds it from clones of its flattened key / value
/// columns, its reconstructed offsets, its `name`, its `keys_sorted` flag, and its element-level
/// validity.
fn clone_map(serie: &CoreMapSerie) -> CoreMapSerie {
    let keys = clone_column(serie.keys());
    let values = clone_column(serie.values());
    let offsets = rebuild_offsets(serie.len(), |index| {
        serie.map_at(index).expect("index within the map count")
    });
    let validity = rebuild_validity(serie.field().nullable(), serie.len(), |index| {
        serie.is_valid(index)
    });
    CoreMapSerie::from_offsets(
        serie.name().unwrap_or(""),
        offsets,
        keys,
        values,
        validity,
        serie.len(),
    )
    .expect("cloned key column keeps its non-nullability and shared length")
    .with_keys_sorted(serie.keys_sorted())
}

/// Deep-clones any borrowed [`CoreColumn`] — the recursive dispatch every read / nested-write flows
/// through. A future (`#[non_exhaustive]`) variant falls back to an all-null run of the same length.
fn clone_column(column: &CoreColumn) -> CoreColumn {
    match column {
        CoreColumn::Null(count) => CoreColumn::Null(*count),
        CoreColumn::Int8(serie) => CoreColumn::Int8(clone_serie(serie)),
        CoreColumn::UInt8(serie) => CoreColumn::UInt8(clone_serie(serie)),
        CoreColumn::Int16(serie) => CoreColumn::Int16(clone_serie(serie)),
        CoreColumn::UInt16(serie) => CoreColumn::UInt16(clone_serie(serie)),
        CoreColumn::Int32(serie) => CoreColumn::Int32(clone_serie(serie)),
        CoreColumn::UInt32(serie) => CoreColumn::UInt32(clone_serie(serie)),
        CoreColumn::Int64(serie) => CoreColumn::Int64(clone_serie(serie)),
        CoreColumn::UInt64(serie) => CoreColumn::UInt64(clone_serie(serie)),
        CoreColumn::Int128(serie) => CoreColumn::Int128(clone_serie(serie)),
        CoreColumn::UInt128(serie) => CoreColumn::UInt128(clone_serie(serie)),
        CoreColumn::Float32(serie) => CoreColumn::Float32(clone_serie(serie)),
        CoreColumn::Float64(serie) => CoreColumn::Float64(clone_serie(serie)),
        CoreColumn::Bool(serie) => CoreColumn::Bool(clone_serie(serie)),
        CoreColumn::Decimal32(serie) => CoreColumn::Decimal32(clone_serie(serie)),
        CoreColumn::Decimal64(serie) => CoreColumn::Decimal64(clone_serie(serie)),
        CoreColumn::Decimal128(serie) => CoreColumn::Decimal128(clone_serie(serie)),
        CoreColumn::Decimal256(serie) => CoreColumn::Decimal256(clone_serie(serie)),
        CoreColumn::Binary(serie) => CoreColumn::Binary(clone_varserie(serie)),
        CoreColumn::Utf8(serie) => CoreColumn::Utf8(clone_varserie(serie)),
        CoreColumn::LargeBinary(serie) => CoreColumn::LargeBinary(clone_varserie(serie)),
        CoreColumn::LargeUtf8(serie) => CoreColumn::LargeUtf8(clone_varserie(serie)),
        CoreColumn::FixedBinary(serie) => CoreColumn::FixedBinary(clone_fixedsize(serie)),
        CoreColumn::FixedUtf8(serie) => CoreColumn::FixedUtf8(clone_fixedsize(serie)),
        CoreColumn::Struct(serie) => CoreColumn::Struct(clone_struct(serie)),
        CoreColumn::List(serie) => CoreColumn::List(clone_list(serie)),
        CoreColumn::Map(serie) => CoreColumn::Map(clone_map(serie)),
        _ => CoreColumn::Null(column.len()),
    }
}

// ---- core Column <-> binding wrapper ---------------------------------------------------

/// Wraps an **owned** core [`CoreColumn`] into the concrete binding class its variant selects — the
/// read boundary. A bufferless [`CoreColumn::Null`] (never produced by these builders) surfaces as a
/// nullable `I8` [`Serie`] of `n` nulls; a future variant collapses the same way.
fn column_to_union(column: CoreColumn) -> ColumnUnion {
    match column {
        CoreColumn::Int8(serie) => Either5::A(Serie::from_inner(SerieInner::I8(serie))),
        CoreColumn::UInt8(serie) => Either5::A(Serie::from_inner(SerieInner::U8(serie))),
        CoreColumn::Int16(serie) => Either5::A(Serie::from_inner(SerieInner::I16(serie))),
        CoreColumn::UInt16(serie) => Either5::A(Serie::from_inner(SerieInner::U16(serie))),
        CoreColumn::Int32(serie) => Either5::A(Serie::from_inner(SerieInner::I32(serie))),
        CoreColumn::UInt32(serie) => Either5::A(Serie::from_inner(SerieInner::U32(serie))),
        CoreColumn::Int64(serie) => Either5::A(Serie::from_inner(SerieInner::I64(serie))),
        CoreColumn::UInt64(serie) => Either5::A(Serie::from_inner(SerieInner::U64(serie))),
        CoreColumn::Int128(serie) => Either5::A(Serie::from_inner(SerieInner::I128(serie))),
        CoreColumn::UInt128(serie) => Either5::A(Serie::from_inner(SerieInner::U128(serie))),
        CoreColumn::Float32(serie) => Either5::A(Serie::from_inner(SerieInner::F32(serie))),
        CoreColumn::Float64(serie) => Either5::A(Serie::from_inner(SerieInner::F64(serie))),
        CoreColumn::Bool(serie) => Either5::A(Serie::from_inner(SerieInner::Bool(serie))),
        CoreColumn::Decimal32(serie) => Either5::A(Serie::from_inner(SerieInner::Decimal32(serie))),
        CoreColumn::Decimal64(serie) => Either5::A(Serie::from_inner(SerieInner::Decimal64(serie))),
        CoreColumn::Decimal128(serie) => {
            Either5::A(Serie::from_inner(SerieInner::Decimal128(serie)))
        }
        CoreColumn::Decimal256(serie) => {
            Either5::A(Serie::from_inner(SerieInner::Decimal256(serie)))
        }
        CoreColumn::Binary(serie) => Either5::B(ByteSerie::from_inner(ByteInner::Binary(serie))),
        CoreColumn::Utf8(serie) => Either5::B(ByteSerie::from_inner(ByteInner::Utf8(serie))),
        CoreColumn::LargeBinary(serie) => {
            Either5::B(ByteSerie::from_inner(ByteInner::LargeBinary(serie)))
        }
        CoreColumn::LargeUtf8(serie) => {
            Either5::B(ByteSerie::from_inner(ByteInner::LargeUtf8(serie)))
        }
        CoreColumn::FixedBinary(serie) => {
            Either5::B(ByteSerie::from_inner(ByteInner::FixedBinary(serie)))
        }
        CoreColumn::FixedUtf8(serie) => {
            Either5::B(ByteSerie::from_inner(ByteInner::FixedUtf8(serie)))
        }
        CoreColumn::Struct(serie) => Either5::C(StructSerie { inner: serie }),
        CoreColumn::List(serie) => Either5::D(ListSerie { inner: serie }),
        CoreColumn::Map(serie) => Either5::E(MapSerie { inner: serie }),
        CoreColumn::Null(count) => Either5::A(Serie::from_inner(SerieInner::I8(
            FixedSerie::<Int8>::from_options(&vec![None; count]),
        ))),
        _ => Either5::A(Serie::from_inner(SerieInner::I8(
            FixedSerie::<Int8>::from_options(&[]),
        ))),
    }
}

/// Clones a borrowed numeric [`Serie`]'s `inner` into an owned [`CoreColumn`], optionally overriding
/// the child `name` (given by `StructSerie.fromColumns` / `setColumn`).
fn serie_to_column(serie: &Serie, name: Option<&str>) -> CoreColumn {
    fn named<T: Encoder + Decoder>(serie: &FixedSerie<T>, name: Option<&str>) -> FixedSerie<T> {
        let cloned = clone_serie(serie);
        match name {
            Some(name) => cloned.with_name(name),
            None => cloned,
        }
    }
    match &serie.inner {
        SerieInner::I8(serie) => CoreColumn::Int8(named(serie, name)),
        SerieInner::U8(serie) => CoreColumn::UInt8(named(serie, name)),
        SerieInner::I16(serie) => CoreColumn::Int16(named(serie, name)),
        SerieInner::U16(serie) => CoreColumn::UInt16(named(serie, name)),
        SerieInner::I32(serie) => CoreColumn::Int32(named(serie, name)),
        SerieInner::U32(serie) => CoreColumn::UInt32(named(serie, name)),
        SerieInner::I64(serie) => CoreColumn::Int64(named(serie, name)),
        SerieInner::U64(serie) => CoreColumn::UInt64(named(serie, name)),
        SerieInner::I128(serie) => CoreColumn::Int128(named(serie, name)),
        SerieInner::U128(serie) => CoreColumn::UInt128(named(serie, name)),
        SerieInner::F32(serie) => CoreColumn::Float32(named(serie, name)),
        SerieInner::F64(serie) => CoreColumn::Float64(named(serie, name)),
        SerieInner::Bool(serie) => CoreColumn::Bool(named(serie, name)),
        SerieInner::Decimal32(serie) => CoreColumn::Decimal32(named(serie, name)),
        SerieInner::Decimal64(serie) => CoreColumn::Decimal64(named(serie, name)),
        SerieInner::Decimal128(serie) => CoreColumn::Decimal128(named(serie, name)),
        SerieInner::Decimal256(serie) => CoreColumn::Decimal256(named(serie, name)),
    }
}

/// Clones a borrowed byte [`ByteSerie`]'s `inner` into an owned [`CoreColumn`], optionally overriding
/// the child `name`.
fn byteserie_to_column(byte: &ByteSerie, name: Option<&str>) -> CoreColumn {
    fn named_var<T: VarLenType>(serie: &VarSerie<T>, name: Option<&str>) -> VarSerie<T> {
        let cloned = clone_varserie(serie);
        match name {
            Some(name) => cloned.with_name(name),
            None => cloned,
        }
    }
    fn named_fixed<T: VarType>(serie: &FixedSizeSerie<T>, name: Option<&str>) -> FixedSizeSerie<T> {
        let cloned = clone_fixedsize(serie);
        match name {
            Some(name) => cloned.with_name(name),
            None => cloned,
        }
    }
    match &byte.inner {
        ByteInner::Binary(serie) => CoreColumn::Binary(named_var(serie, name)),
        ByteInner::Utf8(serie) => CoreColumn::Utf8(named_var(serie, name)),
        ByteInner::LargeBinary(serie) => CoreColumn::LargeBinary(named_var(serie, name)),
        ByteInner::LargeUtf8(serie) => CoreColumn::LargeUtf8(named_var(serie, name)),
        ByteInner::FixedBinary(serie) => CoreColumn::FixedBinary(named_fixed(serie, name)),
        ByteInner::FixedUtf8(serie) => CoreColumn::FixedUtf8(named_fixed(serie, name)),
    }
}

/// Clones a borrowed binding column wrapper into an owned core [`CoreColumn`] — the write boundary,
/// optionally overriding the child `name`. A nested wrapper is deep-cloned then renamed.
fn column_from_binding(wrapper: ColumnRef, name: Option<&str>) -> CoreColumn {
    match wrapper {
        Either5::A(serie) => serie_to_column(serie, name),
        Either5::B(byte) => byteserie_to_column(byte, name),
        Either5::C(nested) => {
            let mut cloned = clone_struct(&nested.inner);
            if let Some(name) = name {
                cloned = cloned.with_name(name);
            }
            CoreColumn::Struct(cloned)
        }
        Either5::D(nested) => {
            let mut cloned = clone_list(&nested.inner);
            if let Some(name) = name {
                cloned = cloned.with_name(name);
            }
            CoreColumn::List(cloned)
        }
        Either5::E(nested) => {
            let mut cloned = clone_map(&nested.inner);
            if let Some(name) = name {
                cloned = cloned.with_name(name);
            }
            CoreColumn::Map(cloned)
        }
    }
}

// ---- nested value marshalling: a core `Value` -> its JS shape --------------------------

/// Bridges any owned `ToNapiValue` into a `JsUnknown` through the raw napi boundary — the general
/// converter [`value_to_unknown`] leans on so a heterogeneous, recursive [`CoreValue`] tree renders
/// with one code path (a leaf as a `number` / `BigInt` / `boolean` / `Buffer` / `string`, a nested
/// row / list / map entries as a JS array).
fn to_unknown<T: ToNapiValue>(env: &Env, value: T) -> napi::Result<JsUnknown> {
    let raw = unsafe { T::to_napi_value(env.raw(), value)? };
    Ok(unsafe { JsUnknown::from_napi_value(env.raw(), raw)? })
}

/// Marshals one erased [`CoreValue`] element to its JS shape: a narrow integer / float / `Decimal32`
/// as a `number`, a wide integer / `Decimal64` / `Decimal128` / `Decimal256` as a `BigInt`, a `Bool`
/// as a `boolean`, `Binary` as a `Buffer`, `Utf8` as a `string`, `Null` as JS `null`; a nested `Row`
/// / `List` as a JS array of its child values, and a `Map` as an **entries array**
/// (`[[key, value], …]`, so any key type is representable).
fn value_to_unknown(env: &Env, value: &CoreValue) -> napi::Result<JsUnknown> {
    match value {
        CoreValue::Null => to_unknown(env, Null),
        CoreValue::Int8(v) => to_unknown(env, *v as f64),
        CoreValue::UInt8(v) => to_unknown(env, *v as f64),
        CoreValue::Int16(v) => to_unknown(env, *v as f64),
        CoreValue::UInt16(v) => to_unknown(env, *v as f64),
        CoreValue::Int32(v) => to_unknown(env, *v as f64),
        CoreValue::UInt32(v) => to_unknown(env, *v as f64),
        CoreValue::Float32(v) => to_unknown(env, *v as f64),
        CoreValue::Float64(v) => to_unknown(env, *v),
        CoreValue::Decimal32(v) => to_unknown(env, *v as f64),
        CoreValue::Int64(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::UInt64(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::Int128(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::UInt128(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::Decimal64(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::Decimal128(v) => to_unknown(env, BigInt::from(*v)),
        CoreValue::Decimal256(v) => to_unknown(env, i256_to_bigint(*v)),
        CoreValue::Bool(v) => to_unknown(env, *v),
        CoreValue::Binary(bytes) => to_unknown(env, Buffer::from(bytes.clone())),
        CoreValue::Utf8(text) => to_unknown(env, text.clone()),
        CoreValue::Row(row) => {
            let items = row
                .values()
                .iter()
                .map(|value| value_to_unknown(env, value))
                .collect::<napi::Result<Vec<_>>>()?;
            to_unknown(env, items)
        }
        CoreValue::List(list) => {
            let items = list
                .values()
                .iter()
                .map(|value| value_to_unknown(env, value))
                .collect::<napi::Result<Vec<_>>>()?;
            to_unknown(env, items)
        }
        CoreValue::Map(map) => {
            let entries = map
                .keys()
                .iter()
                .zip(map.values().iter())
                .map(|(key, value)| {
                    let pair = vec![value_to_unknown(env, key)?, value_to_unknown(env, value)?];
                    to_unknown(env, pair)
                })
                .collect::<napi::Result<Vec<_>>>()?;
            to_unknown(env, entries)
        }
    }
}

/// Renders a core [`CoreColumnField`] as the flat binding [`Field`] — a leaf keeps its
/// [`HeaderField`], a nested field collapses to its `name` / nested [`DataTypeId`] (`Struct` / `List`
/// / `Map`) / `nullable` (its child schema is reachable on the child columns' own `field()`).
fn columnfield_to_field(field: &CoreColumnField) -> Field {
    match field {
        CoreColumnField::Leaf(header) => Field::from_inner(header.clone()),
        other => Field::from_inner(HeaderField::new(
            other.name(),
            other.data_type_id(),
            other.nullable(),
        )),
    }
}

// ---- the struct "table" ----------------------------------------------------------------

/// A **struct column** — the table: an ordered set of equal-length, heterogeneous child columns
/// ([`Serie`] / [`ByteSerie`] / nested), with an optional row-level validity buffer. Mirrors
/// `yggdryl_core::typed::StructSerie`. Because napi crosses classes by copy, [`column`](StructSerie::column)
/// hands back a **copy** of a child and [`setColumn`](StructSerie::set_column) replaces a whole column
/// — the binding form of the core's deep-mut `column_by_name_mut`.
#[napi(namespace = "typed")]
pub struct StructSerie {
    inner: CoreStructSerie,
}

#[napi(namespace = "typed")]
impl StructSerie {
    /// An **empty**, non-nullable struct named `name` (add columns with
    /// [`setColumn`](StructSerie::set_column)).
    #[napi(constructor)]
    pub fn new(name: Option<String>) -> StructSerie {
        StructSerie {
            inner: CoreStructSerie::new(name.as_deref().unwrap_or("")),
        }
    }

    /// A struct from `columns` (each a [`Serie`] / [`ByteSerie`] / nested column), optionally renamed
    /// by the parallel `names`. Throws the guided `Error` when the columns are not all the same length,
    /// or when `names` is given with a different length than `columns`.
    #[napi(factory)]
    pub fn from_columns(
        columns: Vec<Either5<&Serie, &ByteSerie, &StructSerie, &ListSerie, &MapSerie>>,
        names: Option<Vec<String>>,
    ) -> napi::Result<StructSerie> {
        let children: Vec<CoreColumn> = match &names {
            Some(names) => {
                if names.len() != columns.len() {
                    return Err(to_error(format!(
                        "names length {} does not match columns length {}: pass exactly one name per \
                         column, or omit names to keep each column's own name",
                        names.len(),
                        columns.len()
                    )));
                }
                columns
                    .into_iter()
                    .zip(names.iter())
                    .map(|(column, name)| column_from_binding(column, Some(name.as_str())))
                    .collect()
            }
            None => columns
                .into_iter()
                .map(|column| column_from_binding(column, None))
                .collect(),
        };
        Ok(StructSerie {
            inner: CoreStructSerie::from_columns(children).map_err(to_error)?,
        })
    }

    /// The number of child columns.
    #[napi]
    pub fn num_columns(&self) -> u32 {
        self.inner.num_columns() as u32
    }

    /// The number of rows.
    #[napi]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the struct has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// A **copy** of the child column at `index` — a [`Serie`] / [`ByteSerie`] / nested class — or
    /// `null` when out of range. (A copy, not a live handle: mutate a column and write it back with
    /// [`setColumn`](StructSerie::set_column).)
    #[napi]
    pub fn column(
        &self,
        index: u32,
    ) -> Option<Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie>> {
        self.inner
            .column(index as usize)
            .map(|column| column_to_union(clone_column(column)))
    }

    /// A **copy** of the first child column named `name`, or `null` when absent.
    #[napi]
    pub fn column_by_name(
        &self,
        name: String,
    ) -> Option<Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie>> {
        self.inner
            .column_by_name(&name)
            .map(|column| column_to_union(clone_column(column)))
    }

    /// The child column names in order (an unnamed column reports `""`).
    #[napi]
    pub fn column_names(&self) -> Vec<String> {
        self.inner
            .columns()
            .iter()
            .map(|column| column.name().unwrap_or("").to_string())
            .collect()
    }

    /// **Replaces** the child column named `name` with `column` (renamed to `name`), or **appends**
    /// it when `name` is absent — the binding form of the core's deep-mut `column_by_name_mut`.
    /// Throws the guided `Error` when the replacement's length differs from the struct's.
    #[napi]
    pub fn set_column(
        &mut self,
        name: String,
        column: Either5<&Serie, &ByteSerie, &StructSerie, &ListSerie, &MapSerie>,
    ) -> napi::Result<()> {
        let new_column = column_from_binding(column, Some(&name));
        match self.inner.column_by_name_mut(&name) {
            Some(slot) => {
                if new_column.len() != slot.len() {
                    return Err(to_error(format!(
                        "setColumn length mismatch: column {name:?} has {} rows but the struct has \
                         {} — build the column to {} rows (pad or truncate) before setting it",
                        new_column.len(),
                        slot.len(),
                        slot.len()
                    )));
                }
                *slot = new_column;
                Ok(())
            }
            None => {
                let old = std::mem::replace(&mut self.inner, CoreStructSerie::new(""));
                self.inner = old.with_column(new_column).map_err(to_error)?;
                Ok(())
            }
        }
    }

    /// The **row** at `index` — an array of its child values (each a `number` / `BigInt` / `boolean`
    /// / `Buffer` / `string` / `null`, or a nested array / entries array) — or `null` when out of
    /// range.
    #[napi]
    pub fn row(&self, env: Env, index: u32) -> napi::Result<Option<Vec<JsUnknown>>> {
        match self.inner.row(index as usize) {
            None => Ok(None),
            Some(scalar) => {
                let values = scalar
                    .values()
                    .iter()
                    .map(|value| value_to_unknown(&env, value))
                    .collect::<napi::Result<Vec<_>>>()?;
                Ok(Some(values))
            }
        }
    }

    /// Appends a **null row** — grows every child by one null slot and clears the new row's validity
    /// bit.
    #[napi]
    pub fn push_null(&mut self) {
        self.inner.push_null();
    }

    /// Whether the **row** at `index` is valid (non-null). Out of range is `false`.
    #[napi]
    pub fn is_valid(&self, index: u32) -> bool {
        self.inner.is_valid(index as usize)
    }

    /// How many rows are null.
    #[napi]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// This struct's [`StructField`] schema — its name, nullability, and ordered child fields.
    #[napi]
    pub fn field(&self) -> StructField {
        StructField {
            inner: self.inner.field(),
        }
    }

    /// A short debug string — the struct's name, column count, row count, and null count.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructSerie(name={:?}, numColumns={}, len={}, nullCount={})",
            self.inner.name(),
            self.inner.num_columns(),
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

/// A **struct column's schema** — its `name`, `nullable` flag, and ordered child [`Field`]s. Mirrors
/// `yggdryl_core::typed::StructField`.
#[napi(namespace = "typed")]
pub struct StructField {
    inner: CoreStructField,
}

#[napi(namespace = "typed")]
impl StructField {
    /// A struct schema from its optional `name` and ordered child leaf `fields`. (A manually-built
    /// schema carries leaf child fields; a nested child schema is reported by a nested
    /// [`StructSerie::field`] built from real nested columns.)
    #[napi(constructor)]
    pub fn new(name: Option<String>, fields: Vec<&Field>) -> StructField {
        let children: Vec<CoreColumnField> = fields
            .iter()
            .map(|field| CoreColumnField::Leaf(field.inner.clone()))
            .collect();
        StructField {
            inner: CoreStructField::new(name.as_deref(), children),
        }
    }

    /// The child field names in order (an unnamed child reports `""`).
    #[napi]
    pub fn names(&self) -> Vec<String> {
        self.inner.names().iter().map(|s| s.to_string()).collect()
    }

    /// The child [`Field`] at `index`, or `null` when out of range.
    #[napi]
    pub fn field(&self, index: u32) -> Option<Field> {
        self.inner.field(index as usize).map(columnfield_to_field)
    }

    /// The first child [`Field`] named `name`, or `null` when absent.
    #[napi]
    pub fn field_by_name(&self, name: String) -> Option<Field> {
        self.inner.field_by_name(&name).map(columnfield_to_field)
    }

    /// The number of child fields.
    #[napi]
    pub fn num_fields(&self) -> u32 {
        self.inner.num_fields() as u32
    }

    /// The struct's name, or `null` when unset.
    #[napi]
    pub fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// Whether the struct admits null rows.
    #[napi]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// Content equality — equal iff every field (name, nullability, metadata, children) matches.
    #[napi]
    pub fn equals(&self, other: &StructField) -> bool {
        self.inner == other.inner
    }

    /// A short debug string — the struct's name, field count, and nullability.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructField(name={:?}, numFields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            self.inner.nullable()
        )
    }
}

// ---- the list column -------------------------------------------------------------------

/// A **list column** — a variable-length list over a flattened child column: [`push`](ListSerie::push)
/// demarcates each list as the next `childLen` rows of the child. Mirrors
/// `yggdryl_core::typed::ListSerie`.
#[napi(namespace = "typed")]
pub struct ListSerie {
    inner: CoreListSerie,
}

#[napi(namespace = "typed")]
impl ListSerie {
    /// An **empty** list column over the flattened child `values` column (a [`Serie`] / [`ByteSerie`]
    /// / nested column), optionally named. Its rows become list elements as
    /// [`push`](ListSerie::push) demarcates them.
    #[napi(constructor)]
    pub fn new(
        values: Either5<&Serie, &ByteSerie, &StructSerie, &ListSerie, &MapSerie>,
        name: Option<String>,
    ) -> ListSerie {
        let child = column_from_binding(values, None);
        ListSerie {
            inner: CoreListSerie::new(name.as_deref().unwrap_or(""), child),
        }
    }

    /// Appends a **non-null** list spanning the next `child_len` rows of the flattened child column.
    #[napi]
    pub fn push(&mut self, child_len: u32) {
        self.inner.push(child_len as usize);
    }

    /// Appends a **null** list (an empty span with its validity bit cleared).
    #[napi]
    pub fn push_null(&mut self) {
        self.inner.push_null();
    }

    /// The number of lists.
    #[napi]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the column has no lists.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// A **copy** of the flattened child column ([`Serie`] / [`ByteSerie`] / nested) holding every
    /// list's elements.
    #[napi]
    pub fn values(&self) -> Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie> {
        column_to_union(clone_column(self.inner.values()))
    }

    /// The **list element** at `index` as an array of its child values (each a `number` / `BigInt` /
    /// `boolean` / `Buffer` / `string` / `null`, or a nested array / entries array), or `null` when
    /// the list is null or out of range. An empty (non-null) list is an empty array.
    #[napi]
    pub fn list(&self, env: Env, index: u32) -> napi::Result<Option<Vec<JsUnknown>>> {
        match self.inner.list(index as usize) {
            Some(scalar) if !scalar.is_null() => {
                let values = scalar
                    .values()
                    .iter()
                    .map(|value| value_to_unknown(&env, value))
                    .collect::<napi::Result<Vec<_>>>()?;
                Ok(Some(values))
            }
            _ => Ok(None),
        }
    }

    /// Whether the list at `index` is valid (non-null). Out of range is `false`.
    #[napi]
    pub fn is_valid(&self, index: u32) -> bool {
        self.inner.is_valid(index as usize)
    }

    /// How many lists are null.
    #[napi]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// This list's [`Field`] — a flat descriptor (`name` / `List` dtype / `nullable`); the item's
    /// element type is on [`values`](ListSerie::values)`.field()`.
    #[napi]
    pub fn field(&self) -> Field {
        let field = self.inner.field();
        Field::from_inner(HeaderField::new(field.name(), DtId::List, field.nullable()))
    }

    /// A short debug string — the list's name, list count, and null count.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "ListSerie(name={:?}, len={}, nullCount={})",
            self.inner.name(),
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

// ---- the map column --------------------------------------------------------------------

/// A **map column** — a key→value map over flattened key / value columns: [`push`](MapSerie::push)
/// demarcates each map as the next `entryCount` entries. Map **keys must be non-nullable** (an Arrow
/// constraint). Mirrors `yggdryl_core::typed::MapSerie`.
#[napi(namespace = "typed")]
pub struct MapSerie {
    inner: CoreMapSerie,
}

#[napi(namespace = "typed")]
impl MapSerie {
    /// An **empty** map column over the flattened `keys` + `values` columns (each a [`Serie`] /
    /// [`ByteSerie`] / nested column), optionally named. Their rows become entries as
    /// [`push`](MapSerie::push) demarcates them. Throws the guided `Error` when the key column is
    /// nullable or the two columns differ in length.
    #[napi(constructor)]
    pub fn new(
        keys: Either5<&Serie, &ByteSerie, &StructSerie, &ListSerie, &MapSerie>,
        values: Either5<&Serie, &ByteSerie, &StructSerie, &ListSerie, &MapSerie>,
        name: Option<String>,
    ) -> napi::Result<MapSerie> {
        let key_col = column_from_binding(keys, None);
        let value_col = column_from_binding(values, None);
        Ok(MapSerie {
            inner: CoreMapSerie::new(name.as_deref().unwrap_or(""), key_col, value_col)
                .map_err(to_error)?,
        })
    }

    /// Appends a **non-null** map spanning the next `entry_count` entries of the flattened columns.
    #[napi]
    pub fn push(&mut self, entry_count: u32) {
        self.inner.push(entry_count as usize);
    }

    /// Appends a **null** map (an empty span with its validity bit cleared).
    #[napi]
    pub fn push_null(&mut self) {
        self.inner.push_null();
    }

    /// The number of maps.
    #[napi]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the column has no maps.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// A **copy** of the flattened **key** column ([`Serie`] / [`ByteSerie`] / nested).
    #[napi]
    pub fn keys(&self) -> Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie> {
        column_to_union(clone_column(self.inner.keys()))
    }

    /// A **copy** of the flattened **value** column ([`Serie`] / [`ByteSerie`] / nested).
    #[napi]
    pub fn values(&self) -> Either5<Serie, ByteSerie, StructSerie, ListSerie, MapSerie> {
        column_to_union(clone_column(self.inner.values()))
    }

    /// The **map element** at `index` as an **entries array** (`[[key, value], …]`, each value a
    /// `number` / `BigInt` / `boolean` / `Buffer` / `string` / `null` or a nested array), or `null`
    /// when the map is null or out of range. An empty (non-null) map is an empty array.
    #[napi]
    pub fn get(&self, env: Env, index: u32) -> napi::Result<Option<Vec<JsUnknown>>> {
        match self.inner.map(index as usize) {
            Some(scalar) if !scalar.is_null() => {
                let entries = scalar
                    .keys()
                    .iter()
                    .zip(scalar.values().iter())
                    .map(|(key, value)| {
                        let pair =
                            vec![value_to_unknown(&env, key)?, value_to_unknown(&env, value)?];
                        to_unknown(&env, pair)
                    })
                    .collect::<napi::Result<Vec<_>>>()?;
                Ok(Some(entries))
            }
            _ => Ok(None),
        }
    }

    /// Whether the keys are **sorted** within each map (an Arrow schema hint).
    #[napi]
    pub fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// Whether the map at `index` is valid (non-null). Out of range is `false`.
    #[napi]
    pub fn is_valid(&self, index: u32) -> bool {
        self.inner.is_valid(index as usize)
    }

    /// How many maps are null.
    #[napi]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// This map's [`Field`] — a flat descriptor (`name` / `Map` dtype / `nullable`); the key / value
    /// element types are on [`keys`](MapSerie::keys)`.field()` / [`values`](MapSerie::values)`.field()`.
    #[napi]
    pub fn field(&self) -> Field {
        let field = self.inner.field();
        Field::from_inner(HeaderField::new(field.name(), DtId::Map, field.nullable()))
    }

    /// A short debug string — the map's name, map count, and null count.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "MapSerie(name={:?}, len={}, nullCount={})",
            self.inner.name(),
            self.inner.len(),
            self.inner.null_count()
        )
    }
}
