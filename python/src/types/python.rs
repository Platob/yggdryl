//! `PythonMetadata`, the class declaration a field carries under `python:`.
//!
//! Python is the one runtime that can hand a schema its own declaring class,
//! so the class identity a field remembers is built here once - from a class
//! object, from three strings, or from a field already carrying it - and the
//! interior never re-derives it. `properties` is what a caller merges into a
//! metadata mapping so one `Field` construction carries the declaration, and
//! `Field.python` is the live view for a field that already exists.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

use yggdryl::{PythonKind, PythonMetadata};

use crate::value_error;

/// Read a core form out of a `PythonKind` spelling.
fn core_kind_from_value(value: &Bound<'_, PyAny>) -> PyResult<PythonKind> {
    let spelling = value.extract::<String>().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected a Python class kind, one of {}",
            kind_names()
        ))
    })?;
    PythonKind::from_str(&spelling).map_err(value_error)
}

/// The stored spellings, joined for a refusal that has to list them.
fn kind_names() -> String {
    PythonKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The Python class a field's `python:` properties name.
///
/// Immutable: `module`, `qualname` and `kind` are validated together when the
/// value is built, so a declaration is never half-set and never re-validated
/// on the way out.
#[pyclass(
    name = "PythonMetadata",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyPythonMetadata {
    pub(crate) inner: PythonMetadata,
}

impl PyPythonMetadata {
    pub(crate) fn from_core(inner: PythonMetadata) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyPythonMetadata {
    /// Validate one class declaration.
    #[new]
    #[pyo3(signature = (module, qualname, kind = "class"))]
    fn new(module: &str, qualname: &str, kind: &str) -> PyResult<Self> {
        let kind = PythonKind::from_str(kind).map_err(value_error)?;
        PythonMetadata::new(module, qualname, kind)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Read the declaration a class object states about itself.
    ///
    /// `__module__` and `__qualname__` are what Python itself records, so the
    /// caller states only which form the class takes - the one fact the class
    /// object does not carry in an attribute.
    #[classmethod]
    #[pyo3(signature = (value, kind = "class"))]
    fn from_type(_cls: &Bound<'_, PyType>, value: &Bound<'_, PyAny>, kind: &str) -> PyResult<Self> {
        let module = value
            .getattr("__module__")
            .and_then(|module| module.extract::<String>())
            .map_err(|_| PyTypeError::new_err("expected a class with a string __module__"))?;
        // A `TypeAliasType` and a `NewType` carry `__name__` without a
        // `__qualname__`; Python's own qualified name for them is the bare one.
        let qualname = match value.getattr("__qualname__") {
            Ok(qualname) => qualname.extract::<String>().ok(),
            Err(_) => None,
        };
        let qualname = match qualname {
            Some(qualname) => qualname,
            None => value
                .getattr("__name__")
                .and_then(|name| name.extract::<String>())
                .map_err(|_| {
                    PyTypeError::new_err("expected a class with a string __name__ or __qualname__")
                })?,
        };
        Self::new(&module, &qualname, kind)
    }

    /// Every form a declaration can take, in declaration order.
    #[classattr]
    #[allow(non_snake_case)]
    fn KINDS() -> Vec<&'static str> {
        PythonKind::ALL.iter().map(|kind| kind.as_str()).collect()
    }

    /// The dotted module path the class is declared in.
    #[getter]
    fn module(&self) -> &str {
        self.inner.module()
    }

    /// The qualified name the class has inside its module.
    #[getter]
    fn qualname(&self) -> &str {
        self.inner.qualname()
    }

    /// The bare class name, the last segment of the qualified name.
    ///
    /// Derived rather than stored: Python's `__name__` is always the last
    /// segment of its `__qualname__`, so storing both would let one disagree.
    #[getter]
    fn class_name(&self) -> &str {
        self.inner.class_name()
    }

    /// Which Python form the declaration takes.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// The dotted path an importing reader would spell.
    #[getter]
    fn import_path(&self) -> String {
        self.inner.import_path()
    }

    /// Whether `import_path` actually resolves to this class.
    ///
    /// A class declared inside a function body carries `<locals>` in its
    /// qualified name and no import reaches it.
    #[getter]
    fn is_importable(&self) -> bool {
        self.inner.is_importable()
    }

    /// Whether this form is constructed by keyword rather than by position.
    #[getter]
    fn is_keyword_constructed(&self) -> bool {
        self.inner.kind().is_keyword_constructed()
    }

    /// The three metadata entries this declaration stores.
    ///
    /// Merging these into a mapping is what lets one `Field` construction
    /// carry the declaration, without a caller ever spelling a `python:` key.
    #[getter]
    fn properties<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let properties = PyDict::new(py);
        for (key, value) in self.inner.properties() {
            properties.set_item(key, value)?;
        }
        Ok(properties)
    }

    fn __str__(&self) -> String {
        self.inner.import_path()
    }

    fn __repr__(&self) -> String {
        format!(
            "PythonMetadata({:?}, {:?}, {:?})",
            self.inner.module(),
            self.inner.qualname(),
            self.inner.kind().as_str()
        )
    }

    /// A deterministic cross-language hash of the whole declaration.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        operation: pyo3::basic::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String, String, &'static str)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.module().to_owned(),
                self.inner.qualname().to_owned(),
                self.inner.kind().as_str(),
            ),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Read a core declaration out of a `PythonMetadata` or its three parts.
pub(crate) fn core_python_metadata_from_value(
    value: &Bound<'_, PyAny>,
) -> PyResult<PythonMetadata> {
    if let Ok(declared) = value.extract::<PyRef<'_, PyPythonMetadata>>() {
        return Ok(declared.inner.clone());
    }
    if let Ok((module, qualname, kind)) = value.extract::<(String, String, Bound<'_, PyAny>)>() {
        let kind = core_kind_from_value(&kind)?;
        return PythonMetadata::new(module, qualname, kind).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a PythonMetadata or a (module, qualname, kind) tuple",
    ))
}
