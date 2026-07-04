//! The `yggdryl.dtype` submodule — thin wrappers over the `yggdryl-dtype` crate.
//!
//! Every integer and float type is exposed as its data type and its logical
//! optional data type (e.g. `Int64Type`, `OptionalInt64Type`; the `float16`
//! family's native `half::f16` crosses its codec/scalar values as a Python
//! `float`), alongside `BinaryType` / `OptionalBinaryType`, `Utf8Type` /
//! `OptionalUtf8Type` (the `utf8` logical type over binary storage, its value
//! crossing as `str`), `NullType`, `UnionType`, `StructType` (built from a dict
//! mapping field names to example values or dtype instances, resolved through the
//! factory's inference) and its concrete serie type (e.g. `Int64SerieType`, the
//! `list` of `int64` — every integer value type has a buffer-backed serie scalar)
//! — the same suffixed names as the Rust crate, the submodule carrying the
//! concern. Data types expose the descriptor surface (`name`, `arrow_format`,
//! widths), the native byte codec, and — as the model's factory hub — their
//! defaults (`default_scalar`) and their `field` / `scalar` builders (`field`
//! hands back a `yggdryl.field` class, `scalar` and `default_scalar` a
//! `yggdryl.scalar` class).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work), construction of a
//! `UnionType` from arbitrary child fields (its `UnionFields` is an arrow-schema
//! value — `UnionType` is reached through an optional data type's `storage()`),
//! the `DataTypeId` classifier (a method-bearing enum the bindings cannot model
//! uniformly), and the dynamic base nested types and their typed generics
//! (`SerieType` / `TypedSerieType` over a non-integer value type, `MapType` /
//! `TypedMapType`, and the per-family trait pairs), which have no concrete FFI
//! shape yet.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use yggdryl_dtype::{DataType, Logical, Nested, Struct, TypedDataType, Union};
use yggdryl_field::FieldFactory;
use yggdryl_scalar::ScalarFactory;

use crate::DataErr;

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[pyclass]
#[derive(Clone)]
pub struct UnionType {
    pub(crate) inner: yggdryl_dtype::UnionType,
}

#[pymethods]
impl UnionType {
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
            yggdryl_dtype::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_dtype::arrow_schema::UnionMode::Dense => "dense",
        }
    }

    /// A compact type signature for fast debugging (the union's child types).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The Apache Arrow `struct` data type: an ordered set of named child fields,
/// built from a dict declaring each child by example value or dtype instance.
#[pyclass]
#[derive(Clone)]
pub struct StructType {
    pub(crate) inner: yggdryl_dtype::StructType,
}

#[pymethods]
impl StructType {
    /// A struct of the child `fields`: a dict mapping each field name to an
    /// example native value (`int` → `int64`, `bytes` → `binary`, `None` →
    /// `null`, a list of ints → the `int64` serie, a dict → a nested struct) or
    /// a `yggdryl.dtype` class instance; every child field is nullable.
    #[new]
    fn new(fields: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut children = Vec::with_capacity(fields.len());
        for (name, value) in fields.iter() {
            let name = name.extract::<String>().map_err(|_| {
                PyErr::from(DataErr::Message(
                    "cannot build a struct: every dict key must be a str field name".to_string(),
                ))
            })?;
            children.push(yggdryl_dtype::arrow_schema::Field::new(
                name,
                crate::factory::resolve_arrow_dtype(&value)?,
                true,
            ));
        }
        Ok(Self {
            inner: yggdryl_dtype::StructType::new(children.into()),
        })
    }

    /// The type's lowercase name, `"struct"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"+s"`.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A struct has no fixed byte width.
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// A struct has no fixed bit width.
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }

    /// The number of child fields.
    fn child_count(&self) -> usize {
        self.inner.child_count()
    }

    /// The child field names, in declaration order.
    fn field_names(&self) -> Vec<String> {
        self.inner
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect()
    }

    /// A compact type signature for fast debugging (e.g.
    /// `struct<x: int64, y: float64>`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[pyclass]
