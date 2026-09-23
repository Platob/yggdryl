//! Python's native view of the shared [`Serie`]: many values, as a
//! schema-free run or as the Arrow buffers of one field.
//!
//! [`PySerie`] owns only the core value and redirects every verb to it. It is
//! mutable - a write lands in the buffers the column holds - so it has
//! equality over its rows and no hash, which is Python's contract for a
//! mutable container. Arrow crosses by sharing buffers through the C Data and
//! C Stream interfaces, exactly as [`crate::arrow::PyArrowScalar`] does.
//!
//! A nested column is a subclass named for its leaf - `ListSerie`,
//! `LargeListSerie`, `ListViewSerie`, `LargeListViewSerie`,
//! `FixedSizeListSerie`, `MapSerie`, `StructSerie` - adding the verbs that
//! leaf lends: its offsets, the rows it cuts, its entries. Every serie handed
//! out goes through [`described`], so a record's child, a list's items and
//! one row of them come back as their own leaf, all the way down. A write
//! never changes a column's leaf, so the class an object has stays true.

use std::borrow::Cow;

use pyo3::IntoPyObjectExt;
use pyo3::PyClassInitializer;
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyList, PySlice};
use yggdryl::{Field as CoreField, FieldPath, Scalar, Serie};

use crate::datatype::{PyDataType, arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::field::{PyField, core_field_from_value};
use crate::iomedia::{
    batch_reader_from_value, batch_reader_to_pyarrow, batch_to_pyarrow, record_batch_from_value,
};
use crate::scalar::{PyScalar, PyScalarIterator, as_py, as_py_with_field, from_py};
use crate::{compare, normalize_index, value_error};

/// Many values: a schema-free run, or the Arrow buffers of one field.
#[pyclass(
    name = "Serie",
    module = "yggdryl._native",
    subclass,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySerie {
    pub(crate) inner: Serie,
}

impl PySerie {
    pub(crate) const fn from_inner(inner: Serie) -> Self {
        Self { inner }
    }

    /// Resolve a Python index against the serie, negative from the end.
    fn index(&self, index: isize) -> PyResult<usize> {
        normalize_index(index, self.inner.len()).ok_or_else(|| PyIndexError::new_err(index))
    }
}

/// The invariant [`described`] keeps: a nested class is built only over the
/// leaf it is named for, and no write changes a column's leaf.
const LEAF: &str = "a nested Serie class holds the leaf it is named for";

/// Hand `serie` to Python as the class its leaf is named for.
pub(crate) fn described(py: Python<'_>, serie: Serie) -> PyResult<Py<PyAny>> {
    let leaf = Leaf::of(&serie);
    let base = PyClassInitializer::from(PySerie::from_inner(serie));
    Ok(match leaf {
        Leaf::Other => Py::new(py, base)?.into_any(),
        Leaf::List => Py::new(py, base.add_subclass(PyListSerie))?.into_any(),
        Leaf::LargeList => Py::new(py, base.add_subclass(PyLargeListSerie))?.into_any(),
        Leaf::ListView => Py::new(py, base.add_subclass(PyListViewSerie))?.into_any(),
        Leaf::LargeListView => Py::new(py, base.add_subclass(PyLargeListViewSerie))?.into_any(),
        Leaf::FixedSizeList => Py::new(py, base.add_subclass(PyFixedSizeListSerie))?.into_any(),
        Leaf::Map => Py::new(py, base.add_subclass(PyMapSerie))?.into_any(),
        Leaf::Struct => Py::new(py, base.add_subclass(PyStructSerie))?.into_any(),
    })
}

/// Hand every serie of `series` to Python as its own class.
fn described_all<'a>(
    py: Python<'_>,
    series: impl IntoIterator<Item = &'a Serie>,
) -> PyResult<Vec<Py<PyAny>>> {
    series
        .into_iter()
        .map(|serie| described(py, serie.clone()))
        .collect()
}

/// What `_from_pickle` takes back: the rows, and the field they are under.
type PickleArguments = (Vec<PyScalar>, Option<PyField>);

/// Which class a serie is handed out as.
#[derive(Clone, Copy)]
enum Leaf {
    Other,
    List,
    LargeList,
    ListView,
    LargeListView,
    FixedSizeList,
    Map,
    Struct,
}

