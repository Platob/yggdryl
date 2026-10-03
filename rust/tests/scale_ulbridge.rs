//! The streamed capture path at scale: a ULBridge capture of any size read
//! off disk, parsed as FIX, walked through its lifecycle, projected to
//! `marketdata` rows and appended into an Iceberg table, while the process's
//! anonymous resident memory stays where it was a quarter of the way in.
//!
//! The capture is `rust/tests/fix/ulbridge.log` repeated: every copy is the
//! same 144 lines at the same width, each clock moved by the copy and every
//! identifier stepped by it ([`Template`]), so no copy is a repeat of another
//! and every one of them reaches the table. The copies are written through
//! the crate's own Zstandard encoder into a `.log.zst` file and read back
//! through an [`FsFile`] over [`LocalFileSystem`] - plain `std::fs` reads,
//! never a mapping, whose pages would count in the very figure measured - so
//! the coding layer decodes as the read pulls.
//!
//! Memory is read from `/proc/self/status` once per batch on the writer's
//! path and at [`LINE_SAMPLES`] evenly spaced points of the read
//! ([`Probe`]), each reading placed by the lines read so far, which is what
//! a quarter and a half of the run are measured in. `RssAnon` - what the
//! process itself allocated - is the figure asserted; `VmRSS` adds the
//! file-backed pages a spill mapping or a Parquet write leaves cached, which
//! the kernel reclaims, and is reported beside `VmHWM` and `RssFile`. The
//! batch readings are taken while the source is pulled, between commits; a
//! commit's own Parquet encoding is seen by the watchdog's peak, reported
//! beside them. The watchdog also stops the process with a message, and
//! removes what it generated, before `RssAnon` passes [`CEILING`]: the
//! machine this was written on has 15 GB and no swap, and the OOM killer
//! names nothing.
//!
//! Two tests run the one path. [`the_streamed_capture_path_lands_every_row`]
//! is three copies in the ordinary loop, so the path is executed by every
//! `cargo test --all-targets`; [`a_capture_of_any_size_streams_in_constant_memory`]
//! is the scale run, ignored, and a no-op printing `SKIPPED` unless
//! `YGGDRYL_SCALE_BYTES` names the uncompressed size to generate (empty is
//! [`DEFAULT_SCALE_BYTES`]):
//!
//! ```text
//! YGGDRYL_SCALE_BYTES=21474836480 cargo test --release -p yggdryl \
//!     --test scale_ulbridge --features iceberg -- --ignored --nocapture
//! ```
//!
//! `YGGDRYL_SCALE_STAGE` ends the scale run's path early - `lines`, `parse`,
//! `lifecycle`, `market` (the Arrow rows, drained, nothing written) or
//! `write`, the default - so a growth the whole path shows can be put on the
//! stage that owns it. `YGGDRYL_SCALE_FOLDER` is where the input, the table
//! and nothing else are written, the platform temporary folder otherwise;
//! the spill folder is the process default `YGGDRYL_SPILL_FOLDER` states.
//!
//! This target owns its process, for two reasons: the resident set is the
//! process's, and the spill bound every door settles under is the process
//! default ([`SpillOptions::from_env`]), resolved once. The two tests take
//! one lock, so `--include-ignored` never measures one beside the other.

#![cfg(feature = "iceberg")]

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};
use regex::bytes::Regex;
use yggdryl::arrow::BatchReader;
use yggdryl::fix::FixMarketIterator;
use yggdryl::fs::{FsFile, LocalFileSystem};
use yggdryl::graph::MarketData;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
use yggdryl::local::LocalFolder;
use yggdryl::media::IORecordOptions;
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{
    ArrowCastOptions, FixCodec, FixRegistry, IOMedia, Level, Scheme, SerieReader, SerieSource,
    SpillOptions, Timezone,
};

/// The capture every copy repeats, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("fix/ulbridge.log");

/// The lines one copy holds.
const LINES_PER_COPY: u64 = 144;

/// The messages one copy parses into under the codec's defaults: the 79 the
/// capture carries past the refused session traffic and the 57 executions
/// their parse splits off - `rust/tests/fix/ulbridge.rs` pins the same.
const MESSAGES_PER_COPY: u64 = 79 + 57;

