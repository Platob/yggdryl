//! The `yggdryl.scalar` submodule — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer type is exposed as its scalar and its null-or-value optional
//! scalar (e.g. `Int64Scalar`, `OptionalInt64Scalar`), alongside `BinaryScalar` /
//! `OptionalBinaryScalar` (whose value is held as a core positioned-IO
//! `ByteBuffer` — `to_io()` hands one back), `NullScalar` and its serie scalar
//! (e.g. `Int64Serie`, the buffer-backed `list` of `int64`) — the same suffixed
//! names as the Rust crate, the submodule carrying the concern. Scalars expose the
//! `as_*` accessors with the core contract: the value when the target represents
//! it exactly, or a raised `ValueError` naming the fix (strings and bytes cross
//! the FFI boundary as new Python objects, so the Rust-side "borrow, never copy"
//! guarantee applies up to that boundary copy). Optional scalars adapt
//! construction to idioms: they are built straight from the native value
//! (`OptionalInt64Scalar(42)`), the inner scalar being an implementation detail
//! reachable through `scalar()`.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `ScalarFactory` traits (generic Rust bounds; the bindings reach the factories
//! through a data type's `scalar()` / `default_scalar()`), and — for the serie
//! scalars (`Int8Serie` … `UInt64Serie`) — their per-element-null construction,
//! `array` / `nulls` Arrow-buffer surface and `from_io` / `pwrite_io`
//! two-resource bridge (which borrow a second IO resource at once), so a serie
//! built from Python is a dense (all-valid) serie. The still-generic nested
//! scalars — the generic `Serie` / `MapScalar` / `StructScalar` — have no
//! concrete FFI shape yet.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_scalar::Scalar;

use crate::DataErr;

/// Reads `as_str` through the optional charset name — `"utf8"` (the default) or
/// `"latin1"` — shared by every scalar class.
fn as_str_with<S: Scalar>(scalar: &S, charset: Option<&str>) -> Result<String, DataErr> {
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
pub struct NullScalar {
    pub(crate) inner: yggdryl_scalar::NullScalar,
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
    fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType::default()
    }
}

/// A single, possibly-null `binary` value, holding its bytes as a core
/// positioned-IO `ByteBuffer` (`to_io()` hands one back).
#[pyclass]
pub struct BinaryScalar {
    pub(crate) inner: yggdryl_scalar::BinaryScalar,
}

#[pymethods]
impl BinaryScalar {
    /// A `binary` scalar holding `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_scalar::BinaryScalar::new(value),
        }
    }

    /// A null `binary` scalar.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::BinaryScalar::null(),
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
    fn data_type(&self) -> crate::dtype::BinaryType {
        crate::dtype::BinaryType::default()
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
pub struct OptionalBinaryScalar {
    pub(crate) inner:
        yggdryl_scalar::OptionalScalar<yggdryl_dtype::BinaryType, yggdryl_scalar::BinaryScalar>,
}

#[pymethods]
impl OptionalBinaryScalar {
    /// A scalar holding the `binary` value variant `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::BinaryScalar::new(value)),
        }
    }

    /// The null variant.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::OptionalScalar::null(),
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
    fn data_type(&self) -> crate::dtype::OptionalBinaryType {
        crate::dtype::OptionalBinaryType::default()
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
/// types, with the `as_*` accessors on both. `$dtype` / `$opt_dtype` name the
/// `yggdryl.dtype` classes the scalars report.
macro_rules! int_scalar_py {
    ($ty:ident, $opt_ty:ident, $dtype:ident, $opt_dtype:ident, $native:ty, $name:literal) => {
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
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
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
            pub(crate) inner:
                yggdryl_scalar::OptionalScalar<yggdryl_dtype::$dtype, yggdryl_scalar::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_scalar::OptionalScalar::new(yggdryl_scalar::$ty::new(value)),
                }
            }

            /// The null variant.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::OptionalScalar::null(),
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
            fn data_type(&self) -> crate::dtype::$opt_dtype {
                crate::dtype::$opt_dtype::default()
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

int_scalar_py!(
    Int8Scalar,
    OptionalInt8Scalar,
    Int8Type,
    OptionalInt8Type,
    i8,
    "int8"
);
int_scalar_py!(
    Int16Scalar,
    OptionalInt16Scalar,
    Int16Type,
    OptionalInt16Type,
    i16,
    "int16"
);
int_scalar_py!(
    Int32Scalar,
    OptionalInt32Scalar,
    Int32Type,
    OptionalInt32Type,
    i32,
    "int32"
);
int_scalar_py!(
    Int64Scalar,
    OptionalInt64Scalar,
    Int64Type,
    OptionalInt64Type,
    i64,
    "int64"
);
int_scalar_py!(
    UInt8Scalar,
    OptionalUInt8Scalar,
    UInt8Type,
    OptionalUInt8Type,
    u8,
    "uint8"
);
int_scalar_py!(
    UInt16Scalar,
    OptionalUInt16Scalar,
    UInt16Type,
    OptionalUInt16Type,
    u16,
    "uint16"
);
int_scalar_py!(
    UInt32Scalar,
    OptionalUInt32Scalar,
    UInt32Type,
    OptionalUInt32Type,
    u32,
    "uint32"
);
int_scalar_py!(
    UInt64Scalar,
    OptionalUInt64Scalar,
    UInt64Type,
    OptionalUInt64Type,
    u64,
    "uint64"
);

/// Generates the concrete serie scalar of one integer value type: `$ty`, the
/// buffer-backed `list` of `$name` — a thin delegation to `yggdryl_scalar::$ty`.
/// `$scalar` names the element scalar class, `$dtype` the `yggdryl.dtype` class.
macro_rules! int_serie_scalar_py {
    ($ty:ident, $scalar:ident, $dtype:ident, $native:ty, $name:literal) => {
        #[doc = concat!("A single, possibly-null `list` of `", $name, "` — *our array*, the buffer-backed")]
        /// serie scalar. Built dense (all-valid) from Python; the whole serie may still
        #[doc = concat!("be null (`", stringify!($ty), ".null()`).")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[pymethods]
        impl $ty {
            /// A serie holding the native serie `values` (all-valid).
            #[new]
            fn new(values: Vec<$native>) -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::from(values),
                }
            }

            /// The null serie scalar.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::null(),
                }
            }

            /// Whether this scalar holds a null value (distinct from the empty serie).
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The number of elements, `0` when null or empty (`is_null` distinguishes
            /// the two).
            fn len(&self) -> usize {
                self.inner.len()
            }

            /// Whether the sequence holds no elements (also `True` when null).
            fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The whole element buffer as a `list[int]`, or `None` when null.
            fn values(&self) -> Option<Vec<$native>> {
                self.inner.values().map(<[$native]>::to_vec)
            }

            /// The element at `index` read as its native `int`; raises `ValueError` when
            /// null or past the end, and `OverflowError` for a negative index.
            fn get_at(&self, index: usize) -> Result<$native, DataErr> {
                Ok(self.inner.get_at::<$native>(index)?)
            }

            #[doc = concat!("The element at `index` as an `", stringify!($scalar), "`, or `None` when the serie is")]
            /// null or `index` is past the end (a negative index raises
            /// `OverflowError`).
            fn get_scalar_at(&self, index: usize) -> Option<$scalar> {
                self.inner
                    .get_scalar_at(index)
                    .map(|inner| $scalar { inner })
            }

            /// The scalar's data type.
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }
        }
    };
}

