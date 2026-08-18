//! The Arrow line projection: parse throughput and the cost of its hash.
//!
//! The corpus is ~100k synthetic trading-log lines. Parse cases report the
//! *decoded* payload as byte throughput - for the gzip case too, because
//! decoded text is what the parser actually consumes - so the two numbers
//! answer "what does the coding cost" rather than mixing wire sizes. The
//! hash cases isolate the stable FNV-1a fold the `hash` column pays, on the
//! same messages the parse hashes: hashing measured beside the whole parse is
//! what justifies keeping the dependency-free hash (regex and UTF-8 dominate;
//! the recorded numbers in `docs/benchmarks.md` say by how much).
//!
//! `lines_gzip` measures the same projection over the production shape:
//! rotated log files on local storage, a million records split across eight
//! leaves, read through one [`Folder`] handle. It isolates four differences,
//! one per pair of cases, over corpora holding byte-identical records:
//!
//! - `folder/gzip` against `folder/plain` is the net cost of holding the
//!   corpus gzip-coded on local storage - the decision a production reader
//!   actually makes - and not the inflate in isolation. Both report the same
//!   decoded byte count, because the gzip leaves are decoded as a stream and
//!   the parser consumes the same text either way, but the two sides do not
//!   move the same *source* bytes: [`File`] maps its leaf, so `folder/plain`
//!   pulls the whole decoded payload through the mapping while `folder/gzip`
//!   pulls about a fifth of it and inflates. The delta is inflate work minus
//!   the source traffic it saves, two effects of opposite sign that nearly
//!   cancel here - near enough that which side is faster has not been stable
//!   between runs on one box, so read the pair as "coded storage costs about
//!   nothing on this shape", never as a measurement of inflate. Inflate alone
//!   is what `lines_arrow`'s `parse/plain` against `parse/gzip` isolates,
//!   both sides being in-memory [`Buffer`] handles over identical source
//!   bytes. The gzip case's wire bytes are never the throughput reported - a
//!   number over wire bytes would describe the compression ratio of the
//!   fixture, not the parse.
//! - `single/gzip` against `folder/gzip` is what the rotated shape costs:
//!   the same records in one leaf against eight, so the number covers
//!   per-leaf open, listing, and the batch boundary a leaf change forces.
//! - `casts/text` against `casts/typed` is what typed captures cost.
//!   `thread_id` and `latency_us` infer `int64` from their `\d+`
//!   sub-patterns, so the default read casts every closing batch onto the
//!   declared root; `casts/text` declares both `utf8`, which turns the cast
//!   off and leaves the text the builders already produced. The difference
//!   is the price of handing a consumer typed columns instead of strings it
//!   would convert itself.
//! - `scale/125k` through `scale/1m` is the same gzip folder at four sizes.
//!   Per-byte throughput staying flat across them is the evidence that
//!   nothing in the projection is quadratic. It is not evidence about
//!   residency: a reader that buffered the whole decoded corpus before
//!   emitting a batch would look just as flat, buffering being linear too.
//!   These cases time wall clock and nothing else, so the streaming claim -
//!   one batch, not one corpus, held at a time - is not measured here; it
//!   needs a peak-RSS probe across the sizes, which Criterion does not
//!   report. `scale/1m` and `casts/typed` repeat
//!   `folder/gzip`'s work deliberately: a claim is read off cases from the
//!   same run, so each comparison carries its own baseline.
//!
//! Every corpus is generated, written, and proven to yield exactly its record
//! count before the first timer starts - a parser that split or dropped the
//! multi-line records (every fiftieth record carries two continuation lines
//! the projection folds into one row) would otherwise look faster while
//! measuring something else. The fixtures live in a process-unique directory
//! under [`std::env::temp_dir`] and are removed when the group finishes.
//!
//! [`Folder`]: yggdryl::local::Folder

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{Criterion, Throughput};
use yggdryl::io::{Buffer, IOBase, LineRecordOptions};
use yggdryl::local::{File, Folder};
use yggdryl::text::stable_hash_bytes;
use yggdryl::{DataType, Url};

/// Log lines per corpus: enough that per-read setup is noise.
const LINES: usize = 100_000;

/// The header pattern of the synthetic feed, capturing level and logger.
const PATTERN: &str =
    r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]";

/// One session of synthetic trading-log text.
fn corpus() -> String {
    let mut text = String::with_capacity(LINES * 96);
    for index in 0..LINES {
        let (minute, second, micro) = (index / 3_600 % 60, index / 60 % 60, index % 1_000_000);
        let level = ["ii", "ww", "ee"][index % 3];
        let price = 187.0 + (index % 400) as f64 / 100.0;
        text.push_str(&format!(
            "2024-02-01 10:{minute:02}:{second:02}.{micro:06} [{level}] [engine] \
             fill {} SYMB-{:04} @ {price:.2} order={index:08}\n",
            100 + index % 900,
            index % 128,
        ));
    }
    text
}

