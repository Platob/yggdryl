//! Value-directed column typing, from the bytes a cell actually holds.
//!
//! The vocabulary is the one `text/plain` captures already use - boolean,
//! `int64`, `float64`, `date32`, `time64`, `datetime64`, and `utf8` as the
//! floor - so a resource read as lines and the same resource read as cells
//! name their columns the same way, and one text-to-value implementation
//! serves both. Anything outside that vocabulary is a declared schema's job:
//! declare it and the column is read as declared.
//!
//! Folding is [`DataType::merge_with`] with widening on, so a column of `1`
//! and `1.5` meets at `float64` and a column of `1` and `AAPL` meets at
//! `utf8`. A pair with no meeting point falls to `utf8` rather than refusing
//! the read: the cells are all still there, spelled the way the file spelled
//! them.

use crate::types::ascii::iso;
use crate::{DataType, Field, TimeUnit, Timezone};

/// The unit an inferred clock or instant column carries.
///
/// Microseconds hold every ISO reading this crate parses without turning a
/// whole column into nanoseconds no source asked for.
const INFERRED_UNIT: TimeUnit = TimeUnit::Microsecond;

/// One column's inferred datatype, folded cell by cell.
#[derive(Clone, Debug)]
pub(crate) struct Inference {
    dtype: DataType,
    /// Whether any sampled cell was absent, which makes the column nullable.
    absent: bool,
}

impl Inference {
    /// Start from a column nothing has been seen in yet.
    pub(crate) const fn new() -> Self {
        Self {
            dtype: DataType::Null,
            absent: false,
        }
    }

    /// Fold one cell into the column's answer.
    ///
    /// A quoted cell is content whatever it spells, so a quoted empty cell
    /// types the column as text rather than yielding to it as absence.
    pub(crate) fn observe(
        &mut self,
        cell: &[u8],
        quoted: bool,
        null: &str,
        timezone: Option<&Timezone>,
    ) {
        let observed = cell_dtype(cell, quoted, null, timezone);
        if matches!(observed, DataType::Null) {
            self.absent = true;
            return;
        }
        // A pair with no meeting point is not a refusal: text holds both.
        self.dtype = self
            .dtype
            .merge_with(&observed, true)
            .unwrap_or(DataType::Utf8);
    }

    /// Name the column, nullable because a later row may still be absent.
    pub(crate) fn into_field(self, name: impl Into<smol_str::SmolStr>) -> Field {
        // A column read from text is nullable whatever the sample held: the
        // sample is a prefix, and an unsampled row may still spell absence.
        let dtype = match self.dtype {
            DataType::Null => DataType::Utf8,
            dtype => dtype,
        };
        dtype.nullable_field(name)
    }
}

impl Default for Inference {
    fn default() -> Self {
        Self::new()
    }
}

/// Return whether a column of this datatype is read from cells directly.
///
/// The set is exactly what the shared text-to-value conversion implements. A
/// declared column outside it is read as text and converted by the shared cast,
/// which is the crate's one answer for a datatype an encoding cannot spell.
pub(crate) const fn is_read_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Utf8
            | DataType::Boolean
            | DataType::Int64
            | DataType::Float64
            | DataType::Date32
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::DateTime64 { .. }
    )
}

/// Name the datatype one cell's bytes are.
///
/// The ladder is narrowest first, so `7` is an integer rather than a double
/// and `2024-01-01` a date rather than an instant. A numeric with a leading
/// zero stays text: a zero-padded code is an identifier whose padding is part
/// of the value, and reading it as a number deletes that.
fn cell_dtype(cell: &[u8], quoted: bool, null: &str, timezone: Option<&Timezone>) -> DataType {
    if !quoted && cell == null.as_bytes() {
        return DataType::Null;
    }
    if cell.is_empty() {
        return DataType::Utf8;
    }
    // Every typed spelling below is ASCII, so a cell that is not UTF-8 is text
    // and needs no validation pass to prove it.
    let Ok(text) = std::str::from_utf8(cell) else {
        return DataType::Utf8;
    };
    if text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false") {
        return DataType::Boolean;
    }
    if let Some(numeric) = numeric_dtype(text) {
        return numeric;
    }
    temporal_dtype(text, timezone).unwrap_or(DataType::Utf8)
}

/// Name an integer or a double, refusing a padded numeric identifier.
fn numeric_dtype(text: &str) -> Option<DataType> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.starts_with(|first: char| first.is_ascii_digit()) {
        return None;
    }
    if is_zero_padded(digits) {
        return Some(DataType::Utf8);
    }
    if digits.bytes().all(|byte| byte.is_ascii_digit()) {
        // A whole number too wide for the column is text, because widening it
        // to a double would drop digits the file spelled out.
        return Some(
            text.parse::<i64>()
                .map_or(DataType::Utf8, |_| DataType::Int64),
        );
    }
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|_| DataType::Float64)
}

/// Return whether a numeric spelling carries meaning in its leading zeros.
fn is_zero_padded(digits: &str) -> bool {
    digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.")
}

/// Name a date, a clock, or an instant, whichever the reading spells.
fn temporal_dtype(text: &str, timezone: Option<&Timezone>) -> Option<DataType> {
    if iso::parse_date(text).is_ok() {
        return Some(DataType::Date32);
    }
    if iso::parse_time(text).is_ok() {
        return Some(DataType::Time64(INFERRED_UNIT));
    }
    if iso::parse_datetime(text).is_ok() {
        // A reading with no offset is a wall clock in the column's zone, which
        // is naive until the caller names one.
        return Some(DataType::DateTime64 {
            unit: INFERRED_UNIT,
            timezone: timezone.copied().unwrap_or(Timezone::NAIVE),
        });
    }
    if iso::parse_timestamp(text).is_ok() {
        // A reading that names its own offset is an instant, so the column is
        // zoned whether or not the caller named a zone.
        return Some(DataType::DateTime64 {
            unit: INFERRED_UNIT,
            timezone: timezone.copied().unwrap_or(Timezone::UTC),
        });
    }
    None
}
