//! The `yggdryl.types` namespace's **fixed-width value layer** — one nullable value
//! ([`Scalar`](yggdryl_core::io::fixed::Scalar)) and one nullable column
//! ([`Serie`](yggdryl_core::io::fixed::Serie)) per primitive: `U8Scalar`/`U8Serie` …
//! `I256Scalar`/`I256Serie`, `F16Scalar`/`F16Serie` … `F64Scalar`/`F64Serie`.
//!
//! Mirrors `yggdryl_core::io::fixed`'s generic `Scalar<T>` / `Serie<T>` method-for-method; each
//! wrapper is macro-generated and delegates to the core. A `Scalar` is an immutable value (with
//! `equals` / `hashCode` and a byte codec); a `Serie` is a mutable column with `length` / `get` /
//! `toOptions`.
//!
//! **Value marshaling** depends on the element width: the small integers (`u8`…`u32`, `i8`…`i32`)
//! cross as a JS `number`; the wide integers (`u64`/`i64`/`u128`/`i128`) as a **decimal string**
//! (exact at any width); the 96/256-bit integers (`u96`/`i96`/`u256`/`i256`), which have no
//! cross-language numeric form, as their **little-endian bytes** (a `Buffer`); and the floats
//! (`f16`/`f32`/`f64`) as a JS `number`. `null` / `undefined` is a null element throughout.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{Buffer, FromNapiValue, ToNapiValue};
use napi::{Env, JsUnknown, NapiRaw, NapiValue, ValueType};
use napi_derive::napi;

use yggdryl_core::io::fixed::Buffer as CoreBuffer;
use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::fixed::{f16, NativeType, Scalar, Serie, I256, I96, U256, U96};
use yggdryl_core::io::DataTypeId;

use crate::types::{DataType, Field};
use crate::varvalues::{BinaryScalar, Utf8Scalar};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

// ---- per-type value marshaling (element <-> JS) ---------------------------------------------

/// Small integers (`u8`…`u32`, `i8`…`i32`) — cross as a native JS `number` (`u32` / `i32`).
macro_rules! native_int_conv {
    ($t:ty, $js:ty, $to:ident, $from:ident) => {
        fn $to(value: $t) -> $js {
            <$js>::from(value)
        }
        fn $from(value: $js) -> napi::Result<$t> {
            <$t>::try_from(value).map_err(|_| {
                to_error(format!(
                    "{value} is out of range for {}",
                    <$t as NativeType>::NAME
                ))
            })
        }
    };
}
native_int_conv!(u8, u32, u8_to_js, u8_from_js);
native_int_conv!(u16, u32, u16_to_js, u16_from_js);
native_int_conv!(u32, u32, u32_to_js, u32_from_js);
native_int_conv!(i8, i32, i8_to_js, i8_from_js);
native_int_conv!(i16, i32, i16_to_js, i16_from_js);
native_int_conv!(i32, i32, i32_to_js, i32_from_js);

/// Wide native integers (`u64`/`i64`/`u128`/`i128`) — cross as an exact **decimal string**.
macro_rules! string_int_conv {
    ($t:ty, $to:ident, $from:ident) => {
        fn $to(value: $t) -> String {
            value.to_string()
        }
        fn $from(value: String) -> napi::Result<$t> {
            value.parse::<$t>().map_err(|_| {
                to_error(format!(
                    "{value:?} is not a valid {} (out of range or non-integer)",
                    <$t as NativeType>::NAME
                ))
            })
        }
    };
}
string_int_conv!(u64, u64_to_js, u64_from_js);
string_int_conv!(i64, i64_to_js, i64_from_js);
string_int_conv!(u128, u128_to_js, u128_from_js);
string_int_conv!(i128, i128_to_js, i128_from_js);

/// 96/256-bit integers (`u96`/`i96`/`u256`/`i256`) — no numeric form, so they cross as their
/// **little-endian bytes** (a `Buffer` of exactly `$n` bytes).
macro_rules! wide_int_conv {
    ($t:ty, $n:literal, $to:ident, $from:ident) => {
        fn $to(value: $t) -> Buffer {
            value.to_le_bytes().to_vec().into()
        }
        fn $from(value: Buffer) -> napi::Result<$t> {
            let bytes: [u8; $n] = value.as_ref().try_into().map_err(|_| {
                to_error(format!(
                    "{} expects exactly {} little-endian bytes, got {}",
                    <$t as NativeType>::NAME,
                    $n,
                    value.len()
                ))
            })?;
            Ok(<$t>::from_le_bytes(bytes))
        }
    };
}
wide_int_conv!(U96, 12, u96_to_js, u96_from_js);
wide_int_conv!(I96, 12, i96_to_js, i96_from_js);
wide_int_conv!(U256, 32, u256_to_js, u256_from_js);
wide_int_conv!(I256, 32, i256_to_js, i256_from_js);

