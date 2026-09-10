//! The decoded text row, its entry tree, and the path that addresses one.
//!
//! Every class here redirects into the core value it wraps. Nothing is modelled
//! twice: a path is a core `FieldPath`, a line is a core `TextLine`, and a
//! lookup is the core lookup.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyStopIteration, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple, PyType};

use yggdryl::media::text::{TextBytes, TextEntries, TextEntry, TextLine, TextLines};
use yggdryl::{FieldPath, FieldSegment};

use crate::value_error;

/// Resolve whatever spelling of a path the caller used, exactly once.
///
/// A `FieldPath` passes through; text is parsed here, at the boundary, and
/// never again.
pub(crate) fn core_path_from_value(value: &Bound<'_, PyAny>) -> PyResult<FieldPath> {
    if let Ok(path) = value.extract::<PyRef<'_, PyFieldPath>>() {
        return Ok(path.inner.clone());
    }
    if let Ok(text) = value.extract::<String>() {
        return FieldPath::from_str(&text).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a FieldPath or a path string",
    ))
}

/// Hand bytes over in one copy, at their final size.
fn py_bytes<'py>(py: Python<'py>, bytes: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
    PyBytes::new_with(py, bytes.len(), |target| {
        target.copy_from_slice(bytes);
        Ok(())
    })
}

/// One resolved path into a nested schema or value.
#[pyclass(
    name = "FieldPath",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyFieldPath {
    pub(crate) inner: FieldPath,
}

impl PyFieldPath {
    pub(crate) const fn from_core(inner: FieldPath) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyFieldPath {
    /// Parse one path, or copy one already resolved.
    ///
    /// With nothing to parse this is the root, which selects the value it is
    /// applied to.
    #[new]
    #[pyo3(signature = (value = None))]
    fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        match value {
            Some(value) => Ok(Self::from_core(core_path_from_value(value)?)),
            None => Ok(Self::from_core(FieldPath::root())),
        }
    }

    /// The empty path, which selects the value it is applied to.
    #[classmethod]
    fn root(_class: &Bound<'_, PyType>) -> Self {
        Self::from_core(FieldPath::root())
    }

    /// The segments, each a name or a position.
    #[getter]
    fn segments<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let mut segments: Vec<Py<PyAny>> = Vec::with_capacity(self.inner.len());
        for segment in self.inner.segments() {
            segments.push(match segment.as_name() {
                Some(name) => name.into_pyobject(py)?.into_any().unbind(),
                None => segment
                    .as_index()
                    .unwrap_or_default()
                    .into_pyobject(py)?
                    .into_any()
                    .unbind(),
            });
        }
        PyTuple::new(py, segments)
    }

    /// The single name this path addresses, when it addresses exactly one.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.inner.as_name()
    }

    /// What to call what this path reaches, written `... as name`.
    #[getter]
    fn alias(&self) -> Option<&str> {
        self.inner.alias()
    }

    /// The name this path gives what it reaches.
    ///
    /// The alias where one is written, and the last segment's own name
    /// otherwise. A lifted text column takes this.
    #[getter]
    fn column_name(&self) -> Option<&str> {
        self.inner.column_name()
    }

    /// Whether this path selects the value it is applied to.
    #[getter]
    fn is_root(&self) -> bool {
        self.inner.is_root()
    }

    /// This path without its last segment.
    fn parent(&self) -> Option<Self> {
        self.inner.parent().map(Self::from_core)
    }

    /// This path with one more named or positional segment.
    fn join(&self, segment: &Bound<'_, PyAny>) -> PyResult<Self> {
        let segment = if let Ok(name) = segment.extract::<String>() {
            FieldSegment::field(name)
        } else if let Ok(index) = segment.extract::<i64>() {
            FieldSegment::index(index)
        } else {
            return Err(PyTypeError::new_err(
                "expected a segment name or a position",
            ));
        };
        Ok(Self::from_core(self.inner.join(segment)))
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("FieldPath({:?})", self.inner.to_string())
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }
}

/// One key and value a line declared, with whatever it nested.
#[pyclass(
    name = "TextEntry",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyTextEntry {
    inner: TextEntry,
}

impl PyTextEntry {
    pub(crate) const fn from_core(inner: TextEntry) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTextEntry {
    #[getter]
    fn key<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        py_bytes(py, self.inner.key().as_bytes())
    }

    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        py_bytes(py, self.inner.value().as_bytes())
    }

    #[getter]
    fn entries(&self) -> Option<PyTextEntries> {
        self.inner.entries().cloned().map(PyTextEntries::from_core)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("TextEntry({})", self.inner)
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }
}

/// The ordered entries one line or one nested payload declared.
#[pyclass(
    name = "TextEntries",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyTextEntries {
    inner: TextEntries,
}

impl PyTextEntries {
    pub(crate) const fn from_core(inner: TextEntries) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTextEntries {
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __getitem__(&self, index: isize) -> PyResult<PyTextEntry> {
        let len = self.inner.len();
        let at = if index < 0 {
            len.checked_sub(index.unsigned_abs())
        } else {
            usize::try_from(index).ok()
        };
        at.and_then(|at| self.inner.as_slice().get(at))
            .cloned()
            .map(PyTextEntry::from_core)
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("entry index out of range"))
    }

