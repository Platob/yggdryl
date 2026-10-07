//! One row per ISIN and market: every fact an instrument is known by,
//! learned from the statements that name it and filled into the ones that
//! leave it unsaid.
//!
//! The ISIN is the one key: an [`IsinRegistry`] holds one [`IsinEntry`] per
//! canonical ISIN and market it is listed on - its listings, in MIC order,
//! or its unlisted row alone while no market is known. Each row holds the
//! instrument's facts - its detailed CFI code, its country of issue, the
//! pair an FX or referential number names, its underlying (the instrument
//! it is written on), its EUSIPA product category, the stamps, and every
//! code that is no listing code - and its own: its market, its ticker and
//! trading currency, and its listing codes. A ticker leads back to its
//! listing through an exact inverse index, gated by the market. A RIC, a
//! Bloomberg symbol, a FIGI, a CUSIP are equivalents the ISIN fills, never
//! keys a lookup reads.
//!
//! A row is updated on differences and no clock gates it: a statement's
//! non-null valid value fills a column the row lacks and replaces one it
//! holds that differs, an invalid value moves nothing, and `updunix` and
//! `lastunix` are stamps ([`IsinRegistry::merge`]). Wherever a row is
//! created or moves, the
//! facts it implies fill, once the statement has landed, the columns it
//! leaves empty - the national number its ISIN embeds
//! ([`securityid::embedded`](crate::securityid::embedded)) and the currency
//! of its market's country - so a default never displaces a statement.
//! The registry holds its table apart from the store it is bound to:
//! [`IsinRegistry::from_holder`] loads one,
//! [`IsinRegistry::seeded_from_holder`] lays one over the seed,
//! [`IsinRegistry::commit`] writes the table back as one snapshot where it
//! moved, and
//! [`IsinRegistry::from_env`] is the process's own, located by
//! `YGGDRYL_ISIN_REGISTRY_URI` and laid over the seed
//! ([`IsinRegistry::seeded`]): the common instruments
//! `config/isin/instruments.json` states, embedded at build time.

pub(crate) mod env;
pub(crate) mod seed;
pub(crate) mod store;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::sync::{Arc, LazyLock};

use arrow_array::{RecordBatch, RecordBatchIterator};
use arrow_schema::{Field as ArrowField, Schema};
use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::graph::{Element, Event, Market};
use crate::identifier::{IDENTIFIER_VALUE_WIDTH, IDENTIFIER_WORD_WIDTH, fold_into};
use crate::idtype::FIX_SECURITY_SOURCES;
use crate::logging::warning::warned;
use crate::serie::{DateTimeNanosecondSerie, Int32Serie, Utf8StringSerie};
use crate::{
    ArrowCastOptions, Ccy, Cfi, CodeValue, Country, DataType, Error, Eusipa, FISN_WIDTH, Field,
    Fisn, Forex, IOBase, IdKey, IdType, Identifier, Isin, Mic, Result, Scalar, Serie,
    StreamChunkedSerie, StructType, TimeUnit, Timezone,
};

pub(crate) use store::Store;

/// The name of the record a registry row is.
const ROOT: &str = "isinregistry";

/// The columns a row opens with, before one per equivalent type.
const NAMES: [&str; 12] = [
    "isin",
    "updunix",
    "lastunix",
    "cficode",
    "countrycode",
    "forexcode",
    "underlyingisin",
    "eusipacode",
    "miccode",
    "ticker",
    "fisn",
    "currency",
];

/// The place of each column of [`NAMES`] in a row.
const ISIN: usize = 0;
const UPDUNIX: usize = 1;
const LASTUNIX: usize = 2;
const CFICODE: usize = 3;
const COUNTRYCODE: usize = 4;
const FOREXCODE: usize = 5;
const UNDERLYINGISIN: usize = 6;
const EUSIPACODE: usize = 7;
const MICCODE: usize = 8;
const TICKER: usize = 9;
const FISN: usize = 10;
const CURRENCY: usize = 11;

/// The most bytes a ticker holds.
const MAX_TICKER_WIDTH: usize = 64;

/// The heap one retained value may take: the widest value any identifier
/// type admits, behind its `Arc` counters and allocator rounding.
const MAX_VALUE_HEAP_ALLOWANCE: usize =
    IDENTIFIER_VALUE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// What one listing row may take at most: its key and its listings twice
/// over for the B-tree's slack - which bounds the row inline in its
/// listings, or spilled with them to a buffer that doubles - the entry
/// holding its codes inline, a country, a pair, an underlying, a product
/// category, a short name and a currency among them - its codes spilled to
/// the heap with every value at the widest heap a value takes, its ticker's
/// heap, its short name's heap, and its one ticker slot in the ticker index
/// with that ticker's heap.
const ENTRY_CHARGE: usize = 3 * 1024;

const _: () = assert!(
    ENTRY_CHARGE
        >= 2 * (size_of::<SmolStr>() + size_of::<Listings>())
            + IsinRegistry::MAX_EQUIVALENTS
                * (size_of::<(IdType, SmolStr)>() + MAX_VALUE_HEAP_ALLOWANCE)
            + MAX_TICKER_WIDTH
            + 2 * size_of::<usize>()
            + 2 * align_of::<usize>()
            + FISN_WIDTH
            + 2 * size_of::<usize>()
            + 2 * align_of::<usize>()
            + MAX_TICKER_WIDTH
            + size_of::<(SmolStr, TickerSlots)>()
            + 2 * size_of::<usize>()
);

/// The listing rows of one ISIN, in MIC order: one row per market its
/// listing facts were stated on, or the unlisted row alone while no market
/// is known. One listing sits inline, so an instrument of one market costs
/// its row and nothing beside it.
type Listings = SmallVec<[IsinEntry; 1]>;

/// The listing rows stating one ticker, by ISIN and market, in that order.
type TickerSlots = SmallVec<[(SmolStr, Option<Mic>); 1]>;

/// The table a parse reads crosses to every worker thread as it is.
const fn crosses_threads<T: Send + Sync>() {}
const _: () = crosses_threads::<IsinTable>();

/// Every `SecurityIDSource(22)` type but the ISIN, in the code set's order:
/// one column of a row each.
fn equivalents() -> impl Iterator<Item = &'static IdType> {
    FIX_SECURITY_SOURCES
        .iter()
        .map(|(kind, _)| kind)
        .filter(|kind| **kind != IdType::Isin)
}

/// Where `kind` sorts among a row's codes: its `SecurityIDSource(22)` code,
/// whose characters order as the code set does.
fn code_order(kind: &IdType) -> Option<char> {
    kind.fix_security_source().filter(|_| *kind != IdType::Isin)
}

/// The later of two instants, an undated one the oldest.
fn later(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.max(right)
}

/// One listing of one instrument: its ISIN, its market and everything it is
/// known by there.
///
/// The row of an [`IsinRegistry`], one per ISIN and market: `isin`,
/// `updunix` - when the statement that last moved a fact of the instrument
/// happened, nanoseconds since the epoch, UTC - `lastunix` - the latest
/// instant of an event the registry learned this instrument from, the
/// same clock - the detailed `cficode`, the `countrycode` of issue where
/// one was stated, the `forexcode` an FX or referential number names, the
/// `underlyingisin` it is written on - FIX's underlying, a real ISIN other
/// than its own - its `eusipacode`, the EUSIPA product category of a
/// structured product ([`Eusipa`]), the `miccode` the row is the listing
/// of, the `ticker`, the ISO 18774 short name `fisn` the instrument states
/// ([`Fisn`]), the trading `currency` of that listing, and one code per
/// `SecurityIDSource(22)` type but the ISIN, at most
/// [`IsinRegistry::MAX_EQUIVALENTS`] of them, each held as its type stores
/// it. The instrument facts - the stamps, the CFI code, the country, the
/// pair, the underlying, the product category, the short name and every
/// code that is no listing code ([`IdType::is_listing`]) - are the ISIN's,
/// and every listing row of it holds the same; `miccode`, `ticker`,
/// `currency` and the listing codes are the row's own.
///
/// A row the registry folds - created or moved by a statement - fills,
/// once the statement has landed, the columns its own facts imply where it
/// leaves them empty: the national number its ISIN embeds
/// ([`securityid::embedded`](crate::securityid::embedded)) in its
/// equivalent column - `cusip`, `sedol`, `wkn` or `valor` - and, as its
/// `currency`, the legal tender of the country its market is in
/// ([`Mic::country`], [`Country::currency`]); a row of no market, or of a
/// market of no single country, takes no currency, since a currency is a
/// listing's. A default never displaces a statement: it fills only what the
/// statement left empty, and a later statement replaces it as it replaces
/// any value. A statement derives nothing - an entry built here and never
/// folded holds only what it was given.
///
/// ```
/// use yggdryl::{Country, IdType, Isin, IsinEntry};
///
/// # fn main() -> yggdryl::Result<()> {
/// let entry = IsinEntry::new(Isin::new("CH0012214059")?)
///     .try_with_code(IdType::Ric, "HOLN.S")?
///     .try_with_code(IdType::Bloomberg, "HOLN SW Equity")?;
/// assert_eq!(entry.get(&IdType::Ric), Some("HOLN.S"));
/// assert_eq!(entry.country(), Some(Country::new("CH")?), "the prefix, stated nowhere");
/// assert_eq!(IsinEntry::from_scalar(&entry.into_scalar())?, entry);
/// assert!(entry.clone().try_with_code(IdType::Isin, "US0378331005").is_err());
/// let own = IsinEntry::new(Isin::new("CH0012214059")?)
///     .with_underlyingisin(Some(Isin::new("CH0012214059")?));
/// assert_eq!(own.underlyingisin(), None, "an instrument is not written on itself");
/// let warrant = IsinEntry::new(Isin::new("CH0012214059")?)
///     .with_underlyingisin(Some(Isin::new("US0378331005")?));
/// assert_eq!(warrant.underlyingisin().map(Isin::as_str), Some("US0378331005"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsinEntry {
    isin: Isin,
    updunix: Option<i64>,
    lastunix: Option<i64>,
    cficode: Option<Cfi>,
    countrycode: Option<Country>,
    forexcode: Option<Forex>,
    underlyingisin: Option<Isin>,
    eusipacode: Option<Eusipa>,
    miccode: Option<Mic>,
    ticker: Option<SmolStr>,
    fisn: Option<Fisn>,
    currency: Option<Ccy>,
    codes: SmallVec<[(IdType, SmolStr); 4]>,
}

/// What a stored table partitions by: the country prefix of the ISIN every
/// row carries - Iceberg's native truncation of the key, which adds no
/// column.
const PARTITION_BY: &str = "truncate(isin, 2)";

/// The order the rows keep: the ISIN, then the market of each of its
/// listings - the table's own order. An unlisted row stands alone under its
/// ISIN, so where a null market sorts never moves a row.
const SORT_BY: [&str; 2] = ["isin", "miccode"];

