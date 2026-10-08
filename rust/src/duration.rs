//! The duration family: one elapsed count at a fixed-length resolution, in
//! one of two widths.
//!
//! Arrow's `Duration` is 64 bits at every resolution; this crate also holds
//! a 32-bit width, so a column of short spans costs half the bytes and
//! crosses an Arrow boundary as the 64-bit storage. [`DurationType`] names
//! the two leaves, each carrying its unit as a parameter;
//! `DataType::Duration32` and `DataType::Duration64` are the duration datatypes; [`Duration32`]
//! and [`Duration64`] are the values, and [`crate::Scalar::from_duration`]
//! picks the narrowest width that holds a count.
//!
//! ```
//! use yggdryl::DurationType;
//! use yggdryl::{DataType, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every fixed-length unit is valid at either width, days included.
//! assert_eq!(DataType::duration32(TimeUnit::Day)?.to_string(), "duration32(d)");
//! assert_eq!(DataType::from_str("duration64(ns)")?, DataType::duration64(TimeUnit::Nanosecond)?);
//!
//! // An interval layout is no length at all.
//! assert!(DataType::duration64(TimeUnit::MonthDayNano).is_err());
//!
//! // The leaf is what a duration column declares.
//! let leaf = DataType::duration32(TimeUnit::Second)?.duration_type().expect("a duration datatype");
//! assert_eq!(leaf, DurationType::Duration32(TimeUnit::Second));
//! assert_eq!(leaf.bit_width(), 32);
//!
//! // A value takes the narrowest width its count fits.
//! let short = Scalar::from_duration(90, TimeUnit::Second, Timezone::NAIVE)?;
//! assert!(short.as_duration32().is_some());
//! let long = Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)?;
//! assert!(long.as_duration64().is_some());
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{arrow_storage, from_arrow_storage};
use smol_str::format_smolstr;

#[cfg(feature = "http")]
use crate::floating::f64_from_text;
#[cfg(feature = "http")]
use crate::integer::integer_from_text_as;
use crate::temporal::scalars::{narrow_i32, require};
use crate::temporal::{temporal_leaf, validate_duration_unit};
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, Error, Result, Scalar, TimeUnit, Timezone};

/// What every refusal of an elapsed-length spelling names.
#[cfg(feature = "http")]
pub(crate) const DURATION_SPELLINGS: &str =
    "seconds, with an optional fraction and an optional s, ms, us, ns or d unit";

