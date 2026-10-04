//! The instrument registry: one row per ISIN of every fact it is known by,
//! learned from and filled into market data, bound to a store it is loaded
//! from and committed back to. Shared behind one lock, so a codec handed the
//! registry and the caller holding it see one table.
//!
//! No verb here waits on the lock, reads or writes storage or runs a load
//! or a commit while holding the GIL: each runs detached, taking the lock
//! inside the detached closure, and converts what it answered to Python
//! after. A core thread that logs reaches Python's `logging` through the
//! host and takes the GIL on the way, so a Python thread holding it while
//! waiting on the lock would wait for good.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use yggdryl::holder::Holder;
use yggdryl::{DataType, IsinEntry, IsinRegistry, Mic, Scalar};

use crate::fix::PyFixMsg;
use crate::iobase::{PyIOBase, located_holder};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::ioresult::PyIOResult;
use crate::scalar::{as_py, from_py, struct_from_entries};
use crate::uri::core_url_from_value;
use crate::value_error;

/// A table of instruments keyed by ISIN - each row the instrument's CFI
/// code, its country of issue, its currency pair, its market, its ticker
/// and trading currency and one code per `SecurityIDSource(22)` type - that
/// a lifecycle learns into and fills from, and a parse fills from. Bound to
/// the store it was loaded from, committed back only where it moved.
/// Mutable and shared: equal only to itself, never hashed or pickled; its
/// rows cross out as an Arrow stream.
#[pyclass(name = "IsinRegistry", module = "yggdryl._native", frozen)]
pub(crate) struct PyIsinRegistry {
    pub(crate) inner: Arc<Mutex<IsinRegistry>>,
}

/// The table, a poisoned lock recovered: every verb leaves it whole.
fn lock(inner: &Mutex<IsinRegistry>) -> MutexGuard<'_, IsinRegistry> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PyIsinRegistry {
    /// Wraps the table a codec shares.
    pub(crate) fn from_shared(inner: &Arc<Mutex<IsinRegistry>>) -> Self {
        Self {
            inner: Arc::clone(inner),
        }
    }

    /// Wraps a registry the core built.
    fn from_core(inner: IsinRegistry) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Runs `verb` on the table, detached from the GIL and under the lock,
    /// answering what it answered as an owned value.
    fn with<T: Send>(&self, py: Python<'_>, verb: impl FnOnce(&mut IsinRegistry) -> T + Send) -> T {
        let inner = &self.inner;
        py.detach(|| verb(&mut lock(inner)))
    }

    /// One row as the `dict` of its columns, a fact it does not state
    /// `None`.
    fn entry_as_py(py: Python<'_>, entry: Option<IsinEntry>) -> PyResult<Option<Py<PyAny>>> {
        entry.map(|held| as_py(py, &held.into_scalar())).transpose()
    }
}

/// The market one argument names - a MIC in any spelling the core reads -
/// checked through the `mic` datatype's own value contract.
fn mic_of(value: &Bound<'_, PyAny>) -> PyResult<Mic> {
    match DataType::Mic.scalar(from_py(value)?).map_err(value_error)? {
        Scalar::Mic(mic) => Ok(mic),
        other => Err(PyTypeError::new_err(format!(
            "expected a market identifier code, got {}",
            other.kind()
        ))),
    }
}

/// The `(name, value)` pairs of a `**properties` mapping, each value text.
fn properties_of(properties: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, String)>> {
    let Some(properties) = properties else {
        return Ok(Vec::new());
    };
    properties
        .iter()
        .map(|(name, value)| {
            let name: String = name.extract()?;
            let value: String = value.extract().map_err(|_| {
                PyTypeError::new_err(format!(
                    "a property is text, got {} for {name}",
                    value
                        .get_type()
                        .name()
                        .map_or_else(|_| "an object".to_owned(), |held| held.to_string())
                ))
            })?;
            Ok((name, value))
        })
        .collect()
}

/// Reads `location` - an `IOBase`, or anything a location is read from -
/// through `read` on the holder it names, detached from the GIL.
fn read_located<T: Send>(
    py: Python<'_>,
    location: &Bound<'_, PyAny>,
    read: impl FnOnce(&Holder) -> yggdryl::Result<T> + Send,
) -> PyResult<T> {
    if let Ok(handle) = location.extract::<PyRef<'_, PyIOBase>>() {
        let holder = handle.inner()?;
        return py.detach(|| read(holder)).map_err(value_error);
    }
    let holder = located_holder(&core_url_from_value(location)?)?;
    py.detach(|| read(&holder)).map_err(value_error)
}

