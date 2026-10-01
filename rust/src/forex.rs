//! ISO 4217 currency pairs: the code of a foreign exchange instrument, and
//! the reading of a venue's FX symbol into one.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::code::code_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{Ccy, DataType, Result, Scalar, StringEnum, Value};

/// One validated currency pair, spelled `CCY/CCY`.
///
/// The base currency, a solidus and the quote currency - `EUR/USD`: how many
/// units of the quote one unit of the base buys. Both legs are ISO 4217
/// codes the crate lists ([`StringEnum::CURRENCIES`]) - a digital-asset
/// ticker is a [`Ccy`] but no leg, so `BTC/USDT` is no pair - the legs differ, and
/// neither is `XXX` (no currency) or `XTS` (the testing code): a pair of
/// those names no instrument. The precious metals - `XAU`, `XAG`, `XPT`,
/// `XPD` - are currencies to ISO 4217 and legs to this code, and
/// [`Self::is_metal`] says when a pair is one.
///
/// Every spelling a feed writes reads: `EUR/USD`, `EURUSD`, `EUR-USD`,
/// `EUR.USD`, `EUR_USD`, in any case, with ASCII whitespace around it, and
/// canonicalizes to the one stored spelling, so a column of pairs holds one
/// spelling and a value compares by it. Anything past the pair - a tenor, a
/// yellow key, a RIC's `=` - is a symbol rather than a code, and
/// [`FxSymbol::from_symbol`] reads those.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Forex(SmolStr);

impl<'de> Deserialize<'de> for Forex {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

/// The canonical seven bytes of a pair: three, a solidus, three.
type Canonical = [u8; FOREX_WIDTH];

/// The three upper-case letters of one leg.
type Leg = [u8; 3];

/// The separators a pair may spell between its legs beside the canonical
/// solidus.
const SEPARATORS: [u8; 4] = *b"/-._";

/// The ISO 4217 codes that are currencies but name no instrument's leg.
const NO_CURRENCY: [&str; 2] = ["XXX", "XTS"];

/// The ISO 4217 codes of the precious metals.
const METALS: [&str; 4] = ["XAU", "XAG", "XPT", "XPD"];

impl Forex {
    /// Validate and construct a currency pair from any accepted spelling.
    ///
    /// ```
    /// use yggdryl::Forex;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let pair = Forex::new("eurusd")?;
    /// assert_eq!(pair.as_str(), "EUR/USD");
    /// assert_eq!(Forex::new(" EUR-USD ")?, pair);
    /// assert_eq!(pair.base().as_str(), "EUR");
    /// assert_eq!(pair.quote().as_str(), "USD");
    /// assert!(Forex::new("XAU/USD")?.is_metal());
    /// assert!(Forex::new("EUR/EUR").is_err());
    /// assert!(Forex::new("EUR/USD 1M").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the text when it is not two distinct ISO 4217
    /// currencies, neither `XXX` nor `XTS`, spelled as one of the accepted
    /// pairs.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        // The NUL padding a fixed slot leaves goes first, as every code's
        // does, then the whitespace a feed writes around a pair.
        let text = value
            .as_ref()
            .trim_end_matches('\0')
            .trim_matches(|c: char| c.is_ascii_whitespace());
        let canonical =
            canonical_pair(text.as_bytes()).ok_or_else(|| crate::Error::InvalidDataType {
                kind: "forex",
                reason: format_smolstr!(
                    "expected a pair of two distinct ISO 4217 currencies, CCY/CCY, got {text:?}"
                ),
            })?;
        Ok(Self(SmolStr::new(
            std::str::from_utf8(&canonical).expect("two ASCII legs and a solidus"),
        )))
    }

    /// Borrow the canonical pair.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the pair.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// Whether `text` is already the spelling a code stores: `CCY/CCY`, upper
    /// case, both legs valid, nothing around it.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        canonical_pair(text.as_bytes()).is_some_and(|canonical| canonical == text.as_bytes())
    }

    /// The base currency: the first leg, of which one unit is priced.
    #[must_use]
    pub fn base(&self) -> Ccy {
        Ccy::new(&self.as_str()[..3]).expect("a validated leg")
    }

    /// The quote currency: the second leg, in which the base is priced.
    #[must_use]
    pub fn quote(&self) -> Ccy {
        Ccy::new(&self.as_str()[4..]).expect("a validated leg")
    }

    /// Whether either leg is a precious metal - `XAU`, `XAG`, `XPT`, `XPD` -
    /// which ISO 4217 lists as a currency and the market classifies as a
    /// commodity.
    #[must_use]
    pub fn is_metal(&self) -> bool {
        let text = self.as_str();
        METALS.contains(&&text[..3]) || METALS.contains(&&text[4..])
    }
}