/// Read an elapsed length out of the text a setting spells one with: a
/// non-negative number, with a fraction or an exponent, then an optional
/// unit, blanks allowed between.
///
/// A bare number is seconds. The unit is one [`TimeUnit`] reads that is a
/// fixed length - `s`, `ms`, `us`, `ns` and `d`, with their long spellings -
/// so `30s`, `250ms`, `1.5`, `1e3` and `1d` all read, and `1m` does not,
/// because a minute and a month share that letter and nothing here picks
/// between them. A whole count is exact at every magnitude, read through
/// [`integer_from_text_as`]; a fraction or an exponent reads through
/// [`f64_from_text`] and lands at nanosecond resolution. Negative, not
/// finite or past what a [`std::time::Duration`] holds is `None`, never a
/// panic. The one reader every timeout, pause and lease length in the crate
/// reads through - every one of them a setting of the HTTP client or of what
/// rides on it, which is why the reader exists under that feature alone.
#[cfg(feature = "http")]
pub(crate) fn duration_from_text(text: &str) -> Option<std::time::Duration> {
    use std::time::Duration;

    let text = text.trim();
    let split = text
        .bytes()
        .position(|byte| !matches!(byte, b'0'..=b'9' | b'.' | b'+' | b'-' | b'e' | b'E'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let unit = unit.trim_start();
    let unit = if unit.is_empty() {
        TimeUnit::Second
    } else {
        unit.parse::<TimeUnit>().ok()?
    };
    if let Some(count) = integer_from_text_as::<u64>(number) {
        return match unit {
            TimeUnit::Day => count.checked_mul(86_400).map(Duration::from_secs),
            TimeUnit::Second => Some(Duration::from_secs(count)),
            TimeUnit::Millisecond => Some(Duration::from_millis(count)),
            TimeUnit::Microsecond => Some(Duration::from_micros(count)),
            TimeUnit::Nanosecond => Some(Duration::from_nanos(count)),
            TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
        };
    }
    let seconds_per_unit = match unit {
        TimeUnit::Day => 86_400.0,
        TimeUnit::Second => 1.0,
        TimeUnit::Millisecond => 1e-3,
        TimeUnit::Microsecond => 1e-6,
        TimeUnit::Nanosecond => 1e-9,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => return None,
    };
    let seconds = f64_from_text(number)? * seconds_per_unit;
    (seconds >= 0.0)
        .then(|| Duration::try_from_secs_f64(seconds).ok())
        .flatten()
}

// ------------------------------------------------------------------------
// The duration payload: two widths over the fixed-length units.
// ------------------------------------------------------------------------

/// The duration family's datatype payload: one leaf per width, each
/// carrying the resolution its column counts in.
///
/// The unit is a parameter and never a leaf, and both widths carry every
/// fixed-length unit: a day is a length where a clock has none, so
/// `duration32(d)` is valid and `time32(d)` is not.
///
/// ```
/// use yggdryl::DurationType;
/// use yggdryl::{DataTypeId, TimeUnit};
///
/// assert_eq!(DurationType::Duration64(TimeUnit::Day).id(), DataTypeId::Duration64);
/// assert_eq!(
///     DurationType::from_id(DataTypeId::Duration32, TimeUnit::Second),
///     Some(DurationType::Duration32(TimeUnit::Second))
/// );
/// assert!(DurationType::Duration32(TimeUnit::YearMonth).validate().is_err());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DurationType {
    /// 32-bit elapsed-time count.
    Duration32(TimeUnit),
    /// 64-bit elapsed-time count - Arrow's `Duration`.
    Duration64(TimeUnit),
}

impl DurationType {
    /// Every leaf in identifier order, each at the resolution the grammar
    /// and the bindings default it to.
    pub const ALL: [Self; 2] = [
        Self::Duration32(TimeUnit::Millisecond),
        Self::Duration64(TimeUnit::Microsecond),
    ];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
        }
    }

    /// The leaf one identifier names at `unit`, or `None` for an identifier
    /// of another family. The unit is carried, not checked.
    #[must_use]
    pub const fn from_id(id: DataTypeId, unit: TimeUnit) -> Option<Self> {
        match id {
            DataTypeId::Duration32 => Some(Self::Duration32(unit)),
            DataTypeId::Duration64 => Some(Self::Duration64(unit)),
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
            Self::Duration32(unit) | Self::Duration64(unit) => unit,
        }
    }

    /// The physical count width in bits.
    #[must_use]
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::Duration32(_) => 32,
            Self::Duration64(_) => 64,
        }
    }

    /// The name this width states its own refusals under.
    const fn refusal_kind(self) -> &'static str {
        match self {
            Self::Duration32(_) => "Duration32",
            Self::Duration64(_) => "Duration64",
        }
    }

    /// Reject a unit that is not a fixed length.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] naming the width for an interval
    /// layout.
    pub fn validate(self) -> Result<()> {
        validate_duration_unit(self.refusal_kind(), self.unit())
    }
}

impl DataTypeValue for DurationType {
    const FAMILY: &'static str = "duration";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Duration32(unit) => DataType::Duration32(unit),
            Self::Duration64(unit) => DataType::Duration64(unit),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.duration_type()
    }
}

impl fmt::Display for DurationType {
    /// The canonical spelling, `duration64(ns)`, which
    /// [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.as_str(), self.unit())
    }
}

