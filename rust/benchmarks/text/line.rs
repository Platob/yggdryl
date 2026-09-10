//! Plain-text row decoding through the generic record-media surface.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::IOMedia;
use yggdryl::Url;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::{TextOptions, into_arrow_batch, read_text_lines};

const ROWS: usize = crate::bench_profile::corpus(10_000, 500);
const MULTILINE_ROWS: usize = crate::bench_profile::corpus(4_000, 200);
const OVERSIZED_BODY_BYTES: usize = crate::bench_profile::corpus(2 * 1024 * 1024, 64 * 1024);
const RECORD_BYTE_LIMIT: u64 = 4 * 1024;
const ROWHEADER: &str = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+)";

fn corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ROWS * 40);
    for row in 0..ROWS {
        bytes.extend_from_slice(format!("[INFO] id={row} message {row}\n").as_bytes());
    }
    bytes
}

fn multiline_corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MULTILINE_ROWS * 96);
    for row in 0..MULTILINE_ROWS {
        bytes.extend_from_slice(format!("[INFO] id={row} message {row}\n").as_bytes());
        bytes.extend_from_slice(b"  first continuation\n");
        bytes.extend_from_slice(b"  second continuation\n");
    }
    bytes
}

fn oversized_corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(OVERSIZED_BODY_BYTES + 64);
    bytes.extend_from_slice(b"[INFO] id=0 oversized\n");
    bytes.resize(bytes.len() + OVERSIZED_BODY_BYTES, b'x');
    // The next header makes draining the discarded suffix observable.
    bytes.extend_from_slice(b"\n[INFO] id=1 after drain\n");
    bytes
}

fn handle(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(
        Url::from_str("file:///bench.log")
            .expect("a URL")
            .media_type(),
    )
}

fn options(rowheader: Option<&str>) -> RecordOptions {
    let options = match rowheader {
        Some(rowheader) => TextOptions::new()
            .try_with_rowheader(rowheader)
            .expect("a row-header regex"),
        None => TextOptions::new(),
    };
    options.into()
}

fn framed_options(framing: bool, max_record_byte_size: Option<u64>) -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(ROWHEADER)
        .expect("a row-header regex");
    options.set_framing(framing);
    options.set_max_record_byte_size(max_record_byte_size);
    options.into()
}

fn drain(handle: &Buffer, options: &RecordOptions) -> usize {
    handle
        .read_arrow_reader(options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum()
}

pub(crate) fn text_options_benchmarks(criterion: &mut Criterion) {
    let options = options(Some(ROWHEADER));
    let mut group = criterion.benchmark_group("text_options");
    group.bench_function("datatype_from_regex", |bencher| {
        bencher.iter(|| {
            yggdryl::DataType::from_regex(black_box(ROWHEADER), true).expect("a capture Struct")
        });
    });
    group.bench_function("stable_hash", |bencher| {
        bencher.iter(|| black_box(&options).stable_hash());
    });
    group.finish();
}

pub(crate) fn text_records_benchmarks(criterion: &mut Criterion) {
    let bytes = corpus();
    let source = handle(bytes.clone());
    let plain = options(None);
    let captured = options(Some(ROWHEADER));
    assert_eq!(drain(&source, &plain), ROWS);
    assert_eq!(drain(&source, &captured), ROWS);

    let baseline = regex::bytes::Regex::new(ROWHEADER).expect("a row-header regex");
    let mut group = criterion.benchmark_group("text_records");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("baseline/regex_rows", |bencher| {
        bencher.iter(|| {
            bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .filter(|line| baseline.captures(black_box(line)).is_some())
                .count()
        });
    });
    group.bench_function("record/plain", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&plain)));
    });
    group.bench_function("record/rowheader_regex_types", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&captured)));
    });
    group.finish();

    let short_physical = framed_options(false, None);
    let short_framed = framed_options(true, None);
    let multiline_bytes = multiline_corpus();
    let multiline_source = handle(multiline_bytes.clone());
    let multiline_physical = framed_options(false, None);
    let multiline_framed = framed_options(true, None);
    let oversized_bytes = oversized_corpus();
    let oversized_source = handle(oversized_bytes.clone());
    let oversized_physical = framed_options(false, Some(RECORD_BYTE_LIMIT));
    let oversized_framed = framed_options(true, Some(RECORD_BYTE_LIMIT));

    assert_eq!(drain(&source, &short_physical), ROWS);
    assert_eq!(drain(&source, &short_framed), ROWS);
    assert_eq!(
        drain(&multiline_source, &multiline_physical),
        MULTILINE_ROWS * 3
    );
    assert_eq!(drain(&multiline_source, &multiline_framed), MULTILINE_ROWS);
    assert_eq!(drain(&oversized_source, &oversized_physical), 3);
    assert_eq!(drain(&oversized_source, &oversized_framed), 2);

    let mut framing = criterion.benchmark_group("text_record_framing");
    framing.throughput(Throughput::Bytes(bytes.len() as u64));
    framing.bench_function("short/physical", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&short_physical)));
    });
    framing.bench_function("short/framed", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&short_framed)));
    });

    framing.throughput(Throughput::Bytes(multiline_bytes.len() as u64));
    framing.bench_function("multiline/physical", |bencher| {
        bencher.iter(|| drain(black_box(&multiline_source), black_box(&multiline_physical)));
    });
    framing.bench_function("multiline/framed", |bencher| {
        bencher.iter(|| drain(black_box(&multiline_source), black_box(&multiline_framed)));
    });

    framing.throughput(Throughput::Bytes(oversized_bytes.len() as u64));
    framing.bench_function("oversized/physical", |bencher| {
        bencher.iter(|| drain(black_box(&oversized_source), black_box(&oversized_physical)));
    });
    framing.bench_function("oversized/framed", |bencher| {
        bencher.iter(|| drain(black_box(&oversized_source), black_box(&oversized_framed)));
    });
    framing.finish();
}

