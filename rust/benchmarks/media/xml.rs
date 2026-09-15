//! What an XML read and write actually cost, by the path that pays.
//!
//! A document states no schema and carries no index, so every case here reads
//! the document. What the cases separate is *how much of it is decoded*: a
//! declared field types the leaves it names, a projection skips the subtrees it
//! does not, and a schemaless read has to infer the shape from the rows. A
//! regression should name which of those moved.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, xml};
use yggdryl::{Field, IOBase, IOMedia, Limits, Url};

/// Rows per fixture.
const ROWS: usize = crate::bench_profile::corpus(20_000, 256);

/// A document of `rows` rows, four leaf columns, one of them an attribute.
fn document(rows: usize) -> String {
    let mut out = String::with_capacity(rows * 96);
    out.push_str("<rows>");
    for index in 0..rows {
        out.push_str("<row id=\"");
        out.push_str(&index.to_string());
        out.push_str("\"><symbol>ABCDEFGH</symbol><px>9.50</px><qty>");
        out.push_str(&index.to_string());
        out.push_str("</qty></row>");
    }
    out.push_str("</rows>");
    out
}

/// A handle holding one document under an XML media type.
fn handle(document: &str) -> Buffer {
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///bench.xml").unwrap().media_type());
    handle.write_all_bytes(document.as_bytes()).unwrap();
    handle
}

/// The typed field the document's columns prove.
fn declared() -> Field {
    Field::from_str(
        "row: struct<id: int64, symbol: utf8, px: decimal64(18,2), qty: int64> not null",
    )
    .expect("the benchmark field parses")
}

/// Read throughput, by how much the caller told the reader.
pub fn read_benchmarks(criterion: &mut Criterion) {
    let text = document(ROWS);
    let handle = handle(&text);
    let mut group = criterion.benchmark_group("media/xml/read");
    group.throughput(Throughput::Elements(ROWS as u64));

    // Nothing declared: the shape comes from the rows, so inference runs.
    let inferred = handle.record_options().unwrap();
    group.bench_function("inferred", |bencher| {
        bencher.iter(|| {
            let reader = handle.read_arrow_reader(black_box(&inferred)).unwrap();
            black_box(reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>())
        });
    });

    // Declared: no inference, and every leaf crosses the value contract.
    let typed = handle.record_options().unwrap().with_field(declared());
    group.bench_function("declared", |bencher| {
        bencher.iter(|| {
            let reader = handle.read_arrow_reader(black_box(&typed)).unwrap();
            black_box(reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>())
        });
    });

    // One column of four: what a projection is worth.
    let projected = handle
        .record_options()
        .unwrap()
        .with_field(Field::from_str("row: struct<qty: int64> not null").unwrap());
    group.bench_function("projected", |bencher| {
        bencher.iter(|| {
            let reader = handle.read_arrow_reader(black_box(&projected)).unwrap();
            black_box(reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>())
        });
    });

    // The schema alone, which still has to read the document.
    group.bench_function("field", |bencher| {
        bencher.iter(|| black_box(handle.read_arrow_field(black_box(&inferred)).unwrap()));
    });
    group.finish();

    // The value surface, with no Arrow anywhere.
    let mut group = criterion.benchmark_group("media/xml/value");
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("document", |bencher| {
        bencher.iter(|| black_box(xml::from_utf8(black_box(&text)).unwrap()));
    });
    group.finish();
}

/// Write throughput, and what the declaration costs a write.
pub fn write_benchmarks(criterion: &mut Criterion) {
    let text = document(ROWS);
    let source = handle(&text);
    let field = declared();
    let options = source.record_options().unwrap().with_field(field);
    let value = source.read_arrow(Some(&options)).unwrap();

    let mut group = criterion.benchmark_group("media/xml/write");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("overwrite", |bencher| {
        bencher.iter(|| {
            let mut target = handle("");
            target
                .overwrite_arrow_reader(value.clone().into_reader().unwrap(), &options)
                .unwrap();
            black_box(target.size())
        });
    });
    group.finish();
}

/// What a schema costs to read and to write back.
pub fn schema_benchmarks(criterion: &mut Criterion) {
    let field = declared();
    let written = xml::field_into_xsd(&field, yggdryl::text::Formatting::new()).unwrap();

    let mut group = criterion.benchmark_group("media/xml/schema");
    group.bench_function("from_xsd", |bencher| {
        bencher.iter(|| {
            black_box(xml::field_from_xsd(black_box(&written), Limits::default(), None).unwrap())
        });
    });
    group.bench_function("into_xsd", |bencher| {
        bencher.iter(|| {
            black_box(
                xml::field_into_xsd(black_box(&field), yggdryl::text::Formatting::new()).unwrap(),
            )
        });
    });
    group.finish();
}
