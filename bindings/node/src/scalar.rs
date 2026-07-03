//! The `yggdryl.scalar` namespace — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer type is exposed as its scalar and its null-or-value optional
//! scalar (e.g. `Int64`, `OptionalInt64`), alongside `Binary` / `OptionalBinary`
//! (whose value is held as a core positioned-IO `ByteBuffer` — `toIo()` hands one
//! back) and `Null` — the same bare names as the Rust crate, the namespace
//! carrying the concern. Values adapt to JS idioms: the 8–32 bit types use
//! `number`, the 64-bit types use `BigInt`, and scalars expose the `as*`
//! accessors with the core contract — the value when the target represents it
//! exactly, or a thrown error naming the fix (strings and `Buffer`s cross the FFI
//! boundary as new JS objects, so the Rust-side "borrow, never copy" guarantee
//! applies up to that boundary copy). Optional scalars adapt construction to
//! idioms: they are built straight from the native value (`new
//! OptionalInt64(42n)`), the inner scalar being an implementation detail
//! reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `DefaultScalar` traits (generic Rust bounds; the bindings reach defaults
//! through a data type's `defaultScalar()`), and the nested scalars — the generic
//! `Serie` / `Map` / `Struct` and the buffer-backed `Int64Serie` (whose zero-copy
//! Arrow buffers await C Data Interface interop) — which have no concrete FFI
//! shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_scalar::RawScalar;

use crate::{bigint_to_i64, bigint_to_u64, data_error, wire_to_native};

