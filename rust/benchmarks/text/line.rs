//! The Arrow line projection: parse throughput and the cost of its hash.
//!
//! The corpus is anonymized production-shaped OMS log lines -
//! `2026-08-14 00:05:01.167_250 [250-<hex>:<hex>:72503] [OrderFlow_Enrichment]
//! (DEBUG) message` - the `[thread] [logger] (LEVEL)` header real trading
//! systems write. One record spec and **one pattern, spelled identically in
//! Rust, Python, and JavaScript** (`(?P<name>...)` groups, which both engines
//! read), generates every corpus in every language. Parse cases report the
//! *decoded* payload as byte throughput - for the gzip case too, because
//! decoded text is what the parser actually consumes - so the two numbers
//! answer "what does the coding cost" rather than mixing wire sizes. The
//! hash cases isolate the stable FNV-1a fold the `hash` column pays, on the
//! same messages the parse hashes: hashing measured beside the whole parse is
//! what justifies keeping the dependency-free hash (regex and UTF-8 dominate;
//! the recorded numbers in `docs/io.md` say by how much).
//!
//! `lines_gzip` measures the same projection over the production shape:
//! rotated log files on local storage, 200k records split across eight
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
//!   pulls about a ninth of it and inflates. The delta is inflate work minus
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
//!   `port` infers `int64` from its `\d+` sub-pattern, so the default read
//!   casts every closing batch onto the declared root; `casts/text` declares
//!   it `utf8`, which turns the cast off and leaves the text the builders
//!   already produced. The difference is the price of handing a consumer a
//!   typed column instead of strings it would convert itself.
//! - `scale/25k` through `scale/200k` is the same gzip folder at four sizes.
//!   Per-byte throughput staying flat across them is the evidence that
//!   nothing in the projection is quadratic. It is not evidence about
//!   residency: a reader that buffered the whole decoded corpus before
//!   emitting a batch would look just as flat, buffering being linear too.
//!   These cases time wall clock and nothing else, so the streaming claim -
//!   one batch, not one corpus, held at a time - is not measured here; it
//!   needs a peak-RSS probe across the sizes, which Criterion does not
//!   report. `scale/200k` and `casts/typed` repeat
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
use yggdryl::io::{Buffer, IOBase};
use yggdryl::local::{File, Folder};
use yggdryl::text::{TextLineOptions, stable_hash_bytes};
use yggdryl::{DataType, Url};

/// Log lines per small corpus: enough that per-read setup is noise.
const LINES: usize = 50_000;

/// The shared header pattern, **byte-identical in every language**.
///
/// `(?P<name>...)` is the spelling both CPython's `re` and the Rust engine
/// read, so the Python baseline compiles exactly this string. `port` is the
/// one capture whose whole body is `\d+`, which the closed inference table
/// types `int64`.
const PATTERN: &str = r"^(?P<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}_\d{3}) \[(?P<thread>\d+-[^\]]*:(?P<port>\d+))\] \[(?P<logger>[^\]]+)\] \((?P<level>[A-Z]+)\)";

/// Append one record of the shared corpus spec to `text`.
///
/// The anonymized production shape: a microsecond timestamp with the `_`
/// separator, a pool-and-hex thread id, a bracketed logger, a parenthesized
/// level, and one of eight message shapes. `index` drives every varying part
/// through the same arithmetic in each language, so the three generators stay
/// byte-for-byte identical. With `continuations`, every fiftieth record
/// carries two continuation lines that belong to the same row: the multi-line
/// shape a line-counting loop miscounts and the projection folds.
fn record(text: &mut String, index: usize, continuations: bool) {
    use std::fmt::Write as _;

    let (minute, second) = (index / 3_600 % 60, index / 60 % 60);
    let micro = index % 1_000_000;
    let (ms, us) = (micro / 1_000, micro % 1_000);
    let pool = 250 + index % 4;
    let hex_a = (index as u64).wrapping_mul(2_654_435_761) % 4_294_967_296;
    let hex_b = index * 40_503 % 65_536;
    let port = 72_500 + index % 8;
    let logger = [
        "OrderFlow_Enrichment",
        "Regulatory_Timestamps",
        "GatewayBridge",
        "OrderFlow",
        "RiskManager",
        "MarketDataManager",
        "TagWrapper",
        "RouteCheck",
    ][index % 8];
    let level = ["DEBUG", "INFO", "WARNING"][index % 3];
    let message = match index % 8 {
        0 => format!(
            "-> [S] (trade || cancel || tradecancel || replace || new) - ExecType=required, \
             cumQty={}, CompositeID=null",
            index % 100
        ),
        1 => format!("CLIENTID set to ROUTE{:02}", index % 50),
        2 => format!(
            "After Enrichment -> #ROUTINGINDICATOR=yes #CFICODE=ESXXXX #GROUP=GRP{} \
             #ISINCODE=XX{:010}",
            index % 9,
            index % 10_000
        ),
        3 => format!(
            "Message received: Message type [executionreport] from [gateway as FU{:06}] \
             forwarded to [(null) as (null)] [Direct reject]",
            index % 1_000_000
        ),
        4 => String::from(
            "Message rejected because : Ignoring expiry message from fully filled orders",
        ),
        5 => format!(
            "Setting last event id for order , 1 to 20260814-2206{:02}-906-02-1",
            index % 100
        ),
        6 => String::from(
            "Expression from TCRPRICE=xpath(\"/event/action/trade/capturereport/@price\") gives \
             no result, no mapping is done",
        ),
        _ => format!(
            "Found code(db: XX{0:010}_XNAS_USD) from instrument(db: XX{0:010} XNAS USD)",
            index % 10_000
        ),
    };
    writeln!(
        text,
        "2026-08-14 00:{minute:02}:{second:02}.{ms:03}_{us:03} \
         [{pool}-{hex_a:08x}:{hex_b:04x}:{port}] [{logger}] ({level}) {message}",
    )
    .expect("a string write cannot fail");
    if continuations && index % 50 == 49 {
        text.push_str("    at core::enrich(order.rs:118)\n    at core::route(order.rs:64)\n");
    }
}

