//! The `yggdryl.data` namespace — thin wrappers over the `yggdryl-data` crate.
//!
//! Every integer type is exposed as its data type, field, scalar, logical
//! optional data type and field, and null-or-value optional scalar (e.g.
//! `Int64`, `Int64Field`, `Int64Scalar`, `OptionalInt64`, `OptionalInt64Field`,
//! `OptionalInt64Scalar`), alongside the `Null` family and the `Union` data type.
//! Values adapt to JS idioms: the 8–32 bit types use `number`, the 64-bit types
//! use `BigInt`, and scalars expose the `as*` accessors with the core contract —
//! exact conversion or `null` (strings cross the FFI boundary as new JS strings,
//! so the Rust-side "borrow, never copy" guarantee applies up to that boundary
//! copy). Optional scalars adapt construction to idioms: they are built straight
//! from the native value (`new OptionalInt64Scalar(42n)`), the inner scalar being
//! an implementation detail reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` / `arrow-array` values that
//! cannot cross the FFI boundary; C Data Interface interop is future work),
//! construction of a `Union` from arbitrary child fields (its `UnionFields` is an
//! arrow-schema value — `Union` is reached through an optional data type's
//! `storage()`),
//! the `DataTypeId` classifier (a method-bearing enum the bindings cannot
//! model uniformly), and the generic nested families (`ListType` / `MapType` /
//! `StructType` with their scalars) and per-family trait pairs, which have no
//! concrete FFI shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_data::{DataType, Logical, Nested, RawDataType, RawField, RawScalar, RawUnion};

fn data_error(error: yggdryl_data::DataError) -> Error {
    Error::from_reason(error.to_string())
}

/// A `BigInt` as an `i64`, or an actionable error when out of range.
fn bigint_to_i64(value: BigInt) -> Result<i64> {
    let (value, lossless) = value.get_i64();
    if lossless {
        Ok(value)
    } else {
        Err(Error::from_reason(
            "expected an int64 in -(2**63)..=2**63-1",
        ))
    }
}

/// A `BigInt` as a `u64`, or an actionable error when negative or out of range.
fn bigint_to_u64(value: BigInt) -> Result<u64> {
    let (sign, value, lossless) = value.get_u64();
    if !sign && lossless {
        Ok(value)
    } else {
        Err(Error::from_reason("expected a uint64 in 0..=2**64-1"))
    }
}

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[napi(namespace = "data")]
pub struct Union {
    inner: yggdryl_data::UnionType,
}

#[napi(namespace = "data")]
impl Union {
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
            yggdryl_data::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_data::arrow_schema::UnionMode::Dense => "dense",
        }
    }
}

/// A nullable `union` field: a name paired with a `Union` data type.
#[napi(namespace = "data")]
pub struct UnionField {
    inner: yggdryl_data::UnionField,
}