/// Floats (`f16`/`f32`/`f64`) — cross as a native JS `number` (`f16` via `f32`).
fn f64_to_js(value: f64) -> f64 {
    value
}
fn f64_from_js(value: f64) -> napi::Result<f64> {
    Ok(value)
}
fn f32_to_js(value: f32) -> f64 {
    value as f64
}
fn f32_from_js(value: f64) -> napi::Result<f32> {
    Ok(value as f32)
}
fn f16_to_js(value: f16) -> f64 {
    value.to_f32() as f64
}
fn f16_from_js(value: f64) -> napi::Result<f16> {
    Ok(f16::from_f32(value as f32))
}

// ---- erased-value bridge primitives (shared with `varvalues` and `nested`) ------------------

/// Wraps any marshaled Rust value (`u32` / `i32` / `f64` / `String` / `Buffer` / `Null`) into an
/// erased JS value — the one place a heterogeneous leaf native crosses the boundary.
pub(crate) fn to_unknown<V: ToNapiValue>(env: Env, value: V) -> napi::Result<JsUnknown> {
    let raw = unsafe { V::to_napi_value(env.raw(), value)? };
    unsafe { JsUnknown::from_raw(env.raw(), raw) }
}

/// Extracts a concrete Rust value (`u32` / `i32` / `f64` / `String` / `Buffer`) from an erased JS
/// value — the inverse of [`to_unknown`], used when casting a JS value into a target leaf type.
pub(crate) fn from_unknown<V: FromNapiValue>(env: Env, value: &JsUnknown) -> napi::Result<V> {
    unsafe { V::from_napi_value(env.raw(), value.raw()) }
}

/// A JS value as an `i128` integer coefficient (or `None` for a JS `null` / `undefined`) — a finite,
/// **whole** `number` in the *strict* `i128` range, a `boolean` (`0` / `1`), or a decimal `string`. A
/// fractional / non-finite / out-of-range number, or any other type, is a guided error. This is the
/// **single** integer validation the deep-set path ([`fixed_js_to_le_bytes`]) and the `column()`
/// builder (nested's `build_int!`) share — so a deep `setAt` validates a JS `number` identically to
/// `column(_, "iNN")` instead of the ECMAScript `ToInt32` / `ToUint32` truncate-and-wrap that reading
/// it as an intermediate `u32` / `i32` would silently do.
pub(crate) fn js_int_value(env: Env, value: &JsUnknown) -> napi::Result<Option<i128>> {
    Ok(match value.get_type()? {
        ValueType::Null | ValueType::Undefined => None,
        ValueType::Boolean => Some(if from_unknown::<bool>(env, value)? {
            1
        } else {
            0
        }),
        ValueType::Number => {
            let number: f64 = from_unknown(env, value)?;
            // `i128::MAX as f64` rounds UP to `2^127` (one past the true max), so guard the top with a
            // strict `< 2^127` — else the saturating `as i128` silently clamps `2^127` to `i128::MAX`.
            if number.is_finite()
                && number.fract() == 0.0
                && number >= i128::MIN as f64
                && number < 2f64.powi(127)
            {
                Some(number as i128)
            } else {
                return Err(to_error(format!(
                    "the value {number} is not a whole number in range for an integer column"
                )));
            }
        }
        ValueType::String => {
            let text: String = from_unknown(env, value)?;
            Some(
                text.parse::<i128>()
                    .map_err(|_| to_error(format!("the value {text:?} is not a valid integer")))?,
            )
        }
        other => {
            return Err(to_error(format!(
                "expected an integer value for an integer column, got a {other:?} value"
            )))
        }
    })
}

