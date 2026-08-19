//! Decode and encode throughput by type family, and the varint floor.
//!
//! Each family isolates one wire shape - varint integers, length-prefixed
//! strings, exact decimals, deep nesting - so a regression names the shape
//! that regressed. Throughput is reported in both rows (Criterion elements)
//! and encoded bytes, per case.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{Value, avro, json};

/// Rows per fixture.
const ROWS: usize = 10_000;

/// A generator producing one row per index.
type RowMaker = Box<dyn Fn(usize) -> Value>;

/// One family: a schema and a row generator.
fn families() -> Vec<(&'static str, Value, RowMaker)> {
    let record = |fields: &str| -> Value {
        json::from_str(&format!(
            r#"{{"type":"record","name":"row","fields":[{fields}]}}"#
        ))
        .expect("the family schema parses")
    };
    vec![
        (
            "primitives",
            record(
                r#"{"name":"a","type":"long"},{"name":"b","type":"double"},{"name":"c","type":"boolean"}"#,
            ),
            Box::new(|index| {
                Value::from_mapping([
                    (Value::from("a"), Value::from(index as i64 - 5_000)),
                    (Value::from("b"), Value::from(index as f64 * 0.5)),
                    (Value::from("c"), Value::Bool(index % 2 == 0)),
                ])
                .expect("unique keys")
            }),
        ),
        (
            "strings",
            record(r#"{"name":"symbol","type":"string"},{"name":"venue","type":"string"}"#),
            Box::new(|index| {
                Value::from_mapping([
                    (Value::from("symbol"), Value::from(format!("SYM{index:06}"))),
                    (
                        Value::from("venue"),
                        Value::from(["XNAS", "XNYS", "XLON", "XETR"][index % 4]),
                    ),
                ])
                .expect("unique keys")
            }),
        ),
        (
            "decimals",
            record(
                r#"{"name":"price","type":{"type":"bytes","logicalType":"decimal","precision":18,"scale":4}}"#,
            ),
            Box::new(|index| {
                Value::from_mapping([(
                    Value::from("price"),
                    Value::Decimal(1_000_000 + index as i128 * 13, 4),
                )])
                .expect("unique keys")
            }),
        ),
        (
            "nested",
            record(
                r#"{"name":"legs","type":{"type":"array","items":
                    {"type":"record","name":"leg","fields":[
                        {"name":"qty","type":"long"},
                        {"name":"fills","type":{"type":"map","values":"double"}}
                    ]}}}"#,
            ),
            Box::new(|index| {
                let leg = |qty: i64| {
                    Value::from_mapping([
                        (Value::from("qty"), Value::from(qty)),
                        (
                            Value::from("fills"),
                            Value::from_mapping([
                                (Value::from("open"), Value::from(1.5_f64)),
                                (Value::from("close"), Value::from(2.5_f64)),
                            ])
                            .expect("unique keys"),
                        ),
                    ])
                    .expect("unique keys")
                };
                Value::from_mapping([(
                    Value::from("legs"),
                    Value::from_sequence((0..index % 3 + 1).map(|qty| leg(qty as i64))),
                )])
                .expect("unique keys")
            }),
        ),
    ]
}

pub(crate) fn format_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/avro_types");

    for (label, schema, row) in families() {
        let rows: Vec<Value> = (0..ROWS).map(&row).collect();
        let mut stored = Buffer::new();
        avro::write_container(&mut stored, &schema, &[], &rows).expect("the fixture encodes");
        // Proven once outside the timers: the fixture round-trips.
        assert_eq!(
            avro::read_container(&stored).expect("decodes").rows.len(),
            ROWS
        );
        let encoded = stored.read_all_bytes().expect("the buffer reads").len() as u64;

        group.throughput(Throughput::Elements(ROWS as u64));
        group.bench_function(format!("decode_rows/{label}"), |bencher| {
            bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
        });
        group.bench_function(format!("encode_rows/{label}"), |bencher| {
            bencher.iter(|| {
                let mut target = Buffer::new();
                avro::write_container(&mut target, black_box(&schema), &[], black_box(&rows))
                    .expect("encodes");
                target
            });
        });
        group.throughput(Throughput::Bytes(encoded));
        group.bench_function(format!("decode_bytes/{label}"), |bencher| {
            bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
        });
    }

    // The varint floor: one single-object long per iteration isolates the
    // zig-zag encode and decode from every container concern above it.
    let long = avro::Schema::from_str("\"long\"").expect("a long schema");
    let value = Value::I64(-123_456_789);
    let framed = avro::to_single_object_vec(&long, &value).expect("the frame encodes");
    assert_eq!(
        avro::from_single_object_slice(&framed, &long).expect("the frame decodes"),
        value
    );
    group.throughput(Throughput::Elements(1));
    group.bench_function("varint/encode_single_object", |bencher| {
        bencher.iter(|| avro::to_single_object_vec(black_box(&long), black_box(&value)));
    });
    group.bench_function("varint/decode_single_object", |bencher| {
        bencher.iter(|| avro::from_single_object_slice(black_box(&framed), black_box(&long)));
    });

    group.finish();
}
