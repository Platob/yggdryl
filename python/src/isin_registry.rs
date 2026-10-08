//! The instrument registry: one row per ISIN and market of every fact it is
//! known by, learned from and filled into market data, bound to a store it is loaded
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
use pyo3::types::{PyAny, PyDict, PyTuple};
use yggdryl::holder::Holder;
use yggdryl::{
    DataType, IdType, Isin, IsinEntry, IsinRegistry, MatchTier, Mic, Resolution, Scalar, Unmatched,
};

use crate::field::PyField;

use crate::fix::PyFixMsg;
use crate::graph::market_data::market_data_of;
use crate::iobase::{PyIOBase, located_holder};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::ioresult::PyIOResult;
use crate::scalar::{as_py, from_py, struct_from_entries};
use crate::uri::core_url_from_value;
use crate::value_error;

/// A table of instruments keyed by ISIN, one row per listing - per ISIN and
/// market. The instrument facts - the CFI code, the country of issue, the
/// currency pair, the instrument it is written on, the EUSIPA product
/// category, the ISO 18774 short name, the origin currency, `updunix`,
/// `firstunix`, `lastunix` and every code that is no listing code - are the
/// ISIN's and every listing row of it carries them; the listing facts - the
/// market, the ticker, the trading currency and the listing codes - are
/// each row's own. A lifecycle learns into it and fills from it, and a
/// parse fills from it; `resolve` answers which row an element names and
/// how - its ISIN, a code of `LOOKUP_CODES`, its ticker, or its short name
/// in its currency. Bound to the store it was loaded from, committed back
/// only where it moved.
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

