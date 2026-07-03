//! The `yggdryl.data` submodule — thin wrappers over the `yggdryl-data` crate.
//!
//! Every integer type is exposed as its data type, field, scalar, logical
//! optional data type and field, and null-or-value optional scalar (e.g.
//! `Int64`, `Int64Field`, `Int64Scalar`, `OptionalInt64`, `OptionalInt64Field`,
//! `OptionalInt64Scalar`), alongside the `Binary` family (whose scalar holds its
//! bytes as a core positioned-IO `ByteBuffer` — `to_io()` hands one back), the
//! `Null` family and the `Union` data type. Scalars expose the `as_*` accessors
//! with the core contract: the value when the target represents it exactly, or a
//! raised `ValueError` naming the fix (strings and bytes cross the FFI boundary
//! as new Python objects, so the Rust-side "borrow, never copy" guarantee applies
//! up to that boundary copy).
//! Optional scalars adapt construction to idioms: they are built straight from the
//! native value (`OptionalInt64Scalar(42)`), the inner scalar being an
//! implementation detail reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` / `arrow-array` values that
//! cannot cross the FFI boundary; C Data Interface interop is future work),
//! construction of a `Union` from arbitrary child fields (its `UnionFields` is an
//! arrow-schema value — `Union` is reached through an optional data type's
//! `storage()`),
//! the `DataTypeId` classifier (a method-bearing enum the bindings cannot
//! model uniformly), and the nested families — the generic `ListType` / `MapType` /
//! `StructType` with their scalars, the per-family trait pairs, and the
//! buffer-backed `Int64Array` (whose zero-copy Arrow buffers await C Data
//! Interface interop) — which have no concrete FFI shape yet.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_data::{DataType, RawDataType, RawField, RawLogical, RawNested, RawScalar, RawUnion};

/// Wraps an [`yggdryl_data::DataError`] so pyo3 raises it as a Python `ValueError`.
struct DataErr(yggdryl_data::DataError);

impl From<yggdryl_data::DataError> for DataErr {
    fn from(error: yggdryl_data::DataError) -> Self {
        DataErr(error)
    }
}

impl From<DataErr> for PyErr {
    fn from(error: DataErr) -> Self {
        PyValueError::new_err(error.0.to_string())
    }
}

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[pyclass]
#[derive(Clone)]
pub struct Union {
    inner: yggdryl_data::UnionType,
}

#[pymethods]
impl Union {
    /// The type's lowercase name, `"union"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, e.g. `"+us:0,1"`.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A union has no fixed byte width.
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// A union has no fixed bit width.
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }

    /// The number of child fields.
    fn child_count(&self) -> usize {
        self.inner.child_count()
    }

    /// The union's mode: `"sparse"` or `"dense"`.
    fn mode(&self) -> &'static str {
        match self.inner.mode() {
            yggdryl_data::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_data::arrow_schema::UnionMode::Dense => "dense",
        }
    }
}

/// A nullable `union` field: a name paired with a [`Union`] data type.
#[pyclass]
pub struct UnionField {
    inner: yggdryl_data::UnionField,
}

