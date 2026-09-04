//! One way to name a time zone, everywhere in the project.
//!
//! A zone used to be an unvalidated `Option<SmolStr>` sitting in the second
//! slot of a timestamp datatype. That meant `Asia/Calcutta` and `Asia/Kolkata`
//! were different zones, `utc` and `UTC` were different zones, nothing could
//! say what offset any of them implied, and nothing rejected a typo. This type
//! is the single answer to all four.
//!
//! [`Self::NAIVE`] gives every native temporal value a non-optional zone while
//! `DataType::Timestamp(unit, None)` retains Arrow's schema spelling. A zone is
//! a process-lifetime interned handle; [`Self::offset_at`] applies the registry
//! rules bundled by this build.
//!
//! ```
//! use yggdryl::Timezone;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Aliases and case both canonicalize, so two spellings compare equal.
//! assert_eq!(Timezone::from_str("Asia/Calcutta")?, Timezone::from_str("Asia/Kolkata")?);
//! assert_eq!(Timezone::from_str("Z")?, Timezone::UTC);
//!
//! // A registered zone knows its own rules.
//! let new_york = Timezone::from_str("America/New_York")?;
//! assert_eq!(new_york.offset_at(1_700_000_000), Some(-5 * 3600));  // November: EST
//! assert_eq!(new_york.offset_at(1_688_000_000), Some(-4 * 3600));  // June: EDT
//! assert_eq!(new_york.abbreviation_at(1_688_000_000), Some("EDT"));
//!
//! // A fixed offset needs no registry at all.
//! assert_eq!(Timezone::from_str("+05:30")?.offset_at(0), Some(5 * 3600 + 1800));
//! # Ok(())
//! # }
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::{Error, Result, stable_hash_display};

mod registry;

use registry::{Basis, Edge, Zone};

/// Seconds in one day, the modulus every civil-time calculation turns on.
const DAY: i64 = 86_400;

/// A validated, canonical time zone name.
///
/// The name is canonical on arrival: an alias resolves to what it stands for,
/// a fixed offset normalizes to `+HH:MM`, and a registered name normalizes its
/// case. The four-byte handle is process-local and never serialized; names are
/// retained for the process lifetime, so copying a zone never allocates.
#[repr(transparent)]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Timezone(NonZeroU32);

const _: () = assert!(std::mem::size_of::<Timezone>() == 4);

impl Timezone {
    /// A wall-clock value with no time-zone interpretation.
    pub const NAIVE: Self = Self(registry::NAIVE_HANDLE);

    /// Coordinated Universal Time, the zero point every other zone offsets
    /// from and the one zone that is always registered.
    pub const UTC: Self = Self(registry::UTC_HANDLE);

    /// Parse and canonicalize a time zone name.
    ///
    /// Four spellings are accepted, in this order: a registered IANA name, an
    /// alias for one, a fixed offset (`+05:30`, `-0800`, `Z`), and any other
    /// syntactically plausible IANA name, which is kept as written so a zone
    /// this build has no rules for still round-trips through a schema.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name, a name holding a control character,
    /// or an offset whose hour or minute is out of range.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Build a zone from a fixed offset east of UTC, in seconds.
    ///
    /// # Errors
    ///
    /// Returns an error when the offset is beyond ±24 hours or is not a whole
    /// number of minutes, neither of which any real zone uses.
    pub fn from_offset(seconds: i32) -> Result<Self> {
        if seconds.abs() >= 24 * 3_600 {
            return Err(parse_error(
                0,
                "a fixed time zone offset must be within 24 hours of UTC",
            ));
        }
        if seconds % 60 != 0 {
            return Err(parse_error(
                0,
                "a fixed time zone offset must be a whole number of minutes",
            ));
        }
        if seconds == 0 {
            return Ok(Self::UTC);
        }
        Ok(Self(registry::fixed_handle(seconds)))
    }

    /// Return the canonical name without allocating.
    pub fn as_str(&self) -> &str {
        registry::name(self.0)
    }

    /// Borrow the canonical name as a shared string.
    ///
    /// The interner retains this value for the process lifetime. Arrow
    /// projection can therefore clone a long name's shared allocation rather
    /// than copy its bytes.
    pub fn as_smol_str(&self) -> &SmolStr {
        registry::smol_str(self.0)
    }

    /// Consume this zone and return the shared name.
    pub fn into_smol_str(self) -> SmolStr {
        self.as_smol_str().clone()
    }