/// The market operations one copy projects to once walked - the 21 the
/// capture's own walk reaches a book with, which `rust/tests/fix/ulbridge.rs`
/// pins: the NOVN order's acknowledgement, restatement and expiry among them,
/// since an execution report of no fill is its order's leaf. A copy the walk
/// took for a repeat of another would answer fewer.
const OPERATIONS_PER_COPY: u64 = 21;

/// The uncompressed size `YGGDRYL_SCALE_BYTES` stands for when it is set
/// and empty: twenty gibibytes, about ninety-nine thousand copies.
const DEFAULT_SCALE_BYTES: u64 = 20 << 30;

/// The copies the ordinary loop runs: enough for a lifecycle hour to close
/// behind the next and for the writer to commit more than once.
const SMOKE_COPIES: u64 = 3;

/// The `RssAnon` the watchdog stops the process at: about half the 15 GB
/// the machine holds, with no swap behind it.
const CEILING: u64 = 8 << 30;

/// What the last half of a run may hold over the first quarter's peak,
/// beside half as much again: one commit cadence's Parquet encoding (the
/// column chunks of one data file, compressed in memory before they are
/// written) and the allocator's per-thread arenas, which keep what a busier
/// moment asked of them. Neither grows with the input; a structure that does
/// passes this within a few gibibytes of input.
const SLACK: u64 = 512 << 20;

/// How often the watchdog reads the resident set.
const WATCH_EVERY: Duration = Duration::from_millis(100);

/// The rows one `marketdata` batch holds at most at scale: about 12 MiB of
/// Arrow at the three kilobytes a row of this capture occupies.
const BATCH_ROWS: usize = 4_096;

/// The Arrow bytes one `marketdata` batch holds at most.
const BATCH_BYTES: u64 = 16 << 20;

/// The most batches one commit holds: about 768 MiB of rows at 12 MiB a
/// batch, held under the process spill bound until they land, so the spill
/// folder never holds more than one cadence - under a gibibyte.
const MAX_COMMIT_BATCHES: usize = 64;

/// The commits a scale run aims for, so its first quarter already holds
/// several and the comparison with its last half sets commit against commit.
const COMMITS: usize = 16;

/// How many readings of the resident set a run takes as it reads its lines,
/// beside the one after every batch the writer pulls.
const LINE_SAMPLES: u64 = 1_024;

/// One hour and one day, in seconds.
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// The bridge writes Zurich's local clock in front of every line, and the
/// whole capture falls in Zurich summer time: two hours ahead of UTC.
const ZURICH_SUMMER: i64 = 2 * HOUR;

/// The tests' one lock: a resident set is the whole process's.
static PROCESS: Mutex<()> = Mutex::new(());

// --- the generator ----------------------------------------------------------

