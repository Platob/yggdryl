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
use crate::temporal::temporal_leaf;
use crate::value::DataTypeValue;
use crate::{DataType, DataTypeId, Error, Result, Scalar, TimeUnit, Timezone};

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
    /// Parse what follows a timestamp keyword: `datetime64`, `timestamp`,
    /// SQL's and Spark's zoned and unzoned keywords, and Iceberg's
    /// `timestamptz`, `timestamp_ns` and `timestamptz_ns`.
    ///
    /// The keyword decides the defaults, and some decide more than a
    /// default. `timestamp_ns` and `timestamptz_ns` state their unit, so no
    /// parameter list follows them. `timestamp_ltz`,
    /// `timestamp_with_time_zone` and `timestamptz` state a zone - UTC unless
    /// a parameter names another - and `timestamp_ntz` states none. Every
    /// later statement - a zone parameter, a trailing `with time zone` or
    /// `without time zone` - is held to the ones before it, and one saying
    /// the opposite is refused rather than read over them. `with time zone`
    /// after a named zone agrees with it and keeps it.
    ///
    /// The list is positional: a precision or a unit first, then a zone
    /// written bare, as `Some(zone)` or as `None`. `[]` after the keyword is
    /// the serie of it, never an empty list.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a precision past nine, a unit
    /// that is not a clock resolution, a zone no name spells, a zone
    /// statement an earlier one contradicts, a parameter list after a
    /// keyword that states its unit, or a list that does not close.
    pub(crate) fn parse_datetime64(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if let "timestampns" | "timestamptzns" = keyword {
            if self.peek_opening().is_some() && !self.peek_postfix_serie() {
                return Err(self.error_here(
                    "expected no parameter after timestamp_ns or timestamptz_ns, which state \
                     their unit; a precision belongs on timestamp(p) or timestamptz(p)",
                ));
            }
            let timezone = match keyword == "timestamptzns" {
                true => Timezone::UTC,
                false => Timezone::NAIVE,
            };
            return DataType::datetime64(TimeUnit::Nanosecond, timezone);
        }
        // Whether a zone was stated so far, and where: the keyword first.
        let keyword_start = self
            .index
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
            .map_or(0, |token| token.start);
        let mut stated = match keyword {
            "timestampltz" | "timestampwithtimezone" | "timestamptz" => Some((true, keyword_start)),
            "timestampntz" => Some((false, keyword_start)),
            _ => None,
        };
        let mut unit = TimeUnit::Microsecond;
        let mut timezone = match stated {
            Some((true, _)) => Timezone::UTC,
            _ => Timezone::NAIVE,
        };

        if !self.peek_postfix_serie()
            && let Some(close) = self.consume_opening()
        {
            if self.peek_symbol() != Some(close) {
                if let Some(precision) = self.peek_integer() {
                    let precision_start = self.current_position();
                    self.index += 1;
                    unit = precision_to_unit(precision, precision_start)?;
                } else if self.peek_word_is("none") || self.peek_word_is("some") {
                    timezone = self.parse_datetime64_zone(&mut stated)?;
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
                    timezone = self.parse_datetime64_zone(&mut stated)?;
                }
            }
            self.expect_symbol(close)?;
        }

        let suffix = self.current_position();
        if self.consume_word("with") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            self.check_datetime64_zone(stated, true, suffix)?;
            // The suffix says there is a zone; a parameter naming one said which.
            if timezone.is_naive() {
                timezone = Timezone::UTC;
            }
        } else if self.consume_word("without") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            self.check_datetime64_zone(stated, false, suffix)?;
            timezone = Timezone::NAIVE;
        }

        DataType::datetime64(unit, timezone)
    }

    /// Read one zone parameter - bare, `Some(zone)` or `None` - hold it to
    /// what was stated before it, and record it as the latest statement.
    fn parse_datetime64_zone(&mut self, stated: &mut Option<(bool, usize)>) -> Result<Timezone> {
        let position = self.current_position();
        let timezone = if self.consume_word("none") {
            Timezone::NAIVE
        } else if self.consume_word("some") {
            let inner_close = self
                .consume_opening()
                .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
            let timezone = self.parse_timezone()?;
            self.expect_symbol(inner_close)?;
            timezone
        } else {
            self.parse_timezone()?
        };
        self.check_datetime64_zone(*stated, !timezone.is_naive(), position)?;
        *stated = Some((!timezone.is_naive(), position));
        Ok(timezone)
    }

    /// Refuse a zone statement that says the opposite of an earlier one.
    fn check_datetime64_zone(
        &self,
        stated: Option<(bool, usize)>,
        zoned: bool,
        position: usize,
    ) -> Result<()> {
        match stated {
            Some((true, at)) if !zoned => Err(self.error_at(
                position,
                format_smolstr!("expected a zone, as stated at byte {at}, got none"),
            )),
            Some((false, at)) if zoned => Err(self.error_at(
                position,
                format_smolstr!("expected no zone, as stated at byte {at}, got one"),
            )),
            _ => Ok(()),
        }
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
    64,
    valid = |unit: TimeUnit, _timezone: Timezone| unit.is_arrow_time(),
    dtype = |value: &DateTime64| DataType::datetime64(value.unit(), value.timezone()),
    "DateTime64 requires an Arrow clock resolution",
);

