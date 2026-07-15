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

mod backing;
mod civil;
mod date;
mod dtype;
mod duration;
mod error;
mod field;
mod parse;
mod scalar;
mod serie;
mod time;
mod time_unit;
mod timestamp;
mod timezone;
mod widths;

pub use date::{Date32, Date64};
pub use duration::{Duration32, Duration64};
pub use error::TemporalError;
pub use time::{Time32, Time64};
pub use time_unit::TimeUnit;
pub use timestamp::{Ts32, Ts64, Ts96};
pub use timezone::{Timezone, Tz};

// The columnar layer: the two shared traits, the generic quartet, and the nine concept+width
// markers with their `*Type` / `*Field` / `*Scalar` / `*Serie` aliases.
pub use backing::{TemporalBacking, TemporalNative};
pub use dtype::TemporalType;
pub use field::TemporalField;
pub use scalar::TemporalScalar;
pub use serie::TemporalSerie;
pub use widths::{
    Date32Field, Date32Kind, Date32Scalar, Date32Serie, Date32Type, Date64Field, Date64Kind,
    Date64Scalar, Date64Serie, Date64Type, Duration32Field, Duration32Kind, Duration32Scalar,
    Duration32Serie, Duration32Type, Duration64Field, Duration64Kind, Duration64Scalar,
    Duration64Serie, Duration64Type, Time32Field, Time32Kind, Time32Scalar, Time32Serie,
    Time32Type, Time64Field, Time64Kind, Time64Scalar, Time64Serie, Time64Type, Ts32Field,
    Ts32Kind, Ts32Scalar, Ts32Serie, Ts32Type, Ts64Field, Ts64Kind, Ts64Scalar, Ts64Serie,
    Ts64Type, Ts96Field, Ts96Kind, Ts96Scalar, Ts96Serie, Ts96Type,
};

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
