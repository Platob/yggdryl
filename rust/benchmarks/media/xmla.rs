//! XML for Analysis rowsets: the document a `.xmla` handle holds, written
//! from and read back into record columns, and the Discover envelope a
//! client sends.
//!
//! The fixture is a small market row - an identifier, a nullable symbol, a
//! price and a flag - so the numbers describe a table a provider serves, not
//! a synthetic best case.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::xmla::{
    Content, Discover, Method, PropertyList, Request, RequestType, Response, Restrictions, Rowset,
    write_rowset,
};
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

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
            ])
        }),
    )
    .expect("rows under the field")
}

pub(crate) fn xmla_benchmarks(criterion: &mut Criterion) {
    let rows = corpus(10_000, 64);
    let field = field();
    let rowset = Rowset::new(field.clone()).expect("a rowset over a record field");
    let batch = batch(&field, rows);
    let encoded = write_rowset(
        Vec::new(),
        &[],
        Method::Discover,
        &rowset,
        [Ok(batch.clone())],
        Content::SchemaData,
    )
    .expect("the representative rowset encodes");
    // Proven once outside the timers: the document round-trips.
    let response = Response::from_bytes(&encoded, None).expect("the document decodes");
    assert_eq!(response.rows().map(Serie::len), Some(rows));

    let mut group = criterion.benchmark_group("media/xmla");
    group.throughput(Throughput::Elements(rows as u64));
    group.bench_function("write_rowset", |bencher| {
        bencher.iter(|| {
            write_rowset(
                Vec::with_capacity(encoded.len()),
                &[],
                Method::Discover,
                black_box(&rowset),
                [Ok(batch.clone())],
                Content::SchemaData,
            )
            .expect("encodes")
        });
    });
    group.bench_function("read_rowset", |bencher| {
        bencher.iter(|| Response::from_bytes(black_box(&encoded), None).expect("decodes"));
    });
    group.finish();

    let discover = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(
                Restrictions::new()
                    .with("TABLE_CATALOG", "market")
                    .with("TABLE_TYPE", "TABLE"),
            )
            .with_properties(
                PropertyList::new()
                    .with("Content", "SchemaData")
                    .with("Format", "Tabular"),
            ),
    );
    let envelope = discover.into_bytes().expect("the request encodes");
    assert_eq!(
        Request::from_bytes(&envelope)
            .expect("the request decodes")
            .kind(),
        Method::Discover
    );

    let mut group = criterion.benchmark_group("media/xmla/request");
    group.throughput(Throughput::Bytes(envelope.len() as u64));
    group.bench_function("write_discover", |bencher| {
        bencher.iter(|| black_box(&discover).into_bytes().expect("encodes"));
    });
    group.bench_function("read_discover", |bencher| {
        bencher.iter(|| Request::from_bytes(black_box(&envelope)).expect("decodes"));
    });
    group.finish();
}