#[derive(Default)]
pub struct NullType {
    pub(crate) inner: yggdryl_dtype::NullType,
}

#[pymethods]
impl NullType {
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

    /// A compact type signature for fast debugging, `"null"`.
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The Apache Arrow `binary` data type: a variable-length byte sequence.
#[pyclass]
#[derive(Default)]
pub struct BinaryType {
    pub(crate) inner: yggdryl_dtype::BinaryType,
}

#[pymethods]
impl BinaryType {
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

    /// The default scalar: a `yggdryl.scalar.BinaryScalar` holding `b""`.
    fn default_scalar(&self) -> crate::scalar::BinaryScalar {
        crate::scalar::BinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The `binary` field named `name` (nullable by default) — a `yggdryl.field`
    /// class.
    #[pyo3(signature = (name, nullable = true))]
    fn field(&self, name: String, nullable: bool) -> crate::field::BinaryField {
        crate::field::BinaryField {
            inner: self.inner.field(name, nullable),
        }
    }

    /// A `binary` scalar holding `value` — a `yggdryl.scalar` class.
    fn scalar(&self, value: Vec<u8>) -> crate::scalar::BinaryScalar {
        crate::scalar::BinaryScalar {
            inner: self.inner.scalar(value),
        }
    }

    /// The logical optional of this type (stored as the null-or-value union).
    fn optional(&self) -> OptionalBinaryType {
        OptionalBinaryType::default()
    }

    /// A compact type signature for fast debugging, `"binary"`.
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The logical optional of `binary`: a value, or null — stored as the
/// null-or-`binary` union.
#[pyclass]
#[derive(Default)]
pub struct OptionalBinaryType {
    pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::BinaryType>,
}

#[pymethods]
impl OptionalBinaryType {
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
    fn value_type(&self) -> BinaryType {
        BinaryType::default()
    }

    /// The physical storage: the sparse null-or-value union.
    fn storage(&self) -> UnionType {
        UnionType {
            inner: self.inner.storage().clone(),
        }
    }

    /// The default native value: the value type's default, `b""`.
    fn default_value<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.default_value())
    }