#[pymethods]
impl PyIsinRegistry {
    // Shared and mutable: equal only to itself, so never hashed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// An empty registry holding at most `max_instruments` instruments,
    /// bound to no store; learning skips a new ISIN past the bound and
    /// loading refuses it.
    #[new]
    #[pyo3(signature = (max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS))]
    fn new(max_instruments: usize) -> Self {
        Self::from_core(IsinRegistry::new().with_max_instruments(max_instruments))
    }

    /// A registry bound to the store `location` names and loaded from it:
    /// a URL of any scheme this build holds, or a path - an Arrow IPC leaf,
    /// Parquet, a folder of parts, an Iceberg table, an object store - under
    /// the `**properties` a `with (...)` clause would state, its columns
    /// named by the registry's own names or any spelling of an identifier
    /// type; a store holding nothing yet is an empty first run, laid out by
    /// the first `commit`. Clean after the load.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS, **properties))]
    fn from_url(
        py: Python<'_>,
        location: &Bound<'_, PyAny>,
        max_instruments: usize,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        let url = core_url_from_value(location)?;
        let properties = properties_of(properties)?;
        let registry = py
            .detach(|| {
                let holder = Holder::from_url(&url, properties)?;
                IsinRegistry::new()
                    .with_max_instruments(max_instruments)
                    .try_with_holder(holder)
            })
            .map_err(value_error)?;
        Ok(Self::from_core(registry))
    }

    /// The registry the process environment names, loaded on the first
    /// call and shared with every later one and with `FixCodec.from_env`:
    /// an installed registry, else the store `YGGDRYL_ISIN_REGISTRY_URI`
    /// names - a URL of any scheme, a path, `~` the home - else
    /// `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first
    /// `commit` lays out; with no home, an empty registry bound to nothing.
    /// A failed load raises and is retried by the next call.
    #[staticmethod]
    fn from_env(py: Python<'_>) -> PyResult<Self> {
        py.detach(|| IsinRegistry::from_env().map(Self::from_shared))
            .map_err(value_error)
    }

    /// Installs `registry` as the one every later `from_env` answers -
    /// this very table, shared - before anything resolves one; raises once
    /// the default has resolved or been installed.
    #[staticmethod]
    fn install_env(registry: &Bound<'_, Self>) -> PyResult<()> {
        IsinRegistry::install_env_shared(Arc::clone(&registry.get().inner)).map_err(value_error)
    }

    /// A registry read from any Arrow stream - a `pyarrow` reader, table or
    /// batch, or anything exporting `__arrow_c_stream__` - its columns
    /// named as `from_url` reads them; bound to no store, and clean.
    #[staticmethod]
    #[pyo3(signature = (reader, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS))]
    fn from_arrow_reader(
        py: Python<'_>,
        reader: &Bound<'_, PyAny>,
        max_instruments: usize,
    ) -> PyResult<Self> {
        let reader = batch_reader_from_value(reader)?;
        let registry = py
            .detach(|| {
                let registry =
                    IsinRegistry::from_arrow_reader(reader)?.with_max_instruments(max_instruments);
                Ok::<_, yggdryl::arrow::Error>(registry)
            })
            .map_err(value_error)?;
        Ok(Self::from_core(registry))
    }

    /// Folds the rows `location` holds in - an `IOBase`, or anything a
    /// location is read from - by the update rule, leaving the registry
    /// bound to the store it was; how many rows it read.
    fn extend_from_handle(&self, py: Python<'_>, location: &Bound<'_, PyAny>) -> PyResult<usize> {
        let inner = &self.inner;
        read_located(py, location, |holder| {
            lock(inner).extend_from_handle(holder)
        })
    }

    /// Folds an Arrow stream's rows in, by the update rule; how many rows it
    /// read.
    fn extend_from_arrow_reader(
        &self,
        py: Python<'_>,
        reader: &Bound<'_, PyAny>,
    ) -> PyResult<usize> {
        let reader = batch_reader_from_value(reader)?;
        self.with(py, |registry| registry.extend_from_arrow_reader(reader))
            .map_err(value_error)
    }

    /// The rows as a `pyarrow.RecordBatchReader` under the registry's row
    /// field, in ISIN order: a snapshot taken under the lock, streamed after
    /// it is released, which a learn while it streams does not move. Write
    /// it with an `IOBase`'s `write_arrow_reader` - an overwrite saves a
    /// snapshot, a merge by `isin` upserts - or `commit` the registry.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self
            .with(py, |registry| registry.into_arrow_reader())
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Writes the table to the store it is bound to, only where it moved
    /// since it was loaded or last committed: one overwrite of the whole
    /// snapshot, a leaf rewritten, a folder's parts replaced by one, an
    /// Iceberg table replaced in one atomic snapshot across every
    /// partition, an emptied registry truncating a leaf, emptying a table
    /// or removing a folder's parts. The `IOResult` of the write, empty for
    /// a clean registry, which touches the store with no call. Raises on a
    /// registry bound to no store.
    fn commit(&self, py: Python<'_>) -> PyResult<PyIOResult> {
        self.with(py, IsinRegistry::commit)
            .map(PyIOResult::from_core)
            .map_err(value_error)
    }

