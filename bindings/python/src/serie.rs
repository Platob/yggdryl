//! The `Serie` pyclass — a named, typed, Arrow-backed column (a single dataframe
//! column). A thin wrapper over [`yggdryl_serie`]'s `SerieRef`; all logic lives in the
//! core, so the Python and Node bindings behave identically.

use std::sync::Arc;

use pyo3::exceptions::{PyIndexError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};
use yggdryl_scalar::ScalarValue;
use yggdryl_schema::DataType as CoreDataType;
use yggdryl_serie::arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, StringArray,
};
use yggdryl_serie::{
    from_array, from_bytes, CategoricalSerie, DisplayOptions, ListSerie, MapSerie, Scalar,
    SerieRef, StructSerie, UInt64RangeSerie,
};

use crate::datatype::DataType;
use crate::field::Field;
use crate::{schema_err, serie_err};

/// A named, typed, Arrow-backed column. Build one from a list of values
/// (``Serie("n", [1, 2, 3])``), a lazy range (:meth:`range`) or child columns
/// (:meth:`struct`); read it by index, slice / resize / cast it, navigate nested
/// children, and round-trip it losslessly through :meth:`to_bytes`.
#[pyclass(name = "Serie", module = "yggdryl")]
#[derive(Clone)]
pub struct Serie {
    pub(crate) inner: SerieRef,
}

fn wrap(inner: SerieRef) -> Serie {
    Serie { inner }
}

/// Borrows a column as a [`StructSerie`] frame, or raises if it is not a struct column —
/// the gate for the frame (DataFrame) operations.
fn as_frame(serie: &SerieRef) -> PyResult<&StructSerie> {
    serie.as_any().downcast_ref::<StructSerie>().ok_or_else(|| {
        PyTypeError::new_err("not a struct column; build a frame with Serie.struct(...)")
    })
}

/// Borrows a column as a [`UInt64RangeSerie`], or raises if it is not a range/index
/// column — the gate for the index (label ↔ position) operations.
fn as_index(serie: &SerieRef) -> PyResult<&UInt64RangeSerie> {
    serie
        .as_any()
        .downcast_ref::<UInt64RangeSerie>()
        .ok_or_else(|| {
            PyTypeError::new_err(
                "not a range/index column; build one with Serie.range(...) or Serie.index(...)",
            )
        })
}

/// Borrows a column as a [`CategoricalSerie`], or raises if it is not a categorical
/// column — the gate for the dictionary (category / code) operations.
fn as_categorical(serie: &SerieRef) -> PyResult<&CategoricalSerie> {
    serie
        .as_any()
        .downcast_ref::<CategoricalSerie>()
        .ok_or_else(|| {
            PyTypeError::new_err("not a categorical column; build one with serie.categorical()")
        })
}

/// The element kind inferred from a Python list (boolean checked before int, since
/// Python `bool` is a subclass of `int`).
enum Kind {
    Bool,
    Int,
    Float,
    Str,
    Bytes,
}

/// Classifies one non-null Python value into a column [`Kind`].
fn classify(obj: &Bound<'_, PyAny>) -> PyResult<Kind> {
    if obj.is_instance_of::<PyBool>() {
        Ok(Kind::Bool)
    } else if obj.is_instance_of::<PyInt>() {
        Ok(Kind::Int)
    } else if obj.is_instance_of::<PyFloat>() {
        Ok(Kind::Float)
    } else if obj.is_instance_of::<PyString>() {
        Ok(Kind::Str)
    } else if obj.is_instance_of::<PyBytes>() || obj.is_instance_of::<PyByteArray>() {
        Ok(Kind::Bytes)
    } else {
        Err(PyTypeError::new_err(format!(
            "unsupported serie value type '{}'; expected bool / int / float / str / bytes",
            obj.get_type().name()?
        )))
    }
}