    /// The default scalar: the null variant (the scalar models nullness).
    fn default_scalar(&self) -> crate::scalar::OptionalBinaryScalar {
        crate::scalar::OptionalBinaryScalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The optional-`binary` field named `name` (nullable by default) — a
    /// `yggdryl.field` class.
    #[pyo3(signature = (name, nullable = true))]
    fn field(&self, name: String, nullable: bool) -> crate::field::OptionalBinaryField {
        crate::field::OptionalBinaryField {
            inner: self.inner.field(name, nullable),
        }
    }

    /// An optional-`binary` scalar holding the value variant `value` — a
    /// `yggdryl.scalar` class.
    fn scalar(&self, value: Vec<u8>) -> crate::scalar::OptionalBinaryScalar {
        crate::scalar::OptionalBinaryScalar {
            inner: self.inner.scalar(value),
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

    /// A compact type signature for fast debugging (e.g. `optional<binary>`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The Apache Arrow `utf8` data type: a variable-length UTF-8 string. A **logical**
/// type over `binary` storage (a string *is* bytes, reinterpreted as text), so its
/// byte codec is UTF-8 and validates on the way back; the core `Utf8Buffer` stays
/// Rust-only, so the value crosses as Python `str`.
#[pyclass]
#[derive(Default)]
pub struct Utf8Type {
    pub(crate) inner: yggdryl_dtype::Utf8Type,
}

#[pymethods]
impl Utf8Type {
    /// The `utf8` data type.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// The type's lowercase name, `"utf8"`.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The Arrow C Data Interface format string, `"u"`.
    fn arrow_format(&self) -> String {
        self.inner.arrow_format()
    }

    /// A string value has no fixed byte width.
    fn byte_width(&self) -> Option<usize> {
        self.inner.byte_width()
    }

    /// A string value has no fixed bit width.
    fn bit_width(&self) -> Option<usize> {
        self.inner.bit_width()
    }

    /// Serialize a native value into its Arrow bytes — the string's UTF-8 bytes.
    fn native_to_bytes<'py>(&self, py: Python<'py>, value: String) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
    }

    /// Deserialize Arrow bytes into a native value — the exact inverse of
    /// `native_to_bytes`; non-UTF-8 bytes raise `ValueError`.
    fn native_from_bytes(&self, bytes: &[u8]) -> Result<String, DataErr> {
        Ok(self.inner.native_from_bytes(bytes)?)
    }

    /// The type's default native value, `""`.
    fn default_value(&self) -> String {
        self.inner.default_value()
    }

    /// The default scalar: a `yggdryl.scalar.Utf8Scalar` holding `""`.
    fn default_scalar(&self) -> crate::scalar::Utf8Scalar {
        crate::scalar::Utf8Scalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The `utf8` field named `name` (nullable by default) — a `yggdryl.field`
    /// class.
    #[pyo3(signature = (name, nullable = true))]
    fn field(&self, name: String, nullable: bool) -> crate::field::Utf8Field {
        crate::field::Utf8Field {
            inner: self.inner.field(name, nullable),
        }
    }

    /// A `utf8` scalar holding `value` — a `yggdryl.scalar` class.
    fn scalar(&self, value: String) -> crate::scalar::Utf8Scalar {
        crate::scalar::Utf8Scalar {
            inner: self.inner.scalar(value),
        }
    }

    /// The logical optional of this type (stored as the null-or-value union).
    fn optional(&self) -> OptionalUtf8Type {
        OptionalUtf8Type::default()
    }

    /// A compact type signature for fast debugging, `"utf8"`.
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// The logical optional of `utf8`: a value, or null — stored as the null-or-`utf8`
/// union.
#[pyclass]
#[derive(Default)]
pub struct OptionalUtf8Type {
    pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::Utf8Type>,
}

#[pymethods]
impl OptionalUtf8Type {
    /// The optional `utf8` data type.
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
    fn value_type(&self) -> Utf8Type {
        Utf8Type::default()
    }

    /// The physical storage: the sparse null-or-value union.
    fn storage(&self) -> UnionType {
        UnionType {
            inner: self.inner.storage().clone(),
        }
    }

    /// The default native value: the value type's default, `""`.
    fn default_value(&self) -> String {
        self.inner.default_value()
    }

    /// The default scalar: the null variant (the scalar models nullness).
    fn default_scalar(&self) -> crate::scalar::OptionalUtf8Scalar {
        crate::scalar::OptionalUtf8Scalar {
            inner: self.inner.default_scalar(),
        }
    }

    /// The optional-`utf8` field named `name` (nullable by default) — a
    /// `yggdryl.field` class.
    #[pyo3(signature = (name, nullable = true))]
    fn field(&self, name: String, nullable: bool) -> crate::field::OptionalUtf8Field {
        crate::field::OptionalUtf8Field {
            inner: self.inner.field(name, nullable),
        }
    }

    /// An optional-`utf8` scalar holding the value variant `value` — a
    /// `yggdryl.scalar` class.
    fn scalar(&self, value: String) -> crate::scalar::OptionalUtf8Scalar {
        crate::scalar::OptionalUtf8Scalar {
            inner: self.inner.scalar(value),
        }
    }

    /// Serialize a native value into its Arrow bytes — the value type's codec.
    fn native_to_bytes<'py>(&self, py: Python<'py>, value: String) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.native_to_bytes(&value))
    }

    /// Deserialize Arrow bytes into a native value — the exact inverse of
    /// `native_to_bytes`; non-UTF-8 bytes raise `ValueError`.
    fn native_from_bytes(&self, bytes: &[u8]) -> Result<String, DataErr> {
        Ok(self.inner.native_from_bytes(bytes)?)
    }

    /// A compact type signature for fast debugging (e.g. `optional<utf8>`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `repr(x)` shows the pretty form.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` signature — `print(x)` shows the pretty form.
    fn __str__(&self) -> String {
        self.inner.display()
    }
}

/// Generates the two data-type wrappers of one integer type: the data type `$ty`
/// (with the byte codec, defaults, `field` / `scalar` factories and `optional()`)
/// and the logical optional data type `$opt_ty` (over union storage) — each a thin
/// delegation to the `yggdryl-dtype` types. `$field` / `$opt_field` name the
/// `yggdryl.field` classes the factories return, `$scalar` / `$opt_scalar` the
/// `yggdryl.scalar` classes.
macro_rules! int_dtype_py {
    ($ty:ident, $opt_ty:ident, $field:ident, $opt_field:ident, $scalar:ident, $opt_scalar:ident, $native:ty, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::$ty,
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

            /// The default scalar: a `yggdryl.scalar` class holding `0`.
            fn default_scalar(&self) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The field of this type named `name` (nullable by default) — a
            /// `yggdryl.field` class.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable),
                }
            }