/// A buffer whose media type carries the codings its name declares.
fn handle(name: &str, bytes: &[u8]) -> Buffer {
    let mut handle = Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("a valid url")
            .media_type(),
    );
    handle.write_all_bytes(bytes).expect("a seeded fixture");
    handle
}

/// Drain one projection read, returning the row count it materialized.
fn parsed_rows<H: IOBase>(handle: &H, options: &LineRecordOptions) -> usize {
    handle
        .read_arrow_lines(options)
        .expect("a line reader")
        .map(|batch| batch.expect("a parsed batch").num_rows())
        .sum()
}

pub(crate) fn lines_arrow_benchmarks(criterion: &mut Criterion) {
    let options = LineRecordOptions::new(PATTERN).expect("a valid pattern");
    let text = corpus();
    let decoded = text.len() as u64;
    let plain = handle("bench.log", text.as_bytes());
    let gzip = handle(
        "bench.log.gz",
        &yggdryl::gzip::dump(text.as_bytes()).expect("a gzip fixture"),
    );
    // The parse is validated once outside the timed loops.
    assert_eq!(parsed_rows(&plain, &options), LINES);
    assert_eq!(parsed_rows(&gzip, &options), LINES);

    let mut group = criterion.benchmark_group("lines_arrow");
    group.sample_size(10);

    group.throughput(Throughput::Bytes(decoded));
    group.bench_function("parse/plain", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&plain), &options));
    });
    group.bench_function("parse/gzip", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip), &options));
    });
    // The grouping stage alone - the same records without the Arrow
    // projection - so "what does the projection add on top of
    // `read_lines_matching`" is a measured number.
    group.bench_function("group/plain", |bencher| {
        bencher.iter(|| {
            black_box(&plain)
                .read_lines_matching(PATTERN)
                .expect("a record reader")
                .map(|record| record.expect("a record").len())
                .sum::<usize>()
        });
    });

    // The exact messages the parse hashes: header stripped, trimmed. Hashing
    // them alone is the parse's hash cost, measured over the same payload.
    let messages: Vec<&str> = text
        .lines()
        .map(|line| line.split(']').nth(2).expect("a message tail").trim())
        .collect();
    let hashed: u64 = messages.iter().map(|message| message.len() as u64).sum();
    group.throughput(Throughput::Bytes(hashed));
    group.bench_function("hash/corpus", |bencher| {
        bencher.iter(|| {
            messages
                .iter()
                .map(|message| stable_hash_bytes(black_box(message.as_bytes())))
                .fold(0_u64, u64::wrapping_add)
        });
    });
    group.finish();

    // The FNV-1a micro-benchmark on realistic message sizes: the numbers that
    // decide whether a faster hash dependency would ever be worth adding.
    let mut micro = criterion.benchmark_group("lines_hash");
    for (label, size) in [("100b", 100), ("512b", 512), ("2kib", 2_048)] {
        let message: String = "fill 100 SYMB-0042 @ 188.01 order=00001234 "
            .chars()
            .cycle()
            .take(size)
            .collect();
        micro.throughput(Throughput::Bytes(size as u64));
        micro.bench_function(format!("fnv1a/{label}"), |bencher| {
            bencher.iter(|| stable_hash_bytes(black_box(message.as_bytes())));
        });
    }
    micro.finish();
}

/// Records in the rotated corpus: the production shape the group measures.
const RECORDS: usize = 1_000_000;

/// Rotated leaves one folder corpus is split across.
const LEAVES: usize = 8;

/// The rotated feed's header pattern.
///
/// `level` and `logger` are text; `thread_id` and `latency_us` are exactly
/// `\d+`, which the closed inference table types `int64` - so the default
/// read is the typed one, and `casts/text` is the declaration that turns it
/// off.
const ROTATED_PATTERN: &str = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\] \[(?<thread_id>\d+)\] took=(?<latency_us>\d+)";

/// A written corpus: the handle addressing it and what its rows weigh.
struct Corpus<H> {
    /// The handle a case reads through - a folder or a single leaf.
    handle: H,
    /// Decoded text bytes across every leaf, the payload the parser consumes.
    decoded: u64,
    /// Records the projection must yield, proven before any timing.
    rows: usize,
    /// Text lines those records span, proven before any timing: the corpus
    /// carries two continuation lines per fiftieth record, and they belong to
    /// their record's row rather than to rows of their own.
    lines: i64,
}

