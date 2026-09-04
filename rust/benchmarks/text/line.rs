//! Plain-text row decoding through the generic record-media surface.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::IOMedia;
use yggdryl::Url;
use yggdryl::generic::RecordOptions;
use yggdryl::holder::Buffer;
use yggdryl::text::TextOptions;

const ROWS: usize = crate::bench_profile::corpus(50_000, 2_000);
const ROWHEADER: &str = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)";

fn corpus() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ROWS * 40);
    for row in 0..ROWS {
        bytes.extend_from_slice(format!("[INFO] id={row} message {row}\n").as_bytes());
    }
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
    group.bench_function("record/rowheader_autotype", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&captured)));
    });
    group.finish();
}
