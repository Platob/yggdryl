//! Native identifiers: the name a source gave a thing, as a type of name,
//! and the sorted map an element states them in, keyed `src:type`. A source
//! and a type cross as the words they fold to - `fix`, `clordid` - read by
//! the core's [`IdSource`] and [`IdType`].

use std::hash::{DefaultHasher, Hash, Hasher};

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyList, PyString};
use yggdryl::{IdSource, IdType, Identifier, Identifiers};

use crate::{compare, python_hash, value_error};

/// The source `text` folds to.
fn source_of(text: &str) -> PyResult<IdSource> {
    text.parse().map_err(value_error)
}

/// The type `text` folds to.
fn type_of(text: &str) -> PyResult<IdType> {
    text.parse().map_err(value_error)
}

/// One identifier: a source, a type and a value, unique by its key
/// `src:type`.
#[pyclass(
    name = "Identifier",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyIdentifier {
    pub(crate) inner: Identifier,
}

#[pymethods]
impl PyIdentifier {
    #[new]
    #[pyo3(signature = (src, r#type, value))]
    fn new(src: &str, r#type: &str, value: &str) -> PyResult<Self> {
        Identifier::new(source_of(src)?, type_of(r#type)?, value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The identifier a key names, or `None` where it names none, the value
    /// states nothing or its type refuses the value.
    ///
    /// An explicit `src:type` is read as it is. Otherwise a whole name a
    /// security type is spelled by - `ISINCode`, `security_cusip` - is that
    /// type from `base`, and a security type is never read off a key that
    /// names another instrument's (`underlyingisin`, `legisin`). Otherwise
    /// the key folds - lower case, no `_`, `-`, space or `#` - and the
    /// longest identifier name it ends with is the type: a type the crate
    /// names whose spelling ends with `id`, `account`, `isin`, `cusip`,
    /// `sedol` or `figi`, a parentage word (`parent`, `orig`, `origin`,
    /// `original`) right before it kept inside the type. The source is the
    /// rest of the folded key with its dots trimmed at both ends and kept
    /// inside, `base` where nothing is left.
    ///
    /// `firm.x.ParentOrderID` is `firm.x:parentorderid`, `OMS_InstrumentID`
    /// `oms:instrumentid`, `marketorderid` `market:orderid`, `ISINCode`
    /// `base:isin`; `underlyingisin` and `transversalkey` name none.
    #[staticmethod]
    fn from_key(key: &str, value: &str) -> Option<Self> {
        Identifier::from_key(key, value).map(|inner| Self { inner })
    }

    #[getter]
    fn src(&self) -> &str {
        self.inner.src().as_str()
    }
    #[getter(r#type)]
    fn kind(&self) -> &str {
        self.inner.kind().as_str()
    }
    #[getter]
    fn value(&self) -> &str {
        self.inner.value()
    }
    /// The unique key, `src:type`, the map an `Identifiers` keys it by.
    #[getter]
    fn key(&self) -> String {
        self.inner.key().to_string()
    }

    /// Whether this identifier's key is `src:type`, each folded.
    #[pyo3(signature = (src, r#type))]
    fn is_of(&self, src: &str, r#type: &str) -> PyResult<bool> {
        Ok(self.inner.is_of(&source_of(src)?, &type_of(r#type)?))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let quoted =
            |text: &str| -> PyResult<String> { Ok(PyString::new(py, text).repr()?.to_string()) };
        Ok(format!(
            "Identifier({}, {}, {})",
            quoted(self.inner.src().as_str())?,
            quoted(self.inner.kind().as_str())?,
            quoted(self.inner.value())?
        ))
    }
    fn __hash__(&self) -> isize {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        python_hash(hasher.finish())
    }
    fn __richcmp__(&self, other: &Self, operation: CompareOp) -> bool {
        compare(self.inner.cmp(&other.inner), operation)
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String, String, String)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.src().to_string(),
                self.inner.kind().to_string(),
                self.inner.value().to_owned(),
            ),
        )
    }
}

/// A sorted map of identifiers, one per unique key `src:type`, iterated in
/// key order.
#[pyclass(
    name = "Identifiers",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone, Default)]
pub(crate) struct PyIdentifiers {
    pub(crate) inner: Identifiers,
}

impl PyIdentifiers {
    pub(crate) fn from_core(inner: &Identifiers) -> Self {
        Self {
            inner: inner.clone(),
        }
    }
}

#[pymethods]
impl PyIdentifiers {
    /// The map `ids` fill, the first identifier of a key standing.
    #[new]
    #[pyo3(signature = (ids=Vec::new()))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 extracts a sequence argument as an owned `Vec`.
    fn new(ids: Vec<PyRef<'_, PyIdentifier>>) -> Self {
        Self {
            inner: ids.iter().map(|id| id.inner.clone()).collect(),
        }
    }

    /// The value of the first identifier of `type`, a stated source before
    /// a derived one; `None` where none.
    #[pyo3(signature = (r#type))]
    fn get(&self, r#type: &str) -> PyResult<Option<&str>> {
        Ok(self.inner.get(&type_of(r#type)?))
    }

    /// The first identifier of `type`, a stated source before a derived
    /// one; `None` where none.
    #[pyo3(signature = (r#type))]
    fn get_identifier(&self, r#type: &str) -> PyResult<Option<PyIdentifier>> {
        Ok(self
            .inner
            .get_identifier(&type_of(r#type)?)
            .map(|inner| PyIdentifier {
                inner: inner.clone(),
            }))
    }

    /// The value of the identifier keyed `src:type`; `None` where none.
    #[pyo3(signature = (src, r#type))]
    fn get_from(&self, src: &str, r#type: &str) -> PyResult<Option<&str>> {
        Ok(self.inner.get_from(&source_of(src)?, &type_of(r#type)?))
    }

    /// Whether the set holds an identifier of `type`.
    #[pyo3(signature = (r#type))]
    fn contains_kind(&self, r#type: &str) -> PyResult<bool> {
        Ok(self.inner.contains_kind(&type_of(r#type)?))
    }

    /// Every identifier of `type`, one per source.
    #[pyo3(signature = (r#type))]
    fn of_kind(&self, r#type: &str) -> PyResult<Vec<PyIdentifier>> {
        let kind = type_of(r#type)?;
        Ok(self
            .inner
            .of_kind(&kind)
            .map(|inner| PyIdentifier {
                inner: inner.clone(),
            })
            .collect())
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let ids = PyList::new(
            py,
            self.inner.iter().map(|inner| PyIdentifier {
                inner: inner.clone(),
            }),
        )?;
        Ok(ids.try_iter()?.into_any())
    }
    fn __len__(&self) -> usize {
        self.inner.len()
    }
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!("Identifiers({})", self.inner)
    }
    fn __hash__(&self) -> isize {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        python_hash(hasher.finish())
    }
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| other.get().inner == self.inner)
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (Vec<PyIdentifier>,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self
                .inner
                .iter()
                .map(|inner| PyIdentifier {
                    inner: inner.clone(),
                })
                .collect(),),
        )
    }
}