#[pymethods]
impl UnionField {
    /// A field named `name` of the union type `data_type`.
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true))]
    fn new(name: String, data_type: &Union, nullable: bool) -> Self {
        Self {
            inner: yggdryl_data::UnionField::new(name, data_type.inner.clone(), nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> Union {
        Union {
            inner: self.inner.data_type().clone(),
        }
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[pyclass]
#[derive(Default)]
pub struct Null {
    inner: yggdryl_data::Null,
}

#[pymethods]
impl Null {
    /// The null data type.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"null"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"n"`.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// The null type has no storage, so no byte width.
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// The null type has no storage, so no bit width.
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }
}

/// A `null` field: a name paired with the null data type.
#[pyclass]
pub struct NullField {
    inner: yggdryl_data::NullField,
}

#[pymethods]
impl NullField {
    /// A `null` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_data::NullField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> Null {
        Null::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// The `null` scalar: always null, holding no value.
#[pyclass]
#[derive(Default)]
pub struct NullScalar {
    inner: yggdryl_data::NullScalar,
}

#[pymethods]
impl NullScalar {
    /// The null scalar.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Always `True`.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type.
    fn data_type(&self) -> Null {
        Null::default()
    }
}

/// The Apache Arrow `binary` data type: a variable-length byte sequence.
#[pyclass]
#[derive(Default)]
pub struct Binary {
    inner: yggdryl_data::Binary,
}

#[pymethods]
impl Binary {
    /// The `binary` data type.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"binary"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"z"`.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A binary value has no fixed byte width.
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// A binary value has no fixed bit width.
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }

    /// Serialize a native value into its Arrow bytes — the identity for binary.
    fn native_to_bytes<'py>(&self, py: Python<'py>, value: Vec<u8>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
    }

    /// Deserialize Arrow bytes into a native value — the identity for binary
    /// (any length is valid).
    fn native_from_bytes<'py>(
        &self,
        py: Python<'py>,
        bytes: &[u8],
    ) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(
            py,
            &self.inner.native_from_bytes(bytes)?,
        ))
    }

    /// The type's default native value, `b""`.
    fn default_value<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.default_value())
    }

    /// The default scalar: a scalar holding `b""`.
    fn default_scalar(&self) -> BinaryScalar {
        BinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The logical optional of this type (stored as the null-or-value union).
    fn optional(&self) -> OptionalBinary {
        OptionalBinary::default()
    }
}

/// The logical optional of `binary`: a value, or null — stored as the
/// null-or-`binary` union.
#[pyclass]
#[derive(Default)]
pub struct OptionalBinary {
    inner: yggdryl_data::OptionalType<yggdryl_data::Binary>,
}

#[pymethods]
impl OptionalBinary {
    /// The optional `binary` data type.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"optional"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string of the union storage.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// An optional has no fixed byte width (union storage).
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// An optional has no fixed bit width (union storage).
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }

    /// The value type this optional wraps.
    fn value_type(&self) -> Binary {
        Binary::default()
    }

    /// The physical storage: the sparse null-or-value union.
    fn storage(&self) -> Union {
        Union {
            inner: self.inner.storage().clone(),
        }
    }

    /// The default native value: the value type's default, `b""`.
    fn default_value<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.default_value())
    }

    /// The default scalar: the null variant (the scalar models nullness).
    fn default_scalar(&self) -> OptionalBinaryScalar {
        OptionalBinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// Serialize a native value into its Arrow bytes — the value type's codec.
    fn native_to_bytes<'py>(&self, py: Python<'py>, value: Vec<u8>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
    }

    /// Deserialize Arrow bytes into a native value — the exact inverse of
    /// `native_to_bytes`.
    fn native_from_bytes<'py>(
        &self,
        py: Python<'py>,
        bytes: &[u8],
    ) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(
            py,
            &self.inner.native_from_bytes(bytes)?,
        ))
    }
}

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[pyclass]
pub struct OptionalBinaryField {
    inner: yggdryl_data::OptionalField<yggdryl_data::Binary>,
}

#[pymethods]
impl OptionalBinaryField {
    /// An optional-`binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_data::OptionalField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> OptionalBinary {
        OptionalBinary::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[pyclass]
pub struct BinaryField {
    inner: yggdryl_data::BinaryField,
}

#[pymethods]
impl BinaryField {
    /// A `binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_data::BinaryField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> Binary {
        Binary::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A single, possibly-null `binary` value, holding its bytes as a core
/// positioned-IO `ByteBuffer` (`to_io()` hands one back).
#[pyclass]
pub struct BinaryScalar {
    inner: yggdryl_data::BinaryScalar,
}

#[pymethods]
impl BinaryScalar {
    /// A `binary` scalar holding `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_data::BinaryScalar::new(value),
        }
    }

