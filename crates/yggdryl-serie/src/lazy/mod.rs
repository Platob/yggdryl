//! **Lazy** (computed) series — columns that are *not* fully resident in memory but
//! produce their values on demand from a compact description, materialising into a real
//! Arrow-backed column only when asked. Each is a full [`Serie`](crate::Serie):
//!
//! - [`RangeSerie`] — a `uint64` arithmetic range (the backing of the default
//!   [`IndexSerie`](crate::IndexSerie)).
//! - [`DateRangeSerie`] — a day-resolution calendar-date range.

mod daterange;
mod range;

pub use daterange::DateRangeSerie;
pub use range::RangeSerie;