/// The canonical bytes of the pair a region spells, or `None` where the
/// region is no pair: six letters, or seven with a separator between the
/// legs, each leg an ISO 4217 currency this crate lists, neither `XXX` nor
/// `XTS`, and the two distinct.
///
/// Allocation-free: two legs on the stack and two binary searches.
fn canonical_pair(region: &[u8]) -> Option<Canonical> {
    let (base, quote) = match region.len() {
        6 => (&region[..3], &region[3..]),
        7 if SEPARATORS.contains(&region[3]) => (&region[..3], &region[4..]),
        _ => return None,
    };
    let base = leg(base)?;
    let quote = leg(quote)?;
    if base == quote {
        return None;
    }
    let mut canonical = [b'/'; FOREX_WIDTH];
    canonical[..3].copy_from_slice(&base);
    canonical[4..].copy_from_slice(&quote);
    Some(canonical)
}

/// The upper-cased leg three bytes spell, or `None` where they are not
/// three ASCII letters of a currency an instrument can be priced in.
fn leg(bytes: &[u8]) -> Option<Leg> {
    let mut leg = [0_u8; 3];
    for (target, byte) in leg.iter_mut().zip(bytes) {
        if !byte.is_ascii_alphabetic() {
            return None;
        }
        *target = byte.to_ascii_uppercase();
    }
    let text = std::str::from_utf8(&leg).expect("three ASCII letters");
    if NO_CURRENCY.contains(&text) {
        return None;
    }
    StringEnum::CURRENCIES.binary_search(&text).ok()?;
    Some(leg)
}

impl fmt::Display for Forex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Forex, Forex, FOREX_WIDTH);

/// The Arrow extension name of a currency pair.
pub(crate) const FOREX_EXTENSION_NAME: &str = "yggdryl.forex";

/// The most bytes a currency pair may be: two three-letter legs and the
/// solidus between them, the one spelling a column stores.
pub(crate) const FOREX_WIDTH: usize = 7;

impl DataType {
    /// Creates the currency pair datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::forex(), DataType::Forex);
    /// assert_eq!(DataType::forex().to_string(), "forex");
    /// assert_eq!(DataType::forex().code_width(), Some(7));
    /// ```
    #[must_use]
    pub const fn forex() -> Self {
        Self::Forex
    }
}

define_field_types!(ForexType, Forex);

// ------------------------------------------------------------------------
// The FX symbol: a pair, and what the venue said around it.
// ------------------------------------------------------------------------

/// What a venue's FX symbol says about when the pair settles.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FxTenor {
    /// A spot pair: a RIC's `=` or `=R`, or a `SPOT`, `SP`, `TOD` or `TOM`
    /// suffix.
    Spot,
    /// An outright forward: a `1M`-style tenor, `SN`, `SW` or `BROKEN`.
    Forward,
    /// A symbol that states the pair and nothing about its tenor.
    Unstated,
}

/// A venue's FX symbol read into the pair it names, the tenor it states and
/// the FIX `SettlType(63)` that tenor spells.
///
/// The one detector: it reads `Symbol(55)` and every other place a pair is
/// spelled with something around it, allocation-free, and answers `None` for
/// anything that is not one pair - a single currency's RIC (`EUR=`), a tenor
/// glued to one leg (`EUR1M=`), a suffix nothing here names.
///
/// ```
/// use yggdryl::{FxSymbol, FxTenor};
///
/// let forward = FxSymbol::from_symbol("EUR-USD 1M").unwrap();
/// assert_eq!(forward.forex.as_str(), "EUR/USD");
/// assert_eq!(forward.tenor, FxTenor::Forward);
/// assert_eq!(forward.settltype.as_deref(), Some("M1"));
/// let ric = FxSymbol::from_symbol("EURGBP=R").unwrap();
/// assert_eq!(ric.tenor, FxTenor::Spot);
/// assert_eq!(ric.settltype, None);
/// assert_eq!(FxSymbol::from_symbol("EUR=").map(|held| held.forex), None);
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FxSymbol {
    /// The pair the symbol names.
    pub forex: Forex,
    /// What the symbol says about when the pair settles.
    pub tenor: FxTenor,
    /// FIX's `SettlType(63)` for the tenor the symbol spells - `0` for spot,
    /// `M1` for a one-month forward - where a suffix spells one.
    pub settltype: Option<SmolStr>,
}

