//! The `yggdryl.scalar` submodule — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer type is exposed as its scalar and its null-or-value optional
//! scalar (e.g. `Int64`, `OptionalInt64`), alongside `Binary` / `OptionalBinary`
//! (whose value is held as a core positioned-IO `ByteBuffer` — `to_io()` hands
//! one back) and `Null` — the same bare names as the Rust crate, the submodule
//! carrying the concern. Scalars expose the `as_*` accessors with the core
//! contract: the value when the target represents it exactly, or a raised
//! `ValueError` naming the fix (strings and bytes cross the FFI boundary as new
//! Python objects, so the Rust-side "borrow, never copy" guarantee applies up to
//! that boundary copy). Optional scalars adapt construction to idioms: they are
//! built straight from the native value (`OptionalInt64(42)`), the inner scalar
//! being an implementation detail reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `DefaultScalar` traits (generic Rust bounds; the bindings reach defaults
//! through a data type's `default_scalar()`), and the nested scalars — the
//! generic `Serie` / `Map` / `Struct` and the buffer-backed `Int64Serie` (whose
//! zero-copy Arrow buffers await C Data Interface interop) — which have no
//! concrete FFI shape yet.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_scalar::RawScalar;

use crate::DataErr;

/// Reads `as_str` through the optional charset name — `"utf8"` (the default) or
/// `"latin1"` — shared by every scalar class.
fn as_str_with<D: yggdryl_dtype::RawDataType, S: RawScalar<D>>(
    scalar: &S,
    charset: Option<&str>,
) -> Result<String, DataErr> {
    let decoded = match charset {
        None | Some("utf8") => scalar.as_str(None),
        Some("latin1") => scalar.as_str(Some(&yggdryl_core::Latin1)),
        Some(other) => {
            return Err(DataErr::Message(format!(
                "unknown charset \"{other}\"; expected \"utf8\" or \"latin1\""
            )))
        }
    };
    Ok(decoded?.into_owned())
}

/// The `null` scalar: always null, holding no value.
#[pyclass]
#[derive(Default)]
pub struct Null {
    pub(crate) inner: yggdryl_scalar::Null,
}

#[pymethods]
impl Null {
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
    fn data_type(&self) -> crate::dtype::Null {
        crate::dtype::Null::default()
    }
}

/// A single, possibly-null `binary` value, holding its bytes as a core
/// positioned-IO `ByteBuffer` (`to_io()` hands one back).
#[pyclass]
pub struct Binary {
    pub(crate) inner: yggdryl_scalar::Binary,
}

#[pymethods]
impl Binary {
    /// A `binary` scalar holding `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_scalar::Binary::new(value),
        }
    }

    /// A null `binary` scalar.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Binary::null(),
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
    fn data_type(&self) -> crate::dtype::Binary {
        crate::dtype::Binary::default()
    }

    /// The value as a core IO `ByteBuffer` (`yggdryl.core`), ready for
    /// positioned reads and the cursor / slice adapters, or `None` when null
    /// (the bytes cross the FFI boundary as one copy).
    fn to_io(&self) -> Option<crate::core::ByteBuffer> {
        self.inner
            .io()
            .map(|io| crate::core::ByteBuffer::from_inner(io.clone()))
    }

    /// The value as a full-window core IO `ByteBufferSlice` (`yggdryl.core`) —
    /// window-relative positioned reads — or `None` when null (one copy at the
    /// FFI boundary).
    fn to_io_slice(&self) -> Option<crate::core::ByteBufferSlice> {
        self.inner
            .clone()
            .into_io_slice()
            .map(crate::core::ByteBufferSlice::from_inner)
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
    /// The value as a `str`; `charset` picks the decoder (`"utf8"`, the
    /// default, or `"latin1"`); raises `ValueError` when null or not
    /// decodable.
    #[pyo3(signature = (charset = None))]
    fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
        as_str_with(&self.inner, charset)
    }
    /// The value as `bytes` — the native type; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }
}

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[pyclass]
pub struct OptionalBinary {
    pub(crate) inner: yggdryl_scalar::Optional<yggdryl_dtype::Binary, yggdryl_scalar::Binary>,
}

