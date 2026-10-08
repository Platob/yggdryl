//! The logical names: one more spelling for the closest core datatype.
//!
//! A registration is a *name*, never a type. `price` parses to `float64` and
//! displays as `float64`, so the grammar keeps
//! one canonical spelling per datatype and nothing downstream learns a new
//! variant. That is what makes the registry cheap: it is a claim-once
//! register in front of the parser, and every path after it sees an
//! ordinary [`DataType`]. The core's names are claimed before the register
//! answers anything; a crate claims more through
//! [`DataType::register_logical_name`], each name once.
//!
//! Some names resolve to their own datatype: `ccy`, `country`, `mic`,
//! `cfi` and `unit` are registered codes, carrying an identity as well as
//! their storage width; `side`, `marketdatakind`, `marketdatatype` and
//! `timeinforce` are registered enum kinds answered from the market
//! register, each the code of a member of its closed set; and `state` is
//! the core's own enum leaf.
//!
//! The vocabulary follows the FIX Latest datatype table, plus `mic` -
//! ISO 10383's name for what FIX calls `Exchange`. The dictionary generator
//! resolves FIX's `Currency` source type to `ccy`; schema declarations use
//! `ccy` directly.
//!
//! Five FIX base types already have a meaning in the Arrow/SQL grammar and
//! keep it, because a schema string must not change meaning under a reader:
//! `int` is `int32`, `float` is `float32`, `char` and `string` are `utf8`, and
//! `boolean` is `boolean`. The FIX types derived from `int` and `float` do get
//! registrations, and those carry the precision the base type does not.
//!
//! | FIX | base | resolves to | why |
//! | --- | --- | --- | --- |
//! | `Currency` (dictionary source) | String | `ccy` | ISO 4217 alpha-3 or a digital-asset ticker, at most eight bytes |
//! | `Country` | String | `country` | ISO 3166-1 alpha-2, its own two bytes |
//! | `Exchange`, `mic` | String | `mic` | ISO 10383 MIC, exactly 4 bytes |
//! | `cfi` | - | `cfi` | ISO 10962, exactly 6 bytes |
//! | `Side` | char | `side` | a code set the standard declares, stored as its `uint8` code |
//! | `UnitOfMeasure` | String | `unit` | the unit a quantity is stated in, at most 32 bytes |
//! | `Language` | String | `fixed_ascii(2)` | ISO 639-1 alpha-2 |
//! | `MonthYear` | String | `fixed_ascii(8)` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
//! | `Tenor` | Pattern | `fixed_ascii(8)` | `D5`, `W2`, `M3`, `Y1` |
//! | `Pattern` | - | `utf8` | the abstract base of `Tenor` and the reserved ranges |
//! | `Length` | int | `int32` | a byte count |
//! | `TagNum` | int | `int32` | a FIX tag |
//! | `SeqNum` | int | `int64` | a session sequence number outgrows `int32` |
//! | `NumInGroup` | int | `int32` | a repeating-group counter |
//! | `DayOfMonth` | int | `int8` | 1 through 31 |
//! | `Reserved100Plus` | Pattern | `int32` | a user-defined enumeration value |
//! | `Reserved1000Plus` | Pattern | `int32` | as above |
//! | `Reserved4000Plus` | Pattern | `int32` | as above |
//! | `Qty` | float | `float64` | the specification states no scale |
//! | `Price` | float | `float64` | as above |
//! | `PriceOffset` | float | `float64` | as above, signed |
//! | `Percentage` | float | `float64` | `0.0525` is 5.25% |
//! | `Amt` | float | `float64` | one width, so the family is arithmetic |
//! | `UTCTimestamp` | String | `datetime64(ns,"UTC")` | the instant, at the finest FIX width |
//! | `TZTimestamp` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant |
//! | `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
//! | `LocalMktTime` | String | `time64(ns)` | a time of day, one type with `UTCTimeOnly` |
//! | `UTCDateOnly`, `utcdate` | String | `datetime64(ns,"UTC")` | that day at midnight, in UTC |
//! | `LocalMktDate` | String | `datetime64(ns)` | that day at midnight, stating no zone |
//! | `LocalMktDatetime` | String | `datetime64(ns)` | a local instant, stating no zone |
//! | `TZTimeOnly` | String | `datetime64(ns,"UTC")` | the offset resolves into the instant, on the epoch day |
//! | `MultipleCharValue` | char | `utf8` | space-delimited members |
//! | `MultipleStringValue` | String | `utf8` | space-delimited members |
//! | `XID` | String | `utf8` | an XML identifier |
//! | `XIDREF` | String | `utf8` | a reference to one |
//! | `data` | - | `binary` | opaque bytes |
//! | `XMLData` | data | `binary` | an XML document, opaque here |
//!
//! The float family is `float64` because the specification declares all five
//! as `float` subtypes and states no scale for any of them anywhere. A table
//! whose job is to say what a FIX datatype *is* must not improve on the
//! specification, and a pinned scale is wrong in both directions: it truncates
//! a venue quoting finer than eight places and pads every value that does not,
//! and which venues quote how is a fact about a counterparty rather than about
//! a datatype. One width also keeps the family arithmetic - `Amt` at 128 bits
//! beside `Qty` at 64 would make every consumer multiplying a quantity by a
//! price cast first, per row, forever.
//!
//! The cost, plainly: binary floating point does not hold `0.1`, and a column
//! of `float64` is not where a book's notional should be accumulated over a
//! day. It is 53 bits of mantissa, exact for every integer quantity below
//! `2^53` and for the price grids venues actually quote. A venue needing exact
//! decimal declares its own `decimal(precision,scale)`, which is why these are
//! names over the ordinary constructors and not a second numeric model.
//!
//! `TZTimestamp` keeps the instant and drops the local offset, because an
//! Arrow column carries one zone for every row. Read it under
//! `datetime64(ns,"<zone>")` when the local reading is the value.
//!
//! `TZTimeOnly` is the same instant under a missing date, and the epoch day
//! supplies it: `07:39+05:30` is `1970-01-01T02:09:00Z`. That keeps the
//! reading arithmetic - two of them subtract, one sorts against another, the
//! offset is resolved rather than carried as text - where the fixed-width
//! ASCII it used to be kept none of it, and did not even hold the type: the
//! widest legal `TZTimeOnly` is eighteen bytes and the box was sixteen.

