//! The `yggdryl.typed` submodule — the **typed-column surface** of the typed serialization layer.
//!
//! Mirrors `yggdryl_core::typed`'s column surface: a [`Serie`] (a typed column — many elements of
//! one [`DataTypeId`](crate::datatype_id::DataTypeId) over a byte buffer, plus an optional validity
//! bitmap for nulls), the byte-column [`ByteSerie`] (`bytes` / `str` elements — the variable-length
//! `Binary` / `Utf8` and the fixed-size `FixedBinary` / `FixedUtf8`), and their [`Field`] (the
//! column's `name` / `type` / `nullable` metadata, carried in a [`Headers`](crate::headers::Headers)).
//! Where the core `FixedSerie<T>` is generic over its compile-time element type `T`, the binding
//! erases `T` into an [`Inner`] enum — one variant per fixed-width type — and dispatches each method
//! across the variants, so one dynamic `Serie` class covers every dtype; [`ByteSerie`] does the same
//! over its four byte carriers ([`ByteInner`]).
//!
//! Every method is one or two lines over `yggdryl_core`; a reduction on a `bool` (or decimal) column
//! raises a guided `TypeError` (they do not reduce), and a hard-fill read error surfaces as a
//! `ValueError` carrying the core text unchanged. The four fixed-point **decimal** dtypes join the
//! erased [`Inner`]: their unscaled values cross as Python `int`s (a `Decimal256` beyond `i128` as an
//! arbitrary-precision `int`, via [`i256_from_py`] / [`i256_to_py`]), and `with_precision_scale` /
//! `to_decimal_string` / `decimal_precision` / `decimal_scale` add the scale-aware surface.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyInt};

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use yggdryl_core::datatype_id::DataTypeId as CoreId;
use yggdryl_core::io::memory::{self, IOBase, IoError};
use yggdryl_core::typed::{
    fixedbit::Bit,
    fixedbyte::{
        Decimal128, Decimal256, Decimal32, Decimal64, Float32, Float64, Int128, Int16, Int32,
        Int64, Int8, UInt128, UInt16, UInt32, UInt64, UInt8, I256,
    },
    Binary, Decoder, Encoder, Field as _, FixedBinary, FixedSerie, FixedSizeSerie, FixedUtf8,
    HeaderField, Scalar, Serie as _, Utf8, VarSerie, VarType,
};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Converts a Python `int` into an [`I256`] — the 256-bit unscaled value of a `Decimal256`. Values
/// that fit an `i128` take the cheap [`I256::from_i128`] path; wider ones cross as the 32
/// two's-complement little-endian bytes Python's `int.to_bytes(32, "little", signed=True)` renders,
/// so the full 256-bit range round-trips. A value that does not fit 256 bits is a guided
/// `ValueError`.
fn i256_from_py(obj: &Bound<'_, PyAny>) -> PyResult<I256> {
    if let Ok(value) = obj.extract::<i128>() {
        return Ok(I256::from_i128(value));
    }
    let py = obj.py();
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("signed", true)?;
    let bytes = obj
        .call_method("to_bytes", (32, "little"), Some(&kwargs))
        .map_err(|_| {
            PyValueError::new_err(
                "decimal256 value out of range: expected an integer that fits 256 bits \
                 (two's complement, -2**255 .. 2**255 - 1) — use a smaller value",
            )
        })?;
    let raw: Vec<u8> = bytes.extract()?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| PyValueError::new_err("decimal256 int did not encode to 32 bytes"))?;
    Ok(I256::from_le_bytes(bytes))
}

/// Converts an [`I256`] back into a Python `int` — the inverse of [`i256_from_py`]. Values within
/// `i128` use the native conversion; wider ones are rebuilt with `int.from_bytes(..., signed=True)`
/// over the 32 little-endian bytes, so a value beyond `i128` still lands as an exact Python integer.
fn i256_to_py(py: Python<'_>, value: I256) -> PyObject {
    if let Some(value) = value.to_i128() {
        return value.into_py(py);
    }
    let bytes = PyBytes::new_bound(py, &value.to_le_bytes());
    let kwargs = PyDict::new_bound(py);
    kwargs
        .set_item("signed", true)
        .expect("setting a bool dict item never fails");
    py.get_type_bound::<PyInt>()
        .call_method("from_bytes", (bytes, "little"), Some(&kwargs))
        .expect("int.from_bytes over 32 signed little-endian bytes never fails")
        .unbind()
}

