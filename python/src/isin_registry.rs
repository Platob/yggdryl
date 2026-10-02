//! The instrument registry: one row per ISIN of the equivalents it is known
//! by, learned from and filled into market data, read from and written to
//! any holder through the record surface. Shared behind one lock, so a
//! codec handed the registry and the caller holding it see one table.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pyo3::prelude::*;
use pyo3::types::PyAny;
use yggdryl::{IsinEntry, IsinRegistry};

use crate::fix::{PyFixMsg, read_located};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::scalar::{as_py, struct_from_entries};
use crate::value_error;

/// A table of instruments keyed by ISIN - each row the instrument's CFI
/// code, its market, its ticker and one code per `SecurityIDSource(22)`
/// type - that a lifecycle learns into and fills from. Mutable and shared:
/// equal only to itself, never hashed or pickled; its rows cross out as an
/// Arrow stream.
#[pyclass(name = "IsinRegistry", module = "yggdryl._native", frozen)]
pub(crate) struct PyIsinRegistry {
    pub(crate) inner: Arc<Mutex<IsinRegistry>>,
}

impl PyIsinRegistry {
    /// The table, a poisoned lock recovered: every verb leaves it whole.
    fn lock(&self) -> MutexGuard<'_, IsinRegistry> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wraps the table a codec shares.
    pub(crate) fn from_shared(inner: &Arc<Mutex<IsinRegistry>>) -> Self {
        Self {
            inner: Arc::clone(inner),
        }
    }

    /// Wraps a registry the core read.
    fn from_core(inner: IsinRegistry) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// One row as the `dict` of its columns, a fact it does not state
    /// `None`.
    fn entry_as_py(py: Python<'_>, entry: Option<&IsinEntry>) -> PyResult<Option<Py<PyAny>>> {
        entry.map(|held| as_py(py, &held.into_scalar())).transpose()
    }
}

#[pymethods]
impl PyIsinRegistry {
    // Shared and mutable: equal only to itself, so never hashed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// An empty registry holding at most `max_instruments` instruments;
    /// learning skips a new ISIN past the bound and loading refuses it.
    #[new]
    #[pyo3(signature = (max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS))]
    fn new(max_instruments: usize) -> Self {
        Self::from_core(IsinRegistry::new().with_max_instruments(max_instruments))
    }

    /// A registry read from `location` - an `IOBase` or anything a location
    /// is read from: an Arrow IPC file, Parquet, a folder of either, an
    /// object store - its columns named by the registry's own names or any
    /// spelling of an identifier type (`RIC`, `BloombergSymbol`,
    /// `ISINCode`), rows of one ISIN folded by `updunix`; a missing store
    /// is the empty registry.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS))]
    fn from_handle(location: &Bound<'_, PyAny>, max_instruments: usize) -> PyResult<Self> {
        let mut registry = IsinRegistry::new().with_max_instruments(max_instruments);
        read_located(location, |handle| registry.extend_from_handle(handle))?;
        Ok(Self::from_core(registry))
    }

    /// A registry read from any Arrow stream - a `pyarrow` reader, table or
    /// batch, or anything exporting `__arrow_c_stream__` - as `from_handle`
    /// reads a holder's rows.
    #[staticmethod]
    #[pyo3(signature = (reader, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS))]
    fn from_arrow_reader(reader: &Bound<'_, PyAny>, max_instruments: usize) -> PyResult<Self> {
        let mut registry = IsinRegistry::new().with_max_instruments(max_instruments);
        registry
            .extend_from_arrow_reader(batch_reader_from_value(reader)?)
            .map_err(value_error)?;
        Ok(Self::from_core(registry))
    }

    /// Folds the rows `location` holds in, by the update rule; how many rows
    /// it read.
    fn extend_from_handle(&self, location: &Bound<'_, PyAny>) -> PyResult<usize> {
        let mut registry = self.lock();
        read_located(location, |handle| registry.extend_from_handle(handle))
    }

    /// Folds an Arrow stream's rows in, by the update rule; how many rows it
    /// read.
    fn extend_from_arrow_reader(&self, reader: &Bound<'_, PyAny>) -> PyResult<usize> {
        let reader = batch_reader_from_value(reader)?;
        self.lock()
            .extend_from_arrow_reader(reader)
            .map_err(value_error)
    }

    /// The rows as a `pyarrow.RecordBatchReader` under the registry's row
    /// field, in ISIN order: a snapshot taken under the lock, streamed after
    /// it is released, which a learn while it streams does not move. Write
    /// it with an `IOBase`'s `write_arrow_reader` - an overwrite saves a
    /// snapshot, a merge by `isin` upserts.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.lock().into_arrow_reader().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// The row of `isin` as a `dict` of its columns, or `None`.
    fn get(&self, py: Python<'_>, isin: &str) -> PyResult<Option<Py<PyAny>>> {
        Self::entry_as_py(py, self.lock().get(isin))
    }

    /// The row the RIC `ric` names, as a `dict` of its columns, or `None`.
    fn get_by_ric(&self, py: Python<'_>, ric: &str) -> PyResult<Option<Py<PyAny>>> {
        Self::entry_as_py(py, self.lock().get_by_ric(ric))
    }

    /// Folds one row - a mapping of column names to cells, `isin` required
    /// - into the row of its ISIN by the update rule: a column the row
    /// lacks is filled, one it holds is replaced by a statement at or after
    /// the row's `updunix` and kept against an older one, a refining CFI
    /// code refines whatever the time. Whether anything moved.
    fn merge(&self, entry: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entry = IsinEntry::from_scalar(&struct_from_entries(entry)?).map_err(value_error)?;
        self.lock().merge(entry).map_err(value_error)
    }

    /// Removes the row of `isin`, answering it as a `dict`, or `None`.
    fn remove(&self, py: Python<'_>, isin: &str) -> PyResult<Option<Py<PyAny>>> {
        let removed = self.lock().remove(isin);
        Self::entry_as_py(py, removed.as_ref())
    }

    /// Removes every row.
    fn clear(&self) {
        self.lock().clear();
    }

    /// The most instruments it holds.
    #[getter]
    fn max_instruments(&self) -> usize {
        self.lock().max_instruments()
    }

    /// Learns what a message states about its instrument - keyed by its
    /// stated ISIN, else by its stated RIC, which only fills - dated at its
    /// `currunix`. Whether anything moved.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn learn(&self, message: PyRef<'_, PyFixMsg>) -> bool {
        self.lock().learn(message.as_inner())
    }

    /// Fills what a message leaves unsaid about its instrument from the row
    /// its ISIN, else its RIC, names - each equivalent as a `derived`
    /// identifier, the listing codes and the ticker on its own market, its
    /// CFI code where the row's refines it - never its wire. Whether
    /// anything moved; a hashed message is frozen and refuses with
    /// `TypeError`.
    fn fill(&self, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.lock().fill(held))
    }

    /// `learn`, then `fill`. Whether anything moved in either.
    fn enrich(&self, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.lock().enrich(held))
    }

    fn __len__(&self) -> usize {
        self.lock().len()
    }
    fn __bool__(&self) -> bool {
        !self.lock().is_empty()
    }
    /// Whether `other` is this registry - the same shared table.
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| Arc::ptr_eq(&other.get().inner, &self.inner))
    }
    fn __repr__(&self) -> String {
        let registry = self.lock();
        format!(
            "IsinRegistry(len={}, max_instruments={})",
            registry.len(),
            registry.max_instruments()
        )
    }
}