/// The decode path on its own, and the two halves it is built from.
///
/// A line drain measures decoding with no Arrow cost at all; a batch build over
/// lines already decoded measures the Arrow cost with no decode cost. Together
/// they say which half a change in the whole-read number came from.
pub(crate) fn text_line_benchmarks(criterion: &mut Criterion) {
    let bytes = corpus();
    let source = handle(bytes.clone());
    let plain = TextOptions::new();
    let captured = TextOptions::new()
        .try_with_rowheader(ROWHEADER)
        .expect("a row-header regex");
    let mut classified = TextOptions::new();
    classified.parse_mimetype = true;

    let pairs = pair_corpus();
    let pair_source = handle(pairs.clone());
    let lifted = TextOptions::new()
        .try_with_lift_names(["55"])
        .expect("the path parses");
    let unlifted = TextOptions::new();

    assert_eq!(drain_lines(&source, &plain), ROWS);
    assert_eq!(drain_lines(&source, &captured), ROWS);

    let mut group = criterion.benchmark_group("text_lines");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode/plain", |bencher| {
        bencher.iter(|| drain_lines(black_box(&source), black_box(&plain)));
    });
    group.bench_function("decode/captured", |bencher| {
        bencher.iter(|| drain_lines(black_box(&source), black_box(&captured)));
    });
    group.bench_function("decode/classified", |bencher| {
        bencher.iter(|| drain_lines(black_box(&source), black_box(&classified)));
    });

    group.throughput(Throughput::Bytes(pairs.len() as u64));
    // What materializing the entry tree actually costs, against the same read
    // that never asks for one.
    group.bench_function("entries/none", |bencher| {
        bencher.iter(|| drain_lines(black_box(&pair_source), black_box(&unlifted)));
    });
    group.bench_function("entries/lifted", |bencher| {
        bencher.iter(|| drain_lines(black_box(&pair_source), black_box(&lifted)));
    });
    group.finish();

    let decoded: Vec<_> = read_text_lines(&source, &plain)
        .expect("a settled configuration")
        .map(|line| line.expect("a line"))
        .collect();
    let mut building = criterion.benchmark_group("text_batch");
    building.throughput(Throughput::Elements(decoded.len() as u64));
    building.bench_function("build/plain", |bencher| {
        bencher.iter(|| {
            into_arrow_batch(black_box(decoded.clone()), black_box(&plain)).expect("a batch")
        });
    });
    building.finish();
}

/// Lines carrying key/value pairs, which is what an entry tree is read from.
fn pair_corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ROWS * 48);
    for row in 0..ROWS {
        bytes
            .extend_from_slice(format!("8=FIX.4.2|35=D|49=SENDER|55=SYM{row}|10=001\n").as_bytes());
    }
    bytes
}

/// Decode every line and count them, touching no Arrow array.
fn drain_lines(handle: &Buffer, options: &TextOptions) -> usize {
    read_text_lines(handle, options)
        .expect("a settled configuration")
        .fold(0, |seen, line| {
            // Touch the body so the decode is not optimized away.
            seen + line.expect("a line").body().len().min(1)
        })
}
