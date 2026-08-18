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

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::Url;
use yggdryl::io::{Buffer, IOBase, LineRecordOptions};
use yggdryl::text::stable_hash_bytes;

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
fn parsed_rows(handle: &Buffer, options: &LineRecordOptions) -> usize {
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
