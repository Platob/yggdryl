//! One element per instrument: every fact an instrument is known by,
//! learned from the statements that name it and filled into the ones that
//! leave it unsaid.
//!
//! An [`Instrument`] is an [`Element`] of the graph keyed by its cross
//! code: a real ISIN for a security an agency numbered, or a `class:body`
//! written from its CFI class and its typed characteristics for everything
//! no agency numbers (an FX pair, a forward, a swap, an option, a future, a
//! strategy); its identity is the code's digest (`uuid = crossuuid`) and
//! its content code the digest of every fact it holds. It holds its non-listing
//! identifiers ([`Instrument::securityids`]: the ISIN, the CFI, the pair,
//! the short name, the LEI, the national numbers, the number this crate
//! mints for it under `yggdryl:isin`), its country, currency and origin
//! currency, the code of the instrument it is written on and the codes of
//! its legs, its product category, its [`Characteristics`] and its
//! [`Listing`]s, one per market in MIC order. An [`Instruments`] holds one
//! per cross code; an ISIN, a ticker on its market and every code of the
//! closed set [`Instruments::LOOKUP_CODES`] names lead back to it, and
//! [`Instruments::resolve`] is the one door over them: a stated real ISIN,
//! else the code the element's own facts spell, else a code, else the
//! ticker on its market, else - a judgement, scored - the short name in the
//! element's currency.
//!
//! The collection holds its table apart from the store it is bound to:
//! [`Instruments::from_holder`] loads one, [`Instruments::seeded_from_holder`]
//! lays one over the seed, [`Instruments::commit`] writes the table back as
//! one snapshot where it moved, and [`Instruments::from_env`] is the
//! process's own, located by `YGGDRYL_INSTRUMENTS_URI` and laid over the
//! seed ([`Instruments::seeded`]): the common instruments
//! `config/instruments/instruments.json` states, embedded at build time.

pub(crate) mod env;
pub(crate) mod seed;
pub(crate) mod store;

use std::borrow::{Borrow, Cow};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Bound;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock};

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use crate::graph::{Market, Metadata};
use crate::identifier::IDENTIFIER_VALUE_WIDTH;
use crate::{Characteristics, Eusipa, IdKey, IdSource, IdType, Identifier, Identifiers, Listing};
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{Element, ElementColumn, Event};
use yggdryl::implementer::{FISN_WIDTH, Staged, warned};
use yggdryl::xxhash::Xxh3;
use yggdryl::{
    ArrowCastOptions, Ccy, Cfi, CodeValue, Country, DataType, Error, Field, Fisn, Forex, IOBase,
    Isin, Mic, Result, Scalar, Serie, Str, StreamChunkedSerie, TimeUnit, Timezone, Uuid,
};

pub(crate) use store::Store;

/// The name of the record an instrument row is.
const ROOT: &str = "instrument";

/// The columns of a row, in order: the six element columns, then the
/// instrument's.
const NAMES: [&str; 25] = [
    "uuid",
    "crossuuid",
    "crosscode",
    "hashcode",
    "crosshashcode",
    "srcuuids",
    "aliascodes",
    "placeholder",
    "isin",
    "cficode",
    "forexcode",
    "fisn",
    "countrycode",
    "currency",
    "origccy",
    "securityids",
    "underlying",
    "legs",
    "eusipacode",
    "characteristics",
    "listings",
    "metadata",
    "updunix",
    "firstunix",
    "lastunix",
];

/// The place of each column of [`NAMES`] in a row.
const CROSSCODE: usize = 2;
const HASHCODE: usize = 3;
const SRCUUIDS: usize = 5;
const ALIASCODES: usize = 6;
const PLACEHOLDER: usize = 7;
const ISIN: usize = 8;
const CFICODE: usize = 9;
const FOREXCODE: usize = 10;
const FISN: usize = 11;
const COUNTRYCODE: usize = 12;
const CURRENCY: usize = 13;
const ORIGCCY: usize = 14;
const SECURITYIDS: usize = 15;
const UNDERLYING: usize = 16;
const LEGS: usize = 17;
const EUSIPACODE: usize = 18;
const CHARACTERISTICS: usize = 19;
const LISTINGS: usize = 20;
const METADATA: usize = 21;
const UPDUNIX: usize = 22;
const FIRSTUNIX: usize = 23;
const LASTUNIX: usize = 24;

/// The most bytes a cross code holds: a strategy of more legs than fit is
/// refused by name, never truncated.
pub const MAX_CODE_WIDTH: usize = 128;

/// The heap one retained value may take: the widest value any identifier
/// type admits, behind its `Arc` counters and allocator rounding.
const MAX_VALUE_HEAP_ALLOWANCE: usize =
    IDENTIFIER_VALUE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// The heap a code of [`MAX_CODE_WIDTH`] takes behind its counters.
const CODE_HEAP_ALLOWANCE: usize =
    MAX_CODE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// What one instrument may take at most: its key and its element twice over
/// for the tree's slack, its identifiers and its listings' codes each at the
/// widest heap a value takes, its listings' tickers, its aliases, its legs
/// and its slots in the indexes, its metadata entries each at the widest
/// key and value, the record of what the store holds of it - [`ENTRY_COST`],
/// under 32 KiB on a 64-bit target, so the
/// charge is the next power of two of KiB: an instrument of several
/// listings with their codes, its named sources beside its base keys and
/// its metadata is wider than one listing row was.
const ENTRY_CHARGE: usize = 32 * 1024;

/// The heap one metadata key or value may take behind its counters.
const METADATA_HEAP_ALLOWANCE: usize =
    Instrument::MAX_METADATA_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// The sum [`ENTRY_CHARGE`] bounds.
const ENTRY_COST: usize = 2 * (size_of::<Str>() + size_of::<Instrument>())
    + CODE_HEAP_ALLOWANCE
    + Instrument::MAX_SECURITYIDS * (size_of::<Identifier>() + MAX_VALUE_HEAP_ALLOWANCE)
    + FISN_WIDTH
    // A metadata entry: its two texts and the tree node holding them.
    + Instrument::MAX_METADATA * (2 * size_of::<SmolStr>() + 2 * METADATA_HEAP_ALLOWANCE + 2 * size_of::<usize>())
    + Instrument::MAX_LISTINGS
        * (size_of::<Listing>()
            + Listing::MAX_TICKER_WIDTH
            + 2 * size_of::<usize>()
            // A code type is its base key and the named source stating it.
            + Instrument::MAX_LISTING_CODES * 2 * (size_of::<Identifier>() + MAX_VALUE_HEAP_ALLOWANCE))
    + Instrument::MAX_ALIASES * (size_of::<Str>() + CODE_HEAP_ALLOWANCE)
    + Instrument::MAX_LEGS * (size_of::<Leg>() + CODE_HEAP_ALLOWANCE)
    + Instrument::MAX_LISTINGS * (size_of::<(SmolStr, CodeSlots)>() + 2 * size_of::<usize>())
    + (Instruments::MAX_EQUIVALENTS + Instrument::MAX_LISTINGS * Instrument::MAX_LISTING_CODES)
        * (size_of::<(CodeKey, CodeSlots)>() + MAX_VALUE_HEAP_ALLOWANCE + 2 * size_of::<usize>())
    + 2 * (size_of::<(i128, Str)>() + 2 * size_of::<usize>())
    // The record of what the store holds of it.
    + size_of::<(Str, Stored)>()
    + 2 * size_of::<usize>();

const _: () = assert!(ENTRY_CHARGE >= ENTRY_COST);

/// The cross codes one index key names, in code order: one slot per
/// instrument, the first inline, each a clone of the instrument's own key
/// ([`Instrument::get_crosscode`]), so an index holds no copy of a code.
type CodeSlots = SmallVec<[Str; 1]>;

/// The table a parse reads crosses to every worker thread as it is.
const fn crosses_threads<T: Send + Sync>() {}
const _: () = crosses_threads::<InstrumentTable>();

/// The key the number this crate mints is held under on the instrument.
static MINT_KEY: LazyLock<IdKey> = LazyLock::new(|| {
    IdKey::new(
        "yggdryl"
            .parse()
            .expect("the crate's name is a source word"),
        IdType::Isin,
    )
});

/// The later of two instants, an undated one the oldest.
fn later(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    left.max(right)
}

/// The earlier of two instants, an undated one standing for none.
fn earlier(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (held, None) | (None, held) => held,
    }
}

/// The twelve bytes of a canonical ISIN packed to one number, big-endian:
/// what the ISIN index is keyed by, a copy and no parse.
fn packed(isin: &str) -> i128 {
    let mut bytes = [0_u8; 16];
    let text = isin.as_bytes();
    let width = text.len().min(12);
    bytes[..width].copy_from_slice(&text[..width]);
    i128::from_be_bytes(bytes)
}

/// A refusal located on `name`.
fn refused(name: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason,
    }
}

/// The two letters of a CFI's class - its category and group, the
/// positions [`Cfi::refined`] never moves - where ISO 10962 names both;
/// none for an unclassified `X` in either.
fn class_of(code: &str) -> Option<&str> {
    let mut letters = code.chars();
    let category = letters.next()?;
    let group = letters.next()?;
    if category == 'X' || group == 'X' {
        return None;
    }
    Cfi::category_of(category)?.group(group)?;
    Some(&code[..2])
}

/// Whether a class keys its instruments by a body rather than an ISIN: FX
/// (`I`, `J`, `S`), options and warrants (`O`, `H`), futures (`F`) and
/// strategies (`K`).
fn is_body_class(class: &str) -> bool {
    matches!(
        class.as_bytes()[0],
        b'I' | b'J' | b'S' | b'O' | b'H' | b'F' | b'K'
    )
}

/// A cross code is ASCII upper case with no byte below `0x20`, at most
/// [`MAX_CODE_WIDTH`] bytes.
fn check_code(code: &str, name: &str) -> Result<()> {
    if code.is_empty() || code.len() > MAX_CODE_WIDTH {
        return Err(refused(
            name,
            format_smolstr!(
                "expected a cross code of one to {MAX_CODE_WIDTH} bytes, got {} bytes",
                code.len()
            ),
        ));
    }
    if let Some(byte) = code
        .bytes()
        .find(|byte| !byte.is_ascii() || *byte < 0x20 || byte.is_ascii_lowercase())
    {
        return Err(refused(
            name,
            format_smolstr!(
                "expected an upper-case ASCII cross code, got the byte {byte:#04x} in {code:?}"
            ),
        ));
    }
    Ok(())
}

/// One leg of a strategy: the cross code of the leg instrument and its
/// ratio, FIX's `LegRatioQty(623)`, a whole number - `1` writes no prefix
/// into the strategy's code, any other `n*`. A leg is never a strategy.
///
/// ```
/// use yggdryl_market::Leg;
///
/// let leg = Leg::new("FF:EU0009658145:2026-12", 2).unwrap();
/// assert_eq!((leg.code(), leg.ratio()), ("FF:EU0009658145:2026-12", 2));
/// assert!(Leg::new("KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03", 1).is_err());
/// assert!(Leg::new("", 1).is_err());
/// assert!(Leg::new("US0378331005", 0).is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Leg {
    code: Str,
    ratio: u32,
}

impl Leg {
    /// A leg of `code` at `ratio`.
    ///
    /// # Errors
    ///
    /// An empty, lower-case or over-long code, a strategy's code, a ratio of
    /// zero.
    pub fn new(code: &str, ratio: u32) -> Result<Self> {
        check_code(code, "legs")?;
        if code.starts_with('K') && code.as_bytes().get(2) == Some(&b':') {
            return Err(refused(
                "legs",
                format_smolstr!("expected a leg that is no strategy, got {code:?}"),
            ));
        }
        if ratio == 0 {
            return Err(refused(
                "legs",
                SmolStr::new_static("expected a ratio of at least one, got 0"),
            ));
        }
        Ok(Self {
            code: Str::new(code),
            ratio,
        })
    }

    /// The leg instrument's cross code.
    #[must_use]
    pub fn code(&self) -> &str {
        self.code.as_str()
    }

    /// The ratio.
    #[must_use]
    pub fn ratio(&self) -> u32 {
        self.ratio
    }

    /// The leg as the named struct of its two cells, `code` and `ratio`.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        Scalar::from_struct([
            (LEG_NAMES[0], Scalar::from(self.code.storage().clone())),
            (LEG_NAMES[1], self.ratio_cell()),
        ])
        .expect("two distinct names")
    }

    /// The leg as the ordered run `code`, `ratio`: one allocation, the
    /// code's characters shared.
    // Built from borrowed facts, as `Instrument::into_scalar` is.
    #[allow(clippy::wrong_self_convention)]
    fn into_row(&self) -> Scalar {
        Scalar::from_sequence([Scalar::from(self.code.storage().clone()), self.ratio_cell()])
    }

    /// The ratio as the `int32` cell the row holds.
    fn ratio_cell(&self) -> Scalar {
        Scalar::from(i32::try_from(self.ratio).unwrap_or(i32::MAX))
    }

    /// The datatype a leg is: `struct<code: utf8, ratio: int32>`, both
    /// required.
    fn dtype() -> DataType {
        DataType::Struct(yggdryl::implementer::struct_type_from_unique_fields(vec![
            DataType::utf8().required_field(LEG_NAMES[0]),
            DataType::Int32.required_field(LEG_NAMES[1]),
        ]))
    }
}

/// The two columns of a leg.
const LEG_NAMES: [&str; 2] = ["code", "ratio"];

/// A fixed stack slot a cross code is written into, refusing the byte past
/// [`MAX_CODE_WIDTH`].
struct Slot<'a> {
    held: &'a mut [u8; MAX_CODE_WIDTH],
    len: usize,
}

impl fmt::Write for Slot<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        if end > MAX_CODE_WIDTH {
            return Err(fmt::Error);
        }
        self.held[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// The facts a cross code is written from, borrowed wherever they lie - an
/// instrument's or a statement's.
struct Keying<'k> {
    class: Option<&'k str>,
    isin: Option<&'k str>,
    forex: Option<&'k str>,
    characteristics: &'k Characteristics,
    underlying: Option<&'k str>,
    legs: &'k [Leg],
}

/// How a cross code was produced; what a FIX parse reads to write an
/// `instcode` only where the code is a function of the message alone
/// ([`implementer`](crate::implementer)).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Production {
    /// The twelve bytes of a real ISIN.
    Isin,
    /// A real ISIN standing in for a body no statement spells yet.
    Placeholder,
    /// `class:body`.
    Body,
}

/// Writes the cross code `keying` spells into `slot`: the `class:body`
/// where the class is a body class and its body is whole, else the real
/// ISIN - a placeholder where the class is a body class - else none.
///
/// # Errors
///
/// A code past [`MAX_CODE_WIDTH`], named.
fn write_code<'s>(
    keying: &Keying<'_>,
    slot: &'s mut [u8; MAX_CODE_WIDTH],
) -> Result<Option<(&'s str, Production)>> {
    use std::fmt::Write;
    let mut writer = Slot { held: slot, len: 0 };
    let written = (|| -> std::result::Result<Option<Production>, fmt::Error> {
        let Some(class) = keying.class else {
            return Ok(None);
        };
        let characteristics = keying.characteristics;
        let body = |writer: &mut Slot<'_>| -> std::result::Result<bool, fmt::Error> {
            match class.as_bytes()[0] {
                b'I' => {
                    let Some(forex) = keying.forex else {
                        return Ok(false);
                    };
                    write!(writer, "{class}:{forex}")?;
                }
                b'J' => {
                    let (Some(forex), Some(settle)) = (keying.forex, characteristics.settle())
                    else {
                        return Ok(false);
                    };
                    write!(writer, "{class}:{forex}:{settle}")?;
                }
                b'S' => {
                    let (Some(forex), Some(near), Some(far)) = (
                        keying.forex,
                        characteristics.settle(),
                        characteristics.settle2(),
                    ) else {
                        return Ok(false);
                    };
                    write!(writer, "{class}:{forex}:{near}:{far}")?;
                }
                b'O' | b'H' => {
                    let (Some(underlying), Some(expiry), Some(strike)) = (
                        keying.underlying,
                        characteristics.expiry(),
                        characteristics.strikepx(),
                    ) else {
                        return Ok(false);
                    };
                    write!(writer, "{class}:{underlying}:{expiry}:{strike}")?;
                }
                b'F' => {
                    let (Some(underlying), Some(expiry)) =
                        (keying.underlying, characteristics.expiry())
                    else {
                        return Ok(false);
                    };
                    write!(writer, "{class}:{underlying}:{}", expiry.month())?;
                }
                b'K' => {
                    if keying.legs.is_empty() {
                        return Ok(false);
                    }
                    write!(writer, "{class}:")?;
                    for (at, leg) in keying.legs.iter().enumerate() {
                        if at > 0 {
                            writer.write_str("+")?;
                        }
                        if leg.ratio != 1 {
                            write!(writer, "{}*", leg.ratio)?;
                        }
                        writer.write_str(leg.code.as_str())?;
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        };
        if body(&mut writer)? {
            return Ok(Some(Production::Body));
        }
        writer.len = 0;
        Ok(None)
    })();
    match written {
        Ok(Some(production)) => {
            let len = writer.len;
            let text = std::str::from_utf8(&slot[..len]).expect("ASCII facts");
            Ok(Some((text, production)))
        }
        Ok(None) => match keying.isin.filter(|isin| Isin::rank_of(isin) == 2) {
            Some(isin) => {
                let placeholder = keying.class.is_some_and(is_body_class);
                slot[..isin.len()].copy_from_slice(isin.as_bytes());
                let text = std::str::from_utf8(&slot[..isin.len()]).expect("a canonical ISIN");
                Ok(Some((
                    text,
                    if placeholder {
                        Production::Placeholder
                    } else {
                        Production::Isin
                    },
                )))
            }
            None => Ok(None),
        },
        Err(fmt::Error) => Err(refused(
            NAMES[CROSSCODE],
            format_smolstr!(
                "expected a cross code of at most {MAX_CODE_WIDTH} bytes, got one longer for the class {}",
                keying.class.unwrap_or("")
            ),
        )),
    }
}

/// Writes into `slot` the cross code `element`'s own facts spell - its CFI
/// class, its real ISIN, its pair - with `body` what a message spells
/// beside them and `underlying` the cross code of the instrument it is
/// written on, where one is known: the `class:body` where the class keys
/// by a body and the facts spell it whole, else the real ISIN - a
/// placeholder where the class keys by a body - else none. What the
/// table's cascade and a FIX parse spell through
/// ([`implementer`](crate::implementer)); a parse, holding no table, hands
/// no underlying and so spells a derivative no body.
///
/// # Errors
///
/// A code past [`MAX_CODE_WIDTH`], named.
pub(crate) fn spell_code<'s, E: Market + ?Sized>(
    element: &E,
    body: Option<&Body>,
    underlying: Option<&str>,
    slot: &'s mut [u8; MAX_CODE_WIDTH],
) -> Result<Option<(&'s str, Production)>> {
    let ids = element.get_securityids();
    let default = Characteristics::default();
    let keying = Keying {
        class: element.get_cficode().map(Cfi::as_str).and_then(class_of),
        isin: ids.get(&IdType::Isin),
        forex: body
            .and_then(|body| body.forex.as_ref())
            .map(Forex::as_str)
            .or_else(|| ids.get(&IdType::Forex)),
        characteristics: body.map_or(&default, |body| &body.characteristics),
        underlying,
        legs: body.map_or(&[], |body| body.legs.as_slice()),
    };
    write_code(&keying, slot)
}

/// One instrument: an [`Element`] keyed by its cross code, holding every
/// fact it is known by.
///
/// The key is a real ISIN for a security an agency numbered
/// ([`Self::for_security`]), and `class:body` - the two letters of its CFI
/// class, then the body its characteristics spell - for everything no
/// agency numbers ([`Self::for_body`]): `IF:EUR/USD`, `JF:EUR/USD:M3`,
/// `SF:USD/JPY:0:M3`, `OC:US0378331005:2026-12-18:200`,
/// `FF:EU0009658145:2026-12`, `KE:FF:...+FF:...`. A derivative stating a
/// real ISIN whose body no statement spells yet is keyed by that ISIN as a
/// placeholder ([`Self::is_placeholder`]), re-keyed once the body arrives,
/// the old code kept in [`Self::aliascodes`]. Nothing else moves the key: a
/// CFI refined, a ticker, a listing, a real ISIN learned for an FX pair -
/// each moves the content code alone. A `class:body` instrument holding no
/// ISIN of its own is minted one under `yggdryl:isin` ([`Self::minted_isin`]),
/// nine base-36 digits of its key's 128-bit digest behind the prefix `QY`,
/// rank one, so a real number replaces it whatever the order.
///
/// `uuid` is `crossuuid`: an instrument is a thing, not a statement of one,
/// so two statements of one instrument carry one identity and the trait's
/// own fold merges them. `hashcode` digests every fact through its typed
/// accessors; the stamps and the sources are provenance and never fed.
///
/// ```
/// use yggdryl::graph::Element;
/// use yggdryl::{Country, Isin, Mic};
/// use yggdryl_market::{IdType, Instrument, Listing};
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_market::install().unwrap();
/// let apple = Instrument::for_security(Isin::new("US0378331005")?)?
///     .with_listing(Listing::new(Some(Mic::new("XNAS")?)).with_ticker(Some("AAPL".into())))?
///     .try_with_code(IdType::Lei, "HWUPKR0MPOU8FGXBT394")?;
/// assert_eq!(apple.get_crosscode(), "US0378331005");
/// assert_eq!(format!("{:016x}", apple.get_crosshashcode()), "27e376388c8738fd");
/// assert_eq!(apple.get_uuid(), apple.get_crossuuid());
/// assert_eq!(apple.country(), Some(Country::new("US")?), "the prefix, stated nowhere");
/// assert_eq!(apple.ticker(Some(&Mic::new("XNAS")?)), Some("AAPL"));
/// assert_eq!(Instrument::from_scalar(&apple.into_scalar())?, apple);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instrument {
    uuid: Uuid,
    crossuuid: Uuid,
    crosscode: Str,
    hashcode: u64,
    crosshashcode: u64,
    srcuuids: Vec<Uuid>,
    aliascodes: Vec<Str>,
    placeholder: bool,
    securityids: Identifiers,
    countrycode: Option<Country>,
    currency: Option<Ccy>,
    origccy: Option<Ccy>,
    underlying: Option<Str>,
    legs: Vec<Leg>,
    eusipacode: Option<Eusipa>,
    characteristics: Characteristics,
    /// One per market in MIC order, the first inline: most instruments
    /// are listed once, and that listing then costs no allocation of its own.
    listings: SmallVec<[Listing; 1]>,
    metadata: Metadata,
    updunix: Option<i64>,
    firstunix: Option<i64>,
    lastunix: Option<i64>,
}

