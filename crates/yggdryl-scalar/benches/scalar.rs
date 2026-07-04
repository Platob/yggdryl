//! Benchmarks for the scalar layer: scalar construction, the `as_*` accessors and
//! the Arrow interop surface (`to_arrow` / `from_arrow`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64Scalar, Int8Scalar, OptionalScalar, Scalar, ScalarFactory};

type OptionalInt64 = OptionalScalar<dtype::Int64Type, Int64Scalar>;

const N: usize = 4096;

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

    // The same construction through the ScalarFactory surface: the data type builds
    // its scalar.
    group.bench_function("int64_via_factory", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(dtype::Int64Type.scalar(black_box(value)));
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

fn accessor(c: &mut Criterion) {
    let mut group = c.benchmark_group("accessor");
    group.throughput(Throughput::Elements(N as u64));

    let scalar = Int64Scalar::new(42);
    group.bench_function("int64_as_i64_direct", |b| {
        b.iter(|| {
            for _ in 0..N {
                let _ = black_box(black_box(&scalar).as_i64());
            }
        })
    });
    group.bench_function("int64_as_i8_converted", |b| {
        b.iter(|| {
            for _ in 0..N {
                let _ = black_box(black_box(&scalar).as_i8());
            }
        })
    });
    group.bench_function("int64_as_f64_converted", |b| {
        b.iter(|| {
            for _ in 0..N {
                let _ = black_box(black_box(&scalar).as_f64());
            }
        })
    });

    let optional = OptionalInt64::new(Int64Scalar::new(42));
    group.bench_function("optional_as_i64_redirected", |b| {
        b.iter(|| {
            for _ in 0..N {
                let _ = black_box(black_box(&optional).as_i64());
            }
        })
    });

    group.finish();
}

fn optional(c: &mut Criterion) {
    let mut group = c.benchmark_group("optional");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("optional_new", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(OptionalInt64::new(Int64Scalar::new(black_box(value))));
            }
        })
    });

    group.bench_function("optional_to_arrow_value", |b| {
        let scalar = OptionalInt64::new(Int64Scalar::new(42));
        b.iter(|| {
            for _ in 0..N {
                black_box(scalar.to_arrow());
            }
        })
    });

    group.bench_function("optional_to_arrow_null", |b| {
        let scalar = OptionalInt64::null();
        b.iter(|| {
            for _ in 0..N {
                black_box(scalar.to_arrow());
            }
        })
    });

    let arrow = OptionalInt64::new(Int64Scalar::new(42)).to_arrow();
    group.bench_function("optional_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(OptionalInt64::from_arrow(black_box(arrow.as_ref())).unwrap());
            }
        })
    });

    group.finish();
}

criterion_group!(benches, scalar, accessor, optional);
criterion_main!(benches);