    /// Return whether this zone is UTC itself.
    pub fn is_utc(&self) -> bool {
        *self == Self::UTC
    }

    /// Return whether this is the explicit zone-free marker.
    pub fn is_naive(&self) -> bool {
        *self == Self::NAIVE
    }

    /// Return whether this build knows the rules for this zone.
    ///
    /// A zone that is not known still parses, compares, and round-trips; it
    /// simply answers `None` to every offset question rather than guessing.
    pub fn is_known(&self) -> bool {
        self.entry().is_some() || self.fixed_offset().is_some()
    }

    /// Return whether the name is a fixed offset rather than a place.
    pub fn is_fixed(&self) -> bool {
        self.fixed_offset().is_some()
    }

    /// Return whether this zone ever observes daylight saving.
    ///
    /// An unknown zone answers `false`, because nothing is known to observe.
    pub fn observes_saving(&self) -> bool {
        self.entry().is_some_and(|zone| zone.saving.is_some())
    }

    /// Return the offset east of UTC, in seconds, at an instant.
    ///
    /// `epoch` is seconds since the Unix epoch, UTC. The answer accounts for
    /// daylight saving under the rules in force today; a zone this build has
    /// no rules for answers `None`.
    ///
    /// ```
    /// use yggdryl::Timezone;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let sydney = Timezone::from_str("Australia/Sydney")?;
    ///
    /// // Sydney's saving period spans the new year, so January is +11.
    /// assert_eq!(sydney.offset_at(1_704_067_200), Some(11 * 3600));
    /// // ... and July is +10.
    /// assert_eq!(sydney.offset_at(1_720_000_000), Some(10 * 3600));
    /// # Ok(())
    /// # }
    /// ```
    pub fn offset_at(&self, epoch: i64) -> Option<i32> {
        if let Some(offset) = self.fixed_offset() {
            return Some(offset);
        }
        let zone = self.entry()?;
        Some(zone.standard + self.saving_at(zone, epoch))
    }

    /// Return the standard offset east of UTC, ignoring daylight saving.
    pub fn standard_offset(&self) -> Option<i32> {
        self.fixed_offset()
            .or_else(|| self.entry().map(|zone| zone.standard))
    }

    /// Return whether daylight saving is in force at an instant.
    pub fn is_saving_at(&self, epoch: i64) -> Option<bool> {
        if self.is_fixed() {
            return Some(false);
        }
        let zone = self.entry()?;
        Some(self.saving_at(zone, epoch) != 0)
    }

