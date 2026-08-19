//! Avro object container round trips over the shared `Value`.
//!
//! The fixture mirrors what a manifest-shaped row carries - an int, a string,
//! a nullable double, an array, a nested record - so the numbers describe the
//! path Iceberg pays, not a synthetic best case.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{Value, avro, json};

/// Rows in the representative container.
const ROWS: usize = 1_000;

/// The writer schema every benchmark row is encoded against.
fn schema() -> Value {
    json::from_str(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "code", "type": "int"},
            {"name": "name", "type": "string"},
            {"name": "score", "type": ["null", "double"], "default": null},
            {"name": "tags", "type": {"type": "array", "items": "long"}},
            {"name": "nested", "type": {"type": "record", "name": "inner", "fields": [
                {"name": "flag", "type": "boolean"}
            ]}}
        ]}"#,
    )
    .expect("the benchmark schema parses")
}

/// One representative row per index.
fn row(index: usize) -> Value {
    let score = if index % 3 == 0 {
        Value::Null
    } else {
        Value::from(index as f64 * 0.25)
    };
    Value::from_mapping([
        (Value::from("code"), Value::from(index as i64 - 500)),
        (Value::from("name"), Value::from(format!("SYM{index:04}"))),
        (Value::from("score"), score),
        (
            Value::from("tags"),
            Value::from_sequence((0..index % 4).map(|tag| Value::from(tag as i64))),
        ),
        (
            Value::from("nested"),
            Value::from_mapping([(Value::from("flag"), Value::Bool(index % 2 == 0))])
                .expect("unique keys"),
        ),
    ])
    .expect("unique keys")
}

pub(crate) fn avro_benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let rows: Vec<Value> = (0..ROWS).map(row).collect();

    let mut stored = Buffer::new();
    avro::write_container(&mut stored, &schema, &[], &rows)
        .expect("the representative container encodes");
    // Proven once outside the timers: the container round-trips.
    let container = avro::read_container(&stored).expect("the container decodes");
    assert_eq!(container.rows.len(), ROWS);
    let encoded_len = stored.read_all().expect("the buffer reads").len();

    let mut group = criterion.benchmark_group("codec/avro");

    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
    });
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| {
            let mut target = Buffer::new();
            avro::write_container(&mut target, black_box(&schema), &[], black_box(&rows))
                .expect("encodes");
            target
        });
    });

    group.throughput(Throughput::Bytes(encoded_len as u64));
    group.bench_function("decode_representative_bytes", |bencher| {
        bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
    });

    group.finish();
}
