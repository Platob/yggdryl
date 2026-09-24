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

use crate::temporal::scalars::require;
use crate::temporal::{temporal_leaf, validate_duration_unit};
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, Error, Result, Scalar, TimeUnit, Timezone};

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
        self.id().as_str()
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