/// A JS value as a `u128` (or `None` for a JS `null` / `undefined`) — a finite, whole, non-negative
/// `number` below `2^128`, a `boolean` (`0` / `1`), or a decimal `string` over the **full**
/// `[0, u128::MAX]` range (so a value above `i128::MAX`, e.g. `"2e38"`, still builds — the
/// `column(_, "u128")` fix that [`js_int_value`]'s `i128` range would otherwise halve). Anything else
/// is a guided error.
pub(crate) fn js_u128_value(env: Env, value: &JsUnknown) -> napi::Result<Option<u128>> {
    Ok(match value.get_type()? {
        ValueType::Null | ValueType::Undefined => None,
        ValueType::Boolean => Some(if from_unknown::<bool>(env, value)? {
            1
        } else {
            0
        }),
        ValueType::Number => {
            let number: f64 = from_unknown(env, value)?;
            if number.is_finite()
                && number.fract() == 0.0
                && number >= 0.0
                && number < 2f64.powi(128)
            {
                Some(number as u128)
            } else {
                return Err(to_error(format!(
                    "the value {number} is not a whole number in range for a u128 column"
                )));
            }
        }
        ValueType::String => {
            let text: String = from_unknown(env, value)?;
            Some(
                text.parse::<u128>()
                    .map_err(|_| to_error(format!("the value {text:?} is not a valid u128")))?,
            )
        }
        other => {
            return Err(to_error(format!(
                "expected an integer value for a u128 column, got a {other:?} value"
            )))
        }
    })
}

/// A fixed-width primitive leaf's canonical little-endian `bytes` (of type `id`) as its native JS
/// value — a `number`, an exact decimal `string`, or a little-endian `Buffer` — **reusing** the same
/// per-type marshaling the fixed `Scalar` / `Serie` wrappers use. `None` if `id` is not a fixed-width
/// primitive (a decimal / temporal / fixed-size-byte leaf is handled by the caller's byte fallback).
pub(crate) fn fixed_leaf_to_js(
    env: Env,
    id: DataTypeId,
    bytes: &[u8],
) -> napi::Result<Option<JsUnknown>> {
    macro_rules! arm {
        ($t:ty, $to:path) => {{
            let native = <$t as NativeType>::read_le(bytes);
            Some(to_unknown(env, $to(native))?)
        }};
    }
    Ok(match id {
        DataTypeId::U8 => arm!(u8, u8_to_js),
        DataTypeId::U16 => arm!(u16, u16_to_js),
        DataTypeId::U32 => arm!(u32, u32_to_js),
        DataTypeId::U64 => arm!(u64, u64_to_js),
        DataTypeId::U96 => arm!(U96, u96_to_js),
        DataTypeId::U128 => arm!(u128, u128_to_js),
        DataTypeId::U256 => arm!(U256, u256_to_js),
        DataTypeId::I8 => arm!(i8, i8_to_js),
        DataTypeId::I16 => arm!(i16, i16_to_js),
        DataTypeId::I32 => arm!(i32, i32_to_js),
        DataTypeId::I64 => arm!(i64, i64_to_js),
        DataTypeId::I96 => arm!(I96, i96_to_js),
        DataTypeId::I128 => arm!(i128, i128_to_js),
        DataTypeId::I256 => arm!(I256, i256_to_js),
        DataTypeId::F16 => arm!(f16, f16_to_js),
        DataTypeId::F32 => arm!(f32, f32_to_js),
        DataTypeId::F64 => arm!(f64, f64_to_js),
        _ => None,
    })
}

