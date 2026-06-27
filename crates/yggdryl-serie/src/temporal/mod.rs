//! **Temporal** series — columns whose values are calendar/clock values, presented
//! through the core [`DateTime`](yggdryl_core::DateTime) / [`Date`](yggdryl_core::Date)
//! / [`Time`](yggdryl_core::Time) types.
//!
//! - [`TemporalSerie`] — the shared trait: a uniform [`datetime_at`](TemporalSerie::datetime_at)
//!   plus the derived [`date_at`](TemporalSerie::date_at) / [`time_at`](TemporalSerie::time_at).
//! - [`DatetimeSerie`] — the unified timestamp column (any [`TimeUnit`](yggdryl_core::TimeUnit)
//!   + optional timezone).
//! - [`TimeSerie`] — the unified time-of-day column (any unit), values as
//!   [`Time`](yggdryl_core::Time).
//! - [`DurationSerie`] — the unified elapsed-time column (any unit), values as
//!   [`Duration`](yggdryl_core::Duration) (a span, so *not* a [`TemporalSerie`]).
//!
//! The lazy temporal **ranges** ([`DateRangeSerie`](crate::DateRangeSerie),
//! [`DateTimeRangeSerie`](crate::DateTimeRangeSerie), [`TimeRangeSerie`](crate::TimeRangeSerie))
//! also implement [`TemporalSerie`].

mod datetime;
mod duration;
mod time;

pub use datetime::DatetimeSerie;
pub use duration::DurationSerie;
pub use time::TimeSerie;

use yggdryl_core::{Date, DateTime, Time};

use crate::serie::Serie;

/// The shared interface of a temporal column: read any row as a core
/// [`DateTime`](yggdryl_core::DateTime), with the [`Date`](yggdryl_core::Date) /
/// [`Time`](yggdryl_core::Time) projections derived from it.
pub trait TemporalSerie: Serie {
    /// The value at `index` as a [`DateTime`], or `None` when null / out of bounds.
    fn datetime_at(&self, index: usize) -> Option<DateTime>;

    /// The value at `index` projected to its [`Date`] (the calendar day).
    fn date_at(&self, index: usize) -> Option<Date> {
        self.datetime_at(index).map(|dt| dt.date())
    }

    /// The value at `index` projected to its [`Time`] (the clock time of day).
    fn time_at(&self, index: usize) -> Option<Time> {
        self.datetime_at(index).map(|dt| dt.time())
    }
}