/// One session of the shared corpus, single-line records only.
fn corpus() -> String {
    let mut text = String::with_capacity(LINES * 176);
    for index in 0..LINES {
        record(&mut text, index, false);
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
fn parsed_rows<H: IOBase>(handle: &H, options: &TextLineOptions) -> usize {
    handle
        .read_arrow_lines(options)
        .expect("a line reader")
        .map(|batch| batch.expect("a parsed batch").num_rows())
        .sum()
}

pub(crate) fn lines_arrow_benchmarks(criterion: &mut Criterion) {
    let options = TextLineOptions::with_pattern(PATTERN).expect("a valid pattern");
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
            let mut records = black_box(&plain)
                .read_lines_matching(PATTERN)
                .expect("a record reader");
            let mut bytes = 0_usize;
            // Byte-first, and borrowed: the grouping stage never validates
            // UTF-8 and never allocates a record, so this measures the split
            // and nothing else.
            while let Some(record) = records.next() {
                bytes += record.expect("a record").bytes().len();
            }
            bytes
        });
    });

    // The exact messages the parse hashes: header stripped, trimmed. Hashing
    // them alone is the parse's hash cost, measured over the same payload.
    let messages: Vec<&str> = text
        .lines()
        // The first `) ` in a line closes the parenthesized level, so what
        // follows is the message the projection hashes.
        .map(|line| line.split_once(") ").expect("a message tail").1.trim())
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
        let message: String = "CLIENTID set to ROUTE42 ExecType=required cumQty=45 "
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
///
/// Small enough that the whole group runs in well under a minute, large
/// enough that per-read setup is noise; the same count the Python and
/// JavaScript targets read, so the three languages' numbers describe the
/// same work.
const RECORDS: usize = 200_000;

/// Rotated leaves one folder corpus is split across.
const LEAVES: usize = 8;

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

/// The [`LEAVES`] rotated payloads holding `records` records between them.
fn rotated_shards(records: usize) -> Vec<String> {
    assert!(
        records % LEAVES == 0,
        "the corpus splits evenly into leaves"
    );
    let per_leaf = records / LEAVES;
    (0..LEAVES)
        .map(|leaf| {
            let mut text = String::with_capacity(per_leaf * 176);
            for index in leaf * per_leaf..(leaf + 1) * per_leaf {
                record(&mut text, index, true);
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
fn parsed_shape<H: IOBase>(handle: &H, options: &TextLineOptions) -> (usize, i64) {
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
fn proven<H: IOBase>(corpus: &Corpus<H>, options: &TextLineOptions, label: &str) {
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

    let typed = TextLineOptions::with_pattern(PATTERN).expect("a valid pattern");
    // The same capture declared text: the strict cast the inferred int64
    // column pays for is off, and the builders' own utf8 arrays are emitted.
    let text = typed
        .clone()
        .try_with_capture_types([("port", DataType::Utf8)])
        .expect("the pattern names the capture");

    let shards = rotated_shards(RECORDS);
    let gzip = rotated_folder(&root.join("gzip"), &shards, RECORDS, true);
    let plain = rotated_folder(&root.join("plain"), &shards, RECORDS, false);
    let single = single_leaf(&root.join("single"), &shards, RECORDS);
    drop(shards);
    // The scale sweep stops at four points, `scale/200k` reusing the folder
    // the headline cases read: each added point costs a full pass, and the
    // claim is flat per-byte throughput, which four sizes over an eightfold
    // range carry.
    let scale: Vec<(&str, Corpus<Folder>)> = [("25k", 25_000), ("50k", 50_000), ("100k", 100_000)]
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
    // One pass over the corpus is hundreds of milliseconds, not
    // microseconds: ten samples is the whole measurement, and a long warm-up
    // would only repeat it.
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(10));

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
    group.bench_function("scale/200k", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&gzip.handle), &typed));
    });
    group.finish();

    // The fixtures are real directories, so the run removes what it built.
    let _ = std::fs::remove_dir_all(&root);
}