/// Binds a registry to the store `location` names - an `IOBase` already
/// built, or anything a location is read from, under `properties` - through
/// `load`, which runs detached from the GIL with the store's holder.
fn bound(
    py: Python<'_>,
    location: &Bound<'_, PyAny>,
    properties: Option<&Bound<'_, PyDict>>,
    load: impl FnOnce(Holder) -> yggdryl::Result<IsinRegistry> + Send,
) -> PyResult<PyIsinRegistry> {
    let properties = properties_of(properties)?;
    if let Ok(handle) = location.extract::<PyRef<'_, PyIOBase>>() {
        if !properties.is_empty() {
            return Err(PyTypeError::new_err(
                "properties apply to a location, not to a handle already built",
            ));
        }
        let holder = handle.rebuilt()?;
        drop(handle);
        return py
            .detach(move || load(holder))
            .map(PyIsinRegistry::from_core)
            .map_err(value_error);
    }
    let url = core_url_from_value(location)?;
    py.detach(|| load(Holder::from_url(&url, properties)?))
        .map(PyIsinRegistry::from_core)
        .map_err(value_error)
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

    /// The registry's row: the required struct `isinregistry` every listing
    /// row is laid out as - `isin`, `updunix`, `firstunix` (the earliest
    /// instant an event the registry learned from stated the ISIN),
    /// `lastunix` (the latest), `cficode`, `countrycode`, `forexcode`,
    /// `underlyingisin`, `eusipacode` (`int32`, the four-digit code `Eusipa`
    /// reads), `miccode`, `ticker`, `fisn` (the ISO 18774 short name),
    /// `currency`, `origccy` (the currency the instrument was issued in,
    /// where stated), then one column per `SecurityIDSource(22)` type but
    /// the ISIN: forty-six columns - what a
    /// table holding the registry is created from. Its root declares
    /// `PARTITION:by` `["truncate(isin, 2)"]` - an Iceberg table created
    /// from it partitions by the ISIN's country prefix, storing no column -
    /// and `SORT:by` `["isin", "miccode"]`, the order the snapshot streams
    /// in.
    #[staticmethod]
    fn field() -> PyField {
        PyField::from_inner(IsinEntry::field())
    }

    /// A registry holding the seed - the common instruments
    /// `config/isin/instruments.json` states, embedded at build time: each a
    /// stock, a fund or an index by its ISIN, its ticker, its market but an
    /// index's, its trading currency, its country, its detailed CFI code and
    /// its short name where one is known - clean and bound to no store. A
    /// seed row is an ordinary statement, so the facts it implies - the
    /// national number its ISIN embeds, the currency of its market's country
    /// - are derived as for any other; `IsinRegistry()` holds none of it.
    #[staticmethod]
    fn seeded(py: Python<'_>) -> Self {
        py.detach(|| Self::from_core(IsinRegistry::seeded()))
    }

    /// A registry bound to the store `location` names and loaded from it:
    /// a URL of any scheme this build holds, or a path - an Arrow IPC leaf,
    /// Parquet, a folder of parts, an Iceberg table, an object store - under
    /// the `**properties` a `with (...)` clause would state, its columns
    /// named by the registry's own names or any spelling of an identifier
    /// type; a store holding nothing yet is an empty first run, laid out by
    /// the first `commit`. Clean after the load. Unseeded: the store's rows
    /// and nothing else - `seeded_from_url` lays them over the seed.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS, **properties))]
    fn from_url(
        py: Python<'_>,
        location: &Bound<'_, PyAny>,
        max_instruments: usize,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        bound(py, location, properties, |holder| {
            IsinRegistry::new()
                .with_max_instruments(max_instruments)
                .try_with_holder(holder)
        })
    }

    /// `from_url` laid over the seed (`seeded`): the store `location` names,
    /// read the same way, its rows folded over the seed's by the update
    /// rule - a value the store states wins, a fact only the seed states
    /// stands beside it, a seed row it has no row of stands - and a store
    /// holding nothing yet the seed bound to it. Clean after the load, so
    /// the first `commit` after something moved writes the seed's rows with
    /// the store's. `max_instruments` bounds what is learned and merged
    /// after the load, as `from_arrow_reader`'s does.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=IsinRegistry::DEFAULT_MAX_INSTRUMENTS, **properties))]
    fn seeded_from_url(
        py: Python<'_>,
        location: &Bound<'_, PyAny>,
        max_instruments: usize,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        bound(py, location, properties, |holder| {
            IsinRegistry::seeded_from_holder(holder)
                .map(|registry| registry.with_max_instruments(max_instruments))
        })
    }

    /// The registry the process environment names, loaded on the first
    /// call and shared with every later one and with `FixCodec.from_env`:
    /// an installed registry, else the store `YGGDRYL_ISIN_REGISTRY_URI`
    /// names - a URL of any scheme, a path, `~` the home - else
    /// `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first
    /// `commit` lays out; with no home, the seed bound to nothing. A store
    /// is laid over the seed (`seeded`): its rows fold over the seed's by
    /// the update rule, so a value the store states wins and a seed row it
    /// has no row of stands; clean after the load. A failed load raises and
    /// is retried by the next call.
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
    /// named as `from_url` reads them, `firstunix` also as `firstseen` or
    /// `firstseenunix`, `lastunix` as `lastseen` or `lastseenunix`, `origccy`
    /// as `origcurrency`, `originalcurrency` or `issuecurrency`, and several rows of one ISIN on several markets its
    /// several listings; bound to no store, and clean.
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

    /// The listing rows as a `pyarrow.RecordBatchReader` under the
    /// registry's row field, in ISIN then MIC order: a snapshot taken under
    /// the lock, streamed after it is released, which a learn while it
    /// streams does not move. Write it with an `IOBase`'s
    /// `write_arrow_reader` - an overwrite saves a snapshot, a merge by
    /// `isin` and `miccode` upserts - or `commit` the registry.
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

    /// The first listing row of `isin` in MIC order - the unlisted row where
    /// that is all it holds - as a `dict` of its columns, or `None`; its
    /// instrument facts are every listing's, and `listings` answers them
    /// all.
    fn get(&self, py: Python<'_>, isin: &str) -> PyResult<Option<Py<PyAny>>> {
        let entry = self.with(py, |registry| registry.get(isin).cloned());
        Self::entry_as_py(py, entry)
    }

    /// Every listing row of `isin` in MIC order, each a `dict` of its
    /// columns; empty where the ISIN is unknown.
    fn listings(&self, py: Python<'_>, isin: &str) -> PyResult<Vec<Py<PyAny>>> {
        let rows = self.with(py, |registry| registry.listings(isin).to_vec());
        rows.into_iter()
            .map(|row| as_py(py, &row.into_scalar()))
            .collect()
    }

    /// The listing row of `isin` on `market` - a MIC, checked by the `mic`
    /// datatype - as a `dict` of its columns, or `None`.
    fn get_listing(
        &self,
        py: Python<'_>,
        isin: &str,
        market: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = mic_of(market)?;
        let entry = self.with(py, |registry| registry.get_listing(isin, &market).cloned());
        Self::entry_as_py(py, entry)
    }

    /// The listing row the ticker `ticker` names on `market`, as a `dict`
    /// of its columns, or `None`: the one row listing the ticker on
    /// `market` - a MIC, checked by the `mic` datatype - else, none listing
    /// it there, the one listing it on no market; where `market` is
    /// unstated (`None` or `XXXX`), the one row listing it on any. Two rows
    /// answering is ambiguous, and answers none.
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

    /// The listing row the code `value` of type `kind` - one of
    /// `LOOKUP_CODES`, by any spelling of its type - names, as a `dict` of
    /// its columns, or `None`: the one instrument holding the code, then its
    /// row on `market` - a MIC, checked by the `mic` datatype - else the one
    /// row holding the code, else its single row, else its first. Two
    /// instruments holding it is ambiguous and answers `None`, as does a
    /// type that is no lookup code or a value its type refuses.
    #[pyo3(signature = (kind, value, market=None))]
    fn get_by_code(
        &self,
        py: Python<'_>,
        kind: &str,
        value: &str,
        market: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let kind: IdType = kind.parse().map_err(value_error)?;
        let market = market.map(mic_of).transpose()?;
        let entry = self.with(py, |registry| {
            registry.get_by_code(&kind, value, market.as_ref()).cloned()
        });
        Self::entry_as_py(py, entry)
    }

    /// The listing row `element` names, and how - a `Resolution`: a real
    /// ISIN it states decides alone, and one the registry lacks ends the
    /// cascade (`UnknownIsin`); with none, each code of `LOOKUP_CODES` it
    /// states, in that order, then its ticker on its market, the first
    /// naming one instrument matching and the first naming two ending the
    /// cascade (`Ambiguous`); only where all of those found nothing, the
    /// instrument listed in its stated currency whose short name is the
    /// most similar to the one it states, at least `economic_threshold`, an
    /// instrument of another stated origin currency or CFI category dropped
    /// and named. `element` is any market leaf, a `MarketData` or a
    /// `FixMsg`; nothing is filled - `fill` takes an economic match only
    /// where `is_economic_match` says so.
    fn resolve(&self, py: Python<'_>, element: &Bound<'_, PyAny>) -> PyResult<PyResolution> {
        let element = market_data_of(element)?;
        let inner = self.with(py, move |registry| {
            Resolved::from_core(registry.resolve(&element))
        });
        Ok(PyResolution { inner })
    }

    /// The codes a lookup reads, in cascade order, each its type's word.
    #[classattr]
    #[allow(non_snake_case)]
    fn LOOKUP_CODES(py: Python<'_>) -> PyResult<Py<PyTuple>> {
        PyTuple::new(py, IsinRegistry::LOOKUP_CODES.iter().map(IdType::as_str)).map(Bound::unbind)
    }

    /// How similar two short names must be by default for an economic
    /// match.
    #[classattr]
    const DEFAULT_ECONOMIC_THRESHOLD: f64 = IsinRegistry::DEFAULT_ECONOMIC_THRESHOLD;

    /// How similar two short names must be, above `0` and at most `1`, for
    /// an economic match: `DEFAULT_ECONOMIC_THRESHOLD` unless set.
    #[getter]
    fn economic_threshold(&self, py: Python<'_>) -> f64 {
        self.with(py, |registry| registry.economic_threshold())
    }

    /// Sets `economic_threshold`; NaN and a value outside `(0, 1]` raise
    /// `ValueError` naming it, the threshold unmoved.
    fn set_economic_threshold(&self, py: Python<'_>, threshold: f64) -> PyResult<()> {
        self.with(py, |registry| registry.set_economic_threshold(threshold))
            .map_err(value_error)
    }

    /// Whether `fill` takes an economic match where nothing exact names the
    /// element: `False` unless set, since a derived ISIN becomes the key an
    /// element's book and chain live under. A parse never takes one.
    #[getter]
    fn is_economic_match(&self, py: Python<'_>) -> bool {
        self.with(py, |registry| registry.is_economic_match())
    }

    /// Sets `is_economic_match`.
    fn set_economic_match(&self, py: Python<'_>, enabled: bool) {
        self.with(py, |registry| registry.set_economic_match(enabled));
    }

    /// Folds one row - a mapping of column names to cells, `isin` required
    /// - into the listings of its ISIN by the update rule: a stated valid
    /// value fills a column a row lacks and replaces one it holds that
    /// differs, whatever the time, a code that is no real value of its
    /// type dropped; a compatible CFI code refines the held one and a
    /// contradicting one replaces it. The instrument facts fold into every
    /// listing row of the ISIN; the listing facts - the ticker, the
    /// currency, the listing codes - into the row of the market the entry
    /// names, created where the ISIN has none there; with no market, into
    /// the ISIN's single row, or into none, one warning per column, where
    /// it has several. `updunix` moves where a fact moved, `lastunix` to
    /// the later of the two whatever moved. A row then carries the defaults
    /// its facts imply where it states none: the CUSIP, SEDOL, WKN or Valor
    /// its ISIN embeds, and the currency of its market's country. Whether
    /// anything moved.
    fn merge(&self, py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entry = IsinEntry::from_scalar(&struct_from_entries(entry)?).map_err(value_error)?;
        self.with(py, |registry| registry.merge(entry))
            .map_err(value_error)
    }

    /// Removes every listing row of `isin`, answering them in MIC order,
    /// each a `dict`; empty where the ISIN is unknown.
    fn remove(&self, py: Python<'_>, isin: &str) -> PyResult<Vec<Py<PyAny>>> {
        let removed = self.with(py, |registry| registry.remove(isin));
        removed
            .into_iter()
            .map(|row| as_py(py, &row.into_scalar()))
            .collect()
    }

    /// Removes the listing row of `isin` on `market` - a MIC, checked by
    /// the `mic` datatype - answering it as a `dict`, or `None`; the
    /// instrument goes with its last listing.
    fn remove_listing(
        &self,
        py: Python<'_>,
        isin: &str,
        market: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = mic_of(market)?;
        let removed = self.with(py, |registry| registry.remove_listing(isin, &market));
        Self::entry_as_py(py, removed)
    }

    /// Removes every row.
    fn clear(&self, py: Python<'_>) {
        self.with(py, IsinRegistry::clear);
    }

    /// How many listing rows it holds - one per ISIN and market - what a
    /// commit writes; `len` is how many instruments.
    #[getter]
    fn rows(&self, py: Python<'_>) -> usize {
        self.with(py, |registry| registry.rows())
    }

    /// The most instruments it holds.
    #[getter]
    fn max_instruments(&self, py: Python<'_>) -> usize {
        self.with(py, |registry| registry.max_instruments())
    }

    /// Learns what a message states about its instrument - keyed by its
    /// stated real ISIN, its market naming the listing row its listing
    /// facts land on, dated at its `currunix`: its CFI code, its ticker, its
    /// currency, the pair it states and its real equivalents, and its
    /// `lastunix` on every learn, so a message teaching nothing else still
    /// records when the instrument was last met. Whether anything moved.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn learn(&self, py: Python<'_>, message: PyRef<'_, PyFixMsg>) -> bool {
        let message = message.as_inner();
        self.with(py, |registry| registry.learn(message))
    }

    /// Fills what a message leaves unsaid about its instrument from the
    /// listing row `resolve` names - its ISIN, else a lookup code, else its
    /// ticker on its market, else, where `is_economic_match`, its short
    /// name - each equivalent and the pair as a `derived` identifier,
    /// the ticker on its own market,
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

