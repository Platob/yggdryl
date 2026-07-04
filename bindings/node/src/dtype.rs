//! The `yggdryl.dtype` namespace — thin wrappers over the `yggdryl-dtype` crate.
//!
//! Every integer type is exposed as its data type and its logical optional data
//! type (`yggdryl.dtype.Int64Type`, `yggdryl.dtype.OptionalInt64Type`, …),
//! alongside `BinaryType` / `OptionalBinaryType`, `NullType`, `UnionType`,
//! `StructType` (its child fields inferred from a plain JS object of example
//! values, member by member through the factory's inference) and its
//! concrete serie type (e.g. `Int64SerieType`, the `list` of `int64` — every
//! integer value type has a buffer-backed serie scalar) — the same globally-unique
//! names as the Rust crate, the namespace carrying the concern (napi registers class constructors by
//! JS class name in one addon-global registry, and the `…Type` / `…Field` /
//! `…Scalar` suffixes keep the three concerns' classes distinct). Values adapt to
//! JS idioms: the 8–32 bit integers and the floats (and their series' elements)
//! carry their codec values as `number`, the 64-bit integers as `BigInt`. Data
//! types expose the
//! descriptor surface (`name`, `arrowFormat`, widths), the native byte codec,
//! and — where they are typed factories (the integers, `BinaryType`, the serie
//! types and the optionals) — their field / scalar / default builders (`field`
//! hands back a `yggdryl.field` class, `scalar` and `defaultScalar` a
//! `yggdryl.scalar` class).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work), construction of a
//! `UnionType` from arbitrary child fields (its `UnionFields` is an arrow-schema
//! value — `UnionType` is reached through an optional data type's `storage()`),
//! construction of a `StructType` from raw arrow-schema fields likewise (the
//! binding infers its fields from example values instead), the
//! `DataTypeId` classifier (a method-bearing enum the bindings cannot model
//! uniformly), and the dynamic base nested types and their typed generics
//! (`SerieType` / `TypedSerieType` over a non-integer value type, `MapType` /
//! `TypedMapType`, and the per-family trait pairs), which have no
//! concrete FFI shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Object, Result};
use napi_derive::napi;
use yggdryl_dtype::{DataType, Logical, Nested, Struct, TypedDataType, Union};
use yggdryl_field::FieldFactory;
use yggdryl_scalar::ScalarFactory;

use crate::{bigint_to_i64, bigint_to_u64, data_error, wire_to_native, WireFloat};

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[napi(namespace = "dtype")]
pub struct UnionType {
    pub(crate) inner: yggdryl_dtype::UnionType,
}

#[napi(namespace = "dtype")]
impl UnionType {
    /// The type's lowercase name, `"union"`.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, e.g. `"+us:0,1"`.
    #[napi]
    pub fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A union has no fixed byte width.
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width().map(|width| width as u32)
    }

    /// A union has no fixed bit width.
    #[napi]
    pub fn bit_width(&self) -> Option<u32> {
        self.inner.bit_width().map(|width| width as u32)
    }

    /// The number of child fields.
    #[napi]
    pub fn child_count(&self) -> u32 {
        self.inner.child_count() as u32
    }

    /// The union's mode: `"sparse"` or `"dense"`.
    #[napi]
    pub fn mode(&self) -> &'static str {
        match self.inner.mode() {
            yggdryl_dtype::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_dtype::arrow_schema::UnionMode::Dense => "dense",
        }
    }
}

/// The Apache Arrow `struct` data type: an ordered set of named child fields,
/// inferred from a plain JS object of example values — each member through the
/// factory's inference (a whole `number` / `bigint` → `int64`, a fractional
/// `number` → `float64`, a `Buffer` → `binary`, `null` → `null`, a numeric array →
/// the `int64` or `float64` serie). Construction from raw Arrow fields stays
/// Rust-only.
#[napi(namespace = "dtype")]
pub struct StructType {
    pub(crate) inner: yggdryl_dtype::StructType,
}

#[napi(namespace = "dtype")]
impl StructType {
    /// A struct whose child fields are inferred from `fields`' members, each
    /// value an example of its field's type (nullable, like every factory-built
    /// field).
    #[napi(constructor)]
    pub fn new(fields: Object) -> Result<Self> {
        Ok(Self {
            inner: crate::factory::struct_type_from_object(&fields)?,
        })
    }

