//! XML rows: the whole-document surface, and the positional one over it.
//!
//! The pairs are the point. A read of one row is measured beside a read of the
//! document that holds it, and a write of one row beside the overwrite that
//! replaces every row - because what an offset index buys is exactly the
//! difference between those two, and it only shows against the document size.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::xml::Xml;
use yggdryl::{DataType, Field, IOBase, IOMedia, Scalar, Url};

/// Row counts a document is measured at.
const ROWS: [u64; 3] = [64, 1_024, 16_384];

fn field() -> Field {
    DataType::from_fields([
        DataType::Utf8.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Utf8.required_field("note"),
    ])
    .unwrap()
    .required_field("row")
}

fn row(index: u64) -> Scalar {
    Scalar::from_record([
        ("id", Scalar::from(index.to_string())),
        ("symbol", Scalar::from("AAPL")),
        ("note", Scalar::from("a stored row of an ordinary width")),
    ])
    .unwrap()
}

/// A document holding `rows` rows, built through the positional append.
fn document(rows: u64) -> Xml<Buffer> {
    let handle =
        Buffer::new().with_media_type(Url::from_str("file:///rows.xml").unwrap().media_type());
    let mut media = Xml::new(handle).with_field(field());
    media
        .append_row_scalars((0..rows).map(row).collect::<Vec<_>>())
        .unwrap();
    media
}

pub fn xml_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/xml");
    for rows in ROWS {
        let media = document(rows);
        let options = media.record_options().unwrap();
        group.throughput(Throughput::Elements(rows));

        group.bench_with_input(
            BenchmarkId::new("read_arrow_reader", rows),
            &rows,
            |bencher, _| {
                bencher.iter(|| {
                    media
                        .read_arrow_reader(black_box(&options))
                        .unwrap()
                        .map(|batch| batch.unwrap().num_rows())
                        .sum::<usize>()
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("read_row_index", rows),
            &rows,
            |bencher, _| {
                bencher.iter(|| media.read_row_index().unwrap().len());
            },
        );

        // One row, however many the document holds: this is the pair above's
        // whole point, so it is measured against the same sizes.
        group.throughput(Throughput::Elements(1));
        let middle = rows / 2;
        group.bench_with_input(
            BenchmarkId::new("read_row_scalar", rows),
            &rows,
            |bencher, _| {
                bencher.iter(|| media.read_row_scalar(black_box(middle)).unwrap());
            },
        );
        group.bench_with_input(
            BenchmarkId::new("read_range_scalars_16", rows),
            &rows,
            |bencher, _| {
                bencher.iter(|| media.read_range_scalars(black_box(middle), 16).unwrap());
            },
        );

        let same = row(middle);
        let longer = Scalar::from_record([
            ("id", Scalar::from(middle.to_string())),
            ("symbol", Scalar::from("AAPL")),
            (
                "note",
                Scalar::from("a replacement whose text is longer than the row it replaces"),
            ),
        ])
        .unwrap();
        group.bench_with_input(
            BenchmarkId::new("write_row_scalar_same_length", rows),
            &rows,
            |bencher, _| {
                let mut media = document(rows);
                media.open().unwrap();
                bencher.iter(|| media.write_row_scalar(black_box(middle), &same).unwrap());
            },
        );
        group.bench_with_input(
            BenchmarkId::new("write_row_scalar_longer", rows),
            &rows,
            |bencher, _| {
                let mut media = document(rows);
                media.open().unwrap();
                let mut toggle = false;
                bencher.iter(|| {
                    toggle = !toggle;
                    let value = if toggle { &longer } else { &same };
                    media.write_row_scalar(black_box(middle), value).unwrap();
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("append_row_scalar", rows),
            &rows,
            |bencher, _| {
                let mut media = document(rows);
                media.open().unwrap();
                bencher.iter(|| media.append_row_scalars([&same]).unwrap());
            },
        );
    }
    group.finish();
}

pub fn record_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("media/xml/record");
    let rows = 4_096_u64;
    let values: Vec<Scalar> = (0..rows).map(row).collect();
    group.throughput(Throughput::Elements(rows));

    let source = document(rows);
    let options = source.record_options().unwrap();
    let batches: Vec<arrow_array::RecordBatch> = source
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    let schema = batches
        .first()
        .map(arrow_array::RecordBatch::schema)
        .unwrap();

    group.bench_function("overwrite_arrow_reader", |bencher| {
        let mut media = document(0);
        bencher.iter(|| {
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(schema.clone(), batches.clone()),
                    black_box(&options),
                )
                .unwrap();
        });
    });
    group.bench_function("append_arrow_reader", |bencher| {
        let mut media = document(0);
        bencher.iter(|| {
            media
                .append_arrow_reader(
                    yggdryl::arrow::batch_reader(schema.clone(), batches.clone()),
                    black_box(&options),
                )
                .unwrap();
        });
    });
    group.bench_function("append_row_scalars", |bencher| {
        let mut media = document(0);
        media.open().unwrap();
        bencher.iter(|| media.append_row_scalars(black_box(&values)).unwrap());
    });
    group.bench_function("row_size", |bencher| {
        let media = document(rows);
        bencher.iter(|| media.row_size().unwrap());
    });
    group.bench_function("read_arrow_field", |bencher| {
        let media = Xml::new(document(rows).into_handle());
        let options = RecordOptions::for_mime_type(&yggdryl::MimeType::XML).unwrap();
        bencher.iter(|| media.read_arrow_field(black_box(&options)).unwrap());
    });
    group.finish();
}
