//! The bridge's capture as a template copies are rendered from: every copy
//! the same lines at the same width, each clock moved by the copy and every
//! identifier stepped by it, so no copy repeats another and a walk over many
//! of them sees distinct chains.
//!
//! One fixture, because three targets render copies: the scale harness
//! (`scale_ulbridge.rs`), the allocation pins (`allocations.rs`) and the
//! `fix/pipeline` benchmark's `decoded_lifecycle_distinct`.

use regex::bytes::Regex;

/// The capture every copy repeats, exactly as the bridge wrote it.
pub const LOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fix/ulbridge.log"
));

/// The lines one copy holds.
pub const LINES_PER_COPY: u64 = 144;

/// One hour and one day, in seconds.
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// The bridge writes Zurich's local clock in front of every line, and the
/// whole capture falls in Zurich summer time: two hours ahead of UTC.
pub const ZURICH_SUMMER: i64 = 2 * HOUR;

/// The fields whose values are identifiers, as the capture spells them: FIX
/// tags, the bridge's own names and the FIXML attributes. Every occurrence
/// of a value collected here is stepped, wherever it stands - a FIX frame, a
/// bridge row, the prose in front of one, a document inside another.
pub const ID_KEYS: &[&str] = &[
    "11",
    "17",
    "19",
    "37",
    "41",
    "198",
    "526",
    "527",
    "571",
    "818",
    "880",
    "1003",
    "1903",
    "9432",
    "9507",
    "CLORDID",
    "ORIGCLORDID",
    "ORDERID",
    "EXECID",
    "EXECREFID",
    "SECONDARYORDERID",
    "SECONDARYEXECID",
    "SECONDARYCLORDID",
    "TRDMATCHID",
    "TRADEREPORTID",
    "TRADEID",
    "PACKAGEID",
    "PARENTORDERID",
    "#PARENTORDERID",
    "#PARENTCLORDID",
    "#MARKETORDERID",
    "#OMSDEALERORDERID",
    "#OMSDEALERPARENTORDERID",
    "#ULTRADER_CLORDID",
    "#TRANSVERSAL_KEY",
    "#OMSACTIONID",
    "MESSAGELINKID",
    "EXCHANGECLIENTORDERID",
    "CONVERSATIONID",
    "REGULATORYTRADEID",
    "TRADINGVENUETRANSACTIONIDENTIFIERCODE",
    "MAIN.UTI",
    "HEDGEVENUETRANSID",
    "TR_LEGVENUETRANSID",
    "ExecID",
    "ClOrdID",
    "OrderID",
    "OrigClOrdID",
];

/// One rewrite a copy makes at a fixed place of a line: every one keeps its
/// width, so a body length, an `XmlData` length and every offset stand.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// The row header's `YYYY-MM-DD HH:MM:SS`, Zurich's clock, written back
    /// in UTC; its fraction is left as it is.
    Header { utc: i64 },
    /// `YYYYMMDD-HH:MM:SS`; any fraction after it is left as it is.
    Dashed { utc: i64 },
    /// `YYYYMMDDHHMMSS`; any fraction digits after it are left as they are.
    Compact { utc: i64 },
    /// `YYYYMMDD`, moved by the days the line's own clock moved.
    Date { days: i64 },
    /// An identifier: its letters and digits stepped by the copy, the
    /// lower-case hexadecimal ones within their sixteen.
    Id { len: usize, hex: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub at: usize,
    pub kind: Kind,
}

/// One line of the capture and every place a copy rewrites in it.
pub struct Line {
    pub bytes: Vec<u8>,
    /// The line's own clock, in UTC seconds.
    pub header_utc: i64,
    /// What copy zero moves the line's clocks by: its hour packed beside
    /// the capture's other hours.
    pub slot_shift: i64,
    pub patches: Vec<Patch>,
}

