use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{Error, Result};

/// The physical resolution or interval layout of an Arrow temporal value.
///
/// Arrow exposes scalar temporal resolutions and calendar interval layouts as
/// two enums. Yggdryl keeps them in one value vocabulary and uses
/// [`Self::is_temporal`] and [`Self::is_interval`] to distinguish the two
/// lossless Arrow projection categories.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    /// Whole seconds.
    Second,
    /// Thousandths of a second.
    Millisecond,
    /// Millionths of a second.
    Microsecond,
    /// Billionths of a second.
    Nanosecond,
    /// A signed number of calendar months.
    YearMonth,
    /// A pair of signed days and milliseconds.
    DayTime,
    /// Signed months, days, and nanoseconds.
    MonthDayNano,
}

impl TimeUnit {
    /// Every unit in canonical order: the temporal resolutions from coarsest
    /// to finest, then the interval layouts.
    pub const ALL: [Self; 7] = [
        Self::Second,
        Self::Millisecond,
        Self::Microsecond,
        Self::Nanosecond,
        Self::YearMonth,
        Self::DayTime,
        Self::MonthDayNano,
    ];

    /// Parse a temporal resolution or interval layout.
    ///
    /// Parsing is ASCII case-insensitive and accepts common Arrow, SQL, Hive,
    /// and Spark spellings. Leading and trailing whitespace is ignored. ASCII
    /// whitespace, `_`, and `-` may separate words; canonical short units such
    /// as `ms` and interval names such as `year_month` always round-trip through
    /// [`fmt::Display`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Import an Arrow scalar temporal resolution.
    pub const fn from_arrow_time(value: arrow_schema::TimeUnit) -> Self {
        match value {
            arrow_schema::TimeUnit::Second => Self::Second,
            arrow_schema::TimeUnit::Millisecond => Self::Millisecond,
            arrow_schema::TimeUnit::Microsecond => Self::Microsecond,
            arrow_schema::TimeUnit::Nanosecond => Self::Nanosecond,
        }
    }

    /// Import an Arrow calendar interval layout.
    pub const fn from_arrow_interval(value: arrow_schema::IntervalUnit) -> Self {
        match value {
            arrow_schema::IntervalUnit::YearMonth => Self::YearMonth,
            arrow_schema::IntervalUnit::DayTime => Self::DayTime,
            arrow_schema::IntervalUnit::MonthDayNano => Self::MonthDayNano,
        }
    }

    /// Consume this value and project an Arrow scalar temporal resolution.
    ///
    /// Calendar interval layouts return an error instead of being coerced.
    pub fn into_arrow_time(self) -> Result<arrow_schema::TimeUnit> {
        self.try_into()
    }

    /// Consume this value and project an Arrow calendar interval layout.
    ///
    /// Scalar temporal resolutions return an error instead of being coerced.
    pub fn into_arrow_interval(self) -> Result<arrow_schema::IntervalUnit> {
        self.try_into()
    }

    /// Return whether this value is an Arrow scalar temporal resolution.
    pub const fn is_temporal(self) -> bool {
        matches!(
            self,
            Self::Second | Self::Millisecond | Self::Microsecond | Self::Nanosecond
        )
    }

    /// Return whether this value is an Arrow calendar interval layout.
    pub const fn is_interval(self) -> bool {
        matches!(self, Self::YearMonth | Self::DayTime | Self::MonthDayNano)
    }

    /// Return the canonical spelling without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Second => "s",
            Self::Millisecond => "ms",
            Self::Microsecond => "us",
            Self::Nanosecond => "ns",
            Self::YearMonth => "year_month",
            Self::DayTime => "day_time",
            Self::MonthDayNano => "month_day_nano",
        }
    }
}