    /// A null `binary` scalar.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_data::BinaryScalar::null(),
        }
    }

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's value as `bytes`, or `None` when null.
    fn value<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .value()
            .map(|bytes| PyBytes::new_bound(py, bytes))
    }

    /// The scalar's data type.
    fn data_type(&self) -> Binary {
        Binary::default()
    }

    /// The value as a core IO `ByteBuffer` (`yggdryl.core`), ready for
    /// positioned reads and the cursor / slice adapters, or `None` when null
    /// (the bytes cross the FFI boundary as one copy).
    fn to_io(&self) -> Option<crate::core::ByteBuffer> {
        self.inner
            .io()
            .map(|io| crate::core::ByteBuffer::from_inner(io.clone()))
    }

    /// The value as an `int` in the i8 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_i8(&self) -> Result<i8, DataErr> {
        Ok(self.inner.as_i8()?)
    }
    /// The value as an `int` in the i16 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_i16(&self) -> Result<i16, DataErr> {
        Ok(self.inner.as_i16()?)
    }
    /// The value as an `int` in the i32 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_i32(&self) -> Result<i32, DataErr> {
        Ok(self.inner.as_i32()?)
    }
    /// The value as an `int` in the i64 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_i64(&self) -> Result<i64, DataErr> {
        Ok(self.inner.as_i64()?)
    }
    /// The value as an `int` in the u8 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_u8(&self) -> Result<u8, DataErr> {
        Ok(self.inner.as_u8()?)
    }
    /// The value as an `int` in the u16 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_u16(&self) -> Result<u16, DataErr> {
        Ok(self.inner.as_u16()?)
    }
    /// The value as an `int` in the u32 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_u32(&self) -> Result<u32, DataErr> {
        Ok(self.inner.as_u32()?)
    }
    /// The value as an `int` in the u64 range; raises `ValueError` when
    /// null or not exactly representable.
    fn as_u64(&self) -> Result<u64, DataErr> {
        Ok(self.inner.as_u64()?)
    }
    /// The value as a `float`; raises `ValueError` when null or not
    /// exactly representable in f32.
    fn as_f32(&self) -> Result<f32, DataErr> {
        Ok(self.inner.as_f32()?)
    }
    /// The value as a `float`; raises `ValueError` when null or not
    /// exactly representable in f64.
    fn as_f64(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f64()?)
    }
    /// The value as a `bool`; raises `ValueError` when null or the value
    /// is not a boolean.
    fn as_bool(&self) -> Result<bool, DataErr> {
        Ok(self.inner.as_bool()?)
    }
    /// The value as a `str`; raises `ValueError` when null or the bytes are
    /// not valid UTF-8.
    fn as_str(&self) -> Result<String, DataErr> {
        Ok(self.inner.as_str().map(str::to_string)?)
    }
    /// The value as `bytes` — the native type; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }
}

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[pyclass]
pub struct OptionalBinaryScalar {
    inner: yggdryl_data::OptionalScalar<yggdryl_data::Binary, yggdryl_data::BinaryScalar>,
}

