//! The `yggdryl.data` namespace — thin wrappers over the `yggdryl-data` crate.
//!
//! Every integer type is exposed as its data type, field, scalar, logical
//! optional data type and field, and null-or-value optional scalar (e.g.
//! `Int64Type`, `Int64Field`, `Int64`, `OptionalInt64Type`, `OptionalInt64Field`,
//! `OptionalInt64`), alongside the `NullType` family and the `UnionType` data type.
//! Values adapt to JS idioms: the 8–32 bit types use `number`, the 64-bit types
//! use `BigInt`, and scalars expose the `as*` accessors with the core contract —
//! the value when the target represents it exactly, or a thrown error naming the
//! fix (strings and `Buffer`s cross the FFI boundary as new JS objects, so the
//! Rust-side "borrow, never copy" guarantee applies up to that boundary copy).
//! The `BinaryType` family holds its bytes as a core positioned-IO `ByteBuffer` —
//! `toIo()` hands one back. TypedOptional scalars adapt construction to idioms: they
//! are built straight from the native value (`new OptionalInt64(42n)`), the
//! inner scalar being an implementation detail reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` / `arrow-array` values that
//! cannot cross the FFI boundary; C Data Interface interop is future work),
//! construction of a `UnionType` from arbitrary child fields (its `UnionFields` is an
//! arrow-schema value — `UnionType` is reached through an optional data type's
//! `storage()`),
//! the `DataTypeId` classifier (a method-bearing enum the bindings cannot
//! model uniformly), and the nested families — the generic `ListType` / `MapType` /
//! `StructType` with their scalars, the per-family trait pairs, and the
//! buffer-backed `Int64Serie` (whose zero-copy Arrow buffers await C Data
//! Interface interop) — which have no concrete FFI shape yet.

use napi::bindgen_prelude::{BigInt, Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_data::{DataType, RawDataType, RawField, RawLogical, RawNested, RawScalar, RawUnion};

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
pub struct UnionType {
    inner: yggdryl_data::UnionType,
}

#[napi(namespace = "data")]
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
            yggdryl_data::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_data::arrow_schema::UnionMode::Dense => "dense",
        }
    }
}

/// A nullable `union` field: a name paired with a `UnionType` data type.
#[napi(namespace = "data")]
pub struct UnionField {
    inner: yggdryl_data::UnionField,
}