            /// A scalar of this type holding `value` — a `yggdryl.scalar` class.
            fn scalar(&self, value: $native) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.scalar(value),
                }
            }

            /// The logical optional of this type (stored as the null-or-value
            /// union).
            fn optional(&self) -> $opt_ty {
                $opt_ty::default()
            }

            /// A compact type signature for fast debugging (e.g. `int64`,
            /// `list<int64>`, `optional<int64>`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }

        #[doc = concat!("The logical optional of `", $name, "`: a value, or null — stored as the null-or-`", $name, "` union.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::$ty>,
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
            fn storage(&self) -> UnionType {
                UnionType {
                    inner: self.inner.storage().clone(),
                }
            }

            /// The default native value: the value type's default, `0`.
            fn default_value(&self) -> $native {
                self.inner.default_value()
            }

            /// The default scalar: the null variant (the scalar models nullness).
            fn default_scalar(&self) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The optional field of this type named `name` (nullable by default) — a
            /// `yggdryl.field` class.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$opt_field {
                crate::field::$opt_field {
                    inner: self.inner.field(name, nullable),
                }
            }

            /// An optional scalar holding the value variant `value` — a
            /// `yggdryl.scalar` class.
            fn scalar(&self, value: $native) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.scalar(value),
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

            /// A compact type signature for fast debugging (e.g. `optional<int64>`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }
    };
}

int_dtype_py!(
    Int8Type,
    OptionalInt8Type,
    Int8Field,
    OptionalInt8Field,
    Int8Scalar,
    OptionalInt8Scalar,
    i8,
    "int8"
);
int_dtype_py!(
    Int16Type,
    OptionalInt16Type,
    Int16Field,
    OptionalInt16Field,
    Int16Scalar,
    OptionalInt16Scalar,
    i16,
    "int16"
);
int_dtype_py!(
    Int32Type,
    OptionalInt32Type,
    Int32Field,
    OptionalInt32Field,
    Int32Scalar,
    OptionalInt32Scalar,
    i32,
    "int32"
);
int_dtype_py!(
    Int64Type,
    OptionalInt64Type,
    Int64Field,
    OptionalInt64Field,
    Int64Scalar,
    OptionalInt64Scalar,
    i64,
    "int64"
);
int_dtype_py!(
    UInt8Type,
    OptionalUInt8Type,
    UInt8Field,
    OptionalUInt8Field,
    UInt8Scalar,
    OptionalUInt8Scalar,
    u8,
    "uint8"
);
int_dtype_py!(
    UInt16Type,
    OptionalUInt16Type,
    UInt16Field,
    OptionalUInt16Field,
    UInt16Scalar,
    OptionalUInt16Scalar,
    u16,
    "uint16"
);
int_dtype_py!(
    UInt32Type,
    OptionalUInt32Type,
    UInt32Field,
    OptionalUInt32Field,
    UInt32Scalar,
    OptionalUInt32Scalar,
    u32,
    "uint32"
);
int_dtype_py!(
    UInt64Type,
    OptionalUInt64Type,
    UInt64Field,
    OptionalUInt64Field,
    UInt64Scalar,
    OptionalUInt64Scalar,
    u64,
    "uint64"
);
int_dtype_py!(
    Float32Type,
    OptionalFloat32Type,
    Float32Field,
    OptionalFloat32Field,
    Float32Scalar,
    OptionalFloat32Scalar,
    f32,
    "float32"
);
int_dtype_py!(
    Float64Type,
    OptionalFloat64Type,
    Float64Field,
    OptionalFloat64Field,
    Float64Scalar,
    OptionalFloat64Scalar,
    f64,
    "float64"
);