/// The registry's row, declaring how a stored table partitions
/// ([`PARTITION_BY`]) and the order its rows keep ([`SORT_BY`]): what
/// [`IsinEntry::field`] answers, what a commit writes and what the snapshot
/// stream is laid out under, where the order holds by construction.
static FIELD: LazyLock<Field> = LazyLock::new(|| {
    let mut field = ROW.clone();
    field
        .as_partition_mut()
        .set_by_texts([PARTITION_BY])
        .expect("the registry's partition declaration reads");
    field
        .as_sort_mut()
        .set_by_texts(SORT_BY)
        .expect("the registry's order reads");
    field
});

/// The registry's row declaring nothing: what a load lands foreign rows
/// under, since a stream read in, a golden file's included, keeps no order
/// the landing could prove.
static ROW: LazyLock<Field> = LazyLock::new(|| {
    let instant = || DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    };
    let mut fields = vec![
        Field::new(NAMES[ISIN], DataType::isin(), false),
        Field::new(NAMES[UPDUNIX], instant(), true),
        Field::new(NAMES[LASTUNIX], instant(), true),
        Field::new(NAMES[CFICODE], DataType::cfi(), true),
        Field::new(NAMES[COUNTRYCODE], DataType::country(), true),
        Field::new(NAMES[FOREXCODE], DataType::forex(), true),
        Field::new(NAMES[UNDERLYINGISIN], DataType::isin(), true),
        // A category is four digits; `int32` is the narrowest integer every
        // store the registry binds to - an Iceberg table among them - holds.
        Field::new(NAMES[EUSIPACODE], DataType::Int32, true),
        Field::new(NAMES[MICCODE], DataType::Mic, true),
        Field::new(NAMES[TICKER], DataType::utf8(), true),
        Field::new(NAMES[FISN], DataType::fisn(), true),
        Field::new(NAMES[CURRENCY], DataType::ccy(), true),
    ];
    fields.extend(equivalents().map(|kind| Field::new(kind.as_str(), kind.value_dtype(), true)));
    Field::new(
        ROOT,
        DataType::Struct(StructType::from_unique_fields(fields)),
        false,
    )
});

impl IsinEntry {
    /// The row of `isin`, stating nothing else.
    #[must_use]
    pub fn new(isin: Isin) -> Self {
        Self {
            isin,
            updunix: None,
            lastunix: None,
            cficode: None,
            countrycode: None,
            forexcode: None,
            underlyingisin: None,
            eusipacode: None,
            miccode: None,
            ticker: None,
            fisn: None,
            currency: None,
            codes: SmallVec::new(),
        }
    }

    /// The datatype a row is: the struct [`Self::field`] holds.
    #[must_use]
    pub fn dtype() -> DataType {
        FIELD.dtype().clone()
    }

