//! The `yggdryl.converter` namespace — dtype-keyed representation conversion.
//!
//! A thin facade over `yggdryl_core`'s converter family. The core's typed converters
//! fix their element types at compile time, which the FFI cannot hold, so the binding
//! keys them on a dtype **name** at runtime via
//! [`PrimitiveType`](yggdryl_converter::PrimitiveType) — one of `i8 … u64`, `f32`, `f64`.
//! Scalars follow the same JS mapping as the rest of the bindings: the small integers
//! and floats marshal as `number`, while `i64` / `u64` marshal as `bigint` (so pass a
//! `bigint` when the dtype is `i64` / `u64`). An unknown name, an out-of-range value,
//! or invalid UTF-8 throws an `Error` naming the accepted dtypes / formats.

use napi::bindgen_prelude::{BigInt, Buffer, Either};
use napi_derive::napi;

use yggdryl_buffer::IoPrimitive;
use yggdryl_converter::{ConverterKind, PrimitiveType, TypedConverter, Utf8Converter};

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

/// Resolves a converter-kind name, or a guided `Error`.
fn kind(name: &str) -> napi::Result<ConverterKind> {
    ConverterKind::from_name(name).map_err(to_error)
}

/// Resolves an optional dtype name (`undefined` passes through as no argument).
fn opt_dtype(name: Option<String>) -> napi::Result<Option<PrimitiveType>> {
    match name {
        Some(name) => Ok(Some(dtype(&name)?)),
        None => Ok(None),
    }
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

/// Widens a `bigint` to an `i128`, or errors if it exceeds 128 bits — so a huge or negative
/// value reaches the core range check (rule 12) instead of being silently truncated by
/// `get_i64()` / `get_u64()`.
fn bigint_to_i128(big: BigInt) -> napi::Result<i128> {
    let BigInt { sign_bit, words } = big;
    if words.len() > 2 {
        return Err(error_msg(
            "bigint is too large for any integer dtype (exceeds 128 bits)",
        ));
    }
    let lo = words.first().copied().unwrap_or(0) as u128;
    let hi = words.get(1).copied().unwrap_or(0) as u128;
    let magnitude = lo | (hi << 64);
    // |i128::MIN| is i128::MAX + 1, so a negative value may reach exactly that magnitude.
    let limit = if sign_bit {
        (i128::MAX as u128) + 1
    } else {
        i128::MAX as u128
    };
    if magnitude > limit {
        return Err(error_msg(
            "bigint is too large for any integer dtype (exceeds 128 bits)",
        ));
    }
    let signed = magnitude as i128; // wraps to i128::MIN exactly when magnitude == 2^127
    Ok(if sign_bit {
        signed.wrapping_neg()
    } else {
        signed
    })
}

/// Widens a JS `number` or `bigint` to an `i128` for the integer dtypes, rejecting a
/// non-whole `number`. The core [`PrimitiveType::int_to_le_bytes`] then range-checks it,
/// so both a `number` and a `bigint` work for any integer dtype and out-of-range values
/// raise the same guided message as Python — never a silent truncation.
fn as_i128(value: Either<f64, BigInt>, name: &str) -> napi::Result<i128> {
    match value {
        Either::A(number) => {
            if !number.is_finite() || number.fract() != 0.0 {
                return Err(error_msg(&format!(
                    "value {number} is not an integer; {name} needs a whole number"
                )));
            }
            Ok(number as i128)
        }
        Either::B(big) => bigint_to_i128(big),
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

/// Extracts a JS scalar as the `pt` element and returns its little-endian bytes. The
/// integer dtypes widen to `i128` and defer the range check to the core
/// [`PrimitiveType::int_to_le_bytes`], so out-of-range values raise the same guided error
/// as Python instead of `get_i64()` / `get_u64()` silently truncating them.
fn scalar_from_js(pt: PrimitiveType, value: Either<f64, BigInt>) -> napi::Result<Vec<u8>> {
    match pt {
        PrimitiveType::F32 => Ok((as_number(value)? as f32).to_le_vec()),
        PrimitiveType::F64 => Ok(as_number(value)?.to_le_vec()),
        _ => pt
            .int_to_le_bytes(as_i128(value, pt.name())?)
            .map_err(to_error),
    }
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

/// Converts a numeric scalar `value` from `fromDtype` to `toDtype` (C-style `as`),
/// e.g. `convert(300, "i32", "u8")` or `convert(3, "i32", "f32")`.
#[napi(namespace = "converter", js_name = "convert")]
pub fn convert(
    value: Either<f64, BigInt>,
    from_dtype: String,
    to_dtype: String,
) -> napi::Result<Either<f64, BigInt>> {
    let from = dtype(&from_dtype)?;
    let to = dtype(&to_dtype)?;
    let bytes = scalar_from_js(from, value)?;
    let out = from.cast_bytes(to, &bytes).map_err(to_error)?;
    scalar_to_js(to, &out)
}

/// Renders a `dtype` scalar `value` to its string form.
#[napi(namespace = "converter", js_name = "format")]
pub fn format(value: Either<f64, BigInt>, dtype_name: String) -> napi::Result<String> {
    let primitive = dtype(&dtype_name)?;
    let bytes = scalar_from_js(primitive, value)?;
    primitive.format_bytes(&bytes).map_err(to_error)
}

/// Runs a named converter forward over the whole `data` byte array — the general
/// "overall" convert. `converter` is one of `"cast"`, `"string"`, `"bytes"`, `"utf8"`;
/// `fromDtype` / `toDtype` name the dtype arguments the kind needs (both for `cast`,
/// one — `fromDtype` — for `string` / `bytes`, neither for `utf8`).
#[napi(namespace = "converter", js_name = "convertBytes")]
pub fn convert_bytes(
    data: Buffer,
    converter: String,
    from_dtype: Option<String>,
    to_dtype: Option<String>,
) -> napi::Result<Buffer> {
    let out = kind(&converter)?
        .convert_bytes(data.as_ref(), opt_dtype(from_dtype)?, opt_dtype(to_dtype)?)
        .map_err(to_error)?;
    Ok(out.into())
}

/// Runs a named converter backward over the whole `data` byte array — the exact
/// inverse of [`convert_bytes`], with the same arguments (e.g.
/// `invertBytes(le, 'string', 'i32')` renders i32 bytes back to their decimal text).
#[napi(namespace = "converter", js_name = "invertBytes")]
pub fn invert_bytes(
    data: Buffer,
    converter: String,
    from_dtype: Option<String>,
    to_dtype: Option<String>,
) -> napi::Result<Buffer> {
    let out = kind(&converter)?
        .invert_bytes(data.as_ref(), opt_dtype(from_dtype)?, opt_dtype(to_dtype)?)
        .map_err(to_error)?;
    Ok(out.into())
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