/// The capture as a template copies are rendered from.
///
/// The bridge's capture is four bursts on one day - 01:03, 12:46, 14:52 and
/// 21:59 UTC - logged out of order. A copy packs each burst into its own
/// hour, in order, and its lines are emitted in clock order, so a copy spans
/// [`Self::span`] - four hours. Copy `k` is copy zero moved by `k / density`
/// spans and `k % density` seconds: `density` copies share each span, a
/// second apart, and at a density of one a twenty-gibibyte run would be some
/// forty-five years of nearly empty quarters. Every clock of a line moves with
/// it, an order's expiry included, so a chain expires in the copy that
/// placed it; a date moves by the days its line's clock moved.
///
/// Every identifier is stepped by the copy number written in mixed radix
/// from its first character, a digit within the ten digits and a letter
/// within its case: one value steps alike wherever it stands, a value that
/// extends another (`00079132557GLXC0.9` after `00079132557GLXC0`) keeps
/// extending it, and no value comes back in another copy. Instruments -
/// ISINs, tickers, markets - are left as they are, so the books the copies
/// build are the same books.
pub struct Template {
    pub lines: Vec<Line>,
    pub span: i64,
    /// The copies one span holds.
    pub density: u64,
}

impl Template {
    pub fn new(density: u64) -> Self {
        assert!(density > 0, "a span holds at least one copy");
        let header = Regex::new(r"(?-u)^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})")
            .expect("the header clock pattern");
        let context = Regex::new(r"(?-u)^[^\[]*\[\d+-[0-9a-f]{8}:([0-9a-f]{10}):\d+\]")
            .expect("the capture context pattern");
        let identifiers = identifiers();
        let mut lines: Vec<Line> = LOG
            .split_inclusive(|byte| *byte == b'\n')
            .map(|line| {
                let clock = header
                    .captures(line)
                    .expect("every line opens with its clock");
                let field = |index: usize| parse_digits(&clock[index]);
                let local = epoch(field(1), field(2), field(3), field(4), field(5), field(6))
                    .expect("a valid header clock");
                let header_utc = local - ZURICH_SUMMER;
                let mut patches = vec![Patch {
                    at: 0,
                    kind: Kind::Header { utc: header_utc },
                }];
                if let Some(found) = context.captures(line) {
                    let at = found.get(1).expect("the context group").start();
                    patches.push(Patch {
                        at,
                        kind: Kind::Id { len: 10, hex: true },
                    });
                }
                identifier_patches(line, &identifiers, &mut patches);
                clock_patches(line, &mut patches);
                patches.sort_by_key(|patch| patch.at);
                Line {
                    bytes: line.to_vec(),
                    header_utc,
                    slot_shift: 0,
                    patches,
                }
            })
            .collect();
        assert_eq!(lines.len() as u64, LINES_PER_COPY, "the capture's lines");
        let mut hours: Vec<i64> = lines
            .iter()
            .map(|line| line.header_utc.div_euclid(HOUR))
            .collect();
        hours.sort_unstable();
        hours.dedup();
        for line in &mut lines {
            let hour = line.header_utc.div_euclid(HOUR);
            let slot = hours.binary_search(&hour).expect("a listed hour") as i64;
            line.slot_shift = (hours[0] + slot - hour) * HOUR;
        }
        lines.sort_by_key(|line| line.header_utc + line.slot_shift);
        Self {
            lines,
            span: hours.len() as i64 * HOUR,
            density,
        }
    }

    /// Copy `copy` appended to `out`.
    pub fn render(&self, copy: u64, out: &mut Vec<u8>) {
        let moved = i64::try_from(copy / self.density).expect("a span count") * self.span
            + i64::try_from(copy % self.density).expect("a second within the span");
        for line in &self.lines {
            let start = out.len();
            out.extend_from_slice(&line.bytes);
            let target = &mut out[start..];
            let shift = moved + line.slot_shift;
            let days = (line.header_utc + shift).div_euclid(DAY) - line.header_utc.div_euclid(DAY);
            for patch in &line.patches {
                let at = &mut target[patch.at..];
                match patch.kind {
                    Kind::Header { utc } => write_clock(at, utc + shift, ClockLayout::Header),
                    Kind::Dashed { utc } => write_clock(at, utc + shift, ClockLayout::Dashed),
                    Kind::Compact { utc } => write_clock(at, utc + shift, ClockLayout::Compact),
                    Kind::Date { days: date } => write_date(at, date + days),
                    Kind::Id { len, hex } => step_identifier(&mut at[..len], copy, hex),
                }
            }
        }
    }
}