#[napi(namespace = "data")]
impl UnionField {
    /// A field named `name` of the union type `dataType` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, data_type: &Union, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_data::UnionField::new(
                name,
                data_type.inner.clone(),
                nullable.unwrap_or(true),
            ),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> Union {
        Union {
            inner: self.inner.data_type().clone(),
        }
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[napi(namespace = "data")]
#[derive(Default)]
pub struct Null {
    inner: yggdryl_data::Null,
}

#[napi(namespace = "data")]
impl Null {
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

/// A `null` field: a name paired with the null data type.
#[napi(namespace = "data")]
pub struct NullField {
    inner: yggdryl_data::NullField,
}

#[napi(namespace = "data")]
impl NullField {
    /// A `null` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_data::NullField::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> Null {
        Null::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// The `null` scalar: always null, holding no value.
#[napi(namespace = "data")]
#[derive(Default)]
pub struct NullScalar {
    inner: yggdryl_data::NullScalar,
}

#[napi(namespace = "data")]
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
    pub fn data_type(&self) -> Null {
        Null::default()
    }
}

/// Generates the `as*` accessor block for a scalar wrapper class: exact conversion
/// or `null`, with the 64-bit targets as `BigInt` (a separate `#[napi]` impl block —
/// napi merges the blocks into one JS class).
macro_rules! as_accessors_node {
    ($class:ident) => {
        #[napi(namespace = "data")]
        impl $class {
            /// The value as a number in the i8 range, when exactly representable.
            #[napi]
            pub fn as_i8(&self) -> Option<i32> {
                self.inner.as_i8().map(i32::from)
            }
            /// The value as a number in the i16 range, when exactly representable.
            #[napi]
            pub fn as_i16(&self) -> Option<i32> {
                self.inner.as_i16().map(i32::from)
            }
            /// The value as a number in the i32 range, when exactly representable.
            #[napi]
            pub fn as_i32(&self) -> Option<i32> {
                self.inner.as_i32()
            }
            /// The value as a `BigInt` in the i64 range, when exactly representable.
            #[napi]
            pub fn as_i64(&self) -> Option<BigInt> {
                self.inner.as_i64().map(BigInt::from)
            }
            /// The value as a number in the u8 range, when exactly representable.
            #[napi]
            pub fn as_u8(&self) -> Option<u32> {
                self.inner.as_u8().map(u32::from)
            }
            /// The value as a number in the u16 range, when exactly representable.
            #[napi]
            pub fn as_u16(&self) -> Option<u32> {
                self.inner.as_u16().map(u32::from)
            }
            /// The value as a number in the u32 range, when exactly representable.
            #[napi]
            pub fn as_u32(&self) -> Option<u32> {
                self.inner.as_u32()
            }
            /// The value as a `BigInt` in the u64 range, when exactly representable.
            #[napi]
            pub fn as_u64(&self) -> Option<BigInt> {
                self.inner.as_u64().map(BigInt::from)
            }
            /// The value as a number, when exactly representable in f32.
            #[napi]
            pub fn as_f32(&self) -> Option<f32> {
                self.inner.as_f32()
            }
            /// The value as a number, when exactly representable in f64.
            #[napi]
            pub fn as_f64(&self) -> Option<f64> {
                self.inner.as_f64()
            }
            /// The value as a boolean, when the value is a boolean.
            #[napi]
            pub fn as_bool(&self) -> Option<bool> {
                self.inner.as_bool()
            }
            /// The value as a string, when the value is a string.
            #[napi]
            pub fn as_str(&self) -> Option<String> {
                self.inner.as_str().map(str::to_string)
            }
        }
    };
}

/// Generates the width-independent surface of one integer type: the data type
/// `$ty` (descriptor + `optional()`), the field `$field`, and the null factory /
/// nullness / `dataType` / `scalar` of `$scalar` and `$optional`. The
/// width-dependent constructor, `value` and byte codec are generated by
/// [`int_wire_number_node!`] (8–32 bit, JS `number`) or written per 64-bit type
/// with `BigInt`.
macro_rules! int_data_node {
    ($ty:ident, $field:ident, $scalar:ident, $opt_ty:ident, $opt_field:ident, $optional:ident, $native:ty, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type.")]
        #[napi(namespace = "data")]
        #[derive(Default)]
        pub struct $ty {
            inner: yggdryl_data::$ty,
        }

        #[napi(namespace = "data")]
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

            /// The default scalar: a scalar holding `0`.
            #[napi]
            pub fn default_scalar(&self) -> $scalar {
                $scalar {
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
        #[napi(namespace = "data")]
        #[derive(Default)]
        pub struct $opt_ty {
            inner: yggdryl_data::OptionalType<yggdryl_data::$ty>,
        }

        #[napi(namespace = "data")]
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
            pub fn default_scalar(&self) -> $optional {
                $optional {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The physical storage: the sparse null-or-value union.
            #[napi]
            pub fn storage(&self) -> Union {
                Union {
                    inner: self.inner.storage().clone(),
                }
            }
        }

        #[doc = concat!("A nullable optional-`", $name, "` field: a name paired with the logical optional data type.")]
        #[napi(namespace = "data")]
        pub struct $opt_field {
            inner: yggdryl_data::OptionalField<yggdryl_data::$ty>,
        }

        #[napi(namespace = "data")]
        impl $opt_field {
            #[doc = concat!("An optional-`", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_data::OptionalField::new(name, nullable.unwrap_or(true)),
                }
            }

            /// The field's name.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            #[napi]
            pub fn data_type(&self) -> $opt_ty {
                $opt_ty::default()
            }

            /// Whether values in this field may be null.
            #[napi]
            pub fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A nullable `", $name, "` field: a name paired with the data type.")]
        #[napi(namespace = "data")]
        pub struct $field {
            inner: yggdryl_data::$field,
        }

        #[napi(namespace = "data")]
        impl $field {
            #[doc = concat!("A `", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_data::$field::new(name, nullable.unwrap_or(true)),
                }
            }

            /// The field's name.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            #[napi]
            pub fn data_type(&self) -> $ty {
                $ty::default()
            }

            /// Whether values in this field may be null.
            #[napi]
            pub fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A single, possibly-null `", $name, "` value.")]
        #[napi(namespace = "data")]
        pub struct $scalar {
            inner: yggdryl_data::$scalar,
        }

        #[napi(namespace = "data")]
        impl $scalar {
            #[doc = concat!("A null `", $name, "` scalar.")]
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_data::$scalar::null(),
                }
            }

            /// Whether this scalar holds a null value.
            #[napi]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The scalar's data type.
            #[napi]
            pub fn data_type(&self) -> $ty {
                $ty::default()
            }
        }

        as_accessors_node!($scalar);

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[napi(namespace = "data")]
        pub struct $optional {
            inner: yggdryl_data::OptionalScalar<yggdryl_data::$ty, yggdryl_data::$scalar>,
        }

        #[napi(namespace = "data")]
        impl $optional {
            /// The null variant.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_data::OptionalScalar::null(),
                }
            }

            /// Whether this scalar holds the null variant.
            #[napi]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The inner scalar, when this holds the value variant.
            #[napi]
            pub fn scalar(&self) -> Option<$scalar> {
                self.inner.scalar().map(|scalar| $scalar { inner: *scalar })
            }

            /// The scalar's data type: the logical optional of the value type.
            #[napi]
            pub fn data_type(&self) -> $opt_ty {
                $opt_ty::default()
            }
        }

        as_accessors_node!($optional);
    };
}