/// What [`IsinRegistry::resolve`] answered, owned: the row a match names is
/// a copy taken under the lock, boxed so a miss holds no row's room.
#[derive(Clone, Debug, PartialEq)]
enum Resolved {
    Matched {
        entry: Box<IsinEntry>,
        tier: MatchTier,
        derived: bool,
        listing: bool,
    },
    Unmatched(Unmatched),
}

impl Resolved {
    fn from_core(resolution: Resolution<'_>) -> Self {
        match resolution {
            Resolution::Matched {
                entry,
                tier,
                derived,
                listing,
            } => Self::Matched {
                entry: Box::new(entry.clone()),
                tier,
                derived,
                listing,
            },
            Resolution::Unmatched(why) => Self::Unmatched(why),
        }
    }

    /// The tier of a match, or of an ambiguity.
    const fn tier(&self) -> Option<&MatchTier> {
        match self {
            Self::Matched { tier, .. } | Self::Unmatched(Unmatched::Ambiguous { tier, .. }) => {
                Some(tier)
            }
            Self::Unmatched(_) => None,
        }
    }

    /// Why none matched; `None` for a match.
    const fn unmatched(&self) -> Option<&Unmatched> {
        match self {
            Self::Matched { .. } => None,
            Self::Unmatched(why) => Some(why),
        }
    }
}

