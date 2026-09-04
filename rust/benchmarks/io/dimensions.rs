//! Whole-media dimensions: cold metadata traversal, opened-session cache, and
//! the full row decode each metadata counter is meant to avoid.

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{BinaryArray, RecordBatch};
use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion};
use yggdryl::DataType;
use yggdryl::avro::Avro;
use yggdryl::generic::RecordOptions;
use yggdryl::io::Buffer;
use yggdryl::ipc::Ipc;
use yggdryl::parquet::Parquet;
use yggdryl::{IOBase, IOMedia};

use super::{ROWS, batch, handle, stored_with};

/// Drain the ordinary Arrow path so each format's metadata row counter has a
/// directly comparable decode baseline over the same encoded value.
fn decoded_rows<M: IOMedia + ?Sized>(media: &M, options: &RecordOptions) -> u64 {
    media
        .read_arrow_reader(options)
        .expect("the benchmark fixture must decode")
        .map(|batch| batch.expect("the benchmark batch must decode").num_rows() as u64)
        .sum()
}

/// Register the same metadata and dimension cases for one stateful media wrapper.
fn media_cases<M>(group: &mut BenchmarkGroup<'_, WallTime>, format: &str, fresh: M, mut opened: M)
where
    M: IOBase,
{
    let options = fresh
        .record_options()
        .expect("the benchmark encoding has record options");
    let expected_rows = fresh.row_size().expect("the fixture has a row count");
    let expected_columns = fresh.column_size().expect("the fixture has a column count");
    assert_eq!(expected_rows, ROWS as u64);
    assert_eq!(decoded_rows(&fresh, &options), expected_rows);

    opened.open().expect("the benchmark media opens");
    assert_eq!(
        opened.row_size().expect("the opened row count"),
        expected_rows
    );
    assert_eq!(
        opened.column_size().expect("the opened column count"),
        expected_columns
    );

    group.bench_function(format!("{format}/row_size/fresh"), |bencher| {
        bencher.iter(|| {
            black_box(black_box(&fresh).row_size().expect("a fresh row count"));
        });
    });
    group.bench_function(format!("{format}/row_size/opened"), |bencher| {
        bencher.iter(|| {
            black_box(black_box(&opened).row_size().expect("a cached row count"));
        });
    });
    group.bench_function(format!("{format}/column_size/fresh"), |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&fresh)
                    .column_size()
                    .expect("a fresh column count"),
            );
        });
    });
    group.bench_function(format!("{format}/column_size/opened"), |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&opened)
                    .column_size()
                    .expect("a cached column count"),
            );
        });
    });
    group.bench_function(format!("{format}/record_options"), |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&fresh)
                    .record_options()
                    .expect("the encoding has record options"),
            );
        });
    });
    group.bench_function(format!("{format}/is_io"), |bencher| {
        bencher.iter(|| black_box(black_box(&fresh).is_io()));
    });
    group.bench_function(format!("{format}/read_arrow_field/fresh"), |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&fresh)
                    .read_arrow_field(&options)
                    .expect("the fresh field is readable"),
            );
        });
    });
    group.bench_function(format!("{format}/read_arrow_field/opened"), |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&opened)
                    .read_arrow_field(&options)
                    .expect("the opened field is readable"),
            );
        });
    });
    group.bench_function(format!("{format}/read_rows"), |bencher| {
        bencher.iter(|| black_box(decoded_rows(black_box(&fresh), &options)));
    });
}