#[pymethods]
impl OptionalBinaryScalar {
    /// A scalar holding the `binary` value variant `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_data::OptionalScalar::new(yggdryl_data::BinaryScalar::new(value)),
        }
    }

    /// The null variant.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_data::OptionalScalar::null(),
        }
    }

    /// Whether this scalar holds the null variant.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The value as `bytes`, or `None` for the null variant.
    fn value<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .value()
            .map(|bytes| PyBytes::new_bound(py, bytes))
    }

    /// The inner scalar, when this holds the value variant.
    fn scalar(&self) -> Option<BinaryScalar> {
        self.inner.scalar().map(|scalar| BinaryScalar {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    fn data_type(&self) -> OptionalBinary {
        OptionalBinary::default()
    }

    /// The value as an `int` in the i8 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_i8(&self) -> Result<i8, DataErr> {
        Ok(self.inner.as_i8()?)
    }
    /// The value as an `int` in the i16 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_i16(&self) -> Result<i16, DataErr> {
        Ok(self.inner.as_i16()?)
    }
    /// The value as an `int` in the i32 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_i32(&self) -> Result<i32, DataErr> {
        Ok(self.inner.as_i32()?)
    }
    /// The value as an `int` in the i64 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_i64(&self) -> Result<i64, DataErr> {
        Ok(self.inner.as_i64()?)
    }
    /// The value as an `int` in the u8 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_u8(&self) -> Result<u8, DataErr> {
        Ok(self.inner.as_u8()?)
    }
    /// The value as an `int` in the u16 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_u16(&self) -> Result<u16, DataErr> {
        Ok(self.inner.as_u16()?)
    }
    /// The value as an `int` in the u32 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_u32(&self) -> Result<u32, DataErr> {
        Ok(self.inner.as_u32()?)
    }
    /// The value as an `int` in the u64 range; raises `ValueError` (a binary
    /// value has no numeric form).
    fn as_u64(&self) -> Result<u64, DataErr> {
        Ok(self.inner.as_u64()?)
    }
    /// The value as a `float`; raises `ValueError` (a binary value has no
    /// numeric form).
    fn as_f32(&self) -> Result<f32, DataErr> {
        Ok(self.inner.as_f32()?)
    }
    /// The value as a `float`; raises `ValueError` (a binary value has no
    /// numeric form).
    fn as_f64(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f64()?)
    }
    /// The value as a `bool`; raises `ValueError` (a binary value is not a
    /// boolean).
    fn as_bool(&self) -> Result<bool, DataErr> {
        Ok(self.inner.as_bool()?)
    }
    /// The value as a `str`; raises `ValueError` when null or the bytes are
    /// not valid UTF-8.
    fn as_str(&self) -> Result<String, DataErr> {
        Ok(self.inner.as_str().map(str::to_string)?)
    }
    /// The value as `bytes` — the native type; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }
}

