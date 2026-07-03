//! The `yggdryl.scalar` namespace — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer type is exposed as its scalar and its null-or-value optional
//! scalar (e.g. `Int64Scalar`, `OptionalInt64Scalar`), alongside `BinaryScalar` /
//! `OptionalBinaryScalar` (whose value is held as a core positioned-IO
//! `ByteBuffer` — `toIo()` hands one back) and `NullScalar` — the same
//! globally-unique names as the Rust crate, the namespace carrying the concern
//! (the `…Scalar` suffix keeps every class distinct in napi's addon-global
//! registry). Values adapt to JS idioms: the 8–32 bit types use `number`, the
//! 64-bit types use `BigInt`, and scalars expose the `as*` accessors with the core
//! contract — the value when the target represents it exactly, or a thrown error
//! naming the fix (strings and `Buffer`s cross the FFI boundary as new JS objects,
//! so the Rust-side "borrow, never copy" guarantee applies up to that boundary
//! copy). Optional scalars adapt construction to idioms: they are built straight
//! from the native value (`new OptionalInt64Scalar(42n)`), the inner scalar being
//! an implementation detail reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `ScalarFactory` traits (generic Rust bounds; the bindings reach the factories
//! through a data type's `field()` / `scalar()` / `defaultScalar()`), and the
//! nested scalars — the generic `Serie` / `MapScalar` / `StructScalar` and the
//! buffer-backed `Int64Serie` (whose zero-copy Arrow buffers await C Data
//! Interface interop) — which have no concrete FFI shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_scalar::Scalar;

use crate::{bigint_to_i64, bigint_to_u64, data_error, wire_to_native};

/// Reads `as_str` through the optional charset name — `"utf8"` (the default) or
/// `"latin1"` — shared by every scalar class.
fn as_str_with<S: Scalar>(scalar: &S, charset: Option<&str>) -> Result<String> {
    let decoded = match charset {
        None | Some("utf8") => scalar.as_str(None),
        Some("latin1") => scalar.as_str(Some(&yggdryl_core::Latin1)),
        Some(other) => {
            return Err(Error::from_reason(format!(
                "unknown charset \"{other}\"; expected \"utf8\" or \"latin1\""
            )))
        }
    };
    decoded
        .map(std::borrow::Cow::into_owned)
        .map_err(data_error)
}

/// The `null` scalar: always null, holding no value.
#[napi(namespace = "scalar")]
#[derive(Default)]
pub struct NullScalar {
    pub(crate) inner: yggdryl_scalar::NullScalar,
}

#[napi(namespace = "scalar")]
impl NullScalar {
    /// The null scalar.
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Always `true`.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType::default()
    }
}

/// Generates the `as*` accessor block for a scalar wrapper class: the value when
/// exactly representable, or a thrown error naming the fix, with the 64-bit
/// targets as `BigInt` (a separate `#[napi]` impl block — napi merges the blocks
/// into one JS class).
macro_rules! as_accessors_node {
    ($class:ident) => {
        #[napi(namespace = "scalar")]
        impl $class {
            /// The value as a number in the i8 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_i8(&self) -> Result<i32> {
                self.inner.as_i8().map(i32::from).map_err(data_error)
            }
            /// The value as a number in the i16 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_i16(&self) -> Result<i32> {
                self.inner.as_i16().map(i32::from).map_err(data_error)
            }
            /// The value as a number in the i32 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_i32(&self) -> Result<i32> {
                self.inner.as_i32().map_err(data_error)
            }
            /// The value as a `BigInt` in the i64 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_i64(&self) -> Result<BigInt> {
                self.inner.as_i64().map(BigInt::from).map_err(data_error)
            }
            /// The value as a number in the u8 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_u8(&self) -> Result<u32> {
                self.inner.as_u8().map(u32::from).map_err(data_error)
            }
            /// The value as a number in the u16 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_u16(&self) -> Result<u32> {
                self.inner.as_u16().map(u32::from).map_err(data_error)
            }
            /// The value as a number in the u32 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_u32(&self) -> Result<u32> {
                self.inner.as_u32().map_err(data_error)
            }
            /// The value as a `BigInt` in the u64 range; throws when null or not
            /// exactly representable.
            #[napi]
            pub fn as_u64(&self) -> Result<BigInt> {
                self.inner.as_u64().map(BigInt::from).map_err(data_error)
            }
            /// The value as a number; throws when null or not exactly
            /// representable in f32.
            #[napi]
            pub fn as_f32(&self) -> Result<f32> {
                self.inner.as_f32().map_err(data_error)
            }
            /// The value as a number; throws when null or not exactly
            /// representable in f64.
            #[napi]
            pub fn as_f64(&self) -> Result<f64> {
                self.inner.as_f64().map_err(data_error)
            }
            /// The value as a boolean; throws when null or the value is not a
            /// boolean.
            #[napi]
            pub fn as_bool(&self) -> Result<bool> {
                self.inner.as_bool().map_err(data_error)
            }
            /// The value as a string; `charset` picks the decoder (`"utf8"`,
            /// the default, or `"latin1"`); throws when null or not decodable.
            #[napi]
            pub fn as_str(&self, charset: Option<String>) -> Result<String> {
                as_str_with(&self.inner, charset.as_deref())
            }
            /// The value as a `Buffer`; throws when null or the value has no
            /// byte-sequence form.
            #[napi]
            pub fn as_bytes(&self) -> Result<Buffer> {
                self.inner
                    .as_bytes()
                    .map(|bytes| Buffer::from(bytes.to_vec()))
                    .map_err(data_error)
            }
        }
    };
}