/// What a stored table partitions by: the first two bytes of the key -
/// the ISIN's country prefix for a security, the CFI class for everything
/// else - Iceberg's native truncation, which adds no column.
const PARTITION_BY: &str = "truncate(crosscode, 2)";

/// The order the rows keep: the key.
const SORT_BY: [&str; 1] = ["crosscode"];

/// The row declaring how a stored table partitions ([`PARTITION_BY`]) and
/// the order its rows keep ([`SORT_BY`]).
static FIELD: LazyLock<Field> = LazyLock::new(|| {
    let mut field = ROW.clone();
    field
        .as_partition_mut()
        .set_by_texts([PARTITION_BY])
        .expect("the instruments' partition declaration reads");
    field
        .as_sort_mut()
        .set_by_texts(SORT_BY)
        .expect("the instruments' order reads");
    field
});

/// The row declaring nothing: what a load lands foreign rows under.
static ROW: LazyLock<Field> = LazyLock::new(|| {
    let instant = || DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    };
    let mut fields = ElementColumn::fields().expect("the element columns");
    fields.extend([
        DataType::serie(DataType::utf8().required_field("aliascode"))
            .nullable_field(NAMES[ALIASCODES]),
        DataType::Boolean.required_field(NAMES[PLACEHOLDER]),
        DataType::isin().nullable_field(NAMES[ISIN]),
        DataType::cfi().nullable_field(NAMES[CFICODE]),
        DataType::forex().nullable_field(NAMES[FOREXCODE]),
        DataType::fisn().nullable_field(NAMES[FISN]),
        DataType::country().nullable_field(NAMES[COUNTRYCODE]),
        DataType::ccy().nullable_field(NAMES[CURRENCY]),
        DataType::ccy().nullable_field(NAMES[ORIGCCY]),
        Identifiers::dtype().nullable_field(NAMES[SECURITYIDS]),
        DataType::utf8().nullable_field(NAMES[UNDERLYING]),
        DataType::serie(Leg::dtype().required_field("leg")).nullable_field(NAMES[LEGS]),
        // A category is four digits; `int32` is the narrowest integer every
        // store - an Iceberg table among them - holds.
        DataType::Int32.nullable_field(NAMES[EUSIPACODE]),
        Characteristics::field(),
        DataType::serie(Listing::field()).nullable_field(NAMES[LISTINGS]),
        DataType::map_of(DataType::utf8(), DataType::utf8(), true)
            .expect("a text key beside a text value is a map's entries")
            .nullable_field(NAMES[METADATA]),
        instant().nullable_field(NAMES[UPDUNIX]),
        instant().nullable_field(NAMES[FIRSTUNIX]),
        instant().nullable_field(NAMES[LASTUNIX]),
    ]);
    Field::new(
        ROOT,
        DataType::Struct(yggdryl::implementer::struct_type_from_unique_fields(fields)),
        false,
    )
});

impl Element for Instrument {
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        self.crosscode.as_str()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.crosscode = Str::new(crosscode);
    }

    fn get_hashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.crosshashcode = crosshashcode;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.srcuuids
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        yggdryl::implementer::canonicalize_uuids(&mut sources);
        self.srcuuids = sources;
    }

    /// No instant: an instrument is a thing, which stands after nothing.
    fn is_after(&self, _: &Self) -> bool {
        false
    }

    fn finalize(&mut self) {
        self.sync_cross();
        self.uuid = self.crossuuid;
        self.hashcode = self.digest_instrument().as_u64();
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if !yggdryl::implementer::follow_element(&mut self, previous) {
            return None;
        }
        self.finalize();
        Some(self)
    }
}

impl Instrument {
    /// The most listings one instrument holds.
    pub const MAX_LISTINGS: usize = 6;

    /// The most listing code types one listing holds, each under its base
    /// key beside the named source that stated it.
    pub const MAX_LISTING_CODES: usize = 2;

    /// The most codes an instrument keeps of the keys it had before its
    /// re-keys: one per level of the placeholder cascade.
    pub const MAX_ALIASES: usize = 4;

    /// The most legs a strategy holds: what a code of [`MAX_CODE_WIDTH`]
    /// bytes holds of the shortest leg codes.
    pub const MAX_LEGS: usize = (MAX_CODE_WIDTH - 3) / 11;

    /// The most entries [`Self::securityids`] holds in all - the base keys
    /// of the four key types and of [`Instruments::MAX_EQUIVALENTS`]
    /// equivalents, each beside the one named source stating it - a new
    /// named key past it passed over by name.
    pub const MAX_SECURITYIDS: usize = 2 * (Instruments::MAX_EQUIVALENTS + 4);

    /// The most metadata entries one instrument holds; a new key past it is
    /// passed over by name.
    pub const MAX_METADATA: usize = 16;

    /// The most bytes a metadata key or value holds.
    pub const MAX_METADATA_WIDTH: usize = 128;

    /// An instrument stating nothing, under no key.
    fn blank() -> Self {
        Self {
            uuid: Uuid::default(),
            crossuuid: Uuid::default(),
            crosscode: Str::new_static(""),
            hashcode: 0,
            crosshashcode: 0,
            srcuuids: Vec::new(),
            aliascodes: Vec::new(),
            placeholder: false,
            securityids: Identifiers::new(),
            countrycode: None,
            currency: None,
            origccy: None,
            underlying: None,
            legs: Vec::new(),
            eusipacode: None,
            characteristics: Characteristics::default(),
            listings: SmallVec::new(),
            metadata: Metadata::new(),
            updunix: None,
            firstunix: None,
            lastunix: None,
        }
    }

    /// The instrument an agency numbered `isin`: keyed by the number alone,
    /// its CFI a fact beside it.
    ///
    /// # Errors
    ///
    /// An ISIN that is not real ([`CodeValue::is_real`]) - masked, mistyped,
    /// under no listed prefix.
    pub fn for_security(isin: Isin) -> Result<Self> {
        if !isin.is_real() {
            return Err(refused(
                NAMES[ISIN],
                format_smolstr!(
                    "expected an ISIN some agency numbers, got {}",
                    isin.as_str()
                ),
            ));
        }
        let mut instrument = Self::blank();
        instrument
            .securityids
            .insert(Identifier::new(IdKey::base(IdType::Isin), isin.as_str())?);
        instrument.respell()?;
        Ok(instrument)
    }

    /// The instrument no agency numbers: keyed `class:body` from `class` -
    /// the CFI, detailed or not - the pair `forex`, the `characteristics`,
    /// the `underlying` instrument's cross code and the `legs`, as the body
    /// of its class is written ([`MAX_CODE_WIDTH`]); the pair lands in its
    /// `securityids` as the body's one source, and its number is minted
    /// under `yggdryl:isin`.
    ///
    /// # Errors
    ///
    /// A class that keys by no body, a body the facts do not spell whole,
    /// a code past [`MAX_CODE_WIDTH`], an underlying or a leg code that is
    /// no cross code, more legs than fit.
    pub fn for_body(
        class: Cfi,
        forex: Option<&Forex>,
        characteristics: &Characteristics,
        underlying: Option<&str>,
        legs: &[Leg],
    ) -> Result<Self> {
        let mut instrument = Self::blank();
        instrument.put_cfi(Some(&class));
        if let Some(pair) = forex {
            instrument
                .securityids
                .set(Identifier::new(IdKey::base(IdType::Forex), pair.as_str())?);
        }
        instrument.characteristics = characteristics.clone();
        instrument.set_underlying(underlying)?;
        instrument.set_legs(legs)?;
        let mut slot = [0_u8; MAX_CODE_WIDTH];
        match write_code(&instrument.keying(), &mut slot)? {
            Some((_, Production::Body)) => {}
            _ => {
                return Err(refused(
                    NAMES[CROSSCODE],
                    format_smolstr!(
                        "expected a class and the characteristics its body is written from, got the class {} with no body",
                        class.as_str()
                    ),
                ));
            }
        }
        instrument.respell()?;
        Ok(instrument)
    }

    /// The datatype a row is: the struct [`Self::field`] holds.
    #[must_use]
    pub fn dtype() -> DataType {
        FIELD.dtype().clone()
    }