/// Generates the two `float16` data-type wrappers — the data type `$ty` and the
/// logical optional `$opt_ty` — mirroring [`int_dtype_py!`], except the native
/// `half::f16` does not cross the FFI boundary: it crosses as a Python `float`
/// (f64), so the byte codec, `default_value` and `scalar` factory narrow the
/// incoming `f64` to `f16` and widen `f16` back on the way out. `$field` /
/// `$opt_field` name the `yggdryl.field` classes, `$scalar` / `$opt_scalar` the
/// `yggdryl.scalar` classes.
macro_rules! float16_dtype_py {
    ($ty:ident, $opt_ty:ident, $field:ident, $opt_field:ident, $scalar:ident, $opt_scalar:ident, $name:literal) => {
        #[doc = concat!("The Apache Arrow `", $name, "` data type (native `half::f16`, crossing as a Python `float`).")]
        #[pyclass]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::$ty,
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

            /// Serialize a native value into its little-endian Arrow bytes (the
            /// Python `float` narrowed to f16).
            fn native_to_bytes<'py>(&self, py: Python<'py>, value: f64) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(
                    py,
                    &self
                        .inner
                        .native_to_bytes(&yggdryl_dtype::half::f16::from_f64(value)),
                )
            }

            /// Deserialize little-endian Arrow bytes into a native value (widened to
            /// a Python `float`) — the exact inverse of `native_to_bytes`; the wrong
            /// length raises `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<f64, DataErr> {
                Ok(self.inner.native_from_bytes(bytes)?.to_f64())
            }

            /// The type's default native value, `0.0`.
            fn default_value(&self) -> f64 {
                self.inner.default_value().to_f64()
            }

            /// The default scalar: a `yggdryl.scalar` class holding `0.0`.
            fn default_scalar(&self) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The field of this type named `name` (nullable by default) — a
            /// `yggdryl.field` class.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable),
                }
            }

            /// A scalar of this type holding `value` (narrowed to f16) — a
            /// `yggdryl.scalar` class.
            fn scalar(&self, value: f64) -> crate::scalar::$scalar {
                crate::scalar::$scalar {
                    inner: self.inner.scalar(yggdryl_dtype::half::f16::from_f64(value)),
                }
            }

            /// The logical optional of this type (stored as the null-or-value
            /// union).
            fn optional(&self) -> $opt_ty {
                $opt_ty::default()
            }

            /// A compact type signature for fast debugging (e.g. `float16`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }

        #[doc = concat!("The logical optional of `", $name, "`: a value, or null — stored as the null-or-`", $name, "` union.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_dtype::TypedOptionalType<yggdryl_dtype::$ty>,
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
            fn storage(&self) -> UnionType {
                UnionType {
                    inner: self.inner.storage().clone(),
                }
            }

            /// The default native value: the value type's default, `0.0`.
            fn default_value(&self) -> f64 {
                self.inner.default_value().to_f64()
            }

            /// The default scalar: the null variant (the scalar models nullness).
            fn default_scalar(&self) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.default_scalar(),
                }
            }

            /// The optional field of this type named `name` (nullable by default) — a
            /// `yggdryl.field` class.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$opt_field {
                crate::field::$opt_field {
                    inner: self.inner.field(name, nullable),
                }
            }

            /// An optional scalar holding the value variant `value` (narrowed to
            /// f16) — a `yggdryl.scalar` class.
            fn scalar(&self, value: f64) -> crate::scalar::$opt_scalar {
                crate::scalar::$opt_scalar {
                    inner: self.inner.scalar(yggdryl_dtype::half::f16::from_f64(value)),
                }
            }

            /// Serialize a native value into its little-endian Arrow bytes (the
            /// Python `float` narrowed to f16) — the value type's codec.
            fn native_to_bytes<'py>(&self, py: Python<'py>, value: f64) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(
                    py,
                    &self
                        .inner
                        .native_to_bytes(&yggdryl_dtype::half::f16::from_f64(value)),
                )
            }

            /// Deserialize little-endian Arrow bytes into a native value (widened to
            /// a Python `float`) — the exact inverse of `native_to_bytes`; the wrong
            /// length raises `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<f64, DataErr> {
                Ok(self.inner.native_from_bytes(bytes)?.to_f64())
            }

            /// A compact type signature for fast debugging (e.g. `optional<float16>`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }
    };
}