#[pymethods]
impl OptionalBinary {
    /// A scalar holding the `binary` value variant `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_scalar::Optional::new(yggdryl_scalar::Binary::new(value)),
        }
    }

    /// The null variant.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Optional::null(),
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
    fn scalar(&self) -> Option<Binary> {
        self.inner.scalar().map(|scalar| Binary {
            inner: scalar.clone(),
        })
    }

    /// The scalar's data type: the logical optional of the value type.
    fn data_type(&self) -> crate::dtype::OptionalBinary {
        crate::dtype::OptionalBinary::default()
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
    /// The value as a `str`; `charset` picks the decoder (`"utf8"`, the
    /// default, or `"latin1"`); raises `ValueError` when null or not
    /// decodable.
    #[pyo3(signature = (charset = None))]
    fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
        as_str_with(&self.inner, charset)
    }
    /// The value as `bytes` — the native type; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }
}

/// Generates the two scalar wrappers of one integer type: the scalar `$ty` and
/// the null-or-value `$opt_ty` — each a thin delegation to the `yggdryl-scalar`
/// types, with the `as_*` accessors on both.
macro_rules! int_scalar_py {
    ($ty:ident, $opt_ty:ident, $native:ty, $name:literal) => {
        #[doc = concat!("A single, possibly-null `", $name, "` value.")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("A `", $name, "` scalar holding `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::new(value),
                }
            }

            #[doc = concat!("A null `", $name, "` scalar.")]
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::null(),
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
            fn data_type(&self) -> crate::dtype::$ty {
                crate::dtype::$ty::default()
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
            /// The value as a `str`; `charset` picks the decoder (`"utf8"`, the
            /// default, or `"latin1"`); raises `ValueError` when null or not
            /// decodable.
            #[pyo3(signature = (charset = None))]
            fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
                as_str_with(&self.inner, charset)
            }
            /// The value as `bytes`; raises `ValueError` when null or the value
            /// has no byte-sequence form.
            fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
                Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
            }
        }

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[pyclass]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_scalar::Optional<yggdryl_dtype::$ty, yggdryl_scalar::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_scalar::Optional::new(yggdryl_scalar::$ty::new(value)),
                }
            }

            /// The null variant.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::Optional::null(),
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
            fn scalar(&self) -> Option<$ty> {
                self.inner.scalar().map(|scalar| $ty { inner: *scalar })
            }

            /// The scalar's data type: the logical optional of the value type.
            fn data_type(&self) -> crate::dtype::$opt_ty {
                crate::dtype::$opt_ty::default()
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
            /// The value as a `str`; `charset` picks the decoder (`"utf8"`, the
            /// default, or `"latin1"`); raises `ValueError` when null or not
            /// decodable.
            #[pyo3(signature = (charset = None))]
            fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
                as_str_with(&self.inner, charset)
            }
            /// The value as `bytes`; raises `ValueError` when null or the value
            /// has no byte-sequence form.
            fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
                Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
            }
        }
    };
}

int_scalar_py!(Int8, OptionalInt8, i8, "int8");
int_scalar_py!(Int16, OptionalInt16, i16, "int16");
int_scalar_py!(Int32, OptionalInt32, i32, "int32");
int_scalar_py!(Int64, OptionalInt64, i64, "int64");
int_scalar_py!(UInt8, OptionalUInt8, u8, "uint8");
int_scalar_py!(UInt16, OptionalUInt16, u16, "uint16");
int_scalar_py!(UInt32, OptionalUInt32, u32, "uint32");
int_scalar_py!(UInt64, OptionalUInt64, u64, "uint64");

/// Populates the `scalar` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
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
