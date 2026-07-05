//! The `yggdryl.scalar` submodule — thin wrappers over the `yggdryl-scalar` crate.
//!
//! Every integer and float type is exposed as its scalar and its null-or-value
//! optional scalar (e.g. `Int64Scalar`, `OptionalInt64Scalar`; the `float16`
//! family's native `half::f16` crosses as a Python `float`), alongside
//! `BinaryScalar` / `OptionalBinaryScalar` (whose value is held as a core
//! positioned-IO `ByteBuffer` — `to_io()` hands one back), `Utf8Scalar` /
//! `OptionalUtf8Scalar` (a `utf8` value crossing as Python `str`, its UTF-8
//! bytes reachable through `as_bytes` — the core `Utf8Buffer` stays Rust-only,
//! so there is no `to_io()`), `NullScalar`, `RecordScalar` (the `struct` row built
//! from a dict, its children inferred like the factory's) and its serie scalar
//! (e.g. `Int64Serie`, the buffer-backed `list` of `int64`) — the same suffixed
//! names as the Rust crate, the submodule carrying the concern.
//! Scalars expose the
//! `as_*` accessors with the core contract: the value when the target represents
//! it exactly, or a raised `ValueError` naming the fix (strings and bytes cross
//! the FFI boundary as new Python objects, so the Rust-side "borrow, never copy"
//! guarantee applies up to that boundary copy). Optional scalars adapt
//! construction to idioms: they are built straight from the native value
//! (`OptionalInt64Scalar(42)`), the inner scalar being an implementation detail
//! reachable through `scalar()`.
//!
//! Every scalar also exposes `to_pyvalue()` — the **general native accessor**:
//! the whole value converted once in the Rust core and crossing the FFI boundary
//! in a single call (`None` when null; an `int`, `bytes`, `list[int]`, or — for
//! `RecordScalar`, the possibly-null `struct` row built from a dict — an instance
//! of the schema's auto-generated singleton frozen dataclass, one cached class
//! per field-name tuple).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow_scalar` / `to_arrow_array` / `from_arrow`, and `cast_dtype` /
//! `cast_dtype_unchecked` which return a re-typed `arrow-array` value — all exchange
//! `arrow-array` values that cannot cross the
//! FFI boundary; C Data Interface interop is future work), the `FromScalar` /
//! `ScalarFactory` traits (generic Rust bounds; the bindings reach the factories
//! through a data type's `scalar()` / `default_scalar()`), and — for the serie
//! scalars (`Int8Serie` … `UInt64Serie`) — their per-element-null construction,
//! `to_arrow_array` / `nulls` Arrow-buffer surface and `from_io` / `pwrite_io`
//! two-resource bridge (which borrow a second IO resource at once), so a serie
//! built from Python is a dense (all-valid) serie. The still-generic nested
//! scalars — the generic `Serie` / `MapScalar`, the plain `StructScalar` row
//! value (its accessor surface is exposed as `RecordScalar`), the struct-row
//! series `StructSerie` / `TypedStructSerie`, and the type-erased `AnySerie` /
//! `AnyScalar` holders behind them — have no concrete FFI shape yet. A concrete
//! serie iterates its element scalars (`for scalar in serie`, or the materialized
//! `scalars()` list); the core's *lazy* `iter_scalars` and the struct-row
//! `iter_records` (no bound struct serie) stay Rust-only.

// pyo3's `#[pymethods]` expansion re-wraps the already-`PyErr` result of the
// `PyResult`-returning record methods into `PyErr`; clippy flags that generated
// conversion (on the return-type span) as useless.
#![allow(clippy::useless_conversion)]

use std::hash::{Hash, Hasher};

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::{IntoPyDict, PyBytes, PyDict, PyTuple};
use yggdryl_dtype::Struct;
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

/// A value-based, deterministic hash — the body of every scalar/serie
/// `__hash__`. A `DefaultHasher::new()` uses fixed keys (not the randomized
/// `RandomState`), so equal values hash equally within and across processes.
fn value_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Value equality — the body of every scalar/serie `__richcmp__`. `Eq` / `Ne`
/// compare by value; the ordering ops return Python `NotImplemented` (a scalar
/// carries no order).
fn value_richcmp<T: PartialEq>(lhs: &T, rhs: &T, op: CompareOp, py: Python<'_>) -> PyObject {
    match op {
        CompareOp::Eq => (lhs == rhs).into_py(py),
        CompareOp::Ne => (lhs != rhs).into_py(py),
        _ => py.NotImplemented(),
    }
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

    /// A compact rendering for fast debugging — always `null`.
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `repr(x)` shows `null`.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `print(x)` shows `null`.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` rendering with explicit limits (`max_rows` body rows,
    /// `max_width` columns) — series and records honour both.
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
    }

    /// Always `True`.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type.
    fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType::default()
    }

    /// The scalar's native Python value — always `None` (the general native
    /// accessor: one FFI crossing).
    fn to_pyvalue(&self) {}

    /// A value-based hash — every null scalar hashes equally (they are all
    /// equal). Enables `set` / `dict` membership.
    fn __hash__(&self) -> u64 {
        value_hash(&self.inner)
    }

    /// Value equality: every `NullScalar` equals every other; a different class
    /// falls through to Python's default (the ordering ops are `NotImplemented`).
    fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
        value_richcmp(&self.inner, &other.inner, op, py)
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

    /// A compact rendering for fast debugging — the value (`0x0102`, `null`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `repr(x)` shows the value.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `print(x)` shows the value.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` rendering with explicit limits (`max_rows` body rows,
    /// `max_width` columns) — series and records honour both.
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
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

    /// The scalar's native Python value: its `bytes`, or `None` when null (the
    /// general native accessor: one FFI crossing).
    fn to_pyvalue<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.value(py)
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
    /// The value as a `float` (a Python `float`, `f16` widened to f64);
    /// raises `ValueError` when null or not exactly representable in f16.
    fn as_f16(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f16()?.to_f64())
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

    /// A value-based hash (over the bytes, or its null state) — enables `set` /
    /// `dict` membership. Equal values hash equally.
    fn __hash__(&self) -> u64 {
        value_hash(&self.inner)
    }

    /// Value equality: two `BinaryScalar`s are equal when their bytes (or null
    /// state) match; a different class falls through to Python's default (the
    /// ordering ops are `NotImplemented`).
    fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
        value_richcmp(&self.inner, &other.inner, op, py)
    }
}

