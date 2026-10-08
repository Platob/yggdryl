//! The time family: one time of day, held at one resolution in one of two
//! widths.
//!
//! Arrow lays a clock reading out as `Time32` at seconds or milliseconds and
//! `Time64` at microseconds or nanoseconds, so the width follows from the
//! resolution and a caller who states only the resolution gets the width it
//! fits in. [`TimeType`] names the two leaves, each carrying its unit as a
//! parameter; `DataType::Time32` and `DataType::Time64` are the time datatypes; [`Time32`]
//! and [`Time64`] are the values.
//!
//! ```
//! use yggdryl::TimeType;
//! use yggdryl::{DataType, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // The resolution picks the width, and the spelling reads back as itself.
//! assert_eq!(DataType::time(TimeUnit::Second)?, DataType::time32(TimeUnit::Second)?);
//! assert_eq!(DataType::from_str("time")?, DataType::time64(TimeUnit::Microsecond)?);
//! assert_eq!(DataType::time32(TimeUnit::Millisecond)?.to_string(), "time32(ms)");
//!
//! // A width refuses a resolution it does not carry.
//! assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
//!
//! // The leaf is what a time column declares.
//! let leaf = DataType::time64(TimeUnit::Nanosecond)?.time_type().expect("a time datatype");
//! assert_eq!(leaf, TimeType::Time64(TimeUnit::Nanosecond));
//! assert_eq!(leaf.unit(), TimeUnit::Nanosecond);
//!
//! // A value follows the same rule.
//! let noon = Scalar::from_time(43_200, TimeUnit::Second, Timezone::NAIVE)?;
//! assert_eq!(noon.as_time32().map(|(count, ..)| count), Some(43_200));
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{arrow_storage, from_arrow_storage};
use smol_str::format_smolstr;

use crate::invalid;
use crate::parser::{Parser, precision_to_unit};
use crate::temporal::scalars::{narrow_i32, require};
use crate::temporal::{invalid_record, temporal_leaf, validate_time32_unit, validate_time64_unit};
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, Error, Result, Scalar, TimeUnit, Timezone};

// ------------------------------------------------------------------------
// The time payload: two widths, each over the resolutions it carries.
// ------------------------------------------------------------------------

/// The time family's datatype payload: one leaf per width, each carrying
/// the resolution its column counts in.
///
/// The unit is a parameter and never a leaf: `time32(s)` and
/// `time32(ms)` are one storage with one parameter, exactly as the
/// two scales of a `decimal128` are. Which width a unit belongs to is
/// [`Self::for_unit`]'s one rule.
///
/// ```
/// use yggdryl::TimeType;
/// use yggdryl::{DataTypeId, TimeUnit};
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(TimeType::for_unit(TimeUnit::Millisecond)?, TimeType::Time32(TimeUnit::Millisecond));
/// assert_eq!(TimeType::for_unit(TimeUnit::Nanosecond)?, TimeType::Time64(TimeUnit::Nanosecond));
/// assert!(TimeType::for_unit(TimeUnit::Day).is_err());
/// assert_eq!(TimeType::Time32(TimeUnit::Second).id(), DataTypeId::Time32);
/// assert_eq!(
///     TimeType::from_id(DataTypeId::Time64, TimeUnit::Microsecond),
///     Some(TimeType::Time64(TimeUnit::Microsecond))
/// );
/// // The leaf is public, so a width can carry a unit it does not hold; the
/// // check is what a constructor and a boundary both run.
/// assert!(TimeType::Time32(TimeUnit::Nanosecond).validate().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TimeType {
    /// 32-bit time of day; seconds and milliseconds are valid.
    Time32(TimeUnit),
    /// 64-bit time of day; microseconds and nanoseconds are valid.
    Time64(TimeUnit),
}