float16_dtype_py!(
    Float16Type,
    OptionalFloat16Type,
    Float16Field,
    OptionalFloat16Field,
    Float16Scalar,
    OptionalFloat16Scalar,
    "float16"
);

/// Generates the concrete serie data type of one integer value type: `$ty`, the
/// Apache Arrow `list` of `$name` (single nullable `"item"` child) — a thin
/// delegation to `yggdryl_dtype::TypedSerieType<$value_ty>`. `$field` / `$serie` name
/// the `yggdryl.field` / `yggdryl.scalar` classes the factories return.
macro_rules! int_serie_dtype_py {
    ($ty:ident, $value_ty:ident, $field:ident, $serie:ident, $native:ty, $name:literal) => {
        #[doc = concat!("The Apache Arrow `list` of `", $name, "`: a variable-length sequence of `", $name, "`")]
        #[doc = concat!("(single nullable `\"item\"` child), with a buffer-backed serie scalar (`yggdryl.scalar.", stringify!($serie), "`).")]
        #[pyclass]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::TypedSerieType<yggdryl_dtype::$value_ty>,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("The `list` of `", $name, "` data type.")]
            #[new]
            fn new() -> Self {
                Self::default()
            }

            /// The type's lowercase name, `"list"`.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string, `"+l"`.
            fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// A serie has no fixed byte width.
            fn byte_width(&self) -> Option<usize> {
                self.inner.byte_width()
            }

            /// A serie has no fixed bit width.
            fn bit_width(&self) -> Option<usize> {
                self.inner.bit_width()
            }

            /// The number of child fields, `1` (the `"item"` field).
            fn child_count(&self) -> usize {
                self.inner.child_count()
            }

            #[doc = concat!("The value type this serie sequences, `", $name, "`.")]
            fn value_type(&self) -> $value_ty {
                $value_ty::default()
            }

            /// Serialize a native serie into its Arrow bytes — the value type's codec,
            /// concatenated per element.
            fn native_to_bytes<'py>(
                &self,
                py: Python<'py>,
                values: Vec<$native>,
            ) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.native_to_bytes(&values))
            }

            /// Deserialize Arrow bytes into a native serie — the exact inverse of
            /// `native_to_bytes`; a length that is not a whole number of elements raises
            /// `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<Vec<$native>, DataErr> {
                Ok(self.inner.native_from_bytes(bytes)?)
            }

            /// The type's default native value, the empty serie.
            fn default_value(&self) -> Vec<$native> {
                self.inner.default_value()
            }

            #[doc = concat!("The default scalar: a `yggdryl.scalar.", stringify!($serie), "` holding the empty serie.")]
            fn default_scalar(&self) -> crate::scalar::$serie {
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::default(),
                }
            }

            /// The field of this type named `name` (nullable by default) — a
            #[doc = concat!("`yggdryl.field.", stringify!($field), "`.")]
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable),
                }
            }

            #[doc = concat!("A `yggdryl.scalar.", stringify!($serie), "` holding the native serie `values`.")]
            fn scalar(&self, values: Vec<$native>) -> crate::scalar::$serie {
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::from(values),
                }
            }

            /// A compact type signature for fast debugging (e.g. `list<int64>`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }
    };
}

