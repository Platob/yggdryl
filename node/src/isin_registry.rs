//! The instrument registry: one row per ISIN of every fact it is known by,
//! learned from and filled into market data, bound to a store it is loaded
//! from and committed back to. Shared behind one lock, so a codec handed the
//! registry and the caller holding it see one table.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use napi::Either;
use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::holder::Holder;
use yggdryl::{IsinEntry, IsinRegistry};

use crate::field::JsField;

use crate::fix::JsFixMsg;
use crate::iobase::{LocationInput, located_from_input, location_target};
use crate::iomedia::JsBatchReader;
use crate::ioresult::JsIOResult;
use crate::mic::mic_of;
use crate::napi_error;
use crate::text::codec::JsScalar;

/// The root name the registry's row stream crosses under.
const ROOT_NAME: &str = "isinregistry";

/// The bound a caller's count states, or the core's default.
fn bound_of(max_instruments: Option<f64>) -> Result<usize> {
    let Some(max) = max_instruments else {
        return Ok(IsinRegistry::DEFAULT_MAX_INSTRUMENTS);
    };
    let max = crate::exact_i64(max, "maxInstruments")?;
    usize::try_from(max).map_err(|_| napi_error("maxInstruments must not be negative"))
}

/// The store `location` names: an `IOBase` already built, rebuilt, or the
/// holder a location names under `properties`.
fn store_of(
    location: LocationInput<'_>,
    properties: Option<HashMap<String, String>>,
) -> Result<Holder> {
    match location_target(location)? {
        Either::A(handle) => {
            if properties.as_ref().is_some_and(|held| !held.is_empty()) {
                return Err(napi_error(
                    "properties apply to a location, not to a handle already built",
                ));
            }
            Ok(handle.rebuilt()?.into_core())
        }
        Either::B(url) => {
            Holder::from_url(&url, properties.unwrap_or_default()).map_err(napi_error)
        }
    }
}

/// A table of instruments keyed by ISIN - each row the instrument's CFI
/// code, its country of issue, its currency pair, the instrument it is
/// written on, its market, its ticker, its ISO 18774 short name
/// and trading currency and one code per `SecurityIDSource(22)` type - that
/// a lifecycle learns into and fills from, and a parse fills from. Bound to
/// the store it was loaded from, committed back only where it moved.
/// Mutable and shared: equal only to itself; its rows cross out as an Arrow
/// stream.
#[napi(js_name = "IsinRegistry")]
pub struct JsIsinRegistry {
    pub(crate) inner: Arc<Mutex<IsinRegistry>>,
}

impl JsIsinRegistry {
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

    /// Wraps a registry the core built.
    fn from_core(inner: IsinRegistry) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// One row as the struct `Scalar` of its columns, which the loader
    /// reads as a plain object.
    fn row(entry: Option<&IsinEntry>) -> Option<JsScalar> {
        entry.map(|held| JsScalar::from_core(held.into_scalar()))
    }
}

#[napi]
impl JsIsinRegistry {
    /// An empty registry holding at most `maxInstruments` instruments, the
    /// core's 16,384 when unstated, bound to no store; learning skips a new
    /// ISIN past the bound and loading refuses it.
    #[napi(constructor)]
    pub fn new(max_instruments: Option<f64>) -> Result<Self> {
        Ok(Self::from_core(
            IsinRegistry::new().with_max_instruments(bound_of(max_instruments)?),
        ))
    }

    /// The registry's row: the required struct `isinregistry` every row is
    /// laid out as - `isin`, `updunix`, `cficode`, `countrycode`,
    /// `forexcode`, `underlyingisin`, `eusipacode`, `miccode`, `ticker`,
    /// `fisn`, `currency`, then one column per `SecurityIDSource(22)` type
    /// but the ISIN: forty-three columns - what a table holding the
    /// registry is created from. Its root declares `PARTITION:by`
    /// `["truncate(isin, 2)"]` - an Iceberg table created from it partitions
    /// by the ISIN's country prefix, storing no column - and `SORT:by`
    /// `["isin"]`, the order the snapshot streams in.
    #[napi]
    pub fn field() -> JsField {
        JsField::from_core(IsinEntry::field())
    }

    /// A registry holding the seed - the common instruments
    /// `config/isin/instruments.json` states, embedded at build time: each a
    /// stock, a fund or an index by its ISIN, its ticker, its market but an
    /// index's, its trading currency, its country, its detailed CFI code and
    /// its short name - clean, bound to no store, bounded at the core's
    /// 16,384. A seed row is an ordinary statement, so the facts it implies
    /// - the national number its ISIN embeds, the currency of its market's
    /// country - are derived as for any other. `new` holds none of it.
    #[napi(factory)]
    pub fn seeded() -> Self {
        Self::from_core(IsinRegistry::seeded())
    }