impl FromStr for TimeUnit {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let original = value;
        let value = original.trim();
        let start = original.len() - original.trim_start().len();
        let parsed = if matches_alias(value, &["s", "sec", "secs", "second", "seconds"]) {
            Some(Self::Second)
        } else if matches_alias(
            value,
            &["ms", "milli", "millis", "millisecond", "milliseconds"],
        ) {
            Some(Self::Millisecond)
        } else if matches!(value, "\u{b5}s" | "\u{b5}S" | "\u{3bc}s" | "\u{3bc}S")
            || matches_alias(
                value,
                &["us", "micro", "micros", "microsecond", "microseconds"],
            )
        {
            Some(Self::Microsecond)
        } else if matches_alias(value, &["ns", "nano", "nanos", "nanosecond", "nanoseconds"]) {
            Some(Self::Nanosecond)
        } else if matches_alias(
            value,
            &["year", "years", "yearmonth", "yeartomonth", "yearstomonths"],
        ) {
            Some(Self::YearMonth)
        } else if matches_alias(
            value,
            &[
                "daytime",
                "day",
                "days",
                "daytotime",
                "daystotime",
                "daytosecond",
                "daystoseconds",
            ],
        ) {
            Some(Self::DayTime)
        } else if matches_alias(
            value,
            &[
                "monthdaynano",
                "monthdaynanos",
                "monthdaynanosecond",
                "monthdaynanoseconds",
            ],
        ) {
            Some(Self::MonthDayNano)
        } else {
            None
        };

        parsed.ok_or_else(|| Error::Parse {
            target: "time unit",
            position: start
                + value
                    .char_indices()
                    .find_map(|(position, character)| {
                        (!(character.is_ascii_alphanumeric()
                            || character.is_ascii_whitespace()
                            || matches!(character, '_' | '-' | '\u{b5}' | '\u{3bc}')))
                        .then_some(position)
                    })
                    .unwrap_or(0),
            reason: "unknown temporal resolution or interval layout".into(),
        })
    }
}

fn matches_alias(value: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| {
        value
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'_' | b'-'))
            .map(|byte| byte.to_ascii_lowercase())
            .eq(alias.bytes())
    })
}

impl fmt::Display for TimeUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for TimeUnit {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for TimeUnit {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TimeUnitVisitor;

        impl<'de> serde::de::Visitor<'de> for TimeUnitVisitor {
            type Value = TimeUnit;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a temporal resolution or interval layout string")
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                TimeUnit::from_str(value).map_err(E::custom)
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                TimeUnit::from_str(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(TimeUnitVisitor)
    }
}

impl From<arrow_schema::TimeUnit> for TimeUnit {
    fn from(value: arrow_schema::TimeUnit) -> Self {
        Self::from_arrow_time(value)
    }
}

impl From<arrow_schema::IntervalUnit> for TimeUnit {
    fn from(value: arrow_schema::IntervalUnit) -> Self {
        Self::from_arrow_interval(value)
    }
}

impl TryFrom<TimeUnit> for arrow_schema::TimeUnit {
    type Error = Error;

    fn try_from(value: TimeUnit) -> Result<Self> {
        match value {
            TimeUnit::Second => Ok(Self::Second),
            TimeUnit::Millisecond => Ok(Self::Millisecond),
            TimeUnit::Microsecond => Ok(Self::Microsecond),
            TimeUnit::Nanosecond => Ok(Self::Nanosecond),
            TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
                Err(Error::InvalidDataType {
                    kind: "TimeUnit",
                    reason: "an interval layout cannot project to arrow_schema::TimeUnit".into(),
                })
            }
        }
    }
}

impl TryFrom<TimeUnit> for arrow_schema::IntervalUnit {
    type Error = Error;

    fn try_from(value: TimeUnit) -> Result<Self> {
        match value {
            TimeUnit::YearMonth => Ok(Self::YearMonth),
            TimeUnit::DayTime => Ok(Self::DayTime),
            TimeUnit::MonthDayNano => Ok(Self::MonthDayNano),
            TimeUnit::Second
            | TimeUnit::Millisecond
            | TimeUnit::Microsecond
            | TimeUnit::Nanosecond => Err(Error::InvalidDataType {
                kind: "TimeUnit",
                reason: "a temporal resolution cannot project to arrow_schema::IntervalUnit".into(),
            }),
        }
    }
}
