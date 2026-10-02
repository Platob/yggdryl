//! One row per ISIN: every equivalent an instrument is known by, learned
//! from the statements that name it and filled into the ones that leave it
//! unsaid.
//!
//! The ISIN is the one key: an [`IsinRegistry`] holds one [`IsinEntry`] per
//! canonical ISIN - its detailed CFI code, the market its listing facts were
//! stated on, its ticker and one code per FIX `SecurityIDSource(22)` type -
//! and a RIC leads back to its ISIN through an exact inverse index. A
//! Bloomberg symbol, a FIGI, a CUSIP are equivalents the ISIN fills, never
//! keys a lookup reads.
//!
//! A row is updated on differences: the latest statement leads, column by
//! column, and an older one only fills what the row leaves unsaid
//! ([`IsinRegistry::merge`]). The registry is a plain value - a clone is a
//! snapshot, and a write copies only while another clone shares the table -
//! read from and written to any holder through the crate's Arrow record
//! surface: [`IsinRegistry::from_handle`] reads one, and
//! `IOMedia::write_arrow_reader` writes [`IsinRegistry::into_arrow_reader`].

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
use crate::serie::{DateTimeNanosecondSerie, Utf8StringSerie};
use crate::warning::warned;
use crate::{
    ArrowCastOptions, Cfi, DataType, Error, Field, IOBase, IdKey, IdType, Identifier, Isin, Mic,
    Result, Scalar, Serie, SerieReader, StructType, TimeUnit, Timezone,
};

/// The name of the record a registry row is.
const ROOT: &str = "isinregistry";

/// The columns a row opens with, before one per equivalent type.
const NAMES: [&str; 5] = ["isin", "updunix", "cficode", "miccode", "ticker"];

/// The most bytes a ticker holds.
const MAX_TICKER_WIDTH: usize = 64;

/// The market a statement names none of.
const NO_MARKET: &str = "XXXX";

/// The heap one retained value may take: the widest value any identifier
/// type admits, behind its `Arc` counters and allocator rounding.
const MAX_VALUE_HEAP_ALLOWANCE: usize =
    IDENTIFIER_VALUE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// What one instrument's row may take at most: its key and entry twice over
/// for the B-tree's slack, its codes spilled to the heap with every value at
/// the widest heap a value takes, its ticker's heap, and its one RIC slot in
/// the inverse index with that RIC's heap.
const ENTRY_CHARGE: usize = 3 * 1024;

const _: () = assert!(
    ENTRY_CHARGE
        >= 2 * (size_of::<SmolStr>() + size_of::<IsinEntry>())
            + IsinRegistry::MAX_EQUIVALENTS
                * (size_of::<(IdType, SmolStr)>() + MAX_VALUE_HEAP_ALLOWANCE)
            + MAX_TICKER_WIDTH
            + 2 * size_of::<usize>()
            + 2 * align_of::<usize>()
            + 4 * (size_of::<(SmolStr, SmolStr)>() + 1)
            + MAX_VALUE_HEAP_ALLOWANCE
);

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

/// Whether a statement dated `stated` leads a row last moved at `held`: an
/// undated row is the oldest, an undated statement older than any dated
/// row, and a tie goes to the statement read later.
fn leads(stated: Option<i64>, held: Option<i64>) -> bool {
    match (stated, held) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(stated), Some(held)) => stated >= held,
    }
}

/// The later of two instants, an undated one the oldest.
fn later(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.max(right)
}

