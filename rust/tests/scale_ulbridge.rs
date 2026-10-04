//! The capture pipeline at scale, end to end on series: bridge logs read as
//! one stream of text rows, appended into an Iceberg table, read back in
//! order, parsed as FIX and walked through their lifecycle into a second
//! table, and that table read back in order into books - a snapshot of every
//! book each quarter of an hour, and the deltas between them flattened to
//! `marketdata` rows - each in a table of its own.
//!
//! ```text
//! logs/ --read_serie--> text rows --append_serie--> text table
//! text table --read_serie (sorted)--> parse_text_serie --> lifecycle_serie --overwrite_serie--> fix table
//! fix table --read_serie (sorted)--> messages_serie --> lifecycle --> books every 15 minutes
//!     complete books --overwrite_serie--> books table
//!     their deltas   --overwrite_serie--> deltas table
//! ```
//!
//! Every table is created from the schema of the stream written to it, as
//! Iceberg states it, partitioned by `partunix` - `currunix` floored to the
//! quarter hour, a column the table computes for every row it is written -
//! and sorted by `partunix, currunix, seqnum, currhashcode`, with
//! `currunix, currhashcode` - the instant and the content hash - declared
//! its primary key, Iceberg's identifier fields. A read yields
//! partition after partition in that order, so the lifecycle and the books
//! take rows in the order they happened whatever order they were written
//! in, and an overwrite replaces the partitions its rows fall in and no
//! other: running a stage again rewrites what it wrote, and running it over
//! one window of its input rewrites that window.
//!
//! The capture is `rust/tests/fix/ulbridge.log` repeated: every copy is the
//! same 144 lines at the same width, each clock moved by the copy and every
//! identifier stepped by it ([`Template`]), so no copy is a repeat of another
//! and every one of them reaches the tables. The scale run stacks
//! [`DENSITY`] copies a second apart inside each four hours the capture
//! spans, so twenty gibibytes are eight days of quarters holding tens of
//! thousands of rows each, as a bridge's own days are, rather than decades
//! of quarters holding a dozen. The copies are written through
//! the crate's own Zstandard encoder into `.log.zst` files, a run of copies
//! each, and read back as the one table their folder is through the
//! crate's own [`LocalFolder`], the backend a local path resolves to: its
//! leaves are memory-mapped, and a mapping's pages are the file's, counted
//! in `RssFile` and never in the `RssAnon` asserted.
//!
//! Memory is read from `/proc/self/status` after every batch a stage pulls
//! ([`Probe`]), each reading placed by the text rows read so far, which is
//! what a quarter and a half of the run are measured in. `RssAnon` - what
//! the process itself allocated - is the figure asserted where the platform
//! states it; `VmRSS` adds the file-backed pages a spill mapping or a Parquet
//! write leaves cached, which the kernel reclaims, and is reported beside
//! `VmHWM` and `RssFile`. The watchdog also stops the process with a
//! message, and removes what it generated, before `RssAnon` passes
//! [`CEILING`]: the machine this was written on has 15 GB and no swap, and
//! the OOM killer names nothing.
//!
//! Two tests run the one path. [`the_capture_pipeline_lands_every_stage_in_order`]
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
//! `YGGDRYL_SCALE_STAGE` ends the scale run's path early - `lines` (the text
//! rows, drained), `text` (the text table), `parse`, `lifecycle` (each
//! drained), `fix` (the FIX table) or `books`, the default - so a growth the
//! whole path shows can be put on the stage that owns it.
//! `YGGDRYL_SCALE_FOLDER` is where the input, the tables and nothing else
//! are written, the platform temporary folder otherwise; the spill folder is
//! the process default `YGGDRYL_SPILL_FOLDER` states. `YGGDRYL_SCALE_DENSITY`
//! is the copies stacked inside one span, [`DENSITY`] where it is absent:
//! `1` is one copy a span, the sparsest table a run can ask for.
//!
//! This target owns its process, for two reasons: the resident set is the
//! process's, and the spill bound every door settles under is the process
//! default ([`SpillOptions::from_env`]), resolved once. The tests take one
//! lock, so `--include-ignored` never measures one beside the other.

#![cfg(feature = "iceberg")]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow_array::{
    Array, Decimal128Array, RecordBatch, RecordBatchReader, TimestampNanosecondArray,
};
use arrow_schema::{ArrowError, SchemaRef};
use regex::bytes::Regex;
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{BookIterator, MarketData};
use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
use yggdryl::local::LocalFolder;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{
    ArrowCastOptions, DataType, Field, FixCodec, FixMsg, FixRegistry, IOMedia, IOResult, Level,
    Scheme, SerieReader, SerieSource, SpillOptions, Timezone,
};

/// The capture every copy repeats, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("fix/ulbridge.log");

/// The lines one copy holds.
const LINES_PER_COPY: u64 = 144;

/// The messages one copy parses into under the codec's defaults: the 79 the
/// capture carries past the refused session traffic and the 57 executions
/// their parse splits off - `rust/tests/fix/ulbridge.rs` pins the same.
const MESSAGES_PER_COPY: u64 = 79 + 57;

/// The uncompressed size `YGGDRYL_SCALE_BYTES` stands for when it is set
/// and empty: twenty gibibytes, about ninety-nine thousand copies.
const DEFAULT_SCALE_BYTES: u64 = 20 << 30;

