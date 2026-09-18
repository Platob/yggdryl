//! What a message states, structured: each field under the tag and the
//! name the dictionary gives it, a component's members and a repeating
//! group's occurrences nested under the entry that heads them.
//!
//! A [`FixMsg`](crate::FixMsg) holds its typed facts as fields of its own
//! and everything else as its row, and the entries are that row read as a
//! tree: one entry per field the message states, each carrying the tag the
//! dictionary resolved - `0` for a key no dictionary explains - the
//! canonical name, and the value as the wire spells it. A repeating group
//! is one entry under its counter, its value the count, and each occurrence
//! is an entry under it with no value of its own and the occurrence's
//! members nested beneath; a component is an entry with no value and its
//! members beneath. So a consumer walks one shape whatever the message
//! carried, and a wire re-emits from it in pre-order: an entry with a value
//! is one pair, an entry without one is the pairs under it.

use smol_str::SmolStr;

/// One field a message states, beside what is nested under it.
///
/// Every part is present but the value: a resolved field carries its
/// canonical positive tag and name, an unresolved key carries `0` and its
/// own spelling, and an entry that heads others - an occurrence, a
/// component - carries no value at all. Registry tags start at `1`, so `0`
/// is never a field.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FixEntry {
    tag: i32,
    name: SmolStr,
    value: Option<SmolStr>,
    fixentries: Vec<FixEntry>,
}

impl FixEntry {
    /// One entry stating `value` under `tag` and `name`, with nothing
    /// nested under it yet.
    #[must_use]
    pub fn new(tag: i32, name: impl Into<SmolStr>, value: Option<SmolStr>) -> Self {
        Self {
            tag: tag.max(0),
            name: name.into(),
            value,
            fixentries: Vec::new(),
        }
    }

    /// This entry with `entries` nested under it, in their order.
    #[must_use]
    pub fn with_entries(mut self, entries: Vec<FixEntry>) -> Self {
        self.fixentries = entries;
        self
    }

    /// The resolved canonical tag, or `0` for a key no dictionary explains.
    #[must_use]
    pub const fn tag(&self) -> i32 {
        self.tag
    }

    /// The dictionary's canonical name for the field, else the key as it
    /// arrived.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value as the wire spells it, or nothing for an entry that only
    /// heads others.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// The entries nested under this one, in their order: a group's
    /// occurrences, an occurrence's or a component's members.
    #[must_use]
    pub fn entries(&self) -> &[FixEntry] {
        &self.fixentries
    }

    /// Whether this entry states a value of its own.
    #[must_use]
    pub const fn is_stated(&self) -> bool {
        self.value.is_some()
    }
}

/// Emits entries pre-order: each stated pair, then everything nested under
/// it. An entry stating nothing - an occurrence, a component - is the pairs
/// under it.
pub(super) fn emit_bytes(entries: &[FixEntry], separator: u8, bytes: &mut Vec<u8>) {
    for entry in entries {
        if let Some(value) = entry.value() {
            emit_key(entry, bytes);
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(separator);
        }
        emit_bytes(entry.entries(), separator, bytes);
    }
}

/// The key an entry re-emits under: its tag where the dictionary resolved
/// one, its own spelling otherwise.
fn emit_key(entry: &FixEntry, bytes: &mut Vec<u8>) {
    if entry.tag() > 0 {
        bytes.extend_from_slice(smol_str::format_smolstr!("{}", entry.tag()).as_bytes());
    } else {
        bytes.extend_from_slice(entry.name().as_bytes());
    }
}

/// The same walk as bytes, refusing a control byte in any value.
pub(super) fn emit_text(
    entries: &[FixEntry],
    separator: char,
    text: &mut String,
) -> crate::Result<()> {
    for entry in entries {
        if let Some(value) = entry.value() {
            if value.chars().any(char::is_control) {
                return Err(crate::Error::InvalidRecord {
                    path: SmolStr::new(entry.name()),
                    reason: "expected a printable value, got a control byte".into(),
                });
            }
            let mut key = Vec::new();
            emit_key(entry, &mut key);
            text.push_str(std::str::from_utf8(&key).expect("a tag or a name is text"));
            text.push('=');
            text.push_str(value);
            text.push(separator);
        }
        emit_text(entry.entries(), separator, text)?;
    }
    Ok(())
}