int_serie_dtype_py!(
    Int8SerieType,
    Int8Type,
    Int8SerieField,
    Int8Serie,
    i8,
    "int8"
);
int_serie_dtype_py!(
    Int16SerieType,
    Int16Type,
    Int16SerieField,
    Int16Serie,
    i16,
    "int16"
);
int_serie_dtype_py!(
    Int32SerieType,
    Int32Type,
    Int32SerieField,
    Int32Serie,
    i32,
    "int32"
);
int_serie_dtype_py!(
    Int64SerieType,
    Int64Type,
    Int64SerieField,
    Int64Serie,
    i64,
    "int64"
);
int_serie_dtype_py!(
    UInt8SerieType,
    UInt8Type,
    UInt8SerieField,
    UInt8Serie,
    u8,
    "uint8"
);
int_serie_dtype_py!(
    UInt16SerieType,
    UInt16Type,
    UInt16SerieField,
    UInt16Serie,
    u16,
    "uint16"
);
int_serie_dtype_py!(
    UInt32SerieType,
    UInt32Type,
    UInt32SerieField,
    UInt32Serie,
    u32,
    "uint32"
);
int_serie_dtype_py!(
    UInt64SerieType,
    UInt64Type,
    UInt64SerieField,
    UInt64Serie,
    u64,
    "uint64"
);
int_serie_dtype_py!(
    Float32SerieType,
    Float32Type,
    Float32SerieField,
    Float32Serie,
    f32,
    "float32"
);
int_serie_dtype_py!(
    Float64SerieType,
    Float64Type,
    Float64SerieField,
    Float64Serie,
    f64,
    "float64"
);

/// Generates the concrete `float16` serie data type `$ty`, mirroring
/// [`int_serie_dtype_py!`], except the native `half::f16` does not cross the FFI
/// boundary: the byte codec, `default_value` and `scalar` factory narrow each
/// incoming Python `float` (f64) to `f16` and widen `f16` back on the way out.
/// `$field` / `$serie` name the `yggdryl.field` / `yggdryl.scalar` classes.
macro_rules! float16_serie_dtype_py {
    ($ty:ident, $value_ty:ident, $field:ident, $serie:ident, $name:literal) => {
        #[doc = concat!("The Apache Arrow `list` of `", $name, "`: a variable-length sequence of `", $name, "`")]
        #[doc = concat!("(single nullable `\"item\"` child), with a buffer-backed serie scalar (`yggdryl.scalar.", stringify!($serie), "`).")]
        #[pyclass]
        #[derive(Default)]
        pub struct $ty {
            pub(crate) inner: yggdryl_dtype::TypedSerieType<yggdryl_dtype::$value_ty>,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("The `list` of `", $name, "` data type.")]
            #[new]
            fn new() -> Self {
                Self::default()
            }

            /// The type's lowercase name, `"list"`.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The Arrow C Data Interface format string, `"+l"`.
            fn arrow_format(&self) -> String {
                self.inner.arrow_format()
            }

            /// A serie has no fixed byte width.
            fn byte_width(&self) -> Option<usize> {
                self.inner.byte_width()
            }

            /// A serie has no fixed bit width.
            fn bit_width(&self) -> Option<usize> {
                self.inner.bit_width()
            }

            /// The number of child fields, `1` (the `"item"` field).
            fn child_count(&self) -> usize {
                self.inner.child_count()
            }

            #[doc = concat!("The value type this serie sequences, `", $name, "`.")]
            fn value_type(&self) -> $value_ty {
                $value_ty::default()
            }

            /// Serialize a native serie into its Arrow bytes — the value type's codec
            /// (each Python `float` narrowed to f16), concatenated per element.
            fn native_to_bytes<'py>(&self, py: Python<'py>, values: Vec<f64>) -> Bound<'py, PyBytes> {
                let values = values
                    .into_iter()
                    .map(yggdryl_dtype::half::f16::from_f64)
                    .collect::<Vec<_>>();
                PyBytes::new_bound(py, &self.inner.native_to_bytes(&values))
            }

            /// Deserialize Arrow bytes into a native serie (each element widened to a
            /// Python `float`) — the exact inverse of `native_to_bytes`; a length that
            /// is not a whole number of elements raises `ValueError`.
            fn native_from_bytes(&self, bytes: &[u8]) -> Result<Vec<f64>, DataErr> {
                Ok(self
                    .inner
                    .native_from_bytes(bytes)?
                    .iter()
                    .map(|value| value.to_f64())
                    .collect())
            }

            /// The type's default native value, the empty serie.
            fn default_value(&self) -> Vec<f64> {
                self.inner
                    .default_value()
                    .iter()
                    .map(|value| value.to_f64())
                    .collect()
            }

            #[doc = concat!("The default scalar: a `yggdryl.scalar.", stringify!($serie), "` holding the empty serie.")]
            fn default_scalar(&self) -> crate::scalar::$serie {
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::default(),
                }
            }

            /// The field of this type named `name` (nullable by default) — a
            #[doc = concat!("`yggdryl.field.", stringify!($field), "`.")]
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: String, nullable: bool) -> crate::field::$field {
                crate::field::$field {
                    inner: self.inner.field(name, nullable),
                }
            }

            #[doc = concat!("A `yggdryl.scalar.", stringify!($serie), "` holding the native serie `values` (each narrowed to f16).")]
            fn scalar(&self, values: Vec<f64>) -> crate::scalar::$serie {
                crate::scalar::$serie {
                    inner: yggdryl_scalar::$serie::from(
                        values
                            .into_iter()
                            .map(yggdryl_dtype::half::f16::from_f64)
                            .collect::<Vec<_>>(),
                    ),
                }
            }

            /// A compact type signature for fast debugging (`list<float16>`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `repr(x)` shows the pretty form.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` signature — `print(x)` shows the pretty form.
            fn __str__(&self) -> String {
                self.inner.display()
            }
        }
    };
}

