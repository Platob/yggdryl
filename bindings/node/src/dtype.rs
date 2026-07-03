//! The `yggdryl.dtype` namespace — thin wrappers over the `yggdryl-dtype` crate.
//!
//! Every integer type is exposed as its data type and its logical optional data
//! type (`yggdryl.dtype.Int64`, `yggdryl.dtype.OptionalInt64`, …), alongside
//! `Binary` / `OptionalBinary`, `Null` and `Union` — the same bare names as the
//! Rust crate, the namespace carrying the concern. The native classes carry a
//! unique `Dtype` prefix (napi registers class constructors by JS class name in
//! one addon-global registry, so a bare `Int64` here would collide with the
//! field and scalar classes); the hand-written `yggdryl.js` / `yggdryl.d.ts`
//! wrapper strips the prefix into the `dtype` namespace. Values adapt to JS
//! idioms: the 8–32 bit types carry their codec values as `number`, the 64-bit
//! types as `BigInt`. Data types expose the descriptor surface (`name`,
//! `arrowFormat`, widths), the native byte codec, and their defaults
//! (`defaultScalar` hands back a `yggdryl.scalar` class).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work), construction of a
//! `Union` from arbitrary child fields (its `UnionFields` is an arrow-schema
//! value — `Union` is reached through an optional data type's `storage()`), the
//! `DataTypeId` classifier (a method-bearing enum the bindings cannot model
//! uniformly), and the generic nested types (`List` / `Map` / `Struct` and the
//! per-family trait pairs), which have no concrete FFI shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Result};
use napi_derive::napi;
use yggdryl_dtype::{DataType, RawDataType, RawLogical, RawNested, RawUnion};
use yggdryl_scalar::DefaultScalar;

use crate::{bigint_to_i64, bigint_to_u64, data_error, wire_to_native};

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[napi]
pub struct DtypeUnion {
    pub(crate) inner: yggdryl_dtype::Union,
}

#[napi]
impl DtypeUnion {
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

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[napi]
#[derive(Default)]
pub struct DtypeNull {
    pub(crate) inner: yggdryl_dtype::Null,
}

#[napi]
impl DtypeNull {
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
#[napi]
#[derive(Default)]
pub struct DtypeBinary {
    pub(crate) inner: yggdryl_dtype::Binary,
}

#[napi]
impl DtypeBinary {
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
        Buffer::from(DataType::default_value(&self.inner))
    }

    /// The default scalar: a `yggdryl.scalar.Binary` holding empty bytes.
    #[napi]
    pub fn default_scalar(&self) -> crate::scalar::ScalarBinary {
        crate::scalar::ScalarBinary {
            inner: self.inner.default_scalar(),
        }
    }

    /// The logical optional of this type (stored as the null-or-value union).
    #[napi]
    pub fn optional(&self) -> DtypeOptionalBinary {
        DtypeOptionalBinary::default()
    }
}

/// The logical optional of `binary`: a value, or null — stored as the
/// null-or-`binary` union.
#[napi]
#[derive(Default)]
pub struct DtypeOptionalBinary {
    pub(crate) inner: yggdryl_dtype::Optional<yggdryl_dtype::Binary>,
}

#[napi]
impl DtypeOptionalBinary {
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
    pub fn value_type(&self) -> DtypeBinary {
        DtypeBinary::default()
    }

    /// The physical storage: the sparse null-or-value union.
    #[napi]
    pub fn storage(&self) -> DtypeUnion {
        DtypeUnion {
            inner: self.inner.storage().clone(),
        }
    }

    /// The default native value: the value type's default, an empty `Buffer`.
    #[napi]
    pub fn default_value(&self) -> Buffer {
        Buffer::from(DataType::default_value(&self.inner))
    }

