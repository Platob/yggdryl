//! `Eusipa`: the product category of a structured product - EUSIPA's
//! four-digit code, which the SSPA's Swiss map numbers the same way.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// The refusal a value of no category's shape earns: a value's, located at
/// the value itself, since a category is a value of its own rather than a
/// datatype.
fn refusal(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason,
    }
}

/// The members the European Derivative Map lists, by code: EUSIPA's map of
/// February 2024, each by its English name - the tranches of a credit
/// linked note spelled out where the map continues the line above them.
const EUSIPA_NAMES: [(u16, &str); 33] = [
    (1100, "Uncapped Capital Protection"),
    (1120, "Capped Capital Protection"),
    (1130, "Capital Protection with Knock-Out"),
    (1140, "Capital Protection with Coupon"),
    (1199, "Miscellaneous Capital Protection"),
    (1200, "Discount Certificates"),
    (1210, "Barrier Discount Certificates"),
    (1220, "Reverse Convertibles"),
    (1230, "Barrier Reverse Convertibles"),
    (1240, "Capped Outperformance Certificates"),
    (1250, "Capped Bonus Certificates"),
    (1260, "Express Certificates"),
    (1299, "Miscellaneous Yield Enhancement"),
    (1300, "Tracker Certificates"),
    (1310, "Outperformance Certificates"),
    (1320, "Bonus Certificates"),
    (1330, "Outperformance Bonus Certificates"),
    (1340, "Twin-Win Certificates"),
    (1399, "Miscellaneous Participation"),
    (1440, "Credit Linked Note - Linear"),
    (1450, "Credit Linked Note - Equity Tranche"),
    (1460, "Credit Linked Note - Mezz./Senior Tranche"),
    (1499, "Miscellaneous Credit Linked Notes"),
    (2100, "Warrants"),
    (2110, "Spread Warrants"),
    (2199, "Miscellaneous"),
    (2200, "Knock-Out Warrants"),
    (2205, "Open-end Knock-Out Warrants"),
    (2210, "Mini-Futures"),
    (2230, "Double Knock-Out Warrants"),
    (2299, "Miscellaneous"),
    (2300, "Constant Leverage Certificate"),
    (2399, "Miscellaneous Constant Leverage Products"),
];

/// The members the SSPA Swiss Derivative Map lists, by code: its 2023 map
/// (v.23/2) and its 2026 map (v.26/1) list the same categories, each by
/// its English name.
const SSPA_NAMES: [(u16, &str); 24] = [
    (1100, "Capital Protection Note with Participation"),
    (1130, "Capital Protection Note with Barrier"),
    (1135, "Capital Protection Note with Twin Win"),
    (1140, "Capital Protection Note with Coupon"),
    (1200, "Discount Certificate"),
    (1210, "Barrier Discount Certificate"),
    (1220, "Reverse Convertible"),
    (1230, "Barrier Reverse Convertible"),
    (1255, "Conditional Coupon Reverse Convertible"),
    (1260, "Conditional Coupon Barrier Reverse Convertible"),
    (1300, "Tracker Certificate"),
    (1310, "Outperformance Certificate"),
    (1320, "Bonus Certificate"),
    (1330, "Bonus Outperformance Certificate"),
    (1340, "Twin Win Certificate"),
    (1400, "Credit Linked Notes"),
    (
        1410,
        "Conditional Capital Protection Note with add. credit risk",
    ),
    (1420, "Yield Enhancement Certificate with add. credit risk"),
    (1430, "Participation Certificate with add. credit risk"),
    (2100, "Warrant"),
    (2110, "Spread Warrant"),
    (2200, "Warrant with Knock-Out"),
    (2210, "Mini-Future"),
    (2300, "Constant Leverage Certificate"),
];