#[napi(namespace = "data")]
impl UnionField {
    /// A field named `name` of the union type `dataType` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, data_type: &UnionType, nullable: Option<bool>) -> Self {
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
    pub fn data_type(&self) -> UnionType {
        UnionType {
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
pub struct NullType {
    inner: yggdryl_data::NullType,
}

#[napi(namespace = "data")]
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
    pub fn data_type(&self) -> NullType {
        NullType::default()
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
pub struct Null {
    inner: yggdryl_data::Null,
}

#[napi(namespace = "data")]
impl Null {
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
    pub fn data_type(&self) -> NullType {
        NullType::default()
    }
}

/// Generates the `as*` accessor block for a scalar wrapper class: the value when
/// exactly representable, or a thrown error naming the fix, with the 64-bit
/// targets as `BigInt` (a separate `#[napi]` impl block — napi merges the blocks
/// into one JS class).
macro_rules! as_accessors_node {
    ($class:ident) => {
        #[napi(namespace = "data")]
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
            /// The value as a string; throws when null or the value has no
            /// string form.
            #[napi]
            pub fn as_str(&self) -> Result<String> {
                self.inner.as_str().map(str::to_string).map_err(data_error)
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
            pub fn storage(&self) -> UnionType {
                UnionType {
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
            inner: yggdryl_data::Optional<yggdryl_data::$ty, yggdryl_data::$scalar>,
        }

        #[napi(namespace = "data")]
        impl $optional {
            /// The null variant.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: yggdryl_data::Optional::null(),
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
                    inner: yggdryl_data::Optional::new(yggdryl_data::$scalar::new(
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

/// The Apache Arrow `binary` data type: a variable-length byte sequence.
#[napi(namespace = "data")]
#[derive(Default)]
pub struct BinaryType {
    inner: yggdryl_data::BinaryType,
}

#[napi(namespace = "data")]
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
        Buffer::from(DataType::default_value(&self.inner))
    }

    /// The default scalar: a scalar holding empty bytes.
    #[napi]
    pub fn default_scalar(&self) -> Binary {
        Binary {
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
#[napi(namespace = "data")]
#[derive(Default)]
pub struct OptionalBinaryType {
    inner: yggdryl_data::OptionalType<yggdryl_data::BinaryType>,
}

#[napi(namespace = "data")]
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
        Buffer::from(DataType::default_value(&self.inner))
    }

    /// The default scalar: the null variant (the scalar models nullness).
    #[napi]
    pub fn default_scalar(&self) -> OptionalBinary {
        OptionalBinary {
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

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[napi(namespace = "data")]
pub struct OptionalBinaryField {
    inner: yggdryl_data::OptionalField<yggdryl_data::BinaryType>,
}

#[napi(namespace = "data")]
impl OptionalBinaryField {
    /// An optional-`binary` field named `name` (nullable by default).
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
    pub fn data_type(&self) -> OptionalBinaryType {
        OptionalBinaryType::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[napi(namespace = "data")]
pub struct BinaryField {
    inner: yggdryl_data::BinaryField,
}

#[napi(namespace = "data")]
impl BinaryField {
    /// A `binary` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_data::BinaryField::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> BinaryType {
        BinaryType::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A single, possibly-null `binary` value, holding its bytes as a core
/// positioned-IO `ByteBuffer` (`toIo()` hands one back).
#[napi(namespace = "data")]
pub struct Binary {
    inner: yggdryl_data::Binary,
}

#[napi(namespace = "data")]
impl Binary {
    /// A `binary` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_data::Binary::new(value.to_vec()),
        }
    }

    /// A null `binary` scalar.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_data::Binary::null(),
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
    pub fn data_type(&self) -> BinaryType {
        BinaryType::default()
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
}

as_accessors_node!(Binary);

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[napi(namespace = "data")]
pub struct OptionalBinary {
    inner: yggdryl_data::Optional<yggdryl_data::BinaryType, yggdryl_data::Binary>,
}

#[napi(namespace = "data")]
impl OptionalBinary {
    /// A scalar holding the `binary` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: Buffer) -> Self {
        Self {
            inner: yggdryl_data::Optional::new(yggdryl_data::Binary::new(value.to_vec())),
        }
    }

    /// The null variant.
    #[napi(factory)]
    pub fn null() -> Self {
        Self {
            inner: yggdryl_data::Optional::null(),
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
    pub fn scalar(&self) -> Option<Binary> {
        self.inner.scalar().map(|scalar| Binary {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    #[napi]
    pub fn data_type(&self) -> OptionalBinaryType {
        OptionalBinaryType::default()
    }
}

as_accessors_node!(OptionalBinary);

int_data_node!(
    Int8Type,
    Int8Field,
    Int8,
    OptionalInt8Type,
    OptionalInt8Field,
    OptionalInt8,
    i8,
    "int8"
);
int_data_node!(
    Int16Type,
    Int16Field,
    Int16,
    OptionalInt16Type,
    OptionalInt16Field,
    OptionalInt16,
    i16,
    "int16"
);
int_data_node!(
    Int32Type,
    Int32Field,
    Int32,
    OptionalInt32Type,
    OptionalInt32Field,
    OptionalInt32,
    i32,
    "int32"
);
int_data_node!(
    Int64Type,
    Int64Field,
    Int64,
    OptionalInt64Type,
    OptionalInt64Field,
    OptionalInt64,
    i64,
    "int64"
);
int_data_node!(
    UInt8Type,
    UInt8Field,
    UInt8,
    OptionalUInt8Type,
    OptionalUInt8Field,
    OptionalUInt8,
    u8,
    "uint8"
);
int_data_node!(
    UInt16Type,
    UInt16Field,
    UInt16,
    OptionalUInt16Type,
    OptionalUInt16Field,
    OptionalUInt16,
    u16,
    "uint16"
);
int_data_node!(
    UInt32Type,
    UInt32Field,
    UInt32,
    OptionalUInt32Type,
    OptionalUInt32Field,
    OptionalUInt32,
    u32,
    "uint32"
);
int_data_node!(
    UInt64Type,
    UInt64Field,
    UInt64,
    OptionalUInt64Type,
    OptionalUInt64Field,
    OptionalUInt64,
    u64,
    "uint64"
);

int_wire_number_node!(Int8Type, Int8, OptionalInt8Type, OptionalInt8, i8, "int8");
int_wire_number_node!(
    Int16Type,
    Int16,
    OptionalInt16Type,
    OptionalInt16,
    i16,
    "int16"
);
int_wire_number_node!(
    Int32Type,
    Int32,
    OptionalInt32Type,
    OptionalInt32,
    i32,
    "int32"
);
int_wire_number_node!(
    UInt8Type,
    UInt8,
    OptionalUInt8Type,
    OptionalUInt8,
    u8,
    "uint8"
);
int_wire_number_node!(
    UInt16Type,
    UInt16,
    OptionalUInt16Type,
    OptionalUInt16,
    u16,
    "uint16"
);
int_wire_number_node!(
    UInt32Type,
    UInt32,
    OptionalUInt32Type,
    OptionalUInt32,
    u32,
    "uint32"
);

// The 64-bit types carry their values as JS `BigInt` (a `number` cannot represent
// the full range), so their width-dependent surface is written out per type.

#[napi(namespace = "data")]
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

#[napi(namespace = "data")]
impl Int64Type {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl OptionalInt64Type {
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

#[napi(namespace = "data")]
impl Int64 {
    /// An `int64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::Int64::new(bigint_to_i64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "data")]
impl OptionalInt64 {
    /// A scalar holding the `int64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::Optional::new(yggdryl_data::Int64::new(bigint_to_i64(value)?)),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "data")]
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

#[napi(namespace = "data")]
impl UInt64Type {
    /// The type's default native value, `0n`.
    #[napi]
    pub fn default_value(&self) -> BigInt {
        BigInt::from(DataType::default_value(&self.inner))
    }
}

#[napi(namespace = "data")]
impl OptionalUInt64Type {
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

#[napi(namespace = "data")]
impl UInt64 {
    /// A `uint64` scalar holding `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::UInt64::new(bigint_to_u64(value)?),
        })
    }

    /// The scalar's value, or `null` when null.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}

#[napi(namespace = "data")]
impl OptionalUInt64 {
    /// A scalar holding the `uint64` value variant `value`.
    #[napi(constructor)]
    pub fn new(value: BigInt) -> Result<Self> {
        Ok(Self {
            inner: yggdryl_data::Optional::new(yggdryl_data::UInt64::new(bigint_to_u64(value)?)),
        })
    }

    /// The value, or `null` for the null variant.
    #[napi]
    pub fn value(&self) -> Option<BigInt> {
        self.inner.value().copied().map(BigInt::from)
    }
}
