//! The instruments: one row per instrument, keyed by its cross code, of
//! every fact it is known by - its listings nested - learned from and
//! filled into market data, bound to a store it is loaded from and
//! committed back to. Shared behind one lock, so a codec handed the
//! collection and the caller holding it see one table.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use napi::Either;
use napi::bindgen_prelude::{Either11, FromNapiValue, Null, Result, Unknown};
use napi_derive::napi;
use yggdryl::holder::Holder;
use yggdryl_market::{
    IdType, Instrument, Instruments, Listing, MatchTier, Resolution as CoreResolution, Unmatched,
};

use crate::field::JsField;

use crate::fix::JsFixMsg;
use crate::graph::AnyMarketData;
use crate::iobase::{LocationInput, located_from_input, location_target};
use crate::iomedia::JsBatchReader;
use crate::ioresult::JsIOResult;
use crate::mic::mic_of;
use crate::napi_error;
use crate::text::codec::JsScalar;

/// The root name the collection's row stream crosses under.
const ROOT_NAME: &str = "instrument";

/// The bound a caller's count states, or the core's default.
fn bound_of(max_instruments: Option<f64>) -> Result<usize> {
    let Some(max) = max_instruments else {
        return Ok(Instruments::DEFAULT_MAX_INSTRUMENTS);
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

/// What `Instruments.resolve` answers for one element, as a plain object:
/// the instrument it names and how, or why none - each refusal's fields
/// spelled out, so a caller acts on a `CfiConflict` without reading text.
/// A field a variant does not state is `null`.
#[napi(object, object_from_js = false)]
pub struct Resolution {
    /// Whether an instrument was matched.
    pub matched: bool,
    /// The matched instrument as a plain object of its columns.
    #[napi(ts_type = "Record<string, unknown> | null")]
    pub entry: Either<JsScalar, Null>,
    /// The tier that matched - or, for `Ambiguous`, found the two -
    /// `isin`, `crosscode`, `code`, `symbology` or `economic`.
    #[napi(ts_type = "'isin' | 'crosscode' | 'code' | 'symbology' | 'economic' | null")]
    pub tier: Either<&'static str, Null>,
    /// The type of the code a `code` tier read, one of
    /// `Instruments.lookupCodes()`.
    #[napi(ts_type = "string | null")]
    pub kind: Either<String, Null>,
    /// How similar the short names an `economic` tier weighed are, from the
    /// threshold to `1`.
    #[napi(ts_type = "number | null")]
    pub similarity: Either<f64, Null>,
    /// Whether the ISIN was derived - the element stated none; a match only.
    #[napi(ts_type = "boolean | null")]
    pub derived: Either<bool, Null>,
    /// Whether the instrument's listing on the element's market is the
    /// element's: its market is listed, or it states none and the
    /// instrument has one listing; a match only.
    #[napi(ts_type = "boolean | null")]
    pub listing: Either<bool, Null>,
    /// Why none matched, the refusal's name: `NoKey`, `UnknownIsin`,
    /// `UnknownCode`, `NoCandidate`, `Ambiguous`, `CfiConflict`,
    /// `CurrencyConflict` or `BelowThreshold`.
    #[napi(
        ts_type = "'NoKey' | 'UnknownIsin' | 'UnknownCode' | 'NoCandidate' | 'Ambiguous' | 'CfiConflict' | 'CurrencyConflict' | 'BelowThreshold' | null"
    )]
    pub unmatched: Either<&'static str, Null>,
    /// The cross codes of the instruments an `Ambiguous` key or score
    /// names, in code order.
    #[napi(ts_type = "string[] | null")]
    pub codes: Either<Vec<String>, Null>,
    /// The element's own: the ISIN an `UnknownIsin` states, the cross code
    /// an `UnknownCode`'s facts spell, the CFI category of a `CfiConflict`,
    /// the origin currency of a `CurrencyConflict`.
    #[napi(ts_type = "string | null")]
    pub stated: Either<String, Null>,
    /// The instrument's: the CFI category of a `CfiConflict`, the origin
    /// currency of a `CurrencyConflict`.
    #[napi(ts_type = "string | null")]
    pub held: Either<String, Null>,
    /// How similar the most similar instrument of a `BelowThreshold` is.
    #[napi(ts_type = "number | null")]
    pub best: Either<f64, Null>,
    /// The cross code of the instrument a `CfiConflict`, a
    /// `CurrencyConflict` or a `BelowThreshold` names.
    #[napi(ts_type = "string | null")]
    pub code: Either<String, Null>,
}