use std::sync::OnceLock;

use smol_str::format_smolstr;

use crate::plugin::Register;
use crate::{DataType, Error, Result, TimeUnit, Timezone};

use crate::parser::normalized;

/// What the core claims its own names as.
const CORE: &str = "yggdryl";

/// Every claimed logical name, keyed by its folded spelling: the core's own -
/// its codes included - and every name a crate claimed since. Each value
/// carries its name, so a listing names it.
static LOGICAL_NAMES: Register<&'static str, (&'static str, DataType)> =
    Register::new("logical name");
static SEEDED: OnceLock<()> = OnceLock::new();

/// A fixed US-ASCII width, spelled once for the listing below.
///
/// Every width in the listing is a literal above zero, so the leaf is built
/// without the validation `DataType::fixed_ascii` runs.
const fn fixed_ascii(width: u32) -> DataType {
    DataType::FixedAsciiString(width)
}

/// The core's own logical names over its own datatypes, claimed first when
/// the register is seeded. The registered enum kinds' names are the kinds'
/// own, answered from the market register beside these.
///
/// The names are stored in their normalized spelling - lowercase, with no
/// `_`, `-`, or space - which is the form [`DataType::from_logical_name`]
/// folds a caller's spelling into, so `UTCTimestamp`, `utc_timestamp`,
/// and `UTC Timestamp` are one name.
const CORE_NAMES: &[(&str, DataType)] = &[
    // The ISO code vocabularies and the securities identifiers are
    // datatypes of their own, so their names resolve to themselves
    // and display as themselves; `exchange` is FIX's name for the one
    // ISO 10383 calls `mic`.
    ("ccy", DataType::Ccy),
    ("country", DataType::Country),
    ("mic", DataType::Mic),
    ("exchange", DataType::Mic),
    ("cfi", DataType::Cfi),
    ("isin", DataType::Isin),
    ("cusip", DataType::Cusip),
    ("sedol", DataType::Sedol),
    ("bbg", DataType::Bbg),
    ("ric", DataType::Ric),
    ("figi", DataType::Figi),
    // The core's own enum leaf: what state one thing is in, the code of
    // a member of a closed set at the `uint16` width it stores. Not a
    // word the Arrow or SQL grammar owns. `side`, `marketdatakind`,
    // `marketdatatype` and `timeinforce` are registered enum kinds
    // answered from the market register and are never listed here.
    ("state", DataType::State),
    // The unit a quantity is stated in: FIX's `UnitOfMeasure(996)`.
    ("unit", DataType::Unit),
    // The currency pair a foreign exchange instrument is: two legs of
    // `ccy`, the base and the quote.
    ("forex", DataType::Forex),
    // The reference-data codes of ISO TC 68 beside the instrument codes:
    // a legal entity, a business party, a legal form, a digital token and
    // a short name. None is a word the Arrow or SQL grammar owns.
    ("lei", DataType::Lei),
    ("bic", DataType::Bic),
    ("elf", DataType::Elf),
    ("dti", DataType::Dti),
    ("fisn", DataType::Fisn),
    // The rest are names over a fixed US-ASCII width, which is all they
    // need.
    ("language", fixed_ascii(2)),
    ("monthyear", fixed_ascii(8)),
    ("tenor", fixed_ascii(8)),
    ("pattern", DataType::utf8()),
    // The int family, each carrying the range its base type does not.
    ("length", DataType::Int32),
    ("tagnum", DataType::Int32),
    ("seqnum", DataType::Int64),
    ("numingroup", DataType::Int32),
    ("dayofmonth", DataType::Int8),
    ("reserved100plus", DataType::Int32),
    ("reserved1000plus", DataType::Int32),
    ("reserved4000plus", DataType::Int32),
    // The float family, which the specification declares as `float` and
    // states no scale for anywhere.
    //
    // A table whose job is to say what a FIX datatype *is* must not
    // improve on the specification, and a pinned scale is wrong in both
    // directions: it truncates a venue quoting finer than eight places and
    // pads every value that does not, and which venues quote how is a fact
    // about a counterparty rather than about a datatype. Two widths in one
    // family cannot be arithmetic either - `Amt` at 128 bits beside `Qty`
    // at 64 would make every consumer multiplying a quantity by a price
    // cast first, per row, forever.
    //
    // The cost, said plainly: binary floating point does not hold `0.1`,
    // and a column of `float64` is not where a book's notional should be
    // accumulated over a day. It is 53 bits of mantissa - exact for every
    // integer quantity below `2^53` and for the price grids venues
    // actually quote - and nothing is lost by the choice, because the
    // authority for a value is the entry as it arrived: the typed column
    // is a view for arithmetic, and a consumer needing exact decimal reads
    // the entry or casts the column.
    ("qty", DataType::Float64),
    ("price", DataType::Float64),
    ("priceoffset", DataType::Float64),
    ("percentage", DataType::Float64),
    ("amt", DataType::Float64),
    // The temporals.
    (
        "utctimestamp",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "tztimestamp",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    // Every zone-less time of day is one type. A FIX time is ASCII
    // whatever the version, and the two names differ in which clock the
    // value is read against rather than in what it can hold - so pinning
    // one to seconds makes a capture carrying both cast per row to
    // compare them, and loses a millisecond the wire actually sent.
    ("utctimeonly", DataType::Time64(TimeUnit::Nanosecond)),
    ("localmkttime", DataType::Time64(TimeUnit::Nanosecond)),
    // A FIX date is a day, and a day is an instant at midnight rather
    // than a second type to cast through: a capture joining a settlement
    // date to a transact time compares them directly, and a venue that
    // starts sending a time on a field that carried a date widens no
    // column. The zone is the one the name states - a UTC date is UTC,
    // and a local market date states none, so it must not claim one.
    (
        "utcdate",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "utcdateonly",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    (
        "localmktdate",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
    ),
    // A local market value states no zone, so it must not claim one.
    // `Naive` is that statement made explicitly rather than by omission.
    (
        "localmktdatetime",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
    ),
    (
        "tztimeonly",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
    ),
    // The remaining text and binary shapes.
    ("multiplecharvalue", DataType::utf8()),
    ("multiplestringvalue", DataType::utf8()),
    ("xid", DataType::utf8()),
    ("xidref", DataType::utf8()),
    ("data", DataType::binary()),
    ("xmldata", DataType::binary()),
];

/// The core's names claimed, once, before the register answers anything.
fn seed() {
    SEEDED.get_or_init(|| {
        for (name, dtype) in CORE_NAMES.iter().cloned() {
            // The core states each of its names once.
            LOGICAL_NAMES
                .claim(name, (name, dtype), CORE)
                .expect("the core's own logical names claim cleanly");
        }
    });
}

impl DataType {
    /// Every logical name, in name order, paired with what it resolves to:
    /// the core's own, every registered enum kind's under its own name, and
    /// every name a crate claimed through [`Self::register_logical_name`].
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// let names = DataType::logical_names();
    /// assert!(names.contains(&("price", DataType::Float64)));
    /// assert!(names.contains(&("isin", DataType::isin())));
    /// assert!(names.contains(&("exchange", DataType::mic())));
    /// ```
    #[must_use]
    pub fn logical_names() -> Vec<(&'static str, DataType)> {
        seed();
        let mut names = LOGICAL_NAMES.values();
        // A registered enum kind names itself from the market register.
        for kind in crate::market::kinds() {
            if LOGICAL_NAMES.get(kind.name).is_none() {
                names.push((kind.name, kind.dtype()));
            }
        }
        names.sort_by_key(|(name, _)| *name);
        names
    }

    /// Claims `name` for the crate `by`, resolving to `dtype` at every door
    /// a logical name is read: [`Self::from_logical_name`] and the grammar.
    ///
    /// The name is stated in its folded spelling - a lowercase ASCII letter,
    /// then lowercase letters and digits, with no `_`, `-`, or space - which
    /// is what every reader folds a caller's spelling into, and it is
    /// claimed once for the life of the process.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// DataType::register_logical_name("ticksize", DataType::Float64, "venue")?;
    /// assert_eq!(DataType::from_logical_name("Tick_Size")?, DataType::Float64);
    /// assert_eq!("TickSize".parse::<DataType>()?, DataType::Float64);
    /// // A name is claimed once, and a word the grammar owns never.
    /// assert!(DataType::register_logical_name("ticksize", DataType::Int64, "other").is_err());
    /// assert!(DataType::register_logical_name("int", DataType::Int64, "other").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] naming the first claimant where the name
    /// is claimed already, and [`Error::InvalidDataType`] where it is not in
    /// its folded spelling, where the grammar answers it without the
    /// register - a keyword, an enum kind's name - or where `dtype` does
    /// not validate.
    pub fn register_logical_name(
        name: &'static str,
        dtype: DataType,
        by: &'static str,
    ) -> Result<()> {
        let refuse = |expected: &str| Error::InvalidDataType {
            kind: "logical",
            reason: crate::text::expected_got(expected, format_smolstr!("{name:?}")),
        };
        if !name.starts_with(|first: char| first.is_ascii_lowercase())
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(refuse(
                "a name in its folded spelling - a lowercase letter, then lowercase letters and digits",
            ));
        }
        dtype.validate()?;
        seed();
        // The market register seeds under its own lock, so it is seeded
        // before the lock is taken here; then, under that lock from the
        // first read, a name claimed as a kind and as a logical name is a
        // conflict on whichever came second. A word the grammar answers
        // before the register would never reach a name claimed over it; a
        // claimed name is the claim's conflict.
        crate::market::seed();
        let _claiming = crate::market::claiming();
        if LOGICAL_NAMES.get(name).is_none()
            && (crate::market::kind_named(name).is_some() || DataType::from_str(name).is_ok())
        {
            return Err(refuse("a word the datatype grammar does not own"));
        }
        LOGICAL_NAMES.claim(name, (name, dtype), by)
    }

    /// Resolves a registered logical name to the datatype it spells.
    ///
    /// The name folds the way every other datatype name in the grammar does:
    /// trimmed, ASCII case-insensitive, and with `_`, `-`, and spaces
    /// ignored. This is the same lookup [`Self::from_str`] falls back to, so a
    /// name resolves identically alone and inside an expression.
    ///
    /// ```
    /// use yggdryl::{DataType, DateTimeType, TimeUnit, Timezone};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // One canonical spelling: a name resolves to a datatype and displays
    /// // as that datatype.
    /// let price = DataType::from_logical_name("Price")?;
    /// assert_eq!(price, DataType::Float64);
    /// assert_eq!(price.to_string(), "float64");
    ///
    /// // The same lookup backs the grammar, so a name types a column. Some
    /// // of the names answer a datatype of their own rather than a width.
    /// let row: DataType = "struct<ccy: Ccy, venue: MIC, px: Price, at: UTCTimestamp>".parse()?;
    /// assert_eq!(row.get_field_by_path("venue").map(|field| field.dtype().clone()), Some(DataType::Mic));
    /// assert_eq!(row.get_field_by_path("ccy").map(|field| field.dtype().clone()), Some(DataType::Ccy));
    /// assert_eq!(
    ///     row.get_field_by_path("at").map(|field| field.dtype().clone()),
    ///     Some(DataType::DateTime64 {
    ///         unit: TimeUnit::Nanosecond,
    ///         timezone: Timezone::UTC,
    ///     })
    /// );
    ///
    /// // Separators and case are folded, exactly as elsewhere in the grammar.
    /// // A FIX date is that day's midnight, so it resolves to an instant.
    /// assert_eq!(
    ///     DataType::from_logical_name(" utc_date_only ")?,
    ///     DataType::DateTime64 {
    ///         unit: TimeUnit::Nanosecond,
    ///         timezone: Timezone::UTC,
    ///     }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the registered vocabulary when `name` is not in
    /// it.
    pub fn from_logical_name(name: &str) -> Result<Self> {
        folded_logical_name(&normalized(name.trim())).ok_or_else(|| Error::InvalidDataType {
            kind: "logical",
            reason: crate::text::expected_got(
                format_args!("a registered logical name ({})", logical_vocabulary()),
                format_smolstr!("{name:?}"),
            ),
        })
    }
}

/// The register lookup over a name the caller already folded.
///
/// The parser folds every datatype word before dispatching on it, so it holds
/// the folded spelling already; this is that one lookup, shared rather than
/// repeated, and it is why resolving a name inside an expression costs no
/// second normalization. A registered enum kind answers under its own name
/// from the market register.
pub(super) fn folded_logical_name(folded: &str) -> Option<DataType> {
    seed();
    LOGICAL_NAMES
        .get(folded)
        .map(|(_, dtype)| dtype)
        .or_else(|| crate::market::kind_named(folded).map(|kind| kind.dtype()))
}

/// The registered names in name order, for the refusal to name.
fn logical_vocabulary() -> String {
    DataType::logical_names()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>()
        .join(", ")
}