    /// The type's lowercase name, `"struct"`.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"+s"`.
    #[napi]
    pub fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A struct has no fixed byte width.
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width().map(|width| width as u32)
    }

    /// A struct has no fixed bit width.
    #[napi]
    pub fn bit_width(&self) -> Option<u32> {
        self.inner.bit_width().map(|width| width as u32)
    }

    /// The number of child fields.
    #[napi]
    pub fn child_count(&self) -> u32 {
        self.inner.child_count() as u32
    }

    /// The child field names, in order.
    #[napi]
    pub fn field_names(&self) -> Vec<String> {
        self.inner
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect()
    }
}

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[napi(namespace = "dtype")]
#[derive(Default)]
pub struct NullType {
    pub(crate) inner: yggdryl_dtype::NullType,
}

#[napi(namespace = "dtype")]
impl NullType {
    /// The null data type.
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"null"`.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"n"`.
    #[napi]
    pub fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// The null type has no storage, so no byte width.
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width().map(|width| width as u32)
    }

    /// The null type has no storage, so no bit width.
    #[napi]
    pub fn bit_width(&self) -> Option<u32> {
        self.inner.bit_width().map(|width| width as u32)
    }
}

/// The Apache Arrow `binary` data type: a variable-length byte sequence.
#[napi(namespace = "dtype")]
#[derive(Default)]
pub struct BinaryType {
    pub(crate) inner: yggdryl_dtype::BinaryType,
}

#[napi(namespace = "dtype")]
impl BinaryType {
    /// The `binary` data type.
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"binary"`.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"z"`.
    #[napi]
    pub fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A binary value has no fixed byte width.
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width().map(|width| width as u32)
    }

    /// A binary value has no fixed bit width.
    #[napi]
    pub fn bit_width(&self) -> Option<u32> {
        self.inner.bit_width().map(|width| width as u32)
    }

    /// Serialize a native value into its Arrow bytes — the identity for binary.
    #[napi]
    pub fn native_to_bytes(&self, value: Buffer) -> Buffer {
        Buffer::from(self.inner.native_to_bytes(&value.to_vec()))
    }

    /// Deserialize Arrow bytes into a native value — the identity for binary
    /// (any length is valid).
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Buffer> {
        self.inner
            .native_from_bytes(&bytes)
            .map(Buffer::from)
            .map_err(data_error)
    }

    /// The type's default native value, an empty `Buffer`.
    #[napi]
    pub fn default_value(&self) -> Buffer {
        Buffer::from(TypedDataType::default_value(&self.inner))
    }

    /// The field of this type named `name` (nullable by default).
    #[napi]
    pub fn field(&self, name: String, nullable: Option<bool>) -> crate::field::BinaryField {
        crate::field::BinaryField {
            inner: self.inner.field(name, nullable.unwrap_or(true)),
        }
    }

    /// A `yggdryl.scalar.BinaryScalar` holding `value`.
    #[napi]
    pub fn scalar(&self, value: Buffer) -> crate::scalar::BinaryScalar {
        crate::scalar::BinaryScalar {
            inner: self.inner.scalar(value.to_vec()),
        }
    }

    /// The default scalar: a `yggdryl.scalar.BinaryScalar` holding empty bytes.
    #[napi]
    pub fn default_scalar(&self) -> crate::scalar::BinaryScalar {
        crate::scalar::BinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The logical optional of this type (stored as the null-or-value union).
    #[napi]
    pub fn optional(&self) -> OptionalBinaryType {
        OptionalBinaryType::default()
    }
}

/// The logical optional of `binary`: a value, or null — stored as the
/// null-or-`binary` union.
#[napi(namespace = "dtype")]
#[derive(Default)]
pub struct OptionalBinaryType {
    pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::BinaryType>,
}