/// The name `table`, sorted by code, gives `code`.
fn named(table: &[(u16, &'static str)], code: u16) -> Option<&'static str> {
    table
        .binary_search_by_key(&code, |(held, _)| *held)
        .ok()
        .map(|at| table[at].1)
}

/// The product category of a structured product: one four-digit code of
/// EUSIPA's European Derivative Map, which the SSPA's Swiss Derivative Map
/// - the Swiss column of the European one - numbers the same way.
///
/// The first digit is the level - `1` an investment product, `2` a leverage
/// product - the first two the group (`12` yield enhancement, `23` constant
/// leverage) and the last two the member, `99` a group's miscellaneous one:
/// `2300` is a Constant Leverage Certificate in both maps. A code is held
/// by that shape alone, because the maps are snapshots of lists that move -
/// EUSIPA retired `1110` and added its credit linked notes, the SSPA added
/// `1135`, `1255` and its `14xx` - and a feed may state a member either
/// list has not caught up with. Which members each map lists, and by what
/// name, is [`Self::name`] - EUSIPA's map of February 2024 - and
/// [`Self::sspa_name`] - the SSPA's of 2023 (v.23/2) and 2026 (v.26/1),
/// whose category lists agree. The two maps share their numbering and name
/// one code apart: `1260` is an Express Certificate in the European map
/// and a Conditional Coupon Barrier Reverse Convertible in the Swiss one,
/// so the code, never a name, is the fact a row holds.
///
/// A value of its own rather than a datatype: a column of categories is
/// `int32` in a registry's `eusipacode` column - every table format stores that width - and serde writes the number.
///
/// ```
/// use yggdryl::Eusipa;
///
/// # fn main() -> yggdryl::Result<()> {
/// let constant: Eusipa = "2300".parse()?;
/// assert_eq!(constant.code(), 2300);
/// assert_eq!(constant.group(), 23);
/// assert_eq!(constant.level(), 2);
/// assert_eq!(constant.name(), Some("Constant Leverage Certificate"));
/// assert_eq!(constant.sspa_name(), Some("Constant Leverage Certificate"));
/// let express = Eusipa::new(1260)?;
/// assert_eq!(express.name(), Some("Express Certificates"));
/// assert_eq!(express.sspa_name(), Some("Conditional Coupon Barrier Reverse Convertible"));
/// assert!(!Eusipa::new(2301)?.is_listed(), "a code of the shape no map lists");
/// assert!(Eusipa::new(3100).is_err(), "no level");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Eusipa(u16);

impl<'de> Deserialize<'de> for Eusipa {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        u16::deserialize(deserializer)
            .and_then(|code| Self::new(code).map_err(serde::de::Error::custom))
    }
}

impl Eusipa {
    /// A category held by its shape: four digits opening with `1`, an
    /// investment product, or `2`, a leverage product - whatever either map
    /// lists.
    ///
    /// # Errors
    ///
    /// A number outside `1000` to `2999`.
    pub fn new(code: u16) -> Result<Self> {
        if !(1000..=2999).contains(&code) {
            return Err(refusal(format_smolstr!(
                "expected a four-digit EUSIPA product category opening with 1, an investment \
                 product, or 2, a leverage product, got {code}"
            )));
        }
        Ok(Self(code))
    }

    /// The category `text` spells: four ASCII digits once trimmed, of the
    /// shape [`Self::new`] holds - no sign, no leading zero, no fraction.
    ///
    /// # Errors
    ///
    /// Text that is not four digits, and a number [`Self::new`] refuses.
    pub fn from_text(text: &str) -> Result<Self> {
        let digits = text.trim();
        if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(refusal(format_smolstr!(
                "expected a four-digit EUSIPA product category, got {:?}",
                crate::text::elide_to(text, 64)
            )));
        }
        Self::new(
            digits
                .bytes()
                .fold(0_u16, |code, digit| code * 10 + u16::from(digit - b'0')),
        )
    }

    /// The four-digit code.
    #[must_use]
    pub const fn code(&self) -> u16 {
        self.0
    }

    /// The group: the first two digits - `11` capital protection, `12`
    /// yield enhancement, `13` participation, `14` credit linked notes,
    /// `21` and `22` leverage without and with a knock-out, `23` constant
    /// leverage in the European map.
    #[must_use]
    // A code is below 3000, so its group is below 30.
    #[allow(clippy::cast_possible_truncation)]
    pub const fn group(&self) -> u8 {
        (self.0 / 100) as u8
    }

    /// The level: the first digit, `1` an investment product and `2` a
    /// leverage product.
    #[must_use]
    // A code is below 3000, so its level is below 3.
    #[allow(clippy::cast_possible_truncation)]
    pub const fn level(&self) -> u8 {
        (self.0 / 1000) as u8
    }

    /// The English name EUSIPA's European Derivative Map of February 2024
    /// gives the code, `None` where that map lists no such member.
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        named(&EUSIPA_NAMES, self.0)
    }

    /// The English name the SSPA's Swiss Derivative Map - 2023 and 2026,
    /// one list - gives the code, `None` where that map lists no such
    /// member.
    #[must_use]
    pub fn sspa_name(&self) -> Option<&'static str> {
        named(&SSPA_NAMES, self.0)
    }

    /// Whether either map lists the code.
    #[must_use]
    pub fn is_listed(&self) -> bool {
        self.name().is_some() || self.sspa_name().is_some()
    }
}

impl fmt::Display for Eusipa {
    /// The four digits.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl FromStr for Eusipa {
    type Err = Error;

    /// [`Eusipa::from_text`].
    fn from_str(text: &str) -> Result<Self> {
        Self::from_text(text)
    }
}

impl TryFrom<u16> for Eusipa {
    type Error = Error;

    /// [`Eusipa::new`].
    fn try_from(code: u16) -> Result<Self> {
        Self::new(code)
    }
}

impl From<Eusipa> for u16 {
    fn from(category: Eusipa) -> Self {
        category.0
    }
}