float16_serie_dtype_py!(
    Float16SerieType,
    Float16Type,
    Float16SerieField,
    Float16Serie,
    "float16"
);

/// Populates the `dtype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<UnionType>()?;
    module.add_class::<StructType>()?;
    module.add_class::<NullType>()?;
    module.add_class::<BinaryType>()?;
    module.add_class::<OptionalBinaryType>()?;
    module.add_class::<Utf8Type>()?;
    module.add_class::<OptionalUtf8Type>()?;
    module.add_class::<Int8Type>()?;
    module.add_class::<OptionalInt8Type>()?;
    module.add_class::<Int16Type>()?;
    module.add_class::<OptionalInt16Type>()?;
    module.add_class::<Int32Type>()?;
    module.add_class::<OptionalInt32Type>()?;
    module.add_class::<Int64Type>()?;
    module.add_class::<OptionalInt64Type>()?;
    module.add_class::<UInt8Type>()?;
    module.add_class::<OptionalUInt8Type>()?;
    module.add_class::<UInt16Type>()?;
    module.add_class::<OptionalUInt16Type>()?;
    module.add_class::<UInt32Type>()?;
    module.add_class::<OptionalUInt32Type>()?;
    module.add_class::<UInt64Type>()?;
    module.add_class::<OptionalUInt64Type>()?;
    module.add_class::<Float16Type>()?;
    module.add_class::<OptionalFloat16Type>()?;
    module.add_class::<Float32Type>()?;
    module.add_class::<OptionalFloat32Type>()?;
    module.add_class::<Float64Type>()?;
    module.add_class::<OptionalFloat64Type>()?;
    module.add_class::<Int8SerieType>()?;
    module.add_class::<Int16SerieType>()?;
    module.add_class::<Int32SerieType>()?;
    module.add_class::<Int64SerieType>()?;
    module.add_class::<UInt8SerieType>()?;
    module.add_class::<UInt16SerieType>()?;
    module.add_class::<UInt32SerieType>()?;
    module.add_class::<UInt64SerieType>()?;
    module.add_class::<Float16SerieType>()?;
    module.add_class::<Float32SerieType>()?;
    module.add_class::<Float64SerieType>()?;
    Ok(())
}