/// The text lines `records` records of the shared corpus spec occupy.
///
/// One line each, plus the two continuation lines every fiftieth record
/// carries - the same arithmetic the generator performs, stated once.
const fn spanned_lines(records: usize) -> i64 {
    (records + 2 * (records / 50)) as i64
}

/// Append one record of the shared corpus spec to `text`.
///
/// `index` is global across leaves, so the rotated corpus and the single-file
/// one hold byte-identical records. Every fiftieth record carries two
/// continuation lines that belong to the same row: the multi-line shape a
/// line-counting loop miscounts and the projection folds.
fn rotated_record(text: &mut String, index: usize) {
    use std::fmt::Write as _;

    let (minute, second, micro) = (index / 3_600 % 60, index / 60 % 60, index % 1_000_000);
    let level = ["ii", "ww", "ee"][index % 3];
    let logger = ["engine", "router", "ledger", "feed"][index % 4];
    let (thread, latency) = (index % 16, 40 + index % 960);
    let (qty, symbol) = (100 + index % 900, index % 128);
    // Cents rather than a float: the two-decimal price is exact and identical
    // in every language that generates this corpus.
    let cents = 18_700 + index % 400;
    writeln!(
        text,
        "2024-02-01 10:{minute:02}:{second:02}.{micro:06} [{level}] [{logger}] [{thread}] \
         took={latency} fill {qty} SYMB-{symbol:04} @ {}.{:02} order={index:08}",
        cents / 100,
        cents % 100,
    )
    .expect("a string write cannot fail");
    if index % 50 == 49 {
        text.push_str("    at engine::match(order.rs:118)\n    at engine::step(order.rs:64)\n");
    }
}

/// The [`LEAVES`] rotated payloads holding `records` records between them.
fn rotated_shards(records: usize) -> Vec<String> {
    assert!(
        records % LEAVES == 0,
        "the corpus splits evenly into leaves"
    );
    let per_leaf = records / LEAVES;
    (0..LEAVES)
        .map(|leaf| {
            let mut text = String::with_capacity(per_leaf * 128);
            for index in leaf * per_leaf..(leaf + 1) * per_leaf {
                rotated_record(&mut text, index);
            }
            text
        })
        .collect()
}

/// Write one leaf, gzip-coded by the crate's own codec or as plain text.
fn write_leaf(path: &Path, text: &str, gzip: bool) {
    if gzip {
        let coded = yggdryl::gzip::dump(text.as_bytes()).expect("the fixture codes");
        std::fs::write(path, coded).expect("the fixture leaf writes");
    } else {
        std::fs::write(path, text.as_bytes()).expect("the fixture leaf writes");
    }
}

/// Write the shards as `app-0..7.log[.gz]` and address them as one folder.
fn rotated_folder(
    directory: &Path,
    shards: &[String],
    records: usize,
    gzip: bool,
) -> Corpus<Folder> {
    std::fs::create_dir_all(directory).expect("the scratch directory creates");
    let mut decoded = 0;
    for (leaf, text) in shards.iter().enumerate() {
        let name = if gzip {
            format!("app-{leaf}.log.gz")
        } else {
            format!("app-{leaf}.log")
        };
        write_leaf(&directory.join(name), text, gzip);
        decoded += text.len() as u64;
    }
    Corpus {
        handle: Folder::new(directory).expect("the scratch directory is addressable"),
        decoded,
        rows: records,
        lines: spanned_lines(records),
    }
}

/// Write every shard into one `app.log.gz`: the same records, unrotated.
fn single_leaf(directory: &Path, shards: &[String], records: usize) -> Corpus<File> {
    std::fs::create_dir_all(directory).expect("the scratch directory creates");
    let whole = shards.concat();
    let path = directory.join("app.log.gz");
    write_leaf(&path, &whole, true);
    Corpus {
        handle: File::new(&path).expect("the scratch leaf is addressable"),
        decoded: whole.len() as u64,
        rows: records,
        lines: spanned_lines(records),
    }
}

/// The fixture root for this run, unique per process and removed at the end.
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("yggdryl-bench-lines-{}", std::process::id()))
}