impl TimeType {
    /// Every leaf in identifier order, each at the resolution the grammar
    /// and the bindings default it to.
    pub const ALL: [Self; 2] = [
        Self::Time32(TimeUnit::Millisecond),
        Self::Time64(TimeUnit::Microsecond),
    ];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
        }
    }

    /// The leaf one identifier names at `unit`, or `None` for an identifier
    /// of another family. The unit is carried, not checked.
    #[must_use]
    pub const fn from_id(id: DataTypeId, unit: TimeUnit) -> Option<Self> {
        match id {
            DataTypeId::Time32 => Some(Self::Time32(unit)),
            DataTypeId::Time64 => Some(Self::Time64(unit)),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().core_str()
    }

    /// The resolution this column counts in, whichever width holds it.
    #[must_use]
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::Time32(unit) | Self::Time64(unit) => unit,
        }
    }

    /// The physical count width in bits.
    #[must_use]
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::Time32(_) => 32,
            Self::Time64(_) => 64,
        }
    }

    /// The width a resolution fits in: seconds and milliseconds in 32 bits,
    /// microseconds and nanoseconds in 64.
    ///
    /// This is the one rule behind [`DataType::time`]: a caller who states
    /// only the resolution gets the width Arrow holds it at.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a day or an interval layout,
    /// which are not resolutions of a clock.
    pub fn for_unit(unit: TimeUnit) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => Ok(Self::Time32(unit)),
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Ok(Self::Time64(unit)),
            TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
                Err(invalid("Time", "unit must be a temporal resolution"))
            }
        }
    }

    /// Reject a width carrying a resolution it does not hold.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] naming the width and the
    /// resolutions it does carry.
    pub fn validate(self) -> Result<()> {
        match self {
            Self::Time32(unit) => validate_time32_unit(unit),
            Self::Time64(unit) => validate_time64_unit(unit),
        }
    }
}

impl DataTypeValue for TimeType {
    const FAMILY: &'static str = "time";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Time32(unit) => DataType::Time32(unit),
            Self::Time64(unit) => DataType::Time64(unit),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.time_type()
    }
}

impl fmt::Display for TimeType {
    /// The canonical spelling, `time32(ms)`, which
    /// [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.as_str(), self.unit())
    }
}