/// One instrument: its ISIN and the equivalents it is known by.
///
/// The row of an [`IsinRegistry`]: `isin`, `updunix` - when the statement
/// that last moved it happened, nanoseconds since the epoch, UTC - the
/// detailed `cficode`, the `miccode` its listing facts belong to, the
/// `ticker`, and one code per `SecurityIDSource(22)` type but the ISIN, at
/// most [`IsinRegistry::MAX_EQUIVALENTS`] of them, each held as its type
/// stores it.
///
/// ```
/// use yggdryl::{IdType, Isin, IsinEntry};
///
/// # fn main() -> yggdryl::Result<()> {
/// let entry = IsinEntry::new(Isin::new("CH0012214059")?)
///     .try_with_code(IdType::Ric, "HOLN.S")?
///     .try_with_code(IdType::Bloomberg, "HOLN SW Equity")?;
/// assert_eq!(entry.get(&IdType::Ric), Some("HOLN.S"));
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
    miccode: Option<Mic>,
    ticker: Option<SmolStr>,
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
        Field::new(NAMES[3], DataType::Mic, true),
        Field::new(NAMES[4], DataType::utf8(), true),
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
            miccode: None,
            ticker: None,
            codes: SmallVec::new(),
        }
    }

    /// The datatype a row is: the struct [`Self::field`] holds.
    #[must_use]
    pub fn dtype() -> DataType {
        FIELD.dtype().clone()
    }

    /// The required struct `isinregistry` a row is: `isin`, `updunix`,
    /// `cficode`, `miccode`, `ticker`, then one column per
    /// `SecurityIDSource(22)` type but the ISIN, in the code set's order,
    /// each of its type's [`IdType::value_dtype`].
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
    /// since the epoch, UTC; `None` for an undated row, the oldest.
    #[must_use]
    pub fn updunix(&self) -> Option<i64> {
        self.updunix
    }

    /// The detailed CFI classification.
    #[must_use]
    pub fn cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    /// The market the listing facts - the ticker and every listing code
    /// ([`IdType::is_listing`]) - were stated on.
    #[must_use]
    pub fn miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    /// The ticker of the listing.
    #[must_use]
    pub fn ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
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

    /// The row's listing on `code`; `XXXX`, no market, is stored as none.
    #[must_use]
    pub fn with_miccode(mut self, code: Option<Mic>) -> Self {
        self.miccode = code.filter(|code| code.as_str() != NO_MARKET);
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

    /// Drops the code of `kind`.
    fn remove_code(&mut self, kind: &IdType) {
        self.codes.retain(|(held, _)| held != kind);
    }

    /// This row as a statement.
    fn statement(&self) -> Statement<'_> {
        Statement {
            isin: self.isin.clone(),
            updunix: self.updunix,
            cficode: self.cficode.as_ref(),
            miccode: self.miccode.as_ref(),
            ticker: self.ticker.as_deref(),
            codes: self.iter().collect(),
        }
    }

    /// The row a statement makes on its own.
    fn from_statement(statement: &Statement<'_>) -> Self {
        let mut entry = Self::new(statement.isin.clone())
            .with_updunix(statement.updunix)
            .with_cficode(statement.cficode.cloned())
            .with_miccode(statement.miccode.cloned())
            .with_ticker(statement.ticker.map(SmolStr::new));
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
                self.miccode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (NAMES[4], text(self.ticker())),
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
        let [isin, updunix, cficode, miccode, ticker, codes @ ..] = cells.as_ref() else {
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
            .with_miccode(match miccode {
                Scalar::Mic(code) => Some(code.clone()),
                _ => None,
            })
            .with_ticker(ticker.as_str().map(SmolStr::new));
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
/// the ISIN, inline, held.
struct Statement<'s> {
    isin: Isin,
    updunix: Option<i64>,
    cficode: Option<&'s Cfi>,
    miccode: Option<&'s Mic>,
    ticker: Option<&'s str>,
    codes: SmallVec<[(&'s IdType, &'s str); IsinRegistry::MAX_EQUIVALENTS]>,
}

impl Statement<'_> {
    /// Whether it states any listing fact: a ticker or a listing code.
    fn states_listing(&self) -> bool {
        self.ticker.is_some() || self.codes.iter().any(|(kind, _)| kind.is_listing())
    }

    /// Whether it states anything past its ISIN.
    fn states_anything(&self) -> bool {
        self.cficode.is_some()
            || self.miccode.is_some()
            || self.ticker.is_some()
            || !self.codes.is_empty()
    }
}