// The zone is one word beside the count, so a value is two words and never
// a pointer.
const _: () = assert!(std::mem::size_of::<DateTime64>() == 16);

impl DateTime64 {
    /// The datetime `text` spells, in any spelling the crate reads one in.
    ///
    /// This is the crate's one reader of datetime text whose zone the text
    /// may or may not state - an expiry a tool wrote, a capture, a parameter,
    /// a cell, a document - over the ISO 8601 readers every text codec
    /// shares, so no module keeps a reading of its own:
    ///
    /// | Spelling | Example | Reads as |
    /// | --- | --- | --- |
    /// | `Z`, or an offset with or without its colon | `2026-10-03T05:20:00+02:00`, `...+0200` | that instant, in the offset's zone |
    /// | an offset and the zone's bracketed name | `2026-10-03T05:20:00+02:00[Europe/Paris]` | that instant, in the named zone |
    /// | a trailing `UTC`, with or without a blank | `2026-10-03T03:20:00UTC` (the AWS CLI's caches) | that instant, in UTC |
    /// | no zone, `T` or a blank before the clock | `2026-10-03 03:20:00.250` | a wall clock in `naive` |
    /// | a bare date | `2026-10-03` | that day's midnight, a wall clock in `naive` |
    ///
    /// The resolution is the one the digits spell - seconds for `03:20:00`,
    /// milliseconds for `03:20:00.250` - and surrounding blanks are ignored.
    /// A reading that states no zone is a wall clock in `naive`: in
    /// [`Timezone::UTC`] it is that instant, in [`Timezone::NAIVE`] it stays
    /// a wall clock meaning no instant.
    ///
    /// ```
    /// use yggdryl::{DateTime64, TimeUnit, Timezone};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let utc = DateTime64::from_text("2026-10-03T03:20:00Z", Timezone::NAIVE)?;
    /// assert_eq!((utc.count(), utc.unit()), (1_790_997_600, TimeUnit::Second));
    /// for spelled in ["2026-10-03T05:20:00+0200", "2026-10-03T03:20:00 UTC", "2026-10-03 03:20:00"] {
    ///     assert_eq!(DateTime64::from_text(spelled, Timezone::UTC)?.count(), 1_790_997_600);
    /// }
    /// let wall = DateTime64::from_text("2026-10-03 03:20:00.250", Timezone::NAIVE)?;
    /// assert!(wall.timezone().is_naive());
    /// assert_eq!(wall.unit(), TimeUnit::Millisecond);
    /// assert!(DateTime64::from_text("soon", Timezone::UTC).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// The instant reading's refusal, naming the byte it stopped at, when
    /// `text` spells neither an instant nor a wall clock; an error when a
    /// wall clock does not exist in `naive`'s rules.
    pub fn from_text(text: &str, naive: Timezone) -> Result<Self> {
        let text = text.trim();
        let closed;
        let text = match text.strip_suffix("UTC") {
            Some(head) => {
                closed = format!("{}Z", head.trim_end());
                closed.as_str()
            }
            None => text,
        };
        match crate::temporal::parse_timestamp(text) {
            Ok((count, unit, zone)) => Self::new(count, unit, zone),
            Err(zoned) => {
                let (local, unit) = crate::temporal::parse_datetime(text).map_err(|_| zoned)?;
                let count = if naive.is_naive() {
                    local
                } else {
                    wall_clock_in(local, unit, naive)?
                };
                Self::new(count, unit, naive)
            }
        }
    }
}

/// The UTC count of `unit` the wall clock `local` reads in `zone`.
fn wall_clock_in(local: i64, unit: TimeUnit, zone: Timezone) -> Result<i64> {
    let out_of_range = || Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$.timezone"),
        reason: smol_str::SmolStr::new_static("zoned timestamp is out of range"),
    };
    let per = crate::temporal::per_second(unit).ok_or_else(out_of_range)?;
    let seconds = local.div_euclid(per);
    let fraction = local.rem_euclid(per);
    zone.into_utc(seconds)?
        .checked_mul(per)
        .and_then(|seconds| seconds.checked_add(fraction))
        .ok_or_else(out_of_range)
}

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
