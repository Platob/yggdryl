//! The `Serie` pyclass — a named, typed, Arrow-backed column (a single dataframe
//! column). A thin wrapper over [`yggdryl_serie`]'s `SerieRef`; all logic lives in the
//! core, so the Python and Node bindings behave identically.

use std::sync::Arc;

use pyo3::exceptions::{PyIndexError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyBytes, PyFloat, PyInt, PyList, PyString};
use yggdryl_schema::DataType as CoreDataType;
use yggdryl_serie::arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, StringArray,
};
use yggdryl_serie::{
    from_array, from_bytes, CategoricalSerie, DisplayOptions, IndexSerie, RangeSerie, Scalar,
    SerieRef, StructSerie,
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
        wrap(Arc::new(RangeSerie::new(name, start, step, length)))
    }

    /// A lazy ``uint64`` row index of `length` rows (`0..length`).
    #[staticmethod]
    fn index(length: usize) -> Self {
        wrap(Arc::new(IndexSerie::range(length)))
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