#[napi(namespace = "dtype")]
impl OptionalBinaryType {
    /// The optional `binary` data type.
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"optional"`.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string of the union storage.
    #[napi]
    pub fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// An optional has no fixed byte width (union storage).
    #[napi]
    pub fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width().map(|width| width as u32)
    }

    /// An optional has no fixed bit width (union storage).
    #[napi]
    pub fn bit_width(&self) -> Option<u32> {
        self.inner.bit_width().map(|width| width as u32)
    }

    /// The value type this optional wraps.
    #[napi]
    pub fn value_type(&self) -> BinaryType {
        BinaryType::default()
    }

    /// The physical storage: the sparse null-or-value union.
    #[napi]
    pub fn storage(&self) -> UnionType {
        UnionType {
            inner: self.inner.storage().clone(),
        }
    }

    /// The default native value: the value type's default, an empty `Buffer`.
    #[napi]
    pub fn default_value(&self) -> Buffer {
        Buffer::from(TypedDataType::default_value(&self.inner))
    }

    /// The field of this type named `name` (nullable by default).
    #[napi]
    pub fn field(&self, name: String, nullable: Option<bool>) -> crate::field::OptionalBinaryField {
        crate::field::OptionalBinaryField {
            inner: self.inner.field(name, nullable.unwrap_or(true)),
        }
    }

    /// A `yggdryl.scalar.OptionalBinaryScalar` holding the value variant `value`.
    #[napi]
    pub fn scalar(&self, value: Buffer) -> crate::scalar::OptionalBinaryScalar {
        crate::scalar::OptionalBinaryScalar {
            inner: self.inner.scalar(value.to_vec()),
        }
    }

    /// The default scalar: the null variant (the scalar models nullness).
    #[napi]
    pub fn default_scalar(&self) -> crate::scalar::OptionalBinaryScalar {
        crate::scalar::OptionalBinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// Serialize a native value into its Arrow bytes — the value type's codec.
    #[napi]
    pub fn native_to_bytes(&self, value: Buffer) -> Buffer {
        Buffer::from(self.inner.native_to_bytes(&value.to_vec()))
    }

    /// Deserialize Arrow bytes into a native value — the exact inverse of
    /// `nativeToBytes`.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Buffer> {
        self.inner
            .native_from_bytes(&bytes)
            .map(Buffer::from)
            .map_err(data_error)
    }
}

/// Generates the width-independent surface of one integer type: the data type
/// `$ty` (descriptor, defaults, `field()` and `optional()`) and the logical
/// optional data type `$opt_ty` (over union storage). The width-dependent codec
/// and `scalar()` are generated by [`int_wire_number_dtype!`] (8–32 bit, JS
/// `number`) or written per 64-bit type with `BigInt`.
macro_rules! int_dtype_node {
    ($ty:ident, $opt_ty:ident, $field:ident, $opt_field:ident, $scalar:ident, $opt_scalar:ident, $native:ident, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type.")]
        #[napi(namespace = "dtype")]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::$native,
        }

        #[napi(namespace = "dtype")]
        impl $ty {
            #[doc = concat!("The `", $name, "` data type.")]
            #[napi(constructor)]
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self::default()
            }

            #[doc = concat!("The type's lowercase name, `\"", $name, "\"`.")]
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string.
            #[napi]
            pub fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// The fixed size of one value, in bytes.
            #[napi]
            pub fn byte_width(&self) -> Option<u32> {
                self.inner.byte_width().map(|width| width as u32)
            }

            /// The fixed size of one value, in bits.
            #[napi]
            pub fn bit_width(&self) -> Option<u32> {
                self.inner.bit_width().map(|width| width as u32)
            }

            /// The field of this type named `name` (nullable by default).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable.unwrap_or(true)),
                }
            }

            /// The default scalar: a `yggdryl.scalar` class holding `0`.
            #[napi]
            pub fn default_scalar(&self) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The logical optional of this type (stored as the null-or-value
            /// union).
            #[napi]
            pub fn optional(&self) -> $opt_ty {
                $opt_ty::default()
            }
        }

        #[doc = concat!("The logical optional of `", $name, "`: a value, or null — stored as the null-or-`", $name, "` union.")]
        #[napi(namespace = "dtype")]
        #[derive(Default)]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::$native>,
        }

        #[napi(namespace = "dtype")]
        impl $opt_ty {
            #[doc = concat!("The optional `", $name, "` data type.")]
            #[napi(constructor)]
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self::default()
            }

            /// The type's lowercase name, `"optional"`.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string of the union storage.
            #[napi]
            pub fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// An optional has no fixed byte width (union storage).
            #[napi]
            pub fn byte_width(&self) -> Option<u32> {
                self.inner.byte_width().map(|width| width as u32)
            }

            /// An optional has no fixed bit width (union storage).
            #[napi]
            pub fn bit_width(&self) -> Option<u32> {
                self.inner.bit_width().map(|width| width as u32)
            }

            /// The value type this optional wraps.
            #[napi]
            pub fn value_type(&self) -> $ty {
                $ty::default()
            }

            /// The field of this type named `name` (nullable by default).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> crate::field::$opt_field {
                crate::field::$opt_field {
                    inner: self.inner.field(name, nullable.unwrap_or(true)),
                }
            }

            /// The default scalar: the null variant (the scalar models nullness).
            #[napi]
            pub fn default_scalar(&self) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The physical storage: the sparse null-or-value union.
            #[napi]
            pub fn storage(&self) -> UnionType {
                UnionType {
                    inner: self.inner.storage().clone(),
                }
            }
        }
    };
}