    /// The required struct `instrument` a row is: the six element columns
    /// (`uuid`, `crossuuid`, `crosscode`, `hashcode`, `crosshashcode`,
    /// `srcuuids`), `aliascodes` (`serie<utf8>`), `placeholder`
    /// (`boolean`, required), `isin`, `cficode`, `forexcode`, `fisn`,
    /// `countrycode`, `currency`, `origccy`, `securityids` (a sorted
    /// `map<utf8, utf8>`), `underlying` (`utf8`), `legs`
    /// (`serie<struct<code, ratio>>`), `eusipacode` (`int32`),
    /// `characteristics` ([`Characteristics::dtype`]), `listings`
    /// (`serie<` [`Listing::dtype`] `>`), `metadata` (a sorted
    /// `map<utf8, utf8>`, [`Self::metadata`]), `updunix`, `firstunix`,
    /// `lastunix`: twenty-five columns. `isin`, `cficode`, `forexcode` and
    /// `fisn` are projections of `securityids`.
    ///
    /// The root declares `PARTITION:by` `["truncate(crosscode, 2)"]` - the
    /// ISIN's country prefix for a security, the CFI class for everything
    /// else, an Iceberg table partitioning by Iceberg's own truncation of
    /// the key, which stores no column - and `SORT:by` `["crosscode"]`, the
    /// order the snapshot ([`Instruments::into_arrow_reader`]) streams in.
    ///
    /// ```
    /// use yggdryl_market::Instrument;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let field = Instrument::field();
    /// assert_eq!(field.field_len(), 25);
    /// assert_eq!(field.get_metadata("PARTITION:by"), Some(r#"["truncate(crosscode, 2)"]"#));
    /// assert_eq!(field.get_metadata("SORT:by"), Some(r#"["crosscode"]"#));
    /// assert_eq!(field.partition_field_names().count(), 0, "no column is marked");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn field() -> Field {
        FIELD.clone()
    }

    /// The facts the key is written from, borrowed.
    fn keying(&self) -> Keying<'_> {
        Keying {
            class: self.class(),
            isin: self.isin(),
            forex: self.securityids.get(&IdType::Forex),
            characteristics: &self.characteristics,
            underlying: self.underlying(),
            legs: &self.legs,
        }
    }

    /// Writes the key the held facts spell and finalizes; a key no fact
    /// spells is refused.
    fn respell(&mut self) -> Result<()> {
        let mut slot = [0_u8; MAX_CODE_WIDTH];
        let Some((code, production)) = write_code(&self.keying(), &mut slot)? else {
            return Err(refused(
                NAMES[CROSSCODE],
                SmolStr::new_static(
                    "expected a real ISIN or a class and the characteristics its body is written from, got neither",
                ),
            ));
        };
        self.placeholder = production == Production::Placeholder;
        if code != self.crosscode.as_str() {
            self.crosscode = Str::new(code);
        }
        if production == Production::Body && !self.securityids.contains_kind(&IdType::Isin) {
            self.mint();
        }
        self.finalize();
        Ok(())
    }

    /// The digest the mint reads: the XXH3-128 of the key, the value
    /// `crossuuid` takes once every cross identity moves to it.
    fn mint_digest(&self) -> u128 {
        yggdryl::xxhash::xxh128(self.crosscode.as_bytes())
    }

    /// Mints the number this crate gives an instrument no agency numbers
    /// ([`Isin::minted`] over [`Self::mint_digest`]) into `securityids`
    /// under `yggdryl:isin`, filling the base `isin` at rank one.
    fn mint(&mut self) {
        let number = Isin::minted(self.mint_digest());
        if let Ok(id) = Identifier::new(MINT_KEY.clone(), number.as_str()) {
            self.securityids.set(id);
        }
    }

    /// The number [`Isin::minted`] mints for `crosscode`: what
    /// [`Self::minted_isin`] holds, computed from the code alone.
    #[must_use]
    pub fn minted_number(crosscode: &str) -> Isin {
        Isin::minted(yggdryl::xxhash::xxh128(crosscode.as_bytes()))
    }

    /// The content code: the element's digest continued with every fact
    /// the instrument states, each under its name.
    fn digest_instrument(&self) -> Xxh3 {
        let mut state = Element::digest(self);
        {
            let mut staged = Staged::new(&mut state);
            for id in self.securityids.iter() {
                staged.feed("securityids", id.src().as_str().as_bytes());
                staged.feed("securityids", id.kind().as_str().as_bytes());
                staged.feed("securityids", id.value().as_bytes());
            }
            if let Some(code) = &self.countrycode {
                staged.feed(NAMES[COUNTRYCODE], code.as_str().as_bytes());
            }
            if let Some(code) = &self.currency {
                staged.feed(NAMES[CURRENCY], code.as_str().as_bytes());
            }
            if let Some(code) = &self.origccy {
                staged.feed(NAMES[ORIGCCY], code.as_str().as_bytes());
            }
            if let Some(code) = &self.underlying {
                staged.feed(NAMES[UNDERLYING], code.as_bytes());
            }
            for leg in &self.legs {
                staged.feed(NAMES[LEGS], leg.code.as_bytes());
                staged.feed(NAMES[LEGS], &leg.ratio.to_le_bytes());
            }
            self.characteristics
                .feed(|name, bytes| staged.feed(name, bytes));
            if let Some(code) = self.eusipacode {
                staged.feed(NAMES[EUSIPACODE], &code.code().to_le_bytes());
            }
            for listing in &self.listings {
                staged.feed(
                    "listings",
                    listing.miccode().map_or("", Mic::as_str).as_bytes(),
                );
                if let Some(ticker) = listing.ticker() {
                    staged.feed("listings", ticker.as_bytes());
                }
                if let Some(code) = listing.currency() {
                    staged.feed("listings", code.as_str().as_bytes());
                }
                for id in listing.codes().iter() {
                    staged.feed("listings", id.kind().as_str().as_bytes());
                    staged.feed("listings", id.value().as_bytes());
                }
            }
            for (key, value) in &self.metadata {
                staged.feed(NAMES[METADATA], key.as_bytes());
                staged.feed(NAMES[METADATA], value.as_bytes());
            }
            for alias in &self.aliascodes {
                staged.feed(NAMES[ALIASCODES], alias.as_bytes());
            }
            staged.feed(NAMES[PLACEHOLDER], &[u8::from(self.placeholder)]);
        }
        state
    }

    /// The codes this instrument had before its re-keys, sorted, unique.
    #[must_use]
    pub fn aliascodes(&self) -> &[Str] {
        &self.aliascodes
    }

    /// Whether the key is a real ISIN standing in for a body no statement
    /// spells yet: a derivative known by its number alone.
    #[must_use]
    pub fn is_placeholder(&self) -> bool {
        self.placeholder
    }

    /// Every non-listing identifier: the ISIN, the CFI, the pair, the short
    /// name, the LEI, the national numbers, the minted number - and a
    /// listing code stated on no market, until its market is learned.
    #[must_use]
    pub fn securityids(&self) -> &Identifiers {
        &self.securityids
    }

    /// The ISIN: real where an agency numbered the instrument, the minted
    /// one where this crate did, none where a collision refused the mint.
    #[must_use]
    pub fn isin(&self) -> Option<&str> {
        self.securityids.get(&IdType::Isin)
    }

    /// The number this crate minted for the instrument, where it did.
    #[must_use]
    pub fn minted_isin(&self) -> Option<&str> {
        self.securityids.get_from(&MINT_KEY)
    }

    /// Whether `number` is the number this crate minted for this key.
    #[must_use]
    pub fn is_own_mint(&self, number: &Isin) -> bool {
        Isin::is_minted(number.as_str(), self.mint_digest())
    }

    /// The CFI classification, where stated.
    #[must_use]
    pub fn cficode(&self) -> Option<Cfi> {
        self.securityids
            .get(&IdType::Cfi)
            .map(yggdryl::implementer::cfi_from_proven)
    }

    /// The two letters of the CFI class - category and group - where both
    /// are classified.
    #[must_use]
    pub fn class(&self) -> Option<&str> {
        self.securityids.get(&IdType::Cfi).and_then(class_of)
    }

    /// The currency pair an FX instrument is about, where stated.
    #[must_use]
    pub fn forexcode(&self) -> Option<Forex> {
        self.securityids
            .get(&IdType::Forex)
            .and_then(|pair| Forex::new(pair).ok())
    }

    /// The ISO 18774 short name, where stated.
    #[must_use]
    pub fn fisn(&self) -> Option<Fisn> {
        self.securityids
            .get(&IdType::Fisn)
            .and_then(|name| Fisn::new(name).ok())
    }

    /// The country of issue stated beside the key, where ISO 3166 lists it
    /// and it is not the ISIN's own prefix, which [`Self::country`] answers
    /// already.
    #[must_use]
    pub fn countrycode(&self) -> Option<&Country> {
        self.countrycode.as_ref()
    }

    /// The country of issue: the one stated, else a real ISIN's prefix
    /// where ISO 3166 lists it - an agency prefix names none, a minted
    /// number none, an FX pair none.
    #[must_use]
    pub fn country(&self) -> Option<Country> {
        self.countrycode.clone().or_else(|| {
            self.isin()
                .filter(|isin| Isin::rank_of(isin) == 2)
                .and_then(|isin| Country::new(&isin[..2]).ok())
                .filter(Country::is_listed)
        })
    }

    /// The instrument's currency: stated, an FX pair's quote leg, a
    /// derivative's underlying's; none where neither.
    #[must_use]
    pub fn currency(&self) -> Option<&Ccy> {
        self.currency.as_ref()
    }

    /// The currency the instrument was issued in, where a statement named
    /// one: never derived.
    #[must_use]
    pub fn origccy(&self) -> Option<&Ccy> {
        self.origccy.as_ref()
    }

    /// The cross code of the instrument this one is written on.
    #[must_use]
    pub fn underlying(&self) -> Option<&str> {
        self.underlying.as_ref().map(Str::as_str)
    }

    /// The identity of the instrument this one is written on, by the
    /// graph's cross rule over its code.
    #[must_use]
    pub fn underlying_uuid(&self) -> Option<Uuid> {
        self.underlying().map(cross_uuid_of)
    }

    /// The legs of a strategy, sorted as the key sorts them.
    #[must_use]
    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }

    /// The identities of the legs, by the graph's cross rule over their
    /// codes.
    pub fn leg_uuids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.legs.iter().map(|leg| cross_uuid_of(leg.code()))
    }

    /// The EUSIPA product category of a structured product.
    #[must_use]
    pub fn eusipacode(&self) -> Option<Eusipa> {
        self.eusipacode
    }

    /// The typed body facts.
    #[must_use]
    pub fn characteristics(&self) -> &Characteristics {
        &self.characteristics
    }

    /// Every listing, in MIC order.
    #[must_use]
    pub fn listings(&self) -> &[Listing] {
        &self.listings
    }

    /// The listing on `market`; the unlisted one where `market` is none.
    #[must_use]
    pub fn listing(&self, market: Option<&Mic>) -> Option<&Listing> {
        let market = market.filter(|code| !code.is_none());
        self.listings
            .iter()
            .find(|listing| listing.miccode() == market)
    }

    /// Where the listing on `market` sits, or where one would.
    fn listing_at(&self, market: Option<&Mic>) -> std::result::Result<usize, usize> {
        self.listings
            .binary_search_by(|listing| listing.miccode().cmp(&market))
    }

    /// The ticker on `market`; where `market` is none, the one ticker the
    /// instrument's single listing states.
    #[must_use]
    pub fn ticker(&self, market: Option<&Mic>) -> Option<&str> {
        match market.filter(|code| !code.is_none()) {
            Some(market) => self.listing(Some(market))?.ticker(),
            None => match self.listings.as_slice() {
                [one] => one.ticker(),
                _ => None,
            },
        }
    }

    /// The code of `kind`: the security identifiers first, then the
    /// listings in MIC order.
    #[must_use]
    pub fn get(&self, kind: &IdType) -> Option<&str> {
        self.securityids
            .get(kind)
            .or_else(|| self.listings.iter().find_map(|listing| listing.get(kind)))
    }

    /// The complementary facts no typed field holds, by key: what a
    /// statement stated beside its typed facts - a seed object's or a
    /// stored row's columns no field reads, a FIX message's instrument
    /// description - each key held once, a stated value filling an empty
    /// key and replacing a differing one. Never an identifier, which has
    /// its typed home in [`Self::securityids`] or a listing's codes.
    #[must_use]
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Holds `value` under the metadata `key`, replacing the one held.
    ///
    /// # Errors
    ///
    /// An empty key or value, one past [`Self::MAX_METADATA_WIDTH`], a new
    /// key past [`Self::MAX_METADATA`], each named at `$.metadata`.
    pub fn set_metadata(&mut self, key: &str, value: &str) -> Result<()> {
        for (what, text) in [("key", key), ("value", value)] {
            if text.is_empty() || text.len() > Self::MAX_METADATA_WIDTH {
                return Err(refused(
                    NAMES[METADATA],
                    format_smolstr!(
                        "expected a metadata {what} of one to {} bytes, got {} bytes",
                        Self::MAX_METADATA_WIDTH,
                        text.len()
                    ),
                ));
            }
        }
        if !self.accepts_metadata(key) {
            return Err(refused(
                NAMES[METADATA],
                format_smolstr!(
                    "expected at most {} metadata entries, got one more under {key:?}",
                    Self::MAX_METADATA
                ),
            ));
        }
        if self.metadata.get(key).is_none_or(|held| held != value) {
            self.metadata.insert(SmolStr::new(key), SmolStr::new(value));
            self.finalize();
        }
        Ok(())
    }

    /// [`Self::set_metadata`], consuming.
    ///
    /// # Errors
    ///
    /// What [`Self::set_metadata`] refuses.
    pub fn try_with_metadata(mut self, key: &str, value: &str) -> Result<Self> {
        self.set_metadata(key, value)?;
        Ok(self)
    }

    /// Removes the metadata `key`, answering its value.
    pub fn remove_metadata(&mut self, key: &str) -> Option<SmolStr> {
        let removed = self.metadata.remove(key);
        if removed.is_some() {
            self.finalize();
        }
        removed
    }

    /// Whether a metadata entry under `key` lands: a key held, or room
    /// under [`Self::MAX_METADATA`] for a new one.
    fn accepts_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key) || self.metadata.len() < Self::MAX_METADATA
    }

    /// When the statement that last moved a fact happened, nanoseconds since
    /// the epoch, UTC; a stamp.
    #[must_use]
    pub fn updunix(&self) -> Option<i64> {
        self.updunix
    }

    /// The earliest instant of an event the instrument was learned from.
    #[must_use]
    pub fn firstunix(&self) -> Option<i64> {
        self.firstunix
    }

    /// The latest instant of an event the instrument was learned from.
    #[must_use]
    pub fn lastunix(&self) -> Option<i64> {
        self.lastunix
    }

    /// The instrument moved at `unix`.
    #[must_use]
    pub fn with_updunix(mut self, unix: Option<i64>) -> Self {
        self.updunix = unix;
        self
    }

    /// The instrument first learned at `unix`.
    #[must_use]
    pub fn with_firstunix(mut self, unix: Option<i64>) -> Self {
        self.firstunix = unix;
        self
    }

    /// The instrument last learned at `unix`.
    #[must_use]
    pub fn with_lastunix(mut self, unix: Option<i64>) -> Self {
        self.lastunix = unix;
        self
    }

    /// Holds `code` as the CFI, replacing the one held; none clears it.
    fn put_cfi(&mut self, code: Option<&Cfi>) {
        match code {
            Some(code) => {
                if let Ok(id) = Identifier::new(IdKey::base(IdType::Cfi), code.as_str()) {
                    self.securityids.set(id);
                }
            }
            None => {
                self.securityids.remove(&IdKey::base(IdType::Cfi));
            }
        }
    }

    /// The instrument classified as `code`; the key follows where the
    /// class keys it.
    ///
    /// # Errors
    ///
    /// A key the facts no longer spell.
    pub fn try_with_cficode(mut self, code: Option<Cfi>) -> Result<Self> {
        self.put_cfi(code.as_ref());
        self.respell()?;
        Ok(self)
    }

    /// The instrument's country of issue as stated; a code ISO 3166 does
    /// not list is stored as none. Folded into a collection, the ISIN's own
    /// prefix is held by no instrument - it states nothing the key does not
    /// - and takes a differing held country back.
    #[must_use]
    pub fn with_countrycode(mut self, code: Option<Country>) -> Self {
        self.countrycode = code.filter(Country::is_listed);
        self.finalize();
        self
    }

    /// The instrument's currency; `XXX`, no currency, is stored as none.
    #[must_use]
    pub fn with_currency(mut self, code: Option<Ccy>) -> Self {
        self.currency = code.filter(|code| !code.is_none());
        self.finalize();
        self
    }

    /// The instrument's origin currency; `XXX` is stored as none.
    #[must_use]
    pub fn with_origccy(mut self, code: Option<Ccy>) -> Self {
        self.origccy = code.filter(|code| !code.is_none());
        self.finalize();
        self
    }

    /// The instrument's short name.
    #[must_use]
    pub fn with_fisn(mut self, name: Option<Fisn>) -> Self {
        match name {
            Some(name) => {
                if let Ok(id) = Identifier::new(IdKey::base(IdType::Fisn), name.as_str()) {
                    self.securityids.set(id);
                }
            }
            None => {
                self.securityids.remove(&IdKey::base(IdType::Fisn));
            }
        }
        self.finalize();
        self
    }

    /// The instrument's EUSIPA product category.
    #[must_use]
    pub fn with_eusipacode(mut self, code: Option<Eusipa>) -> Self {
        self.eusipacode = code;
        self.finalize();
        self
    }

    /// Holds `code` as the underlying's cross code; the instrument's own
    /// is none.
    fn set_underlying(&mut self, code: Option<&str>) -> Result<()> {
        self.underlying = match code {
            Some(code) if code != self.crosscode.as_str() => {
                check_code(code, NAMES[UNDERLYING])?;
                Some(Str::new(code))
            }
            _ => None,
        };
        Ok(())
    }

    /// The instrument written on the instrument of `code`; the key follows
    /// where the class keys it by its underlying.
    ///
    /// # Errors
    ///
    /// A code that is no cross code, a key the facts no longer spell.
    pub fn try_with_underlying(mut self, code: Option<&str>) -> Result<Self> {
        self.set_underlying(code)?;
        self.respell()?;
        Ok(self)
    }

    /// Holds `legs` sorted and unique, the strategy's.
    fn set_legs(&mut self, legs: &[Leg]) -> Result<()> {
        let mut held: Vec<Leg> = legs.to_vec();
        held.sort();
        held.dedup();
        if held.len() > Self::MAX_LEGS {
            return Err(refused(
                NAMES[LEGS],
                format_smolstr!(
                    "expected at most {} legs, got {}",
                    Self::MAX_LEGS,
                    held.len()
                ),
            ));
        }
        self.legs = held;
        Ok(())
    }

    /// The strategy of `legs`, sorted bytewise and deduplicated; the key
    /// follows.
    ///
    /// # Errors
    ///
    /// More legs than [`Self::MAX_LEGS`], a key the facts no longer spell
    /// or one past [`MAX_CODE_WIDTH`].
    pub fn try_with_legs(mut self, legs: &[Leg]) -> Result<Self> {
        self.set_legs(legs)?;
        self.respell()?;
        Ok(self)
    }

    /// The instrument with `characteristics`; the key follows where the
    /// class keys it by them.
    ///
    /// # Errors
    ///
    /// A key the facts no longer spell or one past [`MAX_CODE_WIDTH`].
    pub fn try_with_characteristics(mut self, characteristics: Characteristics) -> Result<Self> {
        self.characteristics = characteristics;
        self.respell()?;
        Ok(self)
    }

    /// The equivalents held: the base entries of every type but the ISIN,
    /// the CFI, the pair and the short name, which have columns of their
    /// own.
    fn equivalents(&self) -> impl Iterator<Item = &Identifier> {
        self.securityids.iter().filter(|id| {
            id.key().is_base()
                && !matches!(
                    id.kind(),
                    IdType::Isin | IdType::Cfi | IdType::Forex | IdType::Fisn
                )
        })
    }

    /// States `value` as the code of `kind`, held as the type stores it: a
    /// security type into the identifiers; a listing type onto the one
    /// listing, into the identifiers where the instrument holds none.
    ///
    /// # Errors
    ///
    /// A key type - `isin`, `cfi`, `forex`, `fisn` - which has a door of
    /// its own; a value the type refuses; a new type past
    /// [`Instruments::MAX_EQUIVALENTS`]; a listing type where the instrument
    /// holds several listings, which [`Self::with_listing`] states.
    pub fn set_code(&mut self, kind: IdType, value: &str) -> Result<()> {
        if matches!(
            kind,
            IdType::Isin | IdType::Cfi | IdType::Forex | IdType::Fisn
        ) {
            return Err(refused(
                kind.as_str(),
                format_smolstr!(
                    "expected an equivalent code type, got {kind}, which has its own door"
                ),
            ));
        }
        let id = Identifier::new(IdKey::base(kind), value)?;
        if id.kind().is_listing() {
            match self.listings.len() {
                0 => {}
                1 => {
                    let kind = id.kind().clone();
                    if !self.listings[0].adopt(id) {
                        return Err(refused(
                            self.listings[0].miccode().map_or("listing", Mic::as_str),
                            format_smolstr!(
                                "expected at most {} listing codes, got one more",
                                Self::MAX_LISTING_CODES
                            ),
                        ));
                    }
                    // One holder: the listing's.
                    self.securityids.remove(&IdKey::base(kind));
                    self.finalize();
                    return Ok(());
                }
                _ => {
                    return Err(refused(
                        id.kind().as_str(),
                        format_smolstr!(
                            "expected a listing code stated on its market, got {} on an instrument of {} listings",
                            id.value(),
                            self.listings.len()
                        ),
                    ));
                }
            }
        }
        if !self.adopt_equivalent(id) {
            return Err(refused(
                kind_name(&self.securityids),
                format_smolstr!(
                    "expected at most {} equivalents, got one more",
                    Instruments::MAX_EQUIVALENTS
                ),
            ));
        }
        self.finalize();
        Ok(())
    }

    /// Holds `id` - an equivalent its type proved under its base key, or a
    /// named source's statement of any type, which fills the base key by
    /// the map's own rule - where the bounds leave room; answers whether it
    /// landed.
    fn adopt_equivalent(&mut self, id: Identifier) -> bool {
        if !self.accepts_equivalent(id.key()) {
            return false;
        }
        self.securityids.set(id);
        true
    }

    /// Whether an identifier under `key` lands: a base key of a type held,
    /// or of a new type with room under [`Instruments::MAX_EQUIVALENTS`]; a
    /// named key held, or a new one with room under
    /// [`Self::MAX_SECURITYIDS`] for it and the base key it fills.
    fn accepts_equivalent(&self, key: &IdKey) -> bool {
        if key.is_base() {
            return self.securityids.contains_kind(key.kind())
                || self.equivalents().count() < Instruments::MAX_EQUIVALENTS;
        }
        self.securityids.get_from(key).is_some()
            || self.securityids.len() + 2 <= Self::MAX_SECURITYIDS
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

    /// Holds `listing` among the listings, in MIC order: one on a market
    /// already listed replaces that one, the unlisted one is taken over by
    /// the first market; the listing codes the identifiers hold on no
    /// market move onto it, every source's statement of the type with the
    /// base key, so a code has one holder at any instant - a type the
    /// listing already states wins, one past its bound stays where it was.
    /// Answers whether it landed, a new market past [`Self::MAX_LISTINGS`]
    /// passed over.
    fn put_listing(&mut self, mut listing: Listing) -> bool {
        for kind in self
            .securityids
            .iter()
            .filter(|id| id.key().is_base() && id.kind().is_listing())
            .map(|id| id.kind().clone())
            .collect::<Vec<_>>()
        {
            if listing.codes().contains_kind(&kind) {
                self.securityids.remove(&IdKey::base(kind));
                continue;
            }
            if !listing.accepts(&kind) {
                continue;
            }
            let moving: Vec<Identifier> = self.securityids.of_kind(&kind).cloned().collect();
            self.securityids.remove(&IdKey::base(kind));
            for id in moving {
                listing.adopt(id);
            }
        }
        if let Some(market) = listing.miccode()
            && let [unlisted] = self.listings.as_mut_slice()
            && unlisted.miccode().is_none()
        {
            let market = market.clone();
            unlisted.set_miccode(market);
            if let Some(ticker) = listing.ticker() {
                unlisted.set_ticker(Some(ticker));
            }
            if let Some(currency) = listing.currency() {
                unlisted.set_currency(Some(currency.clone()));
            }
            for id in listing.codes().iter().filter(|id| id.key().is_base()) {
                unlisted.adopt(id.clone());
            }
            return true;
        }
        match self.listing_at(listing.miccode()) {
            Ok(at) => {
                self.listings[at] = listing;
                true
            }
            Err(at) => {
                if self.listings.len() >= Self::MAX_LISTINGS {
                    return false;
                }
                self.listings.insert(at, listing);
                true
            }
        }
    }

    /// The instrument with `listing`: one on a market already listed
    /// replaces it; the unlisted listing is taken over by the first market.
    ///
    /// # Errors
    ///
    /// A new market past [`Self::MAX_LISTINGS`].
    pub fn with_listing(mut self, listing: Listing) -> Result<Self> {
        if !self.put_listing(listing) {
            return Err(refused(
                NAMES[LISTINGS],
                format_smolstr!(
                    "expected at most {} listings, got one more",
                    Self::MAX_LISTINGS
                ),
            ));
        }
        self.finalize();
        Ok(self)
    }

    /// Fills the facts the instrument's own imply where it leaves them
    /// empty: the national number a real ISIN embeds
    /// ([`securityid::embedded`](crate::securityid::embedded)), none for a
    /// minted `QY` number; an FX pair's currency, its quote leg; each
    /// listing's currency, the legal tender of its market's country. A
    /// default never displaces a statement.
    fn derive_defaults(&mut self) {
        if let Some(isin) = self.isin().filter(|isin| Isin::rank_of(isin) == 2) {
            let isin = yggdryl::implementer::isin_from_proven(isin);
            for id in crate::securityid::embedded(&isin) {
                if self.get(id.kind()).is_none() {
                    let base = IdKey::base(id.kind().clone());
                    if let Ok(id) = Identifier::new(base, id.value()) {
                        if id.kind().is_listing() {
                            if let [one] = self.listings.as_mut_slice() {
                                one.adopt(id);
                            }
                        } else {
                            self.adopt_equivalent(id);
                        }
                    }
                }
            }
        }
        if self.currency.is_none()
            && let Some(pair) = self.forexcode()
        {
            self.currency = Some(pair.quote()).filter(|code| !code.is_none());
        }
        for listing in &mut self.listings {
            if listing.currency().is_none()
                && let Some(code) = listing
                    .miccode()
                    .and_then(Mic::country)
                    .and_then(|country| country.currency())
                    .filter(|code| !code.is_none())
            {
                listing.set_currency(Some(code));
            }
        }
    }
}

/// The identity the graph's cross rule gives `code`: the one place this
/// crate spells the rule of the day.
fn cross_uuid_of(code: &str) -> Uuid {
    Uuid::from_v8(u128::from(yggdryl::implementer::crosshash(code)))
}

/// The name a refusal of one more equivalent is located on: the last type
/// held.
fn kind_name(ids: &Identifiers) -> &str {
    ids.iter()
        .last()
        .map_or(NAMES[SECURITYIDS], |id| id.kind().as_str())
}

impl Instrument {
    /// Whether the row states a value in the column at `at` of
    /// [`Self::field`]: the element columns, the key and the placeholder
    /// flag always, any other where it is held.
    pub(crate) fn states_column(&self, at: usize) -> bool {
        match at {
            ALIASCODES => !self.aliascodes.is_empty(),
            ISIN => self.isin().is_some(),
            CFICODE => self.securityids.contains_kind(&IdType::Cfi),
            FOREXCODE => self.securityids.contains_kind(&IdType::Forex),
            FISN => self.securityids.contains_kind(&IdType::Fisn),
            COUNTRYCODE => self.countrycode.is_some(),
            CURRENCY => self.currency.is_some(),
            ORIGCCY => self.origccy.is_some(),
            SECURITYIDS => !self.securityids.is_empty(),
            UNDERLYING => self.underlying.is_some(),
            LEGS => !self.legs.is_empty(),
            EUSIPACODE => self.eusipacode.is_some(),
            CHARACTERISTICS => !self.characteristics.is_default(),
            LISTINGS => !self.listings.is_empty(),
            METADATA => !self.metadata.is_empty(),
            UPDUNIX => self.updunix.is_some(),
            FIRSTUNIX => self.firstunix.is_some(),
            LASTUNIX => self.lastunix.is_some(),
            _ => at < NAMES.len(),
        }
    }