    /// Return the abbreviation in use at an instant, such as `EST` or `CEST`.
    pub fn abbreviation_at(&self, epoch: i64) -> Option<&'static str> {
        let zone = self.entry()?;
        if self.saving_at(zone, epoch) == 0 {
            return Some(zone.standard_abbreviation);
        }
        zone.daylight_abbreviation
            .or(Some(zone.standard_abbreviation))
    }

    /// Convert a UTC instant to the local reading in this zone.
    ///
    /// Both values are seconds since the Unix epoch; the local reading is the
    /// wall clock expressed as if it were UTC, which is the convention every
    /// naive timestamp in the project already uses.
    ///
    /// # Errors
    ///
    /// Returns an error when this build has no rules for the zone, because a
    /// silently unconverted instant is worse than a refusal.
    pub fn into_local(self, epoch: i64) -> Result<i64> {
        let offset = self.offset_at(epoch).ok_or_else(|| self.unknown_error())?;
        Ok(epoch + i64::from(offset))
    }

    /// Convert a local reading in this zone to the UTC instant it names.
    ///
    /// A local reading is ambiguous for one hour each year when clocks go back
    /// and non-existent for one hour when they go forward. This resolves both
    /// the way every mainstream library does: the *earlier* interpretation of
    /// an ambiguous reading, and the post-transition offset for a reading that
    /// never happened.
    ///
    /// # Errors
    ///
    /// Returns an error when this build has no rules for the zone.
    pub fn into_utc(self, local: i64) -> Result<i64> {
        // The offset depends on the instant, and the instant is what is being
        // solved for, so guess with the standard offset and correct once.
        let standard = self.standard_offset().ok_or_else(|| self.unknown_error())?;
        let guess = local - i64::from(standard);
        let offset = self.offset_at(guess).ok_or_else(|| self.unknown_error())?;
        let corrected = local - i64::from(offset);
        // Re-reading the offset at the corrected instant catches the case where
        // the first guess landed on the far side of a transition.
        let settled = self
            .offset_at(corrected)
            .ok_or_else(|| self.unknown_error())?;
        Ok(local - i64::from(settled))
    }

    /// Return every zone this build knows the rules for, sorted by name.
    ///
    /// This is the "installed" set: it needs no files, no environment, and no
    /// network, so it answers the same way on every machine the project runs
    /// on, which is the property a schema needs.
    pub fn registered() -> impl ExactSizeIterator<Item = Self> {
        registry::ZONES
            .iter()
            .enumerate()
            .map(|(index, _)| Self(registry::registered_handle(index)))
    }

    /// Return every alias and the canonical name it resolves to.
    pub fn aliases() -> impl ExactSizeIterator<Item = (&'static str, &'static str)> {
        registry::ALIASES.iter().copied()
    }

    /// Return a deterministic cross-language hash of the canonical name.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    /// Look this zone up in the registry.
    fn entry(&self) -> Option<&'static Zone> {
        registry::zone_for_handle(self.0)
    }

    /// Read a fixed offset out of the name, when the name is one.
    fn fixed_offset(&self) -> Option<i32> {
        registry::fixed_offset(self.0)
    }

    /// Return the saving in force at an instant, in seconds.
    fn saving_at(&self, zone: &'static Zone, epoch: i64) -> i32 {
        let Some(rule) = zone.saving else {
            return 0;
        };
        let year = year_of(epoch);
        let start = transition(rule.start, year, zone.standard, rule.save);
        let end = transition(rule.end, year, zone.standard, rule.save);

        let inside = if start <= end {
            // Northern hemisphere: the period sits inside one calendar year.
            epoch >= start && epoch < end
        } else {
            // Southern hemisphere: the period wraps the new year, so a moment
            // is inside it when it is after the spring start or before the
            // autumn end of the same year.
            epoch >= start || epoch < end
        };
        if inside { rule.save } else { 0 }
    }

    /// The error an offset question raises for a zone with no known rules.
    fn unknown_error(&self) -> Error {
        Error::Parse {
            target: "timezone",
            position: 0,
            reason: SmolStr::new(format!(
                "this build has no rules for the time zone {}",
                self.as_str()
            )),
        }
    }
}

/// Build the parse error shape this module reports.
fn parse_error(position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target: "timezone",
        position,
        reason: SmolStr::new_static(reason),
    }
}

/// Read a fixed-offset spelling, returning `None` when it is not one.
///
/// Accepts `+HH:MM`, `+HHMM`, `+HH`, and the same with `-`, plus the bare
/// `UTC±HH:MM` form some systems emit.
///
/// # Errors
///
/// Returns an error when the text looks like an offset but its hour or minute
/// is out of range, which is a typo worth reporting rather than a name.
fn parse_offset(value: &str) -> Result<Option<i32>> {
    let text = value
        .strip_prefix("UTC")
        .or_else(|| value.strip_prefix("utc"))
        .unwrap_or(value);
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'+') => (1, &text[1..]),
        Some(b'-') => (-1, &text[1..]),
        _ => return Ok(None),
    };

    let (hours, minutes) = match rest.split_once(':') {
        Some((hours, minutes)) => (hours, minutes),
        None => match rest.len() {
            4 => rest.split_at(2),
            2 | 1 => (rest, "0"),
            _ => {
                return Err(parse_error(
                    1,
                    "a time zone offset must be HH, HHMM, or HH:MM",
                ));
            }
        },
    };

    let hours: i32 = hours
        .parse()
        .map_err(|_| parse_error(1, "a time zone offset hour must be a number"))?;
    let minutes: i32 = minutes
        .parse()
        .map_err(|_| parse_error(1, "a time zone offset minute must be a number"))?;
    if !(0..24).contains(&hours) {
        return Err(parse_error(1, "a time zone offset hour must be under 24"));
    }
    if !(0..60).contains(&minutes) {
        return Err(parse_error(1, "a time zone offset minute must be under 60"));
    }
    Ok(Some(sign * (hours * 3_600 + minutes * 60)))
}