/// Drain one read, returning the rows it materialized and the lines they span.
///
/// The row count alone cannot tell folding from discarding: a projection that
/// threw every continuation line away instead of folding it into `message`
/// still yields one row per record, and would be charged for text it never
/// touched. Summing the `lines` column is what separates the two.
fn parsed_shape<H: IOBase>(handle: &H, options: &LineRecordOptions) -> (usize, i64) {
    let mut rows = 0_usize;
    let mut spanned = 0_i64;
    for batch in handle.read_arrow_lines(options).expect("a line reader") {
        let batch = batch.expect("a parsed batch");
        rows += batch.num_rows();
        let lines = batch
            .column_by_name("lines")
            .expect("the projection's lines column")
            .as_any()
            .downcast_ref::<arrow_array::Int32Array>()
            .expect("lines is int32");
        spanned += (0..lines.len())
            .map(|row| i64::from(lines.value(row)))
            .sum::<i64>();
    }
    (rows, spanned)
}

/// Prove a corpus yields exactly its records over exactly its lines.
///
/// Both halves run outside every timer. The height catches a parser that
/// split the multi-line records into one row each; the span catches one that
/// dropped their continuation lines instead of folding them.
fn proven<H: IOBase>(corpus: &Corpus<H>, options: &LineRecordOptions, label: &str) {
    let (rows, spanned) = parsed_shape(&corpus.handle, options);
    assert_eq!(
        rows, corpus.rows,
        "{label} must project one row per record, continuation lines folded"
    );
    assert_eq!(
        spanned, corpus.lines,
        "{label} must fold every continuation line into its record, not drop it"
    );
}

/// The rotated-folder projection: coding, shape, typed captures, and scale.
///
/// Fixtures are written once, outside every timer, and every case reports
/// decoded bytes - never the gzip wire size, which would describe the
/// fixture's compressibility rather than the parse.
pub(crate) fn lines_gzip_benchmarks(criterion: &mut Criterion) {
    let root = scratch();
    let _ = std::fs::remove_dir_all(&root);

    let typed = LineRecordOptions::new(ROTATED_PATTERN).expect("a valid pattern");
    // The same captures declared text: the strict cast the inferred int64
    // columns pay for is off, and the builders' own utf8 arrays are emitted.
    let text = typed
        .clone()
        .try_with_capture_types([
            ("thread_id", DataType::Utf8),
            ("latency_us", DataType::Utf8),
        ])
        .expect("the pattern names both captures");

    let shards = rotated_shards(RECORDS);
    let gzip = rotated_folder(&root.join("gzip"), &shards, RECORDS, true);
    let plain = rotated_folder(&root.join("plain"), &shards, RECORDS, false);
    let single = single_leaf(&root.join("single"), &shards, RECORDS);
    drop(shards);
    // The scale sweep stops at four points, `scale/1m` reusing the folder the
    // headline cases read: each added point costs a full pass, and the claim
    // is flat per-byte throughput, which four sizes over an eightfold range
    // carry.
    let scale: Vec<(&str, Corpus<Folder>)> =
        [("125k", 125_000), ("250k", 250_000), ("500k", 500_000)]
            .into_iter()
            .map(|(label, records)| {
                let shards = rotated_shards(records);
                (
                    label,
                    rotated_folder(&root.join(label), &shards, records, true),
                )
            })
            .collect();

    // Proven once, outside the timers: a projection that split or dropped the
    // multi-line records would still look fast.
    proven(&gzip, &typed, "folder/gzip");
    proven(&plain, &typed, "folder/plain");
    proven(&single, &typed, "single/gzip");
    proven(&gzip, &text, "casts/text");
    for (label, corpus) in &scale {
        proven(corpus, &typed, label);
    }

    let mut group = criterion.benchmark_group("lines_gzip");
    // One pass over a million records is seconds, not microseconds: ten
    // samples is the whole measurement, and a long warm-up would only repeat
    // it.
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(30));

    group.throughput(Throughput::Bytes(gzip.decoded));
    group.bench_function("folder/gzip", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip.handle), &typed));
    });
    group.throughput(Throughput::Bytes(plain.decoded));
    group.bench_function("folder/plain", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&plain.handle), &typed));
    });
    group.throughput(Throughput::Bytes(single.decoded));
    group.bench_function("single/gzip", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&single.handle), &typed));
    });

    group.throughput(Throughput::Bytes(gzip.decoded));
    group.bench_function("casts/typed", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip.handle), &typed));
    });
    group.bench_function("casts/text", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip.handle), &text));
    });

    for (label, corpus) in &scale {
        group.throughput(Throughput::Bytes(corpus.decoded));
        group.bench_function(format!("scale/{label}"), |bencher| {
            bencher.iter(|| parsed_rows(black_box(&corpus.handle), &typed));
        });
    }
    group.throughput(Throughput::Bytes(gzip.decoded));
    group.bench_function("scale/1m", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip.handle), &typed));
    });
    group.finish();

    // The fixtures are real directories, so the run removes what it built.
    let _ = std::fs::remove_dir_all(&root);
}