/// Decodes every element of `values` (a Python iterable of `int`) into a `Vec<I256>` — the
/// `Decimal256` counterpart of pyo3's `Vec<i128>` extraction (which has no `I256` impl).
fn i256_values(values: &Bound<'_, PyAny>) -> PyResult<Vec<I256>> {
    let mut out = Vec::with_capacity(values.len().unwrap_or(0));
    for item in values.iter()? {
        out.push(i256_from_py(&item?)?);
    }
    Ok(out)
}

/// Like [`i256_values`], but null-aware — a Python `None` becomes a `None` slot (a nullable
/// `Decimal256` column built through `from_options`).
fn i256_options(values: &Bound<'_, PyAny>) -> PyResult<Vec<Option<I256>>> {
    let mut out = Vec::with_capacity(values.len().unwrap_or(0));
    for item in values.iter()? {
        let item = item?;
        if item.is_none() {
            out.push(None);
        } else {
            out.push(Some(i256_from_py(&item)?));
        }
    }
    Ok(out)
}

/// Converts a decoded native element into a Python object. Every native scalar goes through pyo3's
/// [`IntoPy`], and the 256-bit [`I256`] (which has no `IntoPy` impl) crosses through the
/// [`i256_to_py`] bridge — so one `dispatch!` body serves every dtype, decimals included.
trait IntoPyValue {
    fn into_py_value(self, py: Python<'_>) -> PyObject;
}

macro_rules! impl_into_py_value {
    ($($native:ty),+ $(,)?) => {$(
        impl IntoPyValue for $native {
            fn into_py_value(self, py: Python<'_>) -> PyObject {
                self.into_py(py)
            }
        }
    )+};
}

// The `i32` / `i64` / `i128` impls double as the `Decimal32` / `Decimal64` / `Decimal128` unscaled
// values; `Decimal256` uses the `I256` impl below.
impl_into_py_value!(i8, u8, i16, u16, i32, u32, i64, u64, i128, u128, f32, f64, bool);

impl IntoPyValue for I256 {
    fn into_py_value(self, py: Python<'_>) -> PyObject {
        i256_to_py(py, self)
    }
}

/// Converts a decoded **byte-column** element into a Python object — a `Vec<u8>` crosses as `bytes`
/// (via [`PyBytes`]), a `String` as `str`. The [`IntoPyValue`] counterpart for the byte carriers, so
/// one [`byte_dispatch!`] body serves a binary and a UTF-8 column alike.
trait IntoPyByteValue {
    fn into_py_byte_value(self, py: Python<'_>) -> PyObject;
}

impl IntoPyByteValue for Vec<u8> {
    fn into_py_byte_value(self, py: Python<'_>) -> PyObject {
        PyBytes::new_bound(py, &self).into_any().unbind()
    }
}

impl IntoPyByteValue for String {
    fn into_py_byte_value(self, py: Python<'_>) -> PyObject {
        self.into_py(py)
    }
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
    Decimal32(FixedSerie<Decimal32>),
    Decimal64(FixedSerie<Decimal64>),
    Decimal128(FixedSerie<Decimal128>),
    Decimal256(FixedSerie<Decimal256>),
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
                 'i16', 'u16', 'i32', 'u32', 'i64', 'u64', 'i128', 'u128', 'f32', 'f64', 'bool', \
                 'decimal32', 'decimal64', 'decimal128', 'decimal256'"
            ))
        })
    } else {
        Err(PyTypeError::new_err(
            "dtype must be a yggdryl.datatype_id.DataTypeId or a type name str like 'i64'",
        ))
    }
}

