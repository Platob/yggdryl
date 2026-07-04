//! The `yggdryl.scalar` namespace — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer type is exposed as its scalar and its null-or-value optional
//! scalar (e.g. `Int64Scalar`, `OptionalInt64Scalar`), alongside `BinaryScalar` /
//! `OptionalBinaryScalar` (whose value is held as a core positioned-IO
//! `ByteBuffer` — `toIo()` hands one back), `Utf8Scalar` / `OptionalUtf8Scalar`
//! (the `utf8` string, crossing as a JS `string`; its core `Utf8Buffer` resource
//! stays Rust-only), `NullScalar` and its serie scalar
//! (e.g. `Int64Serie`, the buffer-backed `list` of `int64`) — the same
//! globally-unique names as the Rust crate, the namespace carrying the
//! concern (the `…Scalar` suffix keeps every class distinct in napi's addon-global
//! registry). Values adapt to JS idioms: the 8–32 bit integers and the floats
//! (`float16` / `float32` / `float64`, the `float16` lossily narrowed) use
//! `number`, the 64-bit integers use `BigInt`, and
//! scalars expose the `as*` accessors with the core
//! contract — the value when the target represents it exactly, or a thrown error
//! naming the fix (strings and `Buffer`s cross the FFI boundary as new JS objects,
//! so the Rust-side "borrow, never copy" guarantee applies up to that boundary
//! copy). Optional scalars adapt construction to idioms: they are built straight
//! from the native value (`new OptionalInt64Scalar(42n)`), the inner scalar being
//! an implementation detail reachable through `scalar()`. `RecordScalar` is the
//! `struct` row, built from a plain JS object whose members are inferred like the
//! factory's, read back per field with `get(name)`. Every scalar class carries
//! `toJsValue()`, the general native-value accessor: the whole native value
//! (`null` when null, a record as a plain `{field: value}` object) built in one
//! FFI call — conversions happen once in Rust, never in per-element JS loops.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow_scalar` / `to_arrow_array` / `from_arrow`, and `castDtype` /
//! `castDtypeUnchecked` which return a re-typed `arrow-array` value — all exchange
//! `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `ScalarFactory` traits (generic Rust bounds; the bindings reach the factories
//! through a data type's `field()` / `scalar()` / `defaultScalar()`), and — for
//! the serie scalars (`Int8Serie` … `UInt64Serie`) — their per-element-null
//! construction, `to_arrow_array` / `nulls` Arrow-buffer surface and `fromIo` / `pwriteIo`
//! two-resource bridge (which borrow a second IO resource at once), so a serie
//! built from Node is a dense (all-valid) serie. The still-generic nested scalars
//! — the generic `Serie` / `MapScalar`, the plain `StructScalar` row value (its
//! accessor twin `RecordScalar` *is* bound), the struct-row series `StructSerie` /
//! `TypedStructSerie`, and the type-erased `AnySerie` / `AnyScalar` holders behind
//! them (whose fields cross as native JS values instead of as a class) — have no
//! concrete FFI shape yet. The lazy serie iterators (`iter_scalars` /
//! `iter_records`) are Rust-only too; iterate the materialized `toArray()` instead.

use napi::bindgen_prelude::{
    BigInt, Buffer, Error, FromNapiValue, Null, Object, Result, ToNapiValue,
};
use napi::{Env, JsUnknown};
use napi_derive::napi;
use yggdryl_scalar::arrow_array::{self, Array};
use yggdryl_scalar::half::f16;
use yggdryl_scalar::{arrow_schema, AnyScalar, AnySerie, Scalar};

use crate::{bigint_to_i64, bigint_to_u64, data_error, index_to_usize, wire_to_native, WireFloat};

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

    /// The scalar's native JS value: always `null` — the general native
    /// accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Null {
        Null
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
            /// The value as a number (widened to f64); throws when null or not
            /// exactly representable in f16.
            #[napi]
            pub fn as_f16(&self) -> Result<f64> {
                self.inner.as_f16().map(f16::to_f64).map_err(data_error)
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

    /// The scalar's native JS value: the bytes as a `Buffer`, or `null` when
    /// null — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<Buffer> {
        self.value()
    }
}

