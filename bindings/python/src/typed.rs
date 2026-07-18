//! The `yggdryl.typed` submodule — the **typed-column surface** of the typed serialization layer.
//!
//! Mirrors `yggdryl_core::typed`'s column surface: a [`Serie`] (a typed column — many elements of
//! one [`DataTypeId`](crate::datatype_id::DataTypeId) over a byte buffer, plus an optional validity
//! bitmap for nulls) and its [`Field`] (the column's `name` / `type` / `nullable` metadata, carried
//! in a [`Headers`](crate::headers::Headers)). Where the core `FixedSerie<T>` is generic over its
//! compile-time element type `T`, the binding erases `T` into an [`Inner`] enum — one variant per
//! fixed-width type — and dispatches each method across the variants, so one dynamic `Serie` class
//! covers every dtype.
//!
//! Every method is one or two lines over `yggdryl_core`; a reduction on a `bool` column raises a
//! guided `TypeError` (booleans do not reduce — `Bit` is not numeric), and a hard-fill read error
//! surfaces as a `ValueError` carrying the core text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use yggdryl_core::datatype_id::DataTypeId as CoreId;
use yggdryl_core::io::memory::{self, IOBase, IoError};
use yggdryl_core::typed::{
    fixedbit::Bit,
    fixedbyte::{
        Float32, Float64, Int128, Int16, Int32, Int64, Int8, UInt128, UInt16, UInt32, UInt64, UInt8,
    },
    Decoder, Encoder, Field as _, FixedSerie, HeaderField, Scalar, Serie as _,
};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The **type-erased** column backing [`Serie`] — one variant per fixed-width core element type.
/// A method on `Serie` dispatches across every variant (see the [`dispatch!`] / [`map_variant!`]
/// helpers), so the single dynamic class serves every dtype without 13× hand-written methods.
enum Inner {
    I8(FixedSerie<Int8>),
    U8(FixedSerie<UInt8>),
    I16(FixedSerie<Int16>),
    U16(FixedSerie<UInt16>),
    I32(FixedSerie<Int32>),
    U32(FixedSerie<UInt32>),
    I64(FixedSerie<Int64>),
    U64(FixedSerie<UInt64>),
    I128(FixedSerie<Int128>),
    U128(FixedSerie<UInt128>),
    F32(FixedSerie<Float32>),
    F64(FixedSerie<Float64>),
    Bool(FixedSerie<Bit>),
}

/// Resolves the `dtype` argument (a `yggdryl.datatype_id.DataTypeId`, or a type-name `str` like
/// `"i64"`) into the core [`CoreId`], the fixed-width element type to build.
fn resolve_dtype(dtype: &Bound<'_, PyAny>) -> PyResult<CoreId> {
    if let Ok(id) = dtype.extract::<DataTypeId>() {
        Ok(id.into())
    } else if let Ok(name) = dtype.extract::<String>() {
        CoreId::from_name(&name).ok_or_else(|| {
            PyValueError::new_err(format!(
                "unknown dtype {name:?}: expected a DataTypeId or a type name like 'i8', 'u8', \
                 'i16', 'u16', 'i32', 'u32', 'i64', 'u64', 'i128', 'u128', 'f32', 'f64', 'bool'"
            ))
        })
    } else {
        Err(PyTypeError::new_err(
            "dtype must be a yggdryl.datatype_id.DataTypeId or a type name str like 'i64'",
        ))
    }
}

/// Rebuilds a fresh column that **shares the same encoded bytes** as `s` (cloning only the small
/// data/validity heaps) with its column `name` set — the [`with_name`](Serie::with_name) worker
/// (the core `with_name` consumes `self`, which the `&self` binding cannot).
fn rebuild_named<T: Encoder + Decoder>(
    s: &FixedSerie<T, memory::Heap>,
    name: &str,
) -> FixedSerie<T, memory::Heap> {
    FixedSerie::from_data(s.data().clone(), s.validity().cloned(), s.len()).with_name(name)
}