/// Rebuilds a fresh column that **shares the same encoded bytes** as `s` (cloning only the small
/// data/validity heaps), applying the given `name` and (for a decimal column) `precision`/`scale`
/// overrides — each defaulting to `s`'s current value. This is the shared worker behind
/// [`with_name`](Serie::with_name) and [`with_precision_scale`](Serie::with_precision_scale), whose
/// core counterparts consume `self` (which the `&self` binding cannot). The current `name` /
/// `precision` / `scale` are read back from `s.field()` so a rebuild never drops decimal metadata.
fn rebuild<T: Encoder + Decoder>(
    s: &FixedSerie<T, memory::Heap>,
    name: Option<&str>,
    precision_scale: Option<(u32, i32)>,
) -> FixedSerie<T, memory::Heap> {
    let field = s.field();
    let mut out = FixedSerie::from_data(s.data().clone(), s.validity().cloned(), s.len());
    if let Some(name) = name.or(field.name()) {
        out = out.with_name(name);
    }
    let precision_scale = precision_scale.or(match (field.precision(), field.scale()) {
        (Some(precision), Some(scale)) => Some((precision, scale)),
        _ => None,
    });
    if let Some((precision, scale)) = precision_scale {
        out = out.with_precision_scale(precision, scale);
    }
    out
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
            Inner::Decimal32($s) => $body,
            Inner::Decimal64($s) => $body,
            Inner::Decimal128($s) => $body,
            Inner::Decimal256($s) => $body,
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
            Inner::Decimal32($s) => Inner::Decimal32($make),
            Inner::Decimal64($s) => Inner::Decimal64($make),
            Inner::Decimal128($s) => Inner::Decimal128($make),
            Inner::Decimal256($s) => Inner::Decimal256($make),
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
            CoreId::Decimal32 => $mk!(Decimal32, Decimal32, i32),
            CoreId::Decimal64 => $mk!(Decimal64, Decimal64, i64),
            CoreId::Decimal128 => $mk!(Decimal128, Decimal128, i128),
            // `Decimal256`'s native `I256` has no pyo3 extraction, so its `$mk!` arm marshals each
            // Python `int` element itself (see the `@i256` rule the callers define).
            CoreId::Decimal256 => $mk!(@i256),
            _ => {
                return Err(PyValueError::new_err(
                    "dtype has no fixed-width element type: expected one of i8, u8, i16, u16, \
                     i32, u32, i64, u64, i128, u128, f32, f64, bool, decimal32, decimal64, \
                     decimal128, decimal256 (not 'unknown')",
                ))
            }
        }
    };
}

/// Runs `$body` against the inner `FixedSerie` of whichever **decimal** variant is active (binding
/// it to `$s`), and raises a guided `TypeError` for any non-decimal column — the dispatch behind the
/// decimal-only methods (`to_decimal_string`, `decimal_precision`, `decimal_scale`). `$what` names
/// the method in the error text.
macro_rules! decimal_dispatch {
    ($self:expr, $s:ident => $body:expr, $what:literal) => {
        match &$self.inner {
            Inner::Decimal32($s) => $body,
            Inner::Decimal64($s) => $body,
            Inner::Decimal128($s) => $body,
            Inner::Decimal256($s) => $body,
            _ => {
                return Err(PyTypeError::new_err(concat!(
                    "not a decimal column: ",
                    $what,
                    " applies only to a decimal Serie (dtype Decimal32 / Decimal64 / Decimal128 / \
                     Decimal256) — build the column with a decimal dtype"
                )))
            }
        }
    };
}