    /// A registry bound to the store `location` names and loaded from it:
    /// a URL of any scheme this build holds, a path or an `IOBase` - an
    /// Arrow IPC leaf, Parquet, a folder of parts, an Iceberg table, an
    /// object store - under the `properties` a `with (...)` clause would
    /// state, its columns named by the registry's own names or any spelling
    /// of an identifier type; a store holding nothing yet is an empty first
    /// run, laid out by the first `commit`. Clean after the load. Unseeded:
    /// the store's rows and nothing else - `seededFromUrl` lays them over
    /// the seed.
    #[napi(factory)]
    pub fn from_url(
        location: LocationInput<'_>,
        max_instruments: Option<f64>,
        properties: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        let holder = store_of(location, properties)?;
        let registry = IsinRegistry::new()
            .with_max_instruments(bound_of(max_instruments)?)
            .try_with_holder(holder)
            .map_err(napi_error)?;
        Ok(Self::from_core(registry))
    }

    /// `fromUrl` laid over the seed (`seeded`): the store `location` names,
    /// read the same way, its rows folded over the seed's by the update
    /// rule - a value the store states wins, a fact only the seed states
    /// stands beside it, a seed row it has no row of stands - and a store
    /// holding nothing yet the seed bound to it. Clean after the load, so
    /// the first `commit` after something moved writes the seed's rows with
    /// the store's. `maxInstruments` bounds what is learned and merged
    /// after the load, as `fromArrowReader`'s does.
    #[napi(factory)]
    pub fn seeded_from_url(
        location: LocationInput<'_>,
        max_instruments: Option<f64>,
        properties: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        let bound = bound_of(max_instruments)?;
        let registry = IsinRegistry::seeded_from_holder(store_of(location, properties)?)
            .map_err(napi_error)?
            .with_max_instruments(bound);
        Ok(Self::from_core(registry))
    }

    /// The registry the process environment names, loaded on the first
    /// call and shared with every later one and with `FixCodec.fromEnv`:
    /// an installed registry, else the store `YGGDRYL_ISIN_REGISTRY_URI`
    /// names - a URL of any scheme, a path, `~` the home - else
    /// `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first
    /// `commit` lays out; with no home, the seed bound to nothing. A store
    /// is laid over the seed - its rows win, a seed row it lacks stands -
    /// and the registry is clean after the load.
    /// A failed load throws and is retried by the next call.
    #[napi(factory)]
    pub fn from_env() -> Result<Self> {
        IsinRegistry::from_env()
            .map(Self::from_shared)
            .map_err(napi_error)
    }

    /// Installs `registry` as the one every later `fromEnv` answers - this
    /// very table, shared - before anything resolves one; throws once the
    /// default has resolved or been installed.
    #[napi]
    pub fn install_env(registry: &JsIsinRegistry) -> Result<()> {
        IsinRegistry::install_env_shared(Arc::clone(&registry.inner)).map_err(napi_error)
    }

    /// A registry read from a `BatchReader` - `BatchReader.from` widens an
    /// Arrow JS table, a batch or IPC bytes into one - its columns named as
    /// `fromUrl` reads them; bound to no store, and clean.
    #[napi(factory)]
    pub fn from_arrow_reader(
        reader: &mut JsBatchReader,
        max_instruments: Option<f64>,
    ) -> Result<Self> {
        let bound = bound_of(max_instruments)?;
        let registry = IsinRegistry::from_arrow_reader(reader.take()?)
            .map_err(napi_error)?
            .with_max_instruments(bound);
        Ok(Self::from_core(registry))
    }

    /// Folds the rows `location` holds in, by the update rule, leaving the
    /// registry bound to the store it was; how many rows it read.
    #[napi]
    pub fn extend_from_handle(&self, location: LocationInput<'_>) -> Result<u32> {
        let holder = located_from_input(location)?;
        let read = self
            .lock()
            .extend_from_handle(&holder)
            .map_err(napi_error)?;
        Ok(u32::try_from(read).unwrap_or(u32::MAX))
    }

    /// Folds an Arrow stream's rows in, by the update rule; how many rows it
    /// read.
    #[napi]
    pub fn extend_from_arrow_reader(&self, reader: &mut JsBatchReader) -> Result<u32> {
        let reader = reader.take()?;
        let read = self
            .lock()
            .extend_from_arrow_reader(reader)
            .map_err(napi_error)?;
        Ok(u32::try_from(read).unwrap_or(u32::MAX))
    }