as_accessors_node!(BinaryScalar);

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[napi(namespace = "scalar")]
pub struct OptionalBinaryScalar {
    pub(crate) inner: yggdryl_scalar::TypedOptionalScalar<
        yggdryl_dtype::BinaryType,
        yggdryl_scalar::BinaryScalar,
    >,
}

#[napi(namespace = "scalar")]
impl OptionalBinaryScalar {
    /// A scalar holding the `binary` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::BinaryScalar::new(
                value.to_vec(),
            )),
        }
    }

    /// The null variant.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::null(),
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

    /// The scalar's native JS value: the bytes as a `Buffer`, or `null` for the
    /// null variant — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<Buffer> {
        self.value()
    }
}

as_accessors_node!(OptionalBinaryScalar);

/// A single, possibly-null `utf8` value, crossing the FFI boundary as a JS
/// `string`. The string counterpart of [`BinaryScalar`]: the value is held in the
/// Rust core as a positioned-IO `Utf8Buffer`, but that resource stays Rust-only —
/// the string crosses as text, its UTF-8 bytes reachable through `asBytes()`.
#[napi(namespace = "scalar")]
pub struct Utf8Scalar {
    pub(crate) inner: yggdryl_scalar::Utf8Scalar,
}

#[napi(namespace = "scalar")]
impl Utf8Scalar {
    /// A `utf8` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: String) -> Self {
        Self {
            inner: yggdryl_scalar::Utf8Scalar::new(value),
        }
    }

    /// A null `utf8` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Utf8Scalar::null(),
        }
    }

    /// Whether this scalar holds a null value.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's value as a `string`, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<String> {
        self.inner.value().map(str::to_string)
    }

    /// The scalar's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::Utf8Type {
        crate::dtype::Utf8Type::default()
    }

    /// The scalar's native JS value: the text as a `string`, or `null` when null —
    /// the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<String> {
        self.value()
    }
}

as_accessors_node!(Utf8Scalar);

/// A single value of the union between null and `utf8`: a value variant, or the
/// null variant.
#[napi(namespace = "scalar")]
pub struct OptionalUtf8Scalar {
    pub(crate) inner: yggdryl_scalar::TypedOptionalScalar<
        yggdryl_dtype::Utf8Type,
        yggdryl_scalar::Utf8Scalar,
    >,
}

#[napi(namespace = "scalar")]
impl OptionalUtf8Scalar {
    /// A scalar holding the `utf8` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: String) -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::Utf8Scalar::new(
                value,
            )),
        }
    }

    /// The null variant.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::null(),
        }
    }

    /// Whether this scalar holds the null variant.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The value as a `string`, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<String> {
        self.inner.value().map(str::to_string)
    }

    /// The inner scalar, when this holds the value variant.
    #[napi]
    pub fn scalar(&self) -> Option<Utf8Scalar> {
        self.inner.scalar().map(|scalar| Utf8Scalar {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::OptionalUtf8Type {
        crate::dtype::OptionalUtf8Type::default()
    }

    /// The scalar's native JS value: the text as a `string`, or `null` for the null
    /// variant — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<String> {
        self.value()
    }
}

as_accessors_node!(OptionalUtf8Scalar);

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
                yggdryl_scalar::TypedOptionalScalar<yggdryl_dtype::$dtype, yggdryl_scalar::$ty>,
        }

        #[napi(namespace = "scalar")]
        impl $opt_ty {
            /// The null variant.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::null(),
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

            /// The scalar's native JS value: the number, or `null` when null —
            /// the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<i64> {
                self.value()
            }
        }

        #[napi(namespace = "scalar")]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::$ty::new(
                        wire_to_native::<$native>(value, $name)?,
                    )),
                })
            }

            /// The value, or `null` for the null variant.
            #[napi]
            pub fn value(&self) -> Option<i64> {
                self.inner.value().copied().map(i64::from)
            }

            /// The scalar's native JS value: the number, or `null` for the null
            /// variant — the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<i64> {
                self.value()
            }
        }
    };
}