/// Every identifier value the capture states under one of [`ID_KEYS`],
/// longest first so a value that extends another is matched whole.
pub fn identifiers() -> Vec<Vec<u8>> {
    // `^A` is how one dump spells the field separator, so a key may follow it.
    let pair = Regex::new(
        r#"(?-u)(?:\^A|^|[^A-Za-z0-9_.#])(#?[A-Za-z0-9_.]+)=\[?"?([A-Za-z0-9][A-Za-z0-9._:/@-]*)"#,
    )
    .expect("the pair pattern");
    let mut values: Vec<Vec<u8>> = Vec::new();
    for line in LOG.split(|byte| *byte == b'\n') {
        for found in pair.captures_iter(line) {
            let key = &found[1];
            let value = &found[2];
            let named = ID_KEYS.iter().any(|id| id.as_bytes() == key);
            let all_digits = value.iter().all(u8::is_ascii_digit);
            if named
                && value.len() >= 6
                && value.iter().any(u8::is_ascii_digit)
                && !(all_digits && value.len() == 8)
            {
                values.push(value.to_vec());
            }
        }
    }
    values.sort_by(|left, right| right.len().cmp(&left.len()).then(left.cmp(right)));
    values.dedup();
    values
}

/// Every whole occurrence of an identifier in `line`: one neither preceded
/// nor followed by a letter or a digit, the longest one at each place.
pub fn identifier_patches(line: &[u8], identifiers: &[Vec<u8>], patches: &mut Vec<Patch>) {
    let mut at = 0;
    while at < line.len() {
        let bounded_before = at == 0 || !line[at - 1].is_ascii_alphanumeric();
        let found = bounded_before
            .then(|| {
                identifiers.iter().find(|value| {
                    let end = at + value.len();
                    line[at..].starts_with(value)
                        && (end == line.len() || !line[end].is_ascii_alphanumeric())
                })
            })
            .flatten();
        let Some(value) = found else {
            at += 1;
            continue;
        };
        let taken = patches.iter().any(|patch| overlaps(patch, at, value.len()));
        if !taken {
            let hex = is_uuid(value);
            patches.push(Patch {
                at,
                kind: Kind::Id {
                    len: value.len(),
                    hex,
                },
            });
        }
        at += value.len();
    }
}

/// Every clock and date in `line` that no identifier and no header holds.
pub fn clock_patches(line: &[u8], patches: &mut Vec<Patch>) {
    let mut at = 0;
    while at < line.len() {
        if !line[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < line.len() && line[at].is_ascii_digit() {
            at += 1;
        }
        let run = &line[start..at];
        if patches
            .iter()
            .any(|patch| overlaps(patch, start, run.len()))
        {
            continue;
        }
        let date = (run.len() >= 8).then(|| date_of(&run[..8])).flatten();
        let Some(days) = date else {
            continue;
        };
        if run.len() == 8
            && let Some((hour, minute, second)) = dashed_time(&line[at..])
        {
            patches.push(Patch {
                at: start,
                kind: Kind::Dashed {
                    utc: days * DAY + hour * HOUR + minute * 60 + second,
                },
            });
            at = start + 17;
        } else if run.len() == 8 {
            patches.push(Patch {
                at: start,
                kind: Kind::Date { days },
            });
        } else if matches!(run.len(), 14 | 17 | 20)
            && let Some((hour, minute, second)) = time_of(&run[8..14])
        {
            patches.push(Patch {
                at: start,
                kind: Kind::Compact {
                    utc: days * DAY + hour * HOUR + minute * 60 + second,
                },
            });
        }
    }
}

pub fn overlaps(patch: &Patch, at: usize, len: usize) -> bool {
    let patch_len = match patch.kind {
        Kind::Header { .. } => 19,
        Kind::Dashed { .. } => 17,
        Kind::Compact { .. } => 14,
        Kind::Date { .. } => 8,
        Kind::Id { len, .. } => len,
    };
    at < patch.at + patch_len && patch.at < at + len
}

/// Whether `value` is a UUID spelled in lower-case hexadecimal.
pub fn is_uuid(value: &[u8]) -> bool {
    value.len() == 36
        && value.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}

/// `copy` written in mixed radix over the identifier's letters and digits,
/// from its first, added place by place without a carry.
pub fn step_identifier(value: &mut [u8], copy: u64, hex: bool) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut rest = copy;
    for byte in value.iter_mut() {
        if rest == 0 {
            break;
        }
        let (alphabet_start, radix, index): (u8, u64, u64) = match *byte {
            b'0'..=b'9' if hex => (0, 16, u64::from(*byte - b'0')),
            b'a'..=b'f' if hex => (0, 16, u64::from(*byte - b'a') + 10),
            b'0'..=b'9' => (b'0', 10, u64::from(*byte - b'0')),
            b'a'..=b'z' => (b'a', 26, u64::from(*byte - b'a')),
            b'A'..=b'Z' => (b'A', 26, u64::from(*byte - b'A')),
            _ => continue,
        };
        let stepped = (index + rest % radix) % radix;
        rest /= radix;
        *byte = if hex {
            HEX[stepped as usize]
        } else {
            alphabet_start + stepped as u8
        };
    }
}

