//! The instruments: one element per instrument, keyed by its cross code -
//! a real ISIN, or the `class:body` its class and characteristics spell -
//! holding every identifier and listing it is known by, learned from and
//! filled into market data, bound to a store it is loaded from and
//! committed back to. Shared behind one lock, so a codec handed the
//! collection and the caller holding it see one table.
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
use yggdryl::{DataType, Mic, Scalar, Uuid};
use yggdryl_market::{IdType, Instrument, Instruments, Listing, MatchTier, Resolution, Unmatched};

use crate::field::PyField;

use crate::fix::PyFixMsg;
use crate::graph::market_data::market_data_of;
use crate::iobase::{PyIOBase, located_holder};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::ioresult::PyIOResult;
use crate::scalar::{RowPlan, as_py_with_field, from_py};
use crate::uri::core_url_from_value;
use crate::value_error;

/// The instruments a process knows, one row per instrument keyed by its
/// `crosscode` - a security by its bare real ISIN, an FX pair or a
/// derivative by the `class:body` its CFI class and characteristics spell
/// (`IF:EUR/USD`, `OC:US0378331005:2026-12-18:200`), its identity the
/// code's digest. A row holds every non-listing identifier in
/// `securityids` - the minted `QY` number of an instrument no agency
/// numbers under `yggdryl:isin` - its listings nested, one per market, the
/// codes it had before a re-key in `aliascodes`, and complementary facts in
/// `metadata`. A lifecycle learns into it and fills from it, and a parse
/// fills from it; `resolve` answers which instrument an element names and
/// how - its ISIN, its cross code, a code of `LOOKUP_CODES`, its ticker, or
/// its short name in its currency. Bound to the store it was loaded from,
/// committed back only where it moved.
/// Mutable and shared: equal only to itself, never hashed or pickled; its
/// rows cross out as `dict`s and as an Arrow stream.
#[pyclass(name = "Instruments", module = "yggdryl._native", frozen)]
pub(crate) struct PyInstruments {
    pub(crate) inner: Arc<Mutex<Instruments>>,
}

/// The table, a poisoned lock recovered: every verb leaves it whole.
fn lock(inner: &Mutex<Instruments>) -> MutexGuard<'_, Instruments> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PyInstruments {
    /// Wraps the table a codec shares.
    pub(crate) fn from_shared(inner: &Arc<Mutex<Instruments>>) -> Self {
        Self {
            inner: Arc::clone(inner),
        }
    }

    /// Wraps a collection the core built.
    fn from_core(inner: Instruments) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Runs `verb` on the table, detached from the GIL and under the lock,
    /// answering what it answered as an owned value.
    fn with<T: Send>(&self, py: Python<'_>, verb: impl FnOnce(&mut Instruments) -> T + Send) -> T {
        let inner = &self.inner;
        py.detach(|| verb(&mut lock(inner)))
    }

    /// One instrument as the `dict` of its columns, a fact it does not
    /// state `None`.
    fn entry_as_py(py: Python<'_>, entry: Option<Instrument>) -> PyResult<Option<Py<PyAny>>> {
        entry.map(|held| instrument_as_py(py, &held)).transpose()
    }

    /// One listing as the `dict` of its four columns, or `None`.
    fn listing_as_py(py: Python<'_>, listing: Option<Listing>) -> PyResult<Option<Py<PyAny>>> {
        listing.map(|held| listing_as_py(py, &held)).transpose()
    }
}

/// One listing as the `dict` of its four columns - `miccode`, `ticker`,
/// `currency`, `codes` - a fact it does not state `None`: the core's named
/// row rendered under its field, as an instrument's is.
fn listing_as_py(py: Python<'_>, listing: &Listing) -> PyResult<Py<PyAny>> {
    as_py_with_field(py, &listing.into_scalar(), &Listing::field())
}