/// Generates the width-dependent surface of an 8–32 bit integer type over JS
/// `number`: the data type's byte codec and the scalar / optional-scalar
/// constructor and `value`, range-checked with an actionable error.
macro_rules! int_wire_number_node {
    ($ty:ident, $scalar:ident, $opt_ty:ident, $optional:ident, $native:ty, $name:literal) => {
        #[napi(namespace = "data")]
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

        #[napi(namespace = "data")]
        impl $ty {
            /// The type's default native value, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(DataType::default_value(&self.inner))
            }
        }

        #[napi(namespace = "data")]
        impl $opt_ty {
            /// The default native value of the value type, `0`.
            #[napi]
            pub fn default_value(&self) -> i64 {
                i64::from(DataType::default_value(&self.inner))
            }
        }

        #[napi(namespace = "data")]
        impl $scalar {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_data::$scalar::new(wire_to_native::<$native>(value, $name)?),
                })
            }

            /// The scalar's value, or `null` when null.
            #[napi]
            pub fn value(&self) -> Option<i64> {
                self.inner.value().copied().map(i64::from)
            }
        }

        #[napi(namespace = "data")]
        impl $optional {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[napi(constructor)]
            pub fn new(value: i64) -> Result<Self> {
                Ok(Self {
                    inner: yggdryl_data::OptionalScalar::new(yggdryl_data::$scalar::new(
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

/// A JS `number` (as `i64`) narrowed to the native type, or an actionable error.
fn wire_to_native<T: TryFrom<i64>>(value: i64, name: &str) -> Result<T> {
    T::try_from(value)
        .map_err(|_| Error::from_reason(format!("expected {value} to be in the {name} range")))
}

int_data_node!(
    Int8,
    Int8Field,
    Int8Scalar,
    OptionalInt8,
    OptionalInt8Field,
    OptionalInt8Scalar,
    i8,
    "int8"
);
int_data_node!(
    Int16,
    Int16Field,
    Int16Scalar,
    OptionalInt16,
    OptionalInt16Field,
    OptionalInt16Scalar,
    i16,
    "int16"
);
int_data_node!(
    Int32,
    Int32Field,
    Int32Scalar,
    OptionalInt32,
    OptionalInt32Field,
    OptionalInt32Scalar,
    i32,
    "int32"
);
int_data_node!(
    Int64,
    Int64Field,
    Int64Scalar,
    OptionalInt64,
    OptionalInt64Field,
    OptionalInt64Scalar,
    i64,
    "int64"
);
int_data_node!(
    UInt8,
    UInt8Field,
    UInt8Scalar,
    OptionalUInt8,
    OptionalUInt8Field,
    OptionalUInt8Scalar,
    u8,
    "uint8"
);
int_data_node!(
    UInt16,
    UInt16Field,
    UInt16Scalar,
    OptionalUInt16,
    OptionalUInt16Field,
    OptionalUInt16Scalar,
    u16,
    "uint16"
);
int_data_node!(
    UInt32,
    UInt32Field,
    UInt32Scalar,
    OptionalUInt32,
    OptionalUInt32Field,
    OptionalUInt32Scalar,
    u32,
    "uint32"
);
int_data_node!(
    UInt64,
    UInt64Field,
    UInt64Scalar,
    OptionalUInt64,
    OptionalUInt64Field,
    OptionalUInt64Scalar,
    u64,
    "uint64"
);

int_wire_number_node!(
    Int8,
    Int8Scalar,
    OptionalInt8,
    OptionalInt8Scalar,
    i8,
    "int8"
);
int_wire_number_node!(
    Int16,
    Int16Scalar,
    OptionalInt16,
    OptionalInt16Scalar,
    i16,
    "int16"
);
int_wire_number_node!(
    Int32,
    Int32Scalar,
    OptionalInt32,
    OptionalInt32Scalar,
    i32,
    "int32"
);
int_wire_number_node!(
    UInt8,
    UInt8Scalar,
    OptionalUInt8,
    OptionalUInt8Scalar,
    u8,
    "uint8"
);
int_wire_number_node!(
    UInt16,
    UInt16Scalar,
    OptionalUInt16,
    OptionalUInt16Scalar,
    u16,
    "uint16"
);
int_wire_number_node!(
    UInt32,
    UInt32Scalar,
    OptionalUInt32,
    OptionalUInt32Scalar,
    u32,
    "uint32"
);

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent surface is written out per type.

#[napi(namespace = "data")]
impl Int64 {
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

#[napi(namespace = "data")]
impl Int64 {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl OptionalInt64 {
    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::<i64>::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl Int64Scalar {
    /// An `int64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::Int64Scalar::new(bigint_to_i64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "data")]
impl OptionalInt64Scalar {
    /// A scalar holding the `int64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::OptionalScalar::new(yggdryl_data::Int64Scalar::new(
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

#[napi(namespace = "data")]
impl UInt64 {
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

#[napi(namespace = "data")]
impl UInt64 {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl OptionalUInt64 {
    /// The default native value of the value type, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::<u64>::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl UInt64Scalar {
    /// A `uint64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::UInt64Scalar::new(bigint_to_u64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "data")]
impl OptionalUInt64Scalar {
    /// A scalar holding the `uint64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::OptionalScalar::new(yggdryl_data::UInt64Scalar::new(
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
