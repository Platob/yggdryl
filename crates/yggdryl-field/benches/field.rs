//! Benchmarks for the field layer: the Arrow interop surface (`to_arrow` /
//! `from_arrow`) and schema assembly.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_field::{arrow_schema, Int64, RawField, UInt8};

const N: usize = 4096;

fn arrow_interop(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrow_interop");
    group.throughput(Throughput::Elements(N as u64));

    let field = Int64::new("id", true);
    group.bench_function("field_to_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(field.to_arrow());
            }
        })
    });

    let arrow_field = field.to_arrow();
    group.bench_function("field_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64::from_arrow(black_box(&arrow_field)).unwrap());
            }
        })
    });

    group.finish();
}

fn schema(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("field_new", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(UInt8::new(black_box("flags"), index % 2 == 0));
            }
        })
    });

    let fields: Vec<arrow_schema::Field> = (0..N)
        .map(|i| Int64::new(format!("f{i}"), i % 2 == 0).to_arrow())
        .collect();
    group.bench_function("arrow_schema_from_fields", |b| {
        // `Schema::new` consumes the fields, so clone them *outside* the timing via
        // `iter_batched` — timing the clone would misattribute ~20-30% of the loop.
        b.iter_batched(
            || fields.clone(),
            |fields| black_box(arrow_schema::Schema::new(fields)),
            criterion::BatchSize::LargeInput,
        )
    });

    group.finish();
}

criterion_group!(benches, arrow_interop, schema);
criterion_main!(benches);