    /// The default scalar: the null variant (the scalar models nullness).
    #[napi]
    pub fn default_scalar(&self) -> crate::scalar::ScalarOptionalBinary {
        crate::scalar::ScalarOptionalBinary {
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
/// `$ty` (descriptor, defaults and `optional()`) and the logical optional data
/// type `$opt_ty` (over union storage). The width-dependent codec is generated by
/// [`int_wire_number_dtype!`] (8–32 bit, JS `number`) or written per 64-bit type
/// with `BigInt`.
macro_rules! int_dtype_node {
    ($ty:ident, $opt_ty:ident, $scalar:ident, $opt_scalar:ident, $native:ident, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type.")]
        #[napi]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::$native,
        }

        #[napi]
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
        #[napi]
        #[derive(Default)]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_dtype::Optional<yggdryl_dtype::$native>,
        }

        #[napi]
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

            /// The default scalar: the null variant (the scalar models nullness).
            #[napi]
            pub fn default_scalar(&self) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The physical storage: the sparse null-or-value union.
            #[napi]
            pub fn storage(&self) -> DtypeUnion {
                DtypeUnion {
                    inner: self.inner.storage().clone(),
                }
            }
        }
    };
}

/// Generates the width-dependent codec of an 8–32 bit integer type over JS
/// `number`: `nativeToBytes` / `nativeFromBytes` and `defaultValue` on the data
/// type and its optional, range-checked with an actionable error.
macro_rules! int_wire_number_dtype {
    ($ty:ident, $opt_ty:ident, $native:ty, $name:literal) => {
        #[napi]
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

        #[napi]
        impl $ty {
            /// The type's default native value, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(DataType::default_value(&self.inner))
            }
        }

        #[napi]
        impl $opt_ty {
            /// The default native value of the value type, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(DataType::default_value(&self.inner))
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

int_dtype_node!(
    DtypeInt8,
    DtypeOptionalInt8,
    ScalarInt8,
    ScalarOptionalInt8,
    Int8,
    "int8"
);
int_dtype_node!(
    DtypeInt16,
    DtypeOptionalInt16,
    ScalarInt16,
    ScalarOptionalInt16,
    Int16,
    "int16"
);
int_dtype_node!(
    DtypeInt32,
    DtypeOptionalInt32,
    ScalarInt32,
    ScalarOptionalInt32,
    Int32,
    "int32"
);
int_dtype_node!(
    DtypeInt64,
    DtypeOptionalInt64,
    ScalarInt64,
    ScalarOptionalInt64,
    Int64,
    "int64"
);
int_dtype_node!(
    DtypeUInt8,
    DtypeOptionalUInt8,
    ScalarUInt8,
    ScalarOptionalUInt8,
    UInt8,
    "uint8"
);
int_dtype_node!(
    DtypeUInt16,
    DtypeOptionalUInt16,
    ScalarUInt16,
    ScalarOptionalUInt16,
    UInt16,
    "uint16"
);
int_dtype_node!(
    DtypeUInt32,
    DtypeOptionalUInt32,
    ScalarUInt32,
    ScalarOptionalUInt32,
    UInt32,
    "uint32"
);
int_dtype_node!(
    DtypeUInt64,
    DtypeOptionalUInt64,
    ScalarUInt64,
    ScalarOptionalUInt64,
    UInt64,
    "uint64"
);

int_wire_number_dtype!(DtypeInt8, DtypeOptionalInt8, i8, "int8");
int_wire_number_dtype!(DtypeInt16, DtypeOptionalInt16, i16, "int16");
int_wire_number_dtype!(DtypeInt32, DtypeOptionalInt32, i32, "int32");
int_wire_number_dtype!(DtypeUInt8, DtypeOptionalUInt8, u8, "uint8");
int_wire_number_dtype!(DtypeUInt16, DtypeOptionalUInt16, u16, "uint16");
int_wire_number_dtype!(DtypeUInt32, DtypeOptionalUInt32, u32, "uint32");

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent codec is written out per type.

#[napi]
impl DtypeInt64 {
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

#[napi]
impl DtypeInt64 {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi]
impl DtypeOptionalInt64 {
    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::<i64>::default_value(&self.inner))
    }

    /// Serialize a native value into its little-endian Arrow bytes — the value
    /// type's codec.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(DataType::<i64>::native_to_bytes(
            &self.inner,
            &bigint_to_i64(value)?,
        )))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        DataType::<i64>::native_from_bytes(&self.inner, &bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }
}

#[napi]
impl DtypeUInt64 {
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

#[napi]
impl DtypeUInt64 {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi]
impl DtypeOptionalUInt64 {
    /// Serialize a native value into its little-endian Arrow bytes — the value
    /// type's codec.
    #[napi]
    pub fn native_to_bytes(&self, value: BigInt) -> Result<Buffer> {
        Ok(Buffer::from(DataType::<u64>::native_to_bytes(
            &self.inner,
            &bigint_to_u64(value)?,
        )))
    }

    /// Deserialize little-endian Arrow bytes into a native value — the exact
    /// inverse of `nativeToBytes`; the wrong length throws.
    #[napi]
    pub fn native_from_bytes(&self, bytes: Buffer) -> Result<BigInt> {
        DataType::<u64>::native_from_bytes(&self.inner, &bytes)
            .map(BigInt::from)
            .map_err(data_error)
    }

    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::<u64>::default_value(&self.inner))
    }
}