/// Generates the wire-dependent constructor and `value` of a float scalar (and its
/// optional) over JS `number` — the sole JS numeric wire (an `f64`), which narrows
/// to `f32` (rounding) and widens back exactly through [`WireFloat`]. A float
/// construction never fails (a `number` always narrows), so it is the float analog
/// of [`int_wire_number_scalar!`] without the range check.
macro_rules! float_wire_number_scalar {
    ($ty:ident, $opt_ty:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "scalar")]
        impl $ty {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[napi(constructor)]
            pub fn new(value: f64) -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::new(<$native>::from_wire(value)),
                }
            }

            /// The scalar's value as a number, or `null` when null.
            #[napi]
            pub fn value(&self) -> Option<f64> {
                self.inner.value().copied().map(<$native>::to_wire)
            }

            /// The scalar's native JS value: the number, or `null` when null —
            /// the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<f64> {
                self.value()
            }
        }

        #[napi(namespace = "scalar")]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[napi(constructor)]
            pub fn new(value: f64) -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::$ty::new(
                        <$native>::from_wire(value),
                    )),
                }
            }

            /// The value as a number, or `null` for the null variant.
            #[napi]
            pub fn value(&self) -> Option<f64> {
                self.inner.value().copied().map(<$native>::to_wire)
            }

            /// The scalar's native JS value: the number, or `null` for the null
            /// variant — the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<f64> {
                self.value()
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

// The floats reuse the width-independent scalar surface, then carry their value as
// a JS `number` (both narrow to `f64` on the wire — see `WireFloat`).
int_scalar_node!(
    Float32Scalar,
    OptionalFloat32Scalar,
    Float32Type,
    OptionalFloat32Type,
    "float32"
);
int_scalar_node!(
    Float64Scalar,
    OptionalFloat64Scalar,
    Float64Type,
    OptionalFloat64Type,
    "float64"
);

float_wire_number_scalar!(Float32Scalar, OptionalFloat32Scalar, f32, "float32");
float_wire_number_scalar!(Float64Scalar, OptionalFloat64Scalar, f64, "float64");

// `float16` reuses the same float surface: its native `half::f16` narrows to / widens
// from a JS `number` (an f64) through `WireFloat`, exactly as f32 / f64 do.
int_scalar_node!(
    Float16Scalar,
    OptionalFloat16Scalar,
    Float16Type,
    OptionalFloat16Type,
    "float16"
);

float_wire_number_scalar!(Float16Scalar, OptionalFloat16Scalar, f16, "float16");

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

    /// The scalar's native JS value: the `BigInt`, or `null` when null — the
    /// general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<BigInt> {
        self.value()
    }
}

#[napi(namespace = "scalar")]
impl OptionalInt64Scalar {
    /// A scalar holding the `int64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::Int64Scalar::new(
                bigint_to_i64(value)?,
            )),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }

    /// The scalar's native JS value: the `BigInt`, or `null` for the null
    /// variant — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<BigInt> {
        self.value()
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

    /// The scalar's native JS value: the `BigInt`, or `null` when null — the
    /// general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<BigInt> {
        self.value()
    }
}

#[napi(namespace = "scalar")]
impl OptionalUInt64Scalar {
    /// A scalar holding the `uint64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::UInt64Scalar::new(
                bigint_to_u64(value)?,
            )),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }

    /// The scalar's native JS value: the `BigInt`, or `null` for the null
    /// variant — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<BigInt> {
        self.value()
    }
}