/// Generates the width-dependent codec of an 8–32 bit integer type over JS
/// `number`: `nativeToBytes` / `nativeFromBytes`, `defaultValue` and `scalar()`
/// on the data type and its optional, range-checked with an actionable error.
macro_rules! int_wire_number_dtype {
    ($ty:ident, $opt_ty:ident, $scalar:ident, $opt_scalar:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "dtype")]
        impl $ty {
            /// Serialize a native value into its little-endian Arrow bytes.
            #[napi]
            pub fn native_to_bytes(&self, value: i64) -> Result<Buffer> {
                let value = wire_to_native::<$native>(value, $name)?;
                Ok(Buffer::from(self.inner.native_to_bytes(&value)))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `nativeToBytes`; the wrong length throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<i64> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(i64::from)
                    .map_err(data_error)
            }
        }

        #[napi(namespace = "dtype")]
        impl $ty {
            /// The type's default native value, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(TypedDataType::default_value(&self.inner))
            }

            /// A `yggdryl.scalar` class holding `value`.
            #[napi]
            pub fn scalar(&self, value: i64) -> Result<crate::scalar::$scalar> {
                let value = wire_to_native::<$native>(value, $name)?;
                Ok(crate::scalar::$scalar {
                    inner: self.inner.scalar(value),
                })
            }
        }

        #[napi(namespace = "dtype")]
        impl $opt_ty {
            /// The default native value of the value type, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(TypedDataType::default_value(&self.inner))
            }

            /// A `yggdryl.scalar` class holding the value variant `value`.
            #[napi]
            pub fn scalar(&self, value: i64) -> Result<crate::scalar::$opt_scalar> {
                let value = wire_to_native::<$native>(value, $name)?;
                Ok(crate::scalar::$opt_scalar {
                    inner: self.inner.scalar(value),
                })
            }

            /// Serialize a native value into its little-endian Arrow bytes — the
            /// value type's codec.
            #[napi]
            pub fn native_to_bytes(&self, value: i64) -> Result<Buffer> {
                let value = wire_to_native::<$native>(value, $name)?;
                Ok(Buffer::from(self.inner.native_to_bytes(&value)))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `nativeToBytes`; the wrong length throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<i64> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(i64::from)
                    .map_err(data_error)
            }
        }
    };
}

/// Generates the wire-dependent codec of a float type over JS `number` (the sole JS
/// numeric wire, an `f64` narrowed to the native width through [`WireFloat`]):
/// `nativeToBytes` / `nativeFromBytes`, `defaultValue` and `scalar()` on the data
/// type and its optional. The float analog of [`int_wire_number_dtype!`] — a
/// `number` always narrows, so nothing range-checks.
macro_rules! float_wire_number_dtype {
    ($ty:ident, $opt_ty:ident, $scalar:ident, $opt_scalar:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "dtype")]
        impl $ty {
            /// Serialize a native value into its little-endian Arrow bytes.
            #[napi]
            pub fn native_to_bytes(&self, value: f64) -> Buffer {
                Buffer::from(self.inner.native_to_bytes(&<$native>::from_wire(value)))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `nativeToBytes`; the wrong length throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<f64> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(<$native>::to_wire)
                    .map_err(data_error)
            }
        }

        #[napi(namespace = "dtype")]
        impl $ty {
            /// The type's default native value, `0`.
            #[napi]
            pub fn default_value(&self) -> f64 {
                <$native>::to_wire(TypedDataType::default_value(&self.inner))
            }

            /// A `yggdryl.scalar` class holding `value`.
            #[napi]
            pub fn scalar(&self, value: f64) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.scalar(<$native>::from_wire(value)),
                }
            }
        }

        #[napi(namespace = "dtype")]
        impl $opt_ty {
            /// The default native value of the value type, `0`.
            #[napi]
            pub fn default_value(&self) -> f64 {
                <$native>::to_wire(TypedDataType::default_value(&self.inner))
            }

            /// A `yggdryl.scalar` class holding the value variant `value`.
            #[napi]
            pub fn scalar(&self, value: f64) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.scalar(<$native>::from_wire(value)),
                }
            }

            /// Serialize a native value into its little-endian Arrow bytes — the
            /// value type's codec.
            #[napi]
            pub fn native_to_bytes(&self, value: f64) -> Buffer {
                Buffer::from(self.inner.native_to_bytes(&<$native>::from_wire(value)))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `nativeToBytes`; the wrong length throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<f64> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(<$native>::to_wire)
                    .map_err(data_error)
            }
        }
    };
}