/// Generates the six wrappers of one integer type: the data type `$ty` (with the
/// byte codec and `optional()`), the field `$field`, the scalar `$scalar`, the
/// logical optional data type `$opt_ty` (over union storage), its field
/// `$opt_field` and the null-or-value `$optional` scalar — each a thin delegation
/// to the `yggdryl-data` types, with the `as_*` accessors on both scalars.
macro_rules! int_data_py {
    ($ty:ident, $field:ident, $scalar:ident, $opt_ty:ident, $opt_field:ident, $optional:ident, $native:ty, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $ty {
            inner: yggdryl_data::$ty,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("The `", $name, "` data type.")]
            #[new]
            fn new() -> Self {
                Self::default()
            }

            #[doc = concat!("The type's lowercase name, `\"", $name, "\"`.")]
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string.
            fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// The fixed size of one value, in bytes.
            fn byte_width(&self) -> Option<usize> {
                self.inner.byte_width()
            }

            /// The fixed size of one value, in bits.
            fn bit_width(&self) -> Option<usize> {
                self.inner.bit_width()
            }

            /// Serialize a native value into its little-endian Arrow bytes.
            fn native_to_bytes<'py>(&self, py: Python<'py>, value: $native) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `native_to_bytes`; the wrong length raises `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<$native, DataErr> {
                Ok(self.inner.native_from_bytes(bytes)?)
            }

            /// The type's default native value, `0`.
            fn default_value(&self) -> $native {
                self.inner.default_value()
            }

            /// The default scalar: a scalar holding `0`.
            fn default_scalar(&self) -> $scalar {
                $scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The logical optional of this type (stored as the null-or-value
            /// union).
            fn optional(&self) -> $opt_ty {
                $opt_ty::default()
            }
        }

        #[doc = concat!("The logical optional of `", $name, "`: a value, or null — stored as the null-or-`", $name, "` union.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $opt_ty {
            inner: yggdryl_data::OptionalType<yggdryl_data::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("The optional `", $name, "` data type.")]
            #[new]
            fn new() -> Self {
                Self::default()
            }

            /// The type's lowercase name, `"optional"`.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string of the union storage.
            fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// An optional has no fixed byte width (union storage).
            fn byte_width(&self) -> Option<usize> {
                self.inner.byte_width()
            }

            /// An optional has no fixed bit width (union storage).
            fn bit_width(&self) -> Option<usize> {
                self.inner.bit_width()
            }

            /// The value type this optional wraps.
            fn value_type(&self) -> $ty {
                $ty::default()
            }

            /// The physical storage: the sparse null-or-value union.
            fn storage(&self) -> Union {
                Union {
                    inner: self.inner.storage().clone(),
                }
            }

            /// The default native value: the value type's default, `0`.
            fn default_value(&self) -> $native {
                self.inner.default_value()
            }

            /// The default scalar: the null variant (the scalar models nullness).
            fn default_scalar(&self) -> $optional {
                $optional {
                    inner: self.inner.default_scalar(),
                }
            }

            /// Serialize a native value into its little-endian Arrow bytes — the
            /// value type's codec.
            fn native_to_bytes<'py>(&self, py: Python<'py>, value: $native) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
            }

            /// Deserialize little-endian Arrow bytes into a native value — the exact
            /// inverse of `native_to_bytes`; the wrong length raises `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<$native, DataErr> {
                Ok(self.inner.native_from_bytes(bytes)?)
            }
        }

        #[doc = concat!("A nullable optional-`", $name, "` field: a name paired with the logical optional data type.")]
        #[pyclass]
        pub struct $opt_field {
            inner: yggdryl_data::OptionalField<yggdryl_data::$ty>,
        }

        #[pymethods]
        impl $opt_field {
            #[doc = concat!("An optional-`", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_data::OptionalField::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> $opt_ty {
                $opt_ty::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A nullable `", $name, "` field: a name paired with the data type.")]
        #[pyclass]
        pub struct $field {
            inner: yggdryl_data::$field,
        }

        #[pymethods]
        impl $field {
            #[doc = concat!("A `", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_data::$field::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> $ty {
                $ty::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A single, possibly-null `", $name, "` value.")]
        #[pyclass]
        pub struct $scalar {
            inner: yggdryl_data::$scalar,
        }

        #[pymethods]
        impl $scalar {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_data::$scalar::new(value),
                }
            }

            #[doc = concat!("A null `", $name, "` scalar.")]
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_data::$scalar::null(),
                }
            }

            /// Whether this scalar holds a null value.
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The scalar's value, or `None` when null.
            fn value(&self) -> Option<$native> {
                self.inner.value().copied()
            }

            /// The scalar's data type.
            fn data_type(&self) -> $ty {
                $ty::default()
            }

            /// The value as an `int` in the i8 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i8(&self) -> Result<i8, DataErr> {
                Ok(self.inner.as_i8()?)
            }
            /// The value as an `int` in the i16 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i16(&self) -> Result<i16, DataErr> {
                Ok(self.inner.as_i16()?)
            }
            /// The value as an `int` in the i32 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i32(&self) -> Result<i32, DataErr> {
                Ok(self.inner.as_i32()?)
            }
            /// The value as an `int` in the i64 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i64(&self) -> Result<i64, DataErr> {
                Ok(self.inner.as_i64()?)
            }
            /// The value as an `int` in the u8 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u8(&self) -> Result<u8, DataErr> {
                Ok(self.inner.as_u8()?)
            }
            /// The value as an `int` in the u16 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u16(&self) -> Result<u16, DataErr> {
                Ok(self.inner.as_u16()?)
            }
            /// The value as an `int` in the u32 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u32(&self) -> Result<u32, DataErr> {
                Ok(self.inner.as_u32()?)
            }
            /// The value as an `int` in the u64 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u64(&self) -> Result<u64, DataErr> {
                Ok(self.inner.as_u64()?)
            }
            /// The value as a `float`; raises `ValueError` when null or not
            /// exactly representable in f32.
            fn as_f32(&self) -> Result<f32, DataErr> {
                Ok(self.inner.as_f32()?)
            }
            /// The value as a `float`; raises `ValueError` when null or not
            /// exactly representable in f64.
            fn as_f64(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f64()?)
            }
            /// The value as a `bool`; raises `ValueError` when null or the value
            /// is not a boolean.
            fn as_bool(&self) -> Result<bool, DataErr> {
                Ok(self.inner.as_bool()?)
            }
            /// The value as a `str`; raises `ValueError` when null or the value
            /// has no string form.
            fn as_str(&self) -> Result<String, DataErr> {
                Ok(self.inner.as_str().map(str::to_string)?)
            }
            /// The value as `bytes`; raises `ValueError` when null or the value
            /// has no byte-sequence form.
            fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
                Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
            }
        }

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[pyclass]
        pub struct $optional {
            inner: yggdryl_data::OptionalScalar<yggdryl_data::$ty, yggdryl_data::$scalar>,
        }

        #[pymethods]
        impl $optional {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_data::OptionalScalar::new(yggdryl_data::$scalar::new(value)),
                }
            }

            /// The null variant.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_data::OptionalScalar::null(),
                }
            }

            /// Whether this scalar holds the null variant.
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The value, or `None` for the null variant.
            fn value(&self) -> Option<$native> {
                self.inner.value().copied()
            }

            /// The inner scalar, when this holds the value variant.
            fn scalar(&self) -> Option<$scalar> {
                self.inner.scalar().map(|scalar| $scalar { inner: *scalar })
            }

            /// The scalar's data type: the logical optional of the value type.
            fn data_type(&self) -> $opt_ty {
                $opt_ty::default()
            }

            /// The value as an `int` in the i8 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i8(&self) -> Result<i8, DataErr> {
                Ok(self.inner.as_i8()?)
            }
            /// The value as an `int` in the i16 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i16(&self) -> Result<i16, DataErr> {
                Ok(self.inner.as_i16()?)
            }
            /// The value as an `int` in the i32 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i32(&self) -> Result<i32, DataErr> {
                Ok(self.inner.as_i32()?)
            }
            /// The value as an `int` in the i64 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_i64(&self) -> Result<i64, DataErr> {
                Ok(self.inner.as_i64()?)
            }
            /// The value as an `int` in the u8 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u8(&self) -> Result<u8, DataErr> {
                Ok(self.inner.as_u8()?)
            }
            /// The value as an `int` in the u16 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u16(&self) -> Result<u16, DataErr> {
                Ok(self.inner.as_u16()?)
            }
            /// The value as an `int` in the u32 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u32(&self) -> Result<u32, DataErr> {
                Ok(self.inner.as_u32()?)
            }
            /// The value as an `int` in the u64 range; raises `ValueError` when
            /// null or not exactly representable.
            fn as_u64(&self) -> Result<u64, DataErr> {
                Ok(self.inner.as_u64()?)
            }
            /// The value as a `float`; raises `ValueError` when null or not
            /// exactly representable in f32.
            fn as_f32(&self) -> Result<f32, DataErr> {
                Ok(self.inner.as_f32()?)
            }
            /// The value as a `float`; raises `ValueError` when null or not
            /// exactly representable in f64.
            fn as_f64(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f64()?)
            }
            /// The value as a `bool`; raises `ValueError` when null or the value
            /// is not a boolean.
            fn as_bool(&self) -> Result<bool, DataErr> {
                Ok(self.inner.as_bool()?)
            }
            /// The value as a `str`; raises `ValueError` when null or the value
            /// has no string form.
            fn as_str(&self) -> Result<String, DataErr> {
                Ok(self.inner.as_str().map(str::to_string)?)
            }
            /// The value as `bytes`; raises `ValueError` when null or the value
            /// has no byte-sequence form.
            fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
                Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
            }
        }
    };
}

