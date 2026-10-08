//! The instrument registry: one row per ISIN and market of every fact it is
//! known by, learned from and filled into market data, bound to a store it is loaded
//! from and committed back to. Shared behind one lock, so a codec handed the
//! registry and the caller holding it see one table.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use napi::Either;
use napi::bindgen_prelude::{Either11, FromNapiValue, Null, Result, Unknown};
use napi_derive::napi;
use yggdryl::holder::Holder;
use yggdryl::{IdType, IsinEntry, IsinRegistry, MatchTier, Resolution, Unmatched};

use crate::field::JsField;

use crate::fix::JsFixMsg;
use crate::graph::AnyMarketData;
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

/// What `IsinRegistry.resolve` answers for one element, as a plain object:
/// the listing row it names and how, or why none - each refusal's fields
/// spelled out, so a caller acts on a `CfiConflict` without reading text.
/// A field a variant does not state is `null`.
#[napi(object, object_from_js = false)]
pub struct IsinResolution {
    /// Whether a row was matched.
    pub matched: bool,
    /// The matched row as a plain object of its columns: the listing on the
    /// element's market, else the one row holding the key, else the
    /// instrument's single row, else its first.
    #[napi(ts_type = "Record<string, unknown> | null")]
    pub entry: Either<JsScalar, Null>,
    /// The tier that matched - or, for `Ambiguous`, found the two -
    /// `isin`, `code`, `symbology` or `economic`.
    #[napi(ts_type = "'isin' | 'code' | 'symbology' | 'economic' | null")]
    pub tier: Either<&'static str, Null>,
    /// The type of the code a `code` tier read, one of
    /// `IsinRegistry.lookupCodes()`.
    #[napi(ts_type = "string | null")]
    pub kind: Either<String, Null>,
    /// How similar the short names an `economic` tier weighed are, from the
    /// threshold to `1`.
    #[napi(ts_type = "number | null")]
    pub similarity: Either<f64, Null>,
    /// Whether the ISIN was derived - the element stated none; a match only.
    #[napi(ts_type = "boolean | null")]
    pub derived: Either<bool, Null>,
    /// Whether the row's listing facts belong to the element: its market is
    /// the row's, or either is unstated; a match only.
    #[napi(ts_type = "boolean | null")]
    pub listing: Either<bool, Null>,
    /// Why none matched, the refusal's name: `NoKey`, `UnknownIsin`,
    /// `NoCandidate`, `Ambiguous`, `CfiConflict`, `CurrencyConflict` or
    /// `BelowThreshold`.
    #[napi(
        ts_type = "'NoKey' | 'UnknownIsin' | 'NoCandidate' | 'Ambiguous' | 'CfiConflict' | 'CurrencyConflict' | 'BelowThreshold' | null"
    )]
    pub unmatched: Either<&'static str, Null>,
    /// The instruments an `Ambiguous` key or score names, in ISIN order.
    #[napi(ts_type = "string[] | null")]
    pub isins: Either<Vec<String>, Null>,
    /// The element's own: the ISIN an `UnknownIsin` states, the CFI category
    /// of a `CfiConflict`, the origin currency of a `CurrencyConflict`.
    #[napi(ts_type = "string | null")]
    pub stated: Either<String, Null>,
    /// The instrument's: the CFI category of a `CfiConflict`, the origin
    /// currency of a `CurrencyConflict`.
    #[napi(ts_type = "string | null")]
    pub held: Either<String, Null>,
    /// How similar the most similar instrument of a `BelowThreshold` is.
    #[napi(ts_type = "number | null")]
    pub best: Either<f64, Null>,
    /// The instrument a `CfiConflict`, a `CurrencyConflict` or a
    /// `BelowThreshold` names.
    #[napi(ts_type = "string | null")]
    pub isin: Either<String, Null>,
}

impl IsinResolution {
    /// Every field unstated.
    fn empty(matched: bool) -> Self {
        Self {
            matched,
            entry: Either::B(Null),
            tier: Either::B(Null),
            kind: Either::B(Null),
            similarity: Either::B(Null),
            derived: Either::B(Null),
            listing: Either::B(Null),
            unmatched: Either::B(Null),
            isins: Either::B(Null),
            stated: Either::B(Null),
            held: Either::B(Null),
            best: Either::B(Null),
            isin: Either::B(Null),
        }
    }

    /// `tier` named and its parameters stated.
    fn with_tier(mut self, tier: &MatchTier) -> Self {
        self.tier = Either::A(match tier {
            MatchTier::Isin => "isin",
            MatchTier::Code(kind) => {
                self.kind = Either::A(kind.as_str().to_owned());
                "code"
            }
            MatchTier::Symbology => "symbology",
            MatchTier::Economic { similarity } => {
                self.similarity = Either::A(*similarity);
                "economic"
            }
        });
        self
    }