/// The word a tier is spelled by at the boundary.
const fn tier_name(tier: &MatchTier) -> &'static str {
    match tier {
        MatchTier::Isin => "isin",
        MatchTier::Code(_) => "code",
        MatchTier::Symbology => "symbology",
        MatchTier::Economic { .. } => "economic",
    }
}

/// The name of the `Unmatched` variant, as the core spells it.
const fn unmatched_name(why: &Unmatched) -> &'static str {
    match why {
        Unmatched::NoKey => "NoKey",
        Unmatched::UnknownIsin { .. } => "UnknownIsin",
        Unmatched::NoCandidate => "NoCandidate",
        Unmatched::Ambiguous { .. } => "Ambiguous",
        Unmatched::CfiConflict { .. } => "CfiConflict",
        Unmatched::CurrencyConflict { .. } => "CurrencyConflict",
        Unmatched::BelowThreshold { .. } => "BelowThreshold",
    }
}

/// A tier as `repr` writes it, its field beside it.
fn tier_repr(tier: &MatchTier) -> String {
    match tier {
        MatchTier::Code(kind) => format!("tier='code', kind='{}'", kind.as_str()),
        MatchTier::Economic { similarity } => format!("tier='economic', similarity={similarity}"),
        other => format!("tier='{}'", tier_name(other)),
    }
}