/// Casts a JS value into a fixed-width primitive `id`'s canonical little-endian bytes — **reusing**
/// the same per-type marshaling (and its guided range errors). `None` if `id` is not a fixed-width
/// primitive.
pub(crate) fn fixed_js_to_le_bytes(
    env: Env,
    value: &JsUnknown,
    id: DataTypeId,
) -> napi::Result<Option<Vec<u8>>> {
    // A small integer leaf (`u8`…`u32`, `i8`…`i32`): validate the JS value as an integer via the
    // shared [`js_int_value`] (finite, whole, in `i128` range) and range-check into the target width
    // — **never** the ECMAScript `ToInt32` / `ToUint32` truncate-and-wrap that reading it as the
    // intermediate `u32` / `i32` would do. So a deep `setAt(_, 5_000_000_000)` into an `i32` leaf, or
    // `setAt(_, 3.7)` into any int leaf, raises a guided error instead of silently storing a wrong
    // value — matching `column()`'s int builder and the flat scalar setter's out-of-range message.
    macro_rules! int_arm {
        ($t:ty) => {{
            let coeff = js_int_value(env, value)?
                .ok_or_else(|| to_error("expected an integer value, got null"))?;
            let native = <$t>::try_from(coeff).map_err(|_| {
                to_error(format!(
                    "{coeff} is out of range for {}",
                    <$t as NativeType>::NAME
                ))
            })?;
            let mut buf = [0u8; 32];
            native.write_le(&mut buf);
            Some(buf[..<$t as NativeType>::WIDTH].to_vec())
        }};
    }
    // A wide / string / float leaf: read the JS value directly in its cross-language form (a decimal
    // `string`, a little-endian `Buffer`, or a `number`) — the wide integers and the `u64` / `i64` /
    // `u128` / `i128` strings already carry their full range, and the floats are total.
    macro_rules! arm {
        ($t:ty, $js:ty, $from:path) => {{
            let js: $js = from_unknown(env, value)?;
            let native: $t = $from(js)?;
            let mut buf = [0u8; 32];
            native.write_le(&mut buf);
            Some(buf[..<$t as NativeType>::WIDTH].to_vec())
        }};
    }
    Ok(match id {
        DataTypeId::U8 => int_arm!(u8),
        DataTypeId::U16 => int_arm!(u16),
        DataTypeId::U32 => int_arm!(u32),
        DataTypeId::U64 => arm!(u64, String, u64_from_js),
        DataTypeId::U96 => arm!(U96, Buffer, u96_from_js),
        DataTypeId::U128 => arm!(u128, String, u128_from_js),
        DataTypeId::U256 => arm!(U256, Buffer, u256_from_js),
        DataTypeId::I8 => int_arm!(i8),
        DataTypeId::I16 => int_arm!(i16),
        DataTypeId::I32 => int_arm!(i32),
        DataTypeId::I64 => arm!(i64, String, i64_from_js),
        DataTypeId::I96 => arm!(I96, Buffer, i96_from_js),
        DataTypeId::I128 => arm!(i128, String, i128_from_js),
        DataTypeId::I256 => arm!(I256, Buffer, i256_from_js),
        DataTypeId::F16 => arm!(f16, f64, f16_from_js),
        DataTypeId::F32 => arm!(f32, f64, f32_from_js),
        DataTypeId::F64 => arm!(f64, f64, f64_from_js),
        _ => None,
    })
}