    /// The core's answer, spelled out.
    fn from_core(resolution: &Resolution<'_>) -> Self {
        match resolution {
            Resolution::Matched {
                entry,
                tier,
                derived,
                listing,
            } => {
                let mut answer = Self::empty(true).with_tier(tier);
                answer.entry = Either::A(JsScalar::from_core(entry.into_scalar()));
                answer.derived = Either::A(*derived);
                answer.listing = Either::A(*listing);
                answer
            }
            Resolution::Unmatched(why) => {
                let mut answer = Self::empty(false);
                answer.unmatched = Either::A(match why {
                    Unmatched::NoKey => "NoKey",
                    Unmatched::UnknownIsin { stated } => {
                        answer.stated = Either::A(stated.as_str().to_owned());
                        "UnknownIsin"
                    }
                    Unmatched::NoCandidate => "NoCandidate",
                    Unmatched::Ambiguous { tier, isins } => {
                        answer = answer.with_tier(tier);
                        answer.isins =
                            Either::A(isins.iter().map(|isin| isin.as_str().to_owned()).collect());
                        "Ambiguous"
                    }
                    Unmatched::CfiConflict { stated, held, isin } => {
                        answer.stated = Either::A(stated.to_string());
                        answer.held = Either::A(held.to_string());
                        answer.isin = Either::A(isin.as_str().to_owned());
                        "CfiConflict"
                    }
                    Unmatched::CurrencyConflict { stated, held, isin } => {
                        answer.stated = Either::A(stated.as_str().to_owned());
                        answer.held = Either::A(held.as_str().to_owned());
                        answer.isin = Either::A(isin.as_str().to_owned());
                        "CurrencyConflict"
                    }
                    Unmatched::BelowThreshold { best, isin } => {
                        answer.best = Either::A(*best);
                        answer.isin = Either::A(isin.as_str().to_owned());
                        "BelowThreshold"
                    }
                });
                answer
            }
        }
    }
}

/// What `registry` resolves `item` to, read through the leaf it holds with
/// no copy of it.
fn resolution_of(registry: &IsinRegistry, item: &AnyMarketData<'_>) -> IsinResolution {
    let resolution = match item {
        Either11::A(data) => registry.resolve(&data.inner),
        Either11::B(leaf) => registry.resolve(&leaf.inner),
        Either11::C(leaf) => registry.resolve(&leaf.inner),
        Either11::D(leaf) => registry.resolve(&leaf.inner),
        Either11::E(leaf) => registry.resolve(&leaf.inner),
        Either11::F(leaf) => registry.resolve(&leaf.inner),
        Either11::G(leaf) => registry.resolve(&leaf.inner),
        Either11::H(leaf) => registry.resolve(&leaf.inner),
        Either11::I(leaf) => registry.resolve(&leaf.inner),
        Either11::J(leaf) => registry.resolve(&leaf.inner),
        Either11::K(message) => registry.resolve(message.as_core()),
    };
    IsinResolution::from_core(&resolution)
}