/// The text a value spells on the wire: a code or text as it is, a boolean
/// as `Y` or `N`, an instant as FIX spells one - `YYYYMMDD-HH:MM:SS` with
/// the shortest fraction that keeps it exact - a day as `YYYYMMDD`, a
/// number as its canonical text; nothing for a nested value, which is never
/// one pair.
/// [`wire_text`] under the field the value is stated as: a coded value -
/// a side, a state - spells the wire code the field's own code set gives
/// its name, so `BUY` under `Side(54)` is `1` on the wire and `BUY` in a
/// dictionary that never coded it; everything else spells as it does
/// under no field.
pub(super) fn wire_text_under(field: &crate::Field, value: &crate::Scalar) -> Option<SmolStr> {
    let coded = value.is_code()
        || matches!(
            field.dtype(),
            crate::DataType::Side | crate::DataType::State
        );
    if coded {
        if let Some(code) = value
            .as_str()
            .and_then(|name| field.as_fix().code_by_name(name))
        {
            return Some(SmolStr::new(code.value()));
        }
    }
    wire_text(value)
}

pub(super) fn wire_text(value: &crate::Scalar) -> Option<SmolStr> {
    use crate::Scalar;
    match value {
        Scalar::String(_) => value.as_str().map(SmolStr::new),
        coded if coded.is_code() => value.as_str().map(SmolStr::new),
        Scalar::Boolean(_) => value
            .as_bool()
            .map(|held| SmolStr::new_static(if held { "Y" } else { "N" })),
        Scalar::Sequence(_) | Scalar::Mapping(_) | Scalar::Record(_) | Scalar::Null => None,
        Scalar::Bytes(held) => Some(SmolStr::new(String::from_utf8_lossy(held.as_bytes()))),
        Scalar::Version(held) => Some(smol_str::format_smolstr!("{held}")),
        Scalar::DateTime64(_) => {
            let (count, unit, _) = value.as_datetime64()?;
            Some(fix_timestamp(count.checked_mul(nanos_per(unit)?)?))
        }
        Scalar::Date32(_) => {
            let (days, _, _) = value.as_date32()?;
            Some(fix_date(i64::from(days)))
        }
        Scalar::Date64(_) => {
            let (count, unit, _) = value.as_date64()?;
            let nanos = count.checked_mul(nanos_per(unit)?)?;
            Some(fix_date(nanos.div_euclid(NANOS_PER_DAY)))
        }
        // A decimal writes the number it is rather than the scale it is
        // stored at: this crate keeps a price and a quantity exact, at
        // `decimal128(38, 18)`, and a wire that spelled `12.5` as
        // `12.500000000000000000` would be stating the storage.
        Scalar::Decimal32(_)
        | Scalar::Decimal64(_)
        | Scalar::Decimal128(_)
        | Scalar::Decimal256(_) => crate::types::Decimal::from_scalar(value)
            .map(|held| smol_str::format_smolstr!("{held}")),
        // Every other number and duration writes its leaf's own canonical text.
        other => other
            .leaf_display()
            .map(|held| smol_str::format_smolstr!("{held}")),
    }
}

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

/// How many nanoseconds one count of `unit` is, for the clock units; a
/// calendar unit counts no nanoseconds and answers nothing.
const fn nanos_per(unit: crate::TimeUnit) -> Option<i64> {
    match unit {
        crate::TimeUnit::Second => Some(1_000_000_000),
        crate::TimeUnit::Millisecond => Some(1_000_000),
        crate::TimeUnit::Microsecond => Some(1_000),
        crate::TimeUnit::Nanosecond => Some(1),
        crate::TimeUnit::Day => Some(NANOS_PER_DAY),
        crate::TimeUnit::YearMonth | crate::TimeUnit::DayTime | crate::TimeUnit::MonthDayNano => {
            None
        }
    }
}

/// One day since the epoch as FIX spells it: `YYYYMMDD`.
fn fix_date(days: i64) -> SmolStr {
    let (year, month, day) = crate::types::timezone::civil_from_days(days);
    smol_str::format_smolstr!("{year:04}{month:02}{day:02}")
}

/// One instant as FIX spells it: `YYYYMMDD-HH:MM:SS`, then the shortest of
/// no fraction, three, six or nine digits that keeps the count exact.
fn fix_timestamp(nanos: i64) -> SmolStr {
    let days = nanos.div_euclid(NANOS_PER_DAY);
    let rest = nanos.rem_euclid(NANOS_PER_DAY);
    let seconds = rest / 1_000_000_000;
    let fraction = rest % 1_000_000_000;
    let (hour, minute, second) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    let date = fix_date(days);
    if fraction == 0 {
        smol_str::format_smolstr!("{date}-{hour:02}:{minute:02}:{second:02}")
    } else if fraction % 1_000_000 == 0 {
        smol_str::format_smolstr!(
            "{date}-{hour:02}:{minute:02}:{second:02}.{:03}",
            fraction / 1_000_000
        )
    } else if fraction % 1_000 == 0 {
        smol_str::format_smolstr!(
            "{date}-{hour:02}:{minute:02}:{second:02}.{:06}",
            fraction / 1_000
        )
    } else {
        smol_str::format_smolstr!("{date}-{hour:02}:{minute:02}:{second:02}.{fraction:09}")
    }
}
