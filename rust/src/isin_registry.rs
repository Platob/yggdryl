//! One row per ISIN: every fact an instrument is known by, learned from the
//! statements that name it and filled into the ones that leave it unsaid.
//!
//! The ISIN is the one key: an [`IsinRegistry`] holds one [`IsinEntry`] per
//! canonical ISIN - its detailed CFI code, its country of issue, the pair an
//! FX or referential number names, the market its listing facts were stated
//! on, its ticker and trading currency, and one code per FIX
//! `SecurityIDSource(22)` type - and a ticker leads back to its ISIN through
//! an exact inverse index, gated by the market. A RIC, a Bloomberg symbol, a
//! FIGI, a CUSIP are equivalents the ISIN fills, never keys a lookup reads.
//!
//! A row is updated on differences and no clock gates it: a statement's
//! non-null valid value fills a column the row lacks and replaces one it
//! holds that differs, an invalid value moves nothing, and `updunix` is a
//! stamp ([`IsinRegistry::merge`]). The registry holds its table apart from
//! the store it is bound to: [`IsinRegistry::from_holder`] loads one,
//! [`IsinRegistry::commit`] writes the table back as one snapshot where it
//! moved, and [`IsinRegistry::from_env`] is the process's own, located by
//! `YGGDRYL_ISIN_REGISTRY_URI`.

pub(crate) mod env;
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
use crate::serie::{DateTimeNanosecondSerie, Utf8StringSerie};
use crate::{
    ArrowCastOptions, Ccy, Cfi, CodeValue, Country, DataType, Error, Field, Forex, IOBase, IdKey,
    IdType, Identifier, Isin, Mic, Result, Scalar, Serie, SerieReader, StructType, TimeUnit,
    Timezone,
};

pub(crate) use store::Store;

/// The name of the record a registry row is.
const ROOT: &str = "isinregistry";

/// The columns a row opens with, before one per equivalent type.
const NAMES: [&str; 8] = [
    "isin",
    "updunix",
    "cficode",
    "countrycode",
    "forexcode",
    "miccode",
    "ticker",
    "currency",
];

/// The most bytes a ticker holds.
const MAX_TICKER_WIDTH: usize = 64;

/// The heap one retained value may take: the widest value any identifier
/// type admits, behind its `Arc` counters and allocator rounding.
const MAX_VALUE_HEAP_ALLOWANCE: usize =
    IDENTIFIER_VALUE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// What one instrument's row may take at most: its key and entry twice over
/// for the B-tree's slack - the entry holding its codes inline, a country,
/// a pair and a currency among them - its codes spilled to the heap with
/// every value at the widest heap a value takes, its ticker's heap, and its
/// one ticker slot in the ticker index with that ticker's heap.
const ENTRY_CHARGE: usize = 3 * 1024;

const _: () = assert!(
    ENTRY_CHARGE
        >= 2 * (size_of::<SmolStr>() + size_of::<IsinEntry>())
            + IsinRegistry::MAX_EQUIVALENTS
                * (size_of::<(IdType, SmolStr)>() + MAX_VALUE_HEAP_ALLOWANCE)
            + MAX_TICKER_WIDTH
            + 2 * size_of::<usize>()
            + 2 * align_of::<usize>()
            + MAX_TICKER_WIDTH
            + size_of::<(SmolStr, SmallVec<[SmolStr; 1]>)>()
            + 2 * size_of::<usize>()
);

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

/// One instrument: its ISIN and everything it is known by.
///
/// The row of an [`IsinRegistry`]: `isin`, `updunix` - when the statement
/// that last moved it happened, nanoseconds since the epoch, UTC - the
/// detailed `cficode`, the `countrycode` of issue where one was stated, the
/// `forexcode` an FX or referential number names, the `miccode` its listing
/// facts belong to, the `ticker` and the trading `currency` of that listing,
/// and one code per `SecurityIDSource(22)` type but the ISIN, at most
/// [`IsinRegistry::MAX_EQUIVALENTS`] of them, each held as its type stores
/// it.
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
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsinEntry {
    isin: Isin,
    updunix: Option<i64>,
    cficode: Option<Cfi>,
    countrycode: Option<Country>,
    forexcode: Option<Forex>,
    miccode: Option<Mic>,
    ticker: Option<SmolStr>,
    currency: Option<Ccy>,
    codes: SmallVec<[(IdType, SmolStr); 4]>,
}