    /// The entry a path reaches, or `None`.
    fn get_entry_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<PyTextEntry>> {
        let path = core_path_from_value(path)?;
        Ok(self
            .inner
            .get_entry_by_path(&path)
            .cloned()
            .map(PyTextEntry::from_core))
    }

    /// The entry a path reaches, raising absence.
    fn entry_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<PyTextEntry> {
        let path = core_path_from_value(path)?;
        self.inner
            .entry_by_path(&path)
            .map(|held| PyTextEntry::from_core(held.clone()))
            .map_err(value_error)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("TextEntries({})", self.inner)
    }
}

/// One decoded text row, typed the way its columns are.
#[pyclass(name = "TextLine", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyTextLine {
    inner: TextLine,
}

impl PyTextLine {
    pub(crate) const fn from_core(inner: TextLine) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTextLine {
    /// The physical line number within the object, from zero.
    #[getter]
    fn index(&self) -> u64 {
        self.inner.index()
    }

    /// The object this line was read from.
    #[getter]
    fn url(&self) -> Option<crate::uri::PyUrl> {
        self.inner.url().cloned().map(crate::uri::PyUrl::from_core)
    }

    /// When the record was written, in nanoseconds UTC.
    #[getter]
    fn timestamp(&self) -> Option<i128> {
        self.inner.timestamp()
    }

    /// What the line was classified as.
    #[getter]
    fn bodytype(&self) -> Option<crate::enums::PyMimeType> {
        self.inner
            .bodytype()
            .cloned()
            .map(crate::enums::PyMimeType::from_core)
    }

    /// The line, with whatever was read off its front removed.
    #[getter]
    fn body<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        py_bytes(py, self.inner.body().as_bytes())
    }

    /// Which way the line moved.
    #[getter]
    fn direction(&self) -> Option<&'static str> {
        self.inner.direction()
    }

    /// How many bytes of this record went over the retained limit.
    #[getter]
    fn dropped_byte_size(&self) -> Option<u64> {
        self.inner.dropped_byte_size()
    }

    /// The row header's named captures, in the order the expression declares
    /// them.
    #[getter]
    fn captures<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let mut captures: Vec<Py<PyAny>> = Vec::with_capacity(self.inner.captures().len());
        for capture in self.inner.captures() {
            captures.push(match capture {
                Some(value) => py_bytes(py, value.as_bytes())?.into_any().unbind(),
                None => py.None(),
            });
        }
        PyTuple::new(py, captures)
    }

    /// The key/value tree this line carries.
    #[getter]
    fn entries(&self) -> Option<PyTextEntries> {
        self.inner.entries().cloned().map(PyTextEntries::from_core)
    }

    /// The entry a path reaches, or `None`.
    fn get_entry_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<PyTextEntry>> {
        let path = core_path_from_value(path)?;
        Ok(self
            .inner
            .get_entry_by_path(&path)
            .cloned()
            .map(PyTextEntry::from_core))
    }

    /// The entry a path reaches, raising absence.
    fn entry_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<PyTextEntry> {
        let path = core_path_from_value(path)?;
        self.inner
            .entry_by_path(&path)
            .map(|held| PyTextEntry::from_core(held.clone()))
            .map_err(value_error)
    }

    /// Set the value a path reaches, creating what is not there.
    fn set_entry_by_path(&mut self, path: &Bound<'_, PyAny>, value: &[u8]) -> PyResult<()> {
        let path = core_path_from_value(path)?;
        let value = TextBytes::from_bytes(value).map_err(value_error)?;
        self.inner
            .set_entry_by_path(&path, value)
            .map_err(value_error)
    }

    /// Remove the entry a path reaches.
    fn remove_entry_by_path(&mut self, path: &Bound<'_, PyAny>) -> PyResult<Option<PyTextEntry>> {
        let path = core_path_from_value(path)?;
        Ok(self
            .inner
            .remove_entry_by_path(&path)
            .map(PyTextEntry::from_core))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "TextLine(index={}, body={})",
            self.inner.index(),
            self.inner
        )
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }
}

/// A lazy iterator over decoded lines.
///
/// Lines are pulled one at a time, never collected: a read larger than memory
/// iterates exactly as a reader would.
#[pyclass(name = "TextLines", module = "yggdryl._native", unsendable)]
pub(crate) struct PyTextLines {
    inner: Option<TextLines>,
}

impl PyTextLines {
    pub(crate) const fn from_core(inner: TextLines) -> Self {
        Self { inner: Some(inner) }
    }
}

#[pymethods]
impl PyTextLines {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<PyTextLine> {
        let Some(lines) = self.inner.as_mut() else {
            return Err(PyStopIteration::new_err(()));
        };
        match lines.next() {
            Some(Ok(line)) => Ok(PyTextLine::from_core(line)),
            // A failing read fuses: the error is raised once and the iterator
            // stops, exactly as the core iterator does.
            Some(Err(error)) => {
                self.inner = None;
                Err(value_error(error))
            }
            None => {
                self.inner = None;
                Err(PyStopIteration::new_err(()))
            }
        }
    }
}

/// Register every class this module owns.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyFieldPath>()?;
    module.add_class::<PyTextEntry>()?;
    module.add_class::<PyTextEntries>()?;
    module.add_class::<PyTextLine>()?;
    module.add_class::<PyTextLines>()?;
    Ok(())
}
