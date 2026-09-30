//! CSV: the document a `.csv` handle holds, written from and read back into
//! record batches - plain, quoted-heavy and gzip by name - under a declared
//! field and under the header and the sample alone.
//!
//! The fixture is the small market row the XMLA benchmark uses - an
//! identifier, a nullable symbol, a price and a flag - so the numbers
//! describe a table a caller exports, not a synthetic best case.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::csv::{CsvOptions, overwrite_arrow_reader, read_batch_reader};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Field, IOBase, MediaType, Scalar, Serie, StructType};

use crate::bench_profile::corpus;

/// The record field every benchmark row is laid out under.
fn field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// `rows` representative rows as one record column; `quoted` spells every
/// symbol with the separator inside so each is quoted on the way out.
fn batch(field: &Field, rows: usize, quoted: bool) -> Serie {
    Serie::from_scalars(
        field.clone(),
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                if index % 5 == 0 {
                    Scalar::Null
                } else if quoted {
                    Scalar::from(format!("SYM,{index:04} \"quoted\""))
                } else {
                    Scalar::from(format!("SYM{index:04}"))
                },
                Scalar::from(index as f64 * 0.25),
                Scalar::from(index % 2 == 0),
            ])
        }),
    )
    .expect("rows under the field")
}

fn written(name: &str, field: &Field, rows: usize, quoted: bool) -> Buffer {
    let batch = batch(field, rows, quoted)
        .into_arrow_batch()
        .expect("a batch");
    let mut held = Buffer::new().with_media_type(MediaType::from_file_name(name));
    overwrite_arrow_reader(
        &mut held,
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &CsvOptions::new(),
    )
    .expect("the document writes");
    held
}

fn read_rows(held: &Buffer, field: Option<&Field>) -> usize {
    read_batch_reader(held, field, &CsvOptions::new())
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum()
}

pub(crate) fn csv_benchmarks(criterion: &mut Criterion) {
    let rows = corpus(10_000, 64);
    let field = field();
    let shapes = [
        ("plain", "trades.csv", false),
        ("quoted", "quoted.csv", true),
        ("gzip", "trades.csv.gz", false),
    ];
    for (label, name, quoted) in shapes {
        let batch = batch(&field, rows, quoted)
            .into_arrow_batch()
            .expect("a batch");
        let held = written(name, &field, rows, quoted);
        // Proven once outside the timers: the document round-trips.
        assert_eq!(read_rows(&held, Some(&field)), rows);
        assert_eq!(read_rows(&held, None), rows);

        let mut group = criterion.benchmark_group(format!("media/csv/{label}"));
        group.throughput(Throughput::Bytes(held.size()));
        group.bench_function("write", |bencher| {
            bencher.iter(|| {
                let mut sink = Buffer::new().with_media_type(MediaType::from_file_name(name));
                overwrite_arrow_reader(
                    &mut sink,
                    yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                    black_box(&CsvOptions::new()),
                )
                .expect("writes");
                sink
            });
        });
        group.bench_function("read_declared", |bencher| {
            bencher.iter(|| read_rows(black_box(&held), Some(&field)));
        });
        group.bench_function("read_inferred", |bencher| {
            bencher.iter(|| read_rows(black_box(&held), None));
        });
        group.bench_function("row_size", |bencher| {
            bencher.iter(|| {
                yggdryl::IOMedia::row_size(&yggdryl::csv::Csv::new(black_box(&held).clone()))
                    .expect("a count")
            });
        });
        group.finish();
    }
}