/// A single, possibly-null `binary` value, holding its bytes as a core
/// positioned-IO `ByteBuffer` (`toIo()` hands one back).
#[napi(namespace = "scalar")]
pub struct BinaryScalar {
    pub(crate) inner: yggdryl_scalar::BinaryScalar,
}

#[napi(namespace = "scalar")]
impl BinaryScalar {
    /// A `binary` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_scalar::BinaryScalar::new(value.to_vec()),
        }
    }

    /// A null `binary` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::BinaryScalar::null(),
        }
    }

    /// Whether this scalar holds a null value.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's value as a `Buffer`, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<Buffer> {
        self.inner.value().map(|bytes| Buffer::from(bytes.to_vec()))
    }

    /// The scalar's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::BinaryType {
        crate::dtype::BinaryType::default()
    }

    /// The value as a core IO `ByteBuffer` (`yggdryl.core`), ready for positioned
    /// reads and the cursor / slice adapters, or `null` when null (the bytes
    /// cross the FFI boundary as one copy).
    #[napi]
    pub fn to_io(&self) -> Option<crate::core::ByteBuffer> {
        self.inner
            .io()
            .map(|io| crate::core::ByteBuffer::from_inner(io.clone()))
    }

    /// The value as a full-window core IO `ByteBufferSlice` (`yggdryl.core`) —
    /// window-relative positioned reads — or `null` when null (one copy at the
    /// FFI boundary).
    #[napi]
    pub fn to_io_slice(&self) -> Option<crate::core::ByteBufferSlice> {
        self.inner
            .clone()
            .into_io_slice()
            .map(crate::core::ByteBufferSlice::from_inner)
    }
}

as_accessors_node!(BinaryScalar);

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[napi(namespace = "scalar")]
pub struct OptionalBinaryScalar {
    pub(crate) inner:
        yggdryl_scalar::OptionalScalar<yggdryl_dtype::BinaryType, yggdryl_scalar::BinaryScalar>,
}

#[napi(namespace = "scalar")]
impl OptionalBinaryScalar {
    /// A scalar holding the `binary` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::BinaryScalar::new(
                value.to_vec(),
            )),
        }
    }

    /// The null variant.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::OptionalScalar::null(),
        }
    }

    /// Whether this scalar holds the null variant.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The value as a `Buffer`, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<Buffer> {
        self.inner.value().map(|bytes| Buffer::from(bytes.to_vec()))
    }

    /// The inner scalar, when this holds the value variant.
    #[napi]
    pub fn scalar(&self) -> Option<BinaryScalar> {
        self.inner.scalar().map(|scalar| BinaryScalar {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::OptionalBinaryType {
        crate::dtype::OptionalBinaryType::default()
    }
}

as_accessors_node!(OptionalBinaryScalar);

/// Generates the width-independent surface of one integer type's scalars: the
/// null factory, nullness, `dataType` and `scalar` of `$ty` and `$opt_ty`. The
/// width-dependent constructor and `value` are generated by
/// [`int_wire_number_scalar!`] (8–32 bit, JS `number`) or written per 64-bit type
/// with `BigInt`; the `as*` accessors come from [`as_accessors_node!`].
macro_rules! int_scalar_node {
    ($ty:ident, $opt_ty:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A single, possibly-null `", $name, "` value.")]
        #[napi(namespace = "scalar")]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[napi(namespace = "scalar")]
        impl $ty {
            #[doc = concat!("A null `", $name, "` scalar.")]
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::null(),
                }
            }

            /// Whether this scalar holds a null value.
            #[napi]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The scalar's data type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }
        }

        as_accessors_node!($ty);

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[napi(namespace = "scalar")]
        pub struct $opt_ty {
            pub(crate) inner:
                yggdryl_scalar::OptionalScalar<yggdryl_dtype::$dtype, yggdryl_scalar::$ty>,
        }

        #[napi(namespace = "scalar")]
        impl $opt_ty {
            /// The null variant.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::OptionalScalar::null(),
                }
            }

            /// Whether this scalar holds the null variant.
            #[napi]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The inner scalar, when this holds the value variant.
            #[napi]
            pub fn scalar(&self) -> Option<$ty> {
                self.inner.scalar().map(|scalar| $ty { inner: *scalar })
            }

            /// The scalar's data type: the logical optional of the value type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$opt_dtype {
                crate::dtype::$opt_dtype::default()
            }
        }

        as_accessors_node!($opt_ty);
    };
}

