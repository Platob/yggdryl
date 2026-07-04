//! Benchmarks for the field layer: the Arrow interop surface (`to_arrow` /
//! `from_arrow`) and schema assembly.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_field::yggdryl_dtype::{
    BinaryType, Int64Type, TypedMapType, TypedOptionalType, TypedSerieType, UInt8Type,
};
use yggdryl_field::{arrow_schema, Field, FieldFactory, Int64Field, UInt8Field};

const N: usize = 4096;

fn arrow_interop(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrow_interop");
    group.throughput(Throughput::Elements(N as u64));

    let field = Int64Field::new("id", true);
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
                black_box(Int64Field::from_arrow(black_box(&arrow_field)).unwrap());
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
                black_box(UInt8Field::new(black_box("flags"), index % 2 == 0));
            }
        })
    });

    let fields: Vec<arrow_schema::Field> = (0..N)
        .map(|i| Int64Field::new(format!("f{i}"), i % 2 == 0).to_arrow())
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

// The `FieldFactory` surface across every factory family: the data type builds its
// field.
fn factory(c: &mut Criterion) {
    let mut group = c.benchmark_group("factory");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_field", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(Int64Type.field(black_box("id"), index % 2 == 0));
            }
        })
    });
    group.bench_function("binary_field", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(BinaryType.field(black_box("payload"), index % 2 == 0));
            }
        })
    });

    let optional = TypedOptionalType::new(Int64Type);
    group.bench_function("optional_field", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(optional.field(black_box("score"), index % 2 == 0));
            }
        })
    });

    let serie = TypedSerieType::new(Int64Type);
    group.bench_function("serie_field", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(serie.field(black_box("scores"), index % 2 == 0));
            }
        })
    });

    let map = TypedMapType::new(UInt8Type, Int64Type);
    group.bench_function("map_field", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(map.field(black_box("ranks"), index % 2 == 0));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, arrow_interop, schema, factory);
criterion_main!(benches);