/// The fields whose values are identifiers, as the capture spells them: FIX
/// tags, the bridge's own names and the FIXML attributes. Every occurrence
/// of a value collected here is stepped, wherever it stands - a FIX frame, a
/// bridge row, the prose in front of one, a document inside another.
const ID_KEYS: &[&str] = &[
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
enum Kind {
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
struct Patch {
    at: usize,
    kind: Kind,
}

/// One line of the capture and every place a copy rewrites in it.
struct Line {
    bytes: Vec<u8>,
    /// The line's own clock, in UTC seconds.
    header_utc: i64,
    /// What copy zero moves the line's clocks by: its hour packed beside
    /// the capture's other hours.
    slot_shift: i64,
    patches: Vec<Patch>,
}

/// The capture as a template copies are rendered from.
///
/// The bridge's capture is four bursts on one day - 01:03, 12:46, 14:52 and
/// 21:59 UTC - logged out of order. A copy packs each burst into its own
/// hour, in order, and its lines are emitted in clock order, so a copy spans
/// [`Self::span`] - four hours - and copy `k` is copy zero moved by `k`
/// spans: six copies a day, a twenty-gibibyte run some forty-five years,
/// well inside the nanosecond clock's range. Every clock of a line moves with
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
struct Template {
    lines: Vec<Line>,
    span: i64,
}

impl Template {
    fn new() -> Self {
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
        }
    }

    /// Copy `copy` appended to `out`.
    fn render(&self, copy: u64, out: &mut Vec<u8>) {
        let moved = i64::try_from(copy).expect("a copy count") * self.span;
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
fn identifiers() -> Vec<Vec<u8>> {
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
fn identifier_patches(line: &[u8], identifiers: &[Vec<u8>], patches: &mut Vec<Patch>) {
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
fn clock_patches(line: &[u8], patches: &mut Vec<Patch>) {
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

fn overlaps(patch: &Patch, at: usize, len: usize) -> bool {
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
fn is_uuid(value: &[u8]) -> bool {
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
fn step_identifier(value: &mut [u8], copy: u64, hex: bool) {
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
enum ClockLayout {
    Header,
    Dashed,
    Compact,
}

fn write_clock(target: &mut [u8], utc: i64, layout: ClockLayout) {
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

fn write_date(target: &mut [u8], days: i64) {
    let (year, month, day) = civil(days);
    write_number(&mut target[0..4], year);
    write_number(&mut target[4..6], month);
    write_number(&mut target[6..8], day);
}

/// `value` in decimal, zero-padded to the width of `target`.
fn write_number(target: &mut [u8], mut value: i64) {
    for byte in target.iter_mut().rev() {
        *byte = b'0' + u8::try_from(value % 10).expect("a digit");
        value /= 10;
    }
    assert_eq!(value, 0, "a clock outgrew its width");
}

fn parse_digits(digits: &[u8]) -> i64 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + i64::from(digit - b'0'))
}

/// `YYYYMMDD` as days since the epoch, where it is a date of this century.
fn date_of(digits: &[u8]) -> Option<i64> {
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
fn time_of(digits: &[u8]) -> Option<(i64, i64, i64)> {
    let (hour, minute, second) = (
        parse_digits(&digits[0..2]),
        parse_digits(&digits[2..4]),
        parse_digits(&digits[4..6]),
    );
    (hour < 24 && minute < 60 && second < 60).then_some((hour, minute, second))
}

/// `-HH:MM:SS` at the start of `rest`, as its three fields.
fn dashed_time(rest: &[u8]) -> Option<(i64, i64, i64)> {
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

fn epoch(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    Some(days_from_civil(year, month, day)? * DAY + hour * HOUR + minute * 60 + second)
}

/// Days since 1970-01-01 of a proleptic Gregorian date, where it is one.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
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
fn civil(days: i64) -> (i64, i64, i64) {
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

/// `copies` copies of the capture, Zstandard-encoded into `path` as they are
/// rendered: one copy in memory at a time. Answers the uncompressed bytes.
fn generate(template: &Template, copies: u64, path: &Path) -> u64 {
    let file = File::create(path).expect("the input file creates");
    let mut encoder = yggdryl::zstd::writer_with_level(file, Level::FAST);
    let mut copy = Vec::with_capacity(LOG.len());
    let mut written = 0_u64;
    for index in 0..copies {
        copy.clear();
        template.render(index, &mut copy);
        encoder.write_all(&copy).expect("a copy encodes");
        written += copy.len() as u64;
    }
    encoder.finish().expect("the frame closes");
    written
}

// --- memory -----------------------------------------------------------------

/// The four figures `/proc/self/status` states for the process, in bytes.
#[derive(Clone, Copy, Debug, Default)]
struct Resident {
    vm_rss: u64,
    vm_hwm: u64,
    rss_anon: u64,
    rss_file: u64,
}

impl Resident {
    /// The process's figures now, where the platform states them.
    fn now() -> Option<Self> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut resident = Self::default();
        for line in status.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let target = match name {
                "VmRSS" => &mut resident.vm_rss,
                "VmHWM" => &mut resident.vm_hwm,
                "RssAnon" => &mut resident.rss_anon,
                "RssFile" => &mut resident.rss_file,
                _ => continue,
            };
            let kib: u64 = value.trim().trim_end_matches("kB").trim().parse().ok()?;
            *target = kib * 1024;
        }
        Some(resident)
    }
}

/// One reading of the resident set, placed by how far the run had read.
#[derive(Clone, Copy, Debug)]
struct Sample {
    lines: u64,
    rows: u64,
    resident: Resident,
}

/// How far each stage of the path has read, and the samples taken on it.
#[derive(Default)]
struct Probe {
    /// The lines read between two samples the reading takes.
    every: u64,
    lines: AtomicU64,
    messages: AtomicU64,
    walked: AtomicU64,
    operations: AtomicU64,
    rows: AtomicU64,
    batches: AtomicU64,
    /// What the batches occupy, as the commit cadence counts them.
    arrow_bytes: AtomicU64,
    samples: Mutex<Vec<Sample>>,
}

impl Probe {
    fn sample(&self) {
        if let Some(resident) = Resident::now() {
            let sample = Sample {
                lines: self.lines.load(Ordering::Relaxed),
                rows: self.rows.load(Ordering::Relaxed),
                resident,
            };
            self.samples.lock().expect("the samples").push(sample);
        }
    }
}

/// The `marketdata` batches as the writer pulls them, each counted and the
/// resident set read after it.
struct Sampled {
    inner: BatchReader,
    probe: Arc<Probe>,
}

impl Iterator for Sampled {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.next();
        if let Some(Ok(batch)) = &next {
            self.probe
                .rows
                .fetch_add(batch.num_rows() as u64, Ordering::Relaxed);
            self.probe.batches.fetch_add(1, Ordering::Relaxed);
            self.probe
                .arrow_bytes
                .fetch_add(yggdryl::arrow::memory_size(batch) as u64, Ordering::Relaxed);
            self.probe.sample();
        }
        next
    }
}

impl RecordBatchReader for Sampled {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }
}

/// A thread reading the resident set every [`WATCH_EVERY`], stopping the
/// process - after removing `scratch` - before `RssAnon` passes
/// [`CEILING`].
struct Watchdog {
    stop: Arc<AtomicBool>,
    peak_anon: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    fn start(scratch: PathBuf, probe: Arc<Probe>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak_anon = Arc::new(AtomicU64::new(0));
        let thread = {
            let stop = Arc::clone(&stop);
            let peak_anon = Arc::clone(&peak_anon);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Some(resident) = Resident::now() {
                        peak_anon.fetch_max(resident.rss_anon, Ordering::Relaxed);
                        if resident.rss_anon > CEILING {
                            eprintln!(
                                "ABORTED: RssAnon {} MiB passed the {} MiB ceiling after {} lines \
                                 and {} rows; the scale run stops its own process before the OOM \
                                 killer does (VmRSS {} MiB, RssFile {} MiB)",
                                mib(resident.rss_anon),
                                mib(CEILING),
                                probe.lines.load(Ordering::Relaxed),
                                probe.rows.load(Ordering::Relaxed),
                                mib(resident.vm_rss),
                                mib(resident.rss_file),
                            );
                            let _ = std::fs::remove_dir_all(&scratch);
                            std::process::exit(3);
                        }
                    }
                    std::thread::sleep(WATCH_EVERY);
                }
            })
        };
        Self {
            stop,
            peak_anon,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the watchdog ends");
        }
        self.peak_anon.load(Ordering::Relaxed)
    }
}

fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

// --- the run ----------------------------------------------------------------

/// Where the path ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    /// The lines, read and framed.
    Lines,
    /// The messages they parse into.
    Parse,
    /// The messages walked through their lifecycle.
    Lifecycle,
    /// The walked messages as `marketdata` batches, drained.
    Market,
    /// The batches appended into the table.
    Write,
}

impl Stage {
    fn from_env() -> Self {
        match std::env::var("YGGDRYL_SCALE_STAGE").as_deref() {
            Err(_) | Ok("" | "write") => Self::Write,
            Ok("lines") => Self::Lines,
            Ok("parse") => Self::Parse,
            Ok("lifecycle") => Self::Lifecycle,
            Ok("market") => Self::Market,
            Ok(other) => panic!(
                "expected YGGDRYL_SCALE_STAGE to be lines, parse, lifecycle, market or write, got {other:?}"
            ),
        }
    }
}

/// One run's shape.
struct Run {
    copies: u64,
    stage: Stage,
    batch_rows: usize,
    batch_bytes: u64,
    commit_batches: usize,
    /// Whether a watchdog guards the run.
    watched: bool,
}

/// What one run read, wrote and held.
struct Outcome {
    input_bytes: u64,
    compressed_bytes: u64,
    lines: u64,
    messages: u64,
    walked: u64,
    operations: u64,
    rows: u64,
    batches: u64,
    arrow_bytes: u64,
    table_rows: Option<u64>,
    snapshots: Option<usize>,
    table_bytes: u64,
    baseline: Option<Resident>,
    finale: Option<Resident>,
    samples: Vec<Sample>,
    watched_peak: Option<u64>,
    generated_in: Duration,
    streamed_in: Duration,
}

/// A folder the run owns, removed when the run ends however it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn registry() -> Arc<FixRegistry> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = LocalFolder::new(root).expect("the local dictionary path");
    Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
}

/// The options a bridge log is read under, as `yggdryl market serve` reads
/// one: the bridge's row header, each line numbered and classified, and its
/// clock - which the generator writes in UTC - read in UTC.
fn reading() -> TextOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options
}

fn run(shape: &Run) -> Outcome {
    let parent = std::env::var_os("YGGDRYL_SCALE_FOLDER").map_or_else(
        || {
            LocalFolder::temporary()
                .expect("a temporary folder")
                .path()
                .expect("a local path")
        },
        PathBuf::from,
    );
    let scratch = Scratch(parent.join(format!("yggdryl-scale-ulbridge-{}", std::process::id())));
    std::fs::create_dir_all(&scratch.0).expect("the scratch folder creates");
    let input = scratch.0.join("ulbridge.log.zst");
    let table_root = scratch.0.join("table");

    let registry = registry();
    let spill = SpillOptions::from_env().expect("the process spill default");
    println!(
        "scale_ulbridge: {} copies, stage {:?}, spill bound {} bytes in {}",
        shape.copies,
        shape.stage,
        spill.byte_size(),
        spill.folder().map_or_else(
            || "the platform temporary folder".to_owned(),
            |folder| format!("{:?}", folder.path().ok()),
        ),
    );

    let started = Instant::now();
    let template = Template::new();
    let input_bytes = generate(&template, shape.copies, &input);
    let generated_in = started.elapsed();
    let compressed_bytes = std::fs::metadata(&input).expect("the input").len();
    drop(template);

    let options = reading();
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_capture_names(options.capture_names())
        .with_sorted_lifecycle(true);
    let probe = Arc::new(Probe {
        every: (shape.copies * LINES_PER_COPY / LINE_SAMPLES).max(1),
        ..Probe::default()
    });
    let watchdog = shape
        .watched
        .then(|| Watchdog::start(scratch.0.clone(), Arc::clone(&probe)));
    let baseline = Resident::now();

    let started = Instant::now();
    let file = FsFile::from_path(
        Arc::new(LocalFileSystem::new()),
        input.to_str().expect("a UTF-8 path"),
        None,
    )
    .expect("the input binds");
    let lines = {
        let probe = Arc::clone(&probe);
        read_text_lines(&file, &options)
            .expect("the line reader")
            .inspect(move |_| {
                let read = probe.lines.fetch_add(1, Ordering::Relaxed) + 1;
                if read.is_multiple_of(probe.every) {
                    probe.sample();
                }
            })
    };
    let mut table_rows = None;
    let mut snapshots = None;
    if shape.stage == Stage::Lines {
        lines.for_each(drop);
    } else {
        let messages = {
            let probe = Arc::clone(&probe);
            codec.parse_text_lines(lines).inspect(move |_| {
                probe.messages.fetch_add(1, Ordering::Relaxed);
            })
        };
        if shape.stage == Stage::Parse {
            messages.for_each(drop);
        } else {
            let walked = {
                let probe = Arc::clone(&probe);
                codec.lifecycle(messages).inspect(move |_| {
                    probe.walked.fetch_add(1, Ordering::Relaxed);
                })
            };
            if shape.stage == Stage::Lifecycle {
                walked.for_each(drop);
            } else {
                let operations = {
                    let probe = Arc::clone(&probe);
                    FixMarketIterator::new(walked).inspect(move |operation| {
                        if operation.is_ok() {
                            probe.operations.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                };
                let batches = MarketData::arrow_reader(
                    operations,
                    Some(shape.batch_rows),
                    Some(shape.batch_bytes),
                )
                .expect("the marketdata reader");
                let batches: BatchReader = Box::new(Sampled {
                    inner: batches,
                    probe: Arc::clone(&probe),
                });
                if shape.stage == Stage::Market {
                    for batch in batches {
                        batch.expect("a marketdata batch");
                    }
                } else {
                    let row = MarketData::field().expect("the marketdata row");
                    let mut table = Table::create(
                        LocalFolder::new(table_root.clone()).expect("the table folder"),
                        FormatVersion::V3,
                        row.clone()
                            .into_scheme_compat(&Scheme::ICEBERG)
                            .expect("the row as Iceberg states it"),
                        PartitionSpec::unpartitioned(),
                    )
                    .expect("the table creates");
                    let reader = SerieReader::from_arrow_reader(
                        Some(&row),
                        batches,
                        ArrowCastOptions::new(),
                    )
                    .expect("the marketdata stream");
                    let mut writing = table.record_options().expect("the table's options");
                    writing.set_commit_batch_num(Some(shape.commit_batches));
                    table
                        .append_serie(SerieSource::from(reader), Some(&writing))
                        .expect("the rows append");
                    let reopened = Table::open(
                        LocalFolder::new(table_root.clone()).expect("the table folder"),
                    )
                    .expect("the table reopens");
                    table_rows = Some(reopened.row_size().expect("the table's row count"));
                    snapshots = Some(
                        reopened
                            .inspect_snapshots()
                            .expect("the snapshots table")
                            .map(|batch| batch.expect("a snapshots batch").num_rows())
                            .sum(),
                    );
                }
            }
        }
    }
    let streamed_in = started.elapsed();
    let finale = Resident::now();
    let watched_peak = watchdog.map(Watchdog::finish);
    let table_bytes = folder_bytes(&table_root);
    let samples = probe.samples.lock().expect("the samples").clone();
    Outcome {
        input_bytes,
        compressed_bytes,
        lines: probe.lines.load(Ordering::Relaxed),
        messages: probe.messages.load(Ordering::Relaxed),
        walked: probe.walked.load(Ordering::Relaxed),
        operations: probe.operations.load(Ordering::Relaxed),
        rows: probe.rows.load(Ordering::Relaxed),
        batches: probe.batches.load(Ordering::Relaxed),
        arrow_bytes: probe.arrow_bytes.load(Ordering::Relaxed),
        table_rows,
        snapshots,
        table_bytes,
        baseline,
        finale,
        samples,
        watched_peak,
        generated_in,
        streamed_in,
    }
}

/// The bytes every file under `root` holds.
fn folder_bytes(root: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_bytes(&entry.path()),
            Ok(_) => entry.metadata().map_or(0, |metadata| metadata.len()),
            Err(_) => 0,
        })
        .sum()
}

/// The highest `RssAnon` among the samples taken while the run had read
/// between `from` and `to` of its lines.
fn peak(samples: &[Sample], from: u64, to: u64) -> Option<u64> {
    samples
        .iter()
        .filter(|sample| (from..=to).contains(&sample.lines))
        .map(|sample| sample.resident.rss_anon)
        .max()
}

/// The run's figures, printed for the record.
fn report(outcome: &Outcome) {
    let lines = outcome.lines.max(1);
    println!(
        "scale_ulbridge: generated {} MiB ({} MiB compressed) in {:.1?}; streamed in {:.1?}",
        mib(outcome.input_bytes),
        mib(outcome.compressed_bytes),
        outcome.generated_in,
        outcome.streamed_in,
    );
    println!(
        "scale_ulbridge: {} lines, {} messages, {} walked, {} operations, {} rows in {} batches \
         ({} MiB of Arrow, {} bytes a row), table rows {:?} in {:?} snapshots, {} MiB on disk",
        outcome.lines,
        outcome.messages,
        outcome.walked,
        outcome.operations,
        outcome.rows,
        outcome.batches,
        mib(outcome.arrow_bytes),
        outcome.arrow_bytes / outcome.rows.max(1),
        outcome.table_rows,
        outcome.snapshots,
        mib(outcome.table_bytes),
    );
    let figures = |resident: Option<Resident>| {
        resident.map_or_else(
            || "unstated".to_owned(),
            |resident| {
                format!(
                    "RssAnon {} MiB, VmRSS {} MiB, VmHWM {} MiB, RssFile {} MiB",
                    mib(resident.rss_anon),
                    mib(resident.vm_rss),
                    mib(resident.vm_hwm),
                    mib(resident.rss_file),
                )
            },
        )
    };
    println!(
        "scale_ulbridge: before the stream {}",
        figures(outcome.baseline)
    );
    println!(
        "scale_ulbridge: after the stream  {}",
        figures(outcome.finale)
    );
    if let Some(peak) = outcome.watched_peak {
        println!("scale_ulbridge: watchdog peak RssAnon {} MiB", mib(peak));
    }
    let quarter = peak(&outcome.samples, 0, lines / 4);
    let last_half = peak(&outcome.samples, lines / 2, u64::MAX);
    println!(
        "scale_ulbridge: {} samples; peak RssAnon over the first quarter {:?} MiB, over the last \
         half {:?} MiB",
        outcome.samples.len(),
        quarter.map(mib),
        last_half.map(mib),
    );
    // The trace, at most twenty readings evenly placed, and the growth per
    // gibibyte read between the first quarter and the end.
    let step = (outcome.samples.len() / 20).max(1);
    for sample in outcome.samples.iter().step_by(step) {
        println!(
            "scale_ulbridge:   {:>5.1}% read, {:>12} rows: RssAnon {:>6} MiB, VmRSS {:>6} MiB, \
             RssFile {:>6} MiB",
            sample.lines as f64 * 100.0 / lines as f64,
            sample.rows,
            mib(sample.resident.rss_anon),
            mib(sample.resident.vm_rss),
            mib(sample.resident.rss_file),
        );
    }
    if let (Some(first), Some(last)) = (
        outcome
            .samples
            .iter()
            .find(|sample| sample.lines >= lines / 4),
        outcome.samples.last(),
    ) && last.lines > first.lines
    {
        let read = (last.lines - first.lines) as f64 / lines as f64 * outcome.input_bytes as f64;
        let grown = last.resident.rss_anon as f64 - first.resident.rss_anon as f64;
        println!(
            "scale_ulbridge: RssAnon moved {:+.1} MiB per GiB read after the first quarter",
            grown / read * 1024.0,
        );
    }
}

/// The scale run's size, or `None` where `YGGDRYL_SCALE_BYTES` is absent.
fn scale_bytes() -> Option<u64> {
    let value = std::env::var("YGGDRYL_SCALE_BYTES").ok()?;
    let value = value.trim();
    if value.is_empty() {
        return Some(DEFAULT_SCALE_BYTES);
    }
    Some(value.parse().unwrap_or_else(|error| {
        panic!("expected YGGDRYL_SCALE_BYTES to be a byte count, got {value:?}: {error}")
    }))
}

#[test]
fn the_generator_writes_fresh_copies_of_one_width() {
    let template = Template::new();
    let mut first = Vec::new();
    template.render(0, &mut first);
    let mut second = Vec::new();
    template.render(1, &mut second);
    let mut far = Vec::new();
    template.render(99_999, &mut far);
    assert_eq!(first.len(), LOG.len());
    assert_eq!(second.len(), LOG.len());
    assert_eq!(far.len(), LOG.len());
    // Copy zero is the capture's own lines, in clock order: the same bytes,
    // differently placed.
    let mut sorted_log: Vec<&[u8]> = LOG.split_inclusive(|byte| *byte == b'\n').collect();
    let mut sorted_first: Vec<&[u8]> = first.split_inclusive(|byte| *byte == b'\n').collect();
    sorted_log.sort_unstable();
    sorted_first.sort_unstable();
    assert_ne!(
        sorted_log, sorted_first,
        "copy zero writes its clocks in UTC"
    );
    let header = |line: &[u8]| line[..23].to_vec();
    for copy in [&first, &second, &far] {
        let clocks: Vec<Vec<u8>> = copy
            .split_inclusive(|byte| *byte == b'\n')
            .map(header)
            .collect();
        assert!(
            clocks.windows(2).all(|pair| pair[0] <= pair[1]),
            "a copy's lines are in clock order"
        );
    }
    let last_first = header(
        first
            .split_inclusive(|byte| *byte == b'\n')
            .next_back()
            .unwrap(),
    );
    let first_second = header(
        second
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap(),
    );
    assert!(last_first < first_second, "copy one follows copy zero");
    assert_eq!(&first[..23], b"2026-08-14 01:03:13.314");
    assert_eq!(&second[..23], b"2026-08-14 05:03:13.314");
    // The 99,999th copy is 45 years on, well inside the nanosecond clock.
    assert_eq!(&far[..23], b"2072-03-31 13:03:13.314");
    // No identifier the capture states survives into copy one as a token of
    // its own; one only spelled inside another's value is that value's.
    for value in identifiers() {
        let found = |haystack: &[u8]| {
            haystack
                .windows(value.len())
                .enumerate()
                .any(|(at, window)| {
                    let end = at + value.len();
                    window == value.as_slice()
                        && (at == 0 || !haystack[at - 1].is_ascii_alphanumeric())
                        && (end == haystack.len() || !haystack[end].is_ascii_alphanumeric())
                })
        };
        assert!(
            found(&first),
            "{} is in copy zero",
            String::from_utf8_lossy(&value)
        );
        assert!(
            !found(&second),
            "{} is in copy one",
            String::from_utf8_lossy(&value)
        );
    }
    // Instruments are not identifiers: the ISINs stand in every copy.
    for isin in [&b"CH0012214059"[..], b"TW0002454006", b"CH0012005267"] {
        assert!(far.windows(isin.len()).any(|window| window == isin));
    }
}

#[test]
fn the_streamed_capture_path_lands_every_row() {
    let _process = PROCESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let outcome = run(&Run {
        copies: SMOKE_COPIES,
        stage: Stage::Write,
        // Small batches and a short cadence, so three copies cross several
        // batches and several commits.
        batch_rows: 8,
        batch_bytes: BATCH_BYTES,
        commit_batches: 2,
        watched: false,
    });
    report(&outcome);
    assert_eq!(outcome.lines, SMOKE_COPIES * LINES_PER_COPY);
    assert_eq!(outcome.messages, SMOKE_COPIES * MESSAGES_PER_COPY);
    // Every copy reaches the books the capture reaches: none is a repeat.
    assert_eq!(outcome.operations, SMOKE_COPIES * OPERATIONS_PER_COPY);
    assert_eq!(outcome.rows, outcome.operations, "one row per market leaf");
    assert_eq!(
        outcome.table_rows,
        Some(outcome.rows),
        "the table holds every row the projection produced"
    );
    assert!(
        outcome.snapshots.is_some_and(|snapshots| snapshots >= 2),
        "the cadence committed more than once: {:?}",
        outcome.snapshots
    );
}

#[test]
#[ignore = "generates and streams YGGDRYL_SCALE_BYTES of capture (20 GiB when empty); a release run"]
fn a_capture_of_any_size_streams_in_constant_memory() {
    let _process = PROCESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(bytes) = scale_bytes() else {
        println!("SKIPPED: YGGDRYL_SCALE_BYTES is not set; set it to the input size to generate");
        return;
    };
    let copies = bytes.div_ceil(LOG.len() as u64).max(4);
    let batches = usize::try_from((copies * OPERATIONS_PER_COPY).div_ceil(BATCH_ROWS as u64))
        .expect("a batch count");
    let outcome = run(&Run {
        copies,
        stage: Stage::from_env(),
        batch_rows: BATCH_ROWS,
        batch_bytes: BATCH_BYTES,
        commit_batches: (batches / COMMITS).clamp(1, MAX_COMMIT_BATCHES),
        watched: true,
    });
    report(&outcome);
    assert_eq!(outcome.lines, copies * LINES_PER_COPY);
    if outcome.table_rows.is_some() {
        assert_eq!(outcome.operations, copies * OPERATIONS_PER_COPY);
        assert_eq!(outcome.rows, outcome.operations, "one row per market leaf");
        assert_eq!(
            outcome.table_rows,
            Some(outcome.rows),
            "the table holds every row the projection produced"
        );
    }
    let lines = outcome.lines;
    let quarter = peak(&outcome.samples, 0, lines / 4)
        .expect("the platform states RssAnon, and the first quarter was sampled");
    let last_half = peak(&outcome.samples, lines / 2, u64::MAX).expect("the last half was sampled");
    let allowed = (quarter + quarter / 2).max(quarter + SLACK);
    assert!(
        last_half <= allowed,
        "RssAnon grew with the input: the last half peaked at {} MiB, over the {} MiB the first \
         quarter's {} MiB peak allows",
        mib(last_half),
        mib(allowed),
        mib(quarter),
    );
}
