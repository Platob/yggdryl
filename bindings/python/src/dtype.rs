//! The `yggdryl.dtype` submodule — thin wrappers over the `yggdryl-dtype` crate.
//!
//! Every integer type is exposed as its data type and its logical optional data
//! type (e.g. `Int64Type`, `OptionalInt64Type`), alongside `BinaryType` /
//! `OptionalBinaryType`, `NullType` and `UnionType` — the same suffixed names as
//! the Rust crate, the submodule carrying the concern. Data types expose the
//! descriptor surface (`name`, `arrow_format`, widths), the native byte codec,
//! and — as the model's factory hub — their defaults (`default_scalar`) and their
//! `field` / `scalar` builders (`field` hands back a `yggdryl.field` class,
//! `scalar` and `default_scalar` a `yggdryl.scalar` class).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work), construction of a
//! `UnionType` from arbitrary child fields (its `UnionFields` is an arrow-schema
//! value — `UnionType` is reached through an optional data type's `storage()`),
//! the `DataTypeId` classifier (a method-bearing enum the bindings cannot model
//! uniformly), and the generic nested types (`ListType` / `MapType` / `StructType`
//! and the per-family trait pairs), which have no concrete FFI shape yet.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_dtype::{DataType, Logical, Nested, TypedDataType, Union};
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
}

/// The logical optional of `binary`: a value, or null — stored as the
/// null-or-`binary` union.
#[pyclass]
#[derive(Default)]
pub struct OptionalBinaryType {
    pub(crate) inner: yggdryl_dtype::OptionalType<yggdryl_dtype::BinaryType>,
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
        }

        #[doc = concat!("The logical optional of `", $name, "`: a value, or null — stored as the null-or-`", $name, "` union.")]
        #[pyclass]
        #[derive(Default)]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_dtype::OptionalType<yggdryl_dtype::$ty>,
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

/// Populates the `dtype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<UnionType>()?;
    module.add_class::<NullType>()?;
    module.add_class::<BinaryType>()?;
    module.add_class::<OptionalBinaryType>()?;
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
    Ok(())
}