impl Leaf {
    fn of(serie: &Serie) -> Self {
        match serie {
            Serie::List(_) => Self::List,
            Serie::LargeList(_) => Self::LargeList,
            Serie::ListView(_) => Self::ListView,
            Serie::LargeListView(_) => Self::LargeListView,
            Serie::FixedSizeList(_) => Self::FixedSizeList,
            Serie::Map(_) | Serie::SortedMap(_) => Self::Map,
            Serie::Struct(_) => Self::Struct,
            _ => Self::Other,
        }
    }
}

/// Convert every Python value in `rows` to a core value, once.
fn rows_from_py(rows: &Bound<'_, PyAny>) -> PyResult<Vec<Scalar>> {
    if let Ok(serie) = rows.extract::<PyRef<'_, PySerie>>() {
        return Ok(serie.inner.rows().into_owned());
    }
    rows.try_iter()?
        .map(|row| from_py(&row?))
        .collect::<PyResult<Vec<Scalar>>>()
}

/// Resolve an optional Python field argument once.
fn field_of(field: Option<&Bound<'_, PyAny>>) -> PyResult<Option<CoreField>> {
    field.map(core_field_from_value).transpose()
}

/// Name the field one foreign column proves about itself.
fn inferred_field(array: &arrow_array::ArrayRef) -> PyResult<CoreField> {
    let dtype = yggdryl::DataType::try_from(array.data_type().clone()).map_err(value_error)?;
    Ok(CoreField::new("item", dtype, array.null_count() != 0))
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PySerie {
    /// A schema-free run of `values`, each converted once; a column is
    /// built by [`Self::from_scalars`] and the Arrow doors.
    #[new]
    #[pyo3(signature = (values = None))]
    fn new(values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let rows = values.map(rows_from_py).transpose()?.unwrap_or_default();
        Ok(Self::from_inner(Serie::new(rows)))
    }

    /// The column `field` types `rows` into, each through its contract once.
    #[staticmethod]
    fn from_scalars(
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let serie = Serie::from_scalars(core_field_from_value(field)?, rows_from_py(rows)?)
            .map_err(value_error)?;
        described(py, serie)
    }

    /// The empty column of `field`.
    #[staticmethod]
    fn empty(py: Python<'_>, field: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::empty(core_field_from_value(field)?).map_err(value_error)?,
        )
    }

    /// The empty column of `field`, with room for `rows` rows.
    #[staticmethod]
    fn with_capacity(py: Python<'_>, field: &Bound<'_, PyAny>, rows: usize) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::with_capacity(core_field_from_value(field)?, rows).map_err(value_error)?,
        )
    }

    /// Take one Arrow array as the column of `field`, sharing its buffers.
    ///
    /// Without `field`, the column is named `item` and typed by what the
    /// array proves about itself.
    #[staticmethod]
    #[pyo3(signature = (array, field = None))]
    fn from_arrow_array(
        py: Python<'_>,
        array: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let array = arrow_array_from_pyarrow(array)?;
        let field = match field_of(field)? {
            Some(field) => field,
            None => inferred_field(&array)?,
        };
        described(
            py,
            Serie::from_arrow_array(field, array).map_err(value_error)?,
        )
    }

    /// Take one record batch as a record column named `row`.
    #[staticmethod]
    fn from_arrow_batch(py: Python<'_>, batch: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::from_arrow_batch(&record_batch_from_value(batch)?).map_err(value_error)?,
        )
    }

    /// Drain a record batch stream into one record column named `row`.
    #[staticmethod]
    fn from_arrow_reader(py: Python<'_>, reader: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::from_arrow_reader(batch_reader_from_value(reader)?).map_err(value_error)?,
        )
    }

    /// Rebuild a pickled serie: its rows, under its field when it had one.
    #[staticmethod]
    #[pyo3(signature = (rows, field = None))]
    fn _from_pickle(
        py: Python<'_>,
        rows: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let rows = rows_from_py(rows)?;
        let serie = match field_of(field)? {
            Some(field) => Serie::from_scalars(field, rows).map_err(value_error)?,
            None => Serie::new(rows),
        };
        described(py, serie)
    }

    /// The field a column carries, or `None` for a run.
    #[getter]
    fn field(&self) -> Option<PyField> {
        self.inner.field().cloned().map(PyField::from_inner)
    }

    /// `list(<the field named item>)` for a column; agreed out of a run's rows.
    #[getter]
    fn dtype(&self) -> PyResult<PyDataType> {
        self.inner
            .dtype()
            .map(PyDataType::from_inner)
            .map_err(value_error)
    }

    /// Whether this is a column rather than a schema-free run.
    #[getter]
    fn is_column(&self) -> bool {
        self.inner.is_column()
    }

    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn is_null(&self, index: usize) -> PyResult<bool> {
        self.inner.is_null(index).map_err(value_error)
    }

    /// Row `index`, built as one value.
    fn scalar(&self, index: usize) -> PyResult<PyScalar> {
        self.inner
            .scalar(index)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Row `index`, or `None` past the end.
    fn get(&self, index: usize) -> Option<PyScalar> {
        self.inner
            .get(index)
            .map(|row| PyScalar::from_inner(row.into_owned()))
    }

    /// Every row, each built once and kept by the list alone.
    fn rows(&self) -> Vec<PyScalar> {
        self.inner
            .rows()
            .into_owned()
            .into_iter()
            .map(PyScalar::from_inner)
            .collect()
    }

    /// Every row as the Python value it is, a record as a `dict`.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        match self.inner.field() {
            Some(field) => {
                for row in &self.inner {
                    list.append(as_py_with_field(py, &row, field)?)?;
                }
            }
            None => {
                for row in &self.inner {
                    list.append(as_py(py, &row)?)?;
                }
            }
        }
        Ok(list.into_any().unbind())
    }

    /// This serie as the one value a `Scalar` sequence holds.
    fn into_scalar(&self) -> PyScalar {
        PyScalar::from_inner(Scalar::from(self.inner.clone()))
    }

    /// The run of this serie's rows, dropping the field.
    fn into_run(&self) -> Self {
        Self::from_inner(Serie::from(self.inner.clone().into_run()))
    }

    /// `length` rows from `offset`, sharing a column's buffers.
    fn slice(&self, py: Python<'_>, offset: usize, length: usize) -> PyResult<Py<PyAny>> {
        described(py, self.inner.slice(offset, length).map_err(value_error)?)
    }

    /// A record column's child named `name`, or `None`.
    fn child(&self, py: Python<'_>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .child(name)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// A record column's child, or a union's member, at `index`.
    fn child_at(&self, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .child_at(index)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Every child of a record column, or member of a union.
    fn children(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        described_all(py, self.inner.children())
    }

    /// A sequence column's items, a mapping's entries, an encoding's values.
    fn items(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .items()
            .map(|items| described(py, items.clone()))
            .transpose()
    }

    /// The column `path` reaches, spelled as a field path.
    fn get_child_by_path(&self, py: Python<'_>, path: &str) -> PyResult<Option<Py<PyAny>>> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        self.inner
            .get_child_by_path(&path)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Replace rows `start..end` by `rows`: the one mutation.
    fn splice(&mut self, start: usize, end: usize, rows: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .splice(start..end, rows_from_py(rows)?)
            .map_err(value_error)
    }

    fn set(&mut self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.set(index, from_py(value)?).map_err(value_error)
    }

    fn push(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.push(from_py(value)?).map_err(value_error)
    }

    fn insert(&mut self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .insert(index, from_py(value)?)
            .map_err(value_error)
    }

    /// Remove row `index` and answer it.
    fn remove(&mut self, index: usize) -> PyResult<PyScalar> {
        self.inner
            .remove(index)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Remove the last row and answer it, or `None` when empty.
    fn pop(&mut self) -> PyResult<Option<PyScalar>> {
        self.inner
            .pop()
            .map(|row| row.map(PyScalar::from_inner))
            .map_err(value_error)
    }

    fn truncate(&mut self, len: usize) -> PyResult<()> {
        self.inner.truncate(len).map_err(value_error)
    }

    fn clear(&mut self) -> PyResult<()> {
        self.inner.clear().map_err(value_error)
    }

    fn extend(&mut self, rows: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.extend(rows_from_py(rows)?).map_err(value_error)
    }

    /// Append every row of `other`, buffer to buffer where the fields agree.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn extend_from_serie(&mut self, other: PyRef<'_, Self>) -> PyResult<()> {
        self.inner
            .extend_from_serie(&other.inner)
            .map_err(value_error)
    }

    fn resize(&mut self, len: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.resize(len, from_py(value)?).map_err(value_error)
    }

    /// Replace a record column's child of `child`'s name, or add it.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn set_child(&mut self, child: PyRef<'_, Self>) -> PyResult<()> {
        self.inner
            .set_child(child.inner.clone())
            .map_err(value_error)
    }

    /// Write one cell of row `index`, `path` deep, in place.
    fn set_cell(&mut self, path: &str, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        self.inner
            .set_cell(&path, index, from_py(value)?)
            .map_err(value_error)
    }

    /// The column's buffers as a `pyarrow.Array`, shared; a run has none.
    fn into_arrow_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.require_arrow_array().map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, self.inner.field())
    }

    /// A record column as one `pyarrow.RecordBatch` of its children.
    fn into_arrow_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        batch_to_pyarrow(py, self.inner.into_arrow_batch().map_err(value_error)?)
    }

    /// A record column as a `pyarrow.RecordBatchReader` of one batch.
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        batch_reader_to_pyarrow(py, self.inner.into_arrow_reader().map_err(value_error)?)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Row `key`, negative from the end, or the window a step-1 slice names.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<PySlice>() {
            let window = slice.indices(isize::try_from(self.inner.len()).unwrap_or(isize::MAX))?;
            if window.step != 1 {
                return Err(PyValueError::new_err(
                    "a Serie slices with a step of 1 only",
                ));
            }
            let start = usize::try_from(window.start).unwrap_or(0);
            let length = window.slicelength;
            return self.slice(py, start, length);
        }
        if key.is_instance_of::<pyo3::types::PyBool>() {
            return Err(PyTypeError::new_err("Serie indexes must be int"));
        }
        let index = key
            .extract::<isize>()
            .map_err(|_| PyTypeError::new_err("Serie indexes must be int or slice"))?;
        self.scalar(self.index(index)?)?.into_py_any(py)
    }

    fn __setitem__(&mut self, index: isize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let index = self.index(index)?;
        self.set(index, value)
    }

    fn __delitem__(&mut self, index: isize) -> PyResult<()> {
        let index = self.index(index)?;
        self.remove(index).map(|_| ())
    }

    fn __iter__(&self) -> PyScalarIterator {
        PyScalarIterator::new(self.inner.iter().map(Cow::into_owned))
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let value = from_py(value)?;
        Ok(self.inner.iter().any(|row| *row == value))
    }

    /// The call that rebuilds this serie, its rows spelled as Python values.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let serie = slf.borrow();
        let rows = serie.as_py(slf.py())?.bind(slf.py()).repr()?.to_string();
        Ok(match serie.field() {
            Some(field) => {
                let field = Bound::new(slf.py(), field)?.repr()?.to_string();
                format!("Serie.from_scalars({field}, {rows})")
            }
            None => format!("Serie({rows})"),
        })
    }

    /// Equality and order over the rows alone, as the core defines them.
    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        compare(self.inner.cmp(&other.inner), operation).into_py_any(other.py())
    }

    // A serie is mutable, so it cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Rebuild through `_from_pickle` from the rows and the field.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, PickleArguments)> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.rows(), self.field()),
        ))
    }

    /// A copy sharing the buffers, of the same class.
    fn __copy__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(py, self.inner.clone())
    }

    fn __deepcopy__(&self, py: Python<'_>, _memo: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(py, self.inner.clone())
    }
}