/// The copies the ordinary loop runs: enough for a lifecycle hour to close
/// behind the next and for the writers to commit more than once.
const SMOKE_COPIES: u64 = 3;

/// The primary key every table of the pipeline declares: when a row
/// happened and the hash of what it states.
const PRIMARY_KEY: [&str; 2] = ["currunix", "currhashcode"];

/// The columns every table of the pipeline requires of each row: its key,
/// its place among the rows of its instant, and the code and hash of the
/// chain it belongs to. A text row and a FIX row already do; the
/// `marketdata` row lets a leaf that is no event state none, and these
/// tables hold events alone.
const REQUIRED: [&str; 5] = [
    "currunix",
    "currhashcode",
    "seqnum",
    "crosscode",
    "crosshashcode",
];

/// The copies the scale run stacks inside one span, each a second after the
/// one before: some twenty-four thousand text rows a quarter of an hour.
const DENSITY: u64 = 2_048;

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

/// The rows one batch of any stage holds at most at scale.
const BATCH_ROWS: usize = 4_096;

/// The Arrow bytes one `marketdata` batch holds at most.
const BATCH_BYTES: u64 = 16 << 20;

/// The most batches one commit holds: held under the process spill bound
/// until they land, so the spill folder never holds more than one cadence.
const MAX_COMMIT_BATCHES: usize = 64;

/// The commits a scale run aims for in each table, so its first quarter
/// already holds several and the comparison with its last half sets commit
/// against commit.
const COMMITS: usize = 16;

/// The copies one input file holds at scale: about a hundred megabytes of
/// capture a file, so the folder the text stage reads is many leaves.
const COPIES_PER_FILE: u64 = 512;

/// One hour and one day, in seconds.
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// The quarter of an hour every table partitions by, in nanoseconds, and
/// the grid the books snapshot on, in milliseconds.
const QUARTER_NS: i64 = 900 * 1_000_000_000;
const QUARTER_MS: u64 = 900 * 1_000;

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
struct Template {
    lines: Vec<Line>,
    span: i64,
    /// The copies one span holds.
    density: u64,
}

