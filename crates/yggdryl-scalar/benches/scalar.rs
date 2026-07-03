//! Benchmarks for the scalar layer: scalar construction, the `as_*` accessors and
//! the Arrow interop surface (`to_arrow` / `from_arrow`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64, Int64Serie, Int8, Optional, RawScalar, Serie};

type OptionalInt64 = Optional<dtype::Int64, Int64>;
type Int64SerieGeneric = Serie<dtype::Int64, Int64>;

const N: usize = 4096;

fn scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalar");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_new", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(Int64::new(black_box(value)));
            }
        })
    });

    group.bench_function("int64_to_arrow_value", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(Int64::new(value).to_arrow());
            }
        })
    });

    group.bench_function("int64_to_arrow_null", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64::null().to_arrow());
            }
        })
    });

    let arrow = Int64::new(42).to_arrow();
    group.bench_function("int64_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64::from_arrow(black_box(arrow.as_ref())).unwrap());
            }
        })
    });

    // The narrowest width, to expose any width-dependence in the same paths.
    group.bench_function("int8_to_arrow_value", |b| {
        b.iter(|| {
            for value in 0..N {
                black_box(Int8::new(value as i8).to_arrow());
            }
        })
    });

    group.finish();
}

fn accessor(c: &mut Criterion) {
    let mut group = c.benchmark_group("accessor");
    group.throughput(Throughput::Elements(N as u64));

    let scalar = Int64::new(42);
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

    let optional = OptionalInt64::new(Int64::new(42));
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
                black_box(OptionalInt64::new(Int64::new(black_box(value))));
            }
        })
    });

    group.bench_function("optional_to_arrow_value", |b| {
        let scalar = OptionalInt64::new(Int64::new(42));
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

    let arrow = OptionalInt64::new(Int64::new(42)).to_arrow();
    group.bench_function("optional_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(OptionalInt64::from_arrow(black_box(arrow.as_ref())).unwrap());
            }
        })
    });

    group.finish();
}

fn array(c: &mut Criterion) {
    let mut group = c.benchmark_group("array");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_serie_from_vec", |b| {
        b.iter_batched(
            || (0..N as i64).collect::<Vec<_>>(),
            |values| black_box(Int64Serie::from(values)),
            criterion::BatchSize::LargeInput,
        )
    });

    let numbers = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("int64_serie_values_borrow", |b| {
        b.iter(|| black_box(black_box(&numbers).values()))
    });

    group.bench_function("int64_serie_get_at", |b| {
        b.iter(|| {
            for index in 0..N {
                let _ = black_box(numbers.get_at::<i64>(black_box(index)));
            }
        })
    });

    group.bench_function("int64_serie_to_arrow", |b| {
        b.iter(|| black_box(numbers.to_arrow()))
    });

    let arrow = numbers.to_arrow();
    group.bench_function("int64_serie_from_arrow", |b| {
        b.iter(|| black_box(Int64Serie::from_arrow(black_box(arrow.as_ref())).unwrap()))
    });

    // The generic scalar accessor, for comparison: one inner Arrow round trip per
    // element against the buffer-backed direct read above.
    let generic = Int64SerieGeneric::from_arrow(arrow.as_ref()).unwrap();
    group.bench_function("serie_get_scalar_at", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(generic.get_scalar_at(black_box(index)));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, scalar, accessor, optional, array);
criterion_main!(benches);