    /// The row as the named struct of its cells, a fact it does not state a
    /// null, each nested record - a listing, a leg, the characteristics -
    /// the named struct of its own cells: what a caller reads by name, in
    /// every language.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let mut cells = vec![Scalar::Null; NAMES.len()];
        self.write_cells(&mut cells, true);
        Scalar::from_struct(NAMES.iter().copied().map(SmolStr::new_static).zip(cells))
            .expect("distinct column names")
    }

    /// The row as the ordered run of its cells in [`Self::field`]'s order:
    /// what the snapshot ([`Instruments::into_arrow_reader`]) streams.
    // Built from borrowed facts, as `into_scalar` beside it is.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn into_row(&self) -> Scalar {
        yggdryl::implementer::scalar_try_build_sequence(NAMES.len(), |slots| {
            self.write_cells(slots, false);
            Ok(())
        })
        .expect("writing the cells refuses nothing")
    }

    /// Writes the row's cells into `slots`, one per column in order, each
    /// nested record `named` as the struct of its own cells
    /// ([`Self::into_scalar`]) or else as the ordered run its field
    /// canonicalizes to ([`Self::into_row`]: one allocation per collection
    /// the row states, and no name to fold).
    fn write_cells(&self, slots: &mut [Scalar], named: bool) {
        let instant = |unix: Option<i64>| {
            unix.and_then(|unix| Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok())
                .unwrap_or(Scalar::Null)
        };
        let code = |kind: IdType| {
            self.securityids.get(&kind).map_or(Scalar::Null, |value| {
                kind.value_dtype()
                    .scalar(Scalar::from(value))
                    .unwrap_or_else(|_| Scalar::from(value))
            })
        };
        for (column, slot) in ElementColumn::ALL.into_iter().zip(slots.iter_mut()) {
            *slot = column.fact(self).unwrap_or(Scalar::Null);
        }
        slots[ALIASCODES] = if self.aliascodes.is_empty() {
            Scalar::Null
        } else {
            Scalar::from_sequence(
                self.aliascodes
                    .iter()
                    .map(|code| Scalar::from(code.as_str())),
            )
        };
        slots[PLACEHOLDER] = Scalar::from(self.placeholder);
        slots[ISIN] = code(IdType::Isin);
        slots[CFICODE] = code(IdType::Cfi);
        slots[FOREXCODE] = code(IdType::Forex);
        slots[FISN] = code(IdType::Fisn);
        slots[COUNTRYCODE] = self.countrycode.clone().map_or(Scalar::Null, Scalar::from);
        slots[CURRENCY] = self.currency.clone().map_or(Scalar::Null, Scalar::from);
        slots[ORIGCCY] = self.origccy.clone().map_or(Scalar::Null, Scalar::from);
        slots[SECURITYIDS] = if self.securityids.is_empty() {
            Scalar::Null
        } else {
            self.securityids.into_scalar()
        };
        slots[UNDERLYING] = self.underlying().map_or(Scalar::Null, Scalar::from);
        slots[LEGS] = if self.legs.is_empty() {
            Scalar::Null
        } else if named {
            Scalar::from_sequence(self.legs.iter().map(Leg::into_scalar))
        } else {
            Scalar::from_sequence(self.legs.iter().map(Leg::into_row))
        };
        slots[EUSIPACODE] = self
            .eusipacode
            .map_or(Scalar::Null, |code| Scalar::from(i32::from(code.code())));
        slots[CHARACTERISTICS] = if self.characteristics.is_default() {
            Scalar::Null
        } else if named {
            self.characteristics.into_scalar()
        } else {
            self.characteristics.into_row()
        };
        slots[LISTINGS] = if self.listings.is_empty() {
            Scalar::Null
        } else if named {
            Scalar::from_sequence(self.listings.iter().map(Listing::into_scalar))
        } else {
            Scalar::from_sequence(self.listings.iter().map(Listing::into_row))
        };
        slots[METADATA] = if self.metadata.is_empty() {
            Scalar::Null
        } else {
            let entries: Arc<[(Scalar, Scalar)]> = self
                .metadata
                .iter()
                .map(|(key, value)| (Scalar::from(key.clone()), Scalar::from(value.clone())))
                .collect();
            Scalar::SortedMap(yggdryl::Map::new(entries))
        };
        slots[UPDUNIX] = instant(self.updunix);
        slots[FIRSTUNIX] = instant(self.firstunix);
        slots[LASTUNIX] = instant(self.lastunix);
    }

    /// Reads an instrument back from the named struct [`Self::into_scalar`]
    /// answers - a nested listing, leg or characteristics a struct of its
    /// own names or the ordered run its field canonicalizes to - or from
    /// the ordered row of [`Self::field`]'s cells, through that field's
    /// value door. A column the value lacks or states null is unstated: a
    /// nullable one null, an element column or `placeholder` derived, since
    /// a statement is derived - the key is written again from the typed
    /// columns and the identity and the content code from it - so a row
    /// stating only its facts reads as the instrument those facts spell.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers - a name no
    /// column has among them; a stored `crosscode` the typed columns do not
    /// spell, named beside the one they do at `$.crosscode`; a bound passed.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let children = FIELD.fields();
        // What a column reads as: its cell, a nullable one it lacks null, a
        // required one it lacks - derived at the respelling - its default.
        let cell_of = |child: &Field, cell: Option<&Scalar>| -> Result<Scalar> {
            match cell {
                Some(cell) if !cell.is_null() || child.is_nullable() => Ok(cell.clone()),
                _ if child.is_nullable() => Ok(Scalar::Null),
                _ => child.default_value(),
            }
        };
        let value = match value.as_struct() {
            Some(cells) => {
                let columns = children.iter().map(|child| {
                    Ok((
                        SmolStr::new(child.name()),
                        cell_of(child, cells.get(child.name()))?,
                    ))
                });
                // A name no column has stays, for the value door to refuse by name.
                let foreign = cells
                    .iter()
                    .filter(|(name, _)| !NAMES.contains(&name.as_str()))
                    .map(|(name, cell)| Ok((name.clone(), cell.clone())));
                Scalar::from_struct(columns.chain(foreign).collect::<Result<Vec<_>>>()?)?
            }
            None => match value.sequence_rows() {
                Some(cells) if cells.len() == children.len() => Scalar::from_sequence(
                    children
                        .iter()
                        .zip(cells.iter())
                        .map(|(child, cell)| cell_of(child, Some(cell)))
                        .collect::<Result<Vec<_>>>()?,
                ),
                _ => value.clone(),
            },
        };
        let row = FIELD.scalar(value)?;
        let unread = || Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: yggdryl::implementer::expected_got("the canonical instrument row", row.kind()),
        };
        let cells = row.sequence_rows().ok_or_else(unread)?;
        if cells.len() != NAMES.len() {
            return Err(unread());
        }
        Self::from_cells(&cells)
    }

    /// An instrument from the canonical cells of one row, in
    /// [`Self::field`]'s order.
    fn from_cells(cells: &[Scalar]) -> Result<Self> {
        let unread = |at: usize| Error::InvalidRecord {
            path: format_smolstr!("$.{}", NAMES[at]),
            reason: yggdryl::implementer::expected_got(
                "the column's canonical cell",
                cells[at].kind(),
            ),
        };
        let mut instrument = Self::blank();
        if let Some(ids) = match &cells[SECURITYIDS] {
            Scalar::Null => None,
            other => Some(
                Identifiers::from_scalar(other)
                    .map_err(|error| relocated(NAMES[SECURITYIDS], error))?,
            ),
        } {
            instrument.securityids = ids;
        }
        // The projections fill what the map leaves unsaid.
        for (at, kind) in [
            (ISIN, IdType::Isin),
            (CFICODE, IdType::Cfi),
            (FOREXCODE, IdType::Forex),
            (FISN, IdType::Fisn),
        ] {
            if let Some(value) = cells[at].as_str()
                && !instrument.securityids.contains_kind(&kind)
            {
                instrument.securityids.set(
                    Identifier::new(IdKey::base(kind), value)
                        .map_err(|error| relocated(NAMES[at], error))?,
                );
            }
        }
        instrument.countrycode = match &cells[COUNTRYCODE] {
            Scalar::Country(code) => Some(code.clone()).filter(Country::is_listed),
            _ => None,
        };
        instrument.currency = match &cells[CURRENCY] {
            Scalar::Ccy(code) => Some(code.clone()).filter(|code| !code.is_none()),
            _ => None,
        };
        instrument.origccy = match &cells[ORIGCCY] {
            Scalar::Ccy(code) => Some(code.clone()).filter(|code| !code.is_none()),
            _ => None,
        };
        instrument.set_underlying(cells[UNDERLYING].as_str())?;
        if let Some(rows) = cells[LEGS].sequence_rows() {
            let legs = rows
                .iter()
                .map(|leg| {
                    let leg = Leg::dtype().required_field("leg").scalar(leg.clone())?;
                    let parts = leg.sequence_rows().ok_or_else(|| unread(LEGS))?;
                    let (Some(code), Some(Scalar::Int32(ratio))) =
                        (parts.first().and_then(Scalar::as_str), parts.get(1))
                    else {
                        return Err(unread(LEGS));
                    };
                    Leg::new(code, u32::try_from(ratio.get()).map_err(|_| unread(LEGS))?)
                })
                .collect::<Result<Vec<_>>>()?;
            instrument.set_legs(&legs)?;
        }
        instrument.eusipacode = match &cells[EUSIPACODE] {
            Scalar::Int32(code) => {
                product_category(code.get(), cells[CROSSCODE].as_str().unwrap_or(""))
            }
            _ => None,
        };
        instrument.characteristics = Characteristics::from_scalar(&cells[CHARACTERISTICS])?;
        if let Some(rows) = cells[LISTINGS].sequence_rows() {
            for listing in rows.iter() {
                let listing = Listing::from_scalar(listing)?;
                if !instrument.put_listing(listing) {
                    return Err(refused(
                        NAMES[LISTINGS],
                        format_smolstr!(
                            "expected at most {} listings, got more",
                            Self::MAX_LISTINGS
                        ),
                    ));
                }
            }
        }
        if let Some(entries) = cells[METADATA].as_mapping() {
            for (key, value) in entries {
                let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
                    return Err(unread(METADATA));
                };
                instrument.set_metadata(key, value)?;
            }
        }
        instrument.updunix = cells[UPDUNIX].temporal_count_at(TimeUnit::Nanosecond);
        instrument.firstunix = cells[FIRSTUNIX].temporal_count_at(TimeUnit::Nanosecond);
        instrument.lastunix = cells[LASTUNIX].temporal_count_at(TimeUnit::Nanosecond);
        if let Some(aliases) = cells[ALIASCODES].sequence_rows() {
            for alias in aliases.iter() {
                let Some(alias) = alias.as_str() else {
                    return Err(unread(ALIASCODES));
                };
                instrument.push_alias(alias)?;
            }
        }
        if let Some(sources) = cells[SRCUUIDS].sequence_rows() {
            instrument.set_srcuuids(
                sources
                    .iter()
                    .filter_map(|cell| match cell {
                        Scalar::Uuid(uuid) => Some(*uuid),
                        _ => None,
                    })
                    .collect(),
            );
        }
        let stated = cells[CROSSCODE].as_str().unwrap_or("");
        instrument.respell()?;
        if !stated.is_empty() && stated != instrument.get_crosscode() {
            return Err(refused(
                NAMES[CROSSCODE],
                format_smolstr!(
                    "expected the cross code the row's columns spell, {}, got {stated}",
                    instrument.get_crosscode()
                ),
            ));
        }
        Ok(instrument)
    }

    /// Keeps `alias`, a code this instrument had, sorted and unique.
    ///
    /// # Errors
    ///
    /// A new alias past [`Self::MAX_ALIASES`].
    fn push_alias(&mut self, alias: &str) -> Result<()> {
        if let Err(at) = self
            .aliascodes
            .binary_search_by(|held| held.as_str().cmp(alias))
        {
            if self.aliascodes.len() >= Self::MAX_ALIASES {
                return Err(refused(
                    NAMES[ALIASCODES],
                    format_smolstr!(
                        "expected at most {} aliases, got one more",
                        Self::MAX_ALIASES
                    ),
                ));
            }
            self.aliascodes.insert(at, Str::new(alias));
        }
        Ok(())
    }
}

/// The product category a row of `key` states as `code`, a number of no
/// category's shape dropped with one warning for the column.
fn product_category(code: i32, key: &str) -> Option<Eusipa> {
    let category = u16::try_from(code)
        .ok()
        .and_then(|code| Eusipa::new(code).ok());
    if category.is_none() {
        warned!(
            "instrument value dropped: it is no real code of its type",
            NAMES[EUSIPACODE],
            "{code} under {key}"
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

/// What one statement says about one instrument, borrowed where it lies,
/// keyed by the code its facts spell - written into the caller's slot - so
/// a statement of a held instrument that moves nothing builds nothing.
struct Statement<'s> {
    /// The key the facts spell, and how.
    code: &'s str,
    production: Production,
    /// A real ISIN stated, where the key is not it.
    isin: Option<&'s str>,
    cfi: Option<&'s str>,
    forex: Option<&'s str>,
    fisn: Option<&'s str>,
    countrycode: Option<&'s Country>,
    currency: Option<&'s Ccy>,
    origccy: Option<&'s Ccy>,
    underlying: Option<&'s str>,
    legs: &'s [Leg],
    characteristics: &'s Characteristics,
    eusipacode: Option<Eusipa>,
    miccode: Option<&'s Mic>,
    ticker: Option<&'s str>,
    /// The identifiers stated under their own keys - the security and
    /// listing codes but the four key types under their base keys, and
    /// every named source's statement of any type (`ullink:isin`,
    /// `bloomberg:figi`), which the map's base rule fills the base key from
    /// - each real for its type, never a derivation.
    ids: SmallVec<[(&'s IdKey, &'s str); Instrument::MAX_SECURITYIDS]>,
    /// The complementary facts stated, by key.
    metadata: SmallVec<[(&'s str, &'s str); Instrument::MAX_METADATA]>,
    aliascodes: &'s [Str],
    updunix: Option<i64>,
    firstunix: Option<i64>,
    lastunix: Option<i64>,
}

impl Instrument {
    /// This instrument as a statement: every code it holds that is real
    /// ([`IdType::is_real`]) - a typo or a masked number dropped with one
    /// warning per type, the number this crate minted for the key passed
    /// over silently, since the statement mints it again - its single
    /// listing's facts as the statement's listing facts, and every other
    /// listing folded after by [`Instruments::fold`]'s caller.
    fn statement(&self) -> Statement<'_> {
        let ids = self
            .securityids
            .iter()
            .chain(
                self.listings
                    .iter()
                    .flat_map(|listing| listing.codes().iter()),
            )
            .filter(|id| states_own_key(id))
            // The number this crate minted for the key is the instrument's
            // own derivation, minted again wherever the statement lands:
            // carried by no statement, warned of nowhere.
            .filter(|id| !(*id.key() == *MINT_KEY && Isin::is_minted(id.value(), self.mint_digest())))
            .filter(|id| {
                let real = id.kind().is_real(id.value());
                if !real {
                    warned!(
                        "instrument value dropped: it is no real code of its type",
                        id.kind().as_str(),
                        "{:?} under {}",
                        id.value(),
                        self.crosscode.as_str()
                    );
                }
                real
            })
            .map(|id| (id.key(), id.value()))
            .take(Self::MAX_SECURITYIDS)
            .collect();
        let listing = match self.listings.as_slice() {
            [one] => Some(one),
            _ => None,
        };
        let production = if self.placeholder {
            Production::Placeholder
        } else if self
            .isin()
            .is_some_and(|isin| isin == self.crosscode.as_str())
        {
            Production::Isin
        } else {
            Production::Body
        };
        Statement {
            code: self.crosscode.as_str(),
            production,
            isin: self.isin().filter(|isin| Isin::rank_of(isin) == 2),
            cfi: self.securityids.get(&IdType::Cfi),
            forex: self.securityids.get(&IdType::Forex),
            fisn: self.securityids.get(&IdType::Fisn),
            countrycode: self.countrycode.as_ref(),
            // A security's currency is its listing's; a body's its own.
            currency: if production == Production::Body {
                self.currency.as_ref()
            } else {
                listing.and_then(Listing::currency)
            },
            origccy: self.origccy.as_ref(),
            underlying: self.underlying(),
            legs: &self.legs,
            characteristics: &self.characteristics,
            eusipacode: self.eusipacode,
            miccode: listing.and_then(Listing::miccode),
            ticker: listing.and_then(Listing::ticker),
            ids,
            metadata: self
                .metadata
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect(),
            aliascodes: &self.aliascodes,
            updunix: self.updunix,
            firstunix: self.firstunix,
            lastunix: self.lastunix,
        }
    }

    /// The instrument a statement makes on its own: what it states, nothing
    /// derived, keyed and minted where the key is a body.
    fn from_statement(statement: &Statement<'_>) -> Result<Self> {
        let mut instrument = Self::blank();
        // One vector, reserved once for every entry the statement lands - a
        // named key fills its base key too - and the number a body mints.
        instrument.securityids = Identifiers::with_capacity(
            (5 + 2 * statement.ids.len()).min(Self::MAX_SECURITYIDS + 1),
        );
        if let Some(isin) = statement.isin {
            instrument
                .securityids
                .set(Identifier::new(IdKey::base(IdType::Isin), isin)?);
        }
        if let Some(cfi) = statement.cfi {
            instrument
                .securityids
                .set(Identifier::new(IdKey::base(IdType::Cfi), cfi)?);
        }
        if let Some(forex) = statement.forex {
            instrument
                .securityids
                .set(Identifier::new(IdKey::base(IdType::Forex), forex)?);
        }
        if let Some(fisn) = statement.fisn {
            instrument
                .securityids
                .set(Identifier::new(IdKey::base(IdType::Fisn), fisn)?);
        }
        instrument.countrycode = statement.countrycode.cloned().filter(|code| {
            statement
                .isin
                .is_none_or(|isin| &isin[..2] != code.as_str())
        });
        instrument.currency = statement
            .currency
            .filter(|_| statement.production == Production::Body)
            .cloned();
        instrument.origccy = statement.origccy.cloned();
        instrument.set_underlying(statement.underlying)?;
        instrument.set_legs(statement.legs)?;
        instrument.characteristics = statement.characteristics.clone();
        instrument.eusipacode = statement.eusipacode;
        instrument.updunix = statement.updunix;
        instrument.firstunix = statement.firstunix;
        instrument.lastunix = statement.lastunix;
        for alias in statement.aliascodes {
            instrument.push_alias(alias)?;
        }
        let listing_currency = statement
            .currency
            .filter(|_| statement.production != Production::Body);
        if statement.miccode.is_some() || statement.ticker.is_some() || listing_currency.is_some() {
            let mut listing = Listing::new(statement.miccode.cloned());
            listing.set_ticker(statement.ticker);
            listing.set_currency(listing_currency.cloned());
            instrument.put_listing(listing);
        }
        for (key, value) in &statement.ids {
            let Ok(id) = Identifier::new((*key).clone(), value) else {
                continue;
            };
            instrument.adopt_code(id);
        }
        for (key, value) in &statement.metadata {
            if instrument.accepts_metadata(key) {
                instrument
                    .metadata
                    .insert(SmolStr::new(key), SmolStr::new(value));
            } else {
                warn_metadata_full(key, statement.code);
            }
        }
        instrument.respell()?;
        Ok(instrument)
    }

    /// Holds `id` where a fold lands it: a listing code onto the one
    /// listing where there is one, else among the identifiers until a
    /// market is learned; any other among the identifiers. A new type past
    /// a bound is passed over.
    fn adopt_code(&mut self, id: Identifier) {
        if id.kind().is_listing()
            && let [one] = self.listings.as_mut_slice()
        {
            one.adopt(id);
            return;
        }
        self.adopt_equivalent(id);
    }
}

/// Whether `id` is a statement an instrument carries under its own key: a
/// named source's statement of any type, or a base key of a type that is
/// not one of the four key types - the ISIN, the CFI, the pair and the
/// short name, which have statement fields of their own - and never a
/// derivation.
fn states_own_key(id: &Identifier) -> bool {
    match id.key().src() {
        IdSource::Derived => false,
        IdSource::Base => !matches!(
            id.kind(),
            IdType::Isin | IdType::Cfi | IdType::Forex | IdType::Fisn
        ),
        _ => true,
    }
}

/// Warns, once per key, of a metadata entry passed over past
/// [`Instrument::MAX_METADATA`].
fn warn_metadata_full(key: &str, code: &str) {
    warned!(
        "instrument metadata dropped: the instrument holds its most entries",
        NAMES[METADATA],
        "{key:?} under {code}"
    );
}

impl Instrument {
    /// Whether the listing facts of `statement` land on a listing of this
    /// instrument, and which: the one on its market, created where none,
    /// the single one where it names no market - none of several.
    fn listing_target(&self, statement: &Statement<'_>) -> Target {
        match statement.miccode {
            Some(market) => match self.listing_at(Some(market)) {
                Ok(at) => Target::Listing(at),
                Err(_) if matches!(self.listings.as_slice(), [one] if one.miccode().is_none()) => {
                    Target::Listing(0)
                }
                Err(at) => Target::New(at),
            },
            None => match self.listings.len() {
                0 => Target::Unlisted,
                1 => Target::Listing(0),
                _ => Target::Withheld,
            },
        }
    }
}

/// Where the listing facts of a statement land among an instrument's
/// listings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    /// The listing at this place.
    Listing(usize),
    /// A new listing on the statement's market, at this place.
    New(usize),
    /// No listing yet: an unlisted one, where the statement states a
    /// listing fact.
    Unlisted,
    /// None: the statement names no market and the instrument has several
    /// listings.
    Withheld,
}

impl Statement<'_> {
    /// Warns, once per column, of the ticker and the currency it states
    /// where it names no market and the instrument has several listings.
    fn warn_withheld(&self) {
        let withheld = |column: &str, value: &str| {
            warned!(
                "instrument listing fact dropped: the statement names no market of the instrument's several listings",
                column,
                "{value:?} under {}",
                self.code
            );
        };
        if let Some(ticker) = self.ticker {
            withheld("ticker", ticker);
        }
        if let Some(currency) = self.currency
            && self.production != Production::Body
        {
            withheld("currency", currency.as_str());
        }
    }

    /// Whether the statement spells a body a placeholder keyed by its ISIN
    /// is waiting for: the re-key.
    fn rekeys(&self, held: &Instrument) -> bool {
        held.placeholder
            && self.production == Production::Body
            && self.isin == Some(held.crosscode.as_str())
    }
}