    /// The required struct `isinregistry` a row is: `isin`, `updunix`,
    /// `lastunix`, `cficode`, `countrycode`, `forexcode`, `underlyingisin`,
    /// `eusipacode` (`int32`), `miccode`, `ticker`, `fisn`, `currency`, then
    /// one column per `SecurityIDSource(22)` type but the ISIN, in the code
    /// set's order, each of its type's [`IdType::value_dtype`]: forty-four
    /// columns.
    ///
    /// The root declares how a stored table partitions and the order its
    /// rows keep: `PARTITION:by` `["truncate(isin, 2)"]`, the country prefix
    /// of the ISIN every row carries - an Iceberg table created from the
    /// field partitions by Iceberg's own truncation of the key, which
    /// stores no column, so the row stays forty-four columns and every
    /// listing of one ISIN lands in one partition, and a leaf or a plain
    /// folder, which partition by marked columns alone, are laid out flat -
    /// and `SORT:by` `["isin","miccode"]`, the order the snapshot
    /// ([`IsinRegistry::into_arrow_reader`]) streams in.
    ///
    /// ```
    /// use yggdryl::IsinEntry;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = IsinEntry::field();
    /// assert_eq!(field.field_len(), 44);
    /// assert_eq!(field.get_metadata("PARTITION:by"), Some(r#"["truncate(isin, 2)"]"#));
    /// assert_eq!(field.get_metadata("SORT:by"), Some(r#"["isin","miccode"]"#));
    /// assert_eq!(field.partition_field_names().count(), 0, "no column is marked");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn field() -> Field {
        FIELD.clone()
    }

    /// The ISIN.
    #[must_use]
    pub fn isin(&self) -> &Isin {
        &self.isin
    }

    /// Whether the row states a value in the column at `at` of
    /// [`Self::field`]: the ISIN always, any other where it is held.
    pub(crate) fn states_column(&self, at: usize) -> bool {
        match at {
            ISIN => true,
            UPDUNIX => self.updunix.is_some(),
            LASTUNIX => self.lastunix.is_some(),
            CFICODE => self.cficode.is_some(),
            COUNTRYCODE => self.countrycode.is_some(),
            FOREXCODE => self.forexcode.is_some(),
            UNDERLYINGISIN => self.underlyingisin.is_some(),
            EUSIPACODE => self.eusipacode.is_some(),
            MICCODE => self.miccode.is_some(),
            TICKER => self.ticker.is_some(),
            FISN => self.fisn.is_some(),
            CURRENCY => self.currency.is_some(),
            at => equivalents()
                .nth(at - NAMES.len())
                .is_some_and(|kind| self.codes.iter().any(|(held, _)| held == kind)),
        }
    }

    /// When the statement that last moved a fact of the instrument
    /// happened, nanoseconds since the epoch, UTC; `None` for an undated
    /// row. A stamp: it gates nothing, and a move of [`Self::lastunix`]
    /// alone leaves it.
    #[must_use]
    pub fn updunix(&self) -> Option<i64> {
        self.updunix
    }

    /// The latest instant, nanoseconds since the epoch, UTC, of an event the
    /// registry learned this instrument from - moved by every learn, whether
    /// or not the event taught anything else, so a run that only meets an
    /// instrument still records when; `None` where nothing was learned. An
    /// instrument fact, the later of two statements kept. A stamp: it gates
    /// nothing.
    #[must_use]
    pub fn lastunix(&self) -> Option<i64> {
        self.lastunix
    }

    /// The detailed CFI classification.
    #[must_use]
    pub fn cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    /// The country of issue a statement named beside the ISIN - FIX's
    /// `CountryOfIssue(470)` - held only where ISO 3166 lists it; a row
    /// the registry folded holds none where the stated country is the
    /// ISIN's own prefix, which [`Self::country`] answers already.
    #[must_use]
    pub fn countrycode(&self) -> Option<&Country> {
        self.countrycode.as_ref()
    }

    /// `stated` as a row of `isin` holds it beside the key: none where it
    /// is the ISIN's own prefix.
    fn country_beside(isin: &Isin, stated: &Country) -> Option<Country> {
        (isin.prefix() != stated.as_str()).then(|| stated.clone())
    }

    /// The country of issue: the one stated, else the ISIN's prefix where
    /// ISO 3166 lists it - an agency prefix (`XS`, `EU`, `XT`) names none.
    #[must_use]
    pub fn country(&self) -> Option<Country> {
        self.countrycode.clone().or_else(|| {
            Country::new(self.isin.prefix())
                .ok()
                .filter(Country::is_listed)
        })
    }

    /// The currency pair an FX or referential number names; none for a
    /// security.
    #[must_use]
    pub fn forexcode(&self) -> Option<&Forex> {
        self.forexcode.as_ref()
    }

    /// The ISIN of the instrument this one is written on - FIX's
    /// underlying, `UnderlyingSecurityID(309)` under an ISIN source - held
    /// only where it is real and not the row's own ISIN. An instrument
    /// fact: every listing of the ISIN holds it.
    #[must_use]
    pub fn underlyingisin(&self) -> Option<&Isin> {
        self.underlyingisin.as_ref()
    }

    /// The EUSIPA product category of the structured product the ISIN
    /// numbers - `2300` a Constant Leverage Certificate - held by its shape
    /// ([`Eusipa`]). An instrument fact: every listing of the ISIN holds
    /// it.
    #[must_use]
    pub fn eusipacode(&self) -> Option<Eusipa> {
        self.eusipacode
    }

    /// The market this row is the listing of: the one its listing facts -
    /// the ticker, the currency and every listing code
    /// ([`IdType::is_listing`]) - were stated on; none for the unlisted
    /// row, which an ISIN holds alone while no market is known.
    #[must_use]
    pub fn miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    /// The ticker of the listing.
    #[must_use]
    pub fn ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    /// The ISO 18774 Financial Instrument Short Name the instrument states -
    /// FIX's `FinancialInstrumentShortName(2737)`, `ACME CORP/SH` - an
    /// instrument fact: every listing of the ISIN holds it.
    #[must_use]
    pub fn fisn(&self) -> Option<&Fisn> {
        self.fisn.as_ref()
    }

    /// The trading currency of the listing: the one stated, else - on a
    /// row the registry folded - the legal tender of the country its market
    /// is in; none for a row of no market.
    #[must_use]
    pub fn currency(&self) -> Option<&Ccy> {
        self.currency.as_ref()
    }

    /// The code of `kind`.
    #[must_use]
    pub fn get(&self, kind: &IdType) -> Option<&str> {
        self.codes
            .iter()
            .find(|(held, _)| held == kind)
            .map(|(_, value)| value.as_str())
    }

    /// Every code, in the `SecurityIDSource(22)` code set's order.
    pub fn iter(&self) -> impl Iterator<Item = (&IdType, &str)> {
        self.codes
            .iter()
            .map(|(kind, value)| (kind, value.as_str()))
    }

    /// The row moved at `unix`.
    #[must_use]
    pub fn with_updunix(mut self, unix: Option<i64>) -> Self {
        self.updunix = unix;
        self
    }

    /// The instrument last learned from an event at `unix`.
    #[must_use]
    pub fn with_lastunix(mut self, unix: Option<i64>) -> Self {
        self.lastunix = unix;
        self
    }

    /// The row classified as `code`; a code that says nothing past its
    /// category and group is stored as none.
    #[must_use]
    pub fn with_cficode(mut self, code: Option<Cfi>) -> Self {
        self.cficode = code.filter(|code| Cfi::is_detailed(code.as_str()));
        self
    }

    /// The row's country of issue; a code ISO 3166 does not list is stored
    /// as none.
    #[must_use]
    pub fn with_countrycode(mut self, code: Option<Country>) -> Self {
        self.countrycode = code.filter(Country::is_listed);
        self
    }

    /// The row's currency pair.
    #[must_use]
    pub fn with_forexcode(mut self, code: Option<Forex>) -> Self {
        self.forexcode = code;
        self
    }

    /// The row written on `isin`; the row's own ISIN is stored as none.
    #[must_use]
    pub fn with_underlyingisin(mut self, isin: Option<Isin>) -> Self {
        self.underlyingisin = isin.filter(|underlying| *underlying != self.isin);
        self
    }

    /// The row's EUSIPA product category.
    #[must_use]
    pub fn with_eusipacode(mut self, code: Option<Eusipa>) -> Self {
        self.eusipacode = code;
        self
    }

    /// The row's listing on `code`; `XXXX`, no market, is stored as none.
    #[must_use]
    pub fn with_miccode(mut self, code: Option<Mic>) -> Self {
        self.miccode = code.filter(|code| !code.is_none());
        self
    }

    /// The row's ticker: one to sixty-four bytes once trimmed, else none.
    #[must_use]
    pub fn with_ticker(mut self, ticker: Option<SmolStr>) -> Self {
        self.ticker = ticker
            .map(|ticker| {
                let trimmed = ticker.trim();
                if trimmed.len() == ticker.len() {
                    ticker
                } else {
                    SmolStr::new(trimmed)
                }
            })
            .filter(|ticker| (1..=MAX_TICKER_WIDTH).contains(&ticker.len()));
        self
    }

    /// The row's financial instrument short name.
    #[must_use]
    pub fn with_fisn(mut self, name: Option<Fisn>) -> Self {
        self.fisn = name;
        self
    }

    /// The row's trading currency; `XXX`, no currency, is stored as none.
    #[must_use]
    pub fn with_currency(mut self, code: Option<Ccy>) -> Self {
        self.currency = code.filter(|code| !code.is_none());
        self
    }

    /// States `value` as the code of `kind`, held as the type stores it,
    /// replacing the one held.
    ///
    /// # Errors
    ///
    /// `isin`, which is the key; a type that is no `SecurityIDSource(22)`
    /// type; a value the type refuses; a new type past
    /// [`IsinRegistry::MAX_EQUIVALENTS`].
    pub fn set_code(&mut self, kind: IdType, value: &str) -> Result<()> {
        let refused = |reason: SmolStr| Error::InvalidRecord {
            path: format_smolstr!("$.{kind}"),
            reason,
        };
        if code_order(&kind).is_none() {
            return Err(refused(format_smolstr!(
                "expected one of the SecurityIDSource(22) types but the ISIN, got {kind}"
            )));
        }
        let id = Identifier::new(IdKey::base(kind), value)?;
        self.adopt_code(id.kind(), id.value())
    }

    /// Holds `value`, a code its type's datatype already proved, under
    /// `kind`, one of [`equivalents`].
    ///
    /// # Errors
    ///
    /// A new type past [`IsinRegistry::MAX_EQUIVALENTS`].
    fn adopt_code(&mut self, kind: &IdType, value: &str) -> Result<()> {
        if self.get(kind).is_none() && self.codes.len() >= IsinRegistry::MAX_EQUIVALENTS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{kind}"),
                reason: format_smolstr!(
                    "expected at most {} equivalents in a row, got one more",
                    IsinRegistry::MAX_EQUIVALENTS
                ),
            });
        }
        self.put_code(kind, value);
        Ok(())
    }

    /// [`Self::set_code`], consuming.
    ///
    /// # Errors
    ///
    /// What [`Self::set_code`] refuses.
    pub fn try_with_code(mut self, kind: IdType, value: &str) -> Result<Self> {
        self.set_code(kind, value)?;
        Ok(self)
    }

    /// Holds `value` under `kind`, a code already held as its type stores
    /// it, in the code set's order; a new type past the bound is passed over.
    fn put_code(&mut self, kind: &IdType, value: &str) {
        if let Some((_, held)) = self.codes.iter_mut().find(|(held, _)| held == kind) {
            *held = SmolStr::new(value);
            return;
        }
        if self.codes.len() < IsinRegistry::MAX_EQUIVALENTS {
            let at = self
                .codes
                .iter()
                .position(|(held, _)| code_order(held) > code_order(kind))
                .unwrap_or(self.codes.len());
            self.codes.insert(at, (kind.clone(), SmolStr::new(value)));
        }
    }

    /// This row as a statement: every code it holds that is real
    /// ([`IdType::is_real`]), a typo or a masked number dropped with one
    /// warning per column, so a row the registry holds states only real
    /// values.
    fn statement(&self) -> Statement<'_> {
        let codes = self
            .iter()
            .filter(|(kind, value)| {
                let real = kind.is_real(value);
                if !real {
                    warned!(
                        "instrument registry value dropped: it is no real code of its type",
                        kind.as_str(),
                        "{value:?} under {}",
                        self.isin.as_str()
                    );
                }
                real
            })
            .collect();
        let underlyingisin = self.underlyingisin.as_ref().filter(|underlying| {
            let real = underlying.is_real();
            if !real {
                warned!(
                    "instrument registry value dropped: it is no real code of its type",
                    NAMES[UNDERLYINGISIN],
                    "{:?} under {}",
                    underlying.as_str(),
                    self.isin.as_str()
                );
            }
            real
        });
        Statement {
            isin: self.isin.clone(),
            updunix: self.updunix,
            lastunix: self.lastunix,
            cficode: self.cficode.as_ref(),
            countrycode: self.countrycode.as_ref(),
            forexcode: self.forexcode.as_ref(),
            underlyingisin,
            eusipacode: self.eusipacode,
            miccode: self.miccode.as_ref(),
            ticker: self.ticker.as_deref(),
            fisn: self.fisn.as_ref(),
            currency: self.currency.as_ref(),
            codes,
        }
    }

    /// Fills the columns the row's own facts imply where it leaves them
    /// empty: each code its ISIN embeds ([`securityid::embedded`](crate::securityid::embedded))
    /// in its equivalent column - a type past [`IsinRegistry::MAX_EQUIVALENTS`]
    /// passed over - and the currency, the legal tender of the country its
    /// market is in; a row of no market, or of a market of no single
    /// country, takes none, since a currency is a listing's. Nothing here
    /// replaces a held value: the registry calls it once on the row a fold
    /// leaves, after the statement landed.
    fn derive_defaults(&mut self) {
        // The key is a code inline: held apart, the row stays writable.
        let isin = self.isin.clone();
        for id in crate::securityid::embedded(&isin) {
            if self.get(id.kind()).is_none() {
                // A derivation never refuses a row: the value its type
                // would refuse, or a type with no room left, moves nothing.
                let _ = self.set_code(id.kind().clone(), id.value());
            }
        }
        if self.currency.is_none() {
            self.currency = self
                .miccode
                .as_ref()
                .and_then(Mic::country)
                .and_then(|country| country.currency())
                .filter(|code| !code.is_none());
        }
    }

    /// The row a statement makes on its own: what it states, nothing
    /// derived.
    fn from_statement(statement: &Statement<'_>) -> Self {
        let countrycode = statement
            .countrycode
            .and_then(|stated| Self::country_beside(&statement.isin, stated));
        let mut entry = Self::new(statement.isin.clone())
            .with_updunix(statement.updunix)
            .with_lastunix(statement.lastunix)
            .with_cficode(statement.cficode.cloned())
            .with_countrycode(countrycode)
            .with_forexcode(statement.forexcode.cloned())
            .with_underlyingisin(statement.underlyingisin.cloned())
            .with_eusipacode(statement.eusipacode)
            .with_miccode(statement.miccode.cloned())
            .with_ticker(statement.ticker.map(SmolStr::new))
            .with_fisn(statement.fisn.cloned())
            .with_currency(statement.currency.cloned());
        for (kind, value) in &statement.codes {
            entry.put_code(kind, value);
        }
        entry
    }

    /// A new listing row of this row's instrument on `market`: the
    /// instrument facts this row holds, and no listing fact but the market.
    fn new_listing(&self, market: &Mic) -> Self {
        let mut entry = self.clone();
        entry.miccode = Some(market.clone());
        entry.ticker = None;
        entry.currency = None;
        entry.codes.retain(|(kind, _)| !kind.is_listing());
        entry
    }

    /// The row as the named struct of its cells, a fact it does not state a
    /// null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let text = |value: Option<&str>| value.map_or(Scalar::Null, Scalar::from);
        let instant = |unix: Option<i64>| {
            unix.and_then(|unix| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok())
                .unwrap_or(Scalar::Null)
        };
        let opening = [
            (NAMES[ISIN], Scalar::from(self.isin.clone())),
            (NAMES[UPDUNIX], instant(self.updunix)),
            (NAMES[LASTUNIX], instant(self.lastunix)),
            (
                NAMES[CFICODE],
                self.cficode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[COUNTRYCODE],
                self.countrycode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[FOREXCODE],
                self.forexcode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[UNDERLYINGISIN],
                self.underlyingisin
                    .clone()
                    .map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[EUSIPACODE],
                self.eusipacode
                    .map_or(Scalar::Null, |code| Scalar::from(i32::from(code.code()))),
            ),
            (
                NAMES[MICCODE],
                self.miccode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (NAMES[TICKER], text(self.ticker())),
            (
                NAMES[FISN],
                self.fisn.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[CURRENCY],
                self.currency.clone().map_or(Scalar::Null, Scalar::from),
            ),
        ];
        Scalar::from_struct(
            opening
                .into_iter()
                .map(|(name, cell)| (SmolStr::new_static(name), cell))
                .chain(
                    equivalents().map(|kind| (SmolStr::from(kind.clone()), text(self.get(kind)))),
                ),
        )
        .expect("distinct column names")
    }

    /// Reads a row back from the named struct [`Self::into_scalar`] answers,
    /// a name it lacks a null, or from the ordered row of [`Self::field`]'s
    /// cells, through that field's value door.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers - a missing
    /// or null ISIN, a cell its column's datatype refuses, a name no column
    /// has - and a row stating more than [`IsinRegistry::MAX_EQUIVALENTS`]
    /// equivalents.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let value = match value.as_struct() {
            Some(cells) => {
                let absent = FIELD
                    .dtype()
                    .as_fields()
                    .into_iter()
                    .flatten()
                    .filter(|column| !cells.contains_key(column.name()))
                    .map(|column| (SmolStr::new(column.name()), Scalar::Null));
                Scalar::from_struct(
                    cells
                        .iter()
                        .map(|(name, cell)| (name.clone(), cell.clone()))
                        .chain(absent),
                )?
            }
            None => value.clone(),
        };
        let row = FIELD.scalar(value)?;
        let unread = || Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got("the canonical isinregistry row", row.kind()),
        };
        let cells = row.sequence_rows().ok_or_else(unread)?;
        let [
            isin,
            updunix,
            lastunix,
            cficode,
            countrycode,
            forexcode,
            underlyingisin,
            eusipacode,
            miccode,
            ticker,
            fisn,
            currency,
            codes @ ..,
        ] = cells.as_ref()
        else {
            return Err(unread());
        };
        let Scalar::Isin(isin) = isin else {
            return Err(unread());
        };
        let mut entry = Self::new(isin.clone())
            .with_updunix(updunix.temporal_count_at(TimeUnit::Nanosecond))
            .with_lastunix(lastunix.temporal_count_at(TimeUnit::Nanosecond))
            .with_cficode(match cficode {
                Scalar::Cfi(code) => Some(code.clone()),
                _ => None,
            })
            .with_countrycode(match countrycode {
                Scalar::Country(code) => Some(code.clone()),
                _ => None,
            })
            .with_forexcode(match forexcode {
                Scalar::Forex(pair) => Some(pair.clone()),
                _ => None,
            })
            .with_underlyingisin(match underlyingisin {
                Scalar::Isin(code) => Some(code.clone()),
                _ => None,
            })
            .with_eusipacode(match eusipacode {
                Scalar::Int32(code) => product_category(code.get(), isin.as_str()),
                _ => None,
            })
            .with_miccode(match miccode {
                Scalar::Mic(code) => Some(code.clone()),
                _ => None,
            })
            .with_ticker(ticker.as_str().map(SmolStr::new))
            .with_fisn(match fisn {
                Scalar::Fisn(name) => Some(name.clone()),
                _ => None,
            })
            .with_currency(match currency {
                Scalar::Ccy(code) => Some(code.clone()),
                _ => None,
            });
        for (kind, cell) in equivalents().zip(codes) {
            match cell {
                Scalar::Utf8String(value) => entry.set_code(kind.clone(), value)?,
                proven => {
                    if let Some(value) = proven.as_str() {
                        entry.adopt_code(kind, value)?;
                    }
                }
            }
        }
        Ok(entry)
    }
}

