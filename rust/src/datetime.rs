//! The datetime family: one instant or wall-clock reading, at one resolution,
//! with an explicit zone marker.
//!
//! Arrow's `Timestamp` is a 64-bit count at one of four resolutions with an
//! optional zone name. This family carries the same, with the zone made
//! explicit: [`crate::Timezone::NAIVE`] is a wall clock that means no
//! instant, and any other zone makes the count a UTC instant read in that
//! zone. [`DateTimeType`] has one leaf, `DateTime64`, carrying the unit and
//! the zone as its parameters; [`crate::DataType::DateTime64`] is the one
//! datetime datatype; [`DateTime64`] is the value.
//!
//! ```
//! use yggdryl::DateTimeType;
//! use yggdryl::{DataType, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // A naive column and a zoned one are one datatype with one parameter apart.
//! let naive = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?;
//! let utc = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(naive.to_string(), "datetime64(us)");
//! assert_eq!(utc.to_string(), "datetime64(us,\"UTC\")");
//! assert_eq!(DataType::from_str("timestamp")?, naive);
//!
//! // The leaf is what a datetime column declares, and it reads both back.
//! let leaf = utc.datetime_type().expect("a datetime datatype");
//! assert_eq!(leaf, DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::UTC });
//! assert_eq!(leaf.unit(), TimeUnit::Microsecond);
//! assert!(leaf.timezone().is_utc());
//!
//! // A day is no resolution of a clock.
//! assert!(DataType::datetime64(TimeUnit::Day, Timezone::NAIVE).is_err());
//!
//! let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(at.temporal_timezone(), Some(Timezone::UTC));
//! # Ok(())
//! # }
//! ```

use std::fmt;

pub(crate) use arrow::{
    arrow_storage, from_arrow_storage, from_arrow_storage_owned, into_arrow_storage,
};
use smol_str::format_smolstr;

use crate::invalid;
use crate::parser::{Parser, fmt_quoted, precision_to_unit};
use crate::temporal::scalars::require;
use crate::temporal::{TemporalKind, temporal_leaf};
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, TimeUnit, Timezone};

// ------------------------------------------------------------------------
// The datetime payload: one width, a resolution and a zone.
// ------------------------------------------------------------------------

/// The datetime family's datatype payload: one leaf, carrying the
/// resolution its column counts in and the zone its counts are read in.
///
/// One leaf is the shape every family has, and it is what `id`, `ALL` and
/// `from_id` are written over; a second width would be a second leaf here
/// and no call site would learn a new variant.
///
/// ```
/// use yggdryl::DateTimeType;
/// use yggdryl::{DataTypeId, TimeUnit, Timezone};
///
/// # fn main() -> yggdryl::Result<()> {
/// let leaf = DateTimeType::DateTime64 { unit: TimeUnit::Second, timezone: Timezone::NAIVE };
/// assert_eq!(leaf.id(), DataTypeId::DateTime64);
/// assert_eq!(leaf.with_unit(TimeUnit::Nanosecond)?.unit(), TimeUnit::Nanosecond);
/// assert!(leaf.with_unit(TimeUnit::Day).is_err());
/// assert!(leaf.with_timezone(Timezone::UTC).timezone().is_utc());
/// assert_eq!(DateTimeType::ALL[0].unit(), TimeUnit::Microsecond);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DateTimeType {
    /// A 64-bit count at `unit` since the Unix epoch, read in `timezone`.
    DateTime64 {
        /// The count's temporal resolution.
        unit: TimeUnit,
        /// An IANA zone, a fixed offset, or [`Timezone::NAIVE`] for a wall
        /// clock that names no instant.
        timezone: Timezone,
    },
}

impl DateTimeType {
    /// Every leaf in identifier order, at the resolution and zone the
    /// grammar and the bindings default to.
    pub const ALL: [Self; 1] = [Self::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: Timezone::NAIVE,
    }];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::DateTime64 { .. } => DataTypeId::DateTime64,
        }
    }

    /// The leaf one identifier names at `unit` in `timezone`, or `None` for
    /// an identifier of another family. The unit is carried, not checked.
    #[must_use]
    pub const fn from_id(id: DataTypeId, unit: TimeUnit, timezone: Timezone) -> Option<Self> {
        match id {
            DataTypeId::DateTime64 => Some(Self::DateTime64 { unit, timezone }),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The resolution this column counts in.
    #[must_use]
    pub const fn unit(self) -> TimeUnit {
        match self {
            Self::DateTime64 { unit, .. } => unit,
        }
    }

    /// The zone this column's counts are read in.
    #[must_use]
    pub const fn timezone(self) -> Timezone {
        match self {
            Self::DateTime64 { timezone, .. } => timezone,
        }
    }

    /// The physical count width in bits.
    #[must_use]
    pub const fn bit_width(self) -> u8 {
        match self {
            Self::DateTime64 { .. } => 64,
        }
    }

    /// The family's name, `datetime`, as a datatype spells it.
    #[must_use]
    pub const fn family(self) -> &'static str {
        TemporalKind::DateTime.as_str()
    }

    /// This leaf at another resolution.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit that is not a clock
    /// resolution.
    pub fn with_unit(self, unit: TimeUnit) -> Result<Self> {
        let leaf = match self {
            Self::DateTime64 { timezone, .. } => Self::DateTime64 { unit, timezone },
        };
        leaf.validate()?;
        Ok(leaf)
    }

    /// This leaf read in another zone; every zone is a valid parameter.
    #[must_use]
    pub fn with_timezone(self, timezone: Timezone) -> Self {
        match self {
            Self::DateTime64 { unit, .. } => Self::DateTime64 { unit, timezone },
        }
    }

    /// Reject a resolution Arrow's timestamp does not count in.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a day or an interval layout,
    /// under the leaf's name: the one rule the constructor and
    /// [`DataType::validate`] share.
    pub fn validate(self) -> Result<()> {
        if self.unit().is_arrow_time() {
            Ok(())
        } else {
            Err(invalid("datetime64", "unit must be a temporal resolution"))
        }
    }
}

