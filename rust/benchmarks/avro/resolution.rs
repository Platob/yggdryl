//! What a resolution plan costs to build, and what it costs to not have one.
//!
//! The plan is compiled once per (writer, reader) pair; the per-record path
//! executes it and never re-resolves. `cold_resolve` prices the compile,
//! `resolved_decode` prices executing it per container, and `direct_decode`
//! is the floor where the reader wants exactly what the writer wrote - so
//! the gap between the two decode cases is the true per-record cost of
//! resolving, which should be near zero.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::avro::{Resolution, Schema};
use yggdryl::holder::Buffer;
use yggdryl::{Scalar, avro, json};

/// Rows in the resolution fixture.
const ROWS: usize = crate::bench_profile::corpus(10_000, 512);

pub(crate) fn resolution_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/avro_resolution");

    let writer_json = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"id","type":"long"},
            {"name":"symbol","type":"string"},
            {"name":"qty","type":"int"},
            {"name":"venue","type":"string"},
            {"name":"note","type":"string"}
        ]}"#,
    )
    .expect("the writer schema parses");
    // The reader renames through an alias, promotes int to long, skips two
    // writer columns, and fills one field the writer never had.
    let reader = Schema::from_str(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"id","type":"long"},
            {"name":"quantity","aliases":["qty"],"type":"long"},
            {"name":"symbol","type":"string"},
            {"name":"source","type":"string","default":"bench"}
        ]}"#,
    )
    .expect("the reader schema parses");
    let writer = Schema::from_json(&writer_json).expect("the writer parses");

    let rows: Vec<Scalar> = (0..ROWS)
        .map(|index| {
            Scalar::from_mapping([
                (Scalar::from("id"), Scalar::from(index as i64)),
                (Scalar::from("symbol"), Scalar::from(format!("S{index:05}"))),
                (Scalar::from("qty"), Scalar::from((index % 1_000) as i64)),
                (Scalar::from("venue"), Scalar::from("XNAS")),
                (Scalar::from("note"), Scalar::from("skipped by the reader")),
            ])
            .expect("unique keys")
        })
        .collect();
    let mut stored = Buffer::new();
    avro::write_container(&mut stored, &writer_json, &[], &rows).expect("the fixture encodes");

    // Proven once outside the timers: the plan builds and both decodes agree
    // on the row count.
    assert!(Resolution::from_schemas(&writer, &reader).is_ok());
    assert_eq!(
        avro::read_container_resolved(&stored, &reader)
            .expect("the resolved read decodes")
            .rows
            .len(),
        ROWS
    );

    group.throughput(Throughput::Elements(1));
    group.bench_function("cold_resolve", |bencher| {
        bencher.iter(|| {
            Resolution::from_schemas(black_box(&writer), black_box(&reader))
                .expect("the plan builds")
        });
    });
    group.bench_function("fingerprint", |bencher| {
        bencher.iter(|| black_box(&writer).fingerprint());
    });
    group.bench_function("stable_hash_schema", |bencher| {
        bencher.iter(|| black_box(&writer).stable_hash());
    });

    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("direct_decode", |bencher| {
        bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
    });
    group.bench_function("resolved_decode", |bencher| {
        bencher.iter(|| {
            avro::read_container_resolved(black_box(&stored), black_box(&reader)).expect("decodes")
        });
    });

    group.finish();
}