/// A **typed column** — many elements of one [`DataTypeId`](crate::datatype_id::DataTypeId) over a
/// byte buffer, with an optional validity bitmap for nulls. Built from a list of values (or a list
/// of options, for nulls); `get` / `to_list` are null-aware, `values` reads the raw buffer, and the
/// numeric `sum` / `min` / `max` / `mean` reduce over the byte layer's vectorized kernels (a `bool`
/// or decimal column does not reduce). A **decimal** column additionally carries `precision` /
/// `scale` metadata and renders a scale-aware `to_decimal_string`.
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
            (@i256) => {{
                let v = i256_values(values)?;
                Inner::Decimal256(FixedSerie::<Decimal256>::from_values(&v))
            }};
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
            (@i256) => {{
                let v = i256_options(values)?;
                Inner::Decimal256(FixedSerie::<Decimal256>::from_options(&v))
            }};
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
    /// out of range. A decimal element crosses as its raw **unscaled** integer (a `Decimal256` value
    /// beyond `i128` as an arbitrary-precision Python `int`).
    fn get(&self, py: Python<'_>, index: usize) -> PyObject {
        dispatch!(self, s => match s.get(index) {
            Some(value) => value.into_py_value(py),
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
                Some(value) => value.into_py_value(py),
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
            .map(|value| value.into_py_value(py))
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
    /// [`field`](Serie::field) reports. Any decimal `precision` / `scale` is carried over.
    fn with_name(&self, name: &str) -> Serie {
        Serie {
            inner: map_variant!(self, s => rebuild(s, Some(name), None)),
        }
    }

    /// The column's [`Field`] metadata — its `name`, element type, `nullable` flag, and (for a
    /// decimal column) its `precision` / `scale`.
    fn field(&self) -> Field {
        Field {
            inner: dispatch!(self, s => s.field()),
        }
    }

    /// A **fresh** decimal column addressing the same bytes with its `precision` (max significant
    /// digits) and `scale` (decimal places) set — the metadata [`field`](Serie::field) reports and
    /// [`to_decimal_string`](Serie::to_decimal_string) uses to place the decimal point. Raises
    /// `TypeError` on a non-decimal column.
    fn with_precision_scale(&self, precision: u32, scale: i32) -> PyResult<Serie> {
        match &self.inner {
            Inner::Decimal32(_)
            | Inner::Decimal64(_)
            | Inner::Decimal128(_)
            | Inner::Decimal256(_) => {}
            _ => {
                return Err(PyTypeError::new_err(
                    "not a decimal column: with_precision_scale applies only to a decimal Serie \
                     (dtype Decimal32 / Decimal64 / Decimal128 / Decimal256) — build the column \
                     with a decimal dtype",
                ))
            }
        }
        Ok(Serie {
            inner: map_variant!(self, s => rebuild(s, None, Some((precision, scale)))),
        })
    }

    /// The decimal value at `index` formatted with the column's scale (e.g. `"123.45"`), or `None`
    /// when the element is null or out of range. Raises `TypeError` on a non-decimal column.
    fn to_decimal_string(&self, index: usize) -> PyResult<Option<String>> {
        Ok(decimal_dispatch!(self, s => s.to_decimal_string(index), "to_decimal_string"))
    }

    /// The decimal **precision** (max significant digits) — the set value, else the width's max.
    /// Raises `TypeError` on a non-decimal column.
    fn decimal_precision(&self) -> PyResult<u32> {
        Ok(decimal_dispatch!(self, s => s.decimal_precision(), "decimal_precision"))
    }

    /// The decimal **scale** (decimal places) — the set value, else `0`. Raises `TypeError` on a
    /// non-decimal column.
    fn decimal_scale(&self) -> PyResult<i32> {
        Ok(decimal_dispatch!(self, s => s.decimal_scale(), "decimal_scale"))
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
                        Inner::Decimal32(_)
                        | Inner::Decimal64(_)
                        | Inner::Decimal128(_)
                        | Inner::Decimal256(_) => Err(PyTypeError::new_err(concat!(
                            "decimal serie has no ", $label,
                            ": decimals do not reduce (Decimal is not numeric here) — cast to a \
                             numeric dtype first"
                        ))),
                    }
                }
            )+
        }
    };
}

reduce_methods!((sum, "sum"), (min, "min"), (max, "max"), (mean, "mean"));

/// The **type-erased** column backing [`ByteSerie`] — one variant per byte carrier: the
/// variable-length [`VarSerie`] (`Binary` / `Utf8`, offsets + data) and the fixed-stride
/// [`FixedSizeSerie`] (`FixedBinary` / `FixedUtf8`). A method dispatches across the variants (see the
/// [`byte_dispatch!`] / [`byte_map!`] helpers), so the single dynamic class serves every byte dtype.
enum ByteInner {
    Binary(VarSerie<Binary>),
    Utf8(VarSerie<Utf8>),
    FixedBinary(FixedSizeSerie<FixedBinary>),
    FixedUtf8(FixedSizeSerie<FixedUtf8>),
}