/// Generates the width-independent surface of one integer serie scalar: `$ty`,
/// the buffer-backed `list` of `$name` — a thin delegation to
/// `yggdryl_scalar::$ty`. The width-dependent constructor, `values` and `getAt`
/// are generated by [`int_serie_wire_number_scalar!`] (8-32 bit, JS `number`
/// elements) or written per 64-bit type with `BigInt`.
macro_rules! int_serie_scalar_node {
    ($ty:ident, $scalar:ident, $dtype:ident, $name:literal) => {
        /// A single, possibly-null integer serie — *our array*, the buffer-backed
        /// serie scalar. Built dense (all-valid) from Node; the whole serie may
        /// still be null through the `null()` factory.
        #[doc = concat!("This is the serie of `", $name, "`.")]
        #[napi(namespace = "scalar")]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[napi(namespace = "scalar")]
        impl $ty {
            /// The null serie scalar.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::null(),
                }
            }

            /// Whether this scalar holds a null value (distinct from the empty serie).
            #[napi]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The number of elements, `0` when null or empty (`isNull` distinguishes
            /// the two).
            #[napi]
            pub fn len(&self) -> u32 {
                self.inner.len() as u32
            }

            /// Whether the sequence holds no elements (also `true` when null).
            #[napi]
            pub fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The element at `index` as the element scalar, or `null` when the
            /// serie is null or `index` is out of bounds (a negative index
            /// included).
            #[napi]
            pub fn get_scalar_at(&self, index: i64) -> Option<$scalar> {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| self.inner.get_scalar_at(index))
                    .map(|inner| $scalar { inner })
            }

            /// The scalar's data type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }
        }
    };
}

/// Generates the width-dependent constructor, `values` and `getAt` of an 8-32 bit
/// integer serie over JS `number` elements, each range-checked with an actionable
/// error.
macro_rules! int_serie_wire_number_scalar {
    ($ty:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "scalar")]
        impl $ty {
            /// A serie holding the native serie `values` (all-valid).
            #[napi(constructor)]
            pub fn new(values: Vec<i64>) -> Result<Self> {
                let values = values
                    .into_iter()
                    .map(|value| wire_to_native::<$native>(value, $name))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Self {
                    inner: yggdryl_scalar::$ty::from(values),
                })
            }

            /// The whole element buffer copied out as a JS array of numbers, or
            /// `null` when null — the JS-idiomatic name for a native-container
            /// copy-out (the zero-copy borrow stays Rust-only).
            #[napi]
            pub fn to_array(&self) -> Option<Vec<i64>> {
                self.inner
                    .values()
                    .map(|values| values.iter().copied().map(i64::from).collect())
            }

            /// The element at `index` read as its native number; throws when null,
            /// the index is negative, or the index is past the end.
            #[napi]
            pub fn get_at(&self, index: i64) -> Result<i64> {
                self.inner
                    .get_at::<$native>(index_to_usize(index)?)
                    .map(i64::from)
                    .map_err(data_error)
            }

            /// The scalar's native JS value: the `toArray()` array of numbers, or
            /// `null` when null — the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<Vec<i64>> {
                self.to_array()
            }
        }
    };
}

/// Generates the wire-dependent constructor, `toArray` and `getAt` of a float serie
/// over JS `number` elements (each an `f64` narrowed to the element width through
/// [`WireFloat`]) — the float analog of [`int_serie_wire_number_scalar!`] with no
/// per-element range check.
macro_rules! float_serie_wire_number_scalar {
    ($ty:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "scalar")]
        impl $ty {
            /// A serie holding the native serie `values` (all-valid).
            #[napi(constructor)]
            pub fn new(values: Vec<f64>) -> Self {
                let values = values
                    .into_iter()
                    .map(<$native>::from_wire)
                    .collect::<Vec<_>>();
                Self {
                    inner: yggdryl_scalar::$ty::from(values),
                }
            }

            /// The whole element buffer copied out as a JS array of numbers, or
            /// `null` when null — the JS-idiomatic name for a native-container
            /// copy-out (the zero-copy borrow stays Rust-only).
            #[napi]
            pub fn to_array(&self) -> Option<Vec<f64>> {
                self.inner
                    .values()
                    .map(|values| values.iter().copied().map(<$native>::to_wire).collect())
            }

            /// The element at `index` read as its native number; throws when null,
            /// the index is negative, or the index is past the end.
            #[napi]
            pub fn get_at(&self, index: i64) -> Result<f64> {
                self.inner
                    .get_at::<$native>(index_to_usize(index)?)
                    .map(<$native>::to_wire)
                    .map_err(data_error)
            }

            /// The scalar's native JS value: the `toArray()` array of numbers, or
            /// `null` when null — the general native accessor, one FFI call.
            #[napi]
            pub fn to_js_value(&self) -> Option<Vec<f64>> {
                self.to_array()
            }
        }
    };
}

