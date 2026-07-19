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
//! type-erased over the four byte carriers: variable-length `Binary` / `Utf8` (an `i32` offsets +
//! data buffer) and fixed-size `FixedBinary` / `FixedUtf8` (a fixed byte stride, short values
//! zero-padded and long ones truncated). A binary element crosses as a JS `Buffer`, a UTF-8 element
//! as a JS `string`; it shares the same null-aware `get` / `toList` / `field` surface.
//!
//! A **decimal** column carries a signed *unscaled integer* whose width matches the four native
//! integers: `Decimal32` crosses as a `number` (`i32`), `Decimal64` / `Decimal128` as a `BigInt`
//! (`i64` / `i128`), and `Decimal256` as an arbitrary-precision `BigInt` (its 256-bit [`I256`]
//! bridges to napi's word-based `BigInt`). Decimals are not `Reduce`, so `sum` / `min` / `max` /
//! `mean` throw the guided `Error`; instead read the raw unscaled value with `get` / `values`, place
//! the point with `toDecimalString`, and read `precision` / `scale` from the [`Field`].

use napi::bindgen_prelude::{BigInt, Buffer, Either, Either3};
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
    Binary, Decoder, Encoder, Field as FieldTrait, FixedBinary, FixedSerie, FixedSizeSerie,
    FixedUtf8, HeaderField, Scalar, Serie as SerieTrait, Utf8, VarSerie, VarType,
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

/// A 256-bit unscaled decimal value as a JS `BigInt` — its 32 little-endian two's-complement bytes
/// re-packed into napi's `(sign_bit, magnitude words)` shape (`words` little-endian `u64`).
impl ToJs for I256 {
    fn to_js(self) -> JsValue {
        let sign_bit = self.is_negative();
        let mut bytes = self.to_le_bytes();
        if sign_bit {
            negate_le_32(&mut bytes); // two's-complement magnitude for the sign-plus-magnitude form
        }
        let words = bytes
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
            .collect();
        Either3::B(BigInt { sign_bit, words })
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

impl<T: VarType> RebuildNamed for VarSerie<T> {
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

/// A byte column for each of the four byte carriers — the type-erased backing of [`ByteSerie`].
enum ByteInner {
    Binary(VarSerie<Binary>),
    Utf8(VarSerie<Utf8>),
    FixedBinary(FixedSizeSerie<FixedBinary>),
    FixedUtf8(FixedSizeSerie<FixedUtf8>),
}

/// Runs `$body` against the inner byte column (`$serie`) of whichever variant is present — the
/// four-way match, written once. Every arm must yield the same type.
macro_rules! byte_dispatch {
    ($self:expr, $serie:ident => $body:expr) => {
        match &$self.inner {
            ByteInner::Binary($serie) => $body,
            ByteInner::Utf8($serie) => $body,
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
            ByteInner::FixedBinary($serie) => ByteInner::FixedBinary($build),
            ByteInner::FixedUtf8($serie) => ByteInner::FixedUtf8($build),
        }
    };
}

/// A **byte-blob typed column** — the variable-length / fixed-size counterpart of [`Serie`], over the
/// four byte carriers: variable-length `Binary` (an `i32` offsets + data buffer) / `Utf8`, and
/// fixed-size `FixedBinary` / `FixedUtf8` (a fixed byte stride). A binary element crosses as a JS
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
    /// `Utf8()`, `FixedBinary()`, `FixedUtf8()`). Binary elements arrive as `Buffer`s, UTF-8 elements
    /// as `string`s. A fixed-size `dtype` requires `width` (the fixed element byte length); a
    /// variable-length one takes no `width`.
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
    /// `null` for a variable-length one (`Binary` / `Utf8`).
    #[napi]
    pub fn width(&self) -> Option<u32> {
        match &self.inner {
            ByteInner::Binary(_) | ByteInner::Utf8(_) => None,
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

    /// A copy of this **variable-length** byte column (`Binary` / `Utf8`) with its optional **max
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
                ByteInner::FixedBinary(_) | ByteInner::FixedUtf8(_) => return Err(to_error(
                    "a fixed-size column already has a fixed width: max_width applies only to a \
                     variable binary / utf8 column (its width() is the fixed stride)",
                )),
            };
        Ok(ByteSerie { inner })
    }

    /// The optional **max element byte width** for a **variable-length** column (`Binary` /
    /// `Utf8`) — the value set by [`withMaxWidth`](ByteSerie::with_max_width), or `null` when
    /// unbounded. Always `null` for a **fixed-size** column (`FixedBinary` / `FixedUtf8`), whose
    /// width is exact — read its stride with [`width`](ByteSerie::width).
    #[napi]
    pub fn max_width(&self) -> Option<u32> {
        match &self.inner {
            ByteInner::Binary(serie) => serie.max_width().map(|max| max as u32),
            ByteInner::Utf8(serie) => serie.max_width().map(|max| max as u32),
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
            ByteInner::Binary(_) | ByteInner::Utf8(_) => return Err(var_set_error()),
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
            ByteInner::Binary(_) | ByteInner::Utf8(_) => return Err(var_set_error()),
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