/// What one statement says about one instrument, borrowed where it lies;
/// the ISIN, inline, held. Every value it carries is valid: the caller
/// dropped what was not.
struct Statement<'s> {
    isin: Isin,
    updunix: Option<i64>,
    lastunix: Option<i64>,
    cficode: Option<&'s Cfi>,
    countrycode: Option<&'s Country>,
    forexcode: Option<&'s Forex>,
    underlyingisin: Option<&'s Isin>,
    eusipacode: Option<Eusipa>,
    miccode: Option<&'s Mic>,
    ticker: Option<&'s str>,
    fisn: Option<&'s Fisn>,
    currency: Option<&'s Ccy>,
    codes: SmallVec<[(&'s IdType, &'s str); IsinRegistry::MAX_EQUIVALENTS]>,
}

impl Statement<'_> {
    /// Warns, once per column, of each listing fact it states - the ticker,
    /// the currency, a listing code - where it names no market and its ISIN
    /// has several listings: the fact belongs to one of them, and the
    /// statement does not say which, so it lands on none.
    fn warn_withheld(&self) {
        let withheld = |column: &str, value: &str| {
            warned!(
                "instrument registry listing fact dropped: the statement names no market of the instrument's several listings",
                column,
                "{value:?} under {}",
                self.isin.as_str()
            );
        };
        if let Some(ticker) = self.ticker {
            withheld(NAMES[TICKER], ticker);
        }
        if let Some(currency) = self.currency {
            withheld(NAMES[CURRENCY], currency.as_str());
        }
        for (kind, value) in self.codes.iter().filter(|(kind, _)| kind.is_listing()) {
            withheld(kind.as_str(), value);
        }
    }
}

/// The listing row of an ISIN a statement's listing facts belong to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    /// The row at this place among the ISIN's listings: the one of the
    /// statement's market, the unlisted row its market takes over, or the
    /// single row where the statement names no market.
    Row(usize),
    /// A new row, at this place: the statement names a market the ISIN has
    /// no listing on, and a listing besides.
    New(usize),
    /// None: the statement names no market and the ISIN has several
    /// listings ([`Statement::warn_withheld`]).
    Withheld,
}

impl Target {
    /// Where the listing facts of a statement naming `market` land among
    /// `listings`, an ISIN's rows in MIC order.
    fn of(listings: &[IsinEntry], market: Option<&Mic>) -> Self {
        match market {
            Some(market) => {
                match listings.binary_search_by(|row| row.miccode.as_ref().cmp(&Some(market))) {
                    Ok(at) => Self::Row(at),
                    // The unlisted row stands alone, and becomes the first
                    // listing's.
                    Err(_) if matches!(listings, [row] if row.miccode.is_none()) => Self::Row(0),
                    Err(at) => Self::New(at),
                }
            }
            None if listings.len() == 1 => Self::Row(0),
            None => Self::Withheld,
        }
    }
}

/// `row` with `statement` folded in by the update rule, or `None` where
/// nothing moves: a stated value fills a column the row lacks and replaces
/// one it holds that differs; a CFI code compatible with the held one
/// refines it and a contradicting one replaces it. The instrument facts fold
/// into every row; the listing facts - the market where the row has none,
/// the ticker, the currency and the listing codes - only where `listing`,
/// the row being the statement's [`Target`]. The stamps are the caller's,
/// and nothing is derived here: the registry derives the defaults of the
/// row this answers ([`IsinEntry::derive_defaults`]).
fn folded(row: &IsinEntry, statement: &Statement<'_>, listing: bool) -> Option<IsinEntry> {
    let mut next = Cow::Borrowed(row);
    // A code lands where its type is held or the row has room for one more;
    // past the bound a new type is passed over, and the row is owned only
    // where something lands, so a statement moving nothing answers none.
    let fill = |next: &mut Cow<'_, IsinEntry>, kind: &IdType, value: &str| {
        let lands = next.get(kind).is_some() || next.codes.len() < IsinRegistry::MAX_EQUIVALENTS;
        if lands && next.get(kind) != Some(value) {
            next.to_mut().put_code(kind, value);
        }
    };
    for (kind, value) in statement
        .codes
        .iter()
        .filter(|(kind, _)| !kind.is_listing())
    {
        fill(&mut next, kind, value);
    }
    if let Some(stated) = statement.cficode {
        match &row.cficode {
            None => next.to_mut().cficode = Some(stated.clone()),
            Some(held) if held == stated => {}
            Some(held) => match Cfi::refined(held.as_str(), stated.as_str()) {
                // Filling an `X` contradicts nothing.
                Some(refined) if refined != held.as_str() => {
                    next.to_mut().cficode = Cfi::new(refined).ok();
                }
                Some(_) => {}
                None => next.to_mut().cficode = Some(stated.clone()),
            },
        }
    }
    if let Some(stated) = statement.countrycode {
        // The ISIN's own prefix states nothing the key does not: it takes a
        // differing held country back rather than being held beside it.
        let countrycode = IsinEntry::country_beside(&statement.isin, stated);
        if row.countrycode != countrycode {
            next.to_mut().countrycode = countrycode;
        }
    }
    if let Some(stated) = statement.forexcode
        && row.forexcode.as_ref() != Some(stated)
    {
        next.to_mut().forexcode = Some(stated.clone());
    }
    if let Some(stated) = statement.underlyingisin
        && row.underlyingisin.as_ref() != Some(stated)
    {
        next.to_mut().underlyingisin = Some(stated.clone());
    }
    if let Some(stated) = statement.eusipacode
        && row.eusipacode != Some(stated)
    {
        next.to_mut().eusipacode = Some(stated);
    }
    if let Some(stated) = statement.fisn
        && row.fisn.as_ref() != Some(stated)
    {
        next.to_mut().fisn = Some(stated.clone());
    }
    if listing {
        if row.miccode.is_none()
            && let Some(stated) = statement.miccode
        {
            next.to_mut().miccode = Some(stated.clone());
        }
        for (kind, value) in statement.codes.iter().filter(|(kind, _)| kind.is_listing()) {
            fill(&mut next, kind, value);
        }
        if let Some(stated) = statement.ticker
            && row.ticker() != Some(stated)
        {
            next.to_mut().ticker = Some(SmolStr::new(stated));
        }
        if let Some(stated) = statement.currency
            && row.currency.as_ref() != Some(stated)
        {
            next.to_mut().currency = Some(stated.clone());
        }
    }
    match next {
        Cow::Borrowed(_) => None,
        Cow::Owned(entry) => Some(entry),
    }
}

/// What one learn answered.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Learned {
    /// Whether the row moved.
    pub(crate) moved: bool,
    /// The bound a new ISIN was passed over at, the first time this
    /// registry passes one over: the caller's to warn of, after it has let
    /// go of any lock it holds the registry under.
    pub(crate) full: Option<usize>,
}

/// Warns, once per site, that a registry bounded at `max` instruments is
/// full and learns no new ISIN.
pub(crate) fn warn_full(max: usize) {
    warned!(
        "instrument registry full",
        "isin_registry",
        "{max} instruments, new ISINs are not learned"
    );
}

/// The table a registry holds, shared: the listing rows by ISIN and the
/// ticker index, two counted pointers that cross threads as they are. What a
/// parse door fixes once and every worker fills from, and what a lifecycle
/// fills from under the registry's lock.
#[derive(Clone, Debug)]
pub(crate) struct IsinTable {
    /// The listing rows of each ISIN, keyed by the canonical ISIN text,
    /// never empty.
    rows: Arc<BTreeMap<SmolStr, Listings>>,
    /// Each ticker to the listing rows stating it, by ISIN and market: the
    /// exact inverse of the rows' `ticker`.
    tickers: Arc<HashMap<SmolStr, TickerSlots>>,
}

/// The table every empty registry shares, so making one allocates nothing.
static EMPTY_ROWS: LazyLock<Arc<BTreeMap<SmolStr, Listings>>> = LazyLock::new(Arc::default);
static EMPTY_TICKERS: LazyLock<Arc<HashMap<SmolStr, TickerSlots>>> = LazyLock::new(Arc::default);

impl Default for IsinTable {
    fn default() -> Self {
        Self {
            rows: Arc::clone(&EMPTY_ROWS),
            tickers: Arc::clone(&EMPTY_TICKERS),
        }
    }
}

impl IsinTable {
    /// The first listing of `isin`, in MIC order, borrowed.
    pub(crate) fn get(&self, isin: &str) -> Option<&IsinEntry> {
        self.rows.get(isin)?.first()
    }

    /// Every listing row of `isin`, in MIC order; none where it is unknown.
    pub(crate) fn listings(&self, isin: &str) -> &[IsinEntry] {
        self.rows
            .get(isin)
            .map_or(&[], |listings| listings.as_slice())
    }

    /// The listing row of `isin` on `market`, the unlisted one where
    /// `market` is none.
    fn listing(&self, isin: &str, market: Option<&Mic>) -> Option<&IsinEntry> {
        self.listings(isin)
            .iter()
            .find(|row| row.miccode.as_ref() == market)
    }

    /// The listing row the ticker `ticker` names on `market`
    /// ([`IsinRegistry::get_by_ticker`]).
    pub(crate) fn get_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Option<&IsinEntry> {
        let slots = self.tickers.get(ticker.trim())?;
        // The one slot `matching` answers, none where two do.
        let one = |matching: &dyn Fn(Option<&Mic>) -> bool| {
            let mut found = slots.iter().filter(|(_, held)| matching(held.as_ref()));
            let slot = found.next()?;
            found.next().is_none().then_some(slot)
        };
        let slot = match market.filter(|code| !code.is_none()) {
            Some(market) => {
                let exact = |held: Option<&Mic>| held == Some(market);
                if slots.iter().any(|(_, held)| exact(held.as_ref())) {
                    one(&exact)?
                } else {
                    // No listing on that market: a row listed on none.
                    one(&|held: Option<&Mic>| held.is_none())?
                }
            }
            None => one(&|_| true)?,
        };
        self.listing(&slot.0, slot.1.as_ref())
    }

    /// How many instruments it holds.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// How many listing rows it holds.
    pub(crate) fn rows(&self) -> usize {
        self.rows.values().map(SmallVec::len).sum()
    }