/// Runs `$body` against the inner carrier (bound to `$s`) of whichever variant is active, unifying
/// the arms at `$body`'s type — the [`ByteSerie`] read/scalar dispatch (the byte counterpart of
/// [`dispatch!`]).
macro_rules! byte_dispatch {
    ($self:expr, $s:ident => $body:expr) => {
        match &$self.inner {
            ByteInner::Binary($s) => $body,
            ByteInner::Utf8($s) => $body,
            ByteInner::FixedBinary($s) => $body,
            ByteInner::FixedUtf8($s) => $body,
        }
    };
}

/// Like [`byte_dispatch!`], but re-wraps the per-variant carrier that `$make` produces back into the
/// **matching** [`ByteInner`] variant — the column-returning dispatch (the byte counterpart of
/// [`map_variant!`], behind [`with_name`](ByteSerie::with_name)).
macro_rules! byte_map {
    ($self:expr, $s:ident => $make:expr) => {
        match &$self.inner {
            ByteInner::Binary($s) => ByteInner::Binary($make),
            ByteInner::Utf8($s) => ByteInner::Utf8($make),
            ByteInner::FixedBinary($s) => ByteInner::FixedBinary($make),
            ByteInner::FixedUtf8($s) => ByteInner::FixedUtf8($make),
        }
    };
}

/// A guided `ValueError` for a `width=` passed to a **variable-length** dtype (`Binary` / `Utf8`).
fn var_width_error() -> PyErr {
    PyValueError::new_err(
        "a variable-length column takes no width: drop the width= argument for a binary / utf8 \
         column (its elements size themselves)",
    )
}

/// A guided `ValueError` for a **fixed-size** dtype (`FixedBinary` / `FixedUtf8`) built without a
/// `width=`.
fn fixed_width_missing() -> PyErr {
    PyValueError::new_err(
        "a fixed-size column needs a width: pass width=<N> (the fixed element byte length) for a \
         fixed_binary / fixed_utf8 column",
    )
}

/// A guided `ValueError` for a non-byte dtype passed to [`ByteSerie`] (a numeric / decimal / bool /
/// unknown dtype belongs on [`Serie`]).
fn non_byte_dtype_error() -> PyErr {
    PyValueError::new_err(
        "dtype is not a byte column: expected binary, utf8, fixed_binary, or fixed_utf8 — use \
         Serie for numeric/decimal/bool columns",
    )
}

/// Rebuilds a fresh byte carrier that **shares the same encoded bytes** as `self` (cloning only the
/// small offsets/data/validity heaps), applying `name` — the shared worker behind
/// [`with_name`](ByteSerie::with_name), whose core counterpart consumes `self` (which the `&self`
/// binding cannot). Implemented for both carriers so one [`byte_map!`] body covers the four variants.
trait RebuildByteSerie {
    fn rebuild_with_name(&self, name: &str) -> Self;
}

impl<T: VarType> RebuildByteSerie for VarSerie<T, memory::Heap> {
    fn rebuild_with_name(&self, name: &str) -> Self {
        VarSerie::from_parts(
            self.offsets().clone(),
            self.data().clone(),
            self.validity().cloned(),
            self.len(),
        )
        .with_name(name)
    }
}

impl<T: VarType> RebuildByteSerie for FixedSizeSerie<T, memory::Heap> {
    fn rebuild_with_name(&self, name: &str) -> Self {
        FixedSizeSerie::from_parts(
            self.data().clone(),
            self.validity().cloned(),
            self.len(),
            self.width(),
        )
        .with_name(name)
    }
}

/// A **byte-column** — the variable-length + fixed-size analogue of [`Serie`]: many `bytes` / `str`
/// elements of one byte [`DataTypeId`](crate::datatype_id::DataTypeId) (`Binary` / `Utf8` /
/// `FixedBinary` / `FixedUtf8`) over an offsets + data (or fixed-stride) buffer, with an optional
/// validity bitmap for nulls. Built from a list of `bytes` / `str` (or a list of options, for
/// nulls); `get` / `to_list` are null-aware and `values` reads the raw buffer. A variable-length
/// column sizes each element itself; a fixed-size column packs at a per-column byte `width`
/// (zero-padding a shorter value, truncating a longer one).
#[pyclass(module = "yggdryl.typed")]
pub struct ByteSerie {
    inner: ByteInner,
}

