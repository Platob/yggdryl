//! The `yggdryl.dtype` submodule — thin wrappers over the `yggdryl-dtype` crate.
//!
//! Every integer type is exposed as its data type and its logical optional data
//! type (e.g. `Int64`, `OptionalInt64`), alongside `Binary` / `OptionalBinary`,
//! `Null` and `Union` — the same bare names as the Rust crate, the submodule
//! carrying the concern. Data types expose the descriptor surface (`name`,
//! `arrow_format`, widths), the native byte codec, and their defaults
//! (`default_scalar` hands back a `yggdryl.scalar` class).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work), construction of a
//! `Union` from arbitrary child fields (its `UnionFields` is an arrow-schema
//! value — `Union` is reached through an optional data type's `storage()`), the
//! `DataTypeId` classifier (a method-bearing enum the bindings cannot model
//! uniformly), and the generic nested types (`List` / `Map` / `Struct` and the
//! per-family trait pairs), which have no concrete FFI shape yet.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_dtype::{DataType, RawDataType, RawLogical, RawNested, RawUnion};
use yggdryl_scalar::DefaultScalar;

use crate::DataErr;

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id. Reached through a data type's `optional()`
/// (arbitrary child fields stay Rust-only).
#[pyclass]
#[derive(Clone)]
pub struct Union {
    pub(crate) inner: yggdryl_dtype::Union,
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
            yggdryl_dtype::arrow_schema::UnionMode::Sparse => "sparse",
            yggdryl_dtype::arrow_schema::UnionMode::Dense => "dense",
        }
    }
}

/// The Apache Arrow `null` data type: every value is null, with no storage.
#[pyclass]
#[derive(Default)]
pub struct Null {
    pub(crate) inner: yggdryl_dtype::Null,
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

/// The Apache Arrow `binary` data type: a variable-length byte sequence.
#[pyclass]
#[derive(Default)]
pub struct Binary {
    pub(crate) inner: yggdryl_dtype::Binary,
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

    /// The default scalar: a `yggdryl.scalar.Binary` holding `b""`.
    fn default_scalar(&self) -> crate::scalar::Binary {
        crate::scalar::Binary {
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
    pub(crate) inner: yggdryl_dtype::Optional<yggdryl_dtype::Binary>,
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
    fn default_scalar(&self) -> crate::scalar::OptionalBinary {
        crate::scalar::OptionalBinary {
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

/// Generates the two data-type wrappers of one integer type: the data type `$ty`
/// (with the byte codec, defaults and `optional()`) and the logical optional data
/// type `$opt_ty` (over union storage) — each a thin delegation to the
/// `yggdryl-dtype` types.
macro_rules! int_dtype_py {
    ($ty:ident, $opt_ty:ident, $native:ty, $name:literal) => {
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
            fn default_scalar(&self) -> crate::scalar::$ty {
                crate::scalar::$ty {
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
            pub(crate) inner: yggdryl_dtype::Optional<yggdryl_dtype::$ty>,
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
            fn default_scalar(&self) -> crate::scalar::$opt_ty {
                crate::scalar::$opt_ty {
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
    };
}

int_dtype_py!(Int8, OptionalInt8, i8, "int8");
int_dtype_py!(Int16, OptionalInt16, i16, "int16");
int_dtype_py!(Int32, OptionalInt32, i32, "int32");
int_dtype_py!(Int64, OptionalInt64, i64, "int64");
int_dtype_py!(UInt8, OptionalUInt8, u8, "uint8");
int_dtype_py!(UInt16, OptionalUInt16, u16, "uint16");
int_dtype_py!(UInt32, OptionalUInt32, u32, "uint32");
int_dtype_py!(UInt64, OptionalUInt64, u64, "uint64");

/// Populates the `dtype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Union>()?;
    module.add_class::<Null>()?;
    module.add_class::<Binary>()?;
    module.add_class::<OptionalBinary>()?;
    module.add_class::<Int8>()?;
    module.add_class::<OptionalInt8>()?;
    module.add_class::<Int16>()?;
    module.add_class::<OptionalInt16>()?;
    module.add_class::<Int32>()?;
    module.add_class::<OptionalInt32>()?;
    module.add_class::<Int64>()?;
    module.add_class::<OptionalInt64>()?;
    module.add_class::<UInt8>()?;
    module.add_class::<OptionalUInt8>()?;
    module.add_class::<UInt16>()?;
    module.add_class::<OptionalUInt16>()?;
    module.add_class::<UInt32>()?;
    module.add_class::<OptionalUInt32>()?;
    module.add_class::<UInt64>()?;
    module.add_class::<OptionalUInt64>()?;
    Ok(())
}
