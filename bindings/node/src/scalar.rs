//! The `yggdryl.scalar` namespace — Arrow primitive scalars.
//!
//! Exposes one class per primitive scalar (`I8Scalar` … `F64Scalar`,
//! `BooleanScalar`), mirroring `yggdryl_scalar`. A scalar wraps a single value or is
//! null; each carries `value`, `isNull`, its `dataType` (a
//! [`yggdryl.dtype`](super::dtype) class), the byte codec, and value semantics. Two
//! Node-specific idioms match the buffer layer's: `i64` / `u64` scalar values marshal as
//! `bigint` (napi has no native `u64` `number`), and `F32Scalar` marshals its value
//! over an `f64` JS boundary. Every primitive is present (no omissions).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer};
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
            #[doc = concat!("A single, possibly-null `", $lit, "` value.")]
            #[napi(namespace = "scalar")]
            pub struct $scalar {
                pub(crate) inner: yggdryl_scalar::$scalar,
            }

            #[napi(namespace = "scalar")]
            impl $scalar {
                #[napi(constructor)]
                pub fn new(value: Option<$native>) -> Self {
                    let inner = match value {
                        Some(value) => yggdryl_scalar::$scalar::new(value),
                        None => yggdryl_scalar::$scalar::null(),
                    };
                    Self { inner }
                }

                /// A null scalar of this type.
                #[napi(factory)]
                pub fn null() -> Self {
                    Self { inner: yggdryl_scalar::$scalar::null() }
                }

                /// The scalar's value, or `null` when the scalar is null.
                #[napi(getter)]
                pub fn value(&self) -> Option<$native> {
                    TypedScalar::value(&self.inner)
                }

                /// Whether the scalar holds no value.
                #[napi(getter)]
                pub fn is_null(&self) -> bool {
                    self.inner.is_null()
                }

                /// The scalar's data type (a `yggdryl.dtype` class).
                #[napi(getter)]
                pub fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedScalar::data_type(&self.inner) }
                }

                /// The scalar serialised to bytes (a null flag + the value's bytes).
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

/// A single, possibly-null `int64` value. Its value marshals as a `bigint`, so 64-bit
/// integers survive the JS boundary without the precision loss of a `number`.
#[napi(namespace = "scalar")]
pub struct I64Scalar {
    pub(crate) inner: yggdryl_scalar::I64Scalar,
}

#[napi(namespace = "scalar")]
impl I64Scalar {
    #[napi(constructor)]
    pub fn new(value: Option<BigInt>) -> Self {
        let inner = match value {
            Some(big) => yggdryl_scalar::I64Scalar::new(big.get_i64().0),
            None => yggdryl_scalar::I64Scalar::null(),
        };
        Self { inner }
    }

    /// A null `int64` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::I64Scalar::null(),
        }
    }

    /// The scalar's value (a `bigint`), or `null` when the scalar is null.
    #[napi(getter)]
    pub fn value(&self) -> Option<BigInt> {
        TypedScalar::value(&self.inner).map(BigInt::from)
    }

    /// Whether the scalar holds no value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::I64Type {
        crate::dtype::I64Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to bytes (a null flag + the value's bytes).
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

/// A single, possibly-null `uint64` value. Its value marshals as a `bigint` (napi has no
/// native `u64` `number`).
#[napi(namespace = "scalar")]
pub struct U64Scalar {
    pub(crate) inner: yggdryl_scalar::U64Scalar,
}

#[napi(namespace = "scalar")]
impl U64Scalar {
    #[napi(constructor)]
    pub fn new(value: Option<BigInt>) -> Self {
        let inner = match value {
            Some(big) => yggdryl_scalar::U64Scalar::new(big.get_u64().1),
            None => yggdryl_scalar::U64Scalar::null(),
        };
        Self { inner }
    }

    /// A null `uint64` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::U64Scalar::null(),
        }
    }

    /// The scalar's value (a `bigint`), or `null` when the scalar is null.
    #[napi(getter)]
    pub fn value(&self) -> Option<BigInt> {
        TypedScalar::value(&self.inner).map(BigInt::from)
    }

    /// Whether the scalar holds no value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::U64Type {
        crate::dtype::U64Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to bytes (a null flag + the value's bytes).
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

/// A single, possibly-null `float32` value. Its value marshals over an `f64` JS boundary
/// (napi has no native `f32` `number`).
#[napi(namespace = "scalar")]
pub struct F32Scalar {
    pub(crate) inner: yggdryl_scalar::F32Scalar,
}

#[napi(namespace = "scalar")]
impl F32Scalar {
    #[napi(constructor)]
    pub fn new(value: Option<f64>) -> Self {
        let inner = match value {
            Some(value) => yggdryl_scalar::F32Scalar::new(value as f32),
            None => yggdryl_scalar::F32Scalar::null(),
        };
        Self { inner }
    }

    /// A null `float32` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::F32Scalar::null(),
        }
    }

    /// The scalar's value (widened to `f64`), or `null` when the scalar is null.
    #[napi(getter)]
    pub fn value(&self) -> Option<f64> {
        TypedScalar::value(&self.inner).map(f64::from)
    }

    /// Whether the scalar holds no value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type (a `yggdryl.dtype` class).
    #[napi(getter)]
    pub fn data_type(&self) -> crate::dtype::F32Type {
        crate::dtype::F32Type {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to bytes (a null flag + the value's bytes).
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
