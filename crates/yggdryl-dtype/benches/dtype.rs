//! Benchmarks for the data-type layer: the native byte codec, the descriptor
//! surface and the Arrow interop (`to_arrow` / `from_arrow`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_dtype::{DataType, Int64Type, Int8Type, TypedDataType, UnionType};

const N: usize = 4096;

fn codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_native_to_bytes", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(TypedDataType::native_to_bytes(&Int64Type, &value));
            }
        })
    });

    let bytes = TypedDataType::native_to_bytes(&Int64Type, &0x0123_4567_89AB_CDEFi64);
    group.bench_function("int64_native_from_bytes", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(TypedDataType::native_from_bytes(&Int64Type, black_box(&bytes)).unwrap());
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
                black_box(Int64Type.name());
            }
        })
    });
    group.bench_function("int64_arrow_format", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64Type.arrow_format());
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
                black_box(Int64Type.to_arrow());
            }
        })
    });

    let arrow_type = Int64Type.to_arrow();
    group.bench_function("data_type_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64Type::from_arrow(black_box(&arrow_type)).unwrap());
            }
        })
    });

    group.finish();
}

fn schema(c: &mut Criterion) {
    let mut group = c.benchmark_group("schema");
    group.throughput(Throughput::Elements(N as u64));

    // Heterogeneous descriptors through the vtable, as a schema printer would.
    let types: Vec<Box<dyn DataType>> = vec![Box::new(Int8Type), Box::new(Int64Type)];
    group.bench_function("dyn_to_arrow", |b| {
        b.iter(|| {
            for _ in 0..N / 2 {
                for data_type in &types {
                    black_box(data_type.to_arrow());
                }
            }
        })
    });

    group.finish();
}

fn optional(c: &mut Criterion) {
    let mut group = c.benchmark_group("optional");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("union_optional_data_type", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(UnionType::optional(&Int64Type));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, codec, descriptor, arrow_interop, schema, optional);
criterion_main!(benches);