/// Reads `as_str` through the optional charset name — `"utf8"` (the default) or
/// `"latin1"` — shared by every scalar class.
fn as_str_with<D: yggdryl_dtype::RawDataType, S: RawScalar<D>>(
    scalar: &S,
    charset: Option<&str>,
) -> Result<String> {
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
#[napi]
#[derive(Default)]
pub struct ScalarNull {
    pub(crate) inner: yggdryl_scalar::Null,
}

#[napi]
impl ScalarNull {
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
    pub fn data_type(&self) -> crate::dtype::DtypeNull {
        crate::dtype::DtypeNull::default()
    }
}

/// Generates the `as*` accessor block for a scalar wrapper class: the value when
/// exactly representable, or a thrown error naming the fix, with the 64-bit
/// targets as `BigInt` (a separate `#[napi]` impl block — napi merges the blocks
/// into one JS class).
macro_rules! as_accessors_node {
    ($class:ident) => {
        #[napi]
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
#[napi]
pub struct ScalarBinary {
    pub(crate) inner: yggdryl_scalar::Binary,
}

#[napi]
impl ScalarBinary {
    /// A `binary` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_scalar::Binary::new(value.to_vec()),
        }
    }

    /// A null `binary` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Binary::null(),
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
    pub fn data_type(&self) -> crate::dtype::DtypeBinary {
        crate::dtype::DtypeBinary::default()
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

as_accessors_node!(ScalarBinary);

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[napi]
pub struct ScalarOptionalBinary {
    pub(crate) inner: yggdryl_scalar::Optional<yggdryl_dtype::Binary, yggdryl_scalar::Binary>,
}

#[napi]
impl ScalarOptionalBinary {
    /// A scalar holding the `binary` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_scalar::Optional::new(yggdryl_scalar::Binary::new(value.to_vec())),
        }
    }

    /// The null variant.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Optional::null(),
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
    pub fn scalar(&self) -> Option<ScalarBinary> {
        self.inner.scalar().map(|scalar| ScalarBinary {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::DtypeOptionalBinary {
        crate::dtype::DtypeOptionalBinary::default()
    }
}

as_accessors_node!(ScalarOptionalBinary);

/// Generates the width-independent surface of one integer type's scalars: the
/// null factory, nullness, `dataType` and `scalar` of `$ty` and `$opt_ty`. The
/// width-dependent constructor and `value` are generated by
/// [`int_wire_number_scalar!`] (8–32 bit, JS `number`) or written per 64-bit type
/// with `BigInt`; the `as*` accessors come from [`as_accessors_node!`].
macro_rules! int_scalar_node {
    ($ty:ident, $opt_ty:ident, $inner:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A single, possibly-null `", $name, "` value.")]
        #[napi]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$inner,
        }

        #[napi]
        impl $ty {
            #[doc = concat!("A null `", $name, "` scalar.")]
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::$inner::null(),
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
        #[napi]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_scalar::Optional<yggdryl_dtype::$inner, yggdryl_scalar::$inner>,
        }

        #[napi]
        impl $opt_ty {
            /// The null variant.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::Optional::null(),
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
    ($ty:ident, $opt_ty:ident, $inner:ident, $native:ty, $name:literal) => {
        #[napi]
        impl $ty {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_scalar::$inner::new(wire_to_native::<$native>(value, $name)?),
                })
            }

            /// The scalar's value, or `null` when null.
            #[napi]
            pub fn value(&self) -> Option<i64> {
                self.inner.value().copied().map(i64::from)
            }
        }

        #[napi]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_scalar::Optional::new(yggdryl_scalar::$inner::new(
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
    ScalarInt8,
    ScalarOptionalInt8,
    Int8,
    DtypeInt8,
    DtypeOptionalInt8,
    "int8"
);
int_scalar_node!(
    ScalarInt16,
    ScalarOptionalInt16,
    Int16,
    DtypeInt16,
    DtypeOptionalInt16,
    "int16"
);
int_scalar_node!(
    ScalarInt32,
    ScalarOptionalInt32,
    Int32,
    DtypeInt32,
    DtypeOptionalInt32,
    "int32"
);
int_scalar_node!(
    ScalarInt64,
    ScalarOptionalInt64,
    Int64,
    DtypeInt64,
    DtypeOptionalInt64,
    "int64"
);
int_scalar_node!(
    ScalarUInt8,
    ScalarOptionalUInt8,
    UInt8,
    DtypeUInt8,
    DtypeOptionalUInt8,
    "uint8"
);
int_scalar_node!(
    ScalarUInt16,
    ScalarOptionalUInt16,
    UInt16,
    DtypeUInt16,
    DtypeOptionalUInt16,
    "uint16"
);
int_scalar_node!(
    ScalarUInt32,
    ScalarOptionalUInt32,
    UInt32,
    DtypeUInt32,
    DtypeOptionalUInt32,
    "uint32"
);
int_scalar_node!(
    ScalarUInt64,
    ScalarOptionalUInt64,
    UInt64,
    DtypeUInt64,
    DtypeOptionalUInt64,
    "uint64"
);

int_wire_number_scalar!(ScalarInt8, ScalarOptionalInt8, Int8, i8, "int8");
int_wire_number_scalar!(ScalarInt16, ScalarOptionalInt16, Int16, i16, "int16");
int_wire_number_scalar!(ScalarInt32, ScalarOptionalInt32, Int32, i32, "int32");
int_wire_number_scalar!(ScalarUInt8, ScalarOptionalUInt8, UInt8, u8, "uint8");
int_wire_number_scalar!(ScalarUInt16, ScalarOptionalUInt16, UInt16, u16, "uint16");
int_wire_number_scalar!(ScalarUInt32, ScalarOptionalUInt32, UInt32, u32, "uint32");

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent surface is written out per type.

#[napi]
impl ScalarInt64 {
    /// An `int64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::Int64::new(bigint_to_i64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi]
impl ScalarOptionalInt64 {
    /// A scalar holding the `int64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::Optional::new(yggdryl_scalar::Int64::new(bigint_to_i64(value)?)),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi]
impl ScalarUInt64 {
    /// A `uint64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::UInt64::new(bigint_to_u64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi]
impl ScalarOptionalUInt64 {
    /// A scalar holding the `uint64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::Optional::new(yggdryl_scalar::UInt64::new(bigint_to_u64(
                value,
            )?)),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}