static FIELD: LazyLock<Field> = LazyLock::new(|| {
    let mut fields = vec![
        Field::new(NAMES[0], DataType::isin(), false),
        Field::new(
            NAMES[1],
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            true,
        ),
        Field::new(NAMES[2], DataType::cfi(), true),
        Field::new(NAMES[3], DataType::country(), true),
        Field::new(NAMES[4], DataType::forex(), true),
        Field::new(NAMES[5], DataType::Mic, true),
        Field::new(NAMES[6], DataType::utf8(), true),
        Field::new(NAMES[7], DataType::ccy(), true),
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
            cficode: None,
            countrycode: None,
            forexcode: None,
            miccode: None,
            ticker: None,
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
    /// `cficode`, `countrycode`, `forexcode`, `miccode`, `ticker`,
    /// `currency`, then one column per `SecurityIDSource(22)` type but the
    /// ISIN, in the code set's order, each of its type's
    /// [`IdType::value_dtype`]: forty columns.
    #[must_use]
    pub fn field() -> Field {
        FIELD.clone()
    }

    /// The ISIN.
    #[must_use]
    pub fn isin(&self) -> &Isin {
        &self.isin
    }

    /// When the statement that last moved the row happened, nanoseconds
    /// since the epoch, UTC; `None` for an undated row. A stamp: it gates
    /// nothing.
    #[must_use]
    pub fn updunix(&self) -> Option<i64> {
        self.updunix
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

    /// The market the listing facts - the ticker, the currency and every
    /// listing code ([`IdType::is_listing`]) - were stated on.
    #[must_use]
    pub fn miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    /// The ticker of the listing.
    #[must_use]
    pub fn ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    /// The trading currency of the listing.
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
        Statement {
            isin: self.isin.clone(),
            updunix: self.updunix,
            cficode: self.cficode.as_ref(),
            countrycode: self.countrycode.as_ref(),
            forexcode: self.forexcode.as_ref(),
            miccode: self.miccode.as_ref(),
            ticker: self.ticker.as_deref(),
            currency: self.currency.as_ref(),
            codes,
        }
    }

    /// The row a statement makes on its own.
    fn from_statement(statement: &Statement<'_>) -> Self {
        let countrycode = statement
            .countrycode
            .and_then(|stated| Self::country_beside(&statement.isin, stated));
        let mut entry = Self::new(statement.isin.clone())
            .with_updunix(statement.updunix)
            .with_cficode(statement.cficode.cloned())
            .with_countrycode(countrycode)
            .with_forexcode(statement.forexcode.cloned())
            .with_miccode(statement.miccode.cloned())
            .with_ticker(statement.ticker.map(SmolStr::new))
            .with_currency(statement.currency.cloned());
        for (kind, value) in &statement.codes {
            entry.put_code(kind, value);
        }
        entry
    }

    /// The row as the named struct of its cells, a fact it does not state a
    /// null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let text = |value: Option<&str>| value.map_or(Scalar::Null, Scalar::from);
        let opening = [
            (NAMES[0], Scalar::from(self.isin.clone())),
            (
                NAMES[1],
                self.updunix
                    .and_then(|unix| {
                        Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok()
                    })
                    .unwrap_or(Scalar::Null),
            ),
            (
                NAMES[2],
                self.cficode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[3],
                self.countrycode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[4],
                self.forexcode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[5],
                self.miccode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (NAMES[6], text(self.ticker())),
            (
                NAMES[7],
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
            cficode,
            countrycode,
            forexcode,
            miccode,
            ticker,
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
            .with_miccode(match miccode {
                Scalar::Mic(code) => Some(code.clone()),
                _ => None,
            })
            .with_ticker(ticker.as_str().map(SmolStr::new))
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
    cficode: Option<&'s Cfi>,
    countrycode: Option<&'s Country>,
    forexcode: Option<&'s Forex>,
    miccode: Option<&'s Mic>,
    ticker: Option<&'s str>,
    currency: Option<&'s Ccy>,
    codes: SmallVec<[(&'s IdType, &'s str); IsinRegistry::MAX_EQUIVALENTS]>,
}

impl Statement<'_> {
    /// Whether it states a listing fact that moves a listing: a ticker or a
    /// listing code. A currency alone names no listing.
    fn states_listing(&self) -> bool {
        self.ticker.is_some() || self.codes.iter().any(|(kind, _)| kind.is_listing())
    }

    /// Whether it states anything past its ISIN.
    fn states_anything(&self) -> bool {
        self.cficode.is_some()
            || self.countrycode.is_some()
            || self.forexcode.is_some()
            || self.miccode.is_some()
            || self.ticker.is_some()
            || self.currency.is_some()
            || !self.codes.is_empty()
    }
}

/// `row` with `statement` folded in by the update rule, or `None` where
/// nothing moves: a stated value fills a column the row lacks and replaces
/// one it holds that differs; a CFI code compatible with the held one
/// refines it and a contradicting one replaces it; a listing fact stated on
/// another market switches the listing whole.
fn folded(row: &IsinEntry, statement: &Statement<'_>) -> Option<IsinEntry> {
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
    match (statement.miccode, &row.miccode) {
        (Some(stated), Some(held)) if stated != held => {
            // Another market: a ticker or a listing code stated there
            // switches the listing whole, and what the statement does not
            // restate is cleared; a currency alone leaves the listing.
            if statement.states_listing() {
                let entry = next.to_mut();
                entry.miccode = Some(stated.clone());
                entry.ticker = statement.ticker.map(SmolStr::new);
                entry.currency = statement.currency.cloned();
                entry.codes.retain(|(kind, _)| !kind.is_listing());
                for (kind, value) in statement.codes.iter().filter(|(kind, _)| kind.is_listing()) {
                    entry.put_code(kind, value);
                }
            }
        }
        _ => {
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
    }
    match next {
        Cow::Borrowed(_) => None,
        Cow::Owned(mut entry) => {
            // The later instant; an undated one is the oldest.
            entry.updunix = row.updunix.max(statement.updunix);
            Some(entry)
        }
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

/// The table a registry holds, shared: the rows by ISIN and the ticker
/// index, two counted pointers that cross threads as they are. What a parse
/// door fixes once and every worker fills from, and what a lifecycle fills
/// from under the registry's lock.
#[derive(Clone, Debug)]
pub(crate) struct IsinTable {
    /// The rows, keyed by the canonical ISIN text.
    rows: Arc<BTreeMap<SmolStr, IsinEntry>>,
    /// Each ticker to the ISINs of the rows listing it, one per listing,
    /// in ISIN order: the exact inverse of the rows' `ticker`.
    tickers: Arc<HashMap<SmolStr, SmallVec<[SmolStr; 1]>>>,
}

/// The table every empty registry shares, so making one allocates nothing.
static EMPTY_ROWS: LazyLock<Arc<BTreeMap<SmolStr, IsinEntry>>> = LazyLock::new(Arc::default);
static EMPTY_TICKERS: LazyLock<Arc<HashMap<SmolStr, SmallVec<[SmolStr; 1]>>>> =
    LazyLock::new(Arc::default);

impl Default for IsinTable {
    fn default() -> Self {
        Self {
            rows: Arc::clone(&EMPTY_ROWS),
            tickers: Arc::clone(&EMPTY_TICKERS),
        }
    }
}

impl IsinTable {
    /// The row of `isin`, borrowed.
    pub(crate) fn get(&self, isin: &str) -> Option<&IsinEntry> {
        self.rows.get(isin)
    }

    /// The row the ticker `ticker` names on `market`
    /// ([`IsinRegistry::get_by_ticker`]).
    pub(crate) fn get_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Option<&IsinEntry> {
        let mut found = None;
        for isin in self.tickers.get(ticker.trim())? {
            let row = self.rows.get(isin)?;
            if Self::listed_on(row, market) {
                if found.is_some() {
                    return None;
                }
                found = Some(row);
            }
        }
        found
    }

    /// How many instruments it holds.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether it holds none.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows as a stream under [`IsinEntry::field`], in ISIN order, laid
    /// out one bounded batch at a time; a later write does not move it.
    fn snapshot(&self) -> crate::arrow::Result<BatchReader> {
        crate::arrow::rows::reader(
            &FIELD,
            Snapshot {
                rows: Arc::clone(&self.rows),
                after: None,
            },
            None,
            None,
            None,
        )
    }

    /// The row `element` names, and whether its ISIN is derived from it: the
    /// row of its real ISIN - stated or derived - a miss ending the fill;
    /// else the row its ticker names on its market, whose ISIN is derived
    /// first.
    fn row_of<E: Market + ?Sized>(&self, element: &E) -> Option<(&IsinEntry, bool)> {
        let ids = element.get_securityids();
        match ids.get(&IdType::Isin) {
            Some(isin) if IdType::Isin.is_real(isin) => self.rows.get(isin).map(|row| (row, false)),
            _ => element
                .get_ticker()
                .and_then(|ticker| self.get_by_ticker(ticker, element.get_miccode()))
                .map(|row| (row, true)),
        }
    }

    /// Whether `row` is listed on `market`: both stated and equal, or either
    /// unstated - none and `XXXX` unstated.
    fn listed_on(row: &IsinEntry, market: Option<&Mic>) -> bool {
        match (market.filter(|code| !code.is_none()), &row.miccode) {
            (Some(stated), Some(held)) => stated == held,
            _ => true,
        }
    }

    /// Derives into `element` each security identifier `row` holds of a
    /// type it holds none of - the ISIN itself where `derived`, every
    /// instrument code, the listing codes only on the same market, and the
    /// pair - through [`Market::derive_securityid`]. Whether anything moved.
    fn derive_into<E: Market + ?Sized>(
        row: &IsinEntry,
        derived: bool,
        same_market: bool,
        element: &mut E,
    ) -> bool {
        let mut moved = false;
        if derived {
            moved |= element.derive_securityid(&IdType::Isin, row.isin.as_str());
        }
        for (kind, value) in &row.codes {
            if (kind.is_listing() && !same_market) || element.get_securityids().contains_kind(kind)
            {
                continue;
            }
            moved |= element.derive_securityid(kind, value);
        }
        if let Some(pair) = &row.forexcode
            && !element.get_securityids().contains_kind(&IdType::Forex)
        {
            moved |= element.derive_securityid(&IdType::Forex, pair.as_str());
        }
        moved
    }

    /// Fills the security identifiers `element` leaves unsaid from the row
    /// it names ([`Self::row_of`]): what a parse takes from the table its
    /// door fixed - derived identifiers only, which reach no field, no
    /// wire and no digest. Whether anything moved; nothing is settled.
    pub(crate) fn fill_identifiers<E: Market + ?Sized>(&self, element: &mut E) -> bool {
        let Some((row, derived)) = self.row_of(element) else {
            return false;
        };
        let same_market = Self::listed_on(row, element.get_miccode());
        Self::derive_into(row, derived, same_market, element)
    }

    /// [`Self::fill_identifiers`], then the market facts a lifecycle fills:
    /// the ticker on the same market where the element states none, the CFI
    /// code where it states none or the row's refines it, and the currency
    /// only where both markets are stated and equal, the ticker is the
    /// row's and the element states none. Whether anything moved; nothing
    /// is settled.
    pub(crate) fn fill_unsettled<E: Market + ?Sized>(&self, element: &mut E) -> bool {
        let Some((row, derived)) = self.row_of(element) else {
            return false;
        };
        let market = element.get_miccode().filter(|code| !code.is_none());
        let same_market = Self::listed_on(row, market);
        let markets_equal = same_market && market.is_some() && row.miccode.is_some();
        let mut moved = Self::derive_into(row, derived, same_market, element);
        if same_market
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

/// Every instrument's facts, one row per ISIN.
///
/// The key is a real ISIN - closing under a listed prefix
/// ([`CodeValue::is_real`]) - and a ticker leads back to its row through an
/// exact inverse index, gated by the market the listing was stated on; a
/// RIC, a Bloomberg symbol, a FIGI are equivalents the ISIN fills. Learning
/// is the ordered lifecycle's, or an explicit [`Self::learn`],
/// [`Self::fill`] or [`Self::enrich`]; a parse fills derived identifiers
/// from the table its door fixed and learns nothing. The table is held
/// apart from the store the registry is bound to ([`Self::from_holder`],
/// [`Self::commit`]), and a write marks the registry dirty
/// ([`Self::is_dirty`]) until it is committed.
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

    /// How many instruments it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.table.len()
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

    /// The row of `isin`, borrowed.
    #[must_use]
    pub fn get(&self, isin: &str) -> Option<&IsinEntry> {
        self.table.get(isin)
    }

    /// The row the ticker `ticker` - trimmed of blanks, as a learn stores
    /// it - names on `market`, through the ticker index: the one row
    /// listing the ticker whose market is `market`, or
    /// whose market or `market` - none and `XXXX` unstated - is unstated.
    /// Two rows answering is ambiguous, and answers none.
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

    /// Every row, in ISIN order.
    pub fn iter(&self) -> impl Iterator<Item = &IsinEntry> {
        self.table.rows.values()
    }

    /// Folds `entry` into the row of its ISIN by the update rule: a stated
    /// value fills a column the row lacks and replaces one it holds that
    /// differs, whatever the time; a code that is no real value of its
    /// type is dropped with one warning per column; a CFI code compatible
    /// with the held one refines it and a contradicting one replaces it;
    /// a ticker or a listing code stated on another market switches the
    /// listing whole - market, ticker, currency and listing codes - and a
    /// currency alone never does; `updunix` becomes the later of the two
    /// where something moved. Whether anything moved.
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

    /// Removes the row of `isin`, answering it.
    pub fn remove(&mut self, isin: &str) -> Option<IsinEntry> {
        if !self.table.rows.contains_key(isin) {
            return None;
        }
        let removed = Arc::make_mut(&mut self.table.rows).remove(isin)?;
        if let Some(ticker) = removed.ticker() {
            unlist_ticker(Arc::make_mut(&mut self.table.tickers), ticker, isin);
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

    /// Folds one statement into its row, keyed by its ISIN.
    fn fold(&mut self, statement: &Statement<'_>) -> Result<bool> {
        let current = self.table.rows.get(statement.isin.as_str());
        let next = match current {
            Some(row) => match folded(row, statement) {
                Some(next) => next,
                None => return Ok(false),
            },
            None => {
                if self.table.rows.len() >= self.max_instruments {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("$['{}']", statement.isin.as_str()),
                        reason: format_smolstr!(
                            "expected at most {} instruments, got one more",
                            self.max_instruments
                        ),
                    });
                }
                IsinEntry::from_statement(statement)
            }
        };
        let held_ticker = current.and_then(|row| row.ticker.clone());
        let ticker = next.ticker.clone();
        let isin = SmolStr::new(next.isin.as_str());
        Arc::make_mut(&mut self.table.rows).insert(isin.clone(), next);
        // The ticker index follows the row's ticker: one slot per listing.
        if held_ticker != ticker {
            let tickers = Arc::make_mut(&mut self.table.tickers);
            if let Some(held) = &held_ticker {
                unlist_ticker(tickers, held, &isin);
            }
            if let Some(ticker) = ticker {
                let listed = tickers.entry(ticker).or_default();
                if let Err(at) = listed.binary_search(&isin) {
                    listed.insert(at, isin);
                }
            }
        }
        self.dirty = true;
        Ok(true)
    }

    /// Learns what `event` states about its instrument: keyed by its stated
    /// ISIN (a real one, closing under a listed prefix, never a derivation)
    /// and dated at its `currunix`; reading its detailed CFI code, its
    /// market but `XXXX`, its ticker, its currency but `XXX` (unless it
    /// holds a currency pair, where `Currency(15)` is the dealt currency
    /// and not the listing's), the pair it states as a `forex` identifier,
    /// and each equivalent type its map answers with a real value
    /// ([`IdType::is_real`]), never one it only derived, a masked number or
    /// a typo. A new ISIN past [`Self::max_instruments`] is not learned,
    /// with one warning. Whether anything moved.
    pub fn learn<E: Market + Event + ?Sized>(&mut self, event: &E) -> bool {
        let learned = self.learn_stating(event, None);
        if let Some(max) = learned.full {
            warn_full(max);
        }
        learned.moved
    }

    /// [`Self::learn`] beside the country of issue `event` stated, and
    /// what it answers in full: whether the row moved, and the bound a new
    /// ISIN was passed over at the first time one is, which the caller
    /// warns of itself ([`warn_full`]) - after it has let go of any lock it
    /// holds the registry under, since a warning reaches a host that may
    /// be waiting on that lock. Nothing here logs.
    pub(crate) fn learn_stating<E: Market + Event + ?Sized>(
        &mut self,
        event: &E,
        country: Option<&Country>,
    ) -> Learned {
        let ids = event.get_securityids();
        let stated = |kind: &IdType| ids.get(kind).filter(|_| !ids.is_derived(kind));
        let Some(isin) = stated(&IdType::Isin).filter(|isin| IdType::Isin.is_real(isin)) else {
            return Learned::default();
        };
        let forexcode = stated(&IdType::Forex).and_then(|pair| Forex::new(pair).ok());
        let currency = event.get_currency();
        let statement = Statement {
            isin: Isin::from_proven(isin),
            updunix: Some(event.get_currunix()),
            cficode: event
                .get_cficode()
                .filter(|code| Cfi::is_detailed(code.as_str())),
            countrycode: country.filter(|code| code.is_listed()),
            forexcode: forexcode.as_ref(),
            miccode: event.get_miccode().filter(|code| !code.is_none()),
            ticker: event
                .get_ticker()
                .map(str::trim)
                .filter(|ticker| (1..=MAX_TICKER_WIDTH).contains(&ticker.len())),
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
        if !statement.states_anything() {
            return Learned::default();
        }
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

    /// Fills what `element` leaves unsaid about its instrument from the row
    /// its real ISIN names - stated or derived, a miss ending the fill -
    /// else from the row its ticker names on its market
    /// ([`Self::get_by_ticker`]), whose ISIN is derived first over none or
    /// over a number ranking below it (a masked one, a typo):
    ///
    /// - each equivalent of a type it holds none of, as a derived
    ///   identifier - a listing code only where the markets agree or either
    ///   is unstated (`XXXX` counts as unstated) - and the pair;
    /// - the ticker, on the same market;
    /// - the CFI code where it states none or the row's refines it;
    /// - the currency only where both markets are stated and equal, the
    ///   ticker is the row's and it states none.
    ///
    /// The element is finalized where anything moved, and nothing is built
    /// where nothing is filled. Whether anything moved.
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

    /// Folds `reader`'s rows in, each through [`Self::merge`], so rows of
    /// one ISIN fold in the order they are read; answers how many rows it
    /// read.
    ///
    /// Each column of the reader's schema is read as the registry column it
    /// names, resolved once: its canonical name, a spelling or alias of an
    /// [`IdType`] - `RIC`, `riccode`, `BloombergSymbol`, `ISINCode`,
    /// `ccypair` - a field name a security type is read from (`#ISINCODE`,
    /// `cusip_code`), or `cfi`, `country`, `mic`, `symbol`, `ccy` for the
    /// CFI code, the country, the market, the ticker and the currency; any
    /// other column is passed over. The stream is cast once, into
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
        if !resolved.iter().any(|(_, target)| *target == 0) {
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
        let records = SerieReader::from_arrow_reader(
            Some(&*FIELD),
            Box::new(RecordBatchIterator::new(batches, renamed)),
            ArrowCastOptions::default(),
        )?;
        let mut read = 0;
        for record in records {
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

    /// The rows as a stream under [`IsinEntry::field`], in ISIN order: a
    /// snapshot of the table as it stands, laid out one bounded batch at a
    /// time, which a learn while it streams does not move.
    ///
    /// # Errors
    ///
    /// What laying the rows out refuses, which no registry value causes.
    pub fn into_arrow_reader(&self) -> crate::arrow::Result<BatchReader> {
        self.table.snapshot()
    }
}

/// Drops `isin` from the listings of `ticker` in the ticker index, and the
/// ticker with it once nothing lists it.
fn unlist_ticker(tickers: &mut HashMap<SmolStr, SmallVec<[SmolStr; 1]>>, ticker: &str, isin: &str) {
    if let Some(listed) = tickers.get_mut(ticker) {
        listed.retain(|held| held.as_str() != isin);
        if listed.is_empty() {
            tickers.remove(ticker);
        }
    }
}

/// The rows of one snapshot, in ISIN order, each as its scalar.
struct Snapshot {
    rows: Arc<BTreeMap<SmolStr, IsinEntry>>,
    after: Option<SmolStr>,
}

impl Iterator for Snapshot {
    type Item = Scalar;

    fn next(&mut self) -> Option<Scalar> {
        let next = match &self.after {
            None => self.rows.iter().next(),
            Some(after) => self
                .rows
                .range::<str, _>((Bound::Excluded(after.as_str()), Bound::Unbounded))
                .next(),
        };
        let (isin, entry) = next?;
        self.after = Some(isin.clone());
        Some(entry.into_scalar())
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
    const MARKET: [(&str, usize); 15] = [
        ("isin", 0),
        ("updunix", 1),
        ("cfi", 2),
        ("cficode", 2),
        ("country", 3),
        ("countrycode", 3),
        ("countryofissue", 3),
        ("mic", 5),
        ("miccode", 5),
        ("ticker", 6),
        ("symbol", 6),
        ("tickersymbol", 6),
        ("ccy", 7),
        ("currency", 7),
        ("currencycode", 7),
    ];
    let mut buffer = [0_u8; IDENTIFIER_WORD_WIDTH];
    let folded = fold_into(name, &mut buffer).ok()?;
    if let Some((_, at)) = MARKET.iter().find(|(spelled, _)| *spelled == folded) {
        return Some(*at);
    }
    let kind = folded
        .parse::<IdType>()
        .ok()
        .filter(IdType::is_known)
        .or_else(|| IdType::from_field_name(name))?;
    match kind {
        IdType::Isin => Some(0),
        IdType::Cfi => Some(2),
        IdType::Forex => Some(4),
        kind => equivalents()
            .position(|held| *held == kind)
            .map(|at| at + NAMES.len()),
    }
}

/// One landed record's columns, each narrowed to its storage once.
struct Columns {
    isin: Arc<Utf8StringSerie>,
    updunix: Arc<DateTimeNanosecondSerie>,
    cficode: Arc<Utf8StringSerie>,
    countrycode: Arc<Utf8StringSerie>,
    forexcode: Arc<Utf8StringSerie>,
    miccode: Arc<Utf8StringSerie>,
    ticker: Arc<Utf8StringSerie>,
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
                | Serie::Forex(held),
            ) => Ok((Arc::clone(held), true)),
            _ => Err(unlanded(at)),
        };
        let code = |at: usize| match text(at)? {
            (held, true) => Ok(held),
            (_, false) => Err(unlanded(at)),
        };
        Ok(Self {
            isin: code(0)?,
            updunix: match children.get(1) {
                Some(Serie::DateTimeNanosecond(held)) => Arc::clone(held),
                _ => return Err(unlanded(1)),
            },
            cficode: code(2)?,
            countrycode: code(3)?,
            forexcode: code(4)?,
            miccode: code(5)?,
            ticker: match text(6)? {
                (held, false) => held,
                (_, true) => return Err(unlanded(6)),
            },
            currency: code(7)?,
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
            .with_miccode(self.miccode.value(row).map(Mic::from_proven))
            .with_ticker(self.ticker.value(row).map(SmolStr::new))
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