/// Declare one nested leaf class below [`PySerie`].
macro_rules! nested {
    ($ident:ident, $name:literal, $doc:expr) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", extends = PySerie, skip_from_py_object)]
        pub(crate) struct $ident;
    };
}

nested!(
    PyListSerie,
    "ListSerie",
    "A list column: `int32` offsets over one item column."
);
nested!(
    PyLargeListSerie,
    "LargeListSerie",
    "A large list column: `int64` offsets over one item column."
);
nested!(
    PyListViewSerie,
    "ListViewSerie",
    "A list-view column: `int32` offsets and sizes over one item column."
);
nested!(
    PyLargeListViewSerie,
    "LargeListViewSerie",
    "A large list-view column: `int64` offsets and sizes over one item column."
);
nested!(
    PyFixedSizeListSerie,
    "FixedSizeListSerie",
    "A fixed-size list column: `width` items per row."
);
nested!(
    PyMapSerie,
    "MapSerie",
    "A map column: `int32` offsets over one record column of entries."
);
nested!(
    PyStructSerie,
    "StructSerie",
    "A record column: one child column per child field."
);

/// The row range `range` answers, as the Python pair it is.
fn pair(range: Option<std::ops::Range<usize>>) -> Option<(usize, usize)> {
    range.map(|range| (range.start, range.end))
}