/// Generates the `Scalar` **and** `Serie` napi wrappers for one fixed-width element type.
///
/// `$js` is the JS-facing type; `$to` / `$from` are its marshaling functions (defined above).
macro_rules! napi_fixed {
    // Castable (`NumericCast`) types — the standard surface plus the numeric `to<Type>` casts and
    // the `toUtf8` bridge, spliced into the **single** impl block napi allows per struct.
    (numeric $Scalar:ident, $Serie:ident, $t:ty, $js:ty, $to:path, $from:path, $lit:literal) => {
        napi_fixed!(@impl $Scalar, $Serie, $t, $js, $to, $from, $lit,
            scalar_extra = {
                #[napi]
                pub fn to_u8(&self) -> napi::Result<U8Scalar> {
                    self.inner.cast::<u8>().map(|inner| U8Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u16(&self) -> napi::Result<U16Scalar> {
                    self.inner.cast::<u16>().map(|inner| U16Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u32(&self) -> napi::Result<U32Scalar> {
                    self.inner.cast::<u32>().map(|inner| U32Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u64(&self) -> napi::Result<U64Scalar> {
                    self.inner.cast::<u64>().map(|inner| U64Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i8(&self) -> napi::Result<I8Scalar> {
                    self.inner.cast::<i8>().map(|inner| I8Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i16(&self) -> napi::Result<I16Scalar> {
                    self.inner.cast::<i16>().map(|inner| I16Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i32(&self) -> napi::Result<I32Scalar> {
                    self.inner.cast::<i32>().map(|inner| I32Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i64(&self) -> napi::Result<I64Scalar> {
                    self.inner.cast::<i64>().map(|inner| I64Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i128(&self) -> napi::Result<I128Scalar> {
                    self.inner.cast::<i128>().map(|inner| I128Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f16(&self) -> napi::Result<F16Scalar> {
                    self.inner.cast::<f16>().map(|inner| F16Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f32(&self) -> napi::Result<F32Scalar> {
                    self.inner.cast::<f32>().map(|inner| F32Scalar { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f64(&self) -> napi::Result<F64Scalar> {
                    self.inner.cast::<f64>().map(|inner| F64Scalar { inner }).map_err(to_error)
                }
                /// This scalar as a **UTF-8** scalar — the value's decimal text (a null stays null).
                #[napi]
                pub fn to_utf8(&self) -> Utf8Scalar {
                    Utf8Scalar { inner: self.inner.to_utf8() }
                }
            },
            serie_extra = {
                #[napi]
                pub fn to_u8(&self) -> napi::Result<U8Serie> {
                    self.inner.cast::<u8>().map(|inner| U8Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u16(&self) -> napi::Result<U16Serie> {
                    self.inner.cast::<u16>().map(|inner| U16Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u32(&self) -> napi::Result<U32Serie> {
                    self.inner.cast::<u32>().map(|inner| U32Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_u64(&self) -> napi::Result<U64Serie> {
                    self.inner.cast::<u64>().map(|inner| U64Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i8(&self) -> napi::Result<I8Serie> {
                    self.inner.cast::<i8>().map(|inner| I8Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i16(&self) -> napi::Result<I16Serie> {
                    self.inner.cast::<i16>().map(|inner| I16Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i32(&self) -> napi::Result<I32Serie> {
                    self.inner.cast::<i32>().map(|inner| I32Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i64(&self) -> napi::Result<I64Serie> {
                    self.inner.cast::<i64>().map(|inner| I64Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_i128(&self) -> napi::Result<I128Serie> {
                    self.inner.cast::<i128>().map(|inner| I128Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f16(&self) -> napi::Result<F16Serie> {
                    self.inner.cast::<f16>().map(|inner| F16Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f32(&self) -> napi::Result<F32Serie> {
                    self.inner.cast::<f32>().map(|inner| F32Serie { inner }).map_err(to_error)
                }
                #[napi]
                pub fn to_f64(&self) -> napi::Result<F64Serie> {
                    self.inner.cast::<f64>().map(|inner| F64Serie { inner }).map_err(to_error)
                }
            });
    };
    // Wide (non-`NumericCast`) types — the standard surface only.
    (plain $Scalar:ident, $Serie:ident, $t:ty, $js:ty, $to:path, $from:path, $lit:literal) => {
        napi_fixed!(@impl $Scalar, $Serie, $t, $js, $to, $from, $lit,
            scalar_extra = {}, serie_extra = {});
    };
    (@impl $Scalar:ident, $Serie:ident, $t:ty, $js:ty, $to:path, $from:path, $lit:literal,
     scalar_extra = { $($scalar_extra:tt)* }, serie_extra = { $($serie_extra:tt)* }) => {
        #[doc = concat!("A single, nullable `", $lit, "` value.")]
        #[napi(namespace = "types")]
        pub struct $Scalar {
            pub(crate) inner: Scalar<$t>,
        }

        #[napi(namespace = "types")]
        impl $Scalar {
            /// A scalar from a value (`null` / `undefined` is null).
            #[napi(constructor)]
            pub fn new(value: Option<$js>) -> napi::Result<Self> {
                Ok(Self {
                    inner: match value {
                        Some(value) => Scalar::of($from(value)?),
                        None => Scalar::null(),
                    },
                })
            }

            /// The null scalar.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: Scalar::null(),
                }
            }

            /// The value, or `null` if null.
            #[napi(getter)]
            pub fn value(&self) -> Option<$js> {
                self.inner.value().map($to)
            }

            /// Whether the scalar is null.
            #[napi(getter)]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The element type's name (e.g. `"i64"`).
            #[napi(getter)]
            pub fn type_name(&self) -> &'static str {
                <$t as NativeType>::NAME
            }

            /// This scalar's [`DataType`].
            #[napi(getter)]
            pub fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this scalar's type (default nullable).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
                Field {
                    inner: CoreField::of(
                        &name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        nullable.unwrap_or(true),
                    ),
                }
            }

            /// This scalar broadcast to a length-1 column.
            #[napi]
            pub fn to_serie(&self) -> $Serie {
                $Serie {
                    inner: self.inner.to_serie(),
                }
            }

            /// This scalar as a **binary** scalar — the value's canonical little-endian bytes (a
            /// null stays null). The universal "any → binary" bridge; reverse with
            /// `BinaryScalar.to<Type>`.
            #[napi]
            pub fn to_binary(&self) -> BinaryScalar {
                BinaryScalar {
                    inner: self.inner.to_binary(),
                }
            }

            /// The scalar's canonical bytes (one validity byte then the little-endian value).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a scalar from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                Scalar::<$t>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality (bit-canonical).
            #[napi]
            pub fn equals(&self, other: &$Scalar) -> bool {
                self.inner == other.inner
            }

            /// A content hash consistent with [`equals`](Self::equals).
            #[napi]
            pub fn hash_code(&self) -> i32 {
                java_hash(&self.inner)
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                match self.inner.value() {
                    Some(value) => format!("{}({value:?})", stringify!($Scalar)),
                    None => format!("{}(null)", stringify!($Scalar)),
                }
            }

            $($scalar_extra)*
        }

        #[doc = concat!("A nullable column of `", $lit, "` values.")]
        #[napi(namespace = "types")]
        pub struct $Serie {
            pub(crate) inner: Serie<$t>,
        }

        #[napi(namespace = "types")]
        impl $Serie {
            /// A column from an array of value-or-`null` (empty by default).
            #[napi(constructor)]
            pub fn new(values: Option<Vec<Option<$js>>>) -> napi::Result<Self> {
                Ok(Self {
                    inner: match values {
                        None => Serie::new(),
                        Some(values) => {
                            let options: napi::Result<Vec<Option<$t>>> = values
                                .into_iter()
                                .map(|value| value.map($from).transpose())
                                .collect();
                            Serie::from_options(&options?)
                        }
                    },
                })
            }

            /// A non-null column from an array of present values.
            #[napi(factory)]
            pub fn from_values(values: Vec<$js>) -> napi::Result<Self> {
                let values: napi::Result<Vec<$t>> = values.into_iter().map($from).collect();
                Ok(Self {
                    inner: Serie::from_values(&values?),
                })
            }

            /// A length-1 column broadcasting `scalar`.
            #[napi(factory)]
            pub fn from_scalar(scalar: &$Scalar) -> Self {
                Self {
                    inner: Serie::from_scalar(scalar.inner.clone()),
                }
            }

            /// A column from an array of [`getScalar`](Self::get_scalar)-shaped scalars — a
            /// `null` / `undefined` item is the null scalar. Round-trips a column through its own
            /// scalars.
            #[napi(factory)]
            pub fn from_scalars(scalars: Vec<Option<&$Scalar>>) -> Self {
                let scalars: Vec<Scalar<$t>> = scalars
                    .into_iter()
                    .map(|slot| {
                        slot.map(|scalar| scalar.inner.clone())
                            .unwrap_or_else(Scalar::<$t>::null)
                    })
                    .collect();
                Self {
                    inner: Serie::from_scalars(&scalars),
                }
            }

            /// Appends one element (`null` / `undefined` is a null).
            #[napi]
            pub fn push(&mut self, value: Option<$js>) -> napi::Result<()> {
                self.inner.push(value.map($from).transpose()?);
                Ok(())
            }

            /// The element at `index`, or `null` if it is null or out of range.
            #[napi]
            pub fn get(&self, index: u32) -> Option<$js> {
                self.inner.get(index as usize).map($to)
            }

            /// The element at `index` as a scalar (null if null or out of range).
            #[napi]
            pub fn get_scalar(&self, index: u32) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index as usize),
                }
            }

            /// This column as a single scalar, if it holds exactly one element.
            #[napi]
            pub fn as_scalar(&self) -> Option<$Scalar> {
                self.inner.as_scalar().map(|inner| $Scalar { inner })
            }

            /// Overwrites element `index` (`null` writes a null); throws if out of range.
            #[napi]
            pub fn set(&mut self, index: u32, value: Option<$js>) -> napi::Result<()> {
                self.inner
                    .set(index as usize, value.map($from).transpose()?)
                    .map_err(to_error)
            }

            /// The number of elements.
            #[napi(getter)]
            pub fn length(&self) -> u32 {
                self.inner.len() as u32
            }

            /// The number of null elements.
            #[napi(getter)]
            pub fn null_count(&self) -> u32 {
                self.inner.null_count() as u32
            }

            /// Whether the column carries any nulls.
            #[napi(getter)]
            pub fn has_nulls(&self) -> bool {
                self.inner.has_nulls()
            }

            /// Whether the column is empty.
            #[napi]
            pub fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The elements as an array of value-or-`null`, in order.
            #[napi]
            pub fn to_options(&self) -> Vec<Option<$js>> {
                self.inner
                    .to_options()
                    .into_iter()
                    .map(|value| value.map($to))
                    .collect()
            }

            /// This column's [`DataType`].
            #[napi(getter)]
            pub fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming this column with explicit nullability (default nullable).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
                Field {
                    inner: CoreField::of(
                        &name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        nullable.unwrap_or(true),
                    ),
                }
            }

            /// A [`Field`] naming this column, nullability **inferred** from whether it holds nulls.
            #[napi]
            pub fn to_field(&self, name: String) -> Field {
                Field {
                    inner: CoreField::of(
                        &name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        self.inner.has_nulls(),
                    ),
                }
            }

            /// The column's canonical bytes (`[len][flags][validity?][values]`).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a column from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                Serie::<$t>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality (content, nulls included).
            #[napi]
            pub fn equals(&self, other: &$Serie) -> bool {
                self.inner == other.inner
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                format!(
                    "{}(len={}, nullCount={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.null_count()
                )
            }

            $($serie_extra)*
        }
    };
}

napi_fixed!(numeric U8Scalar, U8Serie, u8, u32, u8_to_js, u8_from_js, "u8");
napi_fixed!(numeric U16Scalar, U16Serie, u16, u32, u16_to_js, u16_from_js, "u16");
napi_fixed!(numeric U32Scalar, U32Serie, u32, u32, u32_to_js, u32_from_js, "u32");
napi_fixed!(numeric U64Scalar, U64Serie, u64, String, u64_to_js, u64_from_js, "u64");
napi_fixed!(plain U96Scalar, U96Serie, U96, Buffer, u96_to_js, u96_from_js, "u96");
napi_fixed!(plain U128Scalar, U128Serie, u128, String, u128_to_js, u128_from_js, "u128");
napi_fixed!(plain U256Scalar, U256Serie, U256, Buffer, u256_to_js, u256_from_js, "u256");
napi_fixed!(numeric I8Scalar, I8Serie, i8, i32, i8_to_js, i8_from_js, "i8");
napi_fixed!(numeric I16Scalar, I16Serie, i16, i32, i16_to_js, i16_from_js, "i16");
napi_fixed!(numeric I32Scalar, I32Serie, i32, i32, i32_to_js, i32_from_js, "i32");
napi_fixed!(numeric I64Scalar, I64Serie, i64, String, i64_to_js, i64_from_js, "i64");
napi_fixed!(plain I96Scalar, I96Serie, I96, Buffer, i96_to_js, i96_from_js, "i96");
napi_fixed!(numeric I128Scalar, I128Serie, i128, String, i128_to_js, i128_from_js, "i128");
napi_fixed!(plain I256Scalar, I256Serie, I256, Buffer, i256_to_js, i256_from_js, "i256");
napi_fixed!(numeric F16Scalar, F16Serie, f16, f64, f16_to_js, f16_from_js, "f16");
napi_fixed!(numeric F32Scalar, F32Serie, f32, f64, f32_to_js, f32_from_js, "f32");
napi_fixed!(numeric F64Scalar, F64Serie, f64, f64, f64_to_js, f64_from_js, "f64");

/// Generates the typed `Buffer` napi wrapper (a contiguous, non-nullable values store) for one
/// fixed-width element type — reusing the same `$to` / `$from` value marshaling as the scalars.
macro_rules! napi_buffer {
    ($Buffer:ident, $t:ty, $js:ty, $to:path, $from:path, $lit:literal) => {
        #[doc = concat!("A contiguous, non-nullable buffer of `", $lit, "` values.")]
        #[napi(namespace = "types")]
        pub struct $Buffer {
            pub(crate) inner: CoreBuffer<$t>,
        }

        #[napi(namespace = "types")]
        impl $Buffer {
            /// A buffer from an array of present values (empty by default).
            #[napi(constructor)]
            pub fn new(values: Option<Vec<$js>>) -> napi::Result<Self> {
                Ok(Self {
                    inner: match values {
                        None => CoreBuffer::new(),
                        Some(values) => {
                            let values: napi::Result<Vec<$t>> =
                                values.into_iter().map($from).collect();
                            CoreBuffer::from_slice(&values?)
                        }
                    },
                })
            }

            /// A buffer wrapping raw little-endian element `bytes` (the inverse of `toBytes`).
            #[napi(factory)]
            pub fn from_bytes(bytes: Buffer) -> Self {
                Self {
                    inner: CoreBuffer::from_bytes(&bytes),
                }
            }

            /// The number of elements.
            #[napi(getter)]
            pub fn count(&self) -> u32 {
                self.inner.count() as u32
            }

            /// The element at `index`, or `null` if out of range.
            #[napi]
            pub fn get(&self, index: u32) -> Option<$js> {
                self.inner.get(index as usize).map($to)
            }

            /// Overwrites element `index`; throws out of range.
            #[napi]
            pub fn set(&mut self, index: u32, value: $js) -> napi::Result<()> {
                if index as usize >= self.inner.count() {
                    return Err(to_error("Buffer index out of range"));
                }
                self.inner.set(index as usize, $from(value)?);
                Ok(())
            }

            /// Appends one element, growing the buffer.
            #[napi]
            pub fn push(&mut self, value: $js) -> napi::Result<()> {
                self.inner.push($from(value)?);
                Ok(())
            }

            /// The elements as an array, in order.
            #[napi]
            pub fn to_values(&self) -> Vec<$js> {
                (0..self.inner.count())
                    .map(|index| $to(self.inner.get(index).expect("index < count")))
                    .collect()
            }

            /// The raw little-endian element bytes (one copy) — the inverse of `fromBytes`.
            #[napi]
            pub fn to_bytes(&self) -> Buffer {
                self.inner.as_bytes().to_vec().into()
            }

            /// This buffer's [`DataType`].
            #[napi(getter)]
            pub fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this buffer's element type (default nullable).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
                Field {
                    inner: CoreField::of(
                        &name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        nullable.unwrap_or(true),
                    ),
                }
            }

            /// The number of elements.
            #[napi(getter)]
            pub fn length(&self) -> u32 {
                self.inner.count() as u32
            }

            /// Whether the buffer is empty.
            #[napi]
            pub fn is_empty(&self) -> bool {
                self.inner.count() == 0
            }

            /// Content equality (the raw bytes).
            #[napi]
            pub fn equals(&self, other: &$Buffer) -> bool {
                self.inner == other.inner
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                format!("{}(count={})", stringify!($Buffer), self.inner.count())
            }
        }
    };
}

napi_buffer!(U8Buffer, u8, u32, u8_to_js, u8_from_js, "u8");
napi_buffer!(U16Buffer, u16, u32, u16_to_js, u16_from_js, "u16");
napi_buffer!(U32Buffer, u32, u32, u32_to_js, u32_from_js, "u32");
napi_buffer!(U64Buffer, u64, String, u64_to_js, u64_from_js, "u64");
napi_buffer!(U96Buffer, U96, Buffer, u96_to_js, u96_from_js, "u96");
napi_buffer!(U128Buffer, u128, String, u128_to_js, u128_from_js, "u128");
napi_buffer!(U256Buffer, U256, Buffer, u256_to_js, u256_from_js, "u256");
napi_buffer!(I8Buffer, i8, i32, i8_to_js, i8_from_js, "i8");
napi_buffer!(I16Buffer, i16, i32, i16_to_js, i16_from_js, "i16");
napi_buffer!(I32Buffer, i32, i32, i32_to_js, i32_from_js, "i32");
napi_buffer!(I64Buffer, i64, String, i64_to_js, i64_from_js, "i64");
napi_buffer!(I96Buffer, I96, Buffer, i96_to_js, i96_from_js, "i96");
napi_buffer!(I128Buffer, i128, String, i128_to_js, i128_from_js, "i128");
napi_buffer!(I256Buffer, I256, Buffer, i256_to_js, i256_from_js, "i256");
napi_buffer!(F16Buffer, f16, f64, f16_to_js, f16_from_js, "f16");
napi_buffer!(F32Buffer, f32, f64, f32_to_js, f32_from_js, "f32");
napi_buffer!(F64Buffer, f64, f64, f64_to_js, f64_from_js, "f64");
