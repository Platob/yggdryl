//! Delimited text: the cell scan, the render, and what positional access buys.
//!
//! The positional group is the one that answers a design question rather than
//! reporting a rate: reaching one row through the sparse index against reading
//! the resource to find it, on the same bytes.

use std::fmt::Write as _;
use std::hint::black_box;

use criterion::Criterion;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::csv::{Csv, CsvOptions};
use yggdryl::{IOBase, IOMedia, Scalar, Url};

/// Rows per fixture, large enough that the index stride is not one.
pub(crate) const ROWS: usize = crate::bench_profile::corpus(65_536, 2_048);

/// A handle whose media type names CSV, so the encoding is never guessed.
fn handle(name: &str, bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("a valid benchmark URL")
            .media_type(),
    )
}

/// Four columns: an integer, a symbol, a double, and a date.
fn plain_fixture() -> Buffer {
    let mut text = String::with_capacity(ROWS * 48);
    text.push_str("id,symbol,price,traded_on\n");
    for row in 0..ROWS {
        writeln!(
            text,
            "{row},SYM{:04},{}.25,2024-01-{:02}",
            row % 1_000,
            row % 500,
            row % 28 + 1
        )
        .expect("writing to a String cannot fail");
    }
    handle("bench-plain.csv", text.into_bytes())
}

/// The same shape with every cell quoted and one embedded separator, so the
/// scan pays for the escape path rather than the borrowed one.
fn quoted_fixture() -> Buffer {
    let mut text = String::with_capacity(ROWS * 64);
    text.push_str("id,symbol,note,traded_on\n");
    for row in 0..ROWS {
        writeln!(
            text,
            "\"{row}\",\"SYM{:04}\",\"held, then \"\"sold\"\"\",\"2024-01-{:02}\"",
            row % 1_000,
            row % 28 + 1
        )
        .expect("writing to a String cannot fail");
    }
    handle("bench-quoted.csv", text.into_bytes())
}

/// Drain the ordinary Arrow path, which is what a caller actually pays.
fn decoded_rows(source: &impl IOBase, options: &RecordOptions) -> u64 {
    source
        .read_arrow_reader(options)
        .expect("the benchmark fixture must decode")
        .map(|batch| batch.expect("the benchmark batch must decode").num_rows() as u64)
        .sum()
}

pub(crate) fn read_benchmarks(criterion: &mut Criterion) {
    let plain = plain_fixture();
    let quoted = quoted_fixture();
    let typed: RecordOptions = CsvOptions::new().into();
    let text_only: RecordOptions = CsvOptions::new().with_autotype(false).into();
    assert_eq!(decoded_rows(&plain, &typed), ROWS as u64);

    let mut group = criterion.benchmark_group("csv_read");
    group.sample_size(10);
    group.bench_function("plain/read_rows/typed", |bencher| {
        bencher.iter(|| black_box(decoded_rows(black_box(&plain), &typed)));
    });
    group.bench_function("plain/read_rows/text", |bencher| {
        bencher.iter(|| black_box(decoded_rows(black_box(&plain), &text_only)));
    });
    group.bench_function("quoted/read_rows/text", |bencher| {
        bencher.iter(|| black_box(decoded_rows(black_box(&quoted), &text_only)));
    });
    group.bench_function("plain/row_size", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&plain)
                    .row_size()
                    .expect("the fixture has a row count"),
            );
        });
    });
    group.finish();
}

pub(crate) fn inference_benchmarks(criterion: &mut Criterion) {
    let plain = plain_fixture();
    let options = CsvOptions::new();
    let sampled = CsvOptions::new().with_infer_row_size(64);
    let declared = CsvOptions::new().with_autotype(false);

    let mut group = criterion.benchmark_group("csv_inference");
    group.sample_size(10);
    for (name, options) in [
        ("default_sample", &options),
        ("small_sample", &sampled),
        ("no_typing", &declared),
    ] {
        group.bench_function(format!("read_field/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(
                    yggdryl::media::csv::read_field(black_box(&plain), options)
                        .expect("the fixture resolves its columns"),
                );
            });
        });
    }
    group.finish();
}

pub(crate) fn write_benchmarks(criterion: &mut Criterion) {
    let plain = plain_fixture();
    let options: RecordOptions = CsvOptions::new().into();
    let batches = plain
        .read_arrow_reader(&options)
        .expect("the fixture decodes")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("every benchmark batch decodes");
    let schema = batches[0].schema();

    let mut group = criterion.benchmark_group("csv_write");
    group.sample_size(10);
    group.bench_function("overwrite_rows", |bencher| {
        bencher.iter(|| {
            let mut target = handle("bench-write.csv", Vec::new());
            target
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(schema.clone(), batches.clone()),
                    &options,
                )
                .expect("the rendered rows must write");
            black_box(target.size());
        });
    });
    group.bench_function("append_rows", |bencher| {
        bencher.iter_batched(
            plain_fixture,
            |mut target| {
                target
                    .append_arrow_reader(
                        yggdryl::arrow::batch_reader(schema.clone(), batches.clone()),
                        &options,
                    )
                    .expect("the appended rows must write");
                black_box(target.size())
            },
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}

pub(crate) fn positional_benchmarks(criterion: &mut Criterion) {
    let fresh = Csv::new(plain_fixture());
    let mut opened = Csv::new(plain_fixture());
    opened.open().expect("the CSV media opens");
    let last = ROWS as u64 - 1;
    assert_eq!(
        opened
            .read_cell_text(last, 0)
            .expect("the final row is read"),
        Some(last.to_string())
    );

    let mut group = criterion.benchmark_group("csv_positional");
    group.sample_size(10);
    group.bench_function("read_cell/fresh", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&fresh)
                    .read_cell_text(black_box(last), 1)
                    .expect("a fresh positional read"),
            );
        });
    });
    group.bench_function("read_cell/opened", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&opened)
                    .read_cell_text(black_box(last), 1)
                    .expect("an indexed positional read"),
            );
        });
    });
    group.bench_function("read_row/opened", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&opened)
                    .read_row_scalar(black_box(last))
                    .expect("an indexed row read"),
            );
        });
    });
    group.bench_function("write_cell/same_width", |bencher| {
        bencher.iter_batched(
            || Csv::new(plain_fixture()),
            |mut media| {
                media
                    .write_cell_text(0, 1, "SYM9999")
                    .expect("a same-width positional write");
                black_box(media.handle().size())
            },
            criterion::BatchSize::SmallInput,
        );
    });
    group.bench_function("write_cell/widened", |bencher| {
        bencher.iter_batched(
            || Csv::new(plain_fixture()),
            |mut media| {
                media
                    .write_cell_scalar(0, 0, &Scalar::from(1_000_000_000_i64))
                    .expect("a widening positional write");
                black_box(media.handle().size())
            },
            criterion::BatchSize::SmallInput,
        );
    });
    group.finish();
}