    /// Whether the table moved since it was loaded or last committed.
    #[getter]
    fn is_dirty(&self, py: Python<'_>) -> bool {
        self.with(py, |registry| registry.is_dirty())
    }

    /// The row of `isin` as a `dict` of its columns, or `None`.
    fn get(&self, py: Python<'_>, isin: &str) -> PyResult<Option<Py<PyAny>>> {
        let entry = self.with(py, |registry| registry.get(isin).cloned());
        Self::entry_as_py(py, entry)
    }

    /// The row the ticker `ticker` names on `market`, as a `dict` of its
    /// columns, or `None`: the one row listing the ticker whose market is
    /// `market` - a MIC, checked by the `mic` datatype - or whose market or
    /// `market` is unstated (`None` or `XXXX`). Two rows answering is
    /// ambiguous, and answers none.
    #[pyo3(signature = (ticker, market=None))]
    fn get_by_ticker(
        &self,
        py: Python<'_>,
        ticker: &str,
        market: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = market.map(mic_of).transpose()?;
        let entry = self.with(py, |registry| {
            registry.get_by_ticker(ticker, market.as_ref()).cloned()
        });
        Self::entry_as_py(py, entry)
    }

    /// Folds one row - a mapping of column names to cells, `isin` required
    /// - into the row of its ISIN by the update rule: a stated valid value
    /// fills a column the row lacks and replaces one it holds that
    /// differs, whatever the time, a code that is no real value of its
    /// type dropped; a compatible CFI code refines the held one and a
    /// contradicting one replaces it; a ticker or a listing code stated on
    /// another market switches the listing whole. Whether anything moved.
    fn merge(&self, py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entry = IsinEntry::from_scalar(&struct_from_entries(entry)?).map_err(value_error)?;
        self.with(py, |registry| registry.merge(entry))
            .map_err(value_error)
    }

    /// Removes the row of `isin`, answering it as a `dict`, or `None`.
    fn remove(&self, py: Python<'_>, isin: &str) -> PyResult<Option<Py<PyAny>>> {
        let removed = self.with(py, |registry| registry.remove(isin));
        Self::entry_as_py(py, removed)
    }

    /// Removes every row.
    fn clear(&self, py: Python<'_>) {
        self.with(py, IsinRegistry::clear);
    }

    /// The most instruments it holds.
    #[getter]
    fn max_instruments(&self, py: Python<'_>) -> usize {
        self.with(py, |registry| registry.max_instruments())
    }

    /// Learns what a message states about its instrument - keyed by its
    /// stated real ISIN, dated at its `currunix`: its CFI code, its market,
    /// its ticker, its currency, the pair it states and its real
    /// equivalents. Whether anything moved.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn learn(&self, py: Python<'_>, message: PyRef<'_, PyFixMsg>) -> bool {
        let message = message.as_inner();
        self.with(py, |registry| registry.learn(message))
    }

    /// Fills what a message leaves unsaid about its instrument from the row
    /// its ISIN names, else its ticker on its market - each equivalent and
    /// the pair as a `derived` identifier, the ticker on its own market,
    /// its CFI code where the row's refines it, the currency on the same
    /// stated market under the row's ticker - never its wire. Whether
    /// anything moved; a hashed message is frozen and refuses with
    /// `TypeError`.
    fn fill(&self, py: Python<'_>, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.with(py, |registry| registry.fill(held)))
    }

    /// `learn`, then `fill`. Whether anything moved in either.
    fn enrich(&self, py: Python<'_>, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.with(py, |registry| registry.enrich(held)))
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.with(py, |registry| registry.len())
    }
    fn __bool__(&self, py: Python<'_>) -> bool {
        self.with(py, |registry| !registry.is_empty())
    }
    /// Whether `other` is this registry - the same shared table.
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| Arc::ptr_eq(&other.get().inner, &self.inner))
    }
    fn __repr__(&self, py: Python<'_>) -> String {
        let (len, max, dirty) = self.with(py, |registry| {
            (
                registry.len(),
                registry.max_instruments(),
                registry.is_dirty(),
            )
        });
        let dirty = if dirty { "True" } else { "False" };
        format!("IsinRegistry(len={len}, max_instruments={max}, dirty={dirty})")
    }
}
