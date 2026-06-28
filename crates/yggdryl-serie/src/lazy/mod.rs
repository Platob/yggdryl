//! **Lazy** (computed) series — columns that are *not* fully resident in memory but
//! produce their values on demand from a compact description, materialising into a real
//! Arrow-backed column only when asked. Each is a full [`Serie`](crate::Serie):
//!
//! - [`RangeSerie`] — a datatype-generic arithmetic range (its `start` / `end` / `step` are
//!   [`ScalarValue`](yggdryl_scalar::ScalarValue)s, computed via [`Scalar`](yggdryl_scalar::Scalar)
//!   math), a `uint64` one doubling as the canonical row index (O(1) label ↔ position lookups).
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