/// Emit the verbs every offsets-cut list leaf lends, over its core leaf.
macro_rules! offset_list {
    ($class:ident, $narrow:ident) => {
        #[pymethods]
        impl $class {
            /// The offsets cut, one more than the rows: row `i` is items
            /// `offsets[i]..offsets[i + 1]`.
            #[getter]
            fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.offsets()
                    .iter()
                    .map(|offset| i64::from(*offset))
                    .collect()
            }

            /// The items row `index` holds, or `None` past the end.
            fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
                pair(slf.as_super().inner.$narrow().expect(LEAF).range(index))
            }

            /// The item column cut to row `index`, sharing its buffers, or
            /// `None` past the end.
            fn row(
                slf: PyRef<'_, Self>,
                py: Python<'_>,
                index: usize,
            ) -> PyResult<Option<Py<PyAny>>> {
                let row = slf.as_super().inner.$narrow().expect(LEAF).row(index);
                row.map(|row| described(py, row)).transpose()
            }
        }
    };
}

offset_list!(PyListSerie, as_list);
offset_list!(PyLargeListSerie, as_large_list);

/// Emit the verbs every list-view leaf lends, over its core leaf.
macro_rules! view_list {
    ($class:ident, $narrow:ident) => {
        #[pymethods]
        impl $class {
            /// Where each row's items start.
            #[getter]
            fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.offsets()
                    .iter()
                    .map(|offset| i64::from(*offset))
                    .collect()
            }

            /// How many items each row holds.
            #[getter]
            fn sizes(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.sizes().iter().map(|size| i64::from(*size)).collect()
            }

            /// The items row `index` holds, or `None` past the end.
            fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
                pair(slf.as_super().inner.$narrow().expect(LEAF).range(index))
            }

            /// The item column cut to row `index`, sharing its buffers, or
            /// `None` past the end.
            fn row(
                slf: PyRef<'_, Self>,
                py: Python<'_>,
                index: usize,
            ) -> PyResult<Option<Py<PyAny>>> {
                let row = slf.as_super().inner.$narrow().expect(LEAF).row(index);
                row.map(|row| described(py, row)).transpose()
            }
        }
    };
}