/// Return the number of days from the Unix epoch to a civil date.
///
/// This is Howard Hinnant's `days_from_civil`, which is exact for every year
/// in range and needs no table.
pub(crate) const fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year } as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Return the civil date a day number falls on, as `(year, month, day)`.
///
/// The exact inverse of [`days_from_civil`], from the same paper.
pub(crate) const fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (
        (if month <= 2 { year + 1 } else { year }) as i32,
        month as u32,
        day as u32,
    )
}

/// Return the civil year a day number falls in.
const fn year_from_days(days: i64) -> i32 {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    (if month >= 10 { year + 1 } else { year }) as i32
}

/// Return the civil year an instant falls in, in UTC.
const fn year_of(epoch: i64) -> i32 {
    year_from_days(epoch.div_euclid(DAY))
}

/// Return the weekday of a day number, 0 for Sunday.
const fn weekday_from_days(days: i64) -> u32 {
    // 1970-01-01 was a Thursday, which is weekday 4.
    (days + 4).rem_euclid(7) as u32
}

/// Return the day number of the nth given weekday in a month.
///
/// `week` counts from 1; 5 means the last such weekday, which is what every
/// zone that says "last Sunday" needs.
fn nth_weekday(year: i32, month: u8, week: u8, weekday: u8) -> i64 {
    let first = days_from_civil(year, u32::from(month), 1);
    let first_weekday = weekday_from_days(first);
    let shift = (u32::from(weekday) + 7 - first_weekday) % 7;
    if week >= 5 {
        // Step forward in weeks while the day is still inside this month.
        let mut day = first + i64::from(shift);
        let next_month = if month == 12 {
            days_from_civil(year + 1, 1, 1)
        } else {
            days_from_civil(year, u32::from(month) + 1, 1)
        };
        while day + 7 < next_month {
            day += 7;
        }
        return day;
    }
    first + i64::from(shift) + i64::from(week - 1) * 7
}

/// Return the UTC instant one edge of a saving rule happens at.
fn transition(edge: Edge, year: i32, standard: i32, save: i32) -> i64 {
    let day = nth_weekday(year, edge.month, edge.week, edge.weekday);
    let local = day * DAY + i64::from(edge.seconds);
    match edge.basis {
        // The rule is written in UTC, so the reading is already the instant.
        Basis::Utc => local,
        // The rule is written in local standard time.
        Basis::Standard => local - i64::from(standard),
        // The rule is written in wall-clock time, which at the end of a saving
        // period still includes the saving being removed.
        Basis::Wall => local - i64::from(standard) - i64::from(save),
    }
}

impl FromStr for Timezone {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value.is_empty() {
            return Err(parse_error(0, "a time zone name must not be empty"));
        }
        if let Some(position) = value.chars().position(char::is_control) {
            return Err(parse_error(
                position,
                "a time zone name must not hold a control character",
            ));
        }

        if value.eq_ignore_ascii_case("naive") {
            return Ok(Self::NAIVE);
        }

        // A fixed offset is canonical as `+HH:MM`, and `+00:00` is UTC itself.
        if let Some(offset) = parse_offset(value)? {
            return Self::from_offset(offset);
        }

        // A registered name wins without consulting the dynamic interner.
        if let Some(handle) = registry::registered(value) {
            return Ok(Self(handle));
        }
        if let Some(canonical) = registry::alias(value) {
            return Ok(Self(registry::intern(canonical)?));
        }
        if let Some(handle) = registry::registered_ignoring_case(value) {
            return Ok(Self(handle));
        }

        // An unregistered name is kept as written: this build has no rules for
        // it, but a schema that names it must still round-trip unchanged.
        Ok(Self(registry::intern(value)?))
    }
}

impl Timezone {
    /// Build a zone from a shared name.
    ///
    /// This is the Arrow import path. Interning canonicalizes it through the
    /// same path as every other spelling; the process registry then owns the
    /// one retained copy.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name or one holding a control character.
    pub fn from_smol_str(value: SmolStr) -> Result<Self> {
        Self::from_str(value.as_str())
    }
}

impl fmt::Debug for Timezone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Timezone")
            .field(&self.as_str())
            .finish()
    }
}

impl PartialOrd for Timezone {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timezone {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl AsRef<str> for Timezone {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Timezone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Timezone {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Timezone {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = SmolStr::deserialize(deserializer)?;
        Self::from_smol_str(value).map_err(D::Error::custom)
    }
}

impl From<Timezone> for SmolStr {
    fn from(value: Timezone) -> Self {
        value.into_smol_str()
    }
}

#[cfg(test)]
mod tests;