/// `row` with `statement` folded in by the update rule, or `None` where
/// nothing moves; `leads` says whether the statement is the newer.
fn folded(row: &IsinEntry, statement: &Statement<'_>, leads: bool) -> Option<IsinEntry> {
    let mut next = Cow::Borrowed(row);
    // A column the row lacks is filled whatever the time; one it holds is
    // replaced only by a newer statement.
    let fill = |next: &mut Cow<'_, IsinEntry>, kind: &IdType, value: &str| match row.get(kind) {
        Some(held) if held == value => {}
        Some(_) if !leads => {}
        _ => next.to_mut().put_code(kind, value),
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
            Some(held) => match Cfi::refined(held.as_str(), stated.as_str()) {
                // Filling an `X` contradicts nothing, whatever the time.
                Some(refined) if refined != held.as_str() => {
                    next.to_mut().cficode = Cfi::new(refined).ok();
                }
                Some(_) => {}
                None if leads && held != stated => next.to_mut().cficode = Some(stated.clone()),
                None => {}
            },
        }
    }
    match (statement.miccode, &row.miccode) {
        (Some(stated), Some(held)) if stated != held => {
            // Another market: a newer statement of a listing fact switches
            // the listing whole; anything else leaves it.
            if leads && statement.states_listing() {
                let entry = next.to_mut();
                entry.miccode = Some(stated.clone());
                entry.ticker = statement.ticker.map(SmolStr::new);
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
            if let Some(stated) = statement.ticker {
                match row.ticker() {
                    Some(held) if held == stated => {}
                    Some(_) if !leads => {}
                    _ => next.to_mut().ticker = Some(SmolStr::new(stated)),
                }
            }
        }
    }
    match next {
        Cow::Borrowed(_) => None,
        Cow::Owned(mut entry) => {
            entry.updunix = later(row.updunix, statement.updunix);
            Some(entry)
        }
    }
}

/// Every instrument's equivalents, one row per ISIN.
///
/// A plain value: a clone is a snapshot that shares the table, and a write
/// copies it only while another clone shares it. Learning and filling are
/// the ordered lifecycle's, or an explicit [`Self::learn`], [`Self::fill`]
/// or [`Self::enrich`]; a parse never reaches one.
///
/// ```
/// use yggdryl::graph::{Event, Market, OrderEvent};
/// use yggdryl::{Cfi, IdKey, IdType, Identifier, IsinRegistry};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut stated = OrderEvent::default();
/// stated.set_currunix(1);
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
/// stated.set_cficode(Some(Cfi::new("ESVUFR")?), true);
/// let mut registry = IsinRegistry::new();
/// assert!(registry.learn(&stated));
/// assert_eq!(registry.get_by_ric("HOLN.S").map(|entry| entry.isin().as_str()), Some("CH0012214059"));
///
/// // A later statement naming only the RIC is filled with the rest.
/// let mut later = OrderEvent::default();
/// later.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
/// assert!(registry.fill(&mut later));
/// assert_eq!(later.get_isincode(), Some("CH0012214059"));
/// assert_eq!(later.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct IsinRegistry {
    /// The rows, keyed by the canonical ISIN text.
    rows: Arc<BTreeMap<SmolStr, IsinEntry>>,
    /// Each row's RIC to its ISIN: the exact inverse of the rows' `ric`.
    rics: Arc<HashMap<SmolStr, SmolStr>>,
    max_instruments: usize,
    /// Whether learning past the bound was warned of already.
    warned: bool,
}