int_serie_scalar_node!(Int8Serie, Int8Scalar, Int8SerieType, "int8");
int_serie_scalar_node!(Int16Serie, Int16Scalar, Int16SerieType, "int16");
int_serie_scalar_node!(Int32Serie, Int32Scalar, Int32SerieType, "int32");
int_serie_scalar_node!(Int64Serie, Int64Scalar, Int64SerieType, "int64");
int_serie_scalar_node!(UInt8Serie, UInt8Scalar, UInt8SerieType, "uint8");
int_serie_scalar_node!(UInt16Serie, UInt16Scalar, UInt16SerieType, "uint16");
int_serie_scalar_node!(UInt32Serie, UInt32Scalar, UInt32SerieType, "uint32");
int_serie_scalar_node!(UInt64Serie, UInt64Scalar, UInt64SerieType, "uint64");

int_serie_wire_number_scalar!(Int8Serie, i8, "int8");
int_serie_wire_number_scalar!(Int16Serie, i16, "int16");
int_serie_wire_number_scalar!(Int32Serie, i32, "int32");
int_serie_wire_number_scalar!(UInt8Serie, u8, "uint8");
int_serie_wire_number_scalar!(UInt16Serie, u16, "uint16");
int_serie_wire_number_scalar!(UInt32Serie, u32, "uint32");

// The float series reuse the width-independent serie surface, then carry their
// elements as JS `number` (both narrow to `f64` on the wire — see `WireFloat`).
int_serie_scalar_node!(Float32Serie, Float32Scalar, Float32SerieType, "float32");
int_serie_scalar_node!(Float64Serie, Float64Scalar, Float64SerieType, "float64");

float_serie_wire_number_scalar!(Float32Serie, f32, "float32");
float_serie_wire_number_scalar!(Float64Serie, f64, "float64");

// The `float16` serie carries its elements as JS `number` (an f64 on the wire, see
// `WireFloat`), like the f32 / f64 series.
int_serie_scalar_node!(Float16Serie, Float16Scalar, Float16SerieType, "float16");
float_serie_wire_number_scalar!(Float16Serie, f16, "float16");

// The 64-bit series carry their elements as JS `BigInt` (a `number` cannot
// represent the full range), so their width-dependent surface is written out per
// type.

#[napi(namespace = "scalar")]
impl Int64Serie {
    /// A serie holding the native serie `values` (all-valid).
    #[napi(constructor)]
    pub fn new(values: Vec<BigInt>) -> Result<Self> {
        let values = values
            .into_iter()
            .map(bigint_to_i64)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            inner: yggdryl_scalar::Int64Serie::from(values),
        })
    }

    /// The whole element buffer copied out as a JS array of `BigInt`, or `null`
    /// when null — the JS-idiomatic name for a native-container copy-out (the
    /// zero-copy borrow stays Rust-only).
    #[napi]
    pub fn to_array(&self) -> Option<Vec<BigInt>> {
        self.inner
            .values()
            .map(|values| values.iter().copied().map(BigInt::from).collect())
    }

    /// The element at `index` read as its native `BigInt`; throws when null,
    /// the index is negative, or the index is past the end.
    #[napi]
    pub fn get_at(&self, index: i64) -> Result<BigInt> {
        self.inner
            .get_at::<i64>(index_to_usize(index)?)
            .map(BigInt::from)
            .map_err(data_error)
    }

    /// The scalar's native JS value: the `toArray()` array of `BigInt`, or
    /// `null` when null — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<Vec<BigInt>> {
        self.to_array()
    }
}