/// A single value of the union between null and `binary`: a value variant, or
/// the null variant.
#[pyclass]
pub struct OptionalBinaryScalar {
    pub(crate) inner: yggdryl_scalar::TypedOptionalScalar<
        yggdryl_dtype::BinaryType,
        yggdryl_scalar::BinaryScalar,
    >,
}

#[pymethods]
impl OptionalBinaryScalar {
    /// A scalar holding the `binary` value variant `value`.
    #[new]
    fn new(value: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::BinaryScalar::new(
                value,
            )),
        }
    }

    /// A compact rendering for fast debugging — the value (`0x0102`, `null`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `repr(x)` shows the value.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `print(x)` shows the value.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` rendering with explicit limits (`max_rows` body rows,
    /// `max_width` columns) — series and records honour both.
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
    }

    /// The null variant.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::null(),
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

    /// The scalar's native Python value: its `bytes`, or `None` for the null
    /// variant (the general native accessor: one FFI crossing).
    fn to_pyvalue<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.value(py)
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
    fn as_f16(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f16()?.to_f64())
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

            /// A compact rendering for fast debugging — the value (`42`, `null`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `repr(x)` shows the value.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `print(x)` shows the value.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` rendering with explicit limits (`max_rows` body
            /// rows, `max_width` columns) — series and records honour both.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
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

            /// The scalar's native Python value: its `int`, or `None` when null
            /// (the general native accessor: one FFI crossing).
            fn to_pyvalue(&self) -> Option<$native> {
                self.value()
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
            /// The value as a `float` (a Python `float`, `f16` widened to f64);
            /// raises `ValueError` when null or not exactly representable in f16.
            fn as_f16(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f16()?.to_f64())
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

            /// A value-based hash (over the value, or its null state) — enables
            /// `set` / `dict` membership. Equal values hash equally.
            fn __hash__(&self) -> u64 {
                value_hash(&self.inner)
            }

            #[doc = concat!("Value equality: two `", $name, "` scalars are equal when their value")]
            /// (or null state) match; a different class falls through to Python's
            /// default (the ordering ops are `NotImplemented`).
            fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
                value_richcmp(&self.inner, &other.inner, op, py)
            }
        }

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[pyclass]
        pub struct $opt_ty {
            pub(crate) inner:
                yggdryl_scalar::TypedOptionalScalar<yggdryl_dtype::$dtype, yggdryl_scalar::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value`.")]
            #[new]
            fn new(value: $native) -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::$ty::new(value)),
                }
            }

            /// A compact rendering for fast debugging — the value (`42`, `null`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `repr(x)` shows the value.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `print(x)` shows the value.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` rendering with explicit limits (`max_rows` body
            /// rows, `max_width` columns) — series and records honour both.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
            }

            /// The null variant.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::null(),
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

            /// The scalar's native Python value: its `int`, or `None` for the
            /// null variant (the general native accessor: one FFI crossing).
            fn to_pyvalue(&self) -> Option<$native> {
                self.value()
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
            /// The value as a `float` (a Python `float`, `f16` widened to f64);
            /// raises `ValueError` when null or not exactly representable in f16.
            fn as_f16(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f16()?.to_f64())
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
int_scalar_py!(
    Float32Scalar,
    OptionalFloat32Scalar,
    Float32Type,
    OptionalFloat32Type,
    f32,
    "float32"
);
int_scalar_py!(
    Float64Scalar,
    OptionalFloat64Scalar,
    Float64Type,
    OptionalFloat64Type,
    f64,
    "float64"
);

/// Generates the two `float16` scalar wrappers — the scalar `$ty` and the
/// null-or-value `$opt_ty` — mirroring [`int_scalar_py!`], except the native
/// `half::f16` does not cross the FFI boundary: it crosses as a Python `float`
/// (f64), so the constructor narrows the incoming `f64` to `f16` and every
/// value-carrying method (`value` / `to_pyvalue` / `as_f16`) widens `f16` back
/// to `f64`. `$dtype` / `$opt_dtype` name the `yggdryl.dtype` classes.
macro_rules! float16_scalar_py {
    ($ty:ident, $opt_ty:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A single, possibly-null `", $name, "` value (crossing as a Python `float`).")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("A `", $name, "` scalar holding `value` (narrowed from the Python `float`).")]
            #[new]
            fn new(value: f64) -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::new(yggdryl_scalar::half::f16::from_f64(value)),
                }
            }

            /// A compact rendering for fast debugging — the value (`1.5`, `null`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `repr(x)` shows the value.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `print(x)` shows the value.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` rendering with explicit limits (`max_rows` body
            /// rows, `max_width` columns) — series and records honour both.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
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

            /// The scalar's value as a Python `float` (f16 widened to f64), or
            /// `None` when null.
            fn value(&self) -> Option<f64> {
                self.inner.value().map(|value| value.to_f64())
            }

            /// The scalar's native Python value: its `float`, or `None` when null
            /// (the general native accessor: one FFI crossing).
            fn to_pyvalue(&self) -> Option<f64> {
                self.value()
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
            /// The value as a `float` (a Python `float`, `f16` widened to f64);
            /// raises `ValueError` when null — always exact (the native width).
            fn as_f16(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f16()?.to_f64())
            }
            /// The value as a `float`; raises `ValueError` when null — always
            /// exact (every f16 widens to f32).
            fn as_f32(&self) -> Result<f32, DataErr> {
                Ok(self.inner.as_f32()?)
            }
            /// The value as a `float`; raises `ValueError` when null — always
            /// exact (every f16 widens to f64).
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

            /// A value-based hash (over the f16 value, or its null state) —
            /// enables `set` / `dict` membership. Equal values hash equally.
            fn __hash__(&self) -> u64 {
                value_hash(&self.inner)
            }

            #[doc = concat!("Value equality: two `", $name, "` scalars are equal when their f16 value")]
            /// (or null state) match; a different class falls through to Python's
            /// default (the ordering ops are `NotImplemented`).
            fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
                value_richcmp(&self.inner, &other.inner, op, py)
            }
        }

        #[doc = concat!("A single value of the union between null and `", $name, "`: a value variant, or the null variant.")]
        #[pyclass]
        pub struct $opt_ty {
            pub(crate) inner:
                yggdryl_scalar::TypedOptionalScalar<yggdryl_dtype::$dtype, yggdryl_scalar::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("A scalar holding the `", $name, "` value variant `value` (narrowed from the Python `float`).")]
            #[new]
            fn new(value: f64) -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::$ty::new(
                        yggdryl_scalar::half::f16::from_f64(value),
                    )),
                }
            }

            /// A compact rendering for fast debugging — the value (`1.5`, `null`).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `repr(x)` shows the value.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` form — `print(x)` shows the value.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` rendering with explicit limits (`max_rows` body
            /// rows, `max_width` columns) — series and records honour both.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
            }

            /// The null variant.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: yggdryl_scalar::TypedOptionalScalar::null(),
                }
            }

            /// Whether this scalar holds the null variant.
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The value as a Python `float` (f16 widened to f64), or `None` for
            /// the null variant.
            fn value(&self) -> Option<f64> {
                self.inner.value().map(|value| value.to_f64())
            }

            /// The inner scalar, when this holds the value variant.
            fn scalar(&self) -> Option<$ty> {
                self.inner.scalar().map(|scalar| $ty { inner: *scalar })
            }

            /// The scalar's native Python value: its `float`, or `None` for the
            /// null variant (the general native accessor: one FFI crossing).
            fn to_pyvalue(&self) -> Option<f64> {
                self.value()
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
            /// The value as a `float` (a Python `float`, `f16` widened to f64);
            /// raises `ValueError` when null — always exact (the native width).
            fn as_f16(&self) -> Result<f64, DataErr> {
                Ok(self.inner.as_f16()?.to_f64())
            }
            /// The value as a `float`; raises `ValueError` when null — always
            /// exact (every f16 widens to f32).
            fn as_f32(&self) -> Result<f32, DataErr> {
                Ok(self.inner.as_f32()?)
            }
            /// The value as a `float`; raises `ValueError` when null — always
            /// exact (every f16 widens to f64).
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

float16_scalar_py!(
    Float16Scalar,
    OptionalFloat16Scalar,
    Float16Type,
    OptionalFloat16Type,
    "float16"
);

/// A single, possibly-null `utf8` value, holding its text as new Python `str`
/// objects at the FFI boundary (the core `Utf8Buffer` stays Rust-only — see the
/// module doc). It mirrors [`BinaryScalar`], except the value crosses as `str`
/// (its UTF-8 `bytes` reachable through `as_bytes`) instead of `bytes`.
#[pyclass]
pub struct Utf8Scalar {
    pub(crate) inner: yggdryl_scalar::Utf8Scalar,
}

#[pymethods]
impl Utf8Scalar {
    /// A `utf8` scalar holding `value`.
    #[new]
    fn new(value: String) -> Self {
        Self {
            inner: yggdryl_scalar::Utf8Scalar::new(value),
        }
    }

    /// A compact rendering for fast debugging — the value (`"hi"`, `null`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `repr(x)` shows the value.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `print(x)` shows the value.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` rendering with explicit limits (`max_rows` body rows,
    /// `max_width` columns) — series and records honour both.
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
    }

    /// A null `utf8` scalar.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::Utf8Scalar::null(),
        }
    }

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's value as `str`, or `None` when null.
    fn value(&self) -> Option<String> {
        self.inner.value().map(str::to_string)
    }

    /// The scalar's native Python value: its `str`, or `None` when null (the
    /// general native accessor: one FFI crossing).
    fn to_pyvalue(&self) -> Option<String> {
        self.value()
    }

    /// The scalar's data type.
    fn data_type(&self) -> crate::dtype::Utf8Type {
        crate::dtype::Utf8Type::default()
    }

    /// The value as an `int` in the i8 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i8(&self) -> Result<i8, DataErr> {
        Ok(self.inner.as_i8()?)
    }
    /// The value as an `int` in the i16 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i16(&self) -> Result<i16, DataErr> {
        Ok(self.inner.as_i16()?)
    }
    /// The value as an `int` in the i32 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i32(&self) -> Result<i32, DataErr> {
        Ok(self.inner.as_i32()?)
    }
    /// The value as an `int` in the i64 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i64(&self) -> Result<i64, DataErr> {
        Ok(self.inner.as_i64()?)
    }
    /// The value as an `int` in the u8 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u8(&self) -> Result<u8, DataErr> {
        Ok(self.inner.as_u8()?)
    }
    /// The value as an `int` in the u16 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u16(&self) -> Result<u16, DataErr> {
        Ok(self.inner.as_u16()?)
    }
    /// The value as an `int` in the u32 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u32(&self) -> Result<u32, DataErr> {
        Ok(self.inner.as_u32()?)
    }
    /// The value as an `int` in the u64 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u64(&self) -> Result<u64, DataErr> {
        Ok(self.inner.as_u64()?)
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f16(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f16()?.to_f64())
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f32(&self) -> Result<f32, DataErr> {
        Ok(self.inner.as_f32()?)
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f64(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f64()?)
    }
    /// The value as a `bool`; raises `ValueError` (a string value is not a
    /// boolean).
    fn as_bool(&self) -> Result<bool, DataErr> {
        Ok(self.inner.as_bool()?)
    }
    /// The value as a `str` — the native type; `charset` picks the decoder
    /// (`"utf8"`, the default, or `"latin1"`); raises `ValueError` when null.
    #[pyo3(signature = (charset = None))]
    fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
        as_str_with(&self.inner, charset)
    }
    /// The value as its UTF-8 `bytes`; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }

    /// A value-based hash (over the text, or its null state) — enables `set` /
    /// `dict` membership. Equal values hash equally.
    fn __hash__(&self) -> u64 {
        value_hash(&self.inner)
    }

    /// Value equality: two `Utf8Scalar`s are equal when their text (or null
    /// state) match; a different class falls through to Python's default (the
    /// ordering ops are `NotImplemented`).
    fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
        value_richcmp(&self.inner, &other.inner, op, py)
    }
}