/// Measure the complete Parquet footer projection and its binding-ready Scalar.
fn parquet_statistics_cases(
    group: &mut BenchmarkGroup<'_, WallTime>,
    fresh: Parquet<Buffer>,
    mut opened: Parquet<Buffer>,
    geospatial: Parquet<Buffer>,
) {
    opened.open().expect("the Parquet fixture opens");
    group.bench_function("parquet/read_statistics/fresh", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&fresh)
                    .read_parquet_statistics()
                    .expect("fresh footer statistics"),
            );
        });
    });
    group.bench_function("parquet/read_statistics/opened", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&opened)
                    .read_parquet_statistics()
                    .expect("cached footer statistics"),
            );
        });
    });
    let statistics = opened
        .read_parquet_statistics()
        .expect("binding projection fixture");
    group.bench_function("parquet/statistics_into_value", |bencher| {
        bencher.iter(|| black_box(yggdryl::Scalar::from(black_box(statistics.clone()))));
    });
    group.bench_function("parquet/read_geospatial_statistics", |bencher| {
        bencher.iter(|| {
            black_box(
                black_box(&geospatial)
                    .read_parquet_geospatial_statistics("shape")
                    .expect("projected WKB statistics"),
            );
        });
    });
    let geospatial_statistics = geospatial
        .read_parquet_geospatial_statistics("shape")
        .expect("binding geospatial projection fixture");
    group.bench_function("parquet/geospatial_statistics_into_value", |bencher| {
        bencher.iter(|| {
            black_box(yggdryl::Scalar::from(black_box(
                geospatial_statistics.clone(),
            )))
        });
    });
}

/// A Parquet WKB column large enough for its projected scan to be measurable.
fn geospatial_fixture() -> Buffer {
    let mut point = vec![1_u8];
    point.extend_from_slice(&1_u32.to_le_bytes());
    point.extend_from_slice(&1_f64.to_le_bytes());
    point.extend_from_slice(&2_f64.to_le_bytes());
    let field = DataType::from_fields([DataType::geometry(None)
        .expect("the default CRS is valid")
        .nullable_field("shape")])
    .expect("a valid geospatial root")
    .required_field("row");
    let shapes = BinaryArray::from_iter_values((0..ROWS).map(|_| point.as_slice()));
    let batch = RecordBatch::try_new(
        field.into_arrow_schema().expect("a projectable root"),
        vec![Arc::new(shapes)],
    )
    .expect("a batch matching the geospatial root");
    stored_with("bench-geospatial.parquet", &batch)
}

/// One plain-text fixture with exactly [`ROWS`] newline-delimited records.
fn text_fixture() -> Buffer {
    let mut text = String::with_capacity(ROWS as usize * 32);
    for row in 0..ROWS {
        writeln!(text, "row {row:08} carries benchmark text")
            .expect("writing to a String cannot fail");
    }
    let mut buffer = handle("bench-dimensions.log");
    buffer
        .write_all_bytes(text.as_bytes())
        .expect("the text fixture must write");
    buffer
}

pub(crate) fn dimension_benchmarks(criterion: &mut Criterion) {
    let source = batch();
    let ipc = stored_with("bench-dimensions.arrows", &source);
    let parquet = stored_with("bench-dimensions.parquet", &source);
    let geospatial = geospatial_fixture();
    let avro = stored_with("bench-dimensions.avro", &source);
    let text = text_fixture();
    let mut avro_options = RecordOptions::Avro(yggdryl::avro::AvroOptions::new());
    let sync_marker = *b"0123456789abcdef";

    let mut group = criterion.benchmark_group("io_dimensions");
    group.sample_size(10);
    parquet_statistics_cases(
        &mut group,
        Parquet::new(parquet.clone()),
        Parquet::new(parquet.clone()),
        Parquet::new(geospatial),
    );
    media_cases(&mut group, "ipc", Ipc::new(ipc.clone()), Ipc::new(ipc));
    media_cases(
        &mut group,
        "parquet",
        Parquet::new(parquet.clone()),
        Parquet::new(parquet),
    );
    media_cases(&mut group, "avro", Avro::new(avro.clone()), Avro::new(avro));
    group.bench_function("avro/options/read_block_codec", |bencher| {
        bencher.iter(|| black_box(black_box(&avro_options).avro_block_codec()));
    });
    group.bench_function("avro/options/set_block_codec_and_sync_marker", |bencher| {
        bencher.iter(|| {
            black_box(&mut avro_options)
                .set_avro_block_codec(black_box("deflate"))
                .expect("the core Avro codec vocabulary accepts deflate");
            black_box(&mut avro_options)
                .set_avro_sync_marker(Some(black_box(&sync_marker)))
                .expect("the marker has exactly sixteen bytes");
        });
    });
    media_cases(&mut group, "text", text.clone(), text);
    group.finish();
}