/// The table every empty registry shares, so making one allocates nothing.
static EMPTY_ROWS: LazyLock<Arc<BTreeMap<SmolStr, IsinEntry>>> = LazyLock::new(Arc::default);
static EMPTY_RICS: LazyLock<Arc<HashMap<SmolStr, SmolStr>>> = LazyLock::new(Arc::default);

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

    /// An empty registry, bounded at [`Self::DEFAULT_MAX_INSTRUMENTS`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Arc::clone(&EMPTY_ROWS),
            rics: Arc::clone(&EMPTY_RICS),
            max_instruments: Self::DEFAULT_MAX_INSTRUMENTS,
            warned: false,
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
        self.rows.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The row of `isin`, borrowed.
    #[must_use]
    pub fn get(&self, isin: &str) -> Option<&IsinEntry> {
        self.rows.get(isin)
    }

    /// The row the RIC `ric` names, through the inverse index.
    #[must_use]
    pub fn get_by_ric(&self, ric: &str) -> Option<&IsinEntry> {
        self.rics.get(ric).and_then(|isin| self.rows.get(isin))
    }

    /// Every row, in ISIN order.
    pub fn iter(&self) -> impl Iterator<Item = &IsinEntry> {
        self.rows.values()
    }

    /// Folds `entry` into the row of its ISIN by the update rule: a column
    /// the row lacks is filled; one it holds is replaced by a statement at
    /// or after the row's `updunix` - an undated row the oldest, an undated
    /// statement older than any dated row - and kept against an older one;
    /// a CFI code that refines the held one refines it whatever the time; a
    /// newer listing fact on another market switches the listing whole; and
    /// a RIC another row holds moves to this row only from a statement at or
    /// after that row's. Whether anything moved.
    ///
    /// # Errors
    ///
    /// A `ZZ` ISIN, which names no country's instrument, and a new ISIN past
    /// [`Self::max_instruments`].
    pub fn merge(&mut self, entry: IsinEntry) -> Result<bool> {
        if entry.isin.as_str().starts_with("ZZ") {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.isin"),
                reason: format_smolstr!(
                    "expected an ISIN some country numbers, got {}",
                    entry.isin.as_str()
                ),
            });
        }
        self.fold(&entry.statement(), false)
    }

    /// Removes the row of `isin`, answering it.
    pub fn remove(&mut self, isin: &str) -> Option<IsinEntry> {
        if !self.rows.contains_key(isin) {
            return None;
        }
        let removed = Arc::make_mut(&mut self.rows).remove(isin)?;
        if let Some(ric) = removed.get(&IdType::Ric) {
            Arc::make_mut(&mut self.rics).remove(ric);
        }
        Some(removed)
    }

    /// Removes every row.
    pub fn clear(&mut self) {
        self.rows = Arc::clone(&EMPTY_ROWS);
        self.rics = Arc::clone(&EMPTY_RICS);
    }

    /// Folds one statement into its row, keyed by its ISIN; `older` forces
    /// it older than any row, so it only fills.
    fn fold(&mut self, statement: &Statement<'_>, older: bool) -> Result<bool> {
        let current = self.rows.get(statement.isin.as_str());
        let mut next = match current {
            Some(row) => {
                match folded(
                    row,
                    statement,
                    !older && leads(statement.updunix, row.updunix),
                ) {
                    Some(next) => next,
                    None => return Ok(false),
                }
            }
            None => {
                if self.rows.len() >= self.max_instruments {
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
        // A RIC names one listing at a time: one another row holds moves
        // here only from a statement at or after that row's.
        let held_ric = current
            .and_then(|row| row.get(&IdType::Ric))
            .map(SmolStr::new);
        let mut taken = None;
        if let Some(ric) = next.get(&IdType::Ric)
            && held_ric.as_deref() != Some(ric)
            && let Some(owner) = self.rics.get(ric)
            && owner.as_str() != statement.isin.as_str()
        {
            let owner_unix = self.rows.get(owner).and_then(IsinEntry::updunix);
            if !older && leads(statement.updunix, owner_unix) {
                taken = Some(owner.clone());
            } else {
                next.remove_code(&IdType::Ric);
                // The RIC the row held stays only on the listing it names:
                // a statement that switched the row to another market
                // leaves it with none.
                if let Some(held) = &held_ric
                    && current.is_some_and(|row| row.miccode == next.miccode)
                {
                    next.put_code(&IdType::Ric, held);
                }
                // Moved only by the RIC it did not take, the row keeps its
                // `updunix`.
                if let Some(row) = current {
                    let moved = std::mem::replace(&mut next.updunix, row.updunix);
                    if *row == next {
                        return Ok(false);
                    }
                    next.updunix = moved;
                }
            }
        }
        let ric = next.get(&IdType::Ric).map(SmolStr::new);
        let isin = SmolStr::new(next.isin.as_str());
        let rows = Arc::make_mut(&mut self.rows);
        if let Some(owner) = &taken
            && let Some(row) = rows.get_mut(owner)
        {
            row.remove_code(&IdType::Ric);
        }
        rows.insert(isin.clone(), next);
        if held_ric != ric {
            let rics = Arc::make_mut(&mut self.rics);
            if let Some(held) = &held_ric
                && rics.get(held) == Some(&isin)
            {
                rics.remove(held);
            }
            if let Some(ric) = ric {
                rics.insert(ric, isin);
            }
        }
        Ok(true)
    }

    /// Learns what `event` states about its instrument: keyed by its stated
    /// ISIN - canonical and not `ZZ` - else by its stated RIC through the
    /// inverse index, which only fills; dated at its `currunix`; reading
    /// its detailed CFI code, its market but `XXXX`, its ticker and each
    /// equivalent type its map answers, never one it only derived. A new
    /// ISIN past [`Self::max_instruments`] is not learned, with one warning.
    /// Whether anything moved.
    pub fn learn<E: Market + Event + ?Sized>(&mut self, event: &E) -> bool {
        let ids = event.get_securityids();
        let stated = |kind: &IdType| ids.get(kind).filter(|_| !ids.is_derived(kind));
        let (isin, older) = match stated(&IdType::Isin)
            .filter(|isin| Isin::is_canonical(isin) && !isin.starts_with("ZZ"))
        {
            Some(isin) => (Isin::from_proven(isin), false),
            None => match stated(&IdType::Ric).and_then(|ric| self.rics.get(ric)) {
                Some(isin) => (Isin::from_proven(isin), true),
                None => return false,
            },
        };
        let statement = Statement {
            isin,
            updunix: Some(event.get_currunix()),
            cficode: event
                .get_cficode()
                .filter(|code| Cfi::is_detailed(code.as_str())),
            miccode: event
                .get_miccode()
                .filter(|code| code.as_str() != NO_MARKET),
            ticker: event
                .get_ticker()
                .map(str::trim)
                .filter(|ticker| (1..=MAX_TICKER_WIDTH).contains(&ticker.len())),
            codes: equivalents()
                .filter_map(|kind| stated(kind).map(|value| (kind, value)))
                .take(Self::MAX_EQUIVALENTS)
                .collect(),
        };
        if !statement.states_anything() {
            return false;
        }
        if !self.rows.contains_key(statement.isin.as_str())
            && self.rows.len() >= self.max_instruments
        {
            if !self.warned {
                self.warned = true;
                warned!(
                    "instrument registry full",
                    "isin_registry",
                    "{} instruments, new ISINs are not learned",
                    self.max_instruments
                );
            }
            return false;
        }
        self.fold(&statement, older).unwrap_or(false)
    }

    /// Fills what `element` leaves unsaid about its instrument from the row
    /// its ISIN - stated or derived - names, else the row its RIC names,
    /// whose ISIN is derived first: each equivalent of a type it holds none
    /// of as a derived identifier, the listing codes and the ticker only
    /// where its market - none and `XXXX` unstated - is the row's or either
    /// is unstated, and its CFI code where it states none or the row's
    /// refines it. The element is finalized where anything moved, and
    /// nothing is built where nothing is filled. Whether anything moved.
    pub fn fill<E: Market + Element + ?Sized>(&self, element: &mut E) -> bool {
        let ids = element.get_securityids();
        let (row, by_ric) = match ids.get(&IdType::Isin) {
            Some(isin) => (self.rows.get(isin), false),
            None => (
                ids.get(&IdType::Ric).and_then(|ric| self.get_by_ric(ric)),
                true,
            ),
        };
        let Some(row) = row else {
            return false;
        };
        let mut moved = false;
        if by_ric {
            moved |= element.derive_securityid(&IdType::Isin, row.isin.as_str());
        }
        let same_market = match (
            element
                .get_miccode()
                .filter(|code| code.as_str() != NO_MARKET),
            &row.miccode,
        ) {
            (Some(stated), Some(held)) => stated == held,
            _ => true,
        };
        for (kind, value) in &row.codes {
            if (kind.is_listing() && !same_market) || element.get_securityids().contains_kind(kind)
            {
                continue;
            }
            moved |= element.derive_securityid(kind, value);
        }
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

    /// A registry read from `reader`'s rows ([`Self::extend_from_arrow_reader`]).
    ///
    /// # Errors
    ///
    /// What [`Self::extend_from_arrow_reader`] refuses.
    pub fn from_arrow_reader(reader: BatchReader) -> crate::arrow::Result<Self> {
        let mut registry = Self::new();
        registry.extend_from_arrow_reader(reader)?;
        Ok(registry)
    }

    /// Folds `reader`'s rows in, each through [`Self::merge`], so rows of
    /// one ISIN fold by their `updunix`; answers how many rows it read.
    ///
    /// Each column of the reader's schema is read as the registry column it
    /// names, resolved once: its canonical name, a spelling or alias of an
    /// [`IdType`] - `RIC`, `riccode`, `BloombergSymbol`, `ISINCode` - a
    /// field name a security type is read from (`#ISINCODE`, `cusip_code`),
    /// or `cfi`, `mic`, `symbol` for the CFI code, the market and the
    /// ticker; any other column is passed over. The stream is cast once,
    /// into [`IsinEntry::field`], a column it lacks null and a value a
    /// column cannot hold null.
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

    /// A registry read from `handle` ([`Self::extend_from_handle`]).
    ///
    /// # Errors
    ///
    /// What [`Self::extend_from_handle`] refuses.
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self> {
        let mut registry = Self::new();
        registry.extend_from_handle(handle)?;
        Ok(registry)
    }

    /// Folds the rows `handle` holds in: the handle's own record stream -
    /// an Arrow IPC file, Parquet, a folder of either, an object store - read
    /// through [`Self::extend_from_arrow_reader`]; a missing store reads as
    /// the empty stream. Answers how many rows it read.
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
        crate::arrow::rows::reader(
            &FIELD,
            Snapshot {
                rows: Arc::clone(&self.rows),
                after: None,
            },
            None,
            None,
            None,
            None,
        )
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
    const MARKET: [(&str, usize); 9] = [
        ("isin", 0),
        ("updunix", 1),
        ("cfi", 2),
        ("cficode", 2),
        ("mic", 3),
        ("miccode", 3),
        ("ticker", 4),
        ("symbol", 4),
        ("tickersymbol", 4),
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
    miccode: Arc<Utf8StringSerie>,
    ticker: Arc<Utf8StringSerie>,
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
                | Serie::Country(held),
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
            miccode: code(3)?,
            ticker: match text(4)? {
                (held, false) => held,
                (_, true) => return Err(unlanded(4)),
            },
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
            .with_miccode(self.miccode.value(row).map(Mic::from_proven))
            .with_ticker(self.ticker.value(row).map(SmolStr::new));
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