int_dtype_node!(
    Int8Type,
    OptionalInt8Type,
    Int8Field,
    OptionalInt8Field,
    Int8Scalar,
    OptionalInt8Scalar,
    Int8Type,
    "int8"
);
int_dtype_node!(
    Int16Type,
    OptionalInt16Type,
    Int16Field,
    OptionalInt16Field,
    Int16Scalar,
    OptionalInt16Scalar,
    Int16Type,
    "int16"
);
int_dtype_node!(
    Int32Type,
    OptionalInt32Type,
    Int32Field,
    OptionalInt32Field,
    Int32Scalar,
    OptionalInt32Scalar,
    Int32Type,
    "int32"
);
int_dtype_node!(
    Int64Type,
    OptionalInt64Type,
    Int64Field,
    OptionalInt64Field,
    Int64Scalar,
    OptionalInt64Scalar,
    Int64Type,
    "int64"
);
int_dtype_node!(
    UInt8Type,
    OptionalUInt8Type,
    UInt8Field,
    OptionalUInt8Field,
    UInt8Scalar,
    OptionalUInt8Scalar,
    UInt8Type,
    "uint8"
);
int_dtype_node!(
    UInt16Type,
    OptionalUInt16Type,
    UInt16Field,
    OptionalUInt16Field,
    UInt16Scalar,
    OptionalUInt16Scalar,
    UInt16Type,
    "uint16"
);
int_dtype_node!(
    UInt32Type,
    OptionalUInt32Type,
    UInt32Field,
    OptionalUInt32Field,
    UInt32Scalar,
    OptionalUInt32Scalar,
    UInt32Type,
    "uint32"
);
int_dtype_node!(
    UInt64Type,
    OptionalUInt64Type,
    UInt64Field,
    OptionalUInt64Field,
    UInt64Scalar,
    OptionalUInt64Scalar,
    UInt64Type,
    "uint64"
);

int_wire_number_dtype!(
    Int8Type,
    OptionalInt8Type,
    Int8Scalar,
    OptionalInt8Scalar,
    i8,
    "int8"
);
int_wire_number_dtype!(
    Int16Type,
    OptionalInt16Type,
    Int16Scalar,
    OptionalInt16Scalar,
    i16,
    "int16"
);
int_wire_number_dtype!(
    Int32Type,
    OptionalInt32Type,
    Int32Scalar,
    OptionalInt32Scalar,
    i32,
    "int32"
);
int_wire_number_dtype!(
    UInt8Type,
    OptionalUInt8Type,
    UInt8Scalar,
    OptionalUInt8Scalar,
    u8,
    "uint8"
);
int_wire_number_dtype!(
    UInt16Type,
    OptionalUInt16Type,
    UInt16Scalar,
    OptionalUInt16Scalar,
    u16,
    "uint16"
);
int_wire_number_dtype!(
    UInt32Type,
    OptionalUInt32Type,
    UInt32Scalar,
    OptionalUInt32Scalar,
    u32,
    "uint32"
);

// The floats reuse the width-independent data-type surface, then carry their codec
// value as a JS `number` (both narrow to `f64` on the wire — see `WireFloat`).
int_dtype_node!(
    Float32Type,
    OptionalFloat32Type,
    Float32Field,
    OptionalFloat32Field,
    Float32Scalar,
    OptionalFloat32Scalar,
    Float32Type,
    "float32"
);
int_dtype_node!(
    Float64Type,
    OptionalFloat64Type,
    Float64Field,
    OptionalFloat64Field,
    Float64Scalar,
    OptionalFloat64Scalar,
    Float64Type,
    "float64"
);

float_wire_number_dtype!(
    Float32Type,
    OptionalFloat32Type,
    Float32Scalar,
    OptionalFloat32Scalar,
    f32,
    "float32"
);
float_wire_number_dtype!(
    Float64Type,
    OptionalFloat64Type,
    Float64Scalar,
    OptionalFloat64Scalar,
    f64,
    "float64"
);

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent surface is written out per type.

#[napi(namespace = "dtype")]
impl Int64Type {
    /// Serialize a native value into its little-endian Arrow bytes.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(
            self.inner.native_to_bytes(&bigint_to_i64(value)?),
        ))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        self.inner
            .native_from_bytes(&bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }
}

#[napi(namespace = "dtype")]
impl Int64Type {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(TypedDataType::default_value(&self.inner))
    }

    /// A `yggdryl.scalar.Int64Scalar` holding `value`.
    #[napi]
    pub fn scalar(&self, value: BigInt) -> Result<crate::scalar::Int64Scalar> {
        Ok(crate::scalar::Int64Scalar {
            inner: self.inner.scalar(bigint_to_i64(value)?),
        })
    }
}