impl From<TimeType> for DataType {
    fn from(value: TimeType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for TimeType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value.time_type() {
            Some(leaf) => Ok(leaf),
            None => Err(Error::InvalidDataType {
                kind: "time",
                reason: format_smolstr!("expected a time datatype, got {value}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The doors into the time family.
// ------------------------------------------------------------------------

impl DataType {
    /// The time-of-day datatype at the width `unit` fits in.
    ///
    /// Seconds and milliseconds land in [`TimeType::Time32`], microseconds
    /// and nanoseconds in [`TimeType::Time64`]; [`TimeType::for_unit`] is the
    /// rule.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a day or an interval layout.
    pub fn time(unit: TimeUnit) -> Result<Self> {
        Ok(TimeType::for_unit(unit)?.into())
    }

    /// A 32-bit time of day at `unit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit other than seconds or
    /// milliseconds.
    pub fn time32(unit: TimeUnit) -> Result<Self> {
        Self::time_of(TimeType::Time32(unit))
    }

    /// A 64-bit time of day at `unit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit other than microseconds
    /// or nanoseconds.
    pub fn time64(unit: TimeUnit) -> Result<Self> {
        Self::time_of(TimeType::Time64(unit))
    }

    /// One checked time leaf, whichever width names it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the width does not carry the
    /// unit.
    pub fn time_of(leaf: TimeType) -> Result<Self> {
        leaf.validate()?;
        Ok(DataTypeValue::into_dtype(leaf))
    }

    /// The leaf a time datatype declares, `None` for every other.
    #[must_use]
    pub const fn time_type(&self) -> Option<TimeType> {
        match self {
            Self::Time32(unit) => Some(TimeType::Time32(*unit)),
            Self::Time64(unit) => Some(TimeType::Time64(*unit)),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// The grammar of the resolution-only spelling.
// ------------------------------------------------------------------------

impl Parser<'_> {
    /// Parse SQL's `time`, `time(p)` or `time(unit)` into the width the
    /// resolution fits in; bare `time` is microseconds.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a precision past nine, a unit no
    /// name spells, or one that is not a clock resolution.
    pub(crate) fn parse_sql_time(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if self.peek_opening().is_none() {
            return DataType::time(TimeUnit::Microsecond);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected time unit"))?;
        let (unit, unit_start) = if let Some(precision) = self.peek_integer() {
            let start = self.current_position();
            self.index += 1;
            (precision_to_unit(precision, start)?, start)
        } else {
            self.parse_time_unit_span(Some(close), "time unit")?
        };
        self.expect_symbol(close)?;
        DataType::time(unit).map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two clock storages.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::TimeType;
    use crate::invalid;
    use crate::{DataType, Result, TimeUnit};

    /// The Arrow storage one time datatype lays out.
    ///
    /// A unit Arrow cannot state is refused here rather than silently
    /// widened: the leaf is public, so `time32(nanosecond)` can reach this
    /// boundary and this is where it stops.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not one the width carries, or when
    /// the datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        let Some(leaf) = dtype.time_type() else {
            return Err(invalid(
                "time",
                format_smolstr!("expected a time datatype, got {dtype}"),
            ));
        };
        leaf.validate()?;
        Ok(match leaf {
            TimeType::Time32(unit) => ArrowDataType::Time32(unit.into_arrow_time()?),
            TimeType::Time64(unit) => ArrowDataType::Time64(unit.into_arrow_time()?),
        })
    }

    /// The time datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not one the width carries, or when
    /// the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Time32(unit) => DataType::time32(TimeUnit::from_arrow_time(*unit)),
            ArrowDataType::Time64(unit) => DataType::time64(TimeUnit::from_arrow_time(*unit)),
            other => Err(invalid(
                "time",
                format_smolstr!("expected a time storage, got {other}"),
            )),
        }
    }
}

// ------------------------------------------------------------------------
// The time values: a count since midnight at one resolution.
// ------------------------------------------------------------------------

temporal_leaf!(
    Time32,
    i32,
    32,
    valid = |unit: TimeUnit, timezone: Timezone| matches!(
        unit,
        TimeUnit::Second | TimeUnit::Millisecond
    ) && timezone.is_naive(),
    dtype = |value: &Time32| DataType::time32(value.unit()),
    "Time32 requires second or millisecond units and the NAIVE timezone",
);
temporal_leaf!(
    Time64,
    i64,
    64,
    valid = |unit: TimeUnit, timezone: Timezone| matches!(
        unit,
        TimeUnit::Microsecond | TimeUnit::Nanosecond
    ) && timezone.is_naive(),
    dtype = |value: &Time64| DataType::time64(value.unit()),
    "Time64 requires microsecond or nanosecond units and the NAIVE timezone",
);

/// Refuse a clock that states a zone, naming the type that reads one.
///
/// An offset makes a clock an instant, and the message says so rather than
/// reporting the offset as trailing text: `09:30:00Z` is no time of day
/// and a `DateTime64` is where it reads.
fn require_zoneless(text: &str) -> Result<()> {
    let zoned = text.ends_with(['Z', 'z'])
        || text
            .len()
            .checked_sub(6)
            .is_some_and(|start| matches!(text.as_bytes()[start], b'+' | b'-'));
    require(
        !zoned,
        "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
    )
}

impl Time32 {
    /// The time of day `text` spells, at the width's resolution.
    ///
    /// The ISO 8601 door of the 32-bit clock, over the one reader every
    /// time of day in the crate reads through:
    ///
    /// | Spelling | Example | Reads as |
    /// | --- | --- | --- |
    /// | `HH:MM:SS` | `09:30:00` | the clock at seconds |
    /// | compact `HHMMSS` | `093000` | the same clock |
    /// | a fraction after `.` or `,`, one to nine digits | `09:30:00.5`, `09:30:00,500` | the clock at milliseconds |
    /// | an hour past the day, to `99` | `25:00:00` | folded into the day: `01:00:00` |
    ///
    /// The resolution is the one the digits spell, held at this width's
    /// floor and ceiling: no fraction is seconds, up to three digits are
    /// milliseconds, and a finer fraction is narrowed to milliseconds where
    /// it is exact - `.000000` is a fraction of zero, which the width holds -
    /// and refused where it is not, because a `Time32` holds no finer count.
    /// [`Time64::from_text`] is the width that does.
    ///
    /// ```
    /// use yggdryl::{Time32, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let clock = Time32::from_text("09:30:00")?;
    /// assert_eq!((clock.count(), clock.unit()), (34_200, TimeUnit::Second));
    /// assert_eq!(Time32::from_text("093000")?, clock);
    /// let half = Time32::from_text("09:30:00.5")?;
    /// assert_eq!((half.count(), half.unit()), (34_200_500, TimeUnit::Millisecond));
    /// assert_eq!(Time32::from_text("09:30:00.000000")?.unit(), TimeUnit::Millisecond);
    /// // A finer fraction is no 32-bit clock; a zone is no time of day.
    /// assert!(Time32::from_text("09:30:00.000001").is_err());
    /// assert!(Time32::from_text("09:30:00Z").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte the text stopped being a
    /// clock at, and [`Error::InvalidRecord`] for a stated zone or a fraction
    /// the width cannot hold.
    pub fn from_text(text: &str) -> Result<Self> {
        require_zoneless(text)?;
        Self::from_reading(crate::temporal::parse_time(text)?)
    }

    /// The time of day `text` spells as FIX spells one: everything
    /// [`Self::from_text`] reads, and the clock half of the two FIX datetime
    /// spellings that have one - a clock that stops at its minutes (`09:30`,
    /// `0930`), and the compact clock running straight into three, six or
    /// nine fraction digits with no decimal sign (`093000123`). A stated zone
    /// is refused as [`Self::from_text`] refuses it, and so are `60` seconds,
    /// a leap second no Arrow clock holds, and a run of ten to twelve
    /// fraction digits. The FIX codec reads every `UTCTimeOnly` and
    /// `LocalMktTime` field through this door and parses none itself.
    ///
    /// # Errors
    ///
    /// [`Self::from_text`]'s.
    pub(crate) fn from_fix_text(text: &str) -> Result<Self> {
        require_zoneless(text)?;
        Self::from_reading(crate::temporal::parse_fix_time(text)?)
    }

    /// The 32-bit value one reading is: seconds and milliseconds as spelled,
    /// a finer fraction narrowed to milliseconds where it is exact.
    fn from_reading((count, unit): (i64, TimeUnit)) -> Result<Self> {
        let (count, unit) = match unit {
            TimeUnit::Second | TimeUnit::Millisecond => (count, unit),
            finer => {
                let per_millisecond = crate::temporal::per_second(finer)
                    .expect("a clock reads at a resolution unit")
                    / 1_000;
                require(
                    count % per_millisecond == 0,
                    "time32 holds no fraction finer than a millisecond; use Time64 for one",
                )?;
                (count / per_millisecond, TimeUnit::Millisecond)
            }
        };
        Self::new(narrow_i32(count, "time32")?, unit, Timezone::NAIVE)
    }
}

impl Time64 {
    /// The time of day `text` spells, at the width's resolution.
    ///
    /// The ISO 8601 door of the 64-bit clock, reading what
    /// [`Time32::from_text`] reads - `HH:MM:SS`, the compact `HHMMSS`, a
    /// fraction of one to nine digits after `.` or `,`, an hour past the day
    /// folded into it - at the resolution the digits spell, widened to this
    /// width's floor: no fraction and up to three digits are microseconds,
    /// four to six microseconds, seven to nine nanoseconds. A column
    /// restates the count at its own unit, which is exact at every width
    /// here because the floor is the coarsest unit the width holds.
    ///
    /// ```
    /// use yggdryl::{Time64, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let clock = Time64::from_text("09:30:00")?;
    /// assert_eq!((clock.count(), clock.unit()), (34_200_000_000, TimeUnit::Microsecond));
    /// assert_eq!(Time64::from_text("093000")?, clock);
    /// let nanos = Time64::from_text("09:30:00.000000001")?;
    /// assert_eq!((nanos.count(), nanos.unit()), (34_200_000_000_001, TimeUnit::Nanosecond));
    /// assert_eq!(Time64::from_text("25:00:00")?.count(), 3_600_000_000);
    /// // A zone is no time of day: a `DateTime64` reads one.
    /// assert!(Time64::from_text("09:30:00Z").is_err());
    /// assert!(Time64::from_text("09:30").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte the text stopped being a
    /// clock at, and [`Error::InvalidRecord`] for a stated zone.
    pub fn from_text(text: &str) -> Result<Self> {
        require_zoneless(text)?;
        Self::from_reading(crate::temporal::parse_time(text)?)
    }

    /// The time of day `text` spells as FIX spells one: everything
    /// [`Self::from_text`] reads, and the clock half of the two FIX datetime
    /// spellings that have one - a clock that stops at its minutes (`09:30`,
    /// `0930`), and the compact clock running straight into three, six or
    /// nine fraction digits with no decimal sign (`093000123`, the clock of
    /// `20240102101530123`). A stated zone is refused as [`Self::from_text`]
    /// refuses it, and so are `60` seconds - a leap second no Arrow clock
    /// holds - and a run of ten to twelve fraction digits. The FIX codec
    /// reads every `UTCTimeOnly` and `LocalMktTime` field through this door
    /// and parses none itself.
    ///
    /// # Errors
    ///
    /// [`Self::from_text`]'s.
    pub(crate) fn from_fix_text(text: &str) -> Result<Self> {
        require_zoneless(text)?;
        Self::from_reading(crate::temporal::parse_fix_time(text)?)
    }

    /// The 64-bit value one reading is: microseconds and nanoseconds as
    /// spelled, a coarser reading widened to microseconds, the width's floor.
    fn from_reading((count, unit): (i64, TimeUnit)) -> Result<Self> {
        let (count, unit) = match unit {
            TimeUnit::Second => (count * 1_000_000, TimeUnit::Microsecond),
            TimeUnit::Millisecond => (count * 1_000, TimeUnit::Microsecond),
            finer => (count, finer),
        };
        Self::new(count, unit, Timezone::NAIVE)
    }
}

impl Scalar {
    /// Build the exact time-of-day width selected by its unit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit no clock counts in, for a
    /// 32-bit count that does not fit, and for a zone other than
    /// [`Timezone::NAIVE`].
    pub fn from_time(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => {
                Self::time32(narrow_i32(count, "time32")?, unit, zone)
            }
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(count, unit, zone),
            _ => Err(invalid_record(
                "time unit must be second, millisecond, microsecond, or nanosecond",
            )),
        }
    }

    /// Build a 32-bit time of day.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than seconds or
    /// milliseconds, or a zone other than [`Timezone::NAIVE`].
    pub fn time32(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Second | TimeUnit::Millisecond),
            "time32 unit must be second or millisecond",
        )?;
        require(
            zone.is_naive(),
            "time32 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time32::new(count, unit, zone).map(Self::Time32)
    }

    /// Build a 64-bit time of day.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit other than microseconds
    /// or nanoseconds, or a zone other than [`Timezone::NAIVE`].
    pub fn time64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond),
            "time64 unit must be microsecond or nanosecond",
        )?;
        require(
            zone.is_naive(),
            "time64 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Time64::new(count, unit, zone).map(Self::Time64)
    }

    /// Return Time32's count, unit, and zone.
    #[must_use]
    pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Time32(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }

    /// Return Time64's count, unit, and zone.
    #[must_use]
    pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Time64(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/time.rs` pins and a caller cannot reach: the
    //! FIX doors of the two clocks, which the FIX codec alone reads through.

    use crate::{Result, Time32, Time64};

    /// Read a time of day as FIX spells one, at 32 bits.
    pub fn time32_from_fix_text(text: &str) -> Result<Time32> {
        Time32::from_fix_text(text)
    }

    /// Read a time of day as FIX spells one, at 64 bits.
    pub fn time64_from_fix_text(text: &str) -> Result<Time64> {
        Time64::from_fix_text(text)
    }
}
