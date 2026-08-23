//! Structured `Value` reads and writes through the generic handle surface.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{Url, Value};

const ROWS: usize = 16_384;

fn value() -> Value {
    Value::from_sequence((0..ROWS).map(|id| {
        Value::from_record([
            (
                "id",
                Value::I64(i64::try_from(id).expect("the fixture id fits i64")),
            ),
            ("symbol", Value::from(format!("SYMBOL-{id:08}"))),
            ("venue", Value::from("XNAS")),
        ])
        .expect("unique field names")
    }))
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("a valid benchmark name")
            .media_type(),
    )
}

pub(crate) fn structured_value_benchmarks(criterion: &mut Criterion) {
    let value = value();
    let decoded = yggdryl::json::into_bytes(&value).expect("the fixture encodes");
    let names = [
        "value.json",
        "value.json.gz",
        "value.json.zz",
        "value.json.zst",
    ];
    let mut stored = Vec::with_capacity(names.len());
    for name in names {
        let mut target = handle(name);
        target.write_value(&value).expect("the fixture writes");
        assert_eq!(target.read_value(None).expect("the fixture reads"), value);
        stored.push((name, target));
    }

    let mut reads = criterion.benchmark_group("io_value_read");
    reads.sample_size(10);
    reads.throughput(Throughput::Bytes(decoded.len() as u64));
    for (name, source) in &stored {
        reads.bench_function(BenchmarkId::from_parameter(name), |bencher| {
            bencher.iter(|| black_box(source).read_value(None).expect("a value read"));
        });
    }
    reads.finish();

    let mut writes = criterion.benchmark_group("io_value_write");
    writes.sample_size(10);
    writes.throughput(Throughput::Bytes(decoded.len() as u64));
    for name in names {
        writes.bench_function(BenchmarkId::from_parameter(name), |bencher| {
            bencher.iter_batched(
                || handle(name),
                |mut target| {
                    target
                        .write_value(black_box(&value))
                        .expect("a value write");
                    black_box(target);
                },
                BatchSize::SmallInput,
            );
        });
    }
    writes.finish();
}
