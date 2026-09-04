//! The logical names: one more spelling for the closest core datatype.
//!
//! A registration is a *name*, never a type. `price` parses to
//! `decimal64(18,8)` and displays as `decimal64(18,8)`, so the grammar keeps
//! one canonical spelling per datatype and nothing downstream learns a new
//! variant. That is what makes the registry cheap: it is a lookup table in
//! front of the parser, and every path after it sees an ordinary
//! [`DataType`].
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
//! | `Currency` | String | `ascii32` | ISO 4217 alpha-3, stored `USD\0` |
//! | `Country` | String | `ascii32` | ISO 3166-1 alpha-2, stored `FR\0\0` |
//! | `Exchange`, `mic` | String | `ascii32` | ISO 10383 MIC, exactly 4 bytes |
//! | `Language` | String | `ascii32` | ISO 639-1 alpha-2 |
//! | `MonthYear` | String | `ascii64` | `YYYYMM`, `YYYYMMDD`, or `YYYYMMWW` |
//! | `Tenor` | Pattern | `ascii64` | `D5`, `W2`, `M3`, `Y1` |
//! | `Pattern` | - | `utf8` | the abstract base of `Tenor` and the reserved ranges |
//! | `Length` | int | `int32` | a byte count |
//! | `TagNum` | int | `int32` | a FIX tag |
//! | `SeqNum` | int | `int64` | a session sequence number outgrows `int32` |
//! | `NumInGroup` | int | `int32` | a repeating-group counter |
//! | `DayOfMonth` | int | `int8` | 1 through 31 |
//! | `Reserved100Plus` | Pattern | `int32` | a user-defined enumeration value |
//! | `Reserved1000Plus` | Pattern | `int32` | as above |
//! | `Reserved4000Plus` | Pattern | `int32` | as above |
//! | `Qty` | float | `decimal64(18,8)` | exact, 8 bytes |
//! | `Price` | float | `decimal64(18,8)` | exact, 8 bytes |
//! | `PriceOffset` | float | `decimal64(18,8)` | exact and signed |
//! | `Percentage` | float | `decimal64(18,8)` | `0.0525` is 5.25% |
//! | `Amt` | float | `decimal128(38,8)` | a notional outgrows 10 integer digits |
//! | `UTCTimestamp` | String | `timestamp(ns,"UTC")` | the instant, at the finest FIX width |
//! | `TZTimestamp` | String | `timestamp(ns,"UTC")` | the offset resolves into the instant |
//! | `UTCTimeOnly` | String | `time64(ns)` | a time of day with a fraction |
//! | `LocalMktTime` | String | `time32(s)` | `HH:MM:SS`, no fraction |
//! | `UTCDateOnly` | String | `date32` | a calendar day |
//! | `LocalMktDate` | String | `date32` | a calendar day |
//! | `TZTimeOnly` | String | `ascii128` | a time of day plus an offset has no Arrow type |
//! | `MultipleCharValue` | char | `utf8` | space-delimited members |
//! | `MultipleStringValue` | String | `utf8` | space-delimited members |
//! | `XID` | String | `utf8` | an XML identifier |
//! | `XIDREF` | String | `utf8` | a reference to one |
//! | `data` | - | `binary` | opaque bytes |
//! | `XMLData` | data | `binary` | an XML document, opaque here |
//!
//! The float family is exact rather than binary floating point because a
//! price that does not round-trip is a broken trade. `decimal64` holds 18
//! digits in 8 bytes, which is 10 integer digits beside the 8 fractional ones
//! every listed venue's tick fits in; a notional needs more integer room, so
//! `Amt` widens to `decimal128`. A venue outside those bounds declares its own
//! `decimal(precision,scale)`, which is why these are names over the ordinary
//! constructors and not a second numeric model.
//!
//! `TZTimestamp` keeps the instant and drops the local offset, because an
//! Arrow column carries one zone for every row. Read it under
//! `timestamp(ns,"<zone>")` when the local reading is the value.

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
        // The ISO code vocabularies, over the ASCII widths.
        ("currency", DataType::Ascii32),
        ("country", DataType::Ascii32),
        ("mic", DataType::Ascii32),
        ("exchange", DataType::Ascii32),
        ("language", DataType::Ascii32),
        ("monthyear", DataType::Ascii64),
        ("tenor", DataType::Ascii64),
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
        // The float family, exact because a price is money.
        (
            "qty",
            DataType::Decimal64 {
                precision: 18,
                scale: 8,
            },
        ),
        (
            "price",
            DataType::Decimal64 {
                precision: 18,
                scale: 8,
            },
        ),
        (
            "priceoffset",
            DataType::Decimal64 {
                precision: 18,
                scale: 8,
            },
        ),
        (
            "percentage",
            DataType::Decimal64 {
                precision: 18,
                scale: 8,
            },
        ),
        (
            "amt",
            DataType::Decimal128 {
                precision: 38,
                scale: 8,
            },
        ),
        // The temporals.
        (
            "utctimestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        ),
        (
            "tztimestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        ),
        ("utctimeonly", DataType::Time64(TimeUnit::Nanosecond)),
        ("localmkttime", DataType::Time32(TimeUnit::Second)),
        ("utcdateonly", DataType::Date32),
        ("localmktdate", DataType::Date32),
        ("tztimeonly", DataType::Ascii128),
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
    /// assert_eq!(price, DataType::decimal64(18, 8)?);
    /// assert_eq!(price.to_string(), "decimal64(18,8)");
    ///
    /// // The same lookup backs the grammar, so a name types a column.
    /// let row: DataType = "struct<ccy: Currency, venue: MIC, px: Price, at: UTCTimestamp>".parse()?;
    /// assert_eq!(row.get_field_by_path("venue").map(|field| field.dtype().clone()), Some(DataType::Ascii32));
    /// assert_eq!(
    ///     row.get_field_by_path("at").map(|field| field.dtype().clone()),
    ///     Some(DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)))
    /// );
    ///
    /// // Separators and case are folded, exactly as elsewhere in the grammar.
    /// assert_eq!(DataType::from_logical_name(" utc_date_only ")?, DataType::Date32);
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