/// A single value of the union between null and `utf8`: a value variant, or the
/// null variant — the string counterpart of [`OptionalBinaryScalar`].
#[pyclass]
pub struct OptionalUtf8Scalar {
    pub(crate) inner:
        yggdryl_scalar::TypedOptionalScalar<yggdryl_dtype::Utf8Type, yggdryl_scalar::Utf8Scalar>,
}

#[pymethods]
impl OptionalUtf8Scalar {
    /// A scalar holding the `utf8` value variant `value`.
    #[new]
    fn new(value: String) -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::new(yggdryl_scalar::Utf8Scalar::new(value)),
        }
    }

    /// A compact rendering for fast debugging — the value (`"hi"`, `null`).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `repr(x)` shows the value.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` form — `print(x)` shows the value.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` rendering with explicit limits (`max_rows` body rows,
    /// `max_width` columns) — series and records honour both.
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
    }

    /// The null variant.
    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: yggdryl_scalar::TypedOptionalScalar::null(),
        }
    }

    /// Whether this scalar holds the null variant.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The value as `str`, or `None` for the null variant.
    fn value(&self) -> Option<String> {
        self.inner.value().map(str::to_string)
    }

    /// The inner scalar, when this holds the value variant.
    fn scalar(&self) -> Option<Utf8Scalar> {
        self.inner.scalar().map(|scalar| Utf8Scalar {
            inner: scalar.clone(),
        })
    }

    /// The scalar's native Python value: its `str`, or `None` for the null
    /// variant (the general native accessor: one FFI crossing).
    fn to_pyvalue(&self) -> Option<String> {
        self.value()
    }

    /// The scalar's data type: the logical optional of the value type.
    fn data_type(&self) -> crate::dtype::OptionalUtf8Type {
        crate::dtype::OptionalUtf8Type::default()
    }

    /// The value as an `int` in the i8 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i8(&self) -> Result<i8, DataErr> {
        Ok(self.inner.as_i8()?)
    }
    /// The value as an `int` in the i16 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i16(&self) -> Result<i16, DataErr> {
        Ok(self.inner.as_i16()?)
    }
    /// The value as an `int` in the i32 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i32(&self) -> Result<i32, DataErr> {
        Ok(self.inner.as_i32()?)
    }
    /// The value as an `int` in the i64 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_i64(&self) -> Result<i64, DataErr> {
        Ok(self.inner.as_i64()?)
    }
    /// The value as an `int` in the u8 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u8(&self) -> Result<u8, DataErr> {
        Ok(self.inner.as_u8()?)
    }
    /// The value as an `int` in the u16 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u16(&self) -> Result<u16, DataErr> {
        Ok(self.inner.as_u16()?)
    }
    /// The value as an `int` in the u32 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u32(&self) -> Result<u32, DataErr> {
        Ok(self.inner.as_u32()?)
    }
    /// The value as an `int` in the u64 range; raises `ValueError` (a string
    /// value has no numeric form).
    fn as_u64(&self) -> Result<u64, DataErr> {
        Ok(self.inner.as_u64()?)
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f16(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f16()?.to_f64())
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f32(&self) -> Result<f32, DataErr> {
        Ok(self.inner.as_f32()?)
    }
    /// The value as a `float`; raises `ValueError` (a string value has no
    /// numeric form).
    fn as_f64(&self) -> Result<f64, DataErr> {
        Ok(self.inner.as_f64()?)
    }
    /// The value as a `bool`; raises `ValueError` (a string value is not a
    /// boolean).
    fn as_bool(&self) -> Result<bool, DataErr> {
        Ok(self.inner.as_bool()?)
    }
    /// The value as a `str` — the native type; `charset` picks the decoder
    /// (`"utf8"`, the default, or `"latin1"`); raises `ValueError` when null.
    #[pyo3(signature = (charset = None))]
    fn as_str(&self, charset: Option<&str>) -> Result<String, DataErr> {
        as_str_with(&self.inner, charset)
    }
    /// The value as its UTF-8 `bytes`; raises `ValueError` when null.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Result<Bound<'py, PyBytes>, DataErr> {
        Ok(PyBytes::new_bound(py, self.inner.as_bytes()?))
    }
}

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

            /// A compact box-drawn table for fast debugging — the item field
            /// header and the first rows (`null` for a null serie).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table — `repr(x)` shows it.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table — `print(x)` shows it.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table with explicit limits — at most `max_rows`
            /// body rows (a `… (N more)` footer past that), fit to `max_width`.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
            }

            /// The item field in compact `name: type` form (e.g. `item: int64`).
            fn field(&self) -> String {
                format!(
                    "{}: {}",
                    self.inner.field().name(),
                    yggdryl_scalar::yggdryl_dtype::signature(self.inner.field().data_type())
                )
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

            /// The whole element buffer copied out as a Python `list[int]`, or
            /// `None` when null — the pyarrow-style name for a native-container
            /// copy-out (the zero-copy borrow stays Rust-only).
            fn to_pylist(&self) -> Option<Vec<$native>> {
                self.inner.values().map(<[$native]>::to_vec)
            }

            /// The scalar's native Python value: its `list[int]`, or `None` when
            /// null (the general native accessor: one FFI crossing) — the serie
            /// spelling of `to_pylist`.
            fn to_pyvalue(&self) -> Option<Vec<$native>> {
                self.to_pylist()
            }

            /// The element at `index` read as its native `int`; raises `ValueError` when
            /// null or past the end, and `OverflowError` for a negative index.
            fn value_at(&self, index: usize) -> Result<$native, DataErr> {
                Ok(self.inner.value_at::<$native>(index)?)
            }

            #[doc = concat!("The element at `index` as an `", stringify!($scalar), "`, or `None` when the serie is")]
            /// null or `index` is past the end (a negative index raises
            /// `OverflowError`).
            fn scalar_at(&self, index: usize) -> Option<$scalar> {
                self.inner
                    .scalar_at(index)
                    .map(|inner| $scalar { inner })
            }

            #[doc = concat!("The elements as a `list[", stringify!($scalar), "]`, or `None` when null")]
            /// — the typed counterpart of `to_pylist` (which copies out the raw
            /// values). Iterating the serie (`for scalar in serie`) walks the same
            /// scalars.
            fn scalars(&self) -> Option<Vec<$scalar>> {
                (!self.inner.is_null()).then(|| {
                    self.inner
                        .iter_scalars()
                        .map(|inner| $scalar { inner })
                        .collect()
                })
            }

            /// Iterate the element scalars — `for scalar in serie` (a null serie is
            /// empty). Each element crosses the FFI boundary as its own scalar object.
            fn __iter__(slf: PyRef<'_, Self>) -> PyResult<Py<PyAny>> {
                let py = slf.py();
                let items = slf
                    .inner
                    .iter_scalars()
                    .map(|inner| Py::new(py, $scalar { inner }))
                    .collect::<PyResult<Vec<_>>>()?;
                let list = pyo3::types::PyList::new_bound(py, items);
                Ok(list.as_any().call_method0("__iter__")?.unbind())
            }

            /// The scalar's data type.
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// A value-based hash (over the elements, or its null state) —
            /// enables `set` / `dict` membership. Equal series hash equally.
            fn __hash__(&self) -> u64 {
                value_hash(&self.inner)
            }

            #[doc = concat!("Value equality: two `", $name, "` series are equal when their elements")]
            /// (or null state) match; a different class falls through to Python's
            /// default (the ordering ops are `NotImplemented`).
            fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
                value_richcmp(&self.inner, &other.inner, op, py)
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
int_serie_scalar_py!(
    Float32Serie,
    Float32Scalar,
    Float32SerieType,
    f32,
    "float32"
);
int_serie_scalar_py!(
    Float64Serie,
    Float64Scalar,
    Float64SerieType,
    f64,
    "float64"
);

/// Generates the concrete `float16` serie scalar `$ty`, mirroring
/// [`int_serie_scalar_py!`], except the native `half::f16` does not cross the FFI
/// boundary: the builder narrows each incoming Python `float` (f64) to `f16`, and
/// every value-carrying method (`to_pylist` / `to_pyvalue` / `value_at`) widens
/// `f16` back to `f64`. `$scalar` names the element scalar, `$dtype` the
/// `yggdryl.dtype` class.
macro_rules! float16_serie_scalar_py {
    ($ty:ident, $scalar:ident, $dtype:ident, $name:literal) => {
        #[doc = concat!("A single, possibly-null `list` of `", $name, "` — *our array*, the buffer-backed")]
        /// serie scalar. Built dense (all-valid) from Python `float` values (narrowed to
        #[doc = concat!("f16); the whole serie may still be null (`", stringify!($ty), ".null()`).")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_scalar::$ty,
        }

        #[pymethods]
        impl $ty {
            /// A serie holding the native serie `values` (all-valid), each narrowed
            /// from the Python `float` to f16.
            #[new]
            fn new(values: Vec<f64>) -> Self {
                Self {
                    inner: yggdryl_scalar::$ty::from(
                        values
                            .into_iter()
                            .map(yggdryl_scalar::half::f16::from_f64)
                            .collect::<Vec<_>>(),
                    ),
                }
            }

            /// A compact box-drawn table for fast debugging — the item field
            /// header and the first rows (`null` for a null serie).
            fn display(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table — `repr(x)` shows it.
            fn __repr__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table — `print(x)` shows it.
            fn __str__(&self) -> String {
                self.inner.display()
            }

            /// The `display()` table with explicit limits — at most `max_rows`
            /// body rows (a `… (N more)` footer past that), fit to `max_width`.
            #[pyo3(signature = (max_rows = 10, max_width = 100))]
            fn display_with(&self, max_rows: usize, max_width: usize) -> String {
                self.inner
                    .display_with(yggdryl_scalar::DisplayOptions { max_rows, max_width })
            }

            /// The item field in compact `name: type` form (`item: float16`).
            fn field(&self) -> String {
                format!(
                    "{}: {}",
                    self.inner.field().name(),
                    yggdryl_scalar::yggdryl_dtype::signature(self.inner.field().data_type())
                )
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

            /// The whole element buffer copied out as a Python `list[float]` (each
            /// f16 widened to f64), or `None` when null — the pyarrow-style name for
            /// a native-container copy-out (the zero-copy borrow stays Rust-only).
            fn to_pylist(&self) -> Option<Vec<f64>> {
                self.inner
                    .values()
                    .map(|values| values.iter().map(|value| value.to_f64()).collect())
            }

            /// The scalar's native Python value: its `list[float]`, or `None` when
            /// null (the general native accessor: one FFI crossing) — the serie
            /// spelling of `to_pylist`.
            fn to_pyvalue(&self) -> Option<Vec<f64>> {
                self.to_pylist()
            }

            /// The element at `index` read as a Python `float` (f16 widened to f64);
            /// raises `ValueError` when null or past the end, and `OverflowError` for
            /// a negative index.
            fn value_at(&self, index: usize) -> Result<f64, DataErr> {
                Ok(self.inner.value_at::<f64>(index)?)
            }

            #[doc = concat!("The element at `index` as a `", stringify!($scalar), "`, or `None` when the serie is")]
            /// null or `index` is past the end (a negative index raises
            /// `OverflowError`).
            fn scalar_at(&self, index: usize) -> Option<$scalar> {
                self.inner
                    .scalar_at(index)
                    .map(|inner| $scalar { inner })
            }

            #[doc = concat!("The elements as a `list[", stringify!($scalar), "]`, or `None` when null")]
            /// — the typed counterpart of `to_pylist` (which copies out the raw
            /// values). Iterating the serie (`for scalar in serie`) walks the same
            /// scalars.
            fn scalars(&self) -> Option<Vec<$scalar>> {
                (!self.inner.is_null()).then(|| {
                    self.inner
                        .iter_scalars()
                        .map(|inner| $scalar { inner })
                        .collect()
                })
            }

            /// Iterate the element scalars — `for scalar in serie` (a null serie is
            /// empty). Each element crosses the FFI boundary as its own scalar object.
            fn __iter__(slf: PyRef<'_, Self>) -> PyResult<Py<PyAny>> {
                let py = slf.py();
                let items = slf
                    .inner
                    .iter_scalars()
                    .map(|inner| Py::new(py, $scalar { inner }))
                    .collect::<PyResult<Vec<_>>>()?;
                let list = pyo3::types::PyList::new_bound(py, items);
                Ok(list.as_any().call_method0("__iter__")?.unbind())
            }

            /// The scalar's data type.
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// A value-based hash (over the f16 elements, or its null state) —
            /// enables `set` / `dict` membership. Equal series hash equally.
            fn __hash__(&self) -> u64 {
                value_hash(&self.inner)
            }

            #[doc = concat!("Value equality: two `", $name, "` series are equal when their f16 elements")]
            /// (or null state) match; a different class falls through to Python's
            /// default (the ordering ops are `NotImplemented`).
            fn __richcmp__(&self, other: PyRef<'_, Self>, op: CompareOp, py: Python<'_>) -> PyObject {
                value_richcmp(&self.inner, &other.inner, op, py)
            }
        }
    };
}

float16_serie_scalar_py!(Float16Serie, Float16Scalar, Float16SerieType, "float16");

/// Raises a `ValueError` naming a record child type the bindings cannot convert
/// to a native Python value yet.
fn child_unrepresentable(data_type: &yggdryl_scalar::arrow_schema::DataType) -> PyErr {
    DataErr::Message(format!(
        "no native Python form for a {data_type} record child yet; supported children are the \
         integer and float types, binary, null, the integer and float series and nested structs"
    ))
    .into()
}

/// A child serie's elements as a Python `list` of native values (a null element
/// reads as `None`) — the record child form of `to_pylist`.
fn serie_to_pylist(py: Python<'_>, serie: &yggdryl_scalar::AnySerie) -> PyResult<PyObject> {
    // `f16` is not a Python type: a float16 serie's elements widen to Python floats.
    if let yggdryl_scalar::AnySerie::Float16(serie) = serie {
        return Ok((0..serie.len())
            .map(|index| {
                serie
                    .scalar_at(index)
                    .and_then(|scalar| scalar.value().copied())
                    .map(|value| value.to_f64())
            })
            .collect::<Vec<_>>()
            .into_py(py));
    }
    macro_rules! elements {
        ($($variant:ident),+ $(,)?) => {
            match serie {
                $(yggdryl_scalar::AnySerie::$variant(serie) => Ok((0..serie.len())
                    .map(|index| {
                        serie
                            .scalar_at(index)
                            .and_then(|scalar| scalar.value().copied())
                    })
                    .collect::<Vec<_>>()
                    .into_py(py)),)+
                other => Err(child_unrepresentable(&other.data_type())),
            }
        };
    }
    elements!(Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64)
}

/// The native Python value of a record's field scalar — `get`, `to_pydict` and
/// `to_pyvalue` all convert through this single crossing.
fn scalar_to_pyvalue(py: Python<'_>, scalar: &yggdryl_scalar::AnyScalar) -> PyResult<PyObject> {
    use yggdryl_scalar::arrow_array::{self, Array};
    use yggdryl_scalar::arrow_schema::DataType as ArrowType;
    // `f16` is not a Python type: a float16 child widens to a Python float.
    if let yggdryl_scalar::AnyScalar::Float16(value) = scalar {
        return Ok(value.value().map(|value| value.to_f64()).into_py(py));
    }
    macro_rules! atom {
        ($($variant:ident),+ $(,)?) => {
            match scalar {
                // The decomposed integer field reads its native value directly.
                $(yggdryl_scalar::AnyScalar::$variant(value) => Ok(value.value().copied().into_py(py)),)+
                yggdryl_scalar::AnyScalar::Arrow(value) => match value.data_type() {
                    ArrowType::Null => Ok(py.None()),
                    ArrowType::Binary => {
                        let value = value
                            .as_any()
                            .downcast_ref::<arrow_array::BinaryArray>()
                            .expect("a binary field is a binary array");
                        Ok(if value.is_null(0) {
                            py.None()
                        } else {
                            PyBytes::new_bound(py, value.value(0)).into_py(py)
                        })
                    }
                    ArrowType::Utf8 => {
                        let value = value
                            .as_any()
                            .downcast_ref::<arrow_array::StringArray>()
                            .expect("a utf8 field is a string array");
                        Ok(if value.is_null(0) {
                            py.None()
                        } else {
                            value.value(0).into_py(py)
                        })
                    }
                    ArrowType::List(_) => {
                        let value = value
                            .as_any()
                            .downcast_ref::<arrow_array::ListArray>()
                            .expect("a serie field is a list array");
                        if value.is_null(0) {
                            return Ok(py.None());
                        }
                        serie_to_pylist(py, &yggdryl_scalar::AnySerie::from_arrow(value.value(0)))
                    }
                    ArrowType::Struct(_) => RecordScalar {
                        inner: yggdryl_scalar::RecordScalar::from_arrow(value.as_ref())
                            .map_err(DataErr::from)?,
                    }
                    .to_pyvalue(py),
                    other => Err(child_unrepresentable(other)),
                },
                other => Err(child_unrepresentable(&other.data_type())),
            }
        };
    }
    atom!(Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64)
}

/// The auto-generated singleton dataclasses behind `RecordScalar.to_pyvalue`,
/// keyed by the tuple of field names — one frozen class per schema, instances
/// per row.
static RECORD_CLASSES: GILOnceCell<Py<PyDict>> = GILOnceCell::new();

/// The frozen `Record` dataclass of the field-name tuple `names`, generated with
/// `dataclasses.make_dataclass` on first use and cached module-wide, so every
/// record of one schema shares one class.
fn record_class<'py>(py: Python<'py>, names: &Bound<'py, PyTuple>) -> PyResult<Bound<'py, PyAny>> {
    let classes = RECORD_CLASSES
        .get_or_init(py, || PyDict::new_bound(py).unbind())
        .bind(py);
    if let Some(class) = classes.get_item(names)? {
        return Ok(class);
    }
    let class = py
        .import_bound("dataclasses")?
        .getattr("make_dataclass")?
        .call(
            ("Record", names.clone()),
            Some(&[("frozen", true)].into_py_dict_bound(py)),
        )?;
    classes.set_item(names, &class)?;
    Ok(class)
}

/// A single, possibly-null `struct` row with per-child native access, built from
/// a dict mapping each field name to a native value — every child inferred and
/// converted once in Rust (`int` → `int64`, `bytes` → `binary`, `None` → `null`,
/// a list of ints → the `int64` serie, a dict → a nested record), each child
/// field nullable.
#[pyclass]
pub struct RecordScalar {
    pub(crate) inner: yggdryl_scalar::RecordScalar,
}

#[pymethods]
impl RecordScalar {
    /// A record holding the dict `row`, each child built from its native value
    /// through the factory's inference.
    #[new]
    fn new(row: &Bound<'_, PyDict>) -> PyResult<Self> {
        Ok(Self {
            inner: crate::factory::record_of(&crate::factory::infer_entries(row)?)?,
        })
    }

    /// A compact transposed `field | value` table for fast debugging (`null`
    /// for a null record).
    fn display(&self) -> String {
        self.inner.display()
    }

    /// The `display()` table — `repr(x)` shows it.
    fn __repr__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` table — `print(x)` shows it.
    fn __str__(&self) -> String {
        self.inner.display()
    }

    /// The `display()` table with explicit limits (`max_rows` body rows,
    /// `max_width` columns).
    #[pyo3(signature = (max_rows = 10, max_width = 100))]
    fn display_with(&self, max_rows: usize, max_width: usize) -> String {
        self.inner.display_with(yggdryl_scalar::DisplayOptions {
            max_rows,
            max_width,
        })
    }

    /// The null record of the struct type `data_type`.
    #[staticmethod]
    fn null(data_type: &crate::dtype::StructType) -> Self {
        Self {
            inner: yggdryl_scalar::RecordScalar::null(data_type.inner.clone()),
        }
    }

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The scalar's data type.
    fn data_type(&self) -> crate::dtype::StructType {
        crate::dtype::StructType {
            inner: self.inner.data_type().clone(),
        }
    }

    /// The child field names, in declaration order.
    fn field_names(&self) -> Vec<String> {
        self.inner
            .data_type()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect()
    }

    /// The native Python value of the field named `name`, or `None` when the
    /// record is null or no field carries the name.
    fn get(&self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        match self.inner.any_scalar_by(name) {
            Some(scalar) => scalar_to_pyvalue(py, &scalar),
            None => Ok(py.None()),
        }
    }

    /// The whole row copied out as a Python `dict` of native values, or `None`
    /// when null — the pyarrow-style name for a native-container copy-out.
    fn to_pydict<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(scalars) = self.inner.value() else {
            return Ok(None);
        };
        let row = PyDict::new_bound(py);
        // Borrow each field in place — no per-field `AnyScalar` clone.
        for (field, scalar) in self.inner.data_type().fields().iter().zip(scalars) {
            row.set_item(field.name(), scalar_to_pyvalue(py, scalar)?)?;
        }
        Ok(Some(row))
    }

    /// The scalar's native Python value: an instance of the schema's
    /// auto-generated singleton dataclass — one frozen class per field-name
    /// tuple, cached module-wide, instances per row — or `None` when null (the
    /// general native accessor: the whole row converted in Rust, one FFI
    /// crossing).
    fn to_pyvalue(&self, py: Python<'_>) -> PyResult<PyObject> {
        let Some(scalars) = self.inner.value() else {
            return Ok(py.None());
        };
        let fields = self.inner.data_type().fields();
        let names = PyTuple::new_bound(py, fields.iter().map(|field| field.name().as_str()));
        let class = record_class(py, &names)?;
        // Borrow each field in place — no per-field `AnyScalar` clone.
        let values = scalars
            .iter()
            .map(|scalar| scalar_to_pyvalue(py, scalar))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(class.call1(PyTuple::new_bound(py, values))?.unbind())
    }
}

/// Populates the `scalar` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NullScalar>()?;
    module.add_class::<BinaryScalar>()?;
    module.add_class::<OptionalBinaryScalar>()?;
    module.add_class::<Utf8Scalar>()?;
    module.add_class::<OptionalUtf8Scalar>()?;
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
    module.add_class::<Float16Scalar>()?;
    module.add_class::<OptionalFloat16Scalar>()?;
    module.add_class::<Float32Scalar>()?;
    module.add_class::<OptionalFloat32Scalar>()?;
    module.add_class::<Float64Scalar>()?;
    module.add_class::<OptionalFloat64Scalar>()?;
    module.add_class::<Int8Serie>()?;
    module.add_class::<Int16Serie>()?;
    module.add_class::<Int32Serie>()?;
    module.add_class::<Int64Serie>()?;
    module.add_class::<UInt8Serie>()?;
    module.add_class::<UInt16Serie>()?;
    module.add_class::<UInt32Serie>()?;
    module.add_class::<UInt64Serie>()?;
    module.add_class::<Float16Serie>()?;
    module.add_class::<Float32Serie>()?;
    module.add_class::<Float64Serie>()?;
    module.add_class::<RecordScalar>()?;
    Ok(())
}