#[napi(namespace = "scalar")]
impl UInt64Serie {
    /// A serie holding the native serie `values` (all-valid).
    #[napi(constructor)]
    pub fn new(values: Vec<BigInt>) -> Result<Self> {
        let values = values
            .into_iter()
            .map(bigint_to_u64)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            inner: yggdryl_scalar::UInt64Serie::from(values),
        })
    }

    /// The whole element buffer copied out as a JS array of `BigInt`, or `null`
    /// when null — the JS-idiomatic name for a native-container copy-out (the
    /// zero-copy borrow stays Rust-only).
    #[napi]
    pub fn to_array(&self) -> Option<Vec<BigInt>> {
        self.inner
            .values()
            .map(|values| values.iter().copied().map(BigInt::from).collect())
    }

    /// The element at `index` read as its native `BigInt`; throws when null,
    /// the index is negative, or the index is past the end.
    #[napi]
    pub fn get_at(&self, index: i64) -> Result<BigInt> {
        self.inner
            .get_at::<u64>(index_to_usize(index)?)
            .map(BigInt::from)
            .map_err(data_error)
    }

    /// The scalar's native JS value: the `toArray()` array of `BigInt`, or
    /// `null` when null — the general native accessor, one FFI call.
    #[napi]
    pub fn to_js_value(&self) -> Option<Vec<BigInt>> {
        self.to_array()
    }
}

// The record scalar: the `struct` row. Its fields are core `AnyScalar` atoms (and
// their list fields core `AnySerie` columns); they cross the FFI boundary as native
// JS values, converted once in Rust by the helpers below.

/// A napi-typed value erased to a `JsUnknown`, so heterogeneous record members
/// share one return surface.
fn to_unknown<T: ToNapiValue>(env: &Env, value: T) -> Result<JsUnknown> {
    let raw = unsafe { T::to_napi_value(env.raw(), value)? };
    unsafe { JsUnknown::from_napi_value(env.raw(), raw) }
}