impl Resolution {
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
            codes: Either::B(Null),
            stated: Either::B(Null),
            held: Either::B(Null),
            best: Either::B(Null),
            code: Either::B(Null),
        }
    }

    /// `tier` named and its parameters stated.
    fn with_tier(mut self, tier: &MatchTier) -> Self {
        self.tier = Either::A(match tier {
            MatchTier::Isin => "isin",
            MatchTier::CrossCode => "crosscode",
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
    fn from_core(resolution: &CoreResolution<'_>) -> Self {
        match resolution {
            CoreResolution::Matched {
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
            CoreResolution::Unmatched(why) => {
                let mut answer = Self::empty(false);
                answer.unmatched = Either::A(match why {
                    Unmatched::NoKey => "NoKey",
                    Unmatched::UnknownIsin { stated } => {
                        answer.stated = Either::A(stated.as_str().to_owned());
                        "UnknownIsin"
                    }
                    Unmatched::UnknownCode { stated } => {
                        answer.stated = Either::A(stated.as_str().to_owned());
                        "UnknownCode"
                    }
                    Unmatched::NoCandidate => "NoCandidate",
                    Unmatched::Ambiguous { tier, codes } => {
                        answer = answer.with_tier(tier);
                        answer.codes =
                            Either::A(codes.iter().map(|code| code.as_str().to_owned()).collect());
                        "Ambiguous"
                    }
                    Unmatched::CfiConflict { stated, held, code } => {
                        answer.stated = Either::A(stated.to_string());
                        answer.held = Either::A(held.to_string());
                        answer.code = Either::A(code.as_str().to_owned());
                        "CfiConflict"
                    }
                    Unmatched::CurrencyConflict { stated, held, code } => {
                        answer.stated = Either::A(stated.as_str().to_owned());
                        answer.held = Either::A(held.as_str().to_owned());
                        answer.code = Either::A(code.as_str().to_owned());
                        "CurrencyConflict"
                    }
                    Unmatched::BelowThreshold { best, code } => {
                        answer.best = Either::A(*best);
                        answer.code = Either::A(code.as_str().to_owned());
                        "BelowThreshold"
                    }
                });
                answer
            }
        }
    }
}

/// What `instruments` resolves `item` to, read through the leaf it holds
/// with no copy of it.
fn resolution_of(instruments: &Instruments, item: &AnyMarketData<'_>) -> Resolution {
    let resolution = match item {
        Either11::A(data) => instruments.resolve(&data.inner),
        Either11::B(leaf) => instruments.resolve(&leaf.inner),
        Either11::C(leaf) => instruments.resolve(&leaf.inner),
        Either11::D(leaf) => instruments.resolve(&leaf.inner),
        Either11::E(leaf) => instruments.resolve(&leaf.inner),
        Either11::F(leaf) => instruments.resolve(&leaf.inner),
        Either11::G(leaf) => instruments.resolve(&leaf.inner),
        Either11::H(leaf) => instruments.resolve(&leaf.inner),
        Either11::I(leaf) => instruments.resolve(&leaf.inner),
        Either11::J(leaf) => instruments.resolve(&leaf.inner),
        Either11::K(message) => instruments.resolve(message.as_core()),
    };
    Resolution::from_core(&resolution)
}

/// The instruments a process knows, one row each, keyed by its cross code -
/// a security by its bare real ISIN, an FX pair, a derivative or a
/// strategy by the code its class and body spell (`IF:EUR/USD`), a number
/// minted for it under `yggdryl:isin` where no agency numbers it - holding
/// the instrument facts (its `securityids`, its CFI code, its country of
/// issue, its currency, its origin currency, the instrument it is written
/// on, its legs, its characteristics, its product category, its short name,
/// its metadata, `updunix`, `firstunix` and `lastunix`) and, nested, one
/// listing per market (its ticker, its trading currency, its listing
/// codes). A lifecycle learns into it and fills from it, and a parse fills
/// from it. With no key, a code of `lookupCodes()` or a ticker on its
/// market finds an instrument (`getByCode`, `getByTicker`, `resolve`).
/// Bound to the store it was loaded from, committed back only where it
/// moved. Mutable and shared: equal only to itself; its rows cross out as
/// an Arrow stream.
#[napi(js_name = "Instruments")]
pub struct JsInstruments {
    pub(crate) inner: Arc<Mutex<Instruments>>,
}

impl JsInstruments {
    /// The table, a poisoned lock recovered: every verb leaves it whole.
    fn lock(&self) -> MutexGuard<'_, Instruments> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

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

    /// One instrument as the named struct `Scalar` of its columns - each
    /// listing, leg and the characteristics a named struct of its own - which
    /// the loader reads as a plain object, nested objects included.
    fn row(entry: Option<&Instrument>) -> Option<JsScalar> {
        entry.map(|held| JsScalar::from_core(held.into_scalar()))
    }

    /// One listing as the struct `Scalar` of its columns.
    fn listing(listing: Option<&Listing>) -> Option<JsScalar> {
        listing.map(|held| JsScalar::from_core(held.into_scalar()))
    }
}

#[napi]
impl JsInstruments {
    /// An empty collection holding at most `maxInstruments` instruments,
    /// the core's 16,384 when unstated, bound to no store; learning skips a
    /// new instrument past the bound and loading refuses it.
    #[napi(constructor)]
    pub fn new(max_instruments: Option<f64>) -> Result<Self> {
        Ok(Self::from_core(
            Instruments::new().with_max_instruments(bound_of(max_instruments)?),
        ))
    }

    /// The collection's row: the required struct `instrument` every
    /// instrument is laid out as - the six element columns (`uuid`,
    /// `crossuuid`, `crosscode`, `hashcode`, `crosshashcode`, `srcuuids`),
    /// `aliascodes`, `placeholder`, `isin`, `cficode`, `forexcode`, `fisn`,
    /// `countrycode`, `currency`, `origccy`, `securityids`, `underlying`,
    /// `legs`, `eusipacode`, `characteristics`, `listings` (one struct per
    /// market: `miccode`, `ticker`, `currency`, `codes`), `metadata`,
    /// `updunix`, `firstunix`, `lastunix`: twenty-five columns - what a
    /// table holding the instruments is created from. Its root declares
    /// `PARTITION:by` `["truncate(crosscode, 2)"]` - an Iceberg table
    /// created from it partitions by the code's first two characters,
    /// storing no column - and `SORT:by` `["crosscode"]`, the order the
    /// snapshot streams in.
    #[napi]
    pub fn field() -> JsField {
        JsField::from_core(Instrument::field())
    }

    /// A collection holding the seed - the common instruments
    /// `config/instruments/instruments.json` states, embedded at build
    /// time: each a stock, a fund or an index by its ISIN, its listings
    /// (its ticker, its market but an index's, its trading currency), its
    /// country, its detailed CFI code and its short name - clean, bound to
    /// no store, bounded at the core's 16,384. A seed object is an ordinary
    /// statement, so the facts it implies - the national number its ISIN
    /// embeds, the currency of its market's country - are derived as for
    /// any other. `new` holds none of it.
    #[napi(factory)]
    pub fn seeded() -> Self {
        Self::from_core(Instruments::seeded())
    }

    /// A collection bound to the store `location` names and loaded from
    /// it: a URL of any scheme this build holds, a path or an `IOBase` - an
    /// Arrow IPC leaf, Parquet, a folder of parts, an Iceberg table, an
    /// object store - under the `properties` a `with (...)` clause would
    /// state; a store holding nothing yet is an empty first run, laid out
    /// by the first `commit`. Clean after the load. Unseeded: the store's
    /// rows and nothing else - `seededFromUrl` lays them over the seed.
    #[napi(factory)]
    pub fn from_url(
        location: LocationInput<'_>,
        max_instruments: Option<f64>,
        properties: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        let holder = store_of(location, properties)?;
        let instruments = Instruments::new()
            .with_max_instruments(bound_of(max_instruments)?)
            .try_with_holder(holder)
            .map_err(napi_error)?;
        Ok(Self::from_core(instruments))
    }

    /// `fromUrl` laid over the seed (`seeded`): the store `location` names,
    /// read the same way, its rows folded over the seed's by the update
    /// rule - a value the store states wins, a fact only the seed states
    /// stands beside it, a seed instrument it has no row of stands - and a
    /// store holding nothing yet the seed bound to it. Clean after the
    /// load, so the first `commit` after something moved writes the seed's
    /// rows with the store's. `maxInstruments` bounds what is learned and
    /// merged after the load, as `fromArrowReader`'s does.
    #[napi(factory)]
    pub fn seeded_from_url(
        location: LocationInput<'_>,
        max_instruments: Option<f64>,
        properties: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        let bound = bound_of(max_instruments)?;
        let instruments = Instruments::seeded_from_holder(store_of(location, properties)?)
            .map_err(napi_error)?
            .with_max_instruments(bound);
        Ok(Self::from_core(instruments))
    }

    /// The instruments the process environment names, loaded on the first
    /// call and shared with every later one and with `FixCodec.fromEnv`:
    /// an installed collection, else the store `YGGDRYL_INSTRUMENTS_URI`
    /// names - a URL of any scheme, a path, `~` the home - else
    /// `~/.config/yggdryl/instruments/`, a folder of Arrow IPC parts the
    /// first `commit` lays out; with no home, the seed bound to nothing. A
    /// store is laid over the seed - its rows win, a seed instrument it
    /// lacks stands - and the collection is clean after the load. A failed
    /// load throws and is retried by the next call.
    #[napi(factory)]
    pub fn from_env() -> Result<Self> {
        Instruments::from_env()
            .map(Self::from_shared)
            .map_err(napi_error)
    }

    /// Installs `instruments` as the collection every later `fromEnv`
    /// answers - this very table, shared - before anything resolves one;
    /// throws once the default has resolved or been installed.
    #[napi]
    pub fn install_env(instruments: &JsInstruments) -> Result<()> {
        Instruments::install_env_shared(Arc::clone(&instruments.inner)).map_err(napi_error)
    }

    /// A collection read from a `BatchReader` - `BatchReader.from` widens
    /// an Arrow JS table, a batch or IPC bytes into one - its columns named
    /// as `fromUrl` reads them; bound to no store, and clean.
    #[napi(factory)]
    pub fn from_arrow_reader(
        reader: &mut JsBatchReader,
        max_instruments: Option<f64>,
    ) -> Result<Self> {
        let bound = bound_of(max_instruments)?;
        let instruments = Instruments::from_arrow_reader(reader.take()?)
            .map_err(napi_error)?
            .with_max_instruments(bound);
        Ok(Self::from_core(instruments))
    }

    /// Folds the rows `location` holds in, by the update rule, leaving the
    /// collection bound to the store it was; how many rows it read.
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

    /// Every instrument as a `BatchReader` under the collection's row
    /// field, in cross code order: a snapshot taken under the lock, which a
    /// learn while it streams does not move. Write it with an `IOBase`'s
    /// `writeArrowReader` - an overwrite saves a snapshot, a merge by
    /// `crosscode` upserts - or `commit` the collection.
    #[napi]
    #[allow(clippy::wrong_self_convention)]
    pub fn into_arrow_reader(&self) -> Result<JsBatchReader> {
        let reader = self.lock().into_arrow_reader().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, ROOT_NAME))
    }

    /// Writes the table to the store it is bound to, only where its content
    /// differs from what the store holds (`isDirty`): one overwrite of the
    /// whole snapshot, a leaf rewritten, a folder's parts replaced by one, an
    /// Iceberg table replaced in one atomic snapshot, an emptied collection
    /// clearing the store. The `IOResult` of the write, empty for a clean
    /// collection, which touches the store with no call. Throws on a
    /// collection bound to no store.
    #[napi]
    pub fn commit(&self) -> Result<JsIOResult> {
        self.lock()
            .commit()
            .map(JsIOResult::from_core)
            .map_err(napi_error)
    }

    /// Whether the table's content differs from what the store holds - as
    /// it was loaded or last committed: an instrument added or removed, or
    /// one whose content code or `firstunix`/`lastunix` window moved. A
    /// fact that moved and moved back since the load is no change, so a run
    /// replayed over the same input leaves a clean collection.
    #[napi(getter)]
    pub fn is_dirty(&self) -> bool {
        self.lock().is_dirty()
    }

    /// The instrument `key` names - its cross code, a code it had before a
    /// re-key, or an ISIN it holds, real or minted - as a plain object of
    /// its columns, or `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get(&self, key: String) -> Option<JsScalar> {
        Self::row(self.lock().get(&key))
    }

    /// The listings of the instrument `key` names in MIC order, each a
    /// plain object of its columns (`miccode`, `ticker`, `currency`,
    /// `codes`); empty where it is unknown.
    #[napi(ts_return_type = "Record<string, unknown>[]")]
    pub fn listings(&self, key: String) -> Vec<JsScalar> {
        self.lock()
            .listings(&key)
            .iter()
            .map(|listing| JsScalar::from_core(listing.into_scalar()))
            .collect()
    }

    /// The listing of the instrument `key` names on `market` - a MIC,
    /// checked by the `mic` datatype - as a plain object of its columns, or
    /// `null`.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_listing(&self, key: String, market: String) -> Result<Option<JsScalar>> {
        let market = mic_of(&market)?;
        Ok(Self::listing(self.lock().get_listing(&key, &market)))
    }

    /// The instrument the ticker `ticker` - trimmed - names on `market`, as
    /// a plain object of its columns, or `null`: the one instrument a
    /// listing of which lists the ticker on `market` - a MIC, checked by
    /// the `mic` datatype - else, none lists it there, on no market; where
    /// `market` is unstated (`null` or `XXXX`), on any. Two instruments
    /// answering is ambiguous, and answers none.
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

    /// The instrument the code `value` of type `kind` - one of
    /// `lookupCodes()`, read as its type stores it - names, as a plain
    /// object of its columns, or `null`. Two instruments holding the code
    /// is ambiguous, and answers none, as does a type no lookup reads and a
    /// value its type refuses; `kind` is read as an identifier type's word,
    /// and a word that reads as none throws naming it.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn get_by_code(&self, kind: String, value: String) -> Result<Option<JsScalar>> {
        let kind: IdType = kind.parse().map_err(napi_error)?;
        Ok(Self::row(self.lock().get_by_code(&kind, &value)))
    }

    /// The identifier types a lookup reads, in the order `resolve` reads
    /// them: the national numbers, the global and the vendor codes, then
    /// every other `SecurityIDSource(22)` code naming one instrument. A
    /// currency, a country, an index or an issuer code is no key.
    #[napi]
    pub fn lookup_codes() -> Vec<String> {
        Instruments::LOOKUP_CODES
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect()
    }

    /// The instrument `element` - a `MarketData`, any market leaf or a
    /// `FixMsg` - names, and how, by the one waterfall a fill reads: a real
    /// ISIN it holds decides alone, one the collection lacks ending the
    /// cascade (`UnknownIsin`); else the cross code its own facts spell -
    /// an FX pair's from its `forex` identifier and its CFI class - one the
    /// collection lacks ending it too (`UnknownCode`); else a minted number
    /// it holds; else each code of `lookupCodes()` it holds, in that order,
    /// then its ticker on its market, the first naming one instrument
    /// matching and the first naming two ending the cascade (`Ambiguous`);
    /// and only where all of those found nothing, the economic match: the
    /// instrument listed in the element's stated currency whose short name
    /// is the most similar to the one it states, at least
    /// `economicThreshold`, an instrument of another stated origin currency
    /// or CFI category dropped (`CurrencyConflict`, `CfiConflict`). `XXX`
    /// states no currency and an unclassified `X` no category. This door
    /// always weighs the economic match; a fill takes it only where
    /// `isEconomicMatch` says so.
    #[napi(
        ts_args_type = "element: MarketData | Order | Quote | Execution | OrderEvent | QuoteEvent | ExecutionEvent | TradeEvent | BookEvent | SnapshotEvent | FixMsg"
    )]
    pub fn resolve(&self, element: Unknown<'_>) -> Result<Resolution> {
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

    /// Folds one instrument - an object of column names to cells, as `get`
    /// answers one, a real `isin` or a class and the characteristics its
    /// body is written from required - into the collection by the update
    /// rule: a stated valid value fills a fact the instrument lacks and
    /// replaces one it holds that differs, whatever the time, a code that
    /// is no real value of its type dropped; a compatible CFI code refines
    /// the held one and a contradicting one replaces it. Each listing it
    /// states folds into the instrument's listing of that market, created
    /// where it has none there. `updunix` moves where a fact moved,
    /// `firstunix` becomes the earlier of the two and `lastunix` the later.
    /// Whether anything moved.
    #[napi(ts_args_type = "entry: Record<string, unknown>")]
    pub fn merge(&self, entry: &JsScalar) -> Result<bool> {
        let entry = Instrument::from_scalar(&entry.inner).map_err(napi_error)?;
        self.lock().merge(entry).map_err(napi_error)
    }

    /// Removes the instrument `key` names - by its cross code, an alias or
    /// an ISIN - answering it as a plain object, or `null` where it is
    /// unknown.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn remove(&self, key: String) -> Option<JsScalar> {
        let removed = self.lock().remove(&key);
        Self::row(removed.as_ref())
    }

    /// Removes the listing of the instrument `key` names on `market` - a
    /// MIC, checked by the `mic` datatype - answering it as a plain object,
    /// or `null`; the instrument stays.
    #[napi(ts_return_type = "Record<string, unknown> | null")]
    pub fn remove_listing(&self, key: String, market: String) -> Result<Option<JsScalar>> {
        let market = mic_of(&market)?;
        let removed = self.lock().remove_listing(&key, &market);
        Ok(Self::listing(removed.as_ref()))
    }

    /// Removes every instrument.
    #[napi]
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// How many instruments it holds.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.lock().len()).unwrap_or(u32::MAX)
    }

    /// How many rows a commit writes and the snapshot streams: one per
    /// instrument, its listings nested.
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

    /// Learns what a message states about its instrument - keyed by the
    /// code its facts spell: a stated real ISIN keys a security, a pair
    /// beside a CFI of class `I*` an FX spot, its number minted - dated at
    /// its `transunix`: its CFI code, its ticker, its currency, the pair it
    /// states, its real equivalents and the origin currency it states, its
    /// listing facts onto the listing its market names - and moves
    /// `firstunix` to its `transunix` where that is earlier and `lastunix`
    /// where it is later, so meeting a known instrument again moves the
    /// collection too. Whether anything moved.
    #[napi]
    pub fn learn(&self, message: &JsFixMsg) -> bool {
        self.lock().learn(message.as_core())
    }

    /// Fills what a message leaves unsaid about its instrument from the
    /// instrument `resolve` names - by its ISIN, else the cross code its
    /// facts spell, else a code of `lookupCodes()`, else its ticker on its
    /// market, else, where `isEconomicMatch`, its short name - each
    /// equivalent and the pair as a `derived` identifier, the listing codes
    /// and the ticker of its own market's listing, its CFI code where the
    /// instrument's refines it, the currency on the same stated market
    /// under the listing's ticker, the origin currency where it holds none,
    /// and the instrument's cross code as its `instcode` - never its wire.
    /// Whether anything moved.
    #[napi]
    pub fn fill(&self, message: &mut JsFixMsg) -> bool {
        self.lock().fill(message.as_core_mut())
    }

    /// `learn`, then `fill`. Whether anything moved in either.
    #[napi]
    pub fn enrich(&self, message: &mut JsFixMsg) -> bool {
        self.lock().enrich(message.as_core_mut())
    }

    /// Whether `other` is this collection - the same shared table.
    #[napi]
    pub fn equals(&self, other: &JsInstruments) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Render `Instruments(len=…, maxInstruments=…, dirty=…)`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        let instruments = self.lock();
        format!(
            "Instruments(len={}, maxInstruments={}, dirty={})",
            instruments.len(),
            instruments.max_instruments(),
            instruments.is_dirty()
        )
    }
}
