//! The `yggdryl.scalar` namespace — Arrow primitive scalars.
//!
//! Exposes one class per primitive scalar (`I8Scalar` … `F64Scalar`,
//! `BooleanScalar`), mirroring `yggdryl_scalar`. A scalar wraps a single, always-present
//! value (nullability is modelled separately — a `NullType` value and, later, union
//! types); each carries `value`, its `dataType` (a [`yggdryl.dtype`](super::dtype) class),
//! the byte codec, and value semantics. Two Node-specific idioms match the buffer layer's:
//! `i64` / `u64` scalar values marshal as `bigint` (napi has no native `u64` `number`),
//! and `F32Scalar` marshals its value over an `f64` JS boundary. Every primitive is present
//! (no omissions).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer, Null};
use napi_derive::napi;

use yggdryl_scalar::{Scalar, TypedScalar};

/// Maps a [`yggdryl_scalar::ScalarError`] to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Generates the napi wrapper for one primitive scalar whose value maps to a native napi
/// scalar (`$native`). `U64Scalar` and `F32Scalar` are hand-written below, since
/// their values need `bigint` / `f64` marshalling. napi does not expand a nested macro
/// inside a `#[napi] impl`, so the whole impl is produced here in one expansion.
macro_rules! napi_primitive_scalar {
    ($( ($scalar:ident, $dtype:ident, $native:ty, $lit:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A single `", $lit, "` value (always present).")]
            #[napi(namespace = "scalar")]
            pub struct $scalar {
                pub(crate) inner: yggdryl_scalar::$scalar,
            }

            #[napi(namespace = "scalar")]
            impl $scalar {
                #[napi(constructor)]
                pub fn new(value: $native) -> Self {
                    Self { inner: yggdryl_scalar::$scalar::new(value) }
                }

                /// The default scalar of this type (its data type's default value).
                #[napi(factory)]
                pub fn default_scalar() -> Self {
                    Self { inner: yggdryl_scalar::$scalar::default_scalar() }
                }

                /// The scalar's value (always present).
                #[napi(getter)]
                pub fn value(&self) -> $native {
                    TypedScalar::value(&self.inner)
                }

                /// The scalar's data type (a `yggdryl.dtype` class).
                #[napi(getter)]
                pub fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedScalar::data_type(&self.inner) }
                }

                /// The scalar serialised to its value's little-endian bytes.
                #[napi]
                pub fn serialize_bytes(&self) -> Buffer {
                    self.inner.serialize_bytes().into()
                }

                /// Reconstructs the scalar from its serialised bytes.
                #[napi(factory)]
                pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                    yggdryl_scalar::$scalar::deserialize_bytes(bytes.as_ref())
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Content equality.
                #[napi]
                pub fn equals(&self, other: &$scalar) -> bool {
                    self.inner == other.inner
                }

                /// Java-style `i32` content hash.
                #[napi]
                pub fn hash_code(&self) -> i32 {
                    let mut hasher = DefaultHasher::new();
                    self.inner.hash(&mut hasher);
                    let hash = hasher.finish();
                    (hash as u32 ^ (hash >> 32) as u32) as i32
                }
            }
        )+
    };
}

napi_primitive_scalar! {
    (I8Scalar, I8Type, i8, "int8"),
    (I16Scalar, I16Type, i16, "int16"),
    (I32Scalar, I32Type, i32, "int32"),
    (U8Scalar, U8Type, u8, "uint8"),
    (U16Scalar, U16Type, u16, "uint16"),
    (U32Scalar, U32Type, u32, "uint32"),
    (F64Scalar, F64Type, f64, "float64"),
    (BooleanScalar, BooleanType, bool, "boolean"),
}

/// A single `int64` value (always present). It marshals as a `bigint`, so 64-bit integers
/// survive the JS boundary without the precision loss of a `number`.
#[napi(namespace = "scalar")]
pub struct I64Scalar {
    pub(crate) inner: yggdryl_scalar::I64Scalar,
}

#[napi(namespace = "scalar")]
impl I64Scalar {
    #[napi(constructor)]
    pub fn new(value: BigInt) -> napi::Result<Self> {
        let (v, lossless) = value.get_i64();
        if !lossless {
            return Err(to_error(
                "value out of range for int64; expected -9223372036854775808..=9223372036854775807",
            ));
        }
        Ok(Self {
            inner: yggdryl_scalar::I64Scalar::new(v),
        })
    }