#[napi(namespace = "dtype")]
impl OptionalInt64Type {
    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(TypedDataType::<i64>::default_value(&self.inner))
    }

    /// A `yggdryl.scalar.OptionalInt64Scalar` holding the value variant `value`.
    #[napi]
    pub fn scalar(&self, value: BigInt) -> Result<crate::scalar::OptionalInt64Scalar> {
        Ok(crate::scalar::OptionalInt64Scalar {
            inner: self.inner.scalar(bigint_to_i64(value)?),
        })
    }

    /// Serialize a native value into its little-endian Arrow bytes — the value
    /// type's codec.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(TypedDataType::<i64>::native_to_bytes(
            &self.inner,
            &bigint_to_i64(value)?,
        )))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        TypedDataType::<i64>::native_from_bytes(&self.inner, &bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }
}

#[napi(namespace = "dtype")]
impl UInt64Type {
    /// Serialize a native value into its little-endian Arrow bytes.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(
            self.inner.native_to_bytes(&bigint_to_u64(value)?),
        ))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        self.inner
            .native_from_bytes(&bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }
}

#[napi(namespace = "dtype")]
impl UInt64Type {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(TypedDataType::default_value(&self.inner))
    }

    /// A `yggdryl.scalar.UInt64Scalar` holding `value`.
    #[napi]
    pub fn scalar(&self, value: BigInt) -> Result<crate::scalar::UInt64Scalar> {
        Ok(crate::scalar::UInt64Scalar {
            inner: self.inner.scalar(bigint_to_u64(value)?),
        })
    }
}

#[napi(namespace = "dtype")]
impl OptionalUInt64Type {
    /// A `yggdryl.scalar.OptionalUInt64Scalar` holding the value variant `value`.
    #[napi]
    pub fn scalar(&self, value: BigInt) -> Result<crate::scalar::OptionalUInt64Scalar> {
        Ok(crate::scalar::OptionalUInt64Scalar {
            inner: self.inner.scalar(bigint_to_u64(value)?),
        })
    }

    /// Serialize a native value into its little-endian Arrow bytes — the value
    /// type's codec.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(TypedDataType::<u64>::native_to_bytes(
            &self.inner,
            &bigint_to_u64(value)?,
        )))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        TypedDataType::<u64>::native_from_bytes(&self.inner, &bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }

    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(TypedDataType::<u64>::default_value(&self.inner))
    }
}

/// Generates the width-independent surface of one integer serie data type: `$ty`,
/// the Apache Arrow `list` of `$name` (single nullable `"item"` child) — a thin
/// delegation to `yggdryl_dtype::TypedSerieType<$value_ty>`. The width-dependent codec,
/// `defaultValue` and `scalar()` are generated by [`int_serie_wire_number_dtype!`]
/// (8-32 bit, JS `number` elements) or written per 64-bit type with `BigInt`.
macro_rules! int_serie_dtype_node {
    ($ty:ident, $value_ty:ident, $field:ident, $serie:ident, $name:literal) => {
        /// The Apache Arrow `list` of one integer value type: a variable-length
        /// sequence (single nullable `"item"` child) with a buffer-backed serie
        /// scalar in `yggdryl.scalar`.
        #[doc = concat!("This is the serie of `", $name, "` — its scalar is `", stringify!($serie), "`.")]
        #[napi(namespace = "dtype")]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::TypedSerieType<yggdryl_dtype::$value_ty>,
        }

        #[napi(namespace = "dtype")]
        impl $ty {
            /// The serie data type.
            #[doc = concat!("The `list` of `", $name, "`.")]
            #[napi(constructor)]
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self::default()
            }

            /// The type's lowercase name, `"list"`.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string, `"+l"`.
            #[napi]
            pub fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// A serie has no fixed byte width.
            #[napi]
            pub fn byte_width(&self) -> Option<u32> {
                self.inner.byte_width().map(|width| width as u32)
            }

            /// A serie has no fixed bit width.
            #[napi]
            pub fn bit_width(&self) -> Option<u32> {
                self.inner.bit_width().map(|width| width as u32)
            }

            /// The number of child fields, `1` (the `"item"` field).
            #[napi]
            pub fn child_count(&self) -> u32 {
                self.inner.child_count() as u32
            }

            /// The value type this serie sequences.
            #[napi]
            pub fn value_type(&self) -> $value_ty {
                $value_ty::default()
            }

            /// The default scalar: a `yggdryl.scalar` serie holding the empty serie.
            #[napi]
            pub fn default_scalar(&self) -> crate::scalar::$serie {
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::default(),
                }
            }

            /// The field of this type named `name` (nullable by default).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable.unwrap_or(true)),
                }
            }
        }
    };
}