impl Template {
    fn new(density: u64) -> Self {
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
    fn render(&self, copy: u64, out: &mut Vec<u8>) {
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

/// Copies `copies` of the capture, Zstandard-encoded into `path` as they
/// are rendered: one copy in memory at a time. Answers the uncompressed
/// bytes.
fn generate(template: &Template, copies: std::ops::Range<u64>, path: &Path) -> u64 {
    let file = File::create(path).expect("the input file creates");
    let mut encoder = yggdryl::zstd::writer_with_level(file, Level::FAST);
    let mut copy = Vec::with_capacity(LOG.len());
    let mut written = 0_u64;
    for index in copies {
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
    /// The text rows read off the input files.
    lines: AtomicU64,
    /// The FIX rows the text rows parsed into.
    messages: AtomicU64,
    /// The FIX rows the lifecycle walked.
    walked: AtomicU64,
    /// The complete books and the delta-only ones the fold answered.
    books: AtomicU64,
    /// The orders and quotes the books' deltas flatten to.
    deltas: AtomicU64,
    /// Every batch any stage pulled, and what they occupy.
    batches: AtomicU64,
    arrow_bytes: AtomicU64,
    samples: Mutex<Vec<Sample>>,
}

impl Probe {
    fn sample(&self) {
        if let Some(resident) = Resident::now() {
            let sample = Sample {
                lines: self.lines.load(Ordering::Relaxed),
                rows: self.walked.load(Ordering::Relaxed),
                resident,
            };
            self.samples.lock().expect("the samples").push(sample);
        }
    }
}

/// One stage's batches as the next stage pulls them: their rows counted
/// under the stage's own counter and the resident set read after each.
struct Counted {
    inner: BatchReader,
    probe: Arc<Probe>,
    counter: fn(&Probe) -> &AtomicU64,
}

impl Iterator for Counted {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.next();
        if let Some(Ok(batch)) = &next {
            (self.counter)(&self.probe).fetch_add(batch.num_rows() as u64, Ordering::Relaxed);
            self.probe.batches.fetch_add(1, Ordering::Relaxed);
            self.probe
                .arrow_bytes
                .fetch_add(yggdryl::arrow::memory_size(batch) as u64, Ordering::Relaxed);
            self.probe.sample();
        }
        next
    }
}

impl RecordBatchReader for Counted {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }
}

/// `reader` with its rows counted as they are pulled. The stream crosses as
/// the batches it already is and comes back under its own root, so nothing
/// is cast or landed for the count.
fn counted(
    reader: SerieReader,
    probe: &Arc<Probe>,
    counter: fn(&Probe) -> &AtomicU64,
) -> SerieReader {
    let root = reader.field().clone();
    let batches: BatchReader = Box::new(Counted {
        inner: reader.into_arrow_reader(),
        probe: Arc::clone(probe),
        counter,
    });
    SerieReader::from_arrow_reader(Some(&root), batches, ArrowCastOptions::new())
        .expect("a counted stream under its own root")
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
                                probe.walked.load(Ordering::Relaxed),
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
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Stage {
    /// The text rows, read off the files and drained.
    Lines,
    /// The text rows appended into their table.
    Text,
    /// The FIX rows the stored text parses into, drained.
    Parse,
    /// The FIX rows walked through their lifecycle, drained.
    Lifecycle,
    /// The walked rows written over their table.
    Fix,
    /// The books folded from the stored FIX rows: the snapshots and the
    /// deltas, each written over its table.
    Books,
}

impl Stage {
    fn from_env() -> Self {
        match std::env::var("YGGDRYL_SCALE_STAGE").as_deref() {
            Err(_) | Ok("" | "books") => Self::Books,
            Ok("lines") => Self::Lines,
            Ok("text") => Self::Text,
            Ok("parse") => Self::Parse,
            Ok("lifecycle") => Self::Lifecycle,
            Ok("fix") => Self::Fix,
            Ok(other) => panic!(
                "expected YGGDRYL_SCALE_STAGE to be lines, text, parse, lifecycle, fix or books, \
                 got {other:?}"
            ),
        }
    }
}

/// One run's shape.
struct Run {
    copies: u64,
    /// The copies one span of the capture holds.
    density: u64,
    /// The copies one input file holds.
    copies_per_file: u64,
    stage: Stage,
    batch_rows: usize,
    batch_bytes: u64,
    commit_batches: usize,
    /// Whether a watchdog guards the run.
    watched: bool,
    /// Whether the tables are checked row by row once written: their order,
    /// their partitions, a window of them and a second run of each stage.
    verified: bool,
}

/// What one table holds once its stage has written it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Stored {
    rows: u64,
    /// The quarters of an hour its rows fall in: one partition each.
    partitions: usize,
    files: usize,
    snapshots: usize,
}

/// What one run read, wrote and held.
struct Outcome {
    input_bytes: u64,
    compressed_bytes: u64,
    files: u64,
    lines: u64,
    messages: u64,
    walked: u64,
    books: u64,
    deltas: u64,
    batches: u64,
    arrow_bytes: u64,
    text: Option<Stored>,
    fix: Option<Stored>,
    snapshots: Option<Stored>,
    flattened: Option<Stored>,
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

/// A table for the rows of one stream, created from the stream's own schema
/// as Iceberg states it: partitioned by the quarter of an hour each row's
/// `currunix` falls in - `partunix`, a column the table computes for every
/// row it is written - and sorted by it, the instant, the place within the
/// instant and the content hash, the instant and the hash its primary key.
fn create(root: &Path, row: &Field) -> IcebergTable<LocalFolder> {
    let mut schema = row
        .clone()
        .into_scheme_compat(&Scheme::ICEBERG)
        .expect("the row as Iceberg states it")
        .with_partition_by(["time_bucket('15 minutes', currunix) as partunix"
            .parse()
            .expect("the partition entry")])
        .expect("the partition column");
    // A table keyed by the instant refuses a row stating none, and no row
    // of it lacks its place.
    for name in REQUIRED {
        let column = schema
            .fields()
            .iter()
            .find(|field| field.name() == name)
            .unwrap_or_else(|| panic!("the row states {name}"))
            .clone()
            .with_nullable(false);
        schema.set_field(name, column).expect("the required column");
    }
    schema
        .as_sort_mut()
        .set_by_texts(["partunix", "currunix", "seqnum", "currhashcode"])
        .expect("the sort order");
    assign_field_ids(&mut schema, 1).expect("the field identifiers");
    // Iceberg names a key by the identifiers of its columns, which exist
    // once the schema is numbered.
    let key = primary_key(&schema);
    schema
        .as_iceberg_mut()
        .set_identifier_field_ids(&key)
        .expect("the primary key");
    let spec = PartitionSpec::from_schema(1, &schema).expect("the partition spec");
    IcebergTable::create(
        LocalFolder::new(root).expect("the table folder"),
        FormatVersion::V3,
        schema,
        spec,
    )
    .expect("the table creates")
}

/// The identifiers of the [`PRIMARY_KEY`] columns under `schema`, in
/// ascending order: Iceberg states a key as a set of identifiers.
fn primary_key(schema: &Field) -> Vec<i32> {
    let mut key: Vec<i32> = PRIMARY_KEY
        .iter()
        .map(|name| {
            let column = schema
                .fields()
                .iter()
                .find(|field| field.name() == *name)
                .unwrap_or_else(|| panic!("the row states {name}"));
            assert!(!column.is_nullable(), "{name}: a key column is required");
            column
                .parquet_field_id()
                .expect("a field identifier")
                .unwrap_or_else(|| panic!("{name} is numbered"))
        })
        .collect();
    key.sort_unstable();
    key
}

/// The options one stage writes its table under: the commit cadence.
fn writing(table: &IcebergTable<LocalFolder>, shape: &Run) -> RecordOptions {
    let mut options = table.record_options().expect("the table's options");
    options.set_commit_batch_num(Some(shape.commit_batches));
    options
}

/// The table's rows in the table's own order, as the rows `row` types -
/// the crate's datatypes again, the partition column dropped - and, where
/// `window` states one, only those it keeps.
fn stored(table: &IcebergTable<LocalFolder>, row: &Field, window: Option<&str>) -> SerieReader {
    let mut options = table.record_options().expect("the table's options");
    options.set_field(row.clone());
    let options = match window {
        Some(window) => options.with_filter(window).expect("the window"),
        None => options,
    };
    table.read_serie(Some(&options)).expect("the table reads")
}

/// The FIX table's rows as messages again, walked in the table's order.
fn walked_again(
    codec: &FixCodec,
    table: &IcebergTable<LocalFolder>,
    row: &Field,
) -> impl Iterator<Item = yggdryl::Result<FixMsg>> + Send + 'static {
    let messages = codec
        .messages_serie(stored(table, row, None))
        .expect("the stored rows as messages");
    codec.lifecycle(messages)
}

/// What `table` holds, counted off its metadata and its plan.
fn held(table: &IcebergTable<LocalFolder>) -> Stored {
    let files = table.data_files().expect("the data files");
    let mut partitions: Vec<_> = files
        .iter()
        .map(|(file, _)| file.partition.clone())
        .collect();
    partitions.sort();
    partitions.dedup();
    Stored {
        rows: table.row_size().expect("the table's row count"),
        partitions: partitions.len(),
        files: files.len(),
        snapshots: table.metadata().expect("the metadata").snapshots().len(),
    }
}

/// Every row of `table` as its sort key, in the order a read yields them:
/// `(partunix, currunix, seqnum, currhashcode)`, the two instants as their
/// nanosecond counts.
fn keys(table: &IcebergTable<LocalFolder>, window: Option<&str>) -> Vec<(i64, i64, i128, i128)> {
    let options = table.record_options().expect("the table's options");
    let options = match window {
        Some(window) => options.with_filter(window).expect("the window"),
        None => options,
    };
    let mut keys = Vec::new();
    for batch in table
        .read_serie(Some(&options))
        .expect("the table reads")
        .into_arrow_reader()
    {
        let batch = batch.expect("a stored batch");
        let instants = |name: &str| {
            batch
                .column_by_name(name)
                .unwrap_or_else(|| panic!("the stored rows carry {name}"))
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap_or_else(|| panic!("{name} is a nanosecond instant"))
                .clone()
        };
        let counts = |name: &str| {
            batch
                .column_by_name(name)
                .unwrap_or_else(|| panic!("the stored rows carry {name}"))
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap_or_else(|| panic!("{name} is stored as decimal(20, 0)"))
                .clone()
        };
        let (partunix, currunix) = (instants("partunix"), instants("currunix"));
        let (seqnum, currhashcode) = (counts("seqnum"), counts("currhashcode"));
        assert_eq!(seqnum.null_count(), 0, "every row states its place");
        for row in 0..batch.num_rows() {
            keys.push((
                partunix.value(row),
                currunix.value(row),
                seqnum.value(row),
                currhashcode.value(row),
            ));
        }
    }
    keys
}

/// Check what one written table promises: every row in the quarter of an
/// hour its instant falls in, the rows read back in the table's order, one
/// partition a quarter, and the identity columns typed `uuid`.
fn verify(name: &str, table: &IcebergTable<LocalFolder>, expected: &Stored) {
    let keys = keys(table, None);
    assert_eq!(
        keys.len() as u64,
        expected.rows,
        "{name}: every stored row reads back"
    );
    for (partunix, currunix, _, _) in &keys {
        assert_eq!(
            *partunix,
            currunix.div_euclid(QUARTER_NS) * QUARTER_NS,
            "{name}: partunix is currunix floored to the quarter hour"
        );
    }
    assert!(
        keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "{name}: a read yields the rows in partunix, currunix, seqnum, currhashcode order"
    );
    let mut quarters: Vec<i64> = keys.iter().map(|key| key.0).collect();
    quarters.dedup();
    assert_eq!(
        quarters.len(),
        expected.partitions,
        "{name}: one partition a quarter of an hour, each read once"
    );
    let schema = table.schema().expect("the table's schema");
    for child in schema.fields() {
        if matches!(child.name(), "curruuid" | "crossuuid" | "prevuuid") {
            assert_eq!(
                *child.dtype(),
                DataType::uuid(),
                "{name}: {} is a uuid, never a fixed binary",
                child.name()
            );
        }
    }
    // Every column says what it holds - the partition column by the term it
    // is derived by - and the table states each as the column's doc.
    let silent: Vec<&str> = schema
        .fields()
        .iter()
        .filter(|child| child.description().is_none())
        .map(Field::name)
        .collect();
    assert!(
        silent.is_empty(),
        "{name}: every column says what it holds, and these do not: {silent:?}"
    );
    assert_eq!(
        schema
            .get_field_by_path("partunix")
            .and_then(Field::description),
        Some("Derived from the row as time_bucket('15 minutes', currunix)."),
        "{name}: the partition column is described by its term"
    );
    assert_eq!(
        schema.get_metadata("PARTITION:by"),
        Some(r#"["partunix"]"#),
        "{name}: the table partitions by its own partunix column"
    );
    // The key is the table's own: a handle opened afresh reads it out of
    // the metadata every commit rewrote.
    let reopened = IcebergTable::open(
        LocalFolder::new(table.root().path().expect("a local table")).expect("the table folder"),
    )
    .expect("the table reopens");
    let stated = reopened.schema().expect("the stored schema");
    assert_eq!(
        stated
            .as_iceberg()
            .identifier_field_ids()
            .expect("the identifier fields"),
        primary_key(stated),
        "{name}: currunix and currhashcode are the table's primary key"
    );
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
    let logs = scratch.0.join("logs");
    std::fs::create_dir_all(&logs).expect("the scratch folder creates");

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
    let template = Template::new(shape.density);
    let mut input_bytes = 0;
    let mut compressed_bytes = 0;
    let mut files = 0;
    let mut first = 0;
    while first < shape.copies {
        let last = (first + shape.copies_per_file).min(shape.copies);
        let path = logs.join(format!("ulbridge-{files:05}.log.zst"));
        input_bytes += generate(&template, first..last, &path);
        compressed_bytes += std::fs::metadata(&path).expect("the input").len();
        files += 1;
        first = last;
    }
    let generated_in = started.elapsed();
    drop(template);

    let mut options = reading();
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_capture_names(options.capture_names())
        .with_sorted_lifecycle(true)
        .with_batch_row_size(shape.batch_rows);
    options.set_batch_row_size(Some(shape.batch_rows));
    let line_options = options.clone();
    let options = RecordOptions::from(options);
    let probe = Arc::new(Probe::default());
    let watchdog = shape
        .watched
        .then(|| Watchdog::start(scratch.0.clone(), Arc::clone(&probe)));
    let baseline = Resident::now();

    let started = Instant::now();
    // Every file the folder holds is one leaf of one table of text rows,
    // read through the native local backend, one leaf open at a time.
    let source = LocalFolder::new(&logs).expect("the input folder");
    let lines = source.read_serie(Some(&options)).expect("the text rows");
    let text_row = lines.field().clone();
    let lines = counted(lines, &probe, |probe| &probe.lines);

    let mut outcome = Outcome {
        input_bytes,
        compressed_bytes,
        files,
        lines: 0,
        messages: 0,
        walked: 0,
        books: 0,
        deltas: 0,
        batches: 0,
        arrow_bytes: 0,
        text: None,
        fix: None,
        snapshots: None,
        flattened: None,
        table_bytes: 0,
        baseline,
        finale: None,
        samples: Vec::new(),
        watched_peak: None,
        generated_in,
        streamed_in: Duration::ZERO,
    };

    if shape.stage == Stage::Lines {
        for record in lines {
            record.expect("a batch of text rows");
        }
    } else {
        // The text table: `append_serie` is the whole call.
        let mut text = create(&scratch.0.join("text"), &text_row);
        let appended = text
            .append_serie(SerieSource::from(lines), Some(&writing(&text, shape)))
            .expect("the text rows append");
        let text_held = held(&text);
        // The write says what it did: every line read is a row written.
        assert_eq!(
            appended,
            IOResult::new(probe.lines.load(Ordering::Relaxed), text_held.rows),
            "the text append's result"
        );
        assert_eq!(appended.skipped_rows, 0, "no line is skipped");
        println!(
            "scale_ulbridge: text table {text_held:?} from {} lines in {:.1?}",
            probe.lines.load(Ordering::Relaxed),
            started.elapsed(),
        );
        outcome.text = Some(text_held);
        if shape.verified {
            verify("text", &text, &text_held);
        }

        if shape.stage >= Stage::Parse {
            // The stored text, in the table's order, parsed and walked.
            let parsed = codec
                .parse_text_serie(stored(&text, &text_row, None))
                .expect("the FIX rows");
            let fix_row = parsed.field().clone();
            let parsed = counted(parsed, &probe, |probe| &probe.messages);
            if shape.stage == Stage::Parse {
                for record in parsed {
                    record.expect("a batch of FIX rows");
                }
            } else {
                let walked = counted(
                    codec.lifecycle_serie(parsed).expect("the walked rows"),
                    &probe,
                    |probe| &probe.walked,
                );
                if shape.stage == Stage::Lifecycle {
                    for record in walked {
                        record.expect("a batch of walked rows");
                    }
                } else {
                    // The FIX table: an overwrite replaces the partitions
                    // the walked rows fall in, so the stage runs again over
                    // any window of the text it was made from.
                    let mut fix = create(&scratch.0.join("fix"), &fix_row);
                    let written = fix
                        .overwrite_serie(SerieSource::from(walked), Some(&writing(&fix, shape)))
                        .expect("the walked rows write");
                    let fix_held = held(&fix);
                    assert_eq!(
                        written,
                        IOResult::new(probe.walked.load(Ordering::Relaxed), fix_held.rows),
                        "the FIX overwrite's result"
                    );
                    println!(
                        "scale_ulbridge: fix table {fix_held:?} from {} messages, {} walked in {:.1?}",
                        probe.messages.load(Ordering::Relaxed),
                        probe.walked.load(Ordering::Relaxed),
                        started.elapsed(),
                    );
                    outcome.fix = Some(fix_held);
                    if shape.verified {
                        verify("fix", &fix, &fix_held);
                        // The serie doors over the stored text walk what the
                        // line doors walk over the files themselves.
                        let by_lines = codec
                            .lifecycle(codec.parse_text_lines(
                                read_text_lines(&source, &line_options).expect("the line reader"),
                            ))
                            .inspect(|message| {
                                message.as_ref().expect("a walked message");
                            })
                            .count() as u64;
                        assert_eq!(
                            probe.walked.load(Ordering::Relaxed),
                            by_lines,
                            "the table's rows walk as the files' lines do"
                        );
                        reprocess(&codec, &text, &text_row, &mut fix, shape, &fix_held);
                    }
                    if shape.stage == Stage::Books {
                        let (snapshots, flattened) =
                            books(&codec, &fix, &fix_row, &scratch.0, shape, &probe);
                        outcome.snapshots = Some(snapshots);
                        outcome.flattened = Some(flattened);
                    }
                }
            }
        }
    }
    outcome.streamed_in = started.elapsed();
    outcome.finale = Resident::now();
    outcome.watched_peak = watchdog.map(Watchdog::finish);
    outcome.table_bytes = ["text", "fix", "books", "deltas"]
        .iter()
        .map(|table| folder_bytes(&scratch.0.join(table)))
        .sum();
    outcome.samples = probe.samples.lock().expect("the samples").clone();
    outcome.lines = probe.lines.load(Ordering::Relaxed);
    outcome.messages = probe.messages.load(Ordering::Relaxed);
    outcome.walked = probe.walked.load(Ordering::Relaxed);
    outcome.books = probe.books.load(Ordering::Relaxed);
    outcome.deltas = probe.deltas.load(Ordering::Relaxed);
    outcome.batches = probe.batches.load(Ordering::Relaxed);
    outcome.arrow_bytes = probe.arrow_bytes.load(Ordering::Relaxed);
    outcome
}

/// Run the FIX stage again, twice: over the whole text table, which leaves
/// the FIX table as it was, and over the window one copy of the capture
/// spans, which rewrites that window's partitions and no other.
fn reprocess(
    codec: &FixCodec,
    text: &IcebergTable<LocalFolder>,
    text_row: &Field,
    fix: &mut IcebergTable<LocalFolder>,
    shape: &Run,
    expected: &Stored,
) {
    let rewrite = |fix: &mut IcebergTable<LocalFolder>, window: Option<&str>| {
        let walked = codec
            .lifecycle_serie(
                codec
                    .parse_text_serie(stored(text, text_row, window))
                    .expect("the FIX rows"),
            )
            .expect("the walked rows");
        let options = writing(fix, shape);
        fix.overwrite_serie(SerieSource::from(walked), Some(&options))
            .expect("the walked rows write again")
    };
    // The data files of every partition, by the quarter of an hour it holds.
    let files = |fix: &IcebergTable<LocalFolder>| -> BTreeMap<Option<i64>, Vec<String>> {
        let mut files: BTreeMap<Option<i64>, Vec<String>> = BTreeMap::new();
        for (file, _) in fix.data_files().expect("the data files") {
            files
                .entry(
                    file.partition
                        .first()
                        .and_then(yggdryl::Scalar::temporal_count),
                )
                .or_default()
                .push(file.file_path.to_string());
        }
        for paths in files.values_mut() {
            paths.sort();
        }
        files
    };

    let rewritten = rewrite(fix, None);
    let again = held(fix);
    assert_eq!(
        rewritten,
        IOResult::new(expected.rows, expected.rows),
        "a second run reads and writes the same rows"
    );
    assert_eq!(
        again.rows, expected.rows,
        "a second run writes the same rows"
    );
    assert_eq!(
        again.partitions, expected.partitions,
        "a second run writes the same partitions"
    );
    verify("fix, written twice", fix, &again);

    // The second copy of the capture: four hours from 05:00 UTC.
    if shape.copies < 3 {
        return;
    }
    let window = "currunix >= '2026-08-14T05:00:00Z' and currunix < '2026-08-14T09:00:00Z'";
    let plan = text.plan_matching(window).expect("the window plans");
    let all = text.data_files().expect("the data files").len();
    assert!(
        !plan.tasks.is_empty() && plan.tasks.len() < all,
        "the window reads {} of the text table's {all} files",
        plan.tasks.len()
    );
    let window_keys = keys(text, Some(window));
    assert_eq!(
        window_keys.len() as u64,
        LINES_PER_COPY,
        "the window holds one copy's lines"
    );
    assert!(
        window_keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "a window of the table is read in the table's order"
    );
    // A message is dated by its own clock, not its line's, so the rows a
    // window of text walks into reach the partitions those clocks fall in -
    // an order's expiry hours after the line that placed it. Each of them is
    // replaced whole and every other partition keeps the very files it had:
    // no partition is half rewritten, none is rewritten for nothing.
    let before = files(fix);
    rewrite(fix, Some(window));
    let windowed = held(fix);
    assert_eq!(
        windowed.rows, expected.rows,
        "the window's rows are replaced, not added"
    );
    let after = files(fix);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "the table holds the same partitions"
    );
    let (mut kept, mut replaced) = (0, 0);
    for (quarter, stood) in &before {
        let stands = &after[quarter];
        if stood == stands {
            kept += 1;
        } else {
            assert!(
                stood.iter().all(|path| !stands.contains(path)),
                "partition {quarter:?} is replaced whole: {stood:?} -> {stands:?}"
            );
            replaced += 1;
        }
    }
    assert!(replaced > 0, "the window's partitions are rewritten");
    assert!(
        kept > 0 && replaced < before.len(),
        "the partitions the window's rows do not reach keep their files: {kept} kept, {replaced} \
         replaced"
    );
    verify("fix, one window rewritten", fix, &windowed);
}

/// The books the stored FIX rows fold into, a snapshot of every book each
/// quarter of an hour: the complete books written over one table, and the
/// deltas of every book flattened to `marketdata` rows over another. The
/// FIX table is read once for each, in its own order.
fn books(
    codec: &FixCodec,
    fix: &IcebergTable<LocalFolder>,
    fix_row: &Field,
    scratch: &Path,
    shape: &Run,
    probe: &Arc<Probe>,
) -> (Stored, Stored) {
    let row = MarketData::field().expect("the marketdata row");

    // Every book the fold answers; the table keeps the complete ones.
    let folded = counted(
        codec
            .book_serie(walked_again(codec, fix, fix_row), QUARTER_MS, None)
            .expect("the books"),
        probe,
        |probe| &probe.books,
    );
    let mut snapshots = create(&scratch.join("books"), &row);
    let complete = writing(&snapshots, shape)
        .with_filter("snapunix is not null")
        .expect("the complete books");
    let kept = snapshots
        .overwrite_serie(SerieSource::from(folded), Some(&complete))
        .expect("the snapshots write");
    let snapshots_held = held(&snapshots);
    // The `where` keeps the incomplete books out, and the result counts them.
    assert_eq!(
        kept,
        IOResult::new(probe.books.load(Ordering::Relaxed), snapshots_held.rows),
        "the snapshot overwrite's result"
    );
    assert_eq!(
        kept.skipped_rows,
        kept.read_rows - snapshots_held.rows,
        "the books no quarter closed are skipped"
    );

    // Every order and quote a book applied, in the order applied.
    let deltas = BookIterator::new(
        walked_again(codec, fix, fix_row).map(|message| message.map(MarketData::from)),
        QUARTER_MS,
    )
    .expect("the book fold")
    .flat_map(|book| match book {
        Ok(book) => book.deltas().cloned().map(Ok).collect::<Vec<_>>(),
        Err(error) => vec![Err(error)],
    });
    let batches = MarketData::arrow_reader(deltas, Some(shape.batch_rows), Some(shape.batch_bytes))
        .expect("the delta rows");
    let flattened = counted(
        SerieReader::from_arrow_reader(Some(&row), batches, ArrowCastOptions::new())
            .expect("the delta stream"),
        probe,
        |probe| &probe.deltas,
    );
    let mut deltas = create(&scratch.join("deltas"), &row);
    let flat = deltas
        .overwrite_serie(SerieSource::from(flattened), Some(&writing(&deltas, shape)))
        .expect("the deltas write");
    let deltas_held = held(&deltas);
    assert_eq!(
        flat,
        IOResult::new(deltas_held.rows, deltas_held.rows),
        "the delta overwrite's result"
    );

    if shape.verified {
        verify("books", &snapshots, &snapshots_held);
        verify("deltas", &deltas, &deltas_held);
        // Both tables read back as the market data they were written from:
        // a snapshot a book, a delta the order or the quote a book applied.
        for (table, held) in [(&snapshots, &snapshots_held), (&deltas, &deltas_held)] {
            let read = MarketData::from_arrow_reader(stored(table, &row, None).into_arrow_reader())
                .expect("the stored rows as market data")
                .collect::<Result<Vec<MarketData>, _>>()
                .expect("every stored row reads back");
            assert_eq!(read.len() as u64, held.rows);
        }
    }
    (snapshots_held, deltas_held)
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
        "scale_ulbridge: generated {} MiB ({} MiB compressed) in {} files in {:.1?}; streamed in \
         {:.1?}",
        mib(outcome.input_bytes),
        mib(outcome.compressed_bytes),
        outcome.files,
        outcome.generated_in,
        outcome.streamed_in,
    );
    println!(
        "scale_ulbridge: {} lines, {} messages, {} walked, {} books, {} deltas in {} batches ({} \
         MiB of Arrow); {} MiB of tables on disk",
        outcome.lines,
        outcome.messages,
        outcome.walked,
        outcome.books,
        outcome.deltas,
        outcome.batches,
        mib(outcome.arrow_bytes),
        mib(outcome.table_bytes),
    );
    for (name, table) in [
        ("text", outcome.text),
        ("fix", outcome.fix),
        ("books", outcome.snapshots),
        ("deltas", outcome.flattened),
    ] {
        if let Some(table) = table {
            println!(
                "scale_ulbridge:   {name:<6} {} rows in {} partitions, {} files, {} snapshots",
                table.rows, table.partitions, table.files, table.snapshots,
            );
        }
    }
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
    // The trace, at most twenty readings evenly placed.
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

/// The copies one span holds at scale: `YGGDRYL_SCALE_DENSITY`, else
/// [`DENSITY`].
fn scale_density() -> u64 {
    match std::env::var("YGGDRYL_SCALE_DENSITY") {
        Ok(value) if !value.trim().is_empty() => match value.trim().parse::<u64>() {
            Ok(density) if density > 0 => density,
            _ => panic!(
                "expected YGGDRYL_SCALE_DENSITY to be a copy count above zero, got {value:?}"
            ),
        },
        _ => DENSITY,
    }
}

#[test]
fn stacked_copies_share_a_span_a_second_apart() {
    let template = Template::new(2);
    let first_clock = |copy: u64| {
        let mut rendered = Vec::new();
        template.render(copy, &mut rendered);
        assert_eq!(rendered.len(), LOG.len());
        String::from_utf8(rendered[..23].to_vec()).unwrap()
    };

    // Two copies a span: the second a second after the first, the third a
    // span - four hours - after it, the fourth a second after that.
    assert_eq!(first_clock(0), "2026-08-14 01:03:13.314");
    assert_eq!(first_clock(1), "2026-08-14 01:03:14.314");
    assert_eq!(first_clock(2), "2026-08-14 05:03:13.314");
    assert_eq!(first_clock(3), "2026-08-14 05:03:14.314");

    // No copy repeats another: the identifiers step by the copy, not the span.
    let mut one = Vec::new();
    template.render(0, &mut one);
    let mut two = Vec::new();
    template.render(1, &mut two);
    let differing = one
        .split_inclusive(|byte| *byte == b'\n')
        .zip(two.split_inclusive(|byte| *byte == b'\n'))
        .filter(|(left, right)| left[23..] != right[23..])
        .count();
    assert!(differing > 0, "a stacked copy steps its identifiers");
}

#[test]
fn the_generator_writes_fresh_copies_of_one_width() {
    let template = Template::new(1);
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
fn the_capture_pipeline_lands_every_stage_in_order() {
    let _process = PROCESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let outcome = run(&Run {
        copies: SMOKE_COPIES,
        // A copy a span: every stage sees quarters that close behind the next.
        density: 1,
        // Two files, so the text stage reads a folder of more than one leaf.
        copies_per_file: 2,
        stage: Stage::Books,
        // Small batches and a short cadence, so three copies cross several
        // batches and several commits.
        batch_rows: 64,
        batch_bytes: BATCH_BYTES,
        commit_batches: 2,
        watched: false,
        verified: true,
    });
    report(&outcome);
    assert_eq!(outcome.files, 2);
    let text = outcome.text.expect("the text table");
    let fix = outcome.fix.expect("the FIX table");
    let snapshots = outcome.snapshots.expect("the books table");
    let flattened = outcome.flattened.expect("the deltas table");

    assert_eq!(
        text.rows,
        SMOKE_COPIES * LINES_PER_COPY,
        "every line is a row"
    );
    assert!(
        text.snapshots >= 2,
        "the cadence committed the text more than once: {text:?}"
    );
    assert!(text.partitions as u64 >= SMOKE_COPIES, "{text:?}");

    // The first parse, and the two runs that wrote the FIX table again: the
    // whole text, then one copy's window of it.
    assert_eq!(
        outcome.messages,
        SMOKE_COPIES * MESSAGES_PER_COPY,
        "every message of every copy parses"
    );
    assert_eq!(
        fix.rows, outcome.walked,
        "the FIX table holds every row the lifecycle walked"
    );
    assert!(fix.rows > 0 && fix.rows <= outcome.messages, "{fix:?}");

    assert!(
        snapshots.rows > 0,
        "the fold answers a book on the grid: {snapshots:?}"
    );
    assert!(
        snapshots.rows <= outcome.books,
        "only complete books are stored: {snapshots:?} of {} folded",
        outcome.books
    );
    assert_eq!(
        flattened.rows, outcome.deltas,
        "the deltas table holds every delta the books applied"
    );
    assert!(flattened.rows > 0, "{flattened:?}");
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
    let batches = usize::try_from((copies * LINES_PER_COPY).div_ceil(BATCH_ROWS as u64))
        .expect("a batch count");
    let outcome = run(&Run {
        copies,
        density: scale_density(),
        copies_per_file: COPIES_PER_FILE,
        stage: Stage::from_env(),
        batch_rows: BATCH_ROWS,
        batch_bytes: BATCH_BYTES,
        commit_batches: (batches / COMMITS).clamp(1, MAX_COMMIT_BATCHES),
        watched: true,
        verified: false,
    });
    report(&outcome);
    assert_eq!(outcome.lines, copies * LINES_PER_COPY);
    if let Some(text) = outcome.text {
        assert_eq!(text.rows, outcome.lines, "the text table holds every line");
    }
    if let Some(fix) = outcome.fix {
        assert_eq!(outcome.messages, copies * MESSAGES_PER_COPY);
        assert_eq!(
            fix.rows, outcome.walked,
            "the FIX table holds every walked row"
        );
    }
    if let Some(flattened) = outcome.flattened {
        assert_eq!(
            flattened.rows, outcome.deltas,
            "the deltas table holds every delta"
        );
    }
    let lines = outcome.lines;
    let (Some(quarter), Some(last_half)) = (
        peak(&outcome.samples, 0, lines / 4),
        peak(&outcome.samples, lines / 2, u64::MAX),
    ) else {
        println!(
            "SKIPPED: this platform states no RssAnon, so the memory bound is not asserted; the \
             path ran and its counts hold"
        );
        return;
    };
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
