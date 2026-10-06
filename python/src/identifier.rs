//! Native identifiers: a value under a key - the source that gave it and the
//! type of name it is - and the sorted map an element states them in. A key
//! crosses as its text, `src:type`, a key from the base source spelled as
//! its type alone (`isin`), read exactly by the core's [`IdKey`]; a type
//! crosses as the word it folds to, read by [`IdType`].

use std::hash::{DefaultHasher, Hash, Hasher};

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyList, PyString};
use yggdryl::{IdKey, IdType, Identifier, Identifiers};

use crate::scalar::{as_py, from_py};
use crate::{compare, python_hash, value_error};

/// The key `text` spells, read exactly.
fn key_of(text: &str) -> PyResult<IdKey> {
    text.parse().map_err(value_error)
}

/// The type `text` folds to.
fn type_of(text: &str) -> PyResult<IdType> {
    text.parse().map_err(value_error)
}

/// One identifier: a value under a key, `src:type`, a key from the base
/// source spelled as its type alone.
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
    /// `key` is read exactly - `src:type`, or a type alone for the base
    /// source, each word folded - and `value` is trimmed text that states
    /// something, held as the key's type stores it: under the `bic` source a
    /// BIC and under `legalentityidentifier` an LEI by shape, whatever the
    /// type, a value either rule refuses a `ValueError` located on the key.
    #[new]
    #[pyo3(signature = (key, value))]
    fn new(key: &str, value: &str) -> PyResult<Self> {
        Identifier::new(key_of(key)?, value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The identifier a name no key spells names, or `None` where it names
    /// none, the value states nothing or its type refuses the value.
    ///
    /// An explicit `src:type` keeps its source. Otherwise a whole name a
    /// security type is spelled by - `ISINCode`, `security_cusip` - is that
    /// type from the base source, and a security type is never read off a
    /// name that names another instrument's (`underlyingisin`, `legisin`).
    /// Otherwise the name folds - lower case, no `_`, `-`, space or `#` -
    /// and the longest identifier name it ends with is the type: a type the
    /// crate names whose spelling ends with `id`, `account`, `isin`,
    /// `cusip`, `sedol` or `figi`, a parentage word (`parent`, `orig`,
    /// `origin`, `original`) right before it kept inside the type. The
    /// source is the rest of the folded name with its dots trimmed at both
    /// ends and kept inside, the base source where nothing is left or where
    /// it folds to a source the crate reserves - `base`, `fix`, `derived` -
    /// which names no namespace: `Derived_ISIN` is `isin`.
    ///
    /// `firm.x.ParentOrderID` is `firm.x:parentorderid`, `OMS_InstrumentID`
    /// `oms:instrumentid`, `marketorderid` `market:orderid`, `ISINCode`
    /// `isin`; `underlyingisin` and `transversalkey` name none.
    #[staticmethod]
    fn from_key(key: &str, value: &str) -> Option<Self> {
        Identifier::from_key(key, value).map(|inner| Self { inner })
    }

    /// Who gave the value: `oms`, `proprietary`, `derived`, `base` where no
    /// source is named.
    #[getter]
    fn src(&self) -> &str {
        self.inner.src().as_str()
    }
    /// The type of name this is: `isin`, `executingtrader`, `clordid`.
    #[getter(r#type)]
    fn kind(&self) -> &str {
        self.inner.kind().as_str()
    }
    /// The value.
    #[getter]
    fn value(&self) -> &str {
        self.inner.value()
    }
    /// The key as an `Identifiers` map spells it: `src:type`, the type
    /// alone for the base source.
    #[getter]
    fn key(&self) -> String {
        self.inner.key().to_string()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let quoted =
            |text: &str| -> PyResult<String> { Ok(PyString::new(py, text).repr()?.to_string()) };
        Ok(format!(
            "Identifier({}, {})",
            quoted(&self.inner.key().to_string())?,
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
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String, String)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.key().to_string(), self.inner.value().to_owned()),
        )
    }
}

/// A sorted map from a key to its value, iterated in key order, whose base
/// key of a type is the type's answer: a named source fills it where it is
/// empty, so `ullink:isin` alone is also `isin`.
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
    /// The map `ids` fill, the first identifier of a key standing and each
    /// named source filling the base key of its type where it is empty.
    #[new]
    #[pyo3(signature = (ids=Vec::new()))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 extracts a sequence argument as an owned `Vec`.
    fn new(ids: Vec<PyRef<'_, PyIdentifier>>) -> Self {
        Self {
            inner: ids.iter().map(|id| id.inner.clone()).collect(),
        }
    }

    /// The map a `dict` from each key's text to its value states - each key
    /// read exactly, `"isin"` the base key and `"ullink:isin"` a named one -
    /// closed so every type held has its base key: a type stating none
    /// takes its first named source's value, else its derivation's.
    ///
    /// A key that reads as no key, a value that states nothing or that its
    /// type refuses, two spellings of one key with two values, and a key or
    /// value that is not text are each a `ValueError` naming the key.
    #[staticmethod]
    fn from_dict(entries: &Bound<'_, PyAny>) -> PyResult<Self> {
        Identifiers::from_scalar(&from_py(entries)?)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The map as a `dict` from each key's text to its value, in key order;
    /// `from_dict` reads it back unchanged.
    #[allow(clippy::wrong_self_convention)]
    fn into_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        as_py(py, &self.inner.into_scalar())
    }

    /// The value of `type`'s base key: the type's answer, whichever source
    /// stated it; `None` where the map holds nothing of the type.
    #[pyo3(signature = (r#type))]
    fn get(&self, r#type: &str) -> PyResult<Option<&str>> {
        Ok(self.inner.get(&type_of(r#type)?))
    }

    /// The value held under exactly `key` - `"isin"`, `"ullink:isin"`;
    /// `None` where none.
    #[pyo3(signature = (key))]
    fn get_from(&self, key: &str) -> PyResult<Option<&str>> {
        Ok(self.inner.get_from(&key_of(key)?))
    }

    /// Whether the map holds anything of `type`.
    #[pyo3(signature = (r#type))]
    fn contains_kind(&self, r#type: &str) -> PyResult<bool> {
        Ok(self.inner.contains_kind(&type_of(r#type)?))
    }

    /// Every identifier of `type`, its base key included, in key order.
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

    /// Whether `type`'s base key holds only a derivation - the value of
    /// `derived:<type>`, which no named source states.
    #[pyo3(signature = (r#type))]
    fn is_derived(&self, r#type: &str) -> PyResult<bool> {
        Ok(self.inner.is_derived(&type_of(r#type)?))
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
    /// Pickled as its `dict`, which `from_dict` reads back raw - a
    /// derivation stays one, where re-inserting the identifiers would read
    /// its base key as a statement.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("from_dict")?.unbind(),
            (self.into_dict(py)?,),
        ))
    }
}
