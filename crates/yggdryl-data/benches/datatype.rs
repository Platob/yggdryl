//! Benchmarks for the data-model layer: the native byte codec, the Arrow interop
//! surface (`to_arrow` / `from_arrow`) and scalar construction.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_data::{
    arrow_schema, Int64, Int64Field, Int64Scalar, Int8, Int8Scalar, RawDataType, RawField,
    RawScalar,
};

const N: usize = 4096;

fn codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_native_to_bytes", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(yggdryl_data::DataType::native_to_bytes(&Int64, &value));
            }
        })
    });

    let bytes = yggdryl_data::DataType::native_to_bytes(&Int64, &0x0123_4567_89AB_CDEFi64);
    group.bench_function("int64_native_from_bytes", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(
                    yggdryl_data::DataType::native_from_bytes(&Int64, black_box(&bytes)).unwrap(),
                );
            }
        })
    });

    group.finish();
}

fn descriptor(c: &mut Criterion) {
    let mut group = c.benchmark_group("descriptor");
    group.throughput(Throughput::Elements(N as u64));

    // `name` borrows; `arrow_format` allocates a String per call.
    group.bench_function("int64_name", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64.name());
            }
        })
    });
    group.bench_function("int64_arrow_format", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64.arrow_format());
            }
        })
    });

    group.finish();
}

fn arrow_interop(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrow_interop");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("data_type_to_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64.to_arrow());
            }
        })
    });

    let arrow_type = Int64.to_arrow();
    group.bench_function("data_type_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64::from_arrow(black_box(&arrow_type)).unwrap());
            }
        })
    });

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

fn scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalar");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_new", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(Int64Scalar::new(black_box(value)));
            }
        })
    });

    group.bench_function("int64_to_arrow_value", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(Int64Scalar::new(value).to_arrow());
            }
        })
    });

    group.bench_function("int64_to_arrow_null", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64Scalar::null().to_arrow());
            }
        })
    });

    let arrow = Int64Scalar::new(42).to_arrow();
    group.bench_function("int64_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64Scalar::from_arrow(black_box(arrow.as_ref())).unwrap());
            }
        })
    });

    // The narrowest width, to expose any width-dependence in the same paths.
    group.bench_function("int8_to_arrow_value", |b| {
        b.iter(|| {
            for value in 0..N {
                black_box(Int8Scalar::new(value as i8).to_arrow());
            }
        })
    });

    group.finish();
}

fn schema(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema");
    group.throughput(Throughput::Elements(N as u64));

    // Heterogeneous descriptors through the vtable, as a schema printer would.
    let types: Vec<Box<dyn RawDataType>> = vec![Box::new(Int8), Box::new(Int64)];
    group.bench_function("dyn_to_arrow", |b| {
        b.iter(|| {
            for _ in 0..N / 2 {
                for data_type in &types {
                    black_box(data_type.to_arrow());
                }
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

criterion_group!(benches, codec, descriptor, arrow_interop, scalar, schema);
criterion_main!(benches);
