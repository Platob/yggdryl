//! Office Open XML workbooks: the records a `.xlsx` handle writes and reads
//! back, and the random-access model beside them.
//!
//! The fixture is a small market row - an identifier, a nullable symbol, a
//! price, a flag and a date - so the numbers describe a sheet a caller
//! exports, not a synthetic best case. Every write renders the part as it is
//! read, every read streams the part, and the workbook doors parse the sheet
//! once into cells.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::excel::{CellRef, Sheet, Workbook};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};

use crate::bench_profile::corpus;

/// The record field every benchmark row is laid out under.
fn field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
            DataType::Date32.required_field("traded"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// `rows` representative rows as one record column.
fn batch(field: &Field, rows: usize) -> Serie {
    Serie::from_scalars(
        field.clone(),
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                if index % 5 == 0 {
                    Scalar::Null
                } else {
                    Scalar::from(format!("SYM{index:04}"))
                },
                Scalar::from(index as f64 * 0.25),
                Scalar::from(index % 2 == 0),
                Scalar::date32(19_723 + (index % 365) as i32),
            ])
        }),
    )
    .expect("rows under the field")
}

/// A fresh in-memory `.xlsx` handle.
fn xlsx() -> Buffer {
    Buffer::new().with_media_type(MimeType::XLSX.into())
}

pub(crate) fn excel_benchmarks(criterion: &mut Criterion) {
    let rows = corpus(10_000, 64);
    let field = field();
    let serie = batch(&field, rows);
    let arrow = serie.clone().into_arrow_batch().expect("a batch");
    let declared = xlsx()
        .record_options()
        .expect("Excel options")
        .with_field(field.clone());
    let inferred = xlsx().record_options().expect("Excel options");

    // Proven once outside the timers: the rows round-trip under the field.
    let mut written = xlsx();
    written
        .overwrite_arrow_batch(arrow.clone(), &declared)
        .expect("the workbook writes");
    let bytes = written.read_all_bytes().expect("the package bytes");
    let read: usize = written
        .read_arrow_reader(&declared)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(read, rows);
    assert_eq!(
        Workbook::from_bytes(bytes.clone())
            .expect("the package opens")
            .sheet("Sheet1")
            .expect("the sheet")
            .len(),
        rows + 1
    );

    let mut group = criterion.benchmark_group("media/excel");
    group.throughput(Throughput::Elements(rows as u64));
    group.bench_function("write_records", |bencher| {
        bencher.iter(|| {
            let mut handle = xlsx();
            handle
                .overwrite_arrow_batch(black_box(arrow.clone()), &declared)
                .expect("writes");
            handle
        });
    });
    group.bench_function("read_records_declared", |bencher| {
        bencher.iter(|| {
            let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
            handle
                .read_arrow_reader(black_box(&declared))
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    group.bench_function("read_records_inferred", |bencher| {
        bencher.iter(|| {
            let handle = Buffer::from_bytes(bytes.clone()).with_media_type(MimeType::XLSX.into());
            let root = handle
                .read_arrow_field(black_box(&inferred))
                .expect("a field");
            let read: usize = handle
                .read_arrow_reader(&inferred)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            (root, read)
        });
    });
    group.bench_function("workbook_open_and_one_cell", |bencher| {
        let reference: CellRef = "C3".parse().expect("a reference");
        bencher.iter(|| {
            Workbook::from_bytes(black_box(bytes.clone()))
                .expect("the package opens")
                .sheet("Sheet1")
                .expect("the sheet")
                .scalar(reference)
        });
    });
    group.bench_function("sheet_into_serie", |bencher| {
        bencher.iter(|| {
            let mut workbook =
                Workbook::from_bytes(black_box(bytes.clone())).expect("the package opens");
            workbook
                .remove_sheet("Sheet1")
                .expect("the sheet parses")
                .expect("the sheet is there")
                .into_serie(Some(&field), true, Default::default())
                .expect("the rows lay out")
        });
    });
    group.bench_function("sheet_from_serie", |bencher| {
        bencher.iter(|| {
            Sheet::from_serie("Sheet1", black_box(&serie), true).expect("the cells lay out")
        });
    });
    group.finish();
}