/// A table of instruments keyed by ISIN, one listing row per market - the
/// instrument facts every listing of an ISIN shares (its CFI code, its
/// country of issue, its currency pair, the instrument it is written on,
/// its product category, its ISO 18774 short name, its origin currency,
/// `updunix`, `firstunix` and `lastunix`), and the listing facts of one
/// market (its ticker, its trading currency, its listing codes) beside one
/// code per `SecurityIDSource(22)` type - that a lifecycle learns into and
/// fills from, and a parse fills from. The ISIN is the key; with none, a
/// code of `lookupCodes()` or a ticker on its market finds an instrument
/// (`getByCode`, `getByTicker`, `resolve`). Bound to the store it was loaded
/// from, committed back only where it moved. Mutable and shared: equal only
/// to itself; its rows cross out as an Arrow stream.
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

    /// Rows as the struct `Scalar`s of their columns, in the order given.
    fn rows_of<'a>(entries: impl IntoIterator<Item = &'a IsinEntry>) -> Vec<JsScalar> {
        entries
            .into_iter()
            .map(|held| JsScalar::from_core(held.into_scalar()))
            .collect()
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

    /// The registry's row: the required struct `isinregistry` every listing
    /// row is laid out as - `isin`, `updunix`, `firstunix`, `lastunix`,
    /// `cficode`, `countrycode`, `forexcode`, `underlyingisin`,
    /// `eusipacode`, `miccode`, `ticker`, `fisn`, `currency`, `origccy`,
    /// then one column per `SecurityIDSource(22)` type but the ISIN:
    /// forty-six columns - what a
    /// table holding the registry is created from. Its root declares
    /// `PARTITION:by` `["truncate(isin, 2)"]` - an Iceberg table created
    /// from it partitions by the ISIN's country prefix, storing no column -
    /// and `SORT:by` `["isin", "miccode"]`, the order the snapshot streams
    /// in.
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

    /// Every listing row as a `BatchReader` under the registry's row field,
    /// in ISIN then MIC order: a snapshot taken under the lock, which a
    /// learn while it streams does not move. Write it with an `IOBase`'s
    /// `writeArrowReader` - an overwrite saves a snapshot, a merge by `isin`
    /// and `miccode` upserts - or `commit` the registry.
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

    /// The first listing row of `isin` in MIC order - the unlisted row
    /// where that is all it holds - as a plain object of its columns, or
    /// `null`. Its instrument facts are every listing's; `listings` answers
    /// them all.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get(&self, isin: String) -> Option<JsScalar> {
        Self::row(self.lock().get(&isin))
    }

    /// Every listing row of `isin` in MIC order, each a plain object of its
    /// columns; empty where the ISIN is unknown.
    #[napi(ts_return_type = "Record<string, unknown>[]")]
    pub fn listings(&self, isin: String) -> Vec<JsScalar> {
        Self::rows_of(self.lock().listings(&isin))
    }

    /// The listing row of `isin` on `market` - a MIC, checked by the `mic`
    /// datatype - as a plain object of its columns, or `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_listing(&self, isin: String, market: String) -> Result<Option<JsScalar>> {
        let market = mic_of(&market)?;
        Ok(Self::row(self.lock().get_listing(&isin, &market)))
    }

    /// The listing row the ticker `ticker` names on `market`, as a plain
    /// object of its columns, or `null`: the one ISIN a row of which lists
    /// the ticker on `market` - a MIC, checked by the `mic` datatype - else
    /// on no market; where `market` is unstated (`null` or `XXXX`), on any;
    /// then that ISIN's row on `market`, else the one row listing the
    /// ticker, else its single row, else its first. Two ISINs answering is
    /// ambiguous, and answers none; two listings of one ISIN are one
    /// instrument.
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

    /// The listing row the code `value` of type `kind` - one of
    /// `lookupCodes()`, read as its type stores it - names on `market`, as a
    /// plain object of its columns, or `null`: the one ISIN a row of which
    /// holds the code, then its row on `market` - a MIC, checked by the
    /// `mic` datatype - else the one row holding the code, else its single
    /// row, else its first. Two ISINs holding the code is ambiguous, and
    /// answers none, as does a type no lookup reads and a value its type
    /// refuses; a word no identifier type spells throws.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_by_code(
        &self,
        kind: String,
        value: String,
        market: Option<String>,
    ) -> Result<Option<JsScalar>> {
        let kind: IdType = kind.parse().map_err(napi_error)?;
        let market = market.as_deref().map(mic_of).transpose()?;
        let registry = self.lock();
        Ok(Self::row(registry.get_by_code(
            &kind,
            &value,
            market.as_ref(),
        )))
    }

    /// The identifier types a lookup reads, in the order `resolve` reads
    /// them: the national numbers, the global and the vendor codes, then
    /// every other `SecurityIDSource(22)` code naming one instrument. A
    /// currency, a country, an index or an issuer code is no key.
    #[napi]
    pub fn lookup_codes() -> Vec<String> {
        IsinRegistry::LOOKUP_CODES
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect()
    }

    /// The listing row `element` - a `MarketData`, any market leaf or a
    /// `FixMsg` - names, and how, by the one waterfall a fill reads: a real
    /// ISIN it holds decides alone, one the registry lacks ending the
    /// cascade (`UnknownIsin`); with none, each code of `lookupCodes()` it
    /// holds, in that order, then its ticker on its market, the first naming
    /// one instrument matching and the first naming two ending the cascade
    /// (`Ambiguous`); and only where all of those found nothing, the
    /// economic match: the instrument listed in the element's stated
    /// currency whose short name is the most similar to the one it states,
    /// at least `economicThreshold`, an instrument of another stated origin
    /// currency or CFI category dropped (`CurrencyConflict`,
    /// `CfiConflict`). `XXX` states no currency and an unclassified `X` no
    /// category. This door always weighs the economic match; a fill takes
    /// it only where `isEconomicMatch` says so.
    #[napi(
        ts_args_type = "element: MarketData | Order | Quote | Execution | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent | FixMsg"
    )]
    pub fn resolve(&self, element: Unknown<'_>) -> Result<IsinResolution> {
        let kind = element.get_type()?.to_string().to_lowercase();
        let item = AnyMarketData::from_unknown(element).map_err(|_| {
            napi_error(format!(
                "expected MarketData, a market leaf or a FixMsg, got {kind}"
            ))
        })?;
        Ok(resolution_of(&self.lock(), &item))
    }

    /// How similar two short names must be, from above `0` to `1`, for an
    /// economic match: `0.85` unless told otherwise.
    #[napi(getter)]
    pub fn economic_threshold(&self) -> f64 {
        self.lock().economic_threshold()
    }

    /// Sets `economicThreshold`; NaN and a value outside `(0, 1]` throw,
    /// naming the value, and move nothing.
    #[napi]
    pub fn set_economic_threshold(&self, threshold: f64) -> Result<()> {
        self.lock()
            .set_economic_threshold(threshold)
            .map_err(napi_error)
    }

    /// Whether a fill - `fill`, `enrich`, a lifecycle's - takes an economic
    /// match where nothing exact names the element; `false` unless told
    /// otherwise, since a derived ISIN becomes the key an element's book and
    /// chain live under. A parse never takes one.
    #[napi(getter)]
    pub fn is_economic_match(&self) -> bool {
        self.lock().is_economic_match()
    }

    /// Sets `isEconomicMatch`.
    #[napi]
    pub fn set_economic_match(&self, enabled: bool) {
        self.lock().set_economic_match(enabled);
    }

    /// Folds one row - an object of column names to cells, `isin` required
    /// - into the listings of its ISIN by the update rule: a stated valid
    /// value fills a column a row lacks and replaces one it holds that
    /// differs, whatever the time, a code that is no real value of its
    /// type dropped; a compatible CFI code refines the held one and a
    /// contradicting one replaces it. The instrument facts fold into every
    /// listing of the ISIN; the listing facts - the ticker, the currency,
    /// the listing codes - into the listing of the market the row names,
    /// created where the ISIN has none there, and, where it names none,
    /// into the ISIN's single listing, or into none, with one warning per
    /// column, where it has several. `updunix` moves where a fact moved,
    /// `firstunix` becomes the earlier of the two and `lastunix` the later.
    /// Whether anything moved.
    #[napi(ts_args_type = "entry: Record<string, unknown>")]
    pub fn merge(&self, entry: &JsScalar) -> Result<bool> {
        let entry = IsinEntry::from_scalar(&entry.inner).map_err(napi_error)?;
        self.lock().merge(entry).map_err(napi_error)
    }

    /// Removes every listing row of `isin`, answering them in MIC order as
    /// plain objects; empty where the ISIN is unknown.
    #[napi(ts_return_type = "Record<string, unknown>[]")]
    pub fn remove(&self, isin: String) -> Vec<JsScalar> {
        let removed = self.lock().remove(&isin);
        Self::rows_of(&removed)
    }

    /// Removes the listing row of `isin` on `market` - a MIC, checked by
    /// the `mic` datatype - answering it as a plain object, or `null`; the
    /// instrument goes with its last listing.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn remove_listing(&self, isin: String, market: String) -> Result<Option<JsScalar>> {
        let market = mic_of(&market)?;
        let removed = self.lock().remove_listing(&isin, &market);
        Ok(Self::row(removed.as_ref()))
    }

    /// Removes every row.
    #[napi]
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// How many instruments it holds: its ISINs.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.lock().len()).unwrap_or(u32::MAX)
    }

    /// How many listing rows it holds - one per ISIN and market, an
    /// unlisted row one: what the snapshot streams and a commit writes.
    #[napi(getter)]
    pub fn rows(&self) -> u32 {
        u32::try_from(self.lock().rows()).unwrap_or(u32::MAX)
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
    /// equivalents and the origin currency it states, onto the listing its
    /// market names - and moves `firstunix` to its `currunix` where that is
    /// earlier and `lastunix` where it is later, so meeting a known
    /// instrument again moves the registry too. Whether anything moved.
    #[napi]
    pub fn learn(&self, message: &JsFixMsg) -> bool {
        self.lock().learn(message.as_core())
    }

    /// Fills what a message leaves unsaid about its instrument from the row
    /// `resolve` names - by its ISIN, else a code of `lookupCodes()`, else
    /// its ticker on its market, else, where `isEconomicMatch`, its short
    /// name - each equivalent and the pair as a `derived` identifier, the
    /// ticker on its own market, its CFI code where the row's refines it,
    /// the currency on the same stated market under the row's ticker, the
    /// origin currency where it holds none - never its wire. Whether
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