/// Runs `$body` against the inner `FixedSerie` (bound to `$s`) of whichever variant is active,
/// unifying the arms at `$body`'s type — the read/scalar dispatch used by the value-agnostic and
/// value-marshalling methods.
macro_rules! dispatch {
    ($self:expr, $s:ident => $body:expr) => {
        match &$self.inner {
            Inner::I8($s) => $body,
            Inner::U8($s) => $body,
            Inner::I16($s) => $body,
            Inner::U16($s) => $body,
            Inner::I32($s) => $body,
            Inner::U32($s) => $body,
            Inner::I64($s) => $body,
            Inner::U64($s) => $body,
            Inner::I128($s) => $body,
            Inner::U128($s) => $body,
            Inner::F32($s) => $body,
            Inner::F64($s) => $body,
            Inner::Bool($s) => $body,
        }
    };
}

/// Like [`dispatch!`], but re-wraps the per-variant `FixedSerie` that `$make` produces back into
/// the **matching** [`Inner`] variant — the column-returning dispatch (`with_name`, `filter`).
macro_rules! map_variant {
    ($self:expr, $s:ident => $make:expr) => {
        match &$self.inner {
            Inner::I8($s) => Inner::I8($make),
            Inner::U8($s) => Inner::U8($make),
            Inner::I16($s) => Inner::I16($make),
            Inner::U16($s) => Inner::U16($make),
            Inner::I32($s) => Inner::I32($make),
            Inner::U32($s) => Inner::U32($make),
            Inner::I64($s) => Inner::I64($make),
            Inner::U64($s) => Inner::U64($make),
            Inner::I128($s) => Inner::I128($make),
            Inner::U128($s) => Inner::U128($make),
            Inner::F32($s) => Inner::F32($make),
            Inner::F64($s) => Inner::F64($make),
            Inner::Bool($s) => Inner::Bool($make),
        }
    };
}

/// Dispatches a `DataTypeId` to a callback macro `$mk!(Variant, Marker, native)` for every
/// fixed-width type — the shared spine of [`Serie::from_values`] / [`Serie::from_options`]. The
/// non-fixed `Unknown` (and any newer/foreign id) is a guided `ValueError`.
macro_rules! by_dtype {
    ($id:expr, $mk:ident) => {
        match $id {
            CoreId::I8 => $mk!(I8, Int8, i8),
            CoreId::U8 => $mk!(U8, UInt8, u8),
            CoreId::I16 => $mk!(I16, Int16, i16),
            CoreId::U16 => $mk!(U16, UInt16, u16),
            CoreId::I32 => $mk!(I32, Int32, i32),
            CoreId::U32 => $mk!(U32, UInt32, u32),
            CoreId::I64 => $mk!(I64, Int64, i64),
            CoreId::U64 => $mk!(U64, UInt64, u64),
            CoreId::I128 => $mk!(I128, Int128, i128),
            CoreId::U128 => $mk!(U128, UInt128, u128),
            CoreId::F32 => $mk!(F32, Float32, f32),
            CoreId::F64 => $mk!(F64, Float64, f64),
            CoreId::Bool => $mk!(Bool, Bit, bool),
            _ => {
                return Err(PyValueError::new_err(
                    "dtype has no fixed-width element type: expected one of i8, u8, i16, u16, \
                     i32, u32, i64, u64, i128, u128, f32, f64, bool (not 'unknown')",
                ))
            }
        }
    };
}

/// A **typed column** — many elements of one [`DataTypeId`](crate::datatype_id::DataTypeId) over a
/// byte buffer, with an optional validity bitmap for nulls. Built from a list of values (or a list
/// of options, for nulls); `get` / `to_list` are null-aware, `values` reads the raw buffer, and the
/// numeric `sum` / `min` / `max` / `mean` reduce over the byte layer's vectorized kernels (a `bool`
/// column does not reduce).
#[pyclass(module = "yggdryl.typed")]
pub struct Serie {
    inner: Inner,
}

#[pymethods]
impl Serie {
    /// A **non-nullable** column holding `values` (a list of numbers / bools), encoded as `dtype`
    /// (a `DataTypeId` or a type-name `str` like `"i64"`).
    #[staticmethod]
    fn from_values(values: &Bound<'_, PyAny>, dtype: &Bound<'_, PyAny>) -> PyResult<Serie> {
        let id = resolve_dtype(dtype)?;
        macro_rules! mk {
            ($variant:ident, $marker:ty, $native:ty) => {{
                let v: Vec<$native> = values.extract()?;
                Inner::$variant(FixedSerie::<$marker>::from_values(&v))
            }};
        }
        Ok(Serie {
            inner: by_dtype!(id, mk),
        })
    }