view_list!(PyListViewSerie, as_list_view);
view_list!(PyLargeListViewSerie, as_large_list_view);

#[pymethods]
impl PyFixedSizeListSerie {
    /// How many items every row holds.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn width(slf: PyRef<'_, Self>) -> usize {
        slf.as_super()
            .inner
            .as_fixed_size_list()
            .expect(LEAF)
            .width()
    }

    /// The items row `index` holds, or `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
        pair(
            slf.as_super()
                .inner
                .as_fixed_size_list()
                .expect(LEAF)
                .range(index),
        )
    }

    /// The item column cut to row `index`, sharing its buffers, or `None`
    /// past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn row(slf: PyRef<'_, Self>, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        let row = slf
            .as_super()
            .inner
            .as_fixed_size_list()
            .expect(LEAF)
            .row(index);
        row.map(|row| described(py, row)).transpose()
    }
}

#[pymethods]
impl PyMapSerie {
    /// The entries: one record column of the entries field, a key and a
    /// value per entry.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn entries(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).entries().clone(),
        )
    }

    /// The entries' key column.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn keys(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).keys().clone(),
        )
    }

    /// The entries' value column.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn values(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).values().clone(),
        )
    }

    /// The offsets cut: row `i` is entries `offsets[i]..offsets[i + 1]`.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
        let leaf = slf.as_super().inner.as_map().expect(LEAF);
        leaf.offsets()
            .iter()
            .map(|offset| i64::from(*offset))
            .collect()
    }

    /// Whether the field declares every row's keys sorted.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn keys_sorted(slf: PyRef<'_, Self>) -> bool {
        slf.as_super().inner.as_map().expect(LEAF).keys_sorted()
    }

    /// The entries row `index` holds, or `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
        pair(slf.as_super().inner.as_map().expect(LEAF).range(index))
    }

    /// The entries column cut to row `index`, sharing its buffers, or
    /// `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn row(slf: PyRef<'_, Self>, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        let row = slf.as_super().inner.as_map().expect(LEAF).row(index);
        row.map(|row| described(py, row)).transpose()
    }
}

#[pymethods]
impl PyStructSerie {
    /// The child fields' names, in the record's order.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn names(slf: PyRef<'_, Self>) -> Vec<String> {
        slf.as_super()
            .inner
            .children()
            .iter()
            .filter_map(|child| child.field().map(|field| field.name().to_owned()))
            .collect()
    }

    /// This record without the child named `name`, sharing every other.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn without_child(slf: PyRef<'_, Self>, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let leaf = slf.as_super().inner.as_struct().expect(LEAF);
        let without = leaf.without_child(name).map_err(value_error)?;
        described(py, yggdryl::SerieValue::into_serie(without))
    }
}