/// Builds the Arrow array for the inferred `kind` from a Python list (`None` → null).
fn build_array(values: &Bound<'_, PyList>, kind: Kind) -> PyResult<ArrayRef> {
    macro_rules! collect {
        ($ty:ty) => {{
            let mut out: Vec<Option<$ty>> = Vec::with_capacity(values.len());
            for item in values.iter() {
                out.push(if item.is_none() {
                    None
                } else {
                    Some(item.extract()?)
                });
            }
            out
        }};
    }
    Ok(match kind {
        Kind::Bool => Arc::new(BooleanArray::from(collect!(bool))),
        Kind::Int => Arc::new(Int64Array::from(collect!(i64))),
        Kind::Float => Arc::new(Float64Array::from(collect!(f64))),
        Kind::Str => Arc::new(StringArray::from_iter(collect!(String).into_iter())),
        Kind::Bytes => Arc::new(BinaryArray::from_iter(collect!(Vec<u8>).into_iter())),
    })
}

/// Resolves a `DataType` pyclass **or** a type string to a core [`CoreDataType`].
fn resolve_dtype(obj: &Bound<'_, PyAny>) -> PyResult<CoreDataType> {
    if let Ok(dt) = obj.extract::<DataType>() {
        return Ok(dt.inner);
    }
    let text: String = obj.extract()?;
    CoreDataType::from_str(&text).map_err(schema_err)
}

/// Maps a core [`Scalar`] to the matching Python object.
fn scalar_to_py(py: Python<'_>, scalar: &Scalar) -> PyObject {
    match scalar {
        Scalar::Null => py.None(),
        Scalar::Boolean(b) => b.into_py(py),
        Scalar::Int(i) => i.into_py(py),
        Scalar::Float(f) => f.into_py(py),
        Scalar::Utf8(s) => s.into_py(py),
        Scalar::Binary(b) => PyBytes::new_bound(py, b).into(),
        Scalar::Other(s) => s.into_py(py),
    }
}

#[pymethods]
impl Serie {
    /// Build a column named `name` from a list of values. The Arrow type is inferred
    /// (`bool` / `int` → int64 / `float` → float64 / `str` → utf8 / `bytes` → binary);
    /// pass `dtype` (a :class:`DataType` or type string) to cast to a specific type.
    /// An empty / all-null list needs an explicit `dtype`.
    #[new]
    #[pyo3(signature = (name, values, dtype = None))]
    fn new(
        name: &str,
        values: &Bound<'_, PyList>,
        dtype: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        // Infer the element kind from the first non-null value, if any.
        let mut kind: Option<Kind> = None;
        for item in values.iter() {
            if !item.is_none() {
                kind = Some(classify(&item)?);
                break;
            }
        }
        let inferred = kind.is_some();
        let base = match kind {
            Some(kind) => build_array(values, kind)?,
            // Empty / all-null: a null int64 base that the `dtype` cast re-types.
            None => Arc::new(Int64Array::from(vec![None::<i64>; values.len()])) as ArrayRef,
        };
        let serie = from_array(name, base).map_err(serie_err)?;
        match dtype {
            Some(obj) => {
                let dt = resolve_dtype(obj)?;
                Ok(wrap(serie.cast(&dt).map_err(serie_err)?))
            }
            None if !inferred && !values.is_empty() => Err(PyTypeError::new_err(
                "cannot infer a dtype from all-null values; pass dtype=...",
            )),
            None => Ok(wrap(serie)),
        }
    }

    /// Alias of the constructor — build a column from a list of values.
    #[staticmethod]
    #[pyo3(signature = (name, values, dtype = None))]
    fn from_values(
        name: &str,
        values: &Bound<'_, PyList>,
        dtype: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        Serie::new(name, values, dtype)
    }

