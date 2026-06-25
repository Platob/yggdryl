//! UTC wall-clock timestamps for request/response timing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The current UTC time as Unix-epoch **seconds** (`0.0` if the clock is before
/// the epoch).
pub(crate) fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Parses an HTTP date (the three RFC 7231 formats) to Unix-epoch seconds,
/// dependency-free. Returns `None` on any unparseable input. Shared by the cookie
/// jar's `Expires` and the `Retry-After` header's date form.
pub(crate) fn parse_http_date(value: &str) -> Option<f64> {
    // IMF-fixdate / RFC 850: "Sun, 06 Nov 1994 08:49:37 GMT" /
    // "Sunday, 06-Nov-94 08:49:37 GMT". asctime: "Sun Nov  6 08:49:37 1994".
    let value = value.trim();
    let (day, month, year, time) = if let Some(rest) = value.split_once(", ") {
        // IMF-fixdate or RFC 850 — split the date portion on space or `-`.
        let date = rest.1;
        let mut fields = date.split([' ', '-']).filter(|field| !field.is_empty());
        let day = fields.next()?;
        let month = fields.next()?;
        let year = fields.next()?;
        let time = fields.next()?;
        (day, month, year, time)
    } else {
        // asctime: "Sun Nov  6 08:49:37 1994".
        let mut fields = value.split_whitespace();
        let _weekday = fields.next()?;
        let month = fields.next()?;
        let day = fields.next()?;
        let time = fields.next()?;
        let year = fields.next()?;
        (day, month, year, time)
    };

    let day: i64 = day.parse().ok()?;
    let month = month_index(month)?;
    let mut year: i64 = year.parse().ok()?;
    if year < 100 {
        // Two-digit years (RFC 850): 70–99 → 1900s, else 2000s (RFC 6265 §5.1.1).
        year += if year >= 70 { 1900 } else { 2000 };
    }
    let mut time_fields = time.split(':');
    let hour: i64 = time_fields.next()?.parse().ok()?;
    let minute: i64 = time_fields.next()?.parse().ok()?;
    let second: i64 = time_fields.next().unwrap_or("0").parse().ok()?;

    Some(civil_to_epoch(year, month, day, hour, minute, second) as f64)
}

/// Month abbreviation (`Jan`…`Dec`) to a 1-based index.
fn month_index(name: &str) -> Option<i64> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let name = name.get(..3)?.to_ascii_lowercase();
    months
        .iter()
        .position(|month| *month == name)
        .map(|index| index as i64 + 1)
}

/// Converts a proleptic-Gregorian civil date-time (UTC) to Unix-epoch seconds,
/// using Howard Hinnant's `days_from_civil` algorithm (no `chrono` dependency).
fn civil_to_epoch(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146097 + day_of_era - 719468;
    days * 86400 + hour * 3600 + minute * 60 + second
}

/// A shared, settable UTC timestamp (Unix-epoch seconds) — an `f64` stored in an
/// [`AtomicU64`] via its bit pattern, so an [`HttpStream`](crate::HttpStream) and
/// the [`HttpResponse`](crate::HttpResponse) that returned it can read the same
/// "connection done" instant once the body reaches EOF or is closed. Defaults to
/// `0.0` (unset).
#[derive(Debug, Clone, Default)]
pub(crate) struct Instant(Arc<AtomicU64>);

impl Instant {
    /// A fresh, unset instant reading `0.0`.
    pub(crate) fn new() -> Instant {
        Instant(Arc::new(AtomicU64::new(0)))
    }

    /// The stored timestamp in Unix-epoch seconds (`0.0` when unset).
    pub(crate) fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::SeqCst))
    }

    /// Stamps the current time, but only if still unset (so the first "done"
    /// wins and a later `close` after EOF does not overwrite it).
    pub(crate) fn stamp_once(&self) {
        if self.get() == 0.0 {
            self.0.store(now_secs().to_bits(), Ordering::SeqCst);
        }
    }
}