    /// Whether it holds none.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Every listing row, in ISIN then MIC order.
    fn iter(&self) -> impl Iterator<Item = &IsinEntry> {
        self.rows.values().flatten()
    }

    /// The listing row `element` names, whether its ISIN is derived from
    /// it, and whether its listing facts are the element's to take. By its
    /// real ISIN - stated or derived - a miss ending the fill: the row of
    /// its stated market, else the ISIN's single row, else its first, for
    /// its instrument facts alone. With none, the row its ticker names on
    /// its market ([`Self::get_by_ticker`]), whose ISIN is derived first.
    fn listing_of<E: Market + ?Sized>(&self, element: &E) -> Option<(&IsinEntry, bool, bool)> {
        let ids = element.get_securityids();
        let market = element.get_miccode().filter(|code| !code.is_none());
        match ids.get(&IdType::Isin) {
            Some(isin) if IdType::Isin.is_real(isin) => {
                let listings = self.listings(isin);
                if let Some(row) = market.and_then(|market| {
                    listings
                        .iter()
                        .find(|row| row.miccode.as_ref() == Some(market))
                }) {
                    return Some((row, false, true));
                }
                match listings {
                    [row] => Some((row, false, true)),
                    [first, ..] => Some((first, false, false)),
                    [] => None,
                }
            }
            _ => element
                .get_ticker()
                .and_then(|ticker| self.get_by_ticker(ticker, market))
                .map(|row| (row, true, true)),
        }
    }

    /// Whether `element`'s market is `row`'s: both stated and equal, or
    /// either unstated - none and `XXXX` unstated.
    fn same_market<E: Market + ?Sized>(row: &IsinEntry, element: &E) -> bool {
        match (
            element.get_miccode().filter(|code| !code.is_none()),
            &row.miccode,
        ) {
            (Some(stated), Some(held)) => stated == held,
            _ => true,
        }
    }

    /// Derives into `element` each security identifier `row` holds of a
    /// type it holds none of - the ISIN itself where `derived`, every
    /// instrument code, the listing codes only where `listed`, the pair and
    /// the short name - through [`Market::derive_securityid`]. Whether
    /// anything moved.
    fn derive_into<E: Market + ?Sized>(
        row: &IsinEntry,
        derived: bool,
        listed: bool,
        element: &mut E,
    ) -> bool {
        let mut moved = false;
        if derived {
            moved |= element.derive_securityid(&IdType::Isin, row.isin.as_str());
        }
        for (kind, value) in &row.codes {
            if (kind.is_listing() && !listed) || element.get_securityids().contains_kind(kind) {
                continue;
            }
            moved |= element.derive_securityid(kind, value);
        }
        if let Some(pair) = &row.forexcode
            && !element.get_securityids().contains_kind(&IdType::Forex)
        {
            moved |= element.derive_securityid(&IdType::Forex, pair.as_str());
        }
        if let Some(name) = &row.fisn
            && !element.get_securityids().contains_kind(&IdType::Fisn)
        {
            moved |= element.derive_securityid(&IdType::Fisn, name.as_str());
        }
        moved
    }

    /// Fills the security identifiers `element` leaves unsaid from the
    /// listing row it names ([`Self::listing_of`]): what a parse takes from
    /// the table its door fixed - derived identifiers only, which reach no
    /// field, no wire and no digest. Whether anything moved; nothing is
    /// settled.
    pub(crate) fn fill_identifiers<E: Market + ?Sized>(&self, element: &mut E) -> bool {
        let Some((row, derived, listing)) = self.listing_of(element) else {
            return false;
        };
        let listed = listing && Self::same_market(row, element);
        Self::derive_into(row, derived, listed, element)
    }

    /// [`Self::fill_identifiers`], then the market facts a lifecycle fills:
    /// the ticker on the same market where the element states none, the CFI
    /// code where it states none or the row's refines it, and the currency
    /// only where both markets are stated and equal, the ticker is the
    /// row's and the element states none. Whether anything moved; nothing
    /// is settled.
    pub(crate) fn fill_unsettled<E: Market + ?Sized>(&self, element: &mut E) -> bool {
        let Some((row, derived, listing)) = self.listing_of(element) else {
            return false;
        };
        let listed = listing && Self::same_market(row, element);
        let markets_equal = listing
            && match (
                element.get_miccode().filter(|code| !code.is_none()),
                &row.miccode,
            ) {
                (Some(stated), Some(held)) => stated == held,
                _ => false,
            };
        let mut moved = Self::derive_into(row, derived, listed, element);
        if listed
            && element.get_ticker().is_none()
            && let Some(ticker) = &row.ticker
        {
            element.set_ticker(Some(ticker.clone()), false);
            moved = true;
        }
        if let Some(learned) = &row.cficode {
            let refined = match element.get_cficode() {
                None => Some(learned.clone()),
                Some(stated) => Cfi::refined(stated.as_str(), learned.as_str())
                    .filter(|refined| refined != stated.as_str())
                    .and_then(|refined| Cfi::new(refined).ok()),
            };
            if let Some(code) = refined {
                element.set_cficode(Some(code), true);
                moved = true;
            }
        }
        if markets_equal
            && element.get_ticker().map(str::trim) == row.ticker.as_deref()
            && element.get_currency().is_none()
            && let Some(currency) = &row.currency
        {
            element.set_currency(currency.clone(), false);
            moved = true;
        }
        moved
    }
}

/// Every instrument's facts, one row per ISIN and market.
///
/// The key is a real ISIN - closing under a listed prefix
/// ([`CodeValue::is_real`]) - and an instrument holds one listing row per
/// market its listing facts were stated on, in MIC order ([`Self::listings`]),
/// or the unlisted row alone while no market is known; its instrument facts
/// are every row's ([`IsinEntry`]). A ticker leads back to its listing
/// through an exact inverse index, gated by the market; a RIC, a Bloomberg
/// symbol, a FIGI are equivalents the ISIN fills. Learning is the ordered
/// lifecycle's, or an explicit [`Self::learn`], [`Self::fill`] or
/// [`Self::enrich`]; a parse fills derived identifiers from the table its
/// door fixed and learns nothing. The table is held apart from the store
/// the registry is bound to ([`Self::from_holder`], [`Self::commit`]), and
/// a write marks the registry dirty ([`Self::is_dirty`]) until it is
/// committed.
///
/// ```
/// use yggdryl::graph::{Event, Market, OrderEvent};
/// use yggdryl::{Cfi, IdKey, IdType, Identifier, IsinRegistry, Mic};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut stated = OrderEvent::default();
/// stated.set_currunix(1);
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
/// stated.set_ticker(Some("HOLN".into()), true);
/// stated.set_miccode(Some(Mic::new("XSWX")?), true);
/// stated.set_cficode(Some(Cfi::new("ESVUFR")?), true);
/// let mut registry = IsinRegistry::new();
/// assert!(registry.learn(&stated));
/// assert!(registry.is_dirty());
/// let row = registry.get_by_ticker("HOLN", Some(&Mic::new("XSWX")?)).expect("the listing");
/// assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"), "a RIC is a listing code, never a key");
///
/// // A later statement naming only the ticker on that market is filled with the rest.
/// let mut later = OrderEvent::default();
/// later.set_ticker(Some("HOLN".into()), true);
/// later.set_miccode(Some(Mic::new("XSWX")?), true);
/// assert!(registry.fill(&mut later));
/// assert_eq!(later.get_isincode(), Some("CH0012214059"));
/// assert_eq!(later.get_securityids().get(&IdType::Ric), Some("HOLN.S"));
/// assert_eq!(later.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
///
/// // The instrument stated on another market is a second listing of it.
/// let mut london = OrderEvent::default();
/// london.set_currunix(2);
/// london.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
/// london.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.L")?)?;
/// london.set_miccode(Some(Mic::new("XLON")?), true);
/// assert!(registry.learn(&london));
/// assert_eq!((registry.len(), registry.rows()), (1, 2));
/// let rics: Vec<_> = registry.listings("CH0012214059").iter().map(|row| row.get(&IdType::Ric)).collect();
/// assert_eq!(rics, [Some("HOLN.L"), Some("HOLN.S")], "MIC order: XLON, then XSWX");
/// assert_eq!(registry.get_listing("CH0012214059", &Mic::new("XLON")?).and_then(|row| row.cficode()), Some(&Cfi::new("ESVUFR")?));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct IsinRegistry {
    table: IsinTable,
    max_instruments: usize,
    /// Whether learning past the bound was warned of already.
    warned: bool,
    /// Whether the table moved since it was loaded or committed.
    dirty: bool,
    /// The store the table is bound to, where it was.
    /// Boxed so a registry stays small enough to sit inline in a walk's
    /// own state: the store is bound once, never per walk.
    store: Option<Box<Store>>,
}

impl Default for IsinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl IsinRegistry {
    /// The instruments a registry holds unless told otherwise: at 3 KiB
    /// each at most, 48 MiB.
    pub const DEFAULT_MAX_INSTRUMENTS: usize = 16_384;

    /// The most equivalents one row holds.
    pub const MAX_EQUIVALENTS: usize = 12;