/// Generates the width-dependent constructor and `value` of an 8–32 bit integer
/// scalar (and its optional) over JS `number`, range-checked with an actionable
/// error.
macro_rules! int_wire_number_scalar {
    ($ty:ident, $opt_ty:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "scalar")]
        impl $ty {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_scalar::$ty::new(wire_to_native::<$native>(value, $name)?),
                })
            }

            /// The scalar's value, or `null` when null.
            #[napi]
            pub fn value(&self) -> Option<i64> {
                self.inner.value().copied().map(i64::from)
            }
        }

        #[napi(namespace = "scalar")]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::$ty::new(
                        wire_to_native::<$native>(value, $name)?,
                    )),
                })
            }

            /// The value, or `null` for the null variant.
            #[napi]
            pub fn value(&self) -> Option<i64> {
                self.inner.value().copied().map(i64::from)
            }
        }
    };
}

int_scalar_node!(
    Int8Scalar,
    OptionalInt8Scalar,
    Int8Type,
    OptionalInt8Type,
    "int8"
);
int_scalar_node!(
    Int16Scalar,
    OptionalInt16Scalar,
    Int16Type,
    OptionalInt16Type,
    "int16"
);
int_scalar_node!(
    Int32Scalar,
    OptionalInt32Scalar,
    Int32Type,
    OptionalInt32Type,
    "int32"
);
int_scalar_node!(
    Int64Scalar,
    OptionalInt64Scalar,
    Int64Type,
    OptionalInt64Type,
    "int64"
);
int_scalar_node!(
    UInt8Scalar,
    OptionalUInt8Scalar,
    UInt8Type,
    OptionalUInt8Type,
    "uint8"
);
int_scalar_node!(
    UInt16Scalar,
    OptionalUInt16Scalar,
    UInt16Type,
    OptionalUInt16Type,
    "uint16"
);
int_scalar_node!(
    UInt32Scalar,
    OptionalUInt32Scalar,
    UInt32Type,
    OptionalUInt32Type,
    "uint32"
);
int_scalar_node!(
    UInt64Scalar,
    OptionalUInt64Scalar,
    UInt64Type,
    OptionalUInt64Type,
    "uint64"
);

int_wire_number_scalar!(Int8Scalar, OptionalInt8Scalar, i8, "int8");
int_wire_number_scalar!(Int16Scalar, OptionalInt16Scalar, i16, "int16");
int_wire_number_scalar!(Int32Scalar, OptionalInt32Scalar, i32, "int32");
int_wire_number_scalar!(UInt8Scalar, OptionalUInt8Scalar, u8, "uint8");
int_wire_number_scalar!(UInt16Scalar, OptionalUInt16Scalar, u16, "uint16");
int_wire_number_scalar!(UInt32Scalar, OptionalUInt32Scalar, u32, "uint32");

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent surface is written out per type.

#[napi(namespace = "scalar")]
impl Int64Scalar {
    /// An `int64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::Int64Scalar::new(bigint_to_i64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "scalar")]
impl OptionalInt64Scalar {
    /// A scalar holding the `int64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::Int64Scalar::new(
                bigint_to_i64(value)?,
            )),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "scalar")]
impl UInt64Scalar {
    /// A `uint64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::UInt64Scalar::new(bigint_to_u64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "scalar")]
impl OptionalUInt64Scalar {
    /// A scalar holding the `uint64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::UInt64Scalar::new(
                bigint_to_u64(value)?,
            )),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}
