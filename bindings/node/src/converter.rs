//! The `yggdryl.converter` namespace — dtype-keyed representation conversion.
//!
//! A thin facade over `yggdryl_core`'s converter family. The core's typed converters
//! fix their element types at compile time, which the FFI cannot hold, so the binding
//! keys them on a dtype **name** at runtime via
//! [`PrimitiveType`](yggdryl_core::PrimitiveType) — one of `i8 … u64`, `f32`, `f64`.
//! Scalars follow the same JS mapping as the rest of the bindings: the small integers
//! and floats marshal as `number`, while `i64` / `u64` marshal as `bigint` (so pass a
//! `bigint` when the dtype is `i64` / `u64`). An unknown name, an out-of-range value,
//! or invalid UTF-8 throws an `Error` naming the accepted dtypes / formats.

use napi::bindgen_prelude::{BigInt, Buffer, Either};
use napi_derive::napi;

use yggdryl_core::{IoPrimitive, PrimitiveType, TypedConverter, Utf8Converter};

/// Maps a core error to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Builds a thrown JS `Error` from a message.
fn error_msg(message: &str) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}

/// Resolves a dtype name, or a guided `Error`.
fn dtype(name: &str) -> napi::Result<PrimitiveType> {
    PrimitiveType::from_name(name).map_err(to_error)
}

/// Extracts the `number` branch, or errors (used for the non-`bigint` dtypes).
fn as_number(value: Either<f64, BigInt>) -> napi::Result<f64> {
    match value {
        Either::A(number) => Ok(number),
        Either::B(_) => Err(error_msg(
            "expected a number for this dtype; pass a bigint only for i64 / u64",
        )),
    }
}

/// Extracts the `bigint` branch, or errors (used for the `i64` / `u64` dtypes).
fn as_bigint(value: Either<f64, BigInt>) -> napi::Result<BigInt> {
    match value {
        Either::B(big) => Ok(big),
        Either::A(_) => Err(error_msg("expected a bigint for the i64 / u64 dtypes")),
    }
}

/// Decodes exactly-`width` little-endian `bytes` into the JS scalar for `pt`.
fn scalar_to_js(pt: PrimitiveType, bytes: &[u8]) -> napi::Result<Either<f64, BigInt>> {
    Ok(match pt {
        PrimitiveType::I8 => Either::A(f64::from(<i8 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::I16 => Either::A(f64::from(<i16 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::I32 => Either::A(f64::from(<i32 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::U8 => Either::A(f64::from(<u8 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::U16 => Either::A(f64::from(<u16 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::U32 => Either::A(f64::from(<u32 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::F32 => Either::A(f64::from(<f32 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::F64 => Either::A(<f64 as IoPrimitive>::from_le_slice(bytes)),
        PrimitiveType::I64 => Either::B(BigInt::from(<i64 as IoPrimitive>::from_le_slice(bytes))),
        PrimitiveType::U64 => Either::B(BigInt::from(<u64 as IoPrimitive>::from_le_slice(bytes))),
        _ => return Err(error_msg("unsupported dtype")),
    })
}

/// Extracts a JS scalar as the `pt` element and returns its little-endian bytes.
fn scalar_from_js(pt: PrimitiveType, value: Either<f64, BigInt>) -> napi::Result<Vec<u8>> {
    Ok(match pt {
        PrimitiveType::I8 => (as_number(value)? as i8).to_le_vec(),
        PrimitiveType::I16 => (as_number(value)? as i16).to_le_vec(),
        PrimitiveType::I32 => (as_number(value)? as i32).to_le_vec(),
        PrimitiveType::U8 => (as_number(value)? as u8).to_le_vec(),
        PrimitiveType::U16 => (as_number(value)? as u16).to_le_vec(),
        PrimitiveType::U32 => (as_number(value)? as u32).to_le_vec(),
        PrimitiveType::F32 => (as_number(value)? as f32).to_le_vec(),
        PrimitiveType::F64 => as_number(value)?.to_le_vec(),
        PrimitiveType::I64 => as_bigint(value)?.get_i64().0.to_le_vec(),
        PrimitiveType::U64 => {
            let (_, unsigned, _) = as_bigint(value)?.get_u64();
            unsigned.to_le_vec()
        }
        _ => return Err(error_msg("unsupported dtype")),
    })
}

/// Casts packed little-endian `data` from `fromDtype` to `toDtype` (C-style `as`),
/// element by element, returning the target's little-endian bytes.
#[napi(namespace = "converter", js_name = "cast")]
pub fn cast(data: Buffer, from_dtype: String, to_dtype: String) -> napi::Result<Buffer> {
    let out = dtype(&from_dtype)?
        .cast_bytes(dtype(&to_dtype)?, data.as_ref())
        .map_err(to_error)?;
    Ok(out.into())
}

/// Flexibly parses `text` into a `dtype` scalar — accepts decimal, `0x`/`0o`/`0b`
/// integers with `_` separators and signs, and decimal/scientific floats.
#[napi(namespace = "converter", js_name = "parse")]
pub fn parse(text: String, dtype_name: String) -> napi::Result<Either<f64, BigInt>> {
    let primitive = dtype(&dtype_name)?;
    let bytes = primitive.parse_bytes(&text).map_err(to_error)?;
    scalar_to_js(primitive, &bytes)
}

/// Renders a `dtype` scalar `value` to its string form.
#[napi(namespace = "converter", js_name = "format")]
pub fn format(value: Either<f64, BigInt>, dtype_name: String) -> napi::Result<String> {
    let primitive = dtype(&dtype_name)?;
    let bytes = scalar_from_js(primitive, value)?;
    primitive.format_bytes(&bytes).map_err(to_error)
}

/// Encodes `text` to its UTF-8 bytes.
#[napi(namespace = "converter", js_name = "utf8Encode")]
pub fn utf8_encode(text: String) -> Buffer {
    text.into_bytes().into()
}

/// Decodes UTF-8 `data` to a string, throwing (naming the failing offset) on invalid
/// UTF-8.
#[napi(namespace = "converter", js_name = "utf8Decode")]
pub fn utf8_decode(data: Buffer) -> napi::Result<String> {
    Utf8Converter::new().decode(data.to_vec()).map_err(to_error)
}
