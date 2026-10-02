//! The instrument registry: one row per ISIN of the equivalents it is known
//! by, learned from and filled into market data, read from and written to
//! any holder through the record surface. Shared behind one lock, so a
//! codec handed the registry and the caller holding it see one table.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::{IsinEntry, IsinRegistry};

use crate::fix::JsFixMsg;
use crate::iobase::{LocationInput, located_from_input};
use crate::iomedia::JsBatchReader;
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

/// A table of instruments keyed by ISIN - each row the instrument's CFI
/// code, its market, its ticker and one code per `SecurityIDSource(22)`
/// type - that a lifecycle learns into and fills from. Mutable and shared:
/// equal only to itself; its rows cross out as an Arrow stream.
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

    /// Wraps a registry the core read.
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
    /// core's 16,384 when unstated; learning skips a new ISIN past the bound
    /// and loading refuses it.
    #[napi(constructor)]
    pub fn new(max_instruments: Option<f64>) -> Result<Self> {
        Ok(Self::from_core(
            IsinRegistry::new().with_max_instruments(bound_of(max_instruments)?),
        ))
    }

    /// A registry read from `location` - an `IOBase` or anything a location
    /// is read from: an Arrow IPC file, Parquet, a folder of either, an
    /// object store - its columns named by the registry's own names or any
    /// spelling of an identifier type (`RIC`, `BloombergSymbol`,
    /// `ISINCode`), rows of one ISIN folded by `updunix`; a missing store is
    /// the empty registry.
    #[napi(factory)]
    pub fn from_handle(location: LocationInput<'_>, max_instruments: Option<f64>) -> Result<Self> {
        let holder = located_from_input(location)?;
        let mut registry = IsinRegistry::new().with_max_instruments(bound_of(max_instruments)?);
        registry.extend_from_handle(&holder).map_err(napi_error)?;
        Ok(Self::from_core(registry))
    }

    /// A registry read from a `BatchReader` - `BatchReader.from` widens an
    /// Arrow JS table, a batch or IPC bytes into one - as `fromHandle` reads
    /// a holder's rows.
    #[napi(factory)]
    pub fn from_arrow_reader(
        reader: &mut JsBatchReader,
        max_instruments: Option<f64>,
    ) -> Result<Self> {
        let mut registry = IsinRegistry::new().with_max_instruments(bound_of(max_instruments)?);
        registry
            .extend_from_arrow_reader(reader.take()?)
            .map_err(napi_error)?;
        Ok(Self::from_core(registry))
    }

    /// Folds the rows `location` holds in, by the update rule; how many rows
    /// it read.
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
    /// upserts.
    #[napi]
    #[allow(clippy::wrong_self_convention)]
    pub fn into_arrow_reader(&self) -> Result<JsBatchReader> {
        let reader = self.lock().into_arrow_reader().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, ROOT_NAME))
    }

    /// The row of `isin` as a plain object of its columns, or `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get(&self, isin: String) -> Option<JsScalar> {
        Self::row(self.lock().get(&isin))
    }

    /// The row the RIC `ric` names, as a plain object of its columns, or
    /// `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_by_ric(&self, ric: String) -> Option<JsScalar> {
        Self::row(self.lock().get_by_ric(&ric))
    }

    /// Folds one row - an object of column names to cells, `isin` required
    /// - into the row of its ISIN by the update rule: a column the row
    /// lacks is filled, one it holds is replaced by a statement at or after
    /// the row's `updunix` and kept against an older one, a refining CFI
    /// code refines whatever the time. Whether anything moved.
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
    /// stated ISIN, else by its stated RIC, which only fills - dated at its
    /// `currunix`. Whether anything moved.
    #[napi]
    pub fn learn(&self, message: &JsFixMsg) -> bool {
        self.lock().learn(message.as_core())
    }

    /// Fills what a message leaves unsaid about its instrument from the row
    /// its ISIN, else its RIC, names - each equivalent as a `derived`
    /// identifier, the listing codes and the ticker on its own market, its
    /// CFI code where the row's refines it - never its wire. Whether
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

    /// Render `IsinRegistry(len=…, maxInstruments=…)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        let registry = self.lock();
        format!(
            "IsinRegistry(len={}, maxInstruments={})",
            registry.len(),
            registry.max_instruments()
        )
    }
}