impl DataTypeValue for DateTimeType {
    const FAMILY: &'static str = "datetime";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Temporal
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::DateTime64 { unit, timezone } => DataType::DateTime64 { unit, timezone },
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.datetime_type()
    }
}

impl fmt::Display for DateTimeType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads
    /// back: `datetime64(us)` for a wall clock, the zone quoted
    /// beside the unit otherwise.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::DateTime64 { unit, timezone } = self;
        write!(formatter, "{}({unit}", self.as_str())?;
        if !timezone.is_naive() {
            formatter.write_str(",")?;
            fmt_quoted(formatter, timezone.as_str())?;
        }
        formatter.write_str(")")
    }
}

impl From<DateTimeType> for DataType {
    fn from(value: DateTimeType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for DateTimeType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value.datetime_type() {
            Some(leaf) => Ok(leaf),
            None => Err(Error::InvalidDataType {
                kind: "datetime",
                reason: format_smolstr!("expected a datetime datatype, got {value}"),
            }),
        }
    }
}

// ------------------------------------------------------------------------
// The door into the datetime family.
// ------------------------------------------------------------------------

impl DataType {
    /// A 64-bit datetime at `unit`, read in `timezone`.
    ///
    /// Use [`Timezone::NAIVE`] for a wall-clock column that names no
    /// instant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a unit that is not a clock
    /// resolution.
    pub fn datetime64(unit: TimeUnit, timezone: Timezone) -> Result<Self> {
        let leaf = DateTimeType::DateTime64 { unit, timezone };
        leaf.validate()?;
        Ok(DataTypeValue::into_dtype(leaf))
    }

