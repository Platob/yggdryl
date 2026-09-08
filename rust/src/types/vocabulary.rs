//! The logical names: one more spelling for the closest core datatype.
//!
//! A registration is a *name*, never a type. `price` parses to `float64` and
//! displays as `float64`, so the grammar keeps
//! one canonical spelling per datatype and nothing downstream learns a new
//! variant. That is what makes the registry cheap: it is a lookup table in
//! front of the parser, and every path after it sees an ordinary
//! [`DataType`].
//!
//! Seven of the names resolve to a datatype spelled the same way - `currency`
//! to [`DataType::Currency`], `country`, `mic`, `cfi`, `side`, `msgtype` and
//! `direction` likewise - because those registered codes are types rather
//! than widths. That is not
//! a second rule: the registry still answers a datatype, and the canonical
//! spelling of that datatype still happens to be what was asked for.
//!
//! The vocabulary is the FIX Latest datatype table, so a FIX field
//! declaration types a column directly, plus `mic` - ISO 10383's own name for
//! what FIX calls `Exchange` - because a MIC column is a MIC column whatever
//! protocol delivered it.
//!
//! Five FIX base types already have a meaning in the Arrow/SQL grammar and
//! keep it, because a schema string must not change meaning under a reader:
//! `int` is `int32`, `float` is `float32`, `char` and `string` are `utf8`, and
//! `boolean` is `boolean`. The FIX types derived from `int` and `float` do get
//! registrations, and those carry the precision the base type does not.
//!
//! | FIX | base | resolves to | why |
//! | --- | --- | --- | --- |
//! | `Currency` | String | `currency` | ISO 4217 alpha-3, its own three bytes |
//! | `Country` | String | `country` | ISO 3166-1 alpha-2, its own two bytes |
//! | `Exchange`, `mic` | String | `mic` | ISO 10383 MIC, exactly 4 bytes |
//! | `cfi` | - | `cfi` | ISO 10962, exactly 6 bytes |
//! | `Side` | char | `side` | a code set the standard declares, 4 bytes |
//! | `MsgType` | String | `msgtype` | a code set the standard declares, 8 bytes |
//! | `direction` | - | `direction` | which way a captured line moved, 4 bytes |
//! | `Language` | String | `ascii(2)` | ISO 639-1 alpha-2 |
//! | `MonthYear` | String | `ascii(8)` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
//! | `Tenor` | Pattern | `ascii(8)` | `D5`, `W2`, `M3`, `Y1` |
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
//! | `LocalMktTime` | String | `time32(s)` | `HH:MM:SS`, no fraction |
//! | `UTCDateOnly` | String | `date32` | a calendar day |
//! | `LocalMktDate` | String | `date32` | a calendar day |
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

use smol_str::format_smolstr;

use crate::{DataType, Error, Result, TimeUnit, Timezone};

use super::parser::normalized;

impl DataType {
    /// The logical names, in registration order, paired with what they resolve to.
    ///
    /// The names are stored in their normalized spelling - lowercase, with no
    /// `_`, `-`, or space - which is the form [`Self::from_logical_name`]
    /// folds a caller's spelling into, so `UTCTimestamp`, `utc_timestamp`,
    /// and `UTC Timestamp` are one name.
    pub const LOGICAL_NAMES: &'static [(&'static str, DataType)] = &[
        // Four ISO code vocabularies are datatypes of their own, so their
        // names resolve to themselves and display as themselves; `exchange`
        // is FIX's name for the one ISO 10383 calls `mic`.
        ("currency", DataType::Currency),
        ("country", DataType::Country),
        ("mic", DataType::Mic),
        ("exchange", DataType::Mic),
        ("cfi", DataType::Cfi),
        // Three more that resolve to themselves. `side` and `msgtype` are FIX
        // code sets the standard itself declares, addressed constantly enough
        // to earn a packed datatype; `direction` is transport rather than FIX,
        // because every captured line has one whatever protocol it carried.
        ("side", DataType::Side),
        ("msgtype", DataType::MsgType),
        ("msgdirection", DataType::MsgDirection),
        // What state one thing is in, and how long an order stands. Neither
        // is a word the Arrow or SQL grammar owns.
        ("state", DataType::State),
        ("timeinforce", DataType::TimeInForce),
        // The spelling this datatype was first published under.
        ("direction", DataType::MsgDirection),
        // The rest are names over an ASCII width, which is all they need.
        ("language", DataType::FixedAscii(2)),
        ("monthyear", DataType::FixedAscii(8)),
        ("tenor", DataType::FixedAscii(8)),
        ("pattern", DataType::Utf8),
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
        ("multiplecharvalue", DataType::Utf8),
        ("multiplestringvalue", DataType::Utf8),
        ("xid", DataType::Utf8),
        ("xidref", DataType::Utf8),
        ("data", DataType::Binary),
        ("xmldata", DataType::Binary),
    ];

    /// Resolves a registered logical name to the datatype it spells.
    ///
    /// The name folds the way every other datatype name in the grammar does:
    /// trimmed, ASCII case-insensitive, and with `_`, `-`, and spaces
    /// ignored. This is the same lookup [`Self::from_str`] falls back to, so a
    /// name resolves identically alone and inside an expression.
    ///
    /// ```
    /// use yggdryl::{DataType, TimeUnit, Timezone};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // One canonical spelling: a name resolves to a datatype and displays
    /// // as that datatype.
    /// let price = DataType::from_logical_name("Price")?;
    /// assert_eq!(price, DataType::Float64);
    /// assert_eq!(price.to_string(), "float64");
    ///
    /// // The same lookup backs the grammar, so a name types a column. Four
    /// // of the names answer a datatype of their own rather than a width.
    /// let row: DataType = "struct<ccy: Currency, venue: MIC, px: Price, at: UTCTimestamp>".parse()?;
    /// assert_eq!(row.get_field_by_path("venue").map(|field| field.dtype().clone()), Some(DataType::Mic));
    /// assert_eq!(row.get_field_by_path("ccy").map(|field| field.dtype().clone()), Some(DataType::Currency));
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

/// The registry lookup over a name the caller already folded.
///
/// The parser folds every datatype word before dispatching on it, so it holds
/// the folded spelling already; this is that one lookup, shared rather than
/// repeated, and it is why resolving a name inside an expression costs no
/// second normalization.
pub(super) fn folded_logical_name(folded: &str) -> Option<DataType> {
    DataType::LOGICAL_NAMES
        .iter()
        .find(|(registered, _)| *registered == folded)
        .map(|(_, dtype)| dtype.clone())
}

/// The registered names in registration order, for the refusal to name.
fn logical_vocabulary() -> String {
    DataType::LOGICAL_NAMES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}