/// `held` with `statement` folded in by the update rule, or `None` where
/// nothing moves: a stated value fills a fact the instrument lacks and
/// replaces one it holds that differs; a CFI compatible with the held one
/// refines it and a contradicting one replaces it; a real ISIN replaces a
/// minted one and never the reverse; the listing facts land where
/// [`Instrument::listing_target`] says. The key facts never move: two
/// statements under one code spell them alike. The stamps are the
/// caller's, and nothing is derived here.
fn folded(held: &Instrument, statement: &Statement<'_>) -> Option<Instrument> {
    let mut next = Cow::Borrowed(held);
    let put = |next: &mut Cow<'_, Instrument>, key: IdKey, value: &str| {
        if next.securityids.get_from(&key) != Some(value)
            && let Ok(id) = Identifier::new(key, value)
        {
            next.to_mut().securityids.insert(id);
        }
    };
    if let Some(isin) = statement.isin {
        put(&mut next, IdKey::base(IdType::Isin), isin);
    }
    if let Some(forex) = statement.forex {
        put(&mut next, IdKey::base(IdType::Forex), forex);
    }
    if let Some(fisn) = statement.fisn
        && held.securityids.get(&IdType::Fisn) != Some(fisn)
        && let Ok(id) = Identifier::new(IdKey::base(IdType::Fisn), fisn)
    {
        next.to_mut().securityids.set(id);
    }
    if let Some(stated) = statement.cfi {
        let refined = match held.securityids.get(&IdType::Cfi) {
            None => Some(SmolStr::new(stated)),
            Some(held) if held == stated => None,
            Some(held) => match Cfi::refined(held, stated) {
                // Filling an `X` contradicts nothing.
                Some(refined) if refined != held => Some(refined),
                Some(_) => None,
                None => Some(SmolStr::new(stated)),
            },
        };
        if let Some(code) = refined
            && let Ok(id) = Identifier::new(IdKey::base(IdType::Cfi), &code)
        {
            next.to_mut().securityids.set(id);
        }
    }
    if let Some(stated) = statement.countrycode {
        // The ISIN's own prefix states nothing the key does not: it takes a
        // differing held country back rather than being held beside it.
        let countrycode = Some(stated.clone()).filter(|code| {
            next.isin()
                .filter(|isin| Isin::rank_of(isin) == 2)
                .is_none_or(|isin| &isin[..2] != code.as_str())
        });
        if held.countrycode != countrycode {
            next.to_mut().countrycode = countrycode;
        }
    }
    if let Some(stated) = statement.origccy
        && held.origccy.as_ref() != Some(stated)
    {
        next.to_mut().origccy = Some(stated.clone());
    }
    if statement.production == Production::Body
        && let Some(stated) = statement.currency
        && held.currency.as_ref() != Some(stated)
    {
        next.to_mut().currency = Some(stated.clone());
    }
    if let Some(stated) = statement.eusipacode
        && held.eusipacode != Some(stated)
    {
        next.to_mut().eusipacode = Some(stated);
    }
    // The underlying and the legs are key facts of a derivative, which the
    // code already settles; a security's underlying is a fact.
    if statement.production != Production::Body
        && let Some(stated) = statement.underlying
        && held.underlying() != Some(stated)
        && stated != held.crosscode.as_str()
    {
        next.to_mut().underlying = Some(Str::new(stated));
    }
    if statement.characteristics.multiplier().is_some()
        && held.characteristics.multiplier() != statement.characteristics.multiplier()
    {
        let characteristics = next
            .characteristics
            .clone()
            .with_multiplier(statement.characteristics.multiplier());
        next.to_mut().characteristics = characteristics;
    }
    if statement.characteristics.exercise().is_some()
        && held.characteristics.exercise() != statement.characteristics.exercise()
    {
        let characteristics = next
            .characteristics
            .clone()
            .with_exercise(statement.characteristics.exercise());
        next.to_mut().characteristics = characteristics;
    }
    for alias in statement.aliascodes {
        if held
            .aliascodes
            .iter()
            .all(|code| code.as_str() != alias.as_str())
            && next.aliascodes.len() < Instrument::MAX_ALIASES
        {
            let _ = next.to_mut().push_alias(alias);
        }
    }
    // The identifiers, each under its own key: a listing code lands on the
    // listing the statement's market names, among the identifiers where it
    // names none and the instrument holds no listing; every other among
    // the identifiers.
    let target = held.listing_target(statement);
    for (key, value) in &statement.ids {
        let kind = key.kind();
        if kind.is_listing() {
            match target {
                Target::Listing(at) => {
                    if held.listings[at].codes().get_from(key) != Some(value)
                        && held.listings[at].accepts(kind)
                        && let Ok(id) = Identifier::new((*key).clone(), value)
                    {
                        let listing = &mut next.to_mut().listings[at];
                        listing.adopt(id);
                    }
                    // One holder at any instant: a code of the type held on no
                    // market while the instrument was listed nowhere moves
                    // onto the listing it is stated on.
                    if held.securityids.contains_kind(kind)
                        && next.listings[at].codes().contains_kind(kind)
                    {
                        next.to_mut().securityids.remove(&IdKey::base(kind.clone()));
                    }
                }
                Target::New(_) => {}
                Target::Unlisted | Target::Withheld => {
                    if held.securityids.get_from(key) != Some(value)
                        && next.accepts_equivalent(key)
                        && let Ok(id) = Identifier::new((*key).clone(), value)
                    {
                        next.to_mut().adopt_equivalent(id);
                    }
                }
            }
        } else if held.securityids.get_from(key) != Some(value)
            && next.accepts_equivalent(key)
            && let Ok(id) = Identifier::new((*key).clone(), value)
        {
            next.to_mut().adopt_equivalent(id);
        }
    }
    // The complementary facts: a stated value fills an empty key and
    // replaces a differing one; a new key past the bound is passed over.
    for (key, value) in &statement.metadata {
        if held.metadata.get(*key).is_some_and(|held| held == value) {
            continue;
        }
        if next.accepts_metadata(key) {
            next.to_mut()
                .metadata
                .insert(SmolStr::new(key), SmolStr::new(value));
        } else {
            warn_metadata_full(key, statement.code);
        }
    }
    match target {
        Target::Listing(at) => {
            let listing = &held.listings[at];
            if let Some(stated) = statement.ticker
                && listing.ticker() != Listing::trimmed_ticker(stated)
            {
                next.to_mut().listings[at].set_ticker(Some(stated));
            }
            if statement.production != Production::Body
                && let Some(stated) = statement.currency
                && listing.currency() != Some(stated)
            {
                next.to_mut().listings[at].set_currency(Some(stated.clone()));
            }
        }
        Target::New(_) | Target::Unlisted => {
            if statement.miccode.is_some()
                || statement.ticker.is_some()
                || statement.currency.is_some()
            {
                let mut listing = Listing::new(statement.miccode.cloned());
                listing.set_ticker(statement.ticker);
                if statement.production != Production::Body {
                    listing.set_currency(statement.currency.cloned());
                }
                for (key, value) in statement
                    .ids
                    .iter()
                    .filter(|(key, _)| key.kind().is_listing())
                {
                    if let Ok(id) = Identifier::new((*key).clone(), value) {
                        listing.adopt(id);
                    }
                }
                if listing.miccode().is_some()
                    || listing.ticker().is_some()
                    || listing.currency().is_some()
                {
                    next.to_mut().put_listing(listing);
                }
            }
        }
        Target::Withheld => statement.warn_withheld(),
    }
    match next {
        Cow::Borrowed(_) => None,
        Cow::Owned(instrument) => Some(instrument),
    }
}

/// What one learn answered.
#[derive(Clone, Copy, Debug, Default)]
pub struct Learned {
    /// Whether an instrument moved.
    pub moved: bool,
    /// The bound a new instrument was passed over at, the first time this
    /// collection passes one over: the caller's to warn of, after it has
    /// let go of any lock it holds the collection under.
    pub full: Option<usize>,
}

/// Warns, once per site, that a collection bounded at `max` instruments is
/// full and learns no new instrument.
pub fn warn_full(max: usize) {
    warned!(
        "instruments full",
        "instruments",
        "{max} instruments, new instruments are not learned"
    );
}

/// A key of the code index: a type of [`Instruments::LOOKUP_CODES`] and a
/// code of it as held.
#[derive(Clone, Debug)]
struct CodeKey(IdType, SmolStr);

/// A code index key read as its type and its text, so a lookup borrows
/// both rather than building a key.
trait CodeLookup {
    fn kind(&self) -> &IdType;
    fn code(&self) -> &str;
}

impl CodeLookup for CodeKey {
    fn kind(&self) -> &IdType {
        &self.0
    }

    fn code(&self) -> &str {
        self.1.as_str()
    }
}

impl CodeLookup for (&IdType, &str) {
    fn kind(&self) -> &IdType {
        self.0
    }

    fn code(&self) -> &str {
        self.1
    }
}

impl PartialEq for dyn CodeLookup + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind() && self.code() == other.code()
    }
}

impl Eq for dyn CodeLookup + '_ {}

impl Hash for dyn CodeLookup + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind().hash(state);
        self.code().hash(state);
    }
}

impl PartialEq for CodeKey {
    fn eq(&self, other: &Self) -> bool {
        (self as &dyn CodeLookup) == (other as &dyn CodeLookup)
    }
}

impl Eq for CodeKey {}

impl Hash for CodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self as &dyn CodeLookup).hash(state);
    }
}

impl<'a> Borrow<dyn CodeLookup + 'a> for CodeKey {
    fn borrow(&self) -> &(dyn CodeLookup + 'a) {
        self
    }
}

/// One key an instrument is indexed under.
#[derive(Clone, Copy)]
enum IndexKey<'a> {
    /// A listing's ticker.
    Ticker(&'a str),
    /// A code of a type [`Instruments::LOOKUP_CODES`] names.
    Code(&'a IdType, &'a str),
    /// An ISIN held - real, or minted.
    Isin(&'a str),
}

impl IndexKey<'_> {
    /// Every key `instrument` is indexed under.
    fn of(instrument: &Instrument) -> impl Iterator<Item = IndexKey<'_>> {
        let isins = instrument
            .securityids
            .of_kind(&IdType::Isin)
            .map(|id| IndexKey::Isin(id.value()));
        let codes = instrument
            .securityids
            .iter()
            .filter(|id| id.key().is_base() && is_lookup(id.kind()))
            .map(|id| IndexKey::Code(id.kind(), id.value()));
        let listings = instrument.listings.iter().flat_map(|listing| {
            listing.ticker().map(IndexKey::Ticker).into_iter().chain(
                listing
                    .codes()
                    .iter()
                    .filter(|id| id.key().is_base() && is_lookup(id.kind()))
                    .map(|id| IndexKey::Code(id.kind(), id.value())),
            )
        });
        isins.chain(codes).chain(listings)
    }

    /// Whether `instrument` is indexed under this key.
    fn held_by(self, instrument: &Instrument) -> bool {
        IndexKey::of(instrument).any(|key| same_key(key, self))
    }
}

/// Whether two index keys are one key.
fn same_key(left: IndexKey<'_>, right: IndexKey<'_>) -> bool {
    match (left, right) {
        (IndexKey::Ticker(held), IndexKey::Ticker(this)) => held == this,
        (IndexKey::Code(held, code), IndexKey::Code(this, other)) => held == this && code == other,
        (IndexKey::Isin(held), IndexKey::Isin(this)) => held == this,
        _ => false,
    }
}

/// Whether a lookup reads codes of `kind` ([`Instruments::LOOKUP_CODES`]).
fn is_lookup(kind: &IdType) -> bool {
    Instruments::LOOKUP_CODES.contains(kind)
}

/// The indexes of a table, borrowed to be kept on its instruments.
struct Indexes<'t> {
    isins: &'t mut HashMap<i128, Str>,
    tickers: &'t mut HashMap<SmolStr, CodeSlots>,
    codes: &'t mut HashMap<CodeKey, CodeSlots>,
}

impl Indexes<'_> {
    /// Lists `code` under `key`, once.
    fn list(&mut self, key: IndexKey<'_>, code: &Str) {
        let slots = match key {
            IndexKey::Isin(isin) => {
                self.isins
                    .entry(packed(isin))
                    .or_insert_with(|| code.clone());
                return;
            }
            IndexKey::Ticker(ticker) => self.tickers.entry(SmolStr::new(ticker)).or_default(),
            IndexKey::Code(kind, text) => self
                .codes
                .entry(CodeKey(kind.clone(), SmolStr::new(text)))
                .or_default(),
        };
        if let Err(at) = slots.binary_search(code) {
            slots.insert(at, code.clone());
        }
    }

    /// Drops `code` from under `key`, and the key with it once nothing is
    /// listed under it.
    fn unlist(&mut self, key: IndexKey<'_>, code: &str) {
        let emptied = match key {
            IndexKey::Isin(isin) => {
                if self
                    .isins
                    .get(&packed(isin))
                    .is_some_and(|held| held.as_str() == code)
                {
                    self.isins.remove(&packed(isin));
                }
                return;
            }
            IndexKey::Ticker(ticker) => self.tickers.get_mut(ticker).map(|slots| {
                slots.retain(|held| held.as_str() != code);
                slots.is_empty()
            }),
            IndexKey::Code(kind, text) => {
                self.codes
                    .get_mut(&(kind, text) as &dyn CodeLookup)
                    .map(|slots| {
                        slots.retain(|held| held.as_str() != code);
                        slots.is_empty()
                    })
            }
        };
        if emptied == Some(true) {
            match key {
                IndexKey::Ticker(ticker) => {
                    self.tickers.remove(ticker);
                }
                IndexKey::Code(kind, text) => {
                    self.codes.remove(&(kind, text) as &dyn CodeLookup);
                }
                IndexKey::Isin(_) => {}
            }
        }
    }

    /// Lists every key of `instrument` under `code`.
    fn list_all(&mut self, instrument: &Instrument, code: &Str) {
        for key in IndexKey::of(instrument) {
            self.list(key, code);
        }
    }

    /// Keeps the indexes on an instrument that was `held` and is now
    /// `next`, both under `code`.
    fn moved(&mut self, code: &Str, held: &Instrument, next: &Instrument) {
        for key in IndexKey::of(held) {
            if !key.held_by(next) {
                self.unlist(key, code);
            }
        }
        for key in IndexKey::of(next) {
            if !key.held_by(held) {
                self.list(key, code);
            }
        }
    }

    /// Drops every key of `removed`, an instrument no longer held.
    fn removed(&mut self, code: &str, removed: &Instrument) {
        for key in IndexKey::of(removed) {
            self.unlist(key, code);
        }
    }
}

/// The cross codes a key names, none, one or several.
enum Found<'t> {
    None,
    One(&'t Str),
    Several(Vec<Str>),
}

impl<'t> Found<'t> {
    /// The codes `codes` answers.
    fn of(mut codes: impl Iterator<Item = &'t Str>) -> Self {
        let Some(first) = codes.next() else {
            return Self::None;
        };
        let Some(second) = codes.next() else {
            return Self::One(first);
        };
        Self::Several([first, second].into_iter().chain(codes).cloned().collect())
    }

    /// The one code, none where it names none or several.
    fn one(self) -> Option<&'t Str> {
        match self {
            Self::One(code) => Some(code),
            Self::None | Self::Several(_) => None,
        }
    }
}

/// The tier of [`Instruments::resolve`] an element was matched by.
#[derive(Clone, Debug, PartialEq)]
pub enum MatchTier {
    /// Its own ISIN: a real one, or the number this crate minted.
    Isin,
    /// The cross code its own facts spell - an FX pair's, a derivative's.
    CrossCode,
    /// A code of this type, one of [`Instruments::LOOKUP_CODES`].
    Code(IdType),
    /// Its ticker on its market.
    Symbology,
    /// Its short name in its currency, this similar to the instrument's
    /// ([`Fisn::similarity`]).
    Economic {
        /// How similar the two short names are, from the threshold to `1`.
        similarity: f64,
    },
}

/// What [`Instruments::resolve`] answers for one element: the instrument it
/// names and how, or why none.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution<'a> {
    /// The instrument the element names.
    Matched {
        /// The instrument.
        entry: &'a Instrument,
        /// The tier that found it.
        tier: MatchTier,
        /// The ISIN was derived - the element stated none.
        derived: bool,
        /// The instrument's listing on the element's market is the
        /// element's: its market is listed, or it states none and the
        /// instrument has one listing.
        listing: bool,
    },
    /// None, and why.
    Unmatched(Unmatched),
}

/// Why [`Instruments::resolve`] matched no instrument.
#[derive(Clone, Debug, PartialEq)]
pub enum Unmatched {
    /// The element states nothing any tier reads.
    NoKey,
    /// A real ISIN the collection does not hold: the cascade ends here,
    /// since another instrument's facts would fill around the one it
    /// states.
    UnknownIsin {
        /// The ISIN the element states.
        stated: Isin,
    },
    /// A cross code the element's facts spell that the collection does not
    /// hold: the cascade ends here too.
    UnknownCode {
        /// The code the facts spell.
        stated: Str,
    },
    /// Every tier looked and found none.
    NoCandidate,
    /// An exact key two instruments hold, or two equal best economic
    /// candidates: a defect to name, never a reason to pick.
    Ambiguous {
        /// The tier that found them.
        tier: MatchTier,
        /// Their cross codes, in code order.
        codes: Vec<SmolStr>,
    },
    /// The most similar instrument is of another CFI category.
    CfiConflict {
        /// The element's category.
        stated: char,
        /// The instrument's.
        held: char,
        /// The instrument's cross code.
        code: SmolStr,
    },
    /// The most similar instrument states another origin currency.
    CurrencyConflict {
        /// The element's origin currency.
        stated: Ccy,
        /// The instrument's.
        held: Ccy,
        /// The instrument's cross code.
        code: SmolStr,
    },
    /// The most similar instrument is less similar than the threshold.
    BelowThreshold {
        /// How similar it is.
        best: f64,
        /// The instrument's cross code.
        code: SmolStr,
    },
}

/// What one economic scan answers: the code and its similarity, or why none.
type Scored<'t> = std::result::Result<(&'t Str, f64), Unmatched>;

/// The economic match's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Economic {
    threshold: f64,
    enabled: bool,
}

impl Default for Economic {
    fn default() -> Self {
        Self {
            threshold: Instruments::DEFAULT_ECONOMIC_THRESHOLD,
            enabled: false,
        }
    }
}

/// What one walk's economic matches answered, by what the scan reads of an
/// element, and the table generation they were answered under.
#[derive(Debug, Default)]
pub struct EconomicMemo {
    generation: u64,
    answers: HashMap<EconomicReading, Option<Str>>,
}

/// What the economic scan reads of an element: its short name, its
/// currency, its origin currency and its CFI category.
type EconomicReading = (SmolStr, Ccy, Option<Ccy>, Option<char>);

/// The generation every table move takes: one counter for the process.
static GENERATIONS: AtomicU64 = AtomicU64::new(0);

/// A generation no table held before.
fn next_generation() -> u64 {
    GENERATIONS.fetch_add(1, AtomicOrdering::Relaxed) + 1
}

/// The CFI category `code` states: its first letter where ISO 10962 names
/// it, none for an unclassified `X`.
fn cfi_category(code: Option<&str>) -> Option<char> {
    let letter = code?.chars().next()?;
    Cfi::category_of(letter).map(|_| letter)
}

/// What a FIX message states of its instrument beside its market facts:
/// what only a message spells, handed to the collection's `learn_stating`
/// and the table's [`InstrumentTable::fill_unsettled`] by the lifecycle
/// through [`implementer`](crate::implementer). Every field is optional;
/// the default states nothing beyond the element.
#[derive(Clone, Debug, Default)]
pub struct Stated<'s> {
    /// The origin currency the message states, read before a fill wrote one.
    pub origccy: Option<&'s Ccy>,
    /// The country of issue the message states that its ISIN does not say.
    pub country: Option<&'s Country>,
    /// The ISIN of the instrument this one is written on, resolved to that
    /// instrument's cross code through the table; one no instrument keys
    /// gives a derivative no body.
    pub underlying: Option<&'s Isin>,
    /// The EUSIPA product category a bridge's key states.
    pub product: Option<Eusipa>,
    /// The body facts the message spells: the pair where it is stated
    /// otherwise than as a `forex` identifier, the characteristics and the
    /// legs.
    pub body: Option<&'s Body>,
    /// The complementary facts the message states of its instrument that
    /// no typed fact holds, by key ([`Instrument::metadata`]): a FIX
    /// message's instrument description - its issuer, its security
    /// description, its security type - each key held once on the
    /// instrument.
    pub metadata: &'s [(&'s str, &'s str)],
}

/// The body facts of a `class:body` instrument a message spells, as intake.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    /// The pair, where stated beside the element's identifiers.
    pub forex: Option<Forex>,
    /// The typed characteristics.
    pub characteristics: Characteristics,
    /// A strategy's legs.
    pub legs: Vec<Leg>,
}

/// The table a collection holds, shared: the instruments by cross code and
/// the indexes over them - the ISINs, the aliases, the tickers and the
/// lookup codes - counted pointers that cross threads as they are, beside
/// the economic match's settings. What a parse door fixes once and every
/// worker fills from, and what a lifecycle fills from under the
/// collection's lock.
#[derive(Clone, Debug)]
pub struct InstrumentTable {
    /// Every instrument, by its cross code, in code order.
    rows: Arc<BTreeMap<Str, Instrument>>,
    /// Each ISIN held - real or minted, packed - to the code holding it.
    isins: Arc<HashMap<i128, Str>>,
    /// Each code an instrument had before a re-key to the code it has.
    aliases: Arc<HashMap<Str, Str>>,
    /// Each ticker to the codes a listing of which states it.
    tickers: Arc<HashMap<SmolStr, CodeSlots>>,
    /// Each lookup code to the codes holding it.
    codes: Arc<HashMap<CodeKey, CodeSlots>>,
    /// The economic match's settings.
    economic: Economic,
    /// What moved the instruments last: a fact, an instrument - never a
    /// stamp alone - takes a new one ([`EconomicMemo`]).
    generation: u64,
}