#[pymethods]
impl ByteSerie {
    /// A **non-nullable** byte column holding `values` (a list of `bytes` for a binary dtype, `str`
    /// for a utf8 dtype), encoded as `dtype` (a `DataTypeId` or a type-name `str` like `"binary"`).
    /// A fixed-size dtype (`"fixed_binary"` / `"fixed_utf8"`) requires `width`; a variable-length
    /// dtype (`"binary"` / `"utf8"`) takes none.
    #[staticmethod]
    #[pyo3(signature = (values, dtype, width=None))]
    fn from_values(
        values: &Bound<'_, PyAny>,
        dtype: &Bound<'_, PyAny>,
        width: Option<usize>,
    ) -> PyResult<ByteSerie> {
        let id = resolve_dtype(dtype)?;
        let inner = match id {
            CoreId::Binary => {
                if width.is_some() {
                    return Err(var_width_error());
                }
                let v: Vec<Vec<u8>> = values.extract()?;
                ByteInner::Binary(VarSerie::<Binary>::from_values(&v))
            }
            CoreId::Utf8 => {
                if width.is_some() {
                    return Err(var_width_error());
                }
                let v: Vec<String> = values.extract()?;
                ByteInner::Utf8(VarSerie::<Utf8>::from_values(&v))
            }
            CoreId::FixedBinary => {
                let width = width.ok_or_else(fixed_width_missing)?;
                let v: Vec<Vec<u8>> = values.extract()?;
                ByteInner::FixedBinary(FixedSizeSerie::<FixedBinary>::from_values(width, &v))
            }
            CoreId::FixedUtf8 => {
                let width = width.ok_or_else(fixed_width_missing)?;
                let v: Vec<String> = values.extract()?;
                ByteInner::FixedUtf8(FixedSizeSerie::<FixedUtf8>::from_values(width, &v))
            }
            _ => return Err(non_byte_dtype_error()),
        };
        Ok(ByteSerie { inner })
    }

    /// A **nullable** byte column from `values` (a list of `bytes` / `str` that may contain `None`),
    /// encoded as `dtype` — each `None` becomes a null. Same `dtype` / `width` handling as
    /// [`from_values`](ByteSerie::from_values).
    #[staticmethod]
    #[pyo3(signature = (values, dtype, width=None))]
    fn from_options(
        values: &Bound<'_, PyAny>,
        dtype: &Bound<'_, PyAny>,
        width: Option<usize>,
    ) -> PyResult<ByteSerie> {
        let id = resolve_dtype(dtype)?;
        let inner = match id {
            CoreId::Binary => {
                if width.is_some() {
                    return Err(var_width_error());
                }
                let v: Vec<Option<Vec<u8>>> = values.extract()?;
                ByteInner::Binary(VarSerie::<Binary>::from_options(&v))
            }
            CoreId::Utf8 => {
                if width.is_some() {
                    return Err(var_width_error());
                }
                let v: Vec<Option<String>> = values.extract()?;
                ByteInner::Utf8(VarSerie::<Utf8>::from_options(&v))
            }
            CoreId::FixedBinary => {
                let width = width.ok_or_else(fixed_width_missing)?;
                let v: Vec<Option<Vec<u8>>> = values.extract()?;
                ByteInner::FixedBinary(FixedSizeSerie::<FixedBinary>::from_options(width, &v))
            }
            CoreId::FixedUtf8 => {
                let width = width.ok_or_else(fixed_width_missing)?;
                let v: Vec<Option<String>> = values.extract()?;
                ByteInner::FixedUtf8(FixedSizeSerie::<FixedUtf8>::from_options(width, &v))
            }
            _ => return Err(non_byte_dtype_error()),
        };
        Ok(ByteSerie { inner })
    }

    /// The number of elements in the column.
    fn len(&self) -> usize {
        byte_dispatch!(self, s => s.len())
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
        byte_dispatch!(self, s => s.is_empty())
    }

    /// The element at `index` as `bytes` (a binary column) or `str` (a utf8 column), or `None` when
    /// it is null or out of range.
    fn get(&self, py: Python<'_>, index: usize) -> PyObject {
        byte_dispatch!(self, s => match s.get(index) {
            Some(value) => value.into_py_byte_value(py),
            None => py.None(),
        })
    }