/// A whole integer column as the JS array its serie class' `toArray()` returns
/// (`null` for the null serie) — the serie classes carry the element conversion,
/// so it lives in one place.
fn serie_to_js(env: &Env, column: &AnySerie) -> Result<JsUnknown> {
    match column {
        AnySerie::Int8(serie) => to_unknown(
            env,
            Int8Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Int16(serie) => to_unknown(
            env,
            Int16Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Int32(serie) => to_unknown(
            env,
            Int32Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Int64(serie) => to_unknown(
            env,
            Int64Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::UInt8(serie) => to_unknown(
            env,
            UInt8Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::UInt16(serie) => to_unknown(
            env,
            UInt16Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::UInt32(serie) => to_unknown(
            env,
            UInt32Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::UInt64(serie) => to_unknown(
            env,
            UInt64Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Float16(serie) => to_unknown(
            env,
            Float16Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Float32(serie) => to_unknown(
            env,
            Float32Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        AnySerie::Float64(serie) => to_unknown(
            env,
            Float64Serie {
                inner: serie.clone(),
            }
            .to_array(),
        ),
        other => Err(Error::from_reason(format!(
            "no JS array for a serie of Arrow type {}; expected an integer or float serie",
            other.data_type()
        ))),
    }
}

/// The single element of a one-element record column as its JS value — the wire
/// types the scalar classes already use (`number` for the 8–32 bit integers and the
/// floats, `BigInt` for the 64-bit integers, a `Buffer` for binary, the `toArray()`
/// array for a serie, `null` for null), read through the classes' own `value()` /
/// `toArray()` so the conversion lives in one place.
fn scalar_to_js(env: &Env, scalar: &AnyScalar) -> Result<JsUnknown> {
    // The decomposed integer field reuses its wrapper class' `value()` conversion.
    match scalar {
        AnyScalar::Int8(inner) => to_unknown(env, Int8Scalar { inner: *inner }.value()),
        AnyScalar::Int16(inner) => to_unknown(env, Int16Scalar { inner: *inner }.value()),
        AnyScalar::Int32(inner) => to_unknown(env, Int32Scalar { inner: *inner }.value()),
        AnyScalar::Int64(inner) => to_unknown(env, Int64Scalar { inner: *inner }.value()),
        AnyScalar::UInt8(inner) => to_unknown(env, UInt8Scalar { inner: *inner }.value()),
        AnyScalar::UInt16(inner) => to_unknown(env, UInt16Scalar { inner: *inner }.value()),
        AnyScalar::UInt32(inner) => to_unknown(env, UInt32Scalar { inner: *inner }.value()),
        AnyScalar::UInt64(inner) => to_unknown(env, UInt64Scalar { inner: *inner }.value()),
        AnyScalar::Float16(inner) => to_unknown(env, Float16Scalar { inner: *inner }.value()),
        AnyScalar::Float32(inner) => to_unknown(env, Float32Scalar { inner: *inner }.value()),
        AnyScalar::Float64(inner) => to_unknown(env, Float64Scalar { inner: *inner }.value()),
        AnyScalar::Arrow(value) => match value.data_type() {
            arrow_schema::DataType::Null => to_unknown(env, Null),
            arrow_schema::DataType::Utf8 => {
                let array = value
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .expect("a utf8 field downcasts to its array");
                to_unknown(
                    env,
                    (!Array::is_null(array, 0)).then(|| array.value(0).to_string()),
                )
            }
            arrow_schema::DataType::Binary => {
                let array = value
                    .as_any()
                    .downcast_ref::<arrow_array::BinaryArray>()
                    .expect("a binary field downcasts to its array");
                to_unknown(
                    env,
                    (!Array::is_null(array, 0)).then(|| Buffer::from(array.value(0).to_vec())),
                )
            }
            arrow_schema::DataType::List(_) => {
                let array = value
                    .as_any()
                    .downcast_ref::<arrow_array::ListArray>()
                    .expect("a list field downcasts to its array");
                if Array::is_null(array, 0) {
                    to_unknown(env, Null)
                } else {
                    serie_to_js(env, &AnySerie::from_arrow(array.value(0)))
                }
            }
            other => Err(Error::from_reason(format!(
                "no JS value for a record field of Arrow type {other}; expected null, an integer, a float, binary, a string or a numeric list"
            ))),
        },
        // `AnyScalar` is non-exhaustive; a future decomposed variant has no wire type yet.
        other => Err(Error::from_reason(format!(
            "no JS value for a record field of Arrow type {}; expected null, an integer, a float, binary, a string or a numeric list",
            other.data_type()
        ))),
    }
}

/// A single, possibly-null `struct` row — *the record*, built from a plain JS
/// object whose members are inferred like the factory's (a whole `number` /
/// `bigint` → `int64`, a fractional `number` → `float64`, a `Buffer` → `binary`,
/// `null` → `null`, a numeric array → the `int64` or `float64` serie), each held as
/// a one-element child column of the shared `yggdryl.dtype.StructType`.
#[napi(namespace = "scalar")]
pub struct RecordScalar {
    pub(crate) inner: yggdryl_scalar::RecordScalar,
}

#[napi(namespace = "scalar")]
impl RecordScalar {
    /// A record whose fields are inferred from `fields`' members, each value the
    /// field's value.
    #[napi(constructor)]
    pub fn new(fields: Object) -> Result<Self> {
        Ok(Self {
            inner: crate::factory::record_from_object(&fields)?,
        })
    }

    /// The null record of the struct type `dataType`.
    #[napi(factory)]
    pub fn null(data_type: &crate::dtype::StructType) -> Self {
        Self {
            inner: yggdryl_scalar::RecordScalar::null(data_type.inner.clone()),
        }
    }

    /// Whether this record is the null row.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::StructType {
        crate::dtype::StructType {
            inner: self.inner.data_type().clone(),
        }
    }

    /// The child field names, in order.
    #[napi]
    pub fn field_names(&self) -> Vec<String> {
        self.data_type().field_names()
    }

    /// The field `name`'s value as its native JS value (the child class' wire
    /// type), or `null` when the record is null or no field carries the name —
    /// one FFI call.
    #[napi]
    pub fn get(&self, env: Env, name: String) -> Result<JsUnknown> {
        match self.inner.scalar_by(&name) {
            Some(scalar) => scalar_to_js(&env, &scalar),
            None => to_unknown(&env, Null),
        }
    }

    /// The record's native JS value: a plain `{field: value}` object, or `null`
    /// for the null row — the general native accessor, one FFI call for the
    /// whole row.
    #[napi]
    pub fn to_js_value(&self, env: Env) -> Result<Option<Object>> {
        let Some(scalars) = self.inner.value() else {
            return Ok(None);
        };
        let mut object = env.create_object()?;
        for (name, scalar) in self.field_names().into_iter().zip(scalars) {
            object.set(name, scalar_to_js(&env, scalar)?)?;
        }
        Ok(Some(object))
    }
}