static EMPTY_ROWS: LazyLock<Arc<BTreeMap<Str, Instrument>>> = LazyLock::new(Arc::default);
static EMPTY_ISINS: LazyLock<Arc<HashMap<i128, Str>>> = LazyLock::new(Arc::default);
static EMPTY_ALIASES: LazyLock<Arc<HashMap<Str, Str>>> = LazyLock::new(Arc::default);
static EMPTY_TICKERS: LazyLock<Arc<HashMap<SmolStr, CodeSlots>>> = LazyLock::new(Arc::default);
static EMPTY_CODES: LazyLock<Arc<HashMap<CodeKey, CodeSlots>>> = LazyLock::new(Arc::default);

impl Default for InstrumentTable {
    fn default() -> Self {
        Self {
            rows: Arc::clone(&EMPTY_ROWS),
            isins: Arc::clone(&EMPTY_ISINS),
            aliases: Arc::clone(&EMPTY_ALIASES),
            tickers: Arc::clone(&EMPTY_TICKERS),
            codes: Arc::clone(&EMPTY_CODES),
            economic: Economic::default(),
            generation: next_generation(),
        }
    }
}

/// How the tier-one cascade ended.
enum Exact<'t> {
    Ended(Resolution<'t>),
    Open { keyed: bool },
}

impl InstrumentTable {
    /// An empty table keeping these settings.
    pub(crate) fn emptied(&self) -> Self {
        Self {
            economic: self.economic,
            ..Self::default()
        }
    }

    /// The indexes, mutable, and the instruments beside them.
    fn indexes(&mut self) -> (Indexes<'_>, &mut BTreeMap<Str, Instrument>) {
        (
            Indexes {
                isins: Arc::make_mut(&mut self.isins),
                tickers: Arc::make_mut(&mut self.tickers),
                codes: Arc::make_mut(&mut self.codes),
            },
            Arc::make_mut(&mut self.rows),
        )
    }

    /// The instruments moved: a memo answered before answers no more.
    fn moved(&mut self) {
        self.generation = next_generation();
    }

    /// The code `key` resolves to: a cross code held, an alias's survivor,
    /// or the code holding `key` as an ISIN.
    fn code_of(&self, key: &str) -> Option<&Str> {
        if let Some((code, _)) = self.rows.get_key_value(key) {
            return Some(code);
        }
        if let Some(code) = self.aliases.get(key) {
            return Some(code);
        }
        if Isin::is_canonical(key) {
            return self.isins.get(&packed(key));
        }
        None
    }

    /// The instrument `key` names: by its cross code, an alias or an ISIN
    /// it holds.
    pub(crate) fn get(&self, key: &str) -> Option<&Instrument> {
        self.rows.get(self.code_of(key)?.as_str())
    }

    /// The instrument whose identity, or whose former identity, is `uuid`.
    pub(crate) fn get_by_uuid(&self, uuid: Uuid) -> Option<&Instrument> {
        self.rows
            .values()
            .find(|instrument| instrument.uuid == uuid)
            .or_else(|| {
                self.aliases
                    .iter()
                    .find(|(alias, _)| cross_uuid_of(alias) == uuid)
                    .and_then(|(_, code)| self.rows.get(code.as_str()))
            })
    }

    /// The codes a listing of which states `ticker` - trimmed - on
    /// `market`: a listing there, else - none does - on no market; where
    /// `market` is unstated, any.
    fn codes_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Found<'_> {
        let ticker = ticker.trim();
        let Some(slots) = self.tickers.get(ticker) else {
            return Found::None;
        };
        let Some(market) = market.filter(|code| !code.is_none()) else {
            return Found::of(slots.iter());
        };
        let states = |code: &&Str, on: Option<&Mic>| {
            self.rows.get(code.as_str()).is_some_and(|instrument| {
                instrument
                    .listings
                    .iter()
                    .any(|listing| listing.ticker() == Some(ticker) && listing.miccode() == on)
            })
        };
        if slots.iter().any(|code| states(&code, Some(market))) {
            Found::of(slots.iter().filter(|code| states(code, Some(market))))
        } else {
            Found::of(slots.iter().filter(|code| states(code, None)))
        }
    }

    /// The codes holding `code` under `kind`, as the type stores it.
    fn codes_by_code(&self, kind: &IdType, code: &str) -> Found<'_> {
        self.codes
            .get(&(kind, code) as &dyn CodeLookup)
            .map_or(Found::None, |slots| Found::of(slots.iter()))
    }

    /// The instrument the ticker `ticker` names on `market`.
    pub(crate) fn get_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Option<&Instrument> {
        let market = market.filter(|code| !code.is_none());
        let code = self.codes_by_ticker(ticker, market).one()?;
        self.rows.get(code.as_str())
    }

    /// The instrument the code `code` of `kind` names, `code` already as
    /// its type stores it.
    fn get_by_code(&self, kind: &IdType, code: &str) -> Option<&Instrument> {
        let code = self.codes_by_code(kind, code).one()?;
        self.rows.get(code.as_str())
    }

    /// How many instruments it holds.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether it holds none.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Every instrument, in code order.
    fn iter(&self) -> impl Iterator<Item = &Instrument> {
        self.rows.values()
    }

    /// Whether `element`'s market is one `instrument` is listed on - or it
    /// states none and the instrument has one listing.
    fn listed<E: Market + ?Sized>(instrument: &Instrument, element: &E) -> bool {
        match element.get_miccode().filter(|code| !code.is_none()) {
            Some(market) => instrument.listing(Some(market)).is_some(),
            None => instrument.listings.len() == 1,
        }
    }

    /// The listing of `instrument` an element on `market` reads: the one
    /// there, the single one where it names none.
    fn listing_for<'i>(instrument: &'i Instrument, market: Option<&Mic>) -> Option<&'i Listing> {
        match market.filter(|code| !code.is_none()) {
            Some(market) => instrument.listing(Some(market)),
            None => match instrument.listings.as_slice() {
                [one] => Some(one),
                _ => None,
            },
        }
    }

    /// A match of `instrument`, found by `tier`, for `element`.
    fn matched<'t, E: Market + ?Sized>(
        element: &E,
        instrument: &'t Instrument,
        tier: MatchTier,
        derived: bool,
    ) -> Resolution<'t> {
        Resolution::Matched {
            entry: instrument,
            tier,
            derived,
            listing: Self::listed(instrument, element),
        }
    }

    /// The cross code `element`'s own facts spell, with the body `stated`
    /// hands over, written into `slot`; the underlying's code the one its
    /// stated ISIN resolves to here.
    fn spelled<'s, E: Market + ?Sized>(
        &self,
        element: &E,
        stated: &Stated<'_>,
        slot: &'s mut [u8; MAX_CODE_WIDTH],
    ) -> Result<Option<(&'s str, Production)>> {
        let underlying = stated
            .underlying
            .and_then(|isin| self.isins.get(&packed(isin.as_str())))
            .map(Str::as_str);
        spell_code(element, stated.body, underlying, slot)
    }

    /// The exact tier: a real ISIN `element` holds decides alone, a miss
    /// ending the cascade; else the code its facts spell, a miss ending it
    /// too; else an ISIN this crate minted that it holds; else each code of
    /// [`Instruments::LOOKUP_CODES`], in that order, then its ticker on its
    /// market, the first naming one instrument matching and the first
    /// naming several ending the cascade.
    fn exact<E: Market + ?Sized>(&self, element: &E, stated: &Stated<'_>) -> Exact<'_> {
        let ids = element.get_securityids();
        let market = element.get_miccode().filter(|code| !code.is_none());
        if let Some(isin) = ids
            .get(&IdType::Isin)
            .filter(|isin| IdType::Isin.is_real(isin))
        {
            return Exact::Ended(match self.isins.get(&packed(isin)) {
                Some(code) => {
                    Self::matched(element, &self.rows[code.as_str()], MatchTier::Isin, false)
                }
                None => Resolution::Unmatched(Unmatched::UnknownIsin {
                    stated: yggdryl::implementer::isin_from_proven(isin),
                }),
            });
        }
        let mut slot = [0_u8; MAX_CODE_WIDTH];
        if let Ok(Some((code, Production::Body))) = self.spelled(element, stated, &mut slot) {
            return Exact::Ended(match self.code_of(code) {
                Some(held) => Self::matched(
                    element,
                    &self.rows[held.as_str()],
                    MatchTier::CrossCode,
                    true,
                ),
                None => Resolution::Unmatched(Unmatched::UnknownCode {
                    stated: Str::new(code),
                }),
            });
        }
        let mut keyed = false;
        if let Some(isin) = ids
            .get(&IdType::Isin)
            .filter(|isin| Isin::is_canonical(isin))
        {
            keyed = true;
            if let Some(code) = self.isins.get(&packed(isin)) {
                return Exact::Ended(Self::matched(
                    element,
                    &self.rows[code.as_str()],
                    MatchTier::Isin,
                    false,
                ));
            }
        }
        for kind in &Instruments::LOOKUP_CODES {
            let Some(code) = ids.get(kind) else {
                continue;
            };
            keyed = true;
            match self.codes_by_code(kind, code) {
                Found::None => {}
                Found::One(held) => {
                    return Exact::Ended(Self::matched(
                        element,
                        &self.rows[held.as_str()],
                        MatchTier::Code(kind.clone()),
                        true,
                    ));
                }
                Found::Several(codes) => {
                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {
                        tier: MatchTier::Code(kind.clone()),
                        codes: spelled_codes(codes),
                    }));
                }
            }
        }
        if let Some(ticker) = element.get_ticker() {
            keyed = true;
            match self.codes_by_ticker(ticker, market) {
                Found::None => {}
                Found::One(held) => {
                    return Exact::Ended(Self::matched(
                        element,
                        &self.rows[held.as_str()],
                        MatchTier::Symbology,
                        true,
                    ));
                }
                Found::Several(codes) => {
                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {
                        tier: MatchTier::Symbology,
                        codes: spelled_codes(codes),
                    }));
                }
            }
        }
        Exact::Open { keyed }
    }

    /// What the economic tier reads of `element`: its real short name, its
    /// stated currency - `XXX` is none - its origin currency where held and
    /// its CFI category; none where it states no short name or no currency.
    fn economic_reading<E: Market + ?Sized>(
        element: &E,
    ) -> Option<(&str, &Ccy, Option<&Ccy>, Option<char>)> {
        let fisn = element
            .get_securityids()
            .get(&IdType::Fisn)
            .filter(|name| IdType::Fisn.is_real(name))?;
        let currency = Some(element.get_currency()).filter(|code| code.rank() > 0)?;
        let origccy = Some(element.get_origccy()).filter(|code| !code.is_none());
        Some((
            fisn,
            currency,
            origccy,
            cfi_category(element.get_cficode().map(Cfi::as_str)),
        ))
    }

    /// Whether `instrument` is an economic candidate - holding a short name,
    /// listed in `currency` - and the conflict that drops it.
    fn candidate<'i>(
        instrument: &'i Instrument,
        currency: &Ccy,
        origccy: Option<&Ccy>,
        category: Option<char>,
    ) -> Option<(&'i str, Option<Unmatched>)> {
        let held = instrument.securityids.get(&IdType::Fisn)?;
        if !instrument
            .listings
            .iter()
            .any(|listing| listing.currency() == Some(currency))
            && instrument.currency.as_ref() != Some(currency)
        {
            return None;
        }
        let code = || instrument.crosscode.storage().clone();
        let conflict = match (origccy, instrument.origccy.as_ref()) {
            (Some(stated), Some(held)) if stated != held => Some(Unmatched::CurrencyConflict {
                stated: stated.clone(),
                held: held.clone(),
                code: code(),
            }),
            _ => match (
                category,
                cfi_category(instrument.securityids.get(&IdType::Cfi)),
            ) {
                (Some(stated), Some(held)) if stated != held => Some(Unmatched::CfiConflict {
                    stated,
                    held,
                    code: code(),
                }),
                _ => None,
            },
        };
        Some((held, conflict))
    }

    /// The instrument whose short name is the most similar to `fisn`, at
    /// least the threshold, among those holding one and listed in
    /// `currency`, an instrument stating another origin currency than
    /// `origccy` or another CFI category than `category` dropped. Two
    /// equally similar bests are ambiguous; a dropped instrument more
    /// similar than every candidate and at the threshold is its conflict; a
    /// best under the threshold is below it. Allocates nothing but the codes
    /// it may answer as ambiguous.
    fn scan(
        &self,
        fisn: &str,
        currency: &Ccy,
        origccy: Option<&Ccy>,
        category: Option<char>,
    ) -> Scored<'_> {
        let threshold = self.economic.threshold;
        let candidate = |instrument| Self::candidate(instrument, currency, origccy, category);
        let mut best: Option<(f64, &Str)> = None;
        let mut tied = false;
        let mut below: Option<(f64, &Str)> = None;
        let mut dropped: Option<(f64, Unmatched)> = None;
        for (code, instrument) in self.rows.iter() {
            let Some((held, conflict)) = candidate(instrument) else {
                continue;
            };
            if yggdryl::implementer::below_threshold(fisn.len(), held.len(), threshold) {
                #[allow(clippy::cast_precision_loss)] // a few dozen bytes
                let bound = 1.0
                    - fisn.len().abs_diff(held.len()) as f64 / fisn.len().max(held.len()) as f64;
                if conflict.is_none() && below.is_none_or(|(score, _)| bound > score) {
                    let score = yggdryl::implementer::similarity(fisn, held);
                    if below.is_none_or(|(held, _)| score > held) {
                        below = Some((score, code));
                    }
                }
                continue;
            }
            let score = yggdryl::implementer::similarity(fisn, held);
            match conflict {
                Some(conflict) => {
                    if score >= threshold && dropped.as_ref().is_none_or(|(held, _)| score > *held)
                    {
                        dropped = Some((score, conflict));
                    }
                }
                None if score >= threshold => match best {
                    Some((held, _)) if (score - held).abs() <= f64::EPSILON => tied = true,
                    Some((held, _)) if score < held => {}
                    _ => {
                        best = Some((score, code));
                        tied = false;
                    }
                },
                None => {
                    if below.is_none_or(|(held, _)| score > held) {
                        below = Some((score, code));
                    }
                }
            }
        }
        if let Some((score, conflict)) = dropped
            && best.is_none_or(|(held, _)| score > held)
        {
            return Err(conflict);
        }
        match (best, below) {
            (Some((score, code)), _) if !tied => Ok((code, score)),
            (Some((score, _)), _) => Err(Unmatched::Ambiguous {
                tier: MatchTier::Economic { similarity: score },
                codes: self
                    .rows
                    .iter()
                    .filter(|(_, instrument)| {
                        candidate(instrument).is_some_and(|(held, conflict)| {
                            conflict.is_none()
                                && !yggdryl::implementer::below_threshold(
                                    fisn.len(),
                                    held.len(),
                                    threshold,
                                )
                                && (yggdryl::implementer::similarity(fisn, held) - score).abs()
                                    <= f64::EPSILON
                        })
                    })
                    .map(|(code, _)| code.storage().clone())
                    .collect(),
            }),
            (None, Some((best, code))) => Err(Unmatched::BelowThreshold {
                best,
                code: code.storage().clone(),
            }),
            (None, None) => Err(Unmatched::NoCandidate),
        }
    }

    /// The economic tier over `element`, entered once the exact tier found
    /// nothing: `keyed` whether it stated a key that tier read.
    fn economic<E: Market + ?Sized>(&self, element: &E, keyed: bool) -> Resolution<'_> {
        let states_fisn = element
            .get_securityids()
            .get(&IdType::Fisn)
            .is_some_and(|name| IdType::Fisn.is_real(name));
        let Some((fisn, currency, origccy, category)) = Self::economic_reading(element) else {
            return Resolution::Unmatched(if keyed || states_fisn {
                Unmatched::NoCandidate
            } else {
                Unmatched::NoKey
            });
        };
        match self.scan(fisn, currency, origccy, category) {
            Ok((code, similarity)) => Self::matched(
                element,
                &self.rows[code.as_str()],
                MatchTier::Economic { similarity },
                true,
            ),
            Err(unmatched) => Resolution::Unmatched(unmatched),
        }
    }

    /// The instrument `element` names ([`Instruments::resolve`]).
    pub(crate) fn resolve<E: Market + ?Sized>(
        &self,
        element: &E,
        stated: &Stated<'_>,
    ) -> Resolution<'_> {
        match self.exact(element, stated) {
            Exact::Ended(resolution) => resolution,
            Exact::Open { keyed } => self.economic(element, keyed),
        }
    }

    /// The instrument a fill takes from: the exact tier's match, and -
    /// where the table states [`Instruments::is_economic_match`] and that
    /// tier found nothing - the economic one's, answered once per reading
    /// and generation where `memo` keeps them. The instrument, whether its
    /// ISIN is derived and whether its listing is the element's.
    fn fill_row<E: Market + ?Sized>(
        &self,
        element: &E,
        stated: &Stated<'_>,
        economic: bool,
        memo: Option<&mut EconomicMemo>,
    ) -> Option<(&Instrument, bool, bool)> {
        fn matched(resolution: Resolution<'_>) -> Option<(&Instrument, bool, bool)> {
            match resolution {
                Resolution::Matched {
                    entry,
                    derived,
                    listing,
                    ..
                } => Some((entry, derived, listing)),
                Resolution::Unmatched(_) => None,
            }
        }
        match self.exact(element, stated) {
            Exact::Ended(resolution) => return matched(resolution),
            Exact::Open { .. } if !(economic && self.economic.enabled) => return None,
            Exact::Open { .. } => {}
        }
        let Some(memo) = memo else {
            return matched(self.economic(element, true));
        };
        let (fisn, currency, origccy, category) = Self::economic_reading(element)?;
        if memo.generation != self.generation {
            memo.answers.clear();
            memo.generation = self.generation;
        }
        let key = (
            SmolStr::new(fisn),
            currency.clone(),
            origccy.cloned(),
            category,
        );
        let code = match memo.answers.get(&key) {
            Some(answer) => answer.clone(),
            None => {
                let answer = self
                    .scan(fisn, currency, origccy, category)
                    .ok()
                    .map(|(code, _)| code.clone());
                memo.answers.insert(key, answer.clone());
                answer
            }
        }?;
        let instrument = self.rows.get(code.as_str())?;
        Some((instrument, true, Self::listed(instrument, element)))
    }

    /// Derives into `element` each security identifier `instrument` holds
    /// of a type it holds none of - the ISIN where `derived`, every other
    /// identifier but the CFI, which is the element's `cficode` fact and
    /// no identifier of its own ([`Self::fill_unsettled`] refines it), the
    /// listing codes of its listing only where `listed` - through
    /// [`Market::derive_securityid`]. Whether anything moved.
    fn derive_into<E: Market + ?Sized>(
        instrument: &Instrument,
        derived: bool,
        listed: bool,
        element: &mut E,
    ) -> bool {
        let mut moved = false;
        for id in instrument
            .securityids
            .iter()
            .filter(|id| id.key().is_base())
        {
            if (*id.kind() == IdType::Isin && !derived) || *id.kind() == IdType::Cfi {
                continue;
            }
            if element.get_securityids().contains_kind(id.kind()) {
                continue;
            }
            moved |= element.derive_securityid(id.kind(), id.value());
        }
        if listed && let Some(listing) = Self::listing_for(instrument, element.get_miccode()) {
            for id in listing.codes().iter().filter(|id| id.key().is_base()) {
                if element.get_securityids().contains_kind(id.kind()) {
                    continue;
                }
                moved |= element.derive_securityid(id.kind(), id.value());
            }
        }
        moved
    }

    /// Fills the security identifiers `element` leaves unsaid from the
    /// instrument the exact tier names ([`Instruments::resolve`]): what a
    /// parse takes from the table its door fixed - derived identifiers
    /// only, which reach no field, no wire and no digest, and never an
    /// economic match. Whether anything moved; nothing is settled.
    pub fn fill_identifiers<E: Market + ?Sized>(&self, element: &mut E) -> bool {
        let Some((instrument, derived, listed)) =
            self.fill_row(element, &Stated::default(), false, None)
        else {
            return false;
        };
        Self::derive_into(instrument, derived, listed, element)
    }

    /// [`Self::fill_identifiers`], then the market facts a lifecycle fills:
    /// the ticker on the same market where the element states none, the
    /// CFI where it states none or the instrument's refines it, the
    /// currency only where both markets are stated and equal, the ticker
    /// is the listing's and the element states none, the origin currency
    /// the instrument holds where the element holds none, and the
    /// instrument's cross code as the element's `instcode` where it holds
    /// none - from the exact tier's instrument, else, where the table
    /// states [`Instruments::is_economic_match`], the economic one's,
    /// `memo` keeping its answers for the walk; `stated` what only a
    /// message spells of the instrument. Whether anything moved; nothing
    /// is settled.
    pub fn fill_unsettled<E: Market + ?Sized>(
        &self,
        element: &mut E,
        stated: &Stated<'_>,
        memo: Option<&mut EconomicMemo>,
    ) -> bool {
        let Some((instrument, derived, listed)) = self.fill_row(element, stated, true, memo) else {
            return false;
        };
        let market = element
            .get_miccode()
            .filter(|code| !code.is_none())
            .cloned();
        let listing = Self::listing_for(instrument, market.as_ref());
        let markets_equal = listed
            && market.is_some()
            && listing.is_some_and(|listing| listing.miccode().is_some());
        let mut moved = Self::derive_into(instrument, derived, listed, element);
        if listed
            && element.get_ticker().is_none()
            && let Some(ticker) = listing.and_then(Listing::ticker)
        {
            element.set_ticker(Some(SmolStr::new(ticker)), false);
            moved = true;
        }
        if let Some(learned) = instrument.securityids.get(&IdType::Cfi) {
            let refined = match element.get_cficode() {
                None => Some(SmolStr::new(learned)),
                Some(stated) => Cfi::refined(stated.as_str(), learned)
                    .filter(|refined| refined != stated.as_str()),
            };
            if let Some(code) = refined.and_then(|code| Cfi::new(code).ok()) {
                element.set_cficode(Some(code), true);
                moved = true;
            }
        }
        if markets_equal
            && element.get_ticker().map(str::trim) == listing.and_then(Listing::ticker)
            && element.get_currency().is_none()
            && let Some(currency) = listing.and_then(Listing::currency)
        {
            element.set_currency(currency.clone(), false);
            moved = true;
        }
        if element.get_origccy().is_none()
            && let Some(origccy) = &instrument.origccy
        {
            element.set_origccy(origccy.clone(), false);
            moved = true;
        }
        if element.get_instcode().is_none() {
            element.set_instcode(Some(instrument.crosscode.clone()), false);
            moved = true;
        }
        moved
    }
}