/// What `IsinRegistry.resolve` answered for one element: the row it names
/// and how, or why none - every field of the core answer an attribute, so a
/// job acts on a `CfiConflict` without parsing text. `matched` and its
/// truth say which; a match states `entry`, `tier` (`"isin"`, `"code"`,
/// `"symbology"` or `"economic"`), `kind` (the type of a `"code"` tier),
/// `similarity` (an `"economic"` one's), `derived` and `listing`; a miss
/// states `unmatched` - `NoKey`, `UnknownIsin`, `NoCandidate`, `Ambiguous`,
/// `CfiConflict`, `CurrencyConflict` or `BelowThreshold` - and the fields
/// its variant carries: `isins` and the tier of an `Ambiguous`, `stated`,
/// `held`, `best` and `isin`. Every other attribute is `None`. Immutable,
/// equal by value, never hashed: the row is a copy taken when it resolved.
#[pyclass(name = "Resolution", module = "yggdryl._native", frozen)]
pub(crate) struct PyResolution {
    inner: Resolved,
}

#[pymethods]
impl PyResolution {
    // Equal by value over a float similarity, so never hashed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Whether a row was matched.
    #[getter]
    const fn matched(&self) -> bool {
        matches!(self.inner, Resolved::Matched { .. })
    }

    /// The listing row matched, as a `dict` of its columns; `None` where
    /// none was.
    #[getter]
    fn entry(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match &self.inner {
            Resolved::Matched { entry, .. } => as_py(py, &entry.into_scalar()).map(Some),
            Resolved::Unmatched(_) => Ok(None),
        }
    }