/// Generates the width-dependent codec of an 8-32 bit integer serie over JS
/// `number` elements: `nativeToBytes` / `nativeFromBytes`, `defaultValue` and
/// `scalar()`, each element range-checked with an actionable error.
macro_rules! int_serie_wire_number_dtype {
    ($ty:ident, $serie:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "dtype")]
        impl $ty {
            /// Serialize a native serie into its Arrow bytes — the value type's codec,
            /// concatenated per element.
            #[napi]
            pub fn native_to_bytes(&self, values: Vec<i64>) -> Result<Buffer> {
                let values = values
                    .into_iter()
                    .map(|value| wire_to_native::<$native>(value, $name))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Buffer::from(self.inner.native_to_bytes(&values)))
            }

            /// Deserialize Arrow bytes into a native serie — the exact inverse of
            /// `nativeToBytes`; a length that is not a whole number of elements throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Vec<i64>> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(|values| values.into_iter().map(i64::from).collect())
                    .map_err(data_error)
            }

            /// The type's default native value, the empty serie.
            #[napi]
            pub fn default_value(&self) -> Vec<i64> {
                TypedDataType::<Vec<$native>>::default_value(&self.inner)
                    .into_iter()
                    .map(i64::from)
                    .collect()
            }

            /// A `yggdryl.scalar` serie holding the native serie `values`.
            #[napi]
            pub fn scalar(&self, values: Vec<i64>) -> Result<crate::scalar::$serie> {
                let values = values
                    .into_iter()
                    .map(|value| wire_to_native::<$native>(value, $name))
                    .collect::<Result<Vec<_>>>()?;
                Ok(crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::from(values),
                })
            }
        }
    };
}

/// Generates the wire-dependent codec of a float serie type over JS `number`
/// elements (each an `f64` narrowed to the element width through [`WireFloat`]):
/// `nativeToBytes` / `nativeFromBytes`, `defaultValue` and `scalar()`. The float
/// analog of [`int_serie_wire_number_dtype!`] with no per-element range check.
macro_rules! float_serie_wire_number_dtype {
    ($ty:ident, $serie:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "dtype")]
        impl $ty {
            /// Serialize a native serie into its Arrow bytes — the value type's codec,
            /// concatenated per element.
            #[napi]
            pub fn native_to_bytes(&self, values: Vec<f64>) -> Buffer {
                let values = values
                    .into_iter()
                    .map(<$native>::from_wire)
                    .collect::<Vec<_>>();
                Buffer::from(self.inner.native_to_bytes(&values))
            }

            /// Deserialize Arrow bytes into a native serie — the exact inverse of
            /// `nativeToBytes`; a length that is not a whole number of elements throws.
            #[napi]
            pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Vec<f64>> {
                self.inner
                    .native_from_bytes(&bytes)
                    .map(|values| values.into_iter().map(<$native>::to_wire).collect())
                    .map_err(data_error)
            }

            /// The type's default native value, the empty serie.
            #[napi]
            pub fn default_value(&self) -> Vec<f64> {
                TypedDataType::<Vec<$native>>::default_value(&self.inner)
                    .into_iter()
                    .map(<$native>::to_wire)
                    .collect()
            }

            /// A `yggdryl.scalar` serie holding the native serie `values`.
            #[napi]
            pub fn scalar(&self, values: Vec<f64>) -> crate::scalar::$serie {
                let values = values
                    .into_iter()
                    .map(<$native>::from_wire)
                    .collect::<Vec<_>>();
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::from(values),
                }
            }
        }
    };
}

int_serie_dtype_node!(Int8SerieType, Int8Type, Int8SerieField, Int8Serie, "int8");
int_serie_dtype_node!(
    Int16SerieType,
    Int16Type,
    Int16SerieField,
    Int16Serie,
    "int16"
);
int_serie_dtype_node!(
    Int32SerieType,
    Int32Type,
    Int32SerieField,
    Int32Serie,
    "int32"
);
int_serie_dtype_node!(
    Int64SerieType,
    Int64Type,
    Int64SerieField,
    Int64Serie,
    "int64"
);
int_serie_dtype_node!(
    UInt8SerieType,
    UInt8Type,
    UInt8SerieField,
    UInt8Serie,
    "uint8"
);
int_serie_dtype_node!(
    UInt16SerieType,
    UInt16Type,
    UInt16SerieField,
    UInt16Serie,
    "uint16"
);
int_serie_dtype_node!(
    UInt32SerieType,
    UInt32Type,
    UInt32SerieField,
    UInt32Serie,
    "uint32"
);
int_serie_dtype_node!(
    UInt64SerieType,
    UInt64Type,
    UInt64SerieField,
    UInt64Serie,
    "uint64"
);