/// The codes of several instruments as a refusal names them.
fn spelled_codes(codes: Vec<Str>) -> Vec<SmolStr> {
    codes.into_iter().map(Str::into_inner).collect()
}

/// What the store holds of one instrument, as it was loaded or last written:
/// the content code, which digests every fact - so a value that moves and
/// moves back within a run is no change - and the window the instrument was
/// met in, whose edges only an event outside it moves. `updunix`, the
/// instant a fact last moved, follows the content and is recorded with
/// nothing: a flip and its flip-back move it and change no fact.
/// [`Instruments::is_dirty`] compares the table against these.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stored {
    hashcode: u64,
    firstunix: Option<i64>,
    lastunix: Option<i64>,
}

impl Stored {
    fn of(instrument: &Instrument) -> Self {
        Self {
            hashcode: instrument.hashcode,
            firstunix: instrument.firstunix,
            lastunix: instrument.lastunix,
        }
    }
}

/// What the store holds of every instrument, by code, shared: the seed's
/// by every seeded collection.
type StoredRows = Arc<HashMap<Str, Stored>>;

static EMPTY_STORED: LazyLock<StoredRows> = LazyLock::new(Arc::default);

/// `table` as a store holds it, each key a clone of the instrument's own.
fn stored_rows(table: &InstrumentTable) -> StoredRows {
    Arc::new(
        table
            .rows
            .iter()
            .map(|(code, held)| (code.clone(), Stored::of(held)))
            .collect(),
    )
}

/// Whether `table` differs from what the store holds (`stored`): an
/// instrument added or removed, or one whose content code or window moved.
fn differs(table: &InstrumentTable, stored: &StoredRows) -> bool {
    table.rows.len() != stored.len()
        || table
            .rows
            .iter()
            .any(|(code, held)| stored.get(code) != Some(&Stored::of(held)))
}

/// Every instrument's facts, one element per cross code.
///
/// The key is the cross code ([`Instrument`]): a real ISIN for a security,
/// `class:body` for everything no agency numbers. An ISIN - real or minted -
/// a ticker on its market, every code of [`Self::LOOKUP_CODES`] and a code
/// an instrument had before a re-key lead back to it; [`Self::resolve`] is
/// the one door over them and over the economic match. Learning is keyed by
/// the code an event's facts spell - a stated real ISIN, the pair and class
/// of an FX element, what a FIX message spells of a derivative - and is the
/// ordered lifecycle's, or an explicit [`Self::learn`], [`Self::fill`] or
/// [`Self::enrich`]; a parse fills derived identifiers from the table its
/// door fixed and learns nothing. The table is held apart from the store
/// the collection is bound to ([`Self::from_holder`], [`Self::commit`]), and
/// it is dirty ([`Self::is_dirty`]) while its content differs from what the
/// store holds.
///
/// ```
/// use yggdryl::graph::Event;
/// use yggdryl_market::graph::{Market, OrderEvent};
/// use yggdryl::{Cfi, Mic};
/// use yggdryl_market::{IdKey, IdType, Identifier, Instruments};
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_market::install().unwrap();
/// let mut stated = OrderEvent::default();
/// stated.set_transunix(1);
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
/// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
/// stated.set_ticker(Some("HOLN".into()), true);
/// stated.set_miccode(Some(Mic::new("XSWX")?), true);
/// stated.set_cficode(Some(Cfi::new("ESVUFR")?), true);
/// let mut instruments = Instruments::new();
/// assert!(instruments.learn(&stated));
/// assert!(instruments.is_dirty());
/// let holcim = instruments.get_by_ticker("HOLN", Some(&Mic::new("XSWX")?)).expect("the instrument");
/// assert_eq!(holcim.get(&IdType::Ric), Some("HOLN.S"), "a RIC is a listing code, and a lookup key");
/// assert_eq!(instruments.get_by_code(&IdType::Ric, "HOLN.S"), Some(holcim));
///
/// // A later statement naming only the ticker on that market is filled with the rest.
/// let mut later = OrderEvent::default();
/// later.set_ticker(Some("HOLN".into()), true);
/// later.set_miccode(Some(Mic::new("XSWX")?), true);
/// assert!(instruments.fill(&mut later));
/// assert_eq!(later.get_isincode(), Some("CH0012214059"));
/// assert_eq!(later.get_instcode(), Some("CH0012214059"));
/// assert_eq!(later.get_securityids().get(&IdType::Ric), Some("HOLN.S"));
/// assert_eq!(later.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
///
/// // The instrument stated on another market is a second listing of it.
/// let mut london = OrderEvent::default();
/// london.set_transunix(2);
/// london.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
/// london.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.L")?)?;
/// london.set_miccode(Some(Mic::new("XLON")?), true);
/// assert!(instruments.learn(&london));
/// assert_eq!(instruments.len(), 1);
/// let rics: Vec<_> = instruments.get("CH0012214059").unwrap().listings().iter().map(|listing| listing.get(&IdType::Ric)).collect();
/// assert_eq!(rics, [Some("HOLN.L"), Some("HOLN.S")], "MIC order: XLON, then XSWX");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Instruments {
    table: InstrumentTable,
    max_instruments: usize,
    /// Whether learning past the bound was warned of already.
    warned: bool,
    /// Whether anything moved since the table was loaded or committed: the
    /// precheck [`Self::is_dirty`] reads before comparing the table with
    /// what the store holds.
    moved: bool,
    /// What the store holds of every instrument, as it was loaded or last
    /// committed - the seed's for a seeded collection, nothing for a new
    /// one - which [`Self::is_dirty`] compares the table against.
    stored: StoredRows,
    /// The store the table is bound to, where it was; boxed so the
    /// collection stays small enough to sit inline in a walk's own state.
    store: Option<Box<Store>>,
}

impl Default for Instruments {
    fn default() -> Self {
        Self::new()
    }
}

impl Instruments {
    /// The instruments a collection holds unless told otherwise: at 32 KiB
    /// each at most, 512 MiB.
    pub const DEFAULT_MAX_INSTRUMENTS: usize = 16_384;

    /// The most equivalents one instrument holds among its identifiers,
    /// beside the ISIN, the CFI, the pair and the short name.
    pub const MAX_EQUIVALENTS: usize = 12;

    /// The listing and instrument codes a lookup reads, in cascade order,
    /// each indexed with one slot per instrument ([`Self::get_by_code`],
    /// [`Self::resolve`]). A currency, a country, an index name and a
    /// synthetic code are shared by whole markets, an LEI and a RED entity
    /// code name an issuer, and the ISDA, FpML, CFTC, clearing house and
    /// letter of credit codes a reference many instruments share, so none
    /// of them is a key.
    pub const LOOKUP_CODES: [IdType; 20] = [
        IdType::Cusip,
        IdType::Sedol,
        IdType::Wkn,
        IdType::Valor,
        IdType::Figi,
        IdType::Bloomberg,
        IdType::Ric,
        IdType::Quik,
        IdType::ExchSymb,
        IdType::Cta,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::Opra,
        IdType::MktAssigned,
        IdType::RedPair,
        IdType::Fim,
        IdType::Umtf,
        IdType::Dti,
    ];

    /// How similar two short names must be for an economic match.
    pub const DEFAULT_ECONOMIC_THRESHOLD: f64 = 0.85;