/// Byte-sized batching against fixed-row batching on uneven records.
///
/// The corpus mixes short lines with ~2 KB stack traces in the same file,
/// which is what makes the two bounds behave differently: a row bound produces
/// batches whose sizes swing widely, while a byte bound
/// produces comparably sized ones whatever the records look like. The cases
/// report the batch-size spread alongside the timing, because the *point* of
/// byte sizing is the spread rather than the speed.
///
/// `detect/*` is the ninth task's comparison: timestamp-anchored detection
/// against the equivalent anchored regex, on the same corpus. Detection has no
/// expression to compile or run, and its cheap first-byte guard rejects a
/// continuation line in one byte - `docs/io.md` records by how much
/// that pays, or whether it does.
///
/// `zone/*` isolates what reading a naive timestamp in a zone costs: unset
/// against a fixed offset against a DST-observing named zone. The offset cache
/// is what should keep the third close to the second, so a regression there
/// shows up as the third pulling away.
pub(crate) fn lines_shape_benchmarks(criterion: &mut Criterion) {
    use yggdryl::text::{Opening, Strip};

    // A corpus whose record sizes swing by an order of magnitude.
    let mut text = String::new();
    for index in 0..20_000_usize {
        record(&mut text, index, true);
        if index % 25 == 0 {
            // A stack trace: ~2 KB in one record, against short neighbours.
            for frame in 0..40 {
                text.push_str("\tat com.example.service.Handler.invoke(Handler.java:");
                text.push_str(&frame.to_string());
                text.push_str(")\n");
            }
        }
    }
    let handle = handle("uneven.log", text.as_bytes());
    let decoded = text.len() as u64;

    let base = TextLineOptions::with_pattern(PATTERN).expect("a valid pattern");
    let mut group = criterion.benchmark_group("lines_shape");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Bytes(decoded));

    // The spread each bound produces, measured once and reported in the docs.
    for (label, options) in [
        ("rows", base.clone().with_batch_size(1_024)),
        ("bytes", base.clone().with_byte_size(1 << 20)),
    ] {
        let sizes: Vec<usize> = handle
            .read_arrow_lines(&options)
            .expect("a reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .collect();
        let widest = sizes.iter().copied().max().unwrap_or(0);
        let narrowest = sizes.iter().copied().min().unwrap_or(0);
        eprintln!(
            "batching/{label}: {} batches, rows {narrowest}..={widest}",
            sizes.len()
        );
        group.bench_function(format!("batching/{label}"), |bencher| {
            bencher.iter(|| parsed_rows(black_box(&handle), &options));
        });
    }

    // Detection against the equivalent regex, on the same corpus.
    let detection = TextLineOptions::new()
        .try_with_opening(Opening::Timestamp)
        .expect("timestamp detection");
    let anchored = TextLineOptions::with_pattern(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*")
        .expect("a valid pattern");
    group.bench_function("detect/timestamp", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&handle), &detection));
    });
    group.bench_function("detect/regex", |bencher| {
        bencher.iter(|| parsed_rows(black_box(&handle), &anchored));
    });

    // What a zone costs a naive timestamp: none, fixed, DST-observing.
    for (label, zone) in [
        ("naive", None),
        ("fixed", Some("+02:00")),
        ("named", Some("Europe/Paris")),
    ] {
        let options = match zone {
            Some(zone) => base
                .clone()
                .try_with_timezone(zone.parse().expect("a known zone"))
                .expect("a zone the registry knows"),
            None => base.clone(),
        };
        group.bench_function(format!("zone/{label}"), |bencher| {
            bencher.iter(|| parsed_rows(black_box(&handle), &options));
        });
    }

    // What the strip options cost, since they are span arithmetic rather than
    // an allocation: `none` should be indistinguishable from the default.
    for (label, strip) in [("whitespace", Strip::Whitespace), ("none", Strip::None)] {
        let options = base
            .clone()
            .with_lstrip(strip.clone())
            .with_rstrip(strip.clone());
        group.bench_function(format!("strip/{label}"), |bencher| {
            bencher.iter(|| parsed_rows(black_box(&handle), &options));
        });
    }
    group.finish();
}