int_serie_wire_number_dtype!(Int8SerieType, Int8Serie, i8, "int8");
int_serie_wire_number_dtype!(Int16SerieType, Int16Serie, i16, "int16");
int_serie_wire_number_dtype!(Int32SerieType, Int32Serie, i32, "int32");
int_serie_wire_number_dtype!(UInt8SerieType, UInt8Serie, u8, "uint8");
int_serie_wire_number_dtype!(UInt16SerieType, UInt16Serie, u16, "uint16");
int_serie_wire_number_dtype!(UInt32SerieType, UInt32Serie, u32, "uint32");

// The float series reuse the width-independent serie-type surface, then carry their
// codec elements as JS `number` (both narrow to `f64` on the wire — see `WireFloat`).
int_serie_dtype_node!(
    Float32SerieType,
    Float32Type,
    Float32SerieField,
    Float32Serie,
    "float32"
);
int_serie_dtype_node!(
    Float64SerieType,
    Float64Type,
    Float64SerieField,
    Float64Serie,
    "float64"
);

float_serie_wire_number_dtype!(Float32SerieType, Float32Serie, f32, "float32");
float_serie_wire_number_dtype!(Float64SerieType, Float64Serie, f64, "float64");

// The 64-bit series carry their elements as JS `BigInt` (a `number` cannot
// represent the full range), so their width-dependent surface is written out per
// type.

#[napi(namespace = "dtype")]
impl Int64SerieType {
    /// Serialize a native serie into its Arrow bytes — the value type's codec,
    /// concatenated per element.
    #[napi]
    pub fn native_to_bytes(&self, values: Vec<BigInt>) -> Result<Buffer> {
        let values = values
            .into_iter()
            .map(bigint_to_i64)
            .collect::<Result<Vec<_>>>()?;
        Ok(Buffer::from(self.inner.native_to_bytes(&values)))
    }

    /// Deserialize Arrow bytes into a native serie — the exact inverse of
    /// `nativeToBytes`; a length that is not a whole number of elements throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Vec<BigInt>> {
        self.inner
            .native_from_bytes(&bytes)
            .map(|values| values.into_iter().map(BigInt::from).collect())
            .map_err(data_error)
    }

    /// The type's default native value, the empty serie.
    #[napi]
    pub fn default_value(&self) -> Vec<BigInt> {
        TypedDataType::<Vec<i64>>::default_value(&self.inner)
            .into_iter()
            .map(BigInt::from)
            .collect()
    }

    /// A `yggdryl.scalar.Int64Serie` holding the native serie `values`.
    #[napi]
    pub fn scalar(&self, values: Vec<BigInt>) -> Result<crate::scalar::Int64Serie> {
        let values = values
            .into_iter()
            .map(bigint_to_i64)
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::scalar::Int64Serie {
            inner: yggdryl_scalar::Int64Serie::from(values),
        })
    }
}

#[napi(namespace = "dtype")]
impl UInt64SerieType {
    /// Serialize a native serie into its Arrow bytes — the value type's codec,
    /// concatenated per element.
    #[napi]
    pub fn native_to_bytes(&self, values: Vec<BigInt>) -> Result<Buffer> {
        let values = values
            .into_iter()
            .map(bigint_to_u64)
            .collect::<Result<Vec<_>>>()?;
        Ok(Buffer::from(self.inner.native_to_bytes(&values)))
    }

    /// Deserialize Arrow bytes into a native serie — the exact inverse of
    /// `nativeToBytes`; a length that is not a whole number of elements throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<Vec<BigInt>> {
        self.inner
            .native_from_bytes(&bytes)
            .map(|values| values.into_iter().map(BigInt::from).collect())
            .map_err(data_error)
    }

    /// The type's default native value, the empty serie.
    #[napi]
    pub fn default_value(&self) -> Vec<BigInt> {
        TypedDataType::<Vec<u64>>::default_value(&self.inner)
            .into_iter()
            .map(BigInt::from)
            .collect()
    }

    /// A `yggdryl.scalar.UInt64Serie` holding the native serie `values`.
    #[napi]
    pub fn scalar(&self, values: Vec<BigInt>) -> Result<crate::scalar::UInt64Serie> {
        let values = values
            .into_iter()
            .map(bigint_to_u64)
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::scalar::UInt64Serie {
            inner: yggdryl_scalar::UInt64Serie::from(values),
        })
    }
}