    /// The rows as a `BatchReader` under the registry's row field, in ISIN
    /// order: a snapshot taken under the lock, which a learn while it
    /// streams does not move. Write it with an `IOBase`'s
    /// `writeArrowReader` - an overwrite saves a snapshot, a merge by `isin`
    /// upserts - or `commit` the registry.
    #[napi]
    #[allow(clippy::wrong_self_convention)]
    pub fn into_arrow_reader(&self) -> Result<JsBatchReader> {
        let reader = self.lock().into_arrow_reader().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, ROOT_NAME))
    }

    /// Writes the table to the store it is bound to, only where it moved
    /// since it was loaded or last committed: one overwrite of the whole
    /// snapshot, a leaf rewritten, a folder's parts replaced by one, an
    /// Iceberg table replaced in one atomic snapshot, an emptied registry
    /// clearing the store. The `IOResult` of the write, empty for a clean
    /// registry, which touches the store with no call. Throws on a registry
    /// bound to no store.
    #[napi]
    pub fn commit(&self) -> Result<JsIOResult> {
        self.lock()
            .commit()
            .map(JsIOResult::from_core)
            .map_err(napi_error)
    }

    /// Whether the table moved since it was loaded or last committed.
    #[napi(getter)]
    pub fn is_dirty(&self) -> bool {
        self.lock().is_dirty()
    }

    /// The row of `isin` as a plain object of its columns, or `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get(&self, isin: String) -> Option<JsScalar> {
        Self::row(self.lock().get(&isin))
    }

    /// The row the ticker `ticker` names on `market`, as a plain object of
    /// its columns, or `null`: the one row listing the ticker whose market
    /// is `market` - a MIC, checked by the `mic` datatype - or whose market
    /// or `market` is unstated (`null` or `XXXX`). Two rows answering is
    /// ambiguous, and answers none.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_by_ticker(
        &self,
        ticker: String,
        market: Option<String>,
    ) -> Result<Option<JsScalar>> {
        let market = market.as_deref().map(mic_of).transpose()?;
        Ok(Self::row(
            self.lock().get_by_ticker(&ticker, market.as_ref()),
        ))
    }

    /// Folds one row - an object of column names to cells, `isin` required
    /// - into the row of its ISIN by the update rule: a stated valid value
    /// fills a column the row lacks and replaces one it holds that
    /// differs, whatever the time, a code that is no real value of its
    /// type dropped; a compatible CFI code refines the held one and a
    /// contradicting one replaces it; a ticker or a listing code stated on
    /// another market switches the listing whole. Whether anything moved.
    #[napi(ts_args_type = "entry: Record<string, unknown>")]
    pub fn merge(&self, entry: &JsScalar) -> Result<bool> {
        let entry = IsinEntry::from_scalar(&entry.inner).map_err(napi_error)?;
        self.lock().merge(entry).map_err(napi_error)
    }

    /// Removes the row of `isin`, answering it as a plain object, or `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn remove(&self, isin: String) -> Option<JsScalar> {
        let removed = self.lock().remove(&isin);
        Self::row(removed.as_ref())
    }

    /// Removes every row.
    #[napi]
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// How many instruments it holds.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.lock().len()).unwrap_or(u32::MAX)
    }

    /// The most instruments it holds.
    #[napi(getter)]
    pub fn max_instruments(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)] // A bound past 2^53 instruments holds no table.
        let bound = self.lock().max_instruments() as f64;
        bound
    }

    /// Learns what a message states about its instrument - keyed by its
    /// stated real ISIN, dated at its `currunix`: its CFI code, its market,
    /// its ticker, its currency, the pair it states and its real
    /// equivalents. Whether anything moved.
    #[napi]
    pub fn learn(&self, message: &JsFixMsg) -> bool {
        self.lock().learn(message.as_core())
    }

    /// Fills what a message leaves unsaid about its instrument from the row
    /// its ISIN names, else its ticker on its market - each equivalent and
    /// the pair as a `derived` identifier, the ticker on its own market,
    /// its CFI code where the row's refines it, the currency on the same
    /// stated market under the row's ticker - never its wire. Whether
    /// anything moved.
    #[napi]
    pub fn fill(&self, message: &mut JsFixMsg) -> bool {
        self.lock().fill(message.as_core_mut())
    }

    /// `learn`, then `fill`. Whether anything moved in either.
    #[napi]
    pub fn enrich(&self, message: &mut JsFixMsg) -> bool {
        self.lock().enrich(message.as_core_mut())
    }

    /// Whether `other` is this registry - the same shared table.
    #[napi]
    pub fn equals(&self, other: &JsIsinRegistry) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Render `IsinRegistry(len=…, maxInstruments=…, dirty=…)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        let registry = self.lock();
        format!(
            "IsinRegistry(len={}, maxInstruments={}, dirty={})",
            registry.len(),
            registry.max_instruments(),
            registry.is_dirty()
        )
    }
}