int_serie_scalar_py!(Int8Serie, Int8Scalar, Int8SerieType, i8, "int8");
int_serie_scalar_py!(Int16Serie, Int16Scalar, Int16SerieType, i16, "int16");
int_serie_scalar_py!(Int32Serie, Int32Scalar, Int32SerieType, i32, "int32");
int_serie_scalar_py!(Int64Serie, Int64Scalar, Int64SerieType, i64, "int64");
int_serie_scalar_py!(UInt8Serie, UInt8Scalar, UInt8SerieType, u8, "uint8");
int_serie_scalar_py!(UInt16Serie, UInt16Scalar, UInt16SerieType, u16, "uint16");
int_serie_scalar_py!(UInt32Serie, UInt32Scalar, UInt32SerieType, u32, "uint32");
int_serie_scalar_py!(UInt64Serie, UInt64Scalar, UInt64SerieType, u64, "uint64");

/// Populates the `scalar` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NullScalar>()?;
    module.add_class::<BinaryScalar>()?;
    module.add_class::<OptionalBinaryScalar>()?;
    module.add_class::<Int8Scalar>()?;
    module.add_class::<OptionalInt8Scalar>()?;
    module.add_class::<Int16Scalar>()?;
    module.add_class::<OptionalInt16Scalar>()?;
    module.add_class::<Int32Scalar>()?;
    module.add_class::<OptionalInt32Scalar>()?;
    module.add_class::<Int64Scalar>()?;
    module.add_class::<OptionalInt64Scalar>()?;
    module.add_class::<UInt8Scalar>()?;
    module.add_class::<OptionalUInt8Scalar>()?;
    module.add_class::<UInt16Scalar>()?;
    module.add_class::<OptionalUInt16Scalar>()?;
    module.add_class::<UInt32Scalar>()?;
    module.add_class::<OptionalUInt32Scalar>()?;
    module.add_class::<UInt64Scalar>()?;
    module.add_class::<OptionalUInt64Scalar>()?;
    module.add_class::<Int8Serie>()?;
    module.add_class::<Int16Serie>()?;
    module.add_class::<Int32Serie>()?;
    module.add_class::<Int64Serie>()?;
    module.add_class::<UInt8Serie>()?;
    module.add_class::<UInt16Serie>()?;
    module.add_class::<UInt32Serie>()?;
    module.add_class::<UInt64Serie>()?;
    Ok(())
}
