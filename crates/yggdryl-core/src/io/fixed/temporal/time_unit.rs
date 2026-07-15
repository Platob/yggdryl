//! [`TimeUnit`] — the resolution of a temporal value, from nanoseconds up to years, with the
//! epoch-conversion helpers and a string parser.
//!
//! The **fixed** units (`Nanosecond` … `Week`) are an exact number of nanoseconds, so a count in
//! one converts to another by a single multiply/divide. The **calendar** units (`Month`, `Year`)
//! have no fixed length — a month is 28–31 days — so they carry no nanosecond size and only take
//! part in calendar-aware arithmetic (see the date/timestamp types), never a plain unit convert.

/// The resolution of a temporal count. `Ord` follows ascending magnitude
/// (`Nanosecond < … < Year`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TimeUnit {
    /// One billionth of a second.
    Nanosecond,
    /// One millionth of a second.
    Microsecond,
    /// One thousandth of a second.
    Millisecond,
    /// One second.
    Second,
    /// Sixty seconds.
    Minute,
    /// Sixty minutes.
    Hour,
    /// Twenty-four hours (`86_400` seconds).
    Day,
    /// Seven days.
    Week,
    /// A **calendar** month (variable length — no fixed nanosecond size).
    Month,
    /// A **calendar** year (variable length — no fixed nanosecond size).
    Year,
}

impl TimeUnit {
    /// Every unit, ascending, for iteration / exhaustive checks.
    pub const ALL: [TimeUnit; 10] = [
        Self::Nanosecond,
        Self::Microsecond,
        Self::Millisecond,
        Self::Second,
        Self::Minute,
        Self::Hour,
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Year,
    ];

    /// The number of **nanoseconds** in one of this unit, or `None` for the calendar units
    /// (`Month` / `Year`) which have no fixed length.
    pub const fn nanos(self) -> Option<i128> {
        const S: i128 = 1_000_000_000; // ns per second
        Some(match self {
            Self::Nanosecond => 1,
            Self::Microsecond => 1_000,
            Self::Millisecond => 1_000_000,
            Self::Second => S,
            Self::Minute => 60 * S,
            Self::Hour => 3_600 * S,
            Self::Day => 86_400 * S,
            Self::Week => 7 * 86_400 * S,
            Self::Month | Self::Year => return None,
        })
    }

    /// Whether this is a **fixed** unit (an exact number of nanoseconds); `false` for the calendar
    /// units `Month` / `Year`.
    pub const fn is_fixed(self) -> bool {
        self.nanos().is_some()
    }

    /// Whether this is a **calendar** unit (`Month` / `Year`).
    pub const fn is_calendar(self) -> bool {
        !self.is_fixed()
    }

    /// A count of `value` in this unit, expressed in **nanoseconds** — `None` for a calendar unit
    /// or on overflow.
    pub fn to_nanos(self, value: i128) -> Option<i128> {
        value.checked_mul(self.nanos()?)
    }

    /// The number of whole `self` units in `nanos` nanoseconds (truncating) — `None` for a
    /// calendar unit.
    pub fn from_nanos(self, nanos: i128) -> Option<i128> {
        nanos.checked_div(self.nanos()?)
    }

    /// Converts a count `value` from unit `from` to unit `to` (truncating on a finer→coarser step),
    /// or `None` if either side is a calendar unit or the intermediate nanoseconds overflow.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::temporal::TimeUnit;
    /// assert_eq!(TimeUnit::convert(1500, TimeUnit::Millisecond, TimeUnit::Second), Some(1));
    /// assert_eq!(TimeUnit::convert(2, TimeUnit::Second, TimeUnit::Millisecond), Some(2000));
    /// ```
    pub fn convert(value: i128, from: TimeUnit, to: TimeUnit) -> Option<i128> {
        if from == to {
            return Some(value);
        }
        let (from_nanos, to_nanos) = (from.nanos()?, to.nanos()?);
        if from_nanos >= to_nanos {
            // Coarser → finer (or equal): exact multiply.
            value.checked_mul(from_nanos / to_nanos)
        } else {
            // Finer → coarser: truncating divide.
            Some(value / (to_nanos / from_nanos))
        }
    }