impl FxSymbol {
    /// The pair a venue's symbol names, with the tenor it states, or `None`
    /// where the symbol names no pair.
    ///
    /// The pair region runs to the first space or `=` after the trimmed
    /// text opens and reads as [`Forex::new`] reads. After an `=`, nothing
    /// or `R` is a spot RIC and anything else no pair. After spaces, one
    /// suffix in any case: `SPOT` or `SP` (spot, `0`), `TOD` (`1`), `TOM`
    /// (`2`), `SN` (forward, `C`), `SW` (`W1`), `BROKEN` (`B`), one to three
    /// digits then `D`, `W`, `M` or `Y` (forward, the unit then the count:
    /// `1M` is `M1`), `CURNCY` (Bloomberg's yellow key, tenor unstated), `ON`
    /// or `TN` (tenor unstated); any other suffix is no pair. No suffix
    /// states no tenor.
    #[must_use]
    pub fn from_symbol(text: &str) -> Option<Self> {
        let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
        let end = text
            .bytes()
            .position(|byte| byte == b' ' || byte == b'=')
            .unwrap_or(text.len());
        let canonical = canonical_pair(&text.as_bytes()[..end])?;
        let forex = Forex(SmolStr::new(
            std::str::from_utf8(&canonical).expect("two ASCII legs and a solidus"),
        ));
        let rest = &text[end..];
        let (tenor, settltype) = if let Some(after) = rest.strip_prefix('=') {
            match after {
                "" | "R" => (FxTenor::Spot, None),
                _ => return None,
            }
        } else {
            let suffix = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
            suffix_tenor(suffix)?
        };
        Some(Self {
            forex,
            tenor,
            settltype,
        })
    }
}

/// The tenor and settlement type one suffix spells, `None` for a suffix
/// this reading does not know; no suffix states no tenor.
fn suffix_tenor(suffix: &str) -> Option<(FxTenor, Option<SmolStr>)> {
    if suffix.is_empty() {
        return Some((FxTenor::Unstated, None));
    }
    let spot =
        |settltype: &'static str| Some((FxTenor::Spot, Some(SmolStr::new_static(settltype))));
    let forward =
        |settltype: &'static str| Some((FxTenor::Forward, Some(SmolStr::new_static(settltype))));
    if suffix.eq_ignore_ascii_case("SPOT") || suffix.eq_ignore_ascii_case("SP") {
        return spot("0");
    }
    if suffix.eq_ignore_ascii_case("TOD") {
        return spot("1");
    }
    if suffix.eq_ignore_ascii_case("TOM") {
        return spot("2");
    }
    if suffix.eq_ignore_ascii_case("SN") {
        return forward("C");
    }
    if suffix.eq_ignore_ascii_case("SW") {
        return forward("W1");
    }
    if suffix.eq_ignore_ascii_case("BROKEN") {
        return forward("B");
    }
    if suffix.eq_ignore_ascii_case("CURNCY")
        || suffix.eq_ignore_ascii_case("ON")
        || suffix.eq_ignore_ascii_case("TN")
    {
        return Some((FxTenor::Unstated, None));
    }
    // One to three digits then a unit letter: `1M`, `12M`, `2Y`, `3W`, `10D`.
    let bytes = suffix.as_bytes();
    let (count, unit) = bytes.split_at(bytes.len().checked_sub(1)?);
    if (1..=3).contains(&count.len())
        && count.iter().all(u8::is_ascii_digit)
        && matches!(unit[0].to_ascii_uppercase(), b'D' | b'W' | b'M' | b'Y')
    {
        let count = std::str::from_utf8(count).expect("ASCII digits");
        return Some((
            FxTenor::Forward,
            Some(format_smolstr!(
                "{}{count}",
                unit[0].to_ascii_uppercase() as char
            )),
        ));
    }
    None
}