    /// The default scalar of this type (`0n`).
    #[napi(factory)]
    pub fn default_scalar() -> Self {
        Self {
            inner: yggdryl_scalar::I64Scalar::default_scalar(),
        }
    }

    /// The scalar's value (a `bigint`, always present).
    #[napi(getter)]
    pub fn value(&self) -> BigInt {
        BigInt::from(TypedScalar::value(&self.inner))
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::I64Type {
        crate::dtype::I64Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to its value's little-endian bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs the scalar from its serialised bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_scalar::I64Scalar::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &I64Scalar) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}

/// A single `uint64` value (always present). It marshals as a `bigint` (napi has no native
/// `u64` `number`).
#[napi(namespace = "scalar")]
pub struct U64Scalar {
    pub(crate) inner: yggdryl_scalar::U64Scalar,
}

#[napi(namespace = "scalar")]
impl U64Scalar {
    #[napi(constructor)]
    pub fn new(value: BigInt) -> napi::Result<Self> {
        let (sign_bit, v, lossless) = value.get_u64();
        if sign_bit || !lossless {
            return Err(to_error(
                "value out of range for uint64; expected 0..=18446744073709551615",
            ));
        }
        Ok(Self {
            inner: yggdryl_scalar::U64Scalar::new(v),
        })
    }

    /// The default scalar of this type (`0n`).
    #[napi(factory)]
    pub fn default_scalar() -> Self {
        Self {
            inner: yggdryl_scalar::U64Scalar::default_scalar(),
        }
    }

    /// The scalar's value (a `bigint`, always present).
    #[napi(getter)]
    pub fn value(&self) -> BigInt {
        BigInt::from(TypedScalar::value(&self.inner))
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::U64Type {
        crate::dtype::U64Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to its value's little-endian bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs the scalar from its serialised bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_scalar::U64Scalar::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &U64Scalar) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}

/// A single `float32` value (always present). It marshals over an `f64` JS boundary (napi
/// has no native `f32` `number`).
#[napi(namespace = "scalar")]
pub struct F32Scalar {
    pub(crate) inner: yggdryl_scalar::F32Scalar,
}

#[napi(namespace = "scalar")]
impl F32Scalar {
    #[napi(constructor)]
    pub fn new(value: f64) -> Self {
        Self {
            inner: yggdryl_scalar::F32Scalar::new(value as f32),
        }
    }

    /// The default scalar of this type (`0`).
    #[napi(factory)]
    pub fn default_scalar() -> Self {
        Self {
            inner: yggdryl_scalar::F32Scalar::default_scalar(),
        }
    }

    /// The scalar's value (widened to `f64`, always present).
    #[napi(getter)]
    pub fn value(&self) -> f64 {
        f64::from(TypedScalar::value(&self.inner))
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::F32Type {
        crate::dtype::F32Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to its value's little-endian bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs the scalar from its serialised bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_scalar::F32Scalar::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &F32Scalar) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}

/// The single value of the `null` data type — a scalar whose value is "null".
///
/// A scalar is always present, so this is not a nullable wrapper: it is the one value of
/// the sui-generis `NullType`. Its `value` is always `null` and it serialises to zero bytes.
#[napi(namespace = "scalar")]
pub struct NullScalar {
    pub(crate) inner: yggdryl_scalar::NullScalar,
}

#[napi(namespace = "scalar")]
impl NullScalar {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            inner: yggdryl_scalar::NullScalar::new(),
        }
    }

    /// The default scalar of this type — the null value.
    #[napi(factory)]
    pub fn default_scalar() -> Self {
        Self {
            inner: yggdryl_scalar::NullScalar::default_scalar(),
        }
    }

    /// The scalar's value — always `null` (the null value).
    #[napi(getter)]
    pub fn value(&self) -> Null {
        Null
    }

    /// The scalar's data type (a `yggdryl.dtype.NullType`).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to its (empty) value bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs the scalar from its serialised bytes (which must be empty).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_scalar::NullScalar::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &NullScalar) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}