    /// The stable, lower-case singular name (`"nanosecond"` … `"year"`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nanosecond => "nanosecond",
            Self::Microsecond => "microsecond",
            Self::Millisecond => "millisecond",
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    /// The short abbreviation (`"ns"`, `"us"`, `"ms"`, `"s"`, `"min"`, `"h"`, `"d"`, `"w"`, `"mo"`,
    /// `"y"`).
    pub const fn abbreviation(self) -> &'static str {
        match self {
            Self::Nanosecond => "ns",
            Self::Microsecond => "us",
            Self::Millisecond => "ms",
            Self::Second => "s",
            Self::Minute => "min",
            Self::Hour => "h",
            Self::Day => "d",
            Self::Week => "w",
            Self::Month => "mo",
            Self::Year => "y",
        }
    }

    /// Parses a unit from its name or abbreviation, case-insensitively (`"ns"`, `"nanosecond"`,
    /// `"µs"`, `"mins"`, `"years"`, …). The inverse of [`name`](TimeUnit::name) /
    /// [`abbreviation`](TimeUnit::abbreviation).
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        // Accept an optional trailing plural "s" for the long names.
        let singular = lower
            .strip_suffix('s')
            .filter(|s| s.len() > 2)
            .unwrap_or(&lower);
        Some(match lower.as_str() {
            "ns" | "nanos" => Self::Nanosecond,
            "us" | "µs" | "μs" | "micros" => Self::Microsecond,
            "ms" | "millis" => Self::Millisecond,
            "s" | "sec" | "secs" => Self::Second,
            "min" | "mins" => Self::Minute,
            "h" | "hr" | "hrs" => Self::Hour,
            "d" => Self::Day,
            "w" | "wk" | "wks" => Self::Week,
            "mo" | "mos" | "mon" => Self::Month,
            "y" | "yr" | "yrs" => Self::Year,
            _ => match singular {
                "nanosecond" => Self::Nanosecond,
                "microsecond" => Self::Microsecond,
                "millisecond" => Self::Millisecond,
                "second" => Self::Second,
                "minute" => Self::Minute,
                "hour" => Self::Hour,
                "day" => Self::Day,
                "week" => Self::Week,
                "month" => Self::Month,
                "year" => Self::Year,
                _ => return None,
            },
        })
    }

    /// The matching Arrow [`TimeUnit`](arrow_schema::TimeUnit) (feature `arrow`) — only the four
    /// sub-second/second units Arrow models; `None` for `Minute`…`Year`.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(self) -> Option<arrow_schema::TimeUnit> {
        Some(match self {
            Self::Second => arrow_schema::TimeUnit::Second,
            Self::Millisecond => arrow_schema::TimeUnit::Millisecond,
            Self::Microsecond => arrow_schema::TimeUnit::Microsecond,
            Self::Nanosecond => arrow_schema::TimeUnit::Nanosecond,
            _ => return None,
        })
    }

    /// The [`TimeUnit`] for an Arrow [`TimeUnit`](arrow_schema::TimeUnit) (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(unit: arrow_schema::TimeUnit) -> Self {
        match unit {
            arrow_schema::TimeUnit::Second => Self::Second,
            arrow_schema::TimeUnit::Millisecond => Self::Millisecond,
            arrow_schema::TimeUnit::Microsecond => Self::Microsecond,
            arrow_schema::TimeUnit::Nanosecond => Self::Nanosecond,
        }
    }
}

impl core::fmt::Display for TimeUnit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.abbreviation())
    }
}

impl core::str::FromStr for TimeUnit {
    type Err = ();
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or(())
    }
}
