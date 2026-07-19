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
            SerieInner::Bool(_) => Err(to_error(
                "a boolean column does not reduce: sum/min/max/mean need a numeric element type — \
                 build a numeric Serie (e.g. DataTypeId.I64())",
            )),
            SerieInner::Decimal32(_)
            | SerieInner::Decimal64(_)
            | SerieInner::Decimal128(_)
            | SerieInner::Decimal256(_) => Err(to_error(
                "a decimal column does not reduce: sum/min/max/mean are not defined for fixed-point \
                 decimals — read the raw unscaled values with get/values or format with \
                 toDecimalString",
            )),
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

    /// This column's [`Field`] metadata — its `name`, element type, `nullable` flag, and (for a
    /// fixed-size column) its fixed byte `width`.
    #[napi]
    pub fn field(&self) -> Field {
        byte_dispatch!(self, serie => Field { inner: serie.field() })
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