#[derive(Clone, Copy)]
pub enum ClockLayout {
    Header,
    Dashed,
    Compact,
}

pub fn write_clock(target: &mut [u8], utc: i64, layout: ClockLayout) {
    let (year, month, day) = civil(utc.div_euclid(DAY));
    let seconds = utc.rem_euclid(DAY);
    // Where the month, the day, the hour, the minute and the second open;
    // the year opens every layout.
    let places: [usize; 5] = match layout {
        ClockLayout::Header => [5, 8, 11, 14, 17],
        ClockLayout::Dashed => [4, 6, 9, 12, 15],
        ClockLayout::Compact => [4, 6, 8, 10, 12],
    };
    let fields = [
        month,
        day,
        seconds / HOUR,
        seconds % HOUR / 60,
        seconds % 60,
    ];
    write_number(&mut target[0..4], year);
    for (at, value) in places.into_iter().zip(fields) {
        write_number(&mut target[at..at + 2], value);
    }
}

pub fn write_date(target: &mut [u8], days: i64) {
    let (year, month, day) = civil(days);
    write_number(&mut target[0..4], year);
    write_number(&mut target[4..6], month);
    write_number(&mut target[6..8], day);
}

/// `value` in decimal, zero-padded to the width of `target`.
pub fn write_number(target: &mut [u8], mut value: i64) {
    for byte in target.iter_mut().rev() {
        *byte = b'0' + u8::try_from(value % 10).expect("a digit");
        value /= 10;
    }
    assert_eq!(value, 0, "a clock outgrew its width");
}

pub fn parse_digits(digits: &[u8]) -> i64 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + i64::from(digit - b'0'))
}

/// `YYYYMMDD` as days since the epoch, where it is a date of this century.
pub fn date_of(digits: &[u8]) -> Option<i64> {
    let year = parse_digits(&digits[0..4]);
    if !(2000..2100).contains(&year) {
        return None;
    }
    let days = days_from_civil(
        year,
        parse_digits(&digits[4..6]),
        parse_digits(&digits[6..8]),
    )?;
    Some(days)
}

/// `HHMMSS` as its three fields, where it is a time of day.
pub fn time_of(digits: &[u8]) -> Option<(i64, i64, i64)> {
    let (hour, minute, second) = (
        parse_digits(&digits[0..2]),
        parse_digits(&digits[2..4]),
        parse_digits(&digits[4..6]),
    );
    (hour < 24 && minute < 60 && second < 60).then_some((hour, minute, second))
}

/// `-HH:MM:SS` at the start of `rest`, as its three fields.
pub fn dashed_time(rest: &[u8]) -> Option<(i64, i64, i64)> {
    let shape = rest.len() >= 9
        && rest[0] == b'-'
        && rest[3] == b':'
        && rest[6] == b':'
        && [1, 2, 4, 5, 7, 8]
            .iter()
            .all(|&index| rest[index].is_ascii_digit());
    if !shape {
        return None;
    }
    let digits = [rest[1], rest[2], rest[4], rest[5], rest[7], rest[8]];
    time_of(&digits)
}

pub fn epoch(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    Some(days_from_civil(year, month, day)? * DAY + hour * HOUR + minute * 60 + second)
}

/// Days since 1970-01-01 of a proleptic Gregorian date, where it is one.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    // A day past its month's end lands in the next month: not a date.
    (civil(days) == (year, month, day)).then_some(days)
}

/// The proleptic Gregorian date `days` after 1970-01-01.
pub fn civil(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