int_data_py!(
    Int8,
    Int8Field,
    Int8Scalar,
    OptionalInt8,
    OptionalInt8Field,
    OptionalInt8Scalar,
    i8,
    "int8"
);
int_data_py!(
    Int16,
    Int16Field,
    Int16Scalar,
    OptionalInt16,
    OptionalInt16Field,
    OptionalInt16Scalar,
    i16,
    "int16"
);
int_data_py!(
    Int32,
    Int32Field,
    Int32Scalar,
    OptionalInt32,
    OptionalInt32Field,
    OptionalInt32Scalar,
    i32,
    "int32"
);
int_data_py!(
    Int64,
    Int64Field,
    Int64Scalar,
    OptionalInt64,
    OptionalInt64Field,
    OptionalInt64Scalar,
    i64,
    "int64"
);
int_data_py!(
    UInt8,
    UInt8Field,
    UInt8Scalar,
    OptionalUInt8,
    OptionalUInt8Field,
    OptionalUInt8Scalar,
    u8,
    "uint8"
);
int_data_py!(
    UInt16,
    UInt16Field,
    UInt16Scalar,
    OptionalUInt16,
    OptionalUInt16Field,
    OptionalUInt16Scalar,
    u16,
    "uint16"
);
int_data_py!(
    UInt32,
    UInt32Field,
    UInt32Scalar,
    OptionalUInt32,
    OptionalUInt32Field,
    OptionalUInt32Scalar,
    u32,
    "uint32"
);
int_data_py!(
    UInt64,
    UInt64Field,
    UInt64Scalar,
    OptionalUInt64,
    OptionalUInt64Field,
    OptionalUInt64Scalar,
    u64,
    "uint64"
);

