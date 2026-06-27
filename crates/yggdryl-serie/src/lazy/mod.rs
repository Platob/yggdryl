//! **Lazy** (computed) series — columns that are *not* fully resident in memory but
//! produce their values on demand from a compact description, materialising into a real
//! Arrow-backed column only when asked. Each is a full [`Serie`](crate::Serie):
//!
//! - [`RangeSerie`] — a `uint64` arithmetic range (the backing of the default
//!   [`IndexSerie`](crate::IndexSerie)).
//! - [`DateRangeSerie`] — a day-resolution calendar-date range.
//! - [`DateTimeRangeSerie`] — a nanosecond timestamp range.
//! - [`TimeRangeSerie`] — a time-of-day range (wrapping within the day).
//!
//! The three temporal ranges implement [`TemporalSerie`](crate::TemporalSerie).

mod daterange;
mod datetimerange;
mod range;
mod timerange;

pub use daterange::DateRangeSerie;
pub use datetimerange::DateTimeRangeSerie;
pub use range::RangeSerie;
pub use timerange::TimeRangeSerie;