    /// An empty registry, bounded at [`Self::DEFAULT_MAX_INSTRUMENTS`],
    /// bound to no store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table: IsinTable::default(),
            max_instruments: Self::DEFAULT_MAX_INSTRUMENTS,
            warned: false,
            dirty: false,
            store: None,
        }
    }

    /// A registry holding the seed - the common instruments
    /// `config/isin/instruments.json` states, embedded at build time: each
    /// a stock, a fund or an index by its ISIN, its ticker, its market but
    /// an index's, its trading currency, its country and its detailed CFI
    /// code - clean and bound to no store, bounded at
    /// [`Self::DEFAULT_MAX_INSTRUMENTS`].
    ///
    /// A seed row is an ordinary statement: the document is read once per
    /// process through the column rule of [`Self::extend_from_arrow_reader`]
    /// and every row folded by [`Self::merge`], so the facts a row implies -
    /// the national number its ISIN embeds, the currency of its market's
    /// country - are derived as for any other. Making one shares that table: no row
    /// is copied until one moves. [`Self::seeded_from_url`] lays a store the
    /// caller names over it and [`Self::from_env`] the one the environment
    /// names; [`Self::new`] holds none of it.
    ///
    /// ```
    /// use yggdryl::{IdType, IsinRegistry, Mic};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let registry = IsinRegistry::seeded();
    /// assert!(!registry.is_dirty() && registry.holder().is_none());
    /// let apple = registry.get_by_ticker("AAPL", Some(&Mic::new("XNAS")?)).expect("seeded");
    /// assert_eq!(apple.isin().as_str(), "US0378331005");
    /// assert_eq!(apple.get(&IdType::Cusip), Some("037833100"), "the CUSIP its ISIN embeds");
    /// assert!(IsinRegistry::new().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn seeded() -> Self {
        Self {
            table: seed::table().clone(),
            ..Self::new()
        }
    }

    /// The registry bounded at `max` instruments.
    #[must_use]
    pub fn with_max_instruments(mut self, max: usize) -> Self {
        self.max_instruments = max;
        self
    }

    /// The most instruments it holds.
    #[must_use]
    pub fn max_instruments(&self) -> usize {
        self.max_instruments
    }

    /// How many instruments it holds: its ISINs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// How many listing rows it holds: one per ISIN and market, an
    /// unlisted row one - what [`Self::iter`] walks and a commit writes.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.table.rows()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Whether the table moved since it was loaded or committed: a learn,
    /// a merge, a removal or a clear that moved something sets it - a fold
    /// of a stream through `extend_from_*` included - and only a load
    /// ([`Self::set_holder`], [`Self::from_arrow_reader`]) or a
    /// [`Self::commit`] clears it.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The table as it stands, shared: what a parse door fixes once.
    pub(crate) fn as_table(&self) -> &IsinTable {
        &self.table
    }

    /// The first listing row of `isin`, in MIC order - the unlisted row
    /// where that is all it holds - borrowed. Its instrument facts are every
    /// listing's; [`Self::listings`] answers them all.
    #[must_use]
    pub fn get(&self, isin: &str) -> Option<&IsinEntry> {
        self.table.get(isin)
    }

    /// Every listing row of `isin`, in MIC order, borrowed; empty where the
    /// ISIN is unknown.
    #[must_use]
    pub fn listings(&self, isin: &str) -> &[IsinEntry] {
        self.table.listings(isin)
    }

    /// The listing row of `isin` on `market`, borrowed.
    #[must_use]
    pub fn get_listing(&self, isin: &str, market: &Mic) -> Option<&IsinEntry> {
        self.table.listing(isin, Some(market))
    }

    /// The listing row the ticker `ticker` - trimmed of blanks, as a learn
    /// stores it - names on `market`, through the ticker index: the one row
    /// listing the ticker on `market`, else - no row listing it there - the
    /// one listing it on no market; where `market` is unstated - none and
    /// `XXXX` - the one row listing it on any. Two rows answering is
    /// ambiguous, and answers none: one ticker on two markets names each
    /// listing on its own, and neither where no market is stated.
    ///
    /// ```
    /// use yggdryl::graph::{Market, OrderEvent};
    /// use yggdryl::{IdKey, IdType, Identifier, IsinRegistry, Mic};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut stated = OrderEvent::default();
    /// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    /// stated.set_ticker(Some("HOLN".into()), true);
    /// stated.set_miccode(Some(Mic::new("XSWX")?), true);
    /// let mut registry = IsinRegistry::new();
    /// assert!(registry.learn(&stated));
    /// let isin = |market: Option<&Mic>| {
    ///     registry.get_by_ticker("HOLN", market).map(|row| row.isin().as_str())
    /// };
    /// assert_eq!(isin(None), Some("CH0012214059"));
    /// assert_eq!(isin(Some(&Mic::new("XSWX")?)), Some("CH0012214059"));
    /// assert_eq!(isin(Some(&Mic::new("XLON")?)), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn get_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Option<&IsinEntry> {
        self.table.get_by_ticker(ticker, market)
    }

    /// Every listing row, in ISIN then MIC order.
    pub fn iter(&self) -> impl Iterator<Item = &IsinEntry> {
        self.table.iter()
    }

    /// Folds `entry` into the listings of its ISIN by the update rule: a
    /// stated value fills a column a row lacks and replaces one it holds
    /// that differs, whatever the time; a code that is no real value of its
    /// type is dropped with one warning per column; a CFI code compatible
    /// with the held one refines it and a contradicting one replaces it.
    /// The instrument facts fold into every listing row of the ISIN; the
    /// listing facts - the ticker, the currency, the listing codes - into
    /// the row of the market the entry names, created where the ISIN has
    /// none there, the unlisted row taken over where that is all it holds;
    /// and, where the entry names no market, into the ISIN's single row, or
    /// into none, with one warning per column, where it has several.
    /// `updunix` becomes the later of the two on every row where a fact
    /// moved, and `lastunix` the later of the two whatever moved. A row
    /// created or moved then fills what its own facts imply where the
    /// statement left it empty - the national number its ISIN embeds, the
    /// currency of its market's country ([`IsinEntry`]). Whether anything
    /// moved.
    ///
    /// # Errors
    ///
    /// An ISIN that is not real ([`CodeValue::is_real`]) - a `ZZ` number,
    /// which names no country's instrument, a masked one, a typo - and a
    /// new ISIN past [`Self::max_instruments`].
    pub fn merge(&mut self, entry: IsinEntry) -> Result<bool> {
        if !entry.isin.is_real() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.isin"),
                reason: format_smolstr!(
                    "expected an ISIN some agency numbers, got {}",
                    entry.isin.as_str()
                ),
            });
        }
        self.fold(&entry.statement())
    }

    /// Removes every listing row of `isin`, answering them in MIC order;
    /// none where the ISIN is unknown.
    pub fn remove(&mut self, isin: &str) -> Vec<IsinEntry> {
        if !self.table.rows.contains_key(isin) {
            return Vec::new();
        }
        let Some(removed) = Arc::make_mut(&mut self.table.rows).remove(isin) else {
            return Vec::new();
        };
        let tickers = Arc::make_mut(&mut self.table.tickers);
        for row in &removed {
            if let Some(ticker) = row.ticker() {
                unlist_ticker(tickers, ticker, isin, row.miccode.as_ref());
            }
        }
        self.dirty = true;
        removed.into_vec()
    }

    /// Removes the listing row of `isin` on `market`, answering it; the
    /// instrument goes with its last listing.
    pub fn remove_listing(&mut self, isin: &str, market: &Mic) -> Option<IsinEntry> {
        let at = self
            .table
            .listings(isin)
            .iter()
            .position(|row| row.miccode.as_ref() == Some(market))?;
        let rows = Arc::make_mut(&mut self.table.rows);
        let listings = rows.get_mut(isin)?;
        let removed = listings.remove(at);
        if listings.is_empty() {
            rows.remove(isin);
        }
        if let Some(ticker) = removed.ticker() {
            unlist_ticker(
                Arc::make_mut(&mut self.table.tickers),
                ticker,
                isin,
                Some(market),
            );
        }
        self.dirty = true;
        Some(removed)
    }

    /// Removes every row.
    pub fn clear(&mut self) {
        if !self.table.is_empty() {
            self.dirty = true;
        }
        self.table = IsinTable::default();
    }

    /// Folds one statement into the listings of its ISIN ([`Self::merge`]),
    /// then derives the defaults of each row it moved - once, after the
    /// statement landed, so a default only fills a column the statement left
    /// empty and never travels as a statement itself. Nothing is built, and
    /// the table is not copied, where nothing moves.
    fn fold(&mut self, statement: &Statement<'_>) -> Result<bool> {
        let isin = statement.isin.as_str();
        let Some(listings) = self.table.rows.get(isin) else {
            if self.table.rows.len() >= self.max_instruments {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$['{isin}']"),
                    reason: format_smolstr!(
                        "expected at most {} instruments, got one more",
                        self.max_instruments
                    ),
                });
            }
            let mut entry = IsinEntry::from_statement(statement);
            entry.derive_defaults();
            let key = SmolStr::new(isin);
            if let Some(ticker) = &entry.ticker {
                list_ticker(
                    Arc::make_mut(&mut self.table.tickers),
                    ticker,
                    &key,
                    entry.miccode.as_ref(),
                );
            }
            Arc::make_mut(&mut self.table.rows).insert(key, Listings::from_buf([entry]));
            self.dirty = true;
            return Ok(true);
        };
        let target = Target::of(listings, statement.miccode);
        if target == Target::Withheld {
            statement.warn_withheld();
        }
        let held_updunix = listings.iter().filter_map(|row| row.updunix).max();
        let lastunix = later(
            listings.iter().filter_map(|row| row.lastunix).max(),
            statement.lastunix,
        );
        let key = SmolStr::new(isin);
        let mut facts = false;
        for at in 0..listings.len() {
            let row = &self.table.listings(isin)[at];
            let Some(mut next) = folded(row, statement, target == Target::Row(at)) else {
                continue;
            };
            next.derive_defaults();
            self.put_listing(&key, at, next);
            facts = true;
        }
        if let (Target::New(at), Some(market)) = (target, statement.miccode) {
            let mut entry = self.table.listings(isin)[0].new_listing(market);
            if let Some(next) = folded(&entry, statement, true) {
                entry = next;
            }
            entry.derive_defaults();
            let rows = Arc::make_mut(&mut self.table.rows);
            let listings = rows.get_mut(isin).expect("a held ISIN");
            listings.insert(at, entry);
            let row = &listings[at];
            if let Some(ticker) = &row.ticker {
                list_ticker(
                    Arc::make_mut(&mut self.table.tickers),
                    ticker,
                    &key,
                    row.miccode.as_ref(),
                );
            }
            facts = true;
        }
        // The stamps are the instrument's: every listing row holds the same.
        let updunix = if facts {
            later(held_updunix, statement.updunix)
        } else {
            held_updunix
        };
        let stamps =
            |row: &IsinEntry| (!facts || row.updunix == updunix) && row.lastunix == lastunix;
        let stamped = !self.table.listings(isin).iter().all(stamps);
        if stamped {
            let rows = Arc::make_mut(&mut self.table.rows);
            for row in rows.get_mut(isin).expect("a held ISIN") {
                if facts {
                    row.updunix = updunix;
                }
                row.lastunix = lastunix;
            }
        }
        let moved = facts || stamped;
        self.dirty |= moved;
        Ok(moved)
    }

    /// Puts `next` at `at` among the listings of `isin`, keeping the
    /// ticker index on the rows' tickers and markets: one slot per listing
    /// row stating a ticker.
    fn put_listing(&mut self, isin: &SmolStr, at: usize, next: IsinEntry) {
        let rows = Arc::make_mut(&mut self.table.rows);
        let row = &mut rows.get_mut(isin.as_str()).expect("a held ISIN")[at];
        let held = std::mem::replace(row, next);
        if (held.ticker.as_ref(), held.miccode.as_ref())
            == (row.ticker.as_ref(), row.miccode.as_ref())
        {
            return;
        }
        let tickers = Arc::make_mut(&mut self.table.tickers);
        if let Some(ticker) = &held.ticker {
            unlist_ticker(tickers, ticker, isin, held.miccode.as_ref());
        }
        if let Some(ticker) = &row.ticker {
            list_ticker(tickers, ticker, isin, row.miccode.as_ref());
        }
    }

    /// Learns what `event` states about its instrument: keyed by its stated
    /// ISIN (a real one, closing under a listed prefix, never a derivation),
    /// its market but `XXXX` naming the listing row its listing facts land
    /// on ([`Self::merge`]), and dated at its `currunix` - its `lastunix`
    /// on every learn, so an event that teaches nothing else still records
    /// when the instrument was last met; reading its detailed CFI code, its
    /// ticker, its currency but `XXX` (unless it holds a currency pair,
    /// where `Currency(15)` is the dealt currency and not the listing's),
    /// the pair it states as a `forex` identifier, the short name it states
    /// as a `fisn` identifier, and each equivalent type its map answers with
    /// a real value ([`IdType::is_real`]), never one it only derived, a
    /// masked number or a typo. A new ISIN past [`Self::max_instruments`] is
    /// not learned, with one warning. Whether anything moved. What a FIX message names
    /// as its underlying and its EUSIPA product category is learned only by
    /// the lifecycle, beside the message
    /// ([`FixCodec::lifecycle`](crate::FixCodec::lifecycle)).
    pub fn learn<E: Market + Event + ?Sized>(&mut self, event: &E) -> bool {
        let learned = self.learn_stating(event, None, None, None);
        if let Some(max) = learned.full {
            warn_full(max);
        }
        learned.moved
    }

    /// [`Self::learn`] beside the country of issue, the underlying and the
    /// EUSIPA product category `event` stated - an underlying that is no
    /// real ISIN or is the event's own ISIN states nothing - and what it
    /// answers in full:
    /// whether a listing row moved, and the bound a new
    /// ISIN was passed over at the first time one is, which the caller
    /// warns of itself ([`warn_full`]) - after it has let go of any lock it
    /// holds the registry under, since a warning reaches a host that may
    /// be waiting on that lock. Nothing here logs.
    pub(crate) fn learn_stating<E: Market + Event + ?Sized>(
        &mut self,
        event: &E,
        country: Option<&Country>,
        underlying: Option<&Isin>,
        product: Option<Eusipa>,
    ) -> Learned {
        let ids = event.get_securityids();
        let stated = |kind: &IdType| ids.get(kind).filter(|_| !ids.is_derived(kind));
        let Some(isin) = stated(&IdType::Isin).filter(|isin| IdType::Isin.is_real(isin)) else {
            return Learned::default();
        };
        let forexcode = stated(&IdType::Forex).and_then(|pair| Forex::new(pair).ok());
        let fisn = stated(&IdType::Fisn)
            .filter(|name| IdType::Fisn.is_real(name))
            .and_then(|name| Fisn::new(name).ok());
        let currency = event.get_currency();
        let statement = Statement {
            isin: Isin::from_proven(isin),
            updunix: Some(event.get_currunix()),
            lastunix: Some(event.get_currunix()),
            cficode: event
                .get_cficode()
                .filter(|code| Cfi::is_detailed(code.as_str())),
            countrycode: country.filter(|code| code.is_listed()),
            forexcode: forexcode.as_ref(),
            underlyingisin: underlying
                .filter(|underlying| underlying.as_str() != isin && underlying.is_real()),
            eusipacode: product,
            miccode: event.get_miccode().filter(|code| !code.is_none()),
            ticker: event
                .get_ticker()
                .map(str::trim)
                .filter(|ticker| (1..=MAX_TICKER_WIDTH).contains(&ticker.len())),
            fisn: fisn.as_ref(),
            currency: Some(currency)
                .filter(|code| !code.is_none() && !ids.contains_kind(&IdType::Forex)),
            codes: equivalents()
                .filter_map(|kind| {
                    stated(kind)
                        .filter(|value| kind.is_real(value))
                        .map(|value| (kind, value))
                })
                .take(Self::MAX_EQUIVALENTS)
                .collect(),
        };
        if !self.table.rows.contains_key(statement.isin.as_str())
            && self.table.rows.len() >= self.max_instruments
        {
            let full = (!self.warned).then_some(self.max_instruments);
            self.warned = true;
            return Learned { moved: false, full };
        }
        Learned {
            moved: self.fold(&statement).unwrap_or(false),
            full: None,
        }
    }

    /// Fills what `element` leaves unsaid about its instrument from the
    /// listing row it names: by its real ISIN - stated or derived, a miss
    /// ending the fill - the row of its market, else the ISIN's single row,
    /// else - its market unstated or listing none of several - the first,
    /// for the instrument's facts alone; with none, the row its ticker names
    /// on its market ([`Self::get_by_ticker`]), whose ISIN is derived first -
    /// over none, or over a number ranking below it, a masked one or a
    /// typo. It takes each equivalent of a type it holds none of as a
    /// derived identifier, the listing codes only where its market - none
    /// and `XXXX` unstated - is the row's or either is unstated, the pair,
    /// the short name, the ticker on the same market, its CFI code where it
    /// states none or the row's refines it, and the currency only where both
    /// markets are stated and equal, the ticker is the row's and it states
    /// none. The element is finalized where anything moved, and nothing is
    /// built where nothing is filled.
    /// Whether anything moved.
    pub fn fill<E: Market + Element + ?Sized>(&self, element: &mut E) -> bool {
        let moved = self.table.fill_unsettled(element);
        if moved {
            element.finalize();
        }
        moved
    }

    /// [`Self::learn`] what `event` states, then [`Self::fill`] what it
    /// leaves unsaid. Whether anything moved in either.
    pub fn enrich<E: Market + Event + ?Sized>(&mut self, event: &mut E) -> bool {
        let learned = self.learn(event);
        self.fill(event) || learned
    }

    /// A registry read from `reader`'s rows ([`Self::extend_from_arrow_reader`]),
    /// bound to no store and clean: what was read is what it holds.
    ///
    /// # Errors
    ///
    /// What [`Self::extend_from_arrow_reader`] refuses.
    pub fn from_arrow_reader(reader: BatchReader) -> crate::arrow::Result<Self> {
        let mut registry = Self::new();
        registry.extend_from_arrow_reader(reader)?;
        registry.dirty = false;
        Ok(registry)
    }

    /// Folds `reader`'s rows in, each through [`Self::merge`] as a
    /// statement, so rows of one ISIN fold in the order they are read and
    /// its rows on several markets load as several listings; answers how
    /// many rows it read.
    ///
    /// Each column of the reader's schema is read as the registry column it
    /// names, resolved once: its canonical name, a spelling or alias of an
    /// [`IdType`] - `RIC`, `riccode`, `BloombergSymbol`, `ISINCode`,
    /// `ccypair` - a field name a security type is read from (`#ISINCODE`,
    /// `cusip_code`), or `cfi`, `country`, `mic`, `symbol`, `ccy` for the
    /// CFI code, the country, the market, the ticker and the currency, or
    /// `lastseen` and `lastseenunix` for `lastunix`, or
    /// `underlyingisin` and any key naming an underlying's ISIN - the
    /// identifier name past `underlying`, as `UnderlyingISIN` is - or
    /// `eusipa`, `eusipacode`, `eusipacategory`, `sspa`, `sspacode` or
    /// `sspacategory` for the product category, Euronext's `EUSIPA_Code`
    /// among them, its cells numbers or text of the number, or `fisn`,
    /// `fisncode`, `shortname` or `financialinstrumentshortname` for the
    /// short name; any other column is passed over. The stream is cast once, into
    /// [`IsinEntry::field`], a column it lacks null and a value a column
    /// cannot hold null.
    ///
    /// # Errors
    ///
    /// No `isin` column in a stream of any column, two columns naming one, a
    /// row a column refuses -
    /// located `$[row].column` - a row past [`Self::MAX_EQUIVALENTS`] or a
    /// new ISIN past [`Self::max_instruments`].
    pub fn extend_from_arrow_reader(&mut self, reader: BatchReader) -> crate::arrow::Result<usize> {
        let schema = reader.schema();
        // A stream of no columns - what a missing store reads as - holds no
        // row to key.
        if schema.fields().is_empty() {
            return Ok(0);
        }
        let mut resolved: Vec<(usize, usize)> = Vec::new();
        for (at, column) in schema.fields().iter().enumerate() {
            let Some(target) = registry_column(column.name()) else {
                continue;
            };
            if let Some((first, _)) = resolved.iter().find(|(_, held)| *held == target) {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!(
                        "expected one column naming {}, got {:?} and {:?}",
                        column_name(target),
                        schema.field(*first).name(),
                        column.name()
                    ),
                }
                .into());
            }
            resolved.push((at, target));
        }
        if !resolved.iter().any(|(_, target)| *target == ISIN) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.isin"),
                reason: format_smolstr!(
                    "expected a column naming the ISIN, got {:?}",
                    schema
                        .fields()
                        .iter()
                        .map(|column| column.name().as_str())
                        .collect::<Vec<_>>()
                ),
            }
            .into());
        }
        // The columns read, renamed to the registry's own names; their
        // buffers are shared, never copied.
        let renamed = Arc::new(Schema::new(
            resolved
                .iter()
                .map(|(at, target)| {
                    let column = schema.field(*at);
                    ArrowField::new(
                        column_name(*target),
                        column.data_type().clone(),
                        column.is_nullable(),
                    )
                    .with_metadata(column.metadata().clone())
                })
                .collect::<Vec<_>>(),
        ));
        let picked: Vec<usize> = resolved.iter().map(|(at, _)| *at).collect();
        let projected = Arc::clone(&renamed);
        let batches = reader.map(move |batch| {
            let batch = batch?;
            RecordBatch::try_new(
                Arc::clone(&projected),
                picked
                    .iter()
                    .map(|at| Arc::clone(batch.column(*at)))
                    .collect(),
            )
        });
        let records = StreamChunkedSerie::from_arrow_reader(
            Some(&*ROW),
            Box::new(RecordBatchIterator::new(batches, renamed)),
            ArrowCastOptions::default(),
        )?;
        let mut read = 0;
        for record in records.into_chunks() {
            let record = record?;
            let columns = Columns::of(&record)?;
            for row in 0..record.len() {
                let located = |error: Error| match error {
                    Error::InvalidRecord { path, reason } => Error::InvalidRecord {
                        path: format_smolstr!(
                            "$[{read}]{}",
                            path.strip_prefix('$').unwrap_or(&path)
                        ),
                        reason,
                    },
                    other => Error::InvalidRecord {
                        path: format_smolstr!("$[{read}]"),
                        reason: format_smolstr!("{other}"),
                    },
                };
                let entry = columns.entry(row).map_err(located)?;
                self.merge(entry).map_err(located)?;
                read += 1;
            }
        }
        Ok(read)
    }

    /// Folds the rows `handle` holds in: the handle's own record stream -
    /// an Arrow IPC file, Parquet, a folder of either, an object store - read
    /// through [`Self::extend_from_arrow_reader`] under the handle's own
    /// options; a missing store reads as the empty stream. Answers how many
    /// rows it read. The registry stays bound to the store it was, if any:
    /// [`Self::set_holder`] binds.
    ///
    /// # Errors
    ///
    /// What the handle's read or [`Self::extend_from_arrow_reader`] refuses.
    pub fn extend_from_handle(&mut self, handle: &dyn IOBase) -> Result<usize> {
        let reader = handle.read_arrow_reader(&handle.record_options()?)?;
        Ok(self.extend_from_arrow_reader(reader)?)
    }

    /// The listing rows as a stream under [`IsinEntry::field`], in ISIN then
    /// MIC order: a snapshot of the table as it stands, laid out one bounded
    /// batch at a time, which a learn while it streams does not move.
    ///
    /// # Errors
    ///
    /// What laying the rows out refuses, which no registry value causes.
    pub fn into_arrow_reader(&self) -> crate::arrow::Result<BatchReader> {
        crate::arrow::rows::reader(&FIELD, Snapshot::of(&self.table), None, None, None)
    }
}