    /// The tier that matched - or, for `Ambiguous`, that found the several
    /// instruments: `"isin"`, `"code"`, `"symbology"` or `"economic"`;
    /// `None` for any other miss.
    #[getter]
    fn tier(&self) -> Option<&'static str> {
        self.inner.tier().map(tier_name)
    }

    /// The type of the code a `"code"` tier read, as its word - the
    /// identifier's `kind`; `None` otherwise.
    #[getter]
    fn kind(&self) -> Option<&str> {
        match self.inner.tier() {
            Some(MatchTier::Code(kind)) => Some(kind.as_str()),
            _ => None,
        }
    }

    /// How similar the two short names of an `"economic"` tier are, from
    /// the threshold to `1`; `None` otherwise.
    #[getter]
    fn similarity(&self) -> Option<f64> {
        match self.inner.tier() {
            Some(MatchTier::Economic { similarity }) => Some(*similarity),
            _ => None,
        }
    }

    /// Whether the ISIN was derived - the element stated none; `None` for a
    /// miss.
    #[getter]
    const fn derived(&self) -> Option<bool> {
        match self.inner {
            Resolved::Matched { derived, .. } => Some(derived),
            Resolved::Unmatched(_) => None,
        }
    }

    /// Whether the row's listing facts belong to the element: its market is
    /// the row's, or either is unstated; `None` for a miss.
    #[getter]
    const fn listing(&self) -> Option<bool> {
        match self.inner {
            Resolved::Matched { listing, .. } => Some(listing),
            Resolved::Unmatched(_) => None,
        }
    }

    /// Why none matched, as the name of the core's `Unmatched` variant;
    /// `None` for a match.
    #[getter]
    fn unmatched(&self) -> Option<&'static str> {
        self.inner.unmatched().map(unmatched_name)
    }

    /// The ISINs an `Ambiguous` miss found, in ISIN order; `None` otherwise.
    #[getter]
    fn isins(&self) -> Option<Vec<String>> {
        match self.inner.unmatched() {
            Some(Unmatched::Ambiguous { isins, .. }) => {
                Some(isins.iter().map(|isin| isin.as_str().to_owned()).collect())
            }
            _ => None,
        }
    }

    /// What the element states: the ISIN of an `UnknownIsin`, the CFI
    /// category letter of a `CfiConflict`, the origin currency of a
    /// `CurrencyConflict`; `None` otherwise.
    #[getter]
    fn stated(&self) -> Option<String> {
        match self.inner.unmatched() {
            Some(Unmatched::UnknownIsin { stated }) => Some(stated.as_str().to_owned()),
            Some(Unmatched::CfiConflict { stated, .. }) => Some(stated.to_string()),
            Some(Unmatched::CurrencyConflict { stated, .. }) => Some(stated.as_str().to_owned()),
            _ => None,
        }
    }

    /// What the instrument holds instead: the CFI category letter of a
    /// `CfiConflict`, the origin currency of a `CurrencyConflict`; `None`
    /// otherwise.
    #[getter]
    fn held(&self) -> Option<String> {
        match self.inner.unmatched() {
            Some(Unmatched::CfiConflict { held, .. }) => Some(held.to_string()),
            Some(Unmatched::CurrencyConflict { held, .. }) => Some(held.as_str().to_owned()),
            _ => None,
        }
    }

    /// How similar the most similar instrument of a `BelowThreshold` miss
    /// is; `None` otherwise.
    #[getter]
    fn best(&self) -> Option<f64> {
        match self.inner.unmatched() {
            Some(Unmatched::BelowThreshold { best, .. }) => Some(*best),
            _ => None,
        }
    }

    /// The instrument a `CfiConflict`, a `CurrencyConflict` or a
    /// `BelowThreshold` miss names; `None` otherwise.
    #[getter]
    fn isin(&self) -> Option<&str> {
        match self.inner.unmatched() {
            Some(
                Unmatched::CfiConflict { isin, .. }
                | Unmatched::CurrencyConflict { isin, .. }
                | Unmatched::BelowThreshold { isin, .. },
            ) => Some(isin.as_str()),
            _ => None,
        }
    }

    const fn __bool__(&self) -> bool {
        self.matched()
    }

    /// Equal where the two answers are: the same row, tier and flags, or the
    /// same miss with the same fields.
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| other.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        let flag = |held: bool| if held { "True" } else { "False" };
        match &self.inner {
            Resolved::Matched {
                entry,
                tier,
                derived,
                listing,
            } => format!(
                "Resolution(matched=True, isin='{}', {}, derived={}, listing={})",
                entry.isin().as_str(),
                tier_repr(tier),
                flag(*derived),
                flag(*listing),
            ),
            Resolved::Unmatched(why) => {
                let fields = match why {
                    Unmatched::NoKey | Unmatched::NoCandidate => String::new(),
                    Unmatched::UnknownIsin { stated } => {
                        format!(", stated='{}'", stated.as_str())
                    }
                    Unmatched::Ambiguous { tier, isins } => format!(
                        ", {}, isins={:?}",
                        tier_repr(tier),
                        isins.iter().map(Isin::as_str).collect::<Vec<_>>()
                    ),
                    Unmatched::CfiConflict { stated, held, isin } => format!(
                        ", stated='{stated}', held='{held}', isin='{}'",
                        isin.as_str()
                    ),
                    Unmatched::CurrencyConflict { stated, held, isin } => format!(
                        ", stated='{}', held='{}', isin='{}'",
                        stated.as_str(),
                        held.as_str(),
                        isin.as_str()
                    ),
                    Unmatched::BelowThreshold { best, isin } => {
                        format!(", best={best}, isin='{}'", isin.as_str())
                    }
                };
                format!(
                    "Resolution(matched=False, unmatched='{}'{fields})",
                    unmatched_name(why)
                )
            }
        }
    }
}