/// Populates the `data` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Union>()?;
    module.add_class::<UnionField>()?;
    module.add_class::<Null>()?;
    module.add_class::<NullField>()?;
    module.add_class::<NullScalar>()?;
    module.add_class::<Binary>()?;
    module.add_class::<BinaryField>()?;
    module.add_class::<BinaryScalar>()?;
    module.add_class::<OptionalBinary>()?;
    module.add_class::<OptionalBinaryField>()?;
    module.add_class::<OptionalBinaryScalar>()?;
    module.add_class::<Int8>()?;
    module.add_class::<Int8Field>()?;
    module.add_class::<Int8Scalar>()?;
    module.add_class::<OptionalInt8>()?;
    module.add_class::<OptionalInt8Field>()?;
    module.add_class::<OptionalInt8Scalar>()?;
    module.add_class::<Int16>()?;
    module.add_class::<Int16Field>()?;
    module.add_class::<Int16Scalar>()?;
    module.add_class::<OptionalInt16>()?;
    module.add_class::<OptionalInt16Field>()?;
    module.add_class::<OptionalInt16Scalar>()?;
    module.add_class::<Int32>()?;
    module.add_class::<Int32Field>()?;
    module.add_class::<Int32Scalar>()?;
    module.add_class::<OptionalInt32>()?;
    module.add_class::<OptionalInt32Field>()?;
    module.add_class::<OptionalInt32Scalar>()?;
    module.add_class::<Int64>()?;
    module.add_class::<Int64Field>()?;
    module.add_class::<Int64Scalar>()?;
    module.add_class::<OptionalInt64>()?;
    module.add_class::<OptionalInt64Field>()?;
    module.add_class::<OptionalInt64Scalar>()?;
    module.add_class::<UInt8>()?;
    module.add_class::<UInt8Field>()?;
    module.add_class::<UInt8Scalar>()?;
    module.add_class::<OptionalUInt8>()?;
    module.add_class::<OptionalUInt8Field>()?;
    module.add_class::<OptionalUInt8Scalar>()?;
    module.add_class::<UInt16>()?;
    module.add_class::<UInt16Field>()?;
    module.add_class::<UInt16Scalar>()?;
    module.add_class::<OptionalUInt16>()?;
    module.add_class::<OptionalUInt16Field>()?;
    module.add_class::<OptionalUInt16Scalar>()?;
    module.add_class::<UInt32>()?;
    module.add_class::<UInt32Field>()?;
    module.add_class::<UInt32Scalar>()?;
    module.add_class::<OptionalUInt32>()?;
    module.add_class::<OptionalUInt32Field>()?;
    module.add_class::<OptionalUInt32Scalar>()?;
    module.add_class::<UInt64>()?;
    module.add_class::<UInt64Field>()?;
    module.add_class::<UInt64Scalar>()?;
    module.add_class::<OptionalUInt64>()?;
    module.add_class::<OptionalUInt64Field>()?;
    module.add_class::<OptionalUInt64Scalar>()?;
    Ok(())
}