    /// The leaf a datetime datatype declares, `None` for every other.
    #[must_use]
    pub const fn datetime_type(&self) -> Option<DateTimeType> {
        match self {
            Self::DateTime64 { unit, timezone } => Some(DateTimeType::DateTime64 {
                unit: *unit,
                timezone: *timezone,
            }),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// The grammar of every timestamp spelling.
// ------------------------------------------------------------------------

impl Parser<'_> {
    /// Parse the parameters after `datetime64`, `timestamp` and SQL's
    /// zoned and unzoned timestamp keywords.
    ///
    /// The list is positional: a precision or a unit first, then a zone
    /// written bare, as `Some(zone)` or as `None`; SQL's trailing `with time
    /// zone` and `without time zone` decide the zone after the list. The
    /// keyword decides the default zone, so `timestampltz` is UTC before
    /// any parameter is read.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a precision past nine, a unit
    /// that is not a clock resolution, a zone no name spells, or a list that
    /// does not close.
    pub(crate) fn parse_datetime64(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        let mut unit = TimeUnit::Microsecond;
        let mut timezone = if keyword == "timestampltz" || keyword == "timestampwithtimezone" {
            Timezone::UTC
        } else {
            Timezone::NAIVE
        };

        if let Some(close) = self.consume_opening() {
            if self.peek_symbol() != Some(close) {
                if let Some(precision) = self.peek_integer() {
                    let precision_start = self.current_position();
                    self.index += 1;
                    unit = precision_to_unit(precision, precision_start)?;
                } else if self.peek_word_is("none") {
                    self.index += 1;
                    timezone = Timezone::NAIVE;
                } else if self.peek_word_is("some") {
                    self.index += 1;
                    let inner_close = self
                        .consume_opening()
                        .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                    timezone = self.parse_timezone()?;
                    self.expect_symbol(inner_close)?;
                } else {
                    let (parsed, unit_start) =
                        self.parse_time_unit_span(Some(close), "datetime64 unit")?;
                    unit = parsed;
                    if !unit.is_arrow_time() {
                        return Err(self.error_at(
                            unit_start,
                            "datetime64 requires a temporal resolution unit",
                        ));
                    }
                }

                if self.consume_separator() {
                    self.consume_label("timezone");
                    if self.peek_word_is("none") {
                        self.index += 1;
                        timezone = Timezone::NAIVE;
                    } else if self.peek_word_is("some") {
                        self.index += 1;
                        let inner_close = self
                            .consume_opening()
                            .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                        timezone = self.parse_timezone()?;
                        self.expect_symbol(inner_close)?;
                    } else {
                        timezone = self.parse_timezone()?;
                    }
                }
            }
            self.expect_symbol(close)?;
        }

        if self.consume_word("with") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::UTC;
        } else if self.consume_word("without") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::NAIVE;
        }

        DataType::datetime64(unit, timezone)
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the timestamp storage, with its zone name.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_schema::DataType as ArrowDataType;
    use smol_str::{SmolStr, format_smolstr};

    use crate::invalid;
    use crate::{DataType, Result, TimeUnit, Timezone};

    /// The Arrow storage one datetime datatype lays out.
    ///
    /// A naive column is a timestamp with no zone; any other zone rides as
    /// its canonical name.
    ///
    /// # Errors
    ///
    /// Returns an error when the unit is not a clock resolution, or when
    /// the datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        match dtype {
            DataType::DateTime64 { unit, timezone } => Ok(ArrowDataType::Timestamp(
                unit.into_arrow_time()?,
                (!timezone.is_naive()).then(|| Arc::<str>::from(timezone.as_smol_str().clone())),
            )),
            other => Err(invalid(
                "datetime",
                format_smolstr!("expected a datetime datatype, got {other}"),
            )),
        }
    }

    /// The same projection, consuming the zone name rather than cloning it.
    ///
    /// # Errors
    ///
    /// [`arrow_storage`] carries the rule.
    pub(crate) fn into_arrow_storage(dtype: DataType) -> Result<ArrowDataType> {
        match dtype {
            DataType::DateTime64 { unit, timezone } => Ok(ArrowDataType::Timestamp(
                unit.into_arrow_time()?,
                (!timezone.is_naive()).then(|| Arc::<str>::from(timezone.into_smol_str())),
            )),
            other => arrow_storage(&other),
        }
    }

    /// The datetime datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the zone name is not one this crate
    /// canonicalizes, or when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Timestamp(unit, timezone) => {
                let timezone = timezone.as_ref().map_or(Ok(Timezone::NAIVE), |value| {
                    Timezone::from_smol_str(SmolStr::from(Arc::clone(value)))
                })?;
                DataType::datetime64(TimeUnit::from_arrow_time(*unit), timezone)
            }
            other => Err(invalid(
                "datetime",
                format_smolstr!("expected a datetime storage, got {other}"),
            )),
        }
    }

    /// The same import, consuming Arrow's shared zone name rather than
    /// cloning it.
    ///
    /// # Errors
    ///
    /// [`from_arrow_storage`] carries the rule.
    pub(crate) fn from_arrow_storage_owned(value: ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Timestamp(unit, timezone) => {
                let timezone = timezone.map_or(Ok(Timezone::NAIVE), |value| {
                    Timezone::from_smol_str(SmolStr::from(value))
                })?;
                DataType::datetime64(TimeUnit::from_arrow_time(unit), timezone)
            }
            other => from_arrow_storage(&other),
        }
    }
}

// ------------------------------------------------------------------------
// The datetime value: a 64-bit count, its resolution and its zone.
// ------------------------------------------------------------------------

temporal_leaf!(
    DateTime64,
    i64,
    DateTime,
    64,
    valid = |unit: TimeUnit, _timezone: Timezone| unit.is_arrow_time(),
    dtype = |value: &DateTime64| DataType::datetime64(value.unit(), value.timezone()),
    "DateTime64 requires an Arrow clock resolution",
);

// The zone is one word beside the count, so a value is two words and never
// a pointer.
const _: () = assert!(std::mem::size_of::<DateTime64>() == 16);

impl Scalar {
    /// Build a 64-bit epoch or wall-clock datetime.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit that is not a clock
    /// resolution.
    pub fn from_datetime(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        Self::datetime64(count, unit, zone)
    }

    /// Build an instant or wall-clock datetime at 64-bit width.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a unit that is not a clock
    /// resolution.
    pub fn datetime64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_arrow_time(),
            "datetime64 requires an Arrow time unit",
        )?;
        DateTime64::new(count, unit, zone).map(Self::DateTime64)
    }

    /// Parse a timezone and build a 64-bit datetime.
    ///
    /// # Errors
    ///
    /// Returns the error the zone name raised, or
    /// [`Error::InvalidRecord`] for a unit that is not a clock resolution.
    pub fn datetime64_in(count: i64, unit: TimeUnit, zone: &str) -> Result<Self> {
        Self::datetime64(count, unit, Timezone::from_str(zone)?)
    }

    /// Return DateTime64's count, unit, and zone.
    #[must_use]
    pub const fn as_datetime64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::DateTime64(value) => Some((value.count(), value.unit(), &value.timezone)),
            _ => None,
        }
    }
}