    /// An empty collection, bounded at [`Self::DEFAULT_MAX_INSTRUMENTS`],
    /// bound to no store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            table: InstrumentTable::default(),
            max_instruments: Self::DEFAULT_MAX_INSTRUMENTS,
            warned: false,
            moved: false,
            stored: Arc::clone(&EMPTY_STORED),
            store: None,
        }
    }

    /// A collection holding the seed - the common instruments
    /// `config/instruments/instruments.json` states, embedded at build time:
    /// each a stock, a fund or an index by its ISIN, its CFI, its country,
    /// its short name and its listings - clean and bound to no store. Making
    /// one shares that table: no instrument is copied until one moves.
    ///
    /// ```
    /// use yggdryl::graph::Element;
    /// use yggdryl::Mic;
    /// use yggdryl_market::{IdType, Instruments};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let seeded = Instruments::seeded();
    /// assert!(!seeded.is_dirty() && seeded.holder().is_none());
    /// let apple = seeded.get_by_ticker("AAPL", Some(&Mic::new("XNAS")?)).expect("seeded");
    /// assert_eq!(apple.get_crosscode(), "US0378331005");
    /// assert_eq!(apple.get(&IdType::Cusip), Some("037833100"), "the CUSIP its ISIN embeds");
    /// assert!(Instruments::new().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn seeded() -> Self {
        Self {
            table: seed::table().clone(),
            stored: Arc::clone(seed::stored()),
            ..Self::new()
        }
    }

    /// The collection bounded at `max` instruments.
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

    /// How many rows a commit writes: one per instrument.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.table.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Whether the table's content differs from what the store holds - as
    /// it was loaded or last committed, the seed for a seeded collection,
    /// nothing for a new one: an instrument added or removed, or one whose
    /// content code (`hashcode`) or window (`firstunix`, `lastunix`) moved.
    /// A fact that moves and moves back since the load is no change, two
    /// sources disagreeing on one metadata key within a run among them, so
    /// a run replayed over the same input leaves a clean collection; the
    /// instant a fact last moved (`updunix`) follows the content and
    /// dirties nothing on its own.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.moved && differs(&self.table, &self.stored)
    }

    /// Records the table as what the store holds: clean until something
    /// moves it.
    fn record_stored(&mut self) {
        self.stored = stored_rows(&self.table);
        self.moved = false;
    }

    /// The table as it stands, shared: what a parse door fixes once.
    pub(crate) fn as_table(&self) -> &InstrumentTable {
        &self.table
    }

    /// The instrument `key` names: by its cross code, a code it had before
    /// a re-key, or an ISIN it holds, real or minted.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Instrument> {
        self.table.get(key)
    }

    /// The instrument whose identity - or former identity - is `uuid`.
    #[must_use]
    pub fn get_by_uuid(&self, uuid: Uuid) -> Option<&Instrument> {
        self.table.get_by_uuid(uuid)
    }

    /// The listings of the instrument `key` names, in MIC order; empty
    /// where it is unknown.
    #[must_use]
    pub fn listings(&self, key: &str) -> &[Listing] {
        self.get(key).map_or(&[], Instrument::listings)
    }

    /// The listing of the instrument `key` names on `market`.
    #[must_use]
    pub fn get_listing(&self, key: &str, market: &Mic) -> Option<&Listing> {
        self.get(key)?.listing(Some(market))
    }

    /// The instrument the ticker `ticker` - trimmed - names on `market`,
    /// through the ticker index: the one instrument a listing of which
    /// lists it on `market`, else - none lists it there - on no market;
    /// where `market` is unstated - none and `XXXX` - on any. Two
    /// instruments answering is ambiguous, and answers none.
    ///
    /// ```
    /// use yggdryl_market::graph::{Market, OrderEvent};
    /// use yggdryl_market::{IdKey, IdType, Identifier, Instruments};
    /// use yggdryl::Mic;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let mut stated = OrderEvent::default();
    /// stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    /// stated.set_ticker(Some("HOLN".into()), true);
    /// stated.set_miccode(Some(Mic::new("XSWX")?), true);
    /// let mut instruments = Instruments::new();
    /// assert!(instruments.learn(&stated));
    /// let isin = |market: Option<&Mic>| {
    ///     instruments.get_by_ticker("HOLN", market).and_then(|held| held.isin())
    /// };
    /// assert_eq!(isin(None), Some("CH0012214059"));
    /// assert_eq!(isin(Some(&Mic::new("XSWX")?)), Some("CH0012214059"));
    /// assert_eq!(isin(Some(&Mic::new("XLON")?)), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn get_by_ticker(&self, ticker: &str, market: Option<&Mic>) -> Option<&Instrument> {
        self.table.get_by_ticker(ticker, market)
    }

    /// The instrument the code `code` of `kind` - one of
    /// [`Self::LOOKUP_CODES`], read as its type stores it - names, through
    /// the code index. Two instruments holding the code is ambiguous, and
    /// answers none, as does a type no lookup reads and a value its type
    /// refuses.
    ///
    /// ```
    /// use yggdryl_market::{IdType, Instruments};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let seeded = Instruments::seeded();
    /// let hsbc = seeded.get_by_code(&IdType::Sedol, "0540528").expect("seeded");
    /// assert_eq!(hsbc.isin(), Some("GB0005405286"), "two listings, one instrument");
    /// assert_eq!(hsbc.listings().len(), 2);
    /// assert!(seeded.get_by_code(&IdType::IsoCcy, "USD").is_none(), "no key");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn get_by_code(&self, kind: &IdType, code: &str) -> Option<&Instrument> {
        if !is_lookup(kind) {
            return None;
        }
        let id = Identifier::new(IdKey::base(kind.clone()), code).ok()?;
        self.table.get_by_code(kind, id.value())
    }

    /// The instrument `element` names, and how, by the one waterfall a fill
    /// reads: a real ISIN it holds decides alone, one the collection lacks
    /// ending the cascade ([`Unmatched::UnknownIsin`]); else the cross code
    /// its own facts spell - an FX pair's from its `forex` identifier and
    /// its CFI class - one the collection lacks ending it too
    /// ([`Unmatched::UnknownCode`]); else a minted number it holds; else
    /// each code of [`Self::LOOKUP_CODES`] it holds, in that order, then its
    /// ticker on its market, the first naming one instrument matching and
    /// the first naming two ending the cascade ([`Unmatched::Ambiguous`]);
    /// and, only where all of those found nothing, the economic match: the
    /// instrument listed in the element's stated currency whose short name
    /// is the most similar to the one it states ([`Fisn::similarity`]), at
    /// least [`Self::economic_threshold`], an instrument of another stated
    /// origin currency or CFI category dropped. A match below the ISIN tier
    /// is derived. An economic match is a judgement: this door always
    /// weighs it, and a fill takes it only where [`Self::is_economic_match`]
    /// says so.
    ///
    /// ```
    /// use yggdryl_market::graph::{Market, OrderEvent};
    /// use yggdryl::Ccy;
    /// use yggdryl_market::{IdKey, IdType, Identifier, Instruments, MatchTier, Resolution};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// #     yggdryl_market::install().unwrap();
    /// let seeded = Instruments::seeded();
    /// let mut element = OrderEvent::default();
    /// element.insert_securityid(Identifier::new(IdKey::base(IdType::Cusip), "037833100")?)?;
    /// let Resolution::Matched { entry, tier, derived, .. } = seeded.resolve(&element) else {
    ///     panic!("Apple by its CUSIP");
    /// };
    /// assert_eq!((entry.isin(), tier, derived), (Some("US0378331005"), MatchTier::Code(IdType::Cusip), true));
    ///
    /// let mut named = OrderEvent::default();
    /// named.insert_securityid(Identifier::new(IdKey::base(IdType::Fisn), "APPLE INC./SH SH")?)?;
    /// named.set_currency(Ccy::new("USD")?, true);
    /// let Resolution::Matched { entry, tier: MatchTier::Economic { similarity }, .. } = seeded.resolve(&named) else {
    ///     panic!("Apple by its short name");
    /// };
    /// assert_eq!(entry.isin(), Some("US0378331005"));
    /// assert!(similarity > Instruments::DEFAULT_ECONOMIC_THRESHOLD);
    /// # Ok(())
    /// # }
    /// ```
    pub fn resolve<E: Market + ?Sized>(&self, element: &E) -> Resolution<'_> {
        self.table.resolve(element, &Stated::default())
    }

    /// How similar two short names must be, from above `0` to `1`, for an
    /// economic match.
    #[must_use]
    pub fn economic_threshold(&self) -> f64 {
        self.table.economic.threshold
    }

    /// Sets [`Self::economic_threshold`].
    ///
    /// # Errors
    ///
    /// NaN, and a value outside `(0, 1]`, named.
    pub fn set_economic_threshold(&mut self, threshold: f64) -> Result<()> {
        if !(threshold > 0.0 && threshold <= 1.0) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.economic_threshold"),
                reason: format_smolstr!("expected a similarity in (0, 1], got {threshold}"),
            });
        }
        self.table.economic.threshold = threshold;
        Ok(())
    }

    /// [`Self::set_economic_threshold`], consuming.
    ///
    /// # Errors
    ///
    /// What [`Self::set_economic_threshold`] refuses.
    pub fn try_with_economic_threshold(mut self, threshold: f64) -> Result<Self> {
        self.set_economic_threshold(threshold)?;
        Ok(self)
    }

    /// Whether a fill takes an economic match where nothing exact names the
    /// element; `false` unless told otherwise. A parse never takes one.
    #[must_use]
    pub fn is_economic_match(&self) -> bool {
        self.table.economic.enabled
    }

    /// Sets [`Self::is_economic_match`].
    pub fn set_economic_match(&mut self, enabled: bool) {
        self.table.economic.enabled = enabled;
    }

    /// [`Self::set_economic_match`], consuming.
    #[must_use]
    pub fn with_economic_match(mut self, enabled: bool) -> Self {
        self.set_economic_match(enabled);
        self
    }

    /// Every instrument, in cross code order.
    pub fn iter(&self) -> impl Iterator<Item = &Instrument> {
        self.table.iter()
    }

    /// Folds `entry` into the instrument of its cross code by the update
    /// rule: a stated value fills a fact the instrument lacks and replaces
    /// one it holds that differs, whatever the time; a code that is no real
    /// value of its type is dropped with one warning per type; a CFI
    /// compatible with the held one refines it and a contradicting one
    /// replaces it; a real ISIN replaces a minted one. The instrument facts
    /// fold into the instrument; the listing facts - the ticker, the
    /// currency, the listing codes - into the listing of each market the
    /// entry lists, created where the instrument has none there. A
    /// placeholder keyed by its ISIN is re-keyed by an entry spelling its
    /// body, the ISIN kept in [`Instrument::aliascodes`]. `updunix` becomes
    /// the later of the two where a fact moved, `firstunix` the earlier and
    /// `lastunix` the later whatever moved. An instrument created or moved
    /// then fills what its own facts imply where the statement left them
    /// empty. Whether anything moved.
    ///
    /// # Errors
    ///
    /// A new instrument past [`Self::max_instruments`].
    pub fn merge(&mut self, entry: Instrument) -> Result<bool> {
        let mut moved = self.fold(&entry.statement())?;
        // The statement carries one listing's facts; several are folded one
        // by one after it.
        let several = entry.listings.len() > 1;
        for listing in entry.listings.iter().filter(|_| several) {
            let mut one = Instrument::blank();
            one.securityids = entry.securityids.clone();
            one.crosscode = entry.crosscode.clone();
            one.placeholder = entry.placeholder;
            one.listings.push(listing.clone());
            one.updunix = entry.updunix;
            one.firstunix = entry.firstunix;
            one.lastunix = entry.lastunix;
            moved |= self.fold(&one.statement())?;
        }
        Ok(moved)
    }

    /// Removes the instrument `key` names - by its cross code, an alias or
    /// an ISIN - answering it; none where it is unknown.
    pub fn remove(&mut self, key: &str) -> Option<Instrument> {
        let code = self.table.code_of(key)?.clone();
        let (mut indexes, rows) = self.table.indexes();
        let removed = rows.remove(code.as_str())?;
        indexes.removed(&code, &removed);
        Arc::make_mut(&mut self.table.aliases).retain(|_, held| *held != code);
        self.table.moved();
        self.moved = true;
        Some(removed)
    }

    /// Removes the listing of the instrument `key` names on `market`,
    /// answering it; the instrument stays.
    pub fn remove_listing(&mut self, key: &str, market: &Mic) -> Option<Listing> {
        let code = self.table.code_of(key)?.clone();
        let held = self.table.rows.get(code.as_str())?;
        let at = held.listing_at(Some(market)).ok()?;
        let mut next = held.clone();
        let removed = next.listings.remove(at);
        next.finalize();
        self.put(&code, next);
        Some(removed)
    }

    /// Removes every instrument.
    pub fn clear(&mut self) {
        if !self.table.is_empty() {
            self.moved = true;
        }
        self.table = self.table.emptied();
    }

    /// Puts `next` under `code`, keeping the indexes.
    fn put(&mut self, code: &Str, next: Instrument) {
        let (mut indexes, rows) = self.table.indexes();
        let held = rows.insert(code.clone(), next);
        let next = &rows[code.as_str()];
        match held {
            Some(held) => indexes.moved(code, &held, next),
            None => indexes.list_all(next, code),
        }
        self.table.moved();
        self.moved = true;
    }

    /// Strips a minted number another instrument already holds off
    /// `instrument`, refusing the mint by name: the instrument keeps its
    /// identity with no number.
    fn check_mint(&self, instrument: &mut Instrument) {
        let Some(number) = instrument.minted_isin() else {
            return;
        };
        let Some(held) = self.table.isins.get(&packed(number)) else {
            return;
        };
        if held.as_str() == instrument.get_crosscode() {
            return;
        }
        warned!(
            "instrument mint refused: another cross code holds the minted number",
            "isin",
            "{number} minted for {} is held by {held}",
            instrument.get_crosscode()
        );
        let number = SmolStr::new(number);
        instrument.securityids.remove(&MINT_KEY);
        if instrument.isin() == Some(number.as_str()) {
            instrument.securityids.remove(&IdKey::base(IdType::Isin));
        }
        instrument.finalize();
    }

    /// Folds one statement into the instrument of its code, then derives
    /// the defaults of the instrument it moved - once, after the statement
    /// landed. Nothing is built, and the table is not copied, where nothing
    /// moves.
    fn fold(&mut self, statement: &Statement<'_>) -> Result<bool> {
        // A body statement naming a real ISIN reaches the placeholder keyed
        // by that number: the re-key.
        let placeholder = || {
            let isin = statement
                .isin
                .filter(|_| statement.production == Production::Body)?;
            let code = self.table.isins.get(&packed(isin))?;
            self.table.rows[code.as_str()]
                .placeholder
                .then(|| code.clone())
        };
        let Some(code) = self
            .table
            .code_of(statement.code)
            .cloned()
            .or_else(placeholder)
        else {
            if self.table.rows.len() >= self.max_instruments {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$['{}']", statement.code),
                    reason: format_smolstr!(
                        "expected at most {} instruments, got one more",
                        self.max_instruments
                    ),
                });
            }
            let mut instrument = Instrument::from_statement(statement)?;
            instrument.derive_defaults();
            self.check_mint(&mut instrument);
            instrument.finalize();
            let code = instrument.crosscode.clone();
            self.put(&code, instrument);
            return Ok(true);
        };
        let held = &self.table.rows[code.as_str()];
        if statement.rekeys(held) {
            let mut next = held.clone();
            next.put_cfi(statement.cfi.and_then(|cfi| Cfi::new(cfi).ok()).as_ref());
            if let Some(forex) = statement.forex
                && let Ok(id) = Identifier::new(IdKey::base(IdType::Forex), forex)
            {
                next.securityids.set(id);
            }
            next.set_underlying(statement.underlying)?;
            next.set_legs(statement.legs)?;
            next.characteristics = statement.characteristics.clone();
            self.rekey(&code, next)?;
            self.fold(statement)?;
            return Ok(true);
        }
        let held_updunix = held.updunix;
        let firstunix = earlier(held.firstunix, statement.firstunix);
        let lastunix = later(held.lastunix, statement.lastunix);
        let mut next = match folded(held, statement) {
            Some(next) => next,
            None if held.firstunix != firstunix || held.lastunix != lastunix => {
                // A stamp alone moves no index, no generation and no fact
                // the content code reads: written where the instrument
                // stands, nothing copied.
                let held = Arc::make_mut(&mut self.table.rows)
                    .get_mut(code.as_str())
                    .expect("the code was found");
                held.firstunix = firstunix;
                held.lastunix = lastunix;
                self.moved = true;
                return Ok(true);
            }
            None => return Ok(false),
        };
        next.updunix = later(held_updunix, statement.updunix);
        next.derive_defaults();
        next.firstunix = firstunix;
        next.lastunix = lastunix;
        next.finalize();
        self.put(&code, next);
        Ok(true)
    }

    /// Re-keys the instrument held under `old` to the code `next`'s facts
    /// spell - the one planned move, a placeholder taking its body - the
    /// old code kept among its aliases, and every derivative naming the old
    /// code as its underlying or a leg re-keyed after it.
    fn rekey(&mut self, old: &Str, mut next: Instrument) -> Result<()> {
        next.push_alias(old.as_str())?;
        next.respell()?;
        let new = next.crosscode.clone();
        if new == *old {
            self.put(old, next);
            return Ok(());
        }
        {
            let (mut indexes, rows) = self.table.indexes();
            if let Some(held) = rows.remove(old.as_str()) {
                indexes.removed(old, &held);
            }
        }
        self.put(&new, next);
        let aliases = Arc::make_mut(&mut self.table.aliases);
        aliases.insert(old.clone(), new.clone());
        for held in aliases.values_mut() {
            if *held == *old {
                *held = new.clone();
            }
        }
        // The cascade: a derivative written on the old code, or holding it
        // as a leg, spells its own key again.
        let dependents: Vec<Str> = self
            .table
            .rows
            .iter()
            .filter(|(_, held)| {
                held.underlying() == Some(old.as_str())
                    || held.legs.iter().any(|leg| leg.code() == old.as_str())
            })
            .map(|(code, _)| code.clone())
            .collect();
        for code in dependents {
            let mut dependent = self.table.rows[code.as_str()].clone();
            if dependent.underlying() == Some(old.as_str()) {
                dependent.underlying = Some(new.clone());
            }
            let legs: Vec<Leg> = dependent
                .legs
                .iter()
                .map(|leg| {
                    if leg.code() == old.as_str() {
                        Leg::new(new.as_str(), leg.ratio)
                    } else {
                        Ok(leg.clone())
                    }
                })
                .collect::<Result<_>>()?;
            dependent.set_legs(&legs)?;
            self.rekey(&code, dependent)?;
        }
        Ok(())
    }

    /// Learns what `event` states about its instrument: keyed by the code
    /// its facts spell - a stated real ISIN, never a derivation, keys a
    /// security, or a derivative as a placeholder where its body is
    /// unspelled; its `forex` identifier beside a CFI of class `I*` keys an
    /// FX spot, minting its number; a `J*` or `S*` class states no settle
    /// on a bare market element, so it learns nothing - its market but
    /// `XXXX` naming the listing its listing facts land on, dated at its
    /// `transunix`; reading its CFI, its ticker, its currency but `XXX`
    /// (unless it holds a pair, where `Currency(15)` is the dealt currency),
    /// its origin currency but `XXX`, the short name it states and each
    /// equivalent its map answers with a real value, never one it only
    /// derived. A new instrument past [`Self::max_instruments`] is not
    /// learned, with one warning. Whether anything moved.
    pub fn learn<E: Market + Event + ?Sized>(&mut self, event: &E) -> bool {
        let origccy = event.get_origccy().clone();
        let stated = Stated {
            origccy: Some(&origccy),
            ..Stated::default()
        };
        let learned = self.learn_stating(event, &stated);
        if let Some(max) = learned.full {
            warn_full(max);
        }
        learned.moved
    }

    /// [`Self::learn`] with what only a message spells laid over the
    /// event's own facts (`stated`): the body of a derivative or a forward,
    /// the underlying's ISIN - resolved to the code of the instrument it
    /// keys here, none giving no body - the country of issue, the origin
    /// currency and the product category. Answers whether an instrument
    /// moved, and the bound a new one was passed over at the first time one
    /// is, which the caller warns of itself ([`warn_full`]) after letting go
    /// of any lock it holds the collection under. Nothing here logs.
    pub(crate) fn learn_stating<E: Market + Event + ?Sized>(
        &mut self,
        event: &E,
        stated: &Stated<'_>,
    ) -> Learned {
        let ids = event.get_securityids();
        let is_stated = |kind: &IdType| ids.get(kind).filter(|_| !ids.is_derived(kind));
        let mut slot = [0_u8; MAX_CODE_WIDTH];
        let isin = is_stated(&IdType::Isin).filter(|isin| IdType::Isin.is_real(isin));
        let spelled = {
            // A derived ISIN - a minted number, a lookup's answer - keys no
            // learn: the key is what the event states.
            let default = Characteristics::default();
            let body = stated.body;
            let underlying = stated
                .underlying
                .and_then(|isin| self.table.isins.get(&packed(isin.as_str())))
                .map(Str::as_str);
            let keying = Keying {
                class: event.get_cficode().map(Cfi::as_str).and_then(class_of),
                isin,
                forex: body
                    .and_then(|body| body.forex.as_ref())
                    .map(Forex::as_str)
                    .or_else(|| ids.get(&IdType::Forex)),
                characteristics: body.map_or(&default, |body| &body.characteristics),
                underlying,
                legs: body.map_or(&[], |body| body.legs.as_slice()),
            };
            match write_code(&keying, &mut slot) {
                Ok(Some(spelled)) => spelled,
                _ => return Learned::default(),
            }
        };
        let (code, production) = spelled;
        let pair = ids.contains_kind(&IdType::Forex);
        let default = Characteristics::default();
        let body = stated.body;
        let underlying = stated
            .underlying
            .and_then(|isin| self.table.isins.get(&packed(isin.as_str())))
            .cloned();
        let currency = event.get_currency();
        let fisn = is_stated(&IdType::Fisn).filter(|name| IdType::Fisn.is_real(name));
        let forex_override = body.and_then(|body| body.forex.as_ref()).map(Forex::as_str);
        let statement = Statement {
            code,
            production,
            isin,
            cfi: event.get_cficode().map(Cfi::as_str),
            forex: forex_override.or_else(|| ids.get(&IdType::Forex)),
            fisn,
            countrycode: stated.country.filter(|code| code.is_listed() && !pair),
            currency: Some(currency).filter(|code| !code.is_none() && !pair),
            origccy: stated.origccy.filter(|code| !code.is_none() && !pair),
            underlying: underlying.as_deref(),
            legs: body.map_or(&[], |body| body.legs.as_slice()),
            characteristics: body.map_or(&default, |body| &body.characteristics),
            eusipacode: stated.product,
            miccode: event.get_miccode().filter(|code| !code.is_none()),
            ticker: event.get_ticker().and_then(Listing::trimmed_ticker),
            ids: ids
                .iter()
                .filter(|id| {
                    // A base key echoing a derivation states nothing; a
                    // named source's statement stands beside one.
                    states_own_key(id)
                        && !(id.key().is_base() && ids.is_derived(id.kind()))
                        && id.kind().is_real(id.value())
                })
                .map(|id| (id.key(), id.value()))
                .take(Instrument::MAX_SECURITYIDS)
                .collect(),
            metadata: stated
                .metadata
                .iter()
                .filter(|(key, value)| !key.is_empty() && !value.is_empty())
                .map(|(key, value)| (key.trim(), value.trim()))
                .filter(|(key, value)| !key.is_empty() && !value.is_empty())
                .take(Instrument::MAX_METADATA)
                .collect(),
            aliascodes: &[],
            updunix: Some(event.get_transunix()),
            firstunix: Some(event.get_transunix()),
            lastunix: Some(event.get_transunix()),
        };
        if self.table.code_of(statement.code).is_none()
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
    /// instrument it names ([`Self::resolve`]): each identifier of a type it
    /// holds none of as a derived identifier - the ISIN where it stated no
    /// real one, the minted number of an FX pair among them - the listing
    /// codes only where its market is listed or it states none and the
    /// instrument has one listing, the ticker on that listing, its CFI
    /// where it states none or the instrument's refines it, the currency
    /// only where both markets are stated and equal, the ticker is the
    /// listing's and it states none, the origin currency the instrument
    /// holds where it holds none, and the instrument's cross code as its
    /// `instcode` where it holds none. The element is finalized where
    /// anything moved. Whether anything moved.
    pub fn fill<E: Market + Element + ?Sized>(&self, element: &mut E) -> bool {
        self.fill_stating(element, &Stated::default())
    }

    /// [`Self::fill`] with what only a message spells laid over the
    /// element's facts.
    pub(crate) fn fill_stating<E: Market + Element + ?Sized>(
        &self,
        element: &mut E,
        stated: &Stated<'_>,
    ) -> bool {
        let moved = self.table.fill_unsettled(element, stated, None);
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

    /// A collection read from `reader`'s rows
    /// ([`Self::extend_from_arrow_reader`]), bound to no store and clean.
    ///
    /// # Errors
    ///
    /// What [`Self::extend_from_arrow_reader`] refuses.
    pub fn from_arrow_reader(reader: BatchReader) -> yggdryl::arrow::Result<Self> {
        let mut instruments = Self::new();
        instruments.extend_from_arrow_reader(reader)?;
        instruments.record_stored();
        Ok(instruments)
    }

    /// Folds `reader`'s rows in, each through [`Self::merge`] as a
    /// statement; answers how many rows it read. The stream is cast once
    /// into [`Instrument::field`], a nullable column it lacks null; a row
    /// whose `crosscode` and `hashcode` are a held instrument's states what
    /// is held and moves its stamps alone, read off the landed columns and
    /// never built; any other row's key is written again from its typed
    /// columns, and a stored `crosscode` they do not spell is refused naming
    /// both. A column no field reads - a golden file's own - lands in each
    /// row's [`Instrument::metadata`] under its name, a text cell as it is
    /// and any other as its JSON text.
    ///
    /// # Errors
    ///
    /// A stream of any column lacking `crosscode` - a store written before
    /// the instrument row, which it is told to drop - a
    /// row a column refuses, located `$[row].column`, a bound passed, and a
    /// new instrument past [`Self::max_instruments`].
    pub fn extend_from_arrow_reader(
        &mut self,
        reader: BatchReader,
    ) -> yggdryl::arrow::Result<usize> {
        let schema = reader.schema();
        // A stream of no columns - what a missing store reads as - holds no
        // row to key.
        if schema.fields().is_empty() {
            return Ok(0);
        }
        if schema.column_with_name(NAMES[CROSSCODE]).is_none() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.crosscode"),
                reason: format_smolstr!(
                    "expected the instrument row, which the table lacks the crosscode column of - a store \
                     written before the instrument row, to drop and lay out afresh; got {:?}",
                    schema
                        .fields()
                        .iter()
                        .map(|column| column.name().as_str())
                        .collect::<Vec<_>>()
                ),
            }
            .into());
        }
        // A column no field reads is a complementary fact of every row that
        // states it ([`Instrument::metadata`]), under the column's name: the
        // stream then lands under its own root and each row is read whole.
        let extras: Vec<(usize, SmolStr)> = schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, column)| column_at(column.name()).is_none())
            .map(|(at, column)| (at, SmolStr::new(column.name())))
            .collect();
        let names: Vec<SmolStr> = schema
            .fields()
            .iter()
            .map(|column| SmolStr::new(column.name()))
            .collect();
        let records = StreamChunkedSerie::from_arrow_reader(
            extras.is_empty().then_some(&*ROW),
            reader,
            ArrowCastOptions::default(),
        )?;
        let mut read = 0;
        for record in records.into_chunks() {
            let record = record?;
            let leaf = |at: usize| record.child_at(at);
            // The columns a known row is read by, once per batch: the key,
            // the content code and the two stamps a fold moves.
            let known = extras.is_empty().then(|| {
                (
                    leaf(CROSSCODE).and_then(Serie::as_utf8),
                    leaf(HASHCODE).and_then(Serie::as_uint64),
                    leaf(FIRSTUNIX).and_then(Serie::as_datetime_nanosecond),
                    leaf(LASTUNIX).and_then(Serie::as_datetime_nanosecond),
                )
            });
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
                // A row whose content code is the held instrument's states
                // what is held: the content code digests every fact, so only
                // the stamps can move, and they move where the instrument
                // stands - nothing built, nothing compared fact by fact.
                if let Some((Some(codes), Some(hashes), firsts, lasts)) = &known
                    && let (Some(code), Some(hash)) = (codes.value(row), hashes.value(row))
                    && let Some(held) = self.table.rows.get(code)
                    && held.hashcode == hash
                {
                    let firstunix =
                        earlier(held.firstunix, firsts.and_then(|stamps| stamps.value(row)));
                    let lastunix = later(held.lastunix, lasts.and_then(|stamps| stamps.value(row)));
                    if held.firstunix != firstunix || held.lastunix != lastunix {
                        let held = Arc::make_mut(&mut self.table.rows)
                            .get_mut(code)
                            .expect("the code was found");
                        held.firstunix = firstunix;
                        held.lastunix = lastunix;
                        self.moved = true;
                    }
                    read += 1;
                    continue;
                }
                let cells = record.scalar(row).map_err(located)?;
                let cells = cells.sequence_rows().ok_or_else(|| {
                    located(Error::InvalidRecord {
                        path: SmolStr::new_static("$"),
                        reason: SmolStr::new_static("expected an instrument row"),
                    })
                })?;
                let entry = if extras.is_empty() {
                    Instrument::from_cells(&cells).map_err(located)?
                } else {
                    let typed = Scalar::from_struct(
                        names
                            .iter()
                            .zip(cells.iter())
                            .filter(|(name, _)| column_at(name).is_some())
                            .map(|(name, cell)| (name.clone(), cell.clone())),
                    )
                    .map_err(located)?;
                    let mut entry = Instrument::from_scalar(&typed).map_err(located)?;
                    for (at, name) in &extras {
                        let cell = &cells[*at];
                        if cell.is_null() {
                            continue;
                        }
                        // A text cell as it is; any other as its JSON text.
                        let text = match cell.as_str() {
                            Some(text) => Cow::Borrowed(text),
                            None => Cow::Owned(
                                yggdryl::into_json_scalar(cell)
                                    .map_err(|error| located(relocated(name, error)))?,
                            ),
                        };
                        entry
                            .set_metadata(name, &text)
                            .map_err(|error| located(relocated(name, error)))?;
                    }
                    entry
                };
                self.merge(entry).map_err(located)?;
                read += 1;
            }
        }
        Ok(read)
    }

    /// Folds the rows `handle` holds in: the handle's own record stream,
    /// read through [`Self::extend_from_arrow_reader`] under the handle's
    /// own options; a missing store reads as the empty stream. Answers how
    /// many rows it read.
    ///
    /// # Errors
    ///
    /// What the handle's read or [`Self::extend_from_arrow_reader`] refuses.
    pub fn extend_from_handle(&mut self, handle: &dyn IOBase) -> Result<usize> {
        let reader = handle.read_arrow_reader(&handle.record_options()?)?;
        Ok(self.extend_from_arrow_reader(reader)?)
    }

    /// The instruments as a stream under [`Instrument::field`], in cross
    /// code order: a snapshot of the table as it stands, laid out one
    /// bounded batch at a time.
    ///
    /// # Errors
    ///
    /// What laying the rows out refuses, which no held value causes.
    pub fn into_arrow_reader(&self) -> yggdryl::arrow::Result<BatchReader> {
        yggdryl::implementer::reader(&FIELD, Snapshot::of(&self.table), None, None, None)
    }
}

/// The rows of one snapshot, in code order, each as its ordered row.
struct Snapshot {
    rows: Arc<BTreeMap<Str, Instrument>>,
    /// The code of the row last answered.
    after: Option<Str>,
}

impl Snapshot {
    fn of(table: &InstrumentTable) -> Self {
        Self {
            rows: Arc::clone(&table.rows),
            after: None,
        }
    }
}

impl Iterator for Snapshot {
    type Item = Scalar;

    fn next(&mut self) -> Option<Scalar> {
        let (code, instrument) = match &self.after {
            Some(after) => self
                .rows
                .range::<str, _>((Bound::Excluded(after.as_str()), Bound::Unbounded))
                .next()?,
            None => self.rows.iter().next()?,
        };
        let row = instrument.into_row();
        self.after = Some(code.clone());
        Some(row)
    }
}

/// The name of the column at `at` in [`Instrument::field`].
pub(crate) fn column_name(at: usize) -> &'static str {
    NAMES[at]
}

/// The place of the column named `name` in [`Instrument::field`], whatever
/// its case.
pub(crate) fn column_at(name: &str) -> Option<usize> {
    NAMES
        .iter()
        .position(|held| yggdryl::implementer::folds_equal(held, name))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/market/tests/root/instrument.rs` pins and a caller cannot
    //! reach.

    /// What one instrument may take at most, in bytes.
    pub const ENTRY_CHARGE: usize = super::ENTRY_CHARGE;
}