impl From<DurationType> for DataType {
    fn from(value: DurationType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for DurationType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value.duration_type() {
            Some(leaf) => Ok(leaf),
            None => Err(Error::InvalidDataType {
                kind: "duration",
                reason: format_smolstr!("expected a duration datatype, got {value}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The doors into the duration family.
// ------------------------------------------------------------------------

impl DataType {
    /// A 32-bit elapsed-time count at `unit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for an interval layout.
    pub fn duration32(unit: TimeUnit) -> Result<Self> {
        Self::duration_of(DurationType::Duration32(unit))
    }

    /// A 64-bit elapsed-time count at `unit`.
    ///
    /// Arrow durations are physically 64-bit at every resolution, so Arrow
    /// import always selects this width.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for an interval layout.
    pub fn duration64(unit: TimeUnit) -> Result<Self> {
        Self::duration_of(DurationType::Duration64(unit))
    }

    /// One checked duration leaf, whichever width names it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for an interval layout.
    pub fn duration_of(leaf: DurationType) -> Result<Self> {
        leaf.validate()?;
        Ok(DataTypeValue::into_dtype(leaf))
    }

    /// The leaf a duration datatype declares, `None` for every other.
    #[must_use]
    pub const fn duration_type(&self) -> Option<DurationType> {
        match self {
            Self::Duration32(unit) => Some(DurationType::Duration32(*unit)),
            Self::Duration64(unit) => Some(DurationType::Duration64(*unit)),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: one 64-bit storage for both widths.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use crate::invalid;
    use crate::{DataType, Result, TimeUnit};

    /// The Arrow storage one duration datatype lays out.
    ///
    /// Arrow has one duration width, so both leaves project to it; the
    /// 32-bit width widens on the way out and reads back as 64 bits.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is an interval layout, or when the
    /// datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        let Some(leaf) = dtype.duration_type() else {
            return Err(invalid(
                "duration",
                format_smolstr!("expected a duration datatype, got {dtype}"),
            ));
        };
        leaf.validate()?;
        Ok(ArrowDataType::Duration(leaf.unit().into_arrow_time()?))
    }

    /// The duration datatype one Arrow storage imports as: always the 64-bit
    /// width, which is what Arrow stores.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Duration(unit) => DataType::duration64(TimeUnit::from_arrow_time(*unit)),
            other => Err(invalid(
                "duration",
                format_smolstr!("expected a duration storage, got {other}"),
            )),
        }
    }
}

// ------------------------------------------------------------------------
// The duration values: an elapsed count at one resolution.
// ------------------------------------------------------------------------

temporal_leaf!(
    Duration32,
    i32,
    32,
    valid = |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
    dtype = |value: &Duration32| DataType::duration32(value.unit()),
    "Duration32 requires a fixed temporal unit and the NAIVE timezone",
);
temporal_leaf!(
    Duration64,
    i64,
    64,
    valid = |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
    dtype = |value: &Duration64| DataType::duration64(value.unit()),
    "Duration64 requires a fixed temporal unit and the NAIVE timezone",
);

impl Duration32 {
    /// The elapsed length `text` spells, at 32 bits.
    ///
    /// The ISO 8601 door of the 32-bit duration: what [`Duration64::from_text`]
    /// reads, narrowed to the width, so a count past `i32` is refused by
    /// name rather than wrapped.
    ///
    /// ```
    /// use yggdryl::{Duration32, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let span = Duration32::from_text("PT1.500S")?;
    /// assert_eq!((span.count(), span.unit()), (1_500, TimeUnit::Millisecond));
    /// assert_eq!(Duration32::from_text("-01:30:00")?.count(), -5_400);
    /// assert!(Duration32::from_text("PT3000000000S").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Duration64::from_text`]'s, and [`Error::InvalidRecord`] for a count
    /// that does not fit 32 bits.
    pub fn from_text(text: &str) -> Result<Self> {
        let (count, unit) = crate::temporal::parse_duration(text)?;
        Self::new(narrow_i32(count, "duration32")?, unit, Timezone::NAIVE)
    }
}

impl Duration64 {
    /// The elapsed length `text` spells.
    ///
    /// The ISO 8601 door of the duration family, over the one reader every
    /// duration cell in the crate reads through. Two grammars read the same
    /// count, the sign leading either as ISO 8601 puts it:
    ///
    /// | Spelling | Example | Reads as |
    /// | --- | --- | --- |
    /// | ISO 8601's general form, a fraction on the seconds alone | `PT90S`, `-P1DT2H3M4.5S` | every component restated in seconds |
    /// | a plain clock, the hours as wide as the count needs | `25:30:00`, `-00:00:01.500` | the elapsed hours as written, never folded |
    ///
    /// The resolution is the one the fraction's digits spell: seconds with
    /// none, milliseconds to three, microseconds to six, nanoseconds to
    /// nine. This is a cell's grammar and nothing else: a setting's `30s`
    /// or `1.5` is no cell and is refused here, as `PT30S` is refused where
    /// a setting is read. No FIX field is a duration, so the FIX codec has
    /// no door of its own into this family.
    ///
    /// ```
    /// use yggdryl::{Duration64, TimeUnit};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let span = Duration64::from_text("PT90S")?;
    /// assert_eq!((span.count(), span.unit()), (90, TimeUnit::Second));
    /// assert_eq!(Duration64::from_text("00:01:30")?, span);
    /// let fine = Duration64::from_text("-PT0.000000001S")?;
    /// assert_eq!((fine.count(), fine.unit()), (-1, TimeUnit::Nanosecond));
    /// assert!(Duration64::from_text("30s").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte the text stopped being a
    /// duration at.
    pub fn from_text(text: &str) -> Result<Self> {
        let (count, unit) = crate::temporal::parse_duration(text)?;
        Self::new(count, unit, Timezone::NAIVE)
    }
}

impl Scalar {
    /// Build the narrowest duration width that holds `count`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an interval layout or a zone
    /// other than [`Timezone::NAIVE`].
    pub fn from_duration(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match i32::try_from(count) {
            Ok(count) => Self::duration32_in(count, unit, zone),
            Err(_) => Self::duration64_in(count, unit, zone),
        }
    }

    /// Build a 32-bit duration.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an interval layout.
    pub fn duration32(count: i32, unit: TimeUnit) -> Result<Self> {
        Self::duration32_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 32-bit duration after validating its explicit timezone
    /// marker.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an interval layout or a zone
    /// other than [`Timezone::NAIVE`].
    pub fn duration32_in(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration32 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration32 timezone must be NAIVE")?;
        Duration32::new(count, unit, zone).map(Self::Duration32)
    }

    /// Build a 64-bit duration.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an interval layout.
    pub fn duration64(count: i64, unit: TimeUnit) -> Result<Self> {
        Self::duration64_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 64-bit duration after validating its explicit timezone
    /// marker.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an interval layout or a zone
    /// other than [`Timezone::NAIVE`].
    pub fn duration64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration64 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration64 timezone must be NAIVE")?;
        Duration64::new(count, unit, zone).map(Self::Duration64)
    }

    /// Return Duration32's count, unit, and zone.
    #[must_use]
    pub const fn as_duration32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Duration32(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }

    /// Return Duration64's count, unit, and zone.
    #[must_use]
    pub const fn as_duration64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Duration64(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/duration.rs` pins and a caller cannot reach: the
    //! one reader a setting's elapsed length goes through.
    /// What every refusal of an elapsed-length spelling names.
    #[cfg(feature = "http")]
    pub const DURATION_SPELLINGS: &str = super::DURATION_SPELLINGS;

    /// Read an elapsed length out of the text a setting spells one with.
    #[cfg(feature = "http")]
    pub fn duration_from_text(text: &str) -> Option<std::time::Duration> {
        super::duration_from_text(text)
    }
}