    /// Every element as an option (null-aware) — a Python list of `bytes` / `str` with `None` in each
    /// null slot.
    fn to_list(&self, py: Python<'_>) -> Vec<PyObject> {
        byte_dispatch!(self, s => s
            .to_options()
            .into_iter()
            .map(|value| match value {
                Some(value) => value.into_py_byte_value(py),
                None => py.None(),
            })
            .collect::<Vec<PyObject>>())
    }

    /// The **raw** values (validity ignored) — a Python list of `bytes` / `str`; a null slot surfaces
    /// its stored bytes. Pair with [`is_valid`](ByteSerie::is_valid) for null-awareness.
    fn values(&self, py: Python<'_>) -> Vec<PyObject> {
        byte_dispatch!(self, s => s
            .values()
            .into_iter()
            .map(|value| value.into_py_byte_value(py))
            .collect::<Vec<PyObject>>())
    }

    /// How many elements are null.
    fn null_count(&self) -> usize {
        byte_dispatch!(self, s => s.null_count())
    }

    /// Whether the element at `index` is **null** (absent, or out of range).
    fn is_null(&self, index: usize) -> bool {
        byte_dispatch!(self, s => s.is_null(index))
    }

    /// Whether the element at `index` is **valid** (present and in range).
    fn is_valid(&self, index: usize) -> bool {
        byte_dispatch!(self, s => s.is_valid(index))
    }

    /// The column's element [`DataTypeId`](crate::datatype_id::DataTypeId).
    fn dtype(&self) -> DataTypeId {
        byte_dispatch!(self, s => s.data_type_id()).into()
    }

    /// The fixed element byte **width** for a fixed-size column (`FixedBinary` / `FixedUtf8`), or
    /// `None` for a variable-length column (`Binary` / `Utf8`, whose elements size themselves).
    fn width(&self) -> Option<usize> {
        match &self.inner {
            ByteInner::Binary(_) | ByteInner::Utf8(_) => None,
            ByteInner::FixedBinary(s) => Some(s.width()),
            ByteInner::FixedUtf8(s) => Some(s.width()),
        }
    }

    /// A **fresh** column addressing the same bytes with its column `name` set — the metadata a
    /// [`field`](ByteSerie::field) reports. Any fixed-size `width` is carried over.
    fn with_name(&self, name: &str) -> ByteSerie {
        ByteSerie {
            inner: byte_map!(self, s => s.rebuild_with_name(name)),
        }
    }

    /// The column's [`Field`] metadata — its `name`, element type, `nullable` flag, and (for a
    /// fixed-size column) its `byte_width`.
    fn field(&self) -> Field {
        Field {
            inner: byte_dispatch!(self, s => s.field()),
        }
    }

    fn __repr__(&self) -> String {
        let dtype = byte_dispatch!(self, s => s.data_type_id()).name();
        let len = byte_dispatch!(self, s => s.len());
        let nulls = byte_dispatch!(self, s => s.null_count());
        let width = match self.width() {
            Some(width) => format!(", width={width}"),
            None => String::new(),
        };
        match byte_dispatch!(self, s => s.field().name().map(str::to_string)) {
            Some(name) => format!(
                "ByteSerie(name={name:?}, dtype='{dtype}'{width}, len={len}, null_count={nulls})"
            ),
            None => {
                format!("ByteSerie(dtype='{dtype}'{width}, len={len}, null_count={nulls})")
            }
        }
    }
}

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

    /// The decimal **precision** (max significant digits) this field carries, or `None` for a
    /// non-decimal field.
    fn precision(&self) -> Option<u32> {
        self.inner.precision()
    }

    /// The decimal **scale** (decimal places) this field carries, or `None` for a non-decimal field.
    fn scale(&self) -> Option<i32> {
        self.inner.scale()
    }

    /// The fixed element **byte width** a fixed-size column (`FixedBinary` / `FixedUtf8`) carries, or
    /// `None` for a variable-length / non-byte field.
    fn byte_width(&self) -> Option<u32> {
        self.inner.byte_width()
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

/// Populates the `typed` submodule with the column surface: [`Serie`], the byte-column
/// [`ByteSerie`], and their [`Field`].
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Serie>()?;
    module.add_class::<ByteSerie>()?;
    module.add_class::<Field>()?;
    Ok(())
}
