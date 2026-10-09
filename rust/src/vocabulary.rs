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
//! The core's own names are its codes and `state`. The codes keep the
//! names FIX gives them beside their own: `exchange` is FIX's name for what
//! ISO 10383 calls `mic`, and the dictionary generator resolves FIX's
//! `Currency` source type to `ccy`, which schema declarations use directly.
//! The FIX Latest datatype names - `Qty`, `UTCTimestamp`, `Tenor` and the
//! rest - are the FIX module's own table, which the core's seed claims
//! beside its names until the FIX crate claims it itself; the listing
//! reads them as one, in name order.
//!
//! | FIX | base | resolves to | why |
//! | --- | --- | --- | --- |
//! | `Currency` (dictionary source) | String | `ccy` | ISO 4217 alpha-3 or a digital-asset ticker, at most eight bytes |
//! | `Country` | String | `country` | ISO 3166-1 alpha-2, its own two bytes |
//! | `Exchange`, `mic` | String | `mic` | ISO 10383 MIC, exactly 4 bytes |
//! | `cfi` | - | `cfi` | ISO 10962, exactly 6 bytes |
//! | `Side` | char | `side` | a code set the standard declares, stored as its `uint8` code |
//! | `UnitOfMeasure` | String | `unit` | the unit a quantity is stated in, at most 32 bytes |

use std::sync::OnceLock;

use smol_str::format_smolstr;

use crate::plugin::Register;
use crate::{DataType, Error, Result};

use crate::parser::normalized;

/// What the core claims its own names as.
const CORE: &str = "yggdryl";

/// Every claimed logical name, keyed by its folded spelling: the core's own -
/// its codes included - and every name a crate claimed since. Each value
/// carries its name, so a listing names it.
static LOGICAL_NAMES: Register<&'static str, (&'static str, DataType)> =
    Register::new("logical name");
static SEEDED: OnceLock<()> = OnceLock::new();

/// The core's own logical names over its own datatypes - its codes and
/// `state` - claimed first when the register is seeded, the FIX Latest
/// names of `crate::fix::LOGICAL_NAMES` beside them. The registered enum
/// kinds' names are the kinds' own, answered from the market register
/// beside these.
///
/// The names are stored in their normalized spelling - lowercase, with no
/// `_`, `-`, or space - which is the form [`DataType::from_logical_name`]
/// folds a caller's spelling into, so `Exchange`, `ex_change` and
/// `EXCHANGE` are one name.
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
];

/// The core's names claimed, once, before the register answers anything,
/// and the FIX Latest names beside them: the FIX module's table, which the
/// core claims until the FIX crate claims it itself.
fn seed() {
    SEEDED.get_or_init(|| {
        for (name, dtype) in CORE_NAMES.iter().chain(crate::fix::LOGICAL_NAMES).cloned() {
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