/// Drops the listing row of `isin` on `market` from the slots of `ticker`
/// in the ticker index, and the ticker with it once no row states it.
fn unlist_ticker(
    tickers: &mut HashMap<SmolStr, TickerSlots>,
    ticker: &str,
    isin: &str,
    market: Option<&Mic>,
) {
    if let Some(listed) = tickers.get_mut(ticker) {
        listed.retain(|(held, mic)| held.as_str() != isin || mic.as_ref() != market);
        if listed.is_empty() {
            tickers.remove(ticker);
        }
    }
}

/// Adds the listing row of `isin` on `market` to the slots of `ticker` in
/// the ticker index, in ISIN then MIC order.
fn list_ticker(
    tickers: &mut HashMap<SmolStr, TickerSlots>,
    ticker: &SmolStr,
    isin: &SmolStr,
    market: Option<&Mic>,
) {
    let slot = (isin.clone(), market.cloned());
    let listed = tickers.entry(ticker.clone()).or_default();
    if let Err(at) = listed.binary_search(&slot) {
        listed.insert(at, slot);
    }
}

/// The listing rows of one snapshot, in ISIN then MIC order, each as its
/// scalar.
struct Snapshot {
    rows: Arc<BTreeMap<SmolStr, Listings>>,
    /// The ISIN of the row last answered, and its place among the ISIN's
    /// listings.
    after: Option<(SmolStr, usize)>,
}