    /// A **nullable** column from `values` (a list that may contain `None`), encoded as `dtype` —
    /// each `None` becomes a null (a cleared validity bit; a default is stored in the slot).
    #[staticmethod]
    fn from_options(values: &Bound<'_, PyAny>, dtype: &Bound<'_, PyAny>) -> PyResult<Serie> {
        let id = resolve_dtype(dtype)?;
        macro_rules! mk {
            ($variant:ident, $marker:ty, $native:ty) => {{
                let v: Vec<Option<$native>> = values.extract()?;
                Inner::$variant(FixedSerie::<$marker>::from_options(&v))
            }};
        }
        Ok(Serie {
            inner: by_dtype!(id, mk),
        })
    }

    /// The number of elements in the column.
    fn len(&self) -> usize {
        dispatch!(self, s => s.len())
    }

    /// The number of elements (so `len(serie)` works).
    fn __len__(&self) -> usize {
        self.len()
    }

    /// Truthiness — `True` when the column holds at least one element.
    fn __bool__(&self) -> bool {
        !self.is_empty()
    }

    /// Whether the column holds no elements.
    fn is_empty(&self) -> bool {
        dispatch!(self, s => s.is_empty())
    }

    /// The element at `index` as a Python `int` / `float` / `bool`, or `None` when it is null or
    /// out of range.
    fn get(&self, py: Python<'_>, index: usize) -> PyObject {
        dispatch!(self, s => match s.get(index) {
            Some(value) => value.into_py(py),
            None => py.None(),
        })
    }

    /// Every element as an option (null-aware) — a Python list of values with `None` in each null
    /// slot.
    fn to_list(&self, py: Python<'_>) -> Vec<PyObject> {
        dispatch!(self, s => s
            .to_options()
            .into_iter()
            .map(|value| match value {
                Some(value) => value.into_py(py),
                None => py.None(),
            })
            .collect::<Vec<PyObject>>())
    }

    /// The **raw** values (validity ignored) — a Python list; a null slot surfaces its stored
    /// default. Pair with [`is_valid`](Serie::is_valid) for null-awareness.
    fn values(&self, py: Python<'_>) -> Vec<PyObject> {
        dispatch!(self, s => s
            .values()
            .into_iter()
            .map(|value| value.into_py(py))
            .collect::<Vec<PyObject>>())
    }

    /// How many elements are null.
    fn null_count(&self) -> usize {
        dispatch!(self, s => s.null_count())
    }

    /// Whether the element at `index` is **null** (absent, or out of range).
    fn is_null(&self, index: usize) -> bool {
        dispatch!(self, s => s.is_null(index))
    }

    /// Whether the element at `index` is **valid** (present and in range).
    fn is_valid(&self, index: usize) -> bool {
        dispatch!(self, s => s.is_valid(index))
    }

    /// The column's element [`DataTypeId`](crate::datatype_id::DataTypeId).
    fn dtype(&self) -> DataTypeId {
        dispatch!(self, s => s.data_type_id()).into()
    }

    /// A **fresh** column addressing the same bytes with its column `name` set — the metadata a
    /// [`field`](Serie::field) reports.
    fn with_name(&self, name: &str) -> Serie {
        Serie {
            inner: map_variant!(self, s => rebuild_named(s, name)),
        }
    }

    /// The column's [`Field`] metadata — its `name`, element type, and `nullable` flag.
    fn field(&self) -> Field {
        Field {
            inner: dispatch!(self, s => s.field()),
        }
    }

    /// **Filters** the column by `mask` — a list of `bool` (or another `bool` `Serie`), keeping
    /// each element whose mask entry is `True` — returning a fresh compacted column.
    fn filter(&self, mask: &Bound<'_, PyAny>) -> PyResult<Serie> {
        let bits: Vec<bool> = if let Ok(other) = mask.extract::<PyRef<'_, Serie>>() {
            match &other.inner {
                Inner::Bool(s) => (0..s.len()).map(|i| s.get(i).unwrap_or(false)).collect(),
                _ => return Err(PyTypeError::new_err(
                    "filter mask Serie must be a bool serie: build it with dtype=DataTypeId.Bool",
                )),
            }
        } else {
            mask.extract::<Vec<bool>>().map_err(|_| {
                PyTypeError::new_err("filter mask must be a list of bool or a bool Serie")
            })?
        };
        let mut heap = memory::Heap::new();
        for (index, &keep) in bits.iter().enumerate() {
            heap.pwrite_bit(index as u64, keep).map_err(ioerr)?;
        }
        Ok(Serie {
            inner: map_variant!(self, s => s.filter(&heap)),
        })
    }

    fn __repr__(&self) -> String {
        let dtype = dispatch!(self, s => s.data_type_id()).name();
        let len = dispatch!(self, s => s.len());
        let nulls = dispatch!(self, s => s.null_count());
        match dispatch!(self, s => s.field().name().map(str::to_string)) {
            Some(name) => {
                format!("Serie(name={name:?}, dtype='{dtype}', len={len}, null_count={nulls})")
            }
            None => format!("Serie(dtype='{dtype}', len={len}, null_count={nulls})"),
        }
    }
}