    /// Reconstruct a column from its Arrow-IPC :meth:`to_bytes` form.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> PyResult<Self> {
        from_bytes(data).map(wrap).map_err(serie_err)
    }

    /// A lazy ``uint64`` arithmetic range column (`start + i*step`), not materialised.
    #[staticmethod]
    #[pyo3(signature = (length, start = 0, step = 1, name = "range"))]
    fn range(length: usize, start: u64, step: u64, name: &str) -> Self {
        wrap(Arc::new(UInt64RangeSerie::new(name, start, step, length)))
    }

    /// A lazy ``uint64`` row index of `length` rows (`0..length`) — a
    /// :class:`UInt64RangeSerie` with the label ↔ position lookups.
    #[staticmethod]
    fn index(length: usize) -> Self {
        wrap(Arc::new(UInt64RangeSerie::indices(length)))
    }

    /// Build a struct column named `name` from its child columns (each child's field,
    /// including its name, becomes a struct field). The children stay lazy until
    /// :meth:`materialize`.
    #[staticmethod]
    #[pyo3(name = "struct")]
    fn struct_(name: &str, children: Vec<Serie>) -> PyResult<Self> {
        let refs: Vec<SerieRef> = children.into_iter().map(|c| c.inner).collect();
        StructSerie::from_children(name, refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// Build a **list** column named `name` from a list of sub-lists (each row a list of
    /// elements, or ``None`` for a null row). The element type is inferred like the
    /// :class:`Serie` constructor; pass `dtype` to cast the elements. An empty / all-empty
    /// input needs an explicit element `dtype`.
    #[staticmethod]
    #[pyo3(name = "list", signature = (name, values, dtype = None))]
    fn list_(
        py: Python<'_>,
        name: &str,
        values: &Bound<'_, PyList>,
        dtype: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        // Flatten the rows, recording each row's element count (`None` = null row).
        let mut flat: Vec<Bound<'_, PyAny>> = Vec::new();
        let mut lengths: Vec<Option<usize>> = Vec::with_capacity(values.len());
        for row in values.iter() {
            if row.is_none() {
                lengths.push(None);
            } else {
                let sub = row.downcast::<PyList>().map_err(|_| {
                    PyTypeError::new_err("each list row must be a list of elements or None")
                })?;
                lengths.push(Some(sub.len()));
                flat.extend(sub.iter());
            }
        }
        let items = Serie::new("item", &PyList::new_bound(py, flat), dtype)?;
        ListSerie::<i32>::from_values(name, items.inner, &lengths)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// Build a **map** column named `name` from a list of dicts (each row a ``dict`` of
    /// key → value, or ``None`` for a null row). Key / value types are inferred like the
    /// :class:`Serie` constructor; pass `key_dtype` / `value_dtype` to cast them.
    #[staticmethod]
    #[pyo3(name = "map", signature = (name, entries, key_dtype = None, value_dtype = None))]
    fn map_(
        py: Python<'_>,
        name: &str,
        entries: &Bound<'_, PyList>,
        key_dtype: Option<&Bound<'_, PyAny>>,
        value_dtype: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut keys: Vec<Bound<'_, PyAny>> = Vec::new();
        let mut vals: Vec<Bound<'_, PyAny>> = Vec::new();
        let mut lengths: Vec<Option<usize>> = Vec::with_capacity(entries.len());
        for row in entries.iter() {
            if row.is_none() {
                lengths.push(None);
            } else {
                let dict = row.downcast::<PyDict>().map_err(|_| {
                    PyTypeError::new_err("each map row must be a dict of key/value pairs or None")
                })?;
                lengths.push(Some(dict.len()));
                for (k, v) in dict.iter() {
                    keys.push(k);
                    vals.push(v);
                }
            }
        }
        let key_col = Serie::new("key", &PyList::new_bound(py, keys), key_dtype)?;
        let value_col = Serie::new("value", &PyList::new_bound(py, vals), value_dtype)?;
        MapSerie::from_values(name, key_col.inner, value_col.inner, &lengths)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    // ---- metadata ----

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type().clone(),
        }
    }

    #[getter]
    fn field(&self) -> Field {
        Field {
            inner: self.inner.field().clone(),
        }
    }

    /// The type category (``primitive`` / ``logical`` / ``nested`` / ``any``).
    #[getter]
    fn category(&self) -> &'static str {
        self.inner.data_type().category().as_str()
    }

    #[getter]
    fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// The number of rows.
    #[getter]
    fn num_rows(&self) -> usize {
        self.inner.num_rows()
    }

    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether the column is materialised (a lazy range / categorical is not).
    #[getter]
    fn is_materialized(&self) -> bool {
        self.inner.is_materialized()
    }

    fn is_null(&self, index: usize) -> bool {
        self.inner.is_null(index)
    }

    fn is_valid(&self, index: usize) -> bool {
        self.inner.is_valid(index)
    }

    // ---- values ----

    /// The value at `index` (`None` for a null or out-of-bounds cell).
    fn value_at(&self, py: Python<'_>, index: usize) -> PyObject {
        scalar_to_py(py, &self.inner.value_at(index))
    }

    /// Every value as a Python list.
    fn to_list(&self, py: Python<'_>) -> Vec<PyObject> {
        (0..self.inner.len())
            .map(|i| scalar_to_py(py, &self.inner.value_at(i)))
            .collect()
    }

    /// A copy of the column with the cell at `index` replaced by `value` (a
    /// :class:`Scalar`). With ``safe`` the value is cast to the column's type first, so
    /// any value can be written. Functional — returns a new column.
    #[pyo3(signature = (index, value, safe = true))]
    fn set_at(&self, index: usize, value: &crate::scalar::Scalar, safe: bool) -> PyResult<Self> {
        let scalar = value.inner.clone().into_scalar();
        self.inner
            .set_at(index, scalar.as_ref(), safe)
            .map(wrap)
            .map_err(serie_err)
    }

    /// A copy of the column with `value` (a :class:`Scalar`) appended as a new last row.
    #[pyo3(signature = (value, safe = true))]
    fn push(&self, value: &crate::scalar::Scalar, safe: bool) -> PyResult<Self> {
        let scalar = value.inner.clone().into_scalar();
        self.inner
            .push(scalar.as_ref(), safe)
            .map(wrap)
            .map_err(serie_err)
    }

    // ---- shape ----

    /// A zero-copy slice of `length` values starting at `offset`.
    fn slice(&self, offset: usize, length: usize) -> Self {
        wrap(self.inner.slice(offset, length))
    }

    /// The first `n` rows (a zero-copy slice).
    fn head(&self, n: usize) -> Self {
        wrap(self.inner.slice(0, n.min(self.inner.len())))
    }

    /// A column of length `new_len`: a slice when shrinking, or extended with fill
    /// (nulls if nullable, else the type default) when growing.
    fn resize(&self, new_len: usize) -> PyResult<Self> {
        self.inner.resize(new_len).map(wrap).map_err(serie_err)
    }

    // ---- transform ----

    /// Cast the column to `dtype` (a :class:`DataType` or type string), converting the
    /// values (lossy / narrowing casts yield null on overflow).
    fn cast(&self, dtype: &Bound<'_, PyAny>) -> PyResult<Self> {
        let dt = resolve_dtype(dtype)?;
        self.inner.cast(&dt).map(wrap).map_err(serie_err)
    }

    /// A **dictionary-encoded** (categorical) view of the column for repeated values.
    fn categorical(&self) -> PyResult<Self> {
        CategoricalSerie::from_serie(self.inner.as_ref())
            .map(|c| wrap(Arc::new(c)))
            .map_err(serie_err)
    }

    /// A fully-materialised, independent copy (a lazy column is computed into a real
    /// array).
    fn materialize(&self) -> Self {
        wrap(self.inner.materialize())
    }

    // ---- nested ----

    /// Navigate a child **node path** (``"a.b.c"``, ``"tags.0"``, ``'["a.b"].c'``) into a
    /// descendant column. Returns ``None`` for a leaf column or an unresolved path;
    /// raises on a malformed path.
    fn select(&self, path: &str) -> PyResult<Option<Serie>> {
        self.inner
            .select(path)
            .map(|opt| opt.map(wrap))
            .map_err(serie_err)
    }

    /// A child column by index (int) or by name (str, case-sensitive then -insensitive),
    /// or ``None``.
    fn child(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<Serie>> {
        let Some(nested) = self.inner.as_nested() else {
            return Ok(None);
        };
        let found = if let Ok(index) = key.extract::<usize>() {
            nested.child(index)
        } else {
            let name: String = key.extract()?;
            nested.child_by_name(&name)
        };
        Ok(found.map(wrap))
    }

    /// All child columns (empty unless this is a nested column).
    fn children(&self) -> Vec<Serie> {
        match self.inner.as_nested() {
            Some(nested) => nested.children().into_iter().map(wrap).collect(),
            None => Vec::new(),
        }
    }

    // ---- range / index ----

    /// Whether this is a canonical ``0..len`` ``uint64`` range index (``start == 0``,
    /// ``step == 1``, the implicit row index) — ``False`` for any other column.
    #[getter]
    fn is_range(&self) -> bool {
        self.inner
            .as_any()
            .downcast_ref::<UInt64RangeSerie>()
            .is_some_and(UInt64RangeSerie::is_range)
    }

    /// The integer label at row `index` (``None`` when out of bounds). Requires a
    /// range/index column.
    fn at(&self, index: usize) -> PyResult<Option<u64>> {
        Ok(as_index(&self.inner)?.at(index))
    }

    /// The first row whose label equals `label`, or ``None``.
    fn position(&self, label: u64) -> PyResult<Option<usize>> {
        Ok(as_index(&self.inner)?.position(label))
    }

    /// Whether `label` is one of the index labels.
    fn contains(&self, label: u64) -> PyResult<bool> {
        Ok(as_index(&self.inner)?.contains(label))
    }

    // ---- categorical ----

    /// The number of distinct categories. Raises if the column is not categorical.
    #[getter]
    fn category_count(&self) -> PyResult<usize> {
        Ok(as_categorical(&self.inner)?.category_count())
    }

    /// The distinct values (the dictionary) as a column named ``"categories"``.
    fn categories(&self) -> PyResult<Serie> {
        as_categorical(&self.inner)?
            .categories()
            .map(wrap)
            .map_err(serie_err)
    }

    /// The dictionary **code** at row `index` (``None`` when null / out of bounds).
    fn code_at(&self, index: usize) -> PyResult<Option<i32>> {
        Ok(as_categorical(&self.inner)?.code_at(index))
    }

    // ---- frame (DataFrame) ----

    /// The frame shape as ``(rows, columns)`` (struct columns only).
    #[getter]
    fn shape(&self) -> PyResult<(usize, usize)> {
        Ok(as_frame(&self.inner)?.shape())
    }

    /// The number of columns (struct columns only).
    #[getter]
    fn num_columns(&self) -> PyResult<usize> {
        Ok(as_frame(&self.inner)?.num_columns())
    }

    /// The column names, in order (struct columns only).
    #[getter]
    fn column_names(&self) -> PyResult<Vec<String>> {
        Ok(as_frame(&self.inner)?
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect())
    }

    /// Project the frame to the named columns, in the requested order.
    fn select_columns(&self, names: Vec<String>) -> PyResult<Self> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        as_frame(&self.inner)?
            .select_columns(&refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// Project **and cast** the frame to an explicit list of :class:`Field`s: each takes
    /// the source column of the same name cast to its type (or a filled column if absent),
    /// in the target order, dropping unlisted columns.
    fn select_fields(&self, fields: Vec<Field>) -> PyResult<Self> {
        let fields: Vec<yggdryl_schema::Field> = fields.into_iter().map(|f| f.inner).collect();
        as_frame(&self.inner)?
            .select_fields(fields)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// A new frame with `column` appended (or replacing an existing column of the same
    /// name). The column length must match the frame's row count.
    fn with_column(&self, column: &Serie) -> PyResult<Self> {
        as_frame(&self.inner)?
            .with_column(column.inner.clone())
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// A new frame without the named columns (absent names are ignored).
    fn drop_columns(&self, names: Vec<String>) -> PyResult<Self> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        as_frame(&self.inner)?
            .drop_columns(&refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// A new frame with column `old` renamed to `new` (a no-op if `old` is absent).
    fn rename(&self, old: &str, new: &str) -> PyResult<Self> {
        as_frame(&self.inner)?
            .rename(old, new)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// The last `n` rows, as a new frame (a zero-copy row slice).
    fn tail(&self, n: usize) -> PyResult<Self> {
        as_frame(&self.inner)?
            .tail(n)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// Keep the rows where `mask` is ``True`` (the mask length must equal the row count).
    fn filter(&self, mask: Vec<bool>) -> PyResult<Self> {
        as_frame(&self.inner)?
            .filter(&mask)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// A new frame with the rows sorted by column `column` (ascending unless
    /// `descending`), reordering every column by the same permutation.
    #[pyo3(signature = (column, descending = false))]
    fn sort_by(&self, column: &str, descending: bool) -> PyResult<Self> {
        as_frame(&self.inner)?
            .sort_by(column, descending)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// Stack `other`'s rows below this frame's (both must share column names and types).
    fn vstack(&self, other: &Serie) -> PyResult<Self> {
        let other = as_frame(&other.inner)?;
        as_frame(&self.inner)?
            .vstack(other)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// A new frame with a ``0..rows`` integer index column named `name` prepended (a lazy
    /// ``uint64`` range, so it costs nothing until materialised).
    fn with_row_index(&self, name: &str) -> PyResult<Self> {
        as_frame(&self.inner)?
            .with_row_index(name)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    /// The record at `index` as a :class:`Scalar` struct — one typed value per column.
    fn row(&self, index: usize) -> PyResult<crate::scalar::Scalar> {
        let record = as_frame(&self.inner)?.row(index).map_err(serie_err)?;
        Ok(crate::scalar::Scalar {
            inner: record.into(),
        })
    }

    /// The frame's rows as a list of native ``dict`` records
    /// (``[{col: value, ...}, ...]``) — the pandas / polars ``to_dicts`` projection.
    fn to_dicts(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let frame = as_frame(&self.inner)?;
        let rows = frame.shape().0;
        (0..rows)
            .map(|i| {
                let row: ScalarValue = frame.row(i).map_err(serie_err)?.into();
                crate::scalar::value_to_py(py, &row)
            })
            .collect()
    }

    /// The frame as an **Arrow IPC stream** — bytes any Arrow library reads back as a
    /// multi-column table (``pyarrow.ipc.open_stream(bytes).read_all()``).
    fn to_arrow_ipc<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = as_frame(&self.inner)?.to_ipc_bytes().map_err(serie_err)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    /// Build a frame named `name` from an **Arrow IPC stream** (as written by
    /// :meth:`to_arrow_ipc` or any Arrow library).
    #[staticmethod]
    fn from_arrow_ipc(name: &str, data: &[u8]) -> PyResult<Self> {
        StructSerie::from_ipc_bytes(name, data)
            .map(|s| wrap(Arc::new(s)))
            .map_err(serie_err)
    }

    // ---- display / serialisation ----

    /// Render the column to a readable string.
    #[pyo3(signature = (max_rows = None, header = true, width = None))]
    fn display(&self, max_rows: Option<usize>, header: bool, width: Option<usize>) -> String {
        let mut opts = DisplayOptions::default().with_header(header);
        if let Some(m) = max_rows {
            opts = opts.with_max_rows(m);
        }
        if let Some(w) = width {
            opts = opts.with_width(w);
        }
        self.inner.display(&opts)
    }

    /// Serialise to lossless Arrow-IPC bytes (round-trips via :meth:`from_bytes`).
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.inner.to_bytes().map_err(serie_err)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.to_bytes(py)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<PyObject> {
        let len = self.inner.len() as isize;
        let idx = if index < 0 { index + len } else { index };
        if idx < 0 || idx >= len {
            return Err(PyIndexError::new_err("serie index out of range"));
        }
        Ok(scalar_to_py(py, &self.inner.value_at(idx as usize)))
    }

    fn __str__(&self) -> String {
        self.inner
            .display(&DisplayOptions::default().with_max_rows(10))
    }

    fn __repr__(&self) -> String {
        format!(
            "Serie('{}', {}, len={})",
            self.inner.name(),
            self.inner.data_type().to_str(),
            self.inner.len()
        )
    }

    /// Value equality: same name / type and the same values.
    fn __eq__(&self, other: &Serie) -> bool {
        if self.inner.field() != other.inner.field() || self.inner.len() != other.inner.len() {
            return false;
        }
        (0..self.inner.len()).all(|i| self.inner.value_at(i) == other.inner.value_at(i))
    }

    fn __hash__(&self) -> u64 {
        crate::hash_str(&format!(
            "{}:{}:{}",
            self.inner.name(),
            self.inner.data_type().to_str(),
            self.inner.len()
        ))
    }

    /// Reconstruct losslessly through Arrow-IPC bytes.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(PyObject, (PyObject,))> {
        let from_bytes = py.get_type_bound::<Self>().getattr("from_bytes")?;
        let bytes = PyBytes::new_bound(py, &self.inner.to_bytes().map_err(serie_err)?);
        Ok((from_bytes.into(), (bytes.into(),)))
    }
}