impl Snapshot {
    /// The rows `table` holds, from the first.
    fn of(table: &IsinTable) -> Self {
        Self {
            rows: Arc::clone(&table.rows),
            after: None,
        }
    }
}

impl Iterator for Snapshot {
    type Item = Scalar;

    fn next(&mut self) -> Option<Scalar> {
        let (isin, at) = match &self.after {
            Some((isin, at))
                if self
                    .rows
                    .get(isin)
                    .is_some_and(|listings| at + 1 < listings.len()) =>
            {
                (isin.clone(), at + 1)
            }
            Some((isin, _)) => {
                let (next, _) = self
                    .rows
                    .range::<str, _>((Bound::Excluded(isin.as_str()), Bound::Unbounded))
                    .next()?;
                (next.clone(), 0)
            }
            None => (self.rows.keys().next()?.clone(), 0),
        };
        let row = self.rows.get(&isin)?.get(at)?.into_scalar();
        self.after = Some((isin, at));
        Some(row)
    }
}

/// The name of the registry column at `at` in [`IsinEntry::field`].
fn column_name(at: usize) -> &'static str {
    NAMES
        .get(at)
        .copied()
        .or_else(|| equivalents().nth(at - NAMES.len()).map(IdType::as_str))
        .expect("a registry column")
}

/// The registry column a source column named `name` is read as, by its
/// place in [`IsinEntry::field`]: its canonical name, an [`IdType`]
/// spelling or alias, a field name a security type is read from, or one of
/// the market spellings; `None` for any other.
fn registry_column(name: &str) -> Option<usize> {
    /// The spellings of the columns no identifier type names.
    const MARKET: [(&str, usize); 28] = [
        ("isin", ISIN),
        ("updunix", UPDUNIX),
        ("lastunix", LASTUNIX),
        ("lastseen", LASTUNIX),
        ("lastseenunix", LASTUNIX),
        ("cfi", CFICODE),
        ("cficode", CFICODE),
        ("country", COUNTRYCODE),
        ("countrycode", COUNTRYCODE),
        ("countryofissue", COUNTRYCODE),
        ("eusipa", EUSIPACODE),
        ("eusipacode", EUSIPACODE),
        ("eusipacategory", EUSIPACODE),
        ("sspa", EUSIPACODE),
        ("sspacode", EUSIPACODE),
        ("sspacategory", EUSIPACODE),
        ("mic", MICCODE),
        ("miccode", MICCODE),
        ("ticker", TICKER),
        ("symbol", TICKER),
        ("tickersymbol", TICKER),
        ("fisn", FISN),
        ("fisncode", FISN),
        ("shortname", FISN),
        ("financialinstrumentshortname", FISN),
        ("ccy", CURRENCY),
        ("currency", CURRENCY),
        ("currencycode", CURRENCY),
    ];
    let mut buffer = [0_u8; IDENTIFIER_WORD_WIDTH];
    let folded = fold_into(name, &mut buffer).ok()?;
    if let Some((_, at)) = MARKET.iter().find(|(spelled, _)| *spelled == folded) {
        return Some(*at);
    }
    if IdType::underlying_security(folded) == Some(IdType::Isin) {
        return Some(UNDERLYINGISIN);
    }
    let kind = folded
        .parse::<IdType>()
        .ok()
        .filter(IdType::is_known)
        .or_else(|| IdType::from_field_name(name))?;
    match kind {
        IdType::Isin => Some(ISIN),
        IdType::Cfi => Some(CFICODE),
        IdType::Forex => Some(FOREXCODE),
        IdType::Fisn => Some(FISN),
        kind => equivalents()
            .position(|held| *held == kind)
            .map(|at| at + NAMES.len()),
    }
}

/// One landed record's columns, each narrowed to its storage once.
struct Columns {
    isin: Arc<Utf8StringSerie>,
    updunix: Arc<DateTimeNanosecondSerie>,
    lastunix: Arc<DateTimeNanosecondSerie>,
    cficode: Arc<Utf8StringSerie>,
    countrycode: Arc<Utf8StringSerie>,
    forexcode: Arc<Utf8StringSerie>,
    underlyingisin: Arc<Utf8StringSerie>,
    eusipacode: Arc<Int32Serie>,
    miccode: Arc<Utf8StringSerie>,
    ticker: Arc<Utf8StringSerie>,
    fisn: Arc<Utf8StringSerie>,
    currency: Arc<Utf8StringSerie>,
    /// Each equivalent's column, and whether it is its type's own code
    /// column, whose cells the landing proved.
    codes: Vec<(&'static IdType, Arc<Utf8StringSerie>, bool)>,
}

impl Columns {
    fn of(record: &Serie) -> Result<Self> {
        let children = record.children();
        let unlanded = |at: usize| Error::InvalidRecord {
            path: format_smolstr!("$.{}", column_name(at)),
            reason: format_smolstr!("expected the column {}'s storage", column_name(at)),
        };
        // A column's text storage, and whether a registered code's own
        // column holds it.
        let text = |at: usize| match children.get(at) {
            Some(Serie::Utf8String(held)) => Ok((Arc::clone(held), false)),
            Some(
                Serie::Isin(held)
                | Serie::Cfi(held)
                | Serie::Mic(held)
                | Serie::Cusip(held)
                | Serie::Sedol(held)
                | Serie::Figi(held)
                | Serie::Ric(held)
                | Serie::Bbg(held)
                | Serie::Ccy(held)
                | Serie::Country(held)
                | Serie::Forex(held)
                | Serie::Fisn(held)
                | Serie::Lei(held)
                | Serie::Dti(held),
            ) => Ok((Arc::clone(held), true)),
            _ => Err(unlanded(at)),
        };
        let code = |at: usize| match text(at)? {
            (held, true) => Ok(held),
            (_, false) => Err(unlanded(at)),
        };
        let instant = |at: usize| match children.get(at) {
            Some(Serie::DateTimeNanosecond(held)) => Ok(Arc::clone(held)),
            _ => Err(unlanded(at)),
        };
        Ok(Self {
            isin: code(ISIN)?,
            updunix: instant(UPDUNIX)?,
            lastunix: instant(LASTUNIX)?,
            cficode: code(CFICODE)?,
            countrycode: code(COUNTRYCODE)?,
            forexcode: code(FOREXCODE)?,
            underlyingisin: code(UNDERLYINGISIN)?,
            eusipacode: match children.get(EUSIPACODE) {
                Some(Serie::Int32(held)) => Arc::clone(held),
                _ => return Err(unlanded(EUSIPACODE)),
            },
            miccode: code(MICCODE)?,
            ticker: match text(TICKER)? {
                (held, false) => held,
                (_, true) => return Err(unlanded(TICKER)),
            },
            fisn: code(FISN)?,
            currency: code(CURRENCY)?,
            codes: equivalents()
                .enumerate()
                .map(|(at, kind)| {
                    let (held, proven) = text(at + NAMES.len())?;
                    Ok((kind, held, proven))
                })
                .collect::<Result<_>>()?,
        })
    }

    /// Row `row` as an entry: a code column's cell adopted as the landing
    /// proved it, a `utf8` one through its type's rule.
    fn entry(&self, row: usize) -> Result<IsinEntry> {
        let isin = self.isin.value(row).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.isin"),
            reason: SmolStr::new_static("expected an ISIN, got null"),
        })?;
        let mut entry = IsinEntry::new(Isin::from_proven(isin))
            .with_updunix(self.updunix.value(row))
            .with_lastunix(self.lastunix.value(row))
            .with_cficode(self.cficode.value(row).map(Cfi::from_proven))
            .with_countrycode(
                self.countrycode
                    .value(row)
                    .and_then(|code| Country::new(code).ok()),
            )
            .with_forexcode(
                self.forexcode
                    .value(row)
                    .and_then(|pair| Forex::new(pair).ok()),
            )
            .with_underlyingisin(self.underlyingisin.value(row).map(Isin::from_proven))
            .with_eusipacode(
                self.eusipacode
                    .value(row)
                    .and_then(|code| product_category(code, isin)),
            )
            .with_miccode(self.miccode.value(row).map(Mic::from_proven))
            .with_ticker(self.ticker.value(row).map(SmolStr::new))
            .with_fisn(self.fisn.value(row).and_then(|name| Fisn::new(name).ok()))
            .with_currency(
                self.currency
                    .value(row)
                    .and_then(|code| Ccy::new(code).ok()),
            );
        for (kind, column, proven) in &self.codes {
            if let Some(value) = column.value(row) {
                if *proven {
                    entry.adopt_code(kind, value)
                } else {
                    entry.set_code((*kind).clone(), value)
                }
                .map_err(|error| relocated(kind.as_str(), error))?;
            }
        }
        Ok(entry)
    }
}

/// The product category a row of `isin` states as `code`, a number of no
/// category's shape dropped with one warning for the column.
fn product_category(code: i32, isin: &str) -> Option<Eusipa> {
    let category = u16::try_from(code)
        .ok()
        .and_then(|code| Eusipa::new(code).ok());
    if category.is_none() {
        warned!(
            "instrument registry value dropped: it is no real code of its type",
            NAMES[EUSIPACODE],
            "{code} under {isin}"
        );
    }
    category
}

/// `error` located on the column `name`.
fn relocated(name: &str, error: Error) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason: match error {
            Error::InvalidRecord { reason, .. } | Error::InvalidDataType { reason, .. } => reason,
            other => format_smolstr!("{other}"),
        },
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/isin_registry.rs` pins and a caller cannot
    //! reach.

    /// What one instrument's row may take at most, in bytes.
    pub const ENTRY_CHARGE: usize = super::ENTRY_CHARGE;
}