/// One instrument as the `dict` of its columns, each nested record - a
/// listing, a leg, the characteristics - a `dict` its field's children key:
/// the core's named row rendered under its field.
fn instrument_as_py(py: Python<'_>, instrument: &Instrument) -> PyResult<Py<PyAny>> {
    as_py_with_field(py, &instrument.into_scalar(), &Instrument::field())
}

/// One instrument from a mapping of the columns of `Instrument::field()` -
/// a `dict`, or a record class's instance - read by name as every record
/// door reads a row, a nested listing, leg or characteristics `dict` by its
/// record's children's names, a column it omits unstated: the core derives
/// an element column or `placeholder` it omits, since a statement is
/// derived. A `dict` key naming no column is refused by name rather than
/// dropped.
fn instrument_of(entry: &Bound<'_, PyAny>) -> PyResult<Instrument> {
    let field = Instrument::field();
    if let Ok(mapping) = entry.cast::<PyDict>() {
        for key in mapping.keys() {
            let key: String = key.extract()?;
            if !field.fields().iter().any(|child| child.name() == key) {
                return Err(value_error(format!(
                    "expected a column of the instrument row, got {key:?}"
                )));
            }
        }
    }
    let row = RowPlan::new(entry.py(), &field).row(entry)?;
    Instrument::from_scalar(&row).map_err(value_error)
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

/// The identity one argument names - a `uuid.UUID`, its text or its
/// sixteen bytes - checked through the `uuid` datatype's own value
/// contract.
fn uuid_of(value: &Bound<'_, PyAny>) -> PyResult<Uuid> {
    match DataType::Uuid
        .scalar(from_py(value)?)
        .map_err(value_error)?
    {
        Scalar::Uuid(uuid) => Ok(uuid),
        other => Err(PyTypeError::new_err(format!(
            "expected a UUID, got {}",
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

/// Binds a collection to the store `location` names - an `IOBase` already
/// built, or anything a location is read from, under `properties` - through
/// `load`, which runs detached from the GIL with the store's holder.
fn bound(
    py: Python<'_>,
    location: &Bound<'_, PyAny>,
    properties: Option<&Bound<'_, PyDict>>,
    load: impl FnOnce(Holder) -> yggdryl::Result<Instruments> + Send,
) -> PyResult<PyInstruments> {
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
            .map(PyInstruments::from_core)
            .map_err(value_error);
    }
    let url = core_url_from_value(location)?;
    py.detach(|| load(Holder::from_url(&url, properties)?))
        .map(PyInstruments::from_core)
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
impl PyInstruments {
    // Shared and mutable: equal only to itself, so never hashed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// An empty collection holding at most `max_instruments` instruments,
    /// bound to no store; learning skips a new instrument past the bound
    /// and loading refuses it.
    #[new]
    #[pyo3(signature = (max_instruments=Instruments::DEFAULT_MAX_INSTRUMENTS))]
    fn new(max_instruments: usize) -> Self {
        Self::from_core(Instruments::new().with_max_instruments(max_instruments))
    }

    /// The instrument row: the required struct `instrument` - the six
    /// element columns (`uuid`, `crossuuid`, `crosscode`, `hashcode`,
    /// `crosshashcode`, `srcuuids`), then `aliascodes`, `placeholder`,
    /// `isin`, `cficode`, `forexcode`, `fisn`, `countrycode`, `currency`,
    /// `origccy`, `securityids`, `underlying` and `legs` (cross codes),
    /// `eusipacode`, `characteristics`, `listings` (one struct per market:
    /// `miccode`, `ticker`, `currency`, `codes`), `metadata` and the stamps
    /// `updunix`, `firstunix`, `lastunix`: twenty-five columns - what a
    /// table holding the instruments is created from. Its root declares
    /// `PARTITION:by` `["truncate(crosscode, 2)"]` and `SORT:by`
    /// `["crosscode"]`, the order the snapshot streams in.
    #[staticmethod]
    fn field() -> PyField {
        PyField::from_inner(Instrument::field())
    }

    /// A collection holding the seed - the common instruments
    /// `config/instruments/instruments.json` states, embedded at build
    /// time: each a stock, a fund or an index by its ISIN, its listings, its
    /// country, its detailed CFI code and its short name where one is known
    /// - clean and bound to no store. A seed object is an ordinary
    /// statement, so the facts it implies - the national number its ISIN
    /// embeds, the currency of its market's country - are derived as for
    /// any other; `Instruments()` holds none of it.
    #[staticmethod]
    fn seeded(py: Python<'_>) -> Self {
        py.detach(|| Self::from_core(Instruments::seeded()))
    }

    /// The number this crate mints for an instrument no agency numbers - a
    /// `QY` ISIN, nine base-36 digits of the code's digest closed by its
    /// check digit - computed from the cross code alone: what an
    /// instrument keyed `crosscode` holds under `yggdryl:isin`, for a reader
    /// joining on `isin`.
    #[staticmethod]
    fn mint(crosscode: &str) -> String {
        Instrument::minted_number(crosscode).as_str().to_owned()
    }

    /// A collection bound to the store `location` names and loaded from it:
    /// a URL of any scheme this build holds, or a path - an Arrow IPC leaf,
    /// Parquet, a folder of parts, an Iceberg table, an object store - under
    /// the `**properties` a `with (...)` clause would state, laid out as
    /// `field()`; a store holding nothing yet is an empty first run, laid
    /// out by the first `commit`, and a table lacking `crosscode` - one
    /// written before the instrument row - is refused by name, to drop and
    /// lay out afresh.
    /// Clean after the load. Unseeded: the store's rows and nothing else -
    /// `seeded_from_url` lays them over the seed.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=Instruments::DEFAULT_MAX_INSTRUMENTS, **properties))]
    fn from_url(
        py: Python<'_>,
        location: &Bound<'_, PyAny>,
        max_instruments: usize,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        bound(py, location, properties, |holder| {
            Instruments::new()
                .with_max_instruments(max_instruments)
                .try_with_holder(holder)
        })
    }

    /// `from_url` laid over the seed (`seeded`): the store `location` names,
    /// read the same way, its rows folded over the seed's by the update
    /// rule - a value the store states wins, a fact only the seed states
    /// stands beside it, a seed instrument it has no row of stands - and a
    /// store holding nothing yet the seed bound to it. Clean after the
    /// load, so the first `commit` after something moved writes the seed's
    /// rows with the store's. `max_instruments` bounds what is learned and
    /// merged after the load, as `from_arrow_reader`'s does.
    #[staticmethod]
    #[pyo3(signature = (location, max_instruments=Instruments::DEFAULT_MAX_INSTRUMENTS, **properties))]
    fn seeded_from_url(
        py: Python<'_>,
        location: &Bound<'_, PyAny>,
        max_instruments: usize,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        bound(py, location, properties, |holder| {
            Instruments::seeded_from_holder(holder)
                .map(|instruments| instruments.with_max_instruments(max_instruments))
        })
    }

    /// The instruments the process environment names, loaded on the first
    /// call and shared with every later one and with `FixCodec.from_env`:
    /// an installed collection, else the store `YGGDRYL_INSTRUMENTS_URI`
    /// names - a URL of any scheme, a path, `~` the home - else
    /// `~/.config/yggdryl/instruments/`, a folder of Arrow IPC parts the
    /// first `commit` lays out; with no home, the seed bound to nothing. A
    /// store is laid over the seed (`seeded`): its rows fold over the seed's
    /// by the update rule, so a value the store states wins and a seed
    /// instrument it has no row of stands; clean after the load. A failed
    /// load raises and is retried by the next call.
    #[staticmethod]
    fn from_env(py: Python<'_>) -> PyResult<Self> {
        py.detach(|| Instruments::from_env().map(Self::from_shared))
            .map_err(value_error)
    }

    /// Installs `instruments` as the collection every later `from_env`
    /// answers - this very table, shared - before anything resolves one;
    /// raises once the default has resolved or been installed.
    #[staticmethod]
    fn install_env(instruments: &Bound<'_, Self>) -> PyResult<()> {
        Instruments::install_env_shared(Arc::clone(&instruments.get().inner)).map_err(value_error)
    }

    /// A collection read from any Arrow stream - a `pyarrow` reader, table
    /// or batch, or anything exporting `__arrow_c_stream__` - laid out as
    /// `field()`, a nullable column it lacks null and a column no field
    /// reads landing in each row's `metadata` under its name; bound to no
    /// store, and clean.
    #[staticmethod]
    #[pyo3(signature = (reader, max_instruments=Instruments::DEFAULT_MAX_INSTRUMENTS))]
    fn from_arrow_reader(
        py: Python<'_>,
        reader: &Bound<'_, PyAny>,
        max_instruments: usize,
    ) -> PyResult<Self> {
        let reader = batch_reader_from_value(reader)?;
        let instruments = py
            .detach(|| {
                let instruments =
                    Instruments::from_arrow_reader(reader)?.with_max_instruments(max_instruments);
                Ok::<_, yggdryl::arrow::Error>(instruments)
            })
            .map_err(value_error)?;
        Ok(Self::from_core(instruments))
    }

    /// Folds the rows `location` holds in - an `IOBase`, or anything a
    /// location is read from - by the update rule, leaving the collection
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
        self.with(py, |instruments| {
            instruments.extend_from_arrow_reader(reader)
        })
        .map_err(value_error)
    }

    /// The instrument rows as a `pyarrow.RecordBatchReader` under
    /// `field()`, in cross code order: a snapshot taken under the lock,
    /// streamed after it is released, which a learn while it streams does
    /// not move. Write it with an `IOBase`'s `write_arrow_reader` - an
    /// overwrite saves a snapshot, a merge by `crosscode` upserts - or
    /// `commit` the collection.
    #[allow(clippy::wrong_self_convention)]
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self
            .with(py, |instruments| instruments.into_arrow_reader())
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Writes the table to the store it is bound to, only where its content
    /// differs from what the store holds (`is_dirty`): one overwrite of the
    /// whole snapshot, a leaf rewritten, a folder's parts replaced by one, an
    /// Iceberg table replaced in one atomic snapshot across every
    /// partition, an emptied collection truncating a leaf, emptying a table
    /// or removing a folder's parts. The `IOResult` of the write, empty for
    /// a clean collection, which touches the store with no call. Raises on
    /// a collection bound to no store.
    fn commit(&self, py: Python<'_>) -> PyResult<PyIOResult> {
        self.with(py, Instruments::commit)
            .map(PyIOResult::from_core)
            .map_err(value_error)
    }

    /// Whether the table's content differs from what the store holds - as
    /// it was loaded or last committed: an instrument added or removed, or
    /// one whose content code or `firstunix`/`lastunix` window moved. A
    /// fact that moved and moved back since the load is no change, so a run
    /// replayed over the same input leaves a clean collection.
    #[getter]
    fn is_dirty(&self, py: Python<'_>) -> bool {
        self.with(py, |instruments| instruments.is_dirty())
    }

    /// The instrument `key` names - its cross code, a code it had before a
    /// re-key, or an ISIN it holds, real or minted - as a `dict` of its
    /// columns, or `None`.
    fn get(&self, py: Python<'_>, key: &str) -> PyResult<Option<Py<PyAny>>> {
        let entry = self.with(py, |instruments| instruments.get(key).cloned());
        Self::entry_as_py(py, entry)
    }

    /// The instrument whose identity - or a former identity - is `uuid` (a
    /// `uuid.UUID`, its text or its sixteen bytes), as a `dict`, or `None`.
    fn get_by_uuid(&self, py: Python<'_>, uuid: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyAny>>> {
        let uuid = uuid_of(uuid)?;
        let entry = self.with(py, |instruments| instruments.get_by_uuid(uuid).cloned());
        Self::entry_as_py(py, entry)
    }

    /// The listings of the instrument `key` names, in MIC order - the
    /// unlisted one first - each a `dict` of `miccode`, `ticker`,
    /// `currency` and `codes`; empty where the instrument is unknown.
    fn listings(&self, py: Python<'_>, key: &str) -> PyResult<Vec<Py<PyAny>>> {
        let listings = self.with(py, |instruments| instruments.listings(key).to_vec());
        listings
            .iter()
            .map(|listing| listing_as_py(py, listing))
            .collect()
    }

    /// The listing of the instrument `key` names on `market` - a MIC,
    /// checked by the `mic` datatype - as a `dict`, or `None`.
    fn get_listing(
        &self,
        py: Python<'_>,
        key: &str,
        market: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = mic_of(market)?;
        let listing = self.with(py, |instruments| {
            instruments.get_listing(key, &market).cloned()
        });
        Self::listing_as_py(py, listing)
    }

    /// The instrument the ticker `ticker` names on `market`, as a `dict`,
    /// or `None`: the one instrument a listing of which lists the ticker on
    /// `market` - a MIC, checked by the `mic` datatype - else, none listing
    /// it there, the one listing it on no market; where `market` is
    /// unstated (`None` or `XXXX`), the one listing it on any. Two
    /// instruments answering is ambiguous, and answers none.
    #[pyo3(signature = (ticker, market=None))]
    fn get_by_ticker(
        &self,
        py: Python<'_>,
        ticker: &str,
        market: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = market.map(mic_of).transpose()?;
        let entry = self.with(py, |instruments| {
            instruments.get_by_ticker(ticker, market.as_ref()).cloned()
        });
        Self::entry_as_py(py, entry)
    }

    /// The instrument the code `value` of type `kind` - one of
    /// `LOOKUP_CODES`, by any spelling of its type - names, as a `dict`, or
    /// `None`. Two instruments holding it is ambiguous and answers `None`,
    /// as does a type that is no lookup code or a value its type refuses.
    fn get_by_code(&self, py: Python<'_>, kind: &str, value: &str) -> PyResult<Option<Py<PyAny>>> {
        let kind: IdType = kind.parse().map_err(value_error)?;
        let entry = self.with(py, |instruments| {
            instruments.get_by_code(&kind, value).cloned()
        });
        Self::entry_as_py(py, entry)
    }

    /// The instrument `element` names, and how - a `Resolution`: a real
    /// ISIN it states decides alone, and one the collection lacks ends the
    /// cascade (`UnknownIsin`); else the cross code its own facts spell -
    /// an FX pair's - one the collection lacks ending it too
    /// (`UnknownCode`); else a minted number it holds; else each code of
    /// `LOOKUP_CODES` it states, in that order, then its ticker on its
    /// market, the first naming one instrument matching and the first
    /// naming two ending the cascade (`Ambiguous`); only where all of those
    /// found nothing, the instrument listed in its stated currency whose
    /// short name is the most similar to the one it states, at least
    /// `economic_threshold`, an instrument of another stated origin
    /// currency or CFI category dropped and named. `element` is any market
    /// leaf, a `MarketData` or a `FixMsg`; nothing is filled - `fill` takes
    /// an economic match only where `is_economic_match` says so.
    fn resolve(&self, py: Python<'_>, element: &Bound<'_, PyAny>) -> PyResult<PyResolution> {
        let element = market_data_of(element)?;
        let inner = self.with(py, move |instruments| {
            Resolved::from_core(instruments.resolve(&element))
        });
        Ok(PyResolution { inner })
    }

    /// The codes a lookup reads, in cascade order, each its type's word.
    #[classattr]
    #[allow(non_snake_case)]
    fn LOOKUP_CODES(py: Python<'_>) -> PyResult<Py<PyTuple>> {
        PyTuple::new(py, Instruments::LOOKUP_CODES.iter().map(IdType::as_str)).map(Bound::unbind)
    }

    /// How similar two short names must be by default for an economic
    /// match.
    #[classattr]
    const DEFAULT_ECONOMIC_THRESHOLD: f64 = Instruments::DEFAULT_ECONOMIC_THRESHOLD;

    /// How similar two short names must be, above `0` and at most `1`, for
    /// an economic match: `DEFAULT_ECONOMIC_THRESHOLD` unless set.
    #[getter]
    fn economic_threshold(&self, py: Python<'_>) -> f64 {
        self.with(py, |instruments| instruments.economic_threshold())
    }

    /// Sets `economic_threshold`; NaN and a value outside `(0, 1]` raise
    /// `ValueError` naming it, the threshold unmoved.
    fn set_economic_threshold(&self, py: Python<'_>, threshold: f64) -> PyResult<()> {
        self.with(py, |instruments| {
            instruments.set_economic_threshold(threshold)
        })
        .map_err(value_error)
    }

    /// Whether `fill` takes an economic match where nothing exact names the
    /// element: `False` unless set, since a derived ISIN becomes the key an
    /// element's book and chain live under. A parse never takes one.
    #[getter]
    fn is_economic_match(&self, py: Python<'_>) -> bool {
        self.with(py, |instruments| instruments.is_economic_match())
    }

    /// Sets `is_economic_match`.
    fn set_economic_match(&self, py: Python<'_>, enabled: bool) {
        self.with(py, |instruments| instruments.set_economic_match(enabled));
    }

    /// Folds one instrument row - a mapping of the columns of `field()`, a
    /// column it omits unstated, the facts its key is written from (a real
    /// ISIN, or a CFI class and its body's characteristics) required - into the
    /// instrument of its cross code by the update rule: a stated value
    /// fills a fact the instrument lacks and replaces one it holds that
    /// differs, whatever the time; a code that is no real value of its type
    /// is dropped; a CFI compatible with the held one refines it and a
    /// contradicting one replaces it; a real ISIN replaces a minted one; a
    /// `metadata` entry fills or replaces its key. The listing facts fold
    /// into the listing of each market the row lists, created where the
    /// instrument has none there. A placeholder keyed by its ISIN is
    /// re-keyed by a row spelling its body, the ISIN kept in `aliascodes`.
    /// Whether anything moved.
    fn merge(&self, py: Python<'_>, entry: &Bound<'_, PyAny>) -> PyResult<bool> {
        let entry = instrument_of(entry)?;
        self.with(py, |instruments| instruments.merge(entry))
            .map_err(value_error)
    }

    /// Removes the instrument `key` names - its cross code, an alias or an
    /// ISIN - answering it as a `dict`, or `None` where it is unknown.
    fn remove(&self, py: Python<'_>, key: &str) -> PyResult<Option<Py<PyAny>>> {
        let removed = self.with(py, |instruments| instruments.remove(key));
        Self::entry_as_py(py, removed)
    }

    /// Removes the listing of the instrument `key` names on `market` - a
    /// MIC, checked by the `mic` datatype - answering it as a `dict`, or
    /// `None`; the instrument stays.
    fn remove_listing(
        &self,
        py: Python<'_>,
        key: &str,
        market: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let market = mic_of(market)?;
        let removed = self.with(py, |instruments| instruments.remove_listing(key, &market));
        Self::listing_as_py(py, removed)
    }

    /// Removes every instrument.
    fn clear(&self, py: Python<'_>) {
        self.with(py, Instruments::clear);
    }

    /// How many rows a commit writes: one per instrument, its listings
    /// nested - what `len` counts.
    #[getter]
    fn rows(&self, py: Python<'_>) -> usize {
        self.with(py, |instruments| instruments.rows())
    }

    /// The most instruments it holds.
    #[getter]
    fn max_instruments(&self, py: Python<'_>) -> usize {
        self.with(py, |instruments| instruments.max_instruments())
    }

    /// Learns what a message states about its instrument as a market
    /// element - keyed by the code its facts spell: a stated real ISIN, or
    /// its `forex` pair beside an `I*` class, minting its number - its
    /// market naming the listing its listing facts land on, dated at its
    /// `transunix`: its CFI code, its ticker, its currency, its short name
    /// and its real equivalents, and its `lastunix` on every learn, so a
    /// message teaching nothing else still records when the instrument was
    /// last met. A derivative's characteristics are the FIX lifecycle's to
    /// learn, through a codec holding the collection. Whether anything
    /// moved.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn learn(&self, py: Python<'_>, message: PyRef<'_, PyFixMsg>) -> bool {
        let message = message.as_inner();
        self.with(py, |instruments| instruments.learn(message))
    }

    /// Fills what a message leaves unsaid about its instrument from the
    /// instrument `resolve` names - its ISIN, its cross code, else a lookup
    /// code, else its ticker on its market, else, where
    /// `is_economic_match`, its short name - each identifier as a `derived`
    /// one, the listing codes and the ticker of its own listing, its CFI
    /// code where the instrument's refines it, the currency on the same
    /// stated market under the listing's ticker, and the instrument's cross
    /// code as its `instcode` where it holds none - never its wire. Whether
    /// anything moved; a hashed message is frozen and refuses with
    /// `TypeError`.
    fn fill(&self, py: Python<'_>, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.with(py, |instruments| instruments.fill(held)))
    }

    /// `learn`, then `fill`. Whether anything moved in either.
    fn enrich(&self, py: Python<'_>, mut message: PyRefMut<'_, PyFixMsg>) -> PyResult<bool> {
        let held = message.as_inner_mut()?;
        Ok(self.with(py, |instruments| instruments.enrich(held)))
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.with(py, |instruments| instruments.len())
    }
    fn __bool__(&self, py: Python<'_>) -> bool {
        self.with(py, |instruments| !instruments.is_empty())
    }
    /// Whether `other` is this collection - the same shared table.
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|other| Arc::ptr_eq(&other.get().inner, &self.inner))
    }
    fn __repr__(&self, py: Python<'_>) -> String {
        let (len, max, dirty) = self.with(py, |instruments| {
            (
                instruments.len(),
                instruments.max_instruments(),
                instruments.is_dirty(),
            )
        });
        let dirty = if dirty { "True" } else { "False" };
        format!("Instruments(len={len}, max_instruments={max}, dirty={dirty})")
    }
}

/// What [`Instruments::resolve`] answered, owned: the instrument a match
/// names is a copy taken under the lock, boxed so a miss holds no row's
/// room.
#[derive(Clone, Debug, PartialEq)]
enum Resolved {
    Matched {
        entry: Box<Instrument>,
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
        MatchTier::CrossCode => "crosscode",
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
        Unmatched::UnknownCode { .. } => "UnknownCode",
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

/// What `Instruments.resolve` answered for one element: the instrument it
/// names and how, or why none - every field of the core answer an
/// attribute, so a job acts on a `CfiConflict` without parsing text.
/// `matched` and its truth say which; a match states `entry`, `tier`
/// (`"isin"`, `"crosscode"`, `"code"`, `"symbology"` or `"economic"`),
/// `kind` (the type of a `"code"` tier), `similarity` (an `"economic"`
/// one's), `derived` and `listing`; a miss states `unmatched` - `NoKey`,
/// `UnknownIsin`, `UnknownCode`, `NoCandidate`, `Ambiguous`, `CfiConflict`,
/// `CurrencyConflict` or `BelowThreshold` - and the fields its variant
/// carries: `codes` and the tier of an `Ambiguous`, `stated`, `held`,
/// `best` and `code`. Every other attribute is `None`. Immutable, equal by
/// value, never hashed: the instrument is a copy taken when it resolved.
#[pyclass(name = "Resolution", module = "yggdryl._native", frozen)]
pub(crate) struct PyResolution {
    inner: Resolved,
}

#[pymethods]
impl PyResolution {
    // Equal by value over a float similarity, so never hashed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Whether an instrument was matched.
    #[getter]
    const fn matched(&self) -> bool {
        matches!(self.inner, Resolved::Matched { .. })
    }

    /// The instrument matched, as a `dict` of its columns; `None` where
    /// none was.
    #[getter]
    fn entry(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match &self.inner {
            Resolved::Matched { entry, .. } => instrument_as_py(py, entry).map(Some),
            Resolved::Unmatched(_) => Ok(None),
        }
    }

    /// The tier that matched - or, for `Ambiguous`, that found the several
    /// instruments: `"isin"`, `"crosscode"`, `"code"`, `"symbology"` or
    /// `"economic"`; `None` for any other miss.
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

    /// Whether the match was derived - the element stated no real ISIN of
    /// it; `None` for a miss.
    #[getter]
    const fn derived(&self) -> Option<bool> {
        match self.inner {
            Resolved::Matched { derived, .. } => Some(derived),
            Resolved::Unmatched(_) => None,
        }
    }

    /// Whether the instrument's listing on the element's market is the
    /// element's: its market is listed, or it states none and the
    /// instrument has one listing; `None` for a miss.
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

    /// The cross codes an `Ambiguous` miss found, in code order; `None`
    /// otherwise.
    #[getter]
    fn codes(&self) -> Option<Vec<String>> {
        match self.inner.unmatched() {
            Some(Unmatched::Ambiguous { codes, .. }) => {
                Some(codes.iter().map(|code| code.as_str().to_owned()).collect())
            }
            _ => None,
        }
    }

    /// What the element states: the ISIN of an `UnknownIsin`, the cross
    /// code of an `UnknownCode`, the CFI category letter of a
    /// `CfiConflict`, the origin currency of a `CurrencyConflict`; `None`
    /// otherwise.
    #[getter]
    fn stated(&self) -> Option<String> {
        match self.inner.unmatched() {
            Some(Unmatched::UnknownIsin { stated }) => Some(stated.as_str().to_owned()),
            Some(Unmatched::UnknownCode { stated }) => Some(stated.as_str().to_owned()),
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

    /// The cross code of the instrument a `CfiConflict`, a
    /// `CurrencyConflict` or a `BelowThreshold` miss names; `None`
    /// otherwise.
    #[getter]
    fn code(&self) -> Option<&str> {
        match self.inner.unmatched() {
            Some(
                Unmatched::CfiConflict { code, .. }
                | Unmatched::CurrencyConflict { code, .. }
                | Unmatched::BelowThreshold { code, .. },
            ) => Some(code.as_str()),
            _ => None,
        }
    }

    const fn __bool__(&self) -> bool {
        self.matched()
    }

    /// Equal where the two answers are: the same instrument, tier and
    /// flags, or the same miss with the same fields.
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
                "Resolution(matched=True, crosscode='{}', {}, derived={}, listing={})",
                yggdryl::graph::Element::get_crosscode(entry.as_ref()),
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
                    Unmatched::UnknownCode { stated } => {
                        format!(", stated='{}'", stated.as_str())
                    }
                    // A code's `Debug` is its text's.
                    Unmatched::Ambiguous { tier, codes } => {
                        format!(", {}, codes={codes:?}", tier_repr(tier))
                    }
                    Unmatched::CfiConflict { stated, held, code } => {
                        format!(", stated='{stated}', held='{held}', code='{code}'")
                    }
                    Unmatched::CurrencyConflict { stated, held, code } => format!(
                        ", stated='{}', held='{}', code='{code}'",
                        stated.as_str(),
                        held.as_str(),
                    ),
                    Unmatched::BelowThreshold { best, code } => {
                        format!(", best={best}, code='{code}'")
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
