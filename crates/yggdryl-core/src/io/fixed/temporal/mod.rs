//! `io::fixed::temporal` — the **temporal** value types: dates, times, timestamps, and durations,
//! over a [`TimeUnit`] resolution and a [`Tz`] timezone, mirroring (and extending) Arrow's temporal
//! family.
//!
//! Two shared axes, exposed by the [`Temporal`] trait, run through everything: a value's
//! **resolution** ([`TimeUnit`], nanoseconds → years) and its **timezone** ([`Tz`]: naive, UTC, a
//! fixed offset, or a DST-aware IANA zone backed by the full tz database). Over those sit the
//! byte-width value types, all self-describing native Rust values with easy conversions between
//! one another:
//!
//! | concept | widths | backing | meaning |
//! | --- | --- | --- | --- |
//! | date | [`Date32`], [`Date64`] | `i32` days / `i64` millis | a calendar day (naive) |
//! | time-of-day | [`Time32`], [`Time64`] | `i32` (s/ms) / `i64` (µs/ns) | a wall-clock time (naive) |
//! | timestamp | [`Ts32`], [`Ts64`], [`Ts96`] | `i32`/`i64`/`i96` count since epoch | an instant (naive or zoned) |
//! | duration | [`Duration32`], [`Duration64`] | `i32` / `i64` count | an elapsed span (unit only) |
//!
//! `chrono` / `chrono-tz` power the timezone offsets and are kept an implementation detail — they
//! never appear in a public signature.

mod civil;
mod date;
mod duration;
mod error;
mod parse;
mod time;
mod time_unit;
mod timestamp;
mod timezone;

pub use date::{Date32, Date64};
pub use duration::{Duration32, Duration64};
pub use error::TemporalError;
pub use time::{Time32, Time64};
pub use time_unit::TimeUnit;
pub use timestamp::{Ts32, Ts64, Ts96};
pub use timezone::{Timezone, Tz};

/// The contract shared by every temporal value: its resolution ([`TimeUnit`]) and its timezone
/// ([`Tz`], naive for the zone-less date/time/duration types).
///
/// Every value type in this module — [`Date32`], [`Time64`], [`Ts64`], [`Duration64`], … —
/// reports its [`time_unit`](Temporal::time_unit) and its [`timezone`](Temporal::timezone), so code
/// can be generic over "a temporal value" and read those two axes uniformly.
pub trait Temporal {
    /// The value's resolution (`Day` for a date, `Nanosecond` for a nanosecond timestamp, …).
    fn time_unit(&self) -> TimeUnit;

    /// The value's timezone — [`Tz::NAIVE`] for the zone-less date / time / duration types, a real
    /// zone for a zoned timestamp.
    fn timezone(&self) -> Tz;

    /// Whether the value carries an explicit (non-naive) timezone.
    fn is_zoned(&self) -> bool {
        !self.timezone().is_naive()
    }
}