/// Emits the numeric reductions (`sum` / `min` / `max` / `mean`) — each dispatches to the core
/// reduction across the numeric variants and raises a guided `TypeError` for a `bool` column
/// (booleans do not reduce; `Bit` is not numeric). `sum` of an integer column returns a Python
/// `int` (`i128` / `u128` fit); `mean` and float columns return a `float`; an empty `min` / `max` /
/// `mean` is `None`.
macro_rules! reduce_methods {
    ($(($method:ident, $label:literal)),+ $(,)?) => {
        #[pymethods]
        impl Serie {
            $(
                #[doc = concat!("The **", $label, "** of the column, reduced over the data buffer; \
                    raises `TypeError` for a bool column (booleans do not reduce).")]
                fn $method(&self, py: Python<'_>) -> PyResult<PyObject> {
                    match &self.inner {
                        Inner::I8(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::U8(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::I16(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::U16(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::I32(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::U32(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::I64(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::U64(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::I128(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::U128(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::F32(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::F64(s) => Ok(s.$method().map_err(ioerr)?.into_py(py)),
                        Inner::Bool(_) => Err(PyTypeError::new_err(concat!(
                            "bool serie has no ", $label,
                            ": booleans do not reduce (Bit is not numeric) — use a numeric dtype"
                        ))),
                    }
                }
            )+
        }
    };
}

reduce_methods!((sum, "sum"), (min, "min"), (max, "max"), (mean, "mean"));

/// A **column descriptor** — a column's `name`, element [`DataTypeId`](crate::datatype_id::DataTypeId),
/// and nullability, carried in a [`Headers`](crate::headers::Headers) map (so it serializes, hashes,
/// and travels like any other metadata). Wraps the core `HeaderField`.
#[pyclass(module = "yggdryl.typed")]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: HeaderField,
}

#[pymethods]
impl Field {
    /// A field from its `name` (optional), element `dtype` (a `DataTypeId` or a type-name `str`),
    /// and `nullable` flag (default `False`).
    #[new]
    #[pyo3(signature = (name = None, dtype = None, nullable = false))]
    fn new(
        name: Option<String>,
        dtype: Option<&Bound<'_, PyAny>>,
        nullable: bool,
    ) -> PyResult<Self> {
        let dtype = dtype.ok_or_else(|| {
            PyTypeError::new_err(
                "Field(...) requires a dtype: a yggdryl.datatype_id.DataTypeId or a type name \
                 like 'i64'",
            )
        })?;
        let id = resolve_dtype(dtype)?;
        Ok(Field {
            inner: HeaderField::new(name.as_deref(), id, nullable),
        })
    }

    /// The column name, if set.
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// The element [`DataTypeId`](crate::datatype_id::DataTypeId).
    fn dtype(&self) -> DataTypeId {
        self.inner.data_type_id().into()
    }

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The backing [`Headers`](crate::headers::Headers) metadata map, as an owned **copy** (name /
    /// type / nullable live here, alongside any extra annotations).
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// Hashes by the canonical field metadata (equal fields hash equal) — a `Field` is an
    /// immutable value, so it works as a map key / in a set.
    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __repr__(&self) -> String {
        format!(
            "Field(name={:?}, dtype='{}', nullable={})",
            self.inner.name(),
            self.inner.data_type_id().name(),
            if self.inner.nullable() {
                "True"
            } else {
                "False"
            },
        )
    }
}

/// Populates the `typed` submodule with the column surface: [`Serie`] and its [`Field`].
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Serie>()?;
    module.add_class::<Field>()?;
    Ok(())
}
