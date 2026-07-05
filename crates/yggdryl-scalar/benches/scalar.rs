//! Benchmarks for the scalar layer: scalar construction, the `as_*` accessors and
//! the Arrow interop surface (`to_arrow` / `from_arrow`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64Scalar, Int8Scalar, Scalar, ScalarFactory, TypedOptionalScalar};

type OptionalInt64 = TypedOptionalScalar<dtype::Int64Type, Int64Scalar>;

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
                black_box(Int64Scalar::new(value).to_arrow_scalar().into_inner());
            }
        })
    });

    group.bench_function("int64_to_arrow_null", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(Int64Scalar::null().to_arrow_scalar().into_inner());
            }
        })
    });

    let arrow = Int64Scalar::new(42).to_arrow_scalar().into_inner();
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
                black_box(Int8Scalar::new(value as i8).to_arrow_scalar().into_inner());
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
                black_box(scalar.to_arrow_scalar().into_inner());
            }
        })
    });

    group.bench_function("optional_to_arrow_null", |b| {
        let scalar = OptionalInt64::null();
        b.iter(|| {
            for _ in 0..N {
                black_box(scalar.to_arrow_scalar().into_inner());
            }
        })
    });

    let arrow = OptionalInt64::new(Int64Scalar::new(42))
        .to_arrow_scalar()
        .into_inner();
    group.bench_function("optional_from_arrow", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(OptionalInt64::from_arrow(black_box(arrow.as_ref())).unwrap());
            }
        })
    });

    group.finish();
}

// The `ScalarFactory` surface across every factory family: the data type builds a
// value scalar, the null scalar and the default scalar.
fn factory(c: &mut Criterion) {
    let mut group = c.benchmark_group("factory");
    group.throughput(Throughput::Elements(N as u64));

    // Integer: value, null and default.
    group.bench_function("int64_scalar", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(dtype::Int64Type.scalar(black_box(value)));
            }
        })
    });
    group.bench_function("int64_null_scalar", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(dtype::Int64Type.null_scalar());
            }
        })
    });
    group.bench_function("int64_default_scalar", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(dtype::Int64Type.default_scalar());
            }
        })
    });

    // Binary: build from owned bytes.
    group.bench_function("binary_scalar", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(dtype::BinaryType.scalar(black_box(vec![1u8, 2, 3, 4])));
            }
        })
    });

    // Optional: wrap the value variant.
    let optional = dtype::TypedOptionalType::new(dtype::Int64Type);
    group.bench_function("optional_scalar", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(optional.scalar(black_box(value)));
            }
        })
    });

    // Serie: build a sequence through the value type's own factory.
    let serie = dtype::TypedSerieType::new(dtype::Int64Type);
    group.bench_function("serie_scalar", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(serie.scalar(black_box(vec![1i64, 2, 3, 4])));
            }
        })
    });

    // Map: build entries through the key and value factories.
    let map = dtype::TypedMapType::new(dtype::UInt8Type, dtype::Int64Type);
    group.bench_function("map_scalar", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(map.scalar(black_box(vec![(1u8, 2i64), (3, 4)])));
            }
        })
    });

    group.finish();
}

// The RecordScalar generic struct-row accessor: build a row and read children.
fn record(c: &mut Criterion) {
    use yggdryl_scalar::yggdryl_dtype::arrow_schema;
    use yggdryl_scalar::{AnyScalar, NestedSerie, RecordScalar};

    let mut group = c.benchmark_group("record");
    group.throughput(Throughput::Elements(N as u64));

    let point = yggdryl_scalar::yggdryl_dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]));
    group.bench_function("record_new", |b| {
        b.iter(|| {
            for value in 0..N as i64 {
                black_box(
                    RecordScalar::new(
                        point.clone(),
                        vec![
                            AnyScalar::from(Int64Scalar::new(black_box(value))),
                            AnyScalar::from(Int64Scalar::new(black_box(value + 1))),
                        ],
                    )
                    .unwrap(),
                );
            }
        })
    });

    let row = RecordScalar::new(
        point.clone(),
        vec![
            AnyScalar::from(Int64Scalar::new(1)),
            AnyScalar::from(Int64Scalar::new(2)),
        ],
    )
    .unwrap();
    group.bench_function("record_any_scalar_by", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(row.any_scalar_by(black_box("y")));
            }
        })
    });
    group.bench_function("record_any_scalar_at", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(row.any_scalar_at(black_box(1)));
            }
        })
    });
    // The typed field accessor: recovers a concrete scalar (Rust-only).
    group.bench_function("record_scalar_by", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(row.scalar_by::<Int64Scalar>(black_box("y")));
            }
        })
    });
    // The native-value field accessor by name and by index.
    group.bench_function("record_value_by", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(row.value_by::<i64>(black_box("y")));
            }
        })
    });
    group.bench_function("record_value_at", |b| {
        b.iter(|| {
            for _ in 0..N {
                black_box(row.value_at::<i64>(black_box(1)));
            }
        })
    });

    // A serie of struct rows: build it once, then read rows and field columns back.
    let rows: Vec<RecordScalar> = (0..N as i64)
        .map(|value| {
            RecordScalar::new(
                point.clone(),
                vec![
                    AnyScalar::from(Int64Scalar::new(value)),
                    AnyScalar::from(Int64Scalar::new(value + 1)),
                ],
            )
            .unwrap()
        })
        .collect();
    let points = yggdryl_scalar::TypedStructSerie::new(point.clone(), rows);
    group.bench_function("struct_serie_scalar_at", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(points.scalar_at(black_box(index)));
            }
        })
    });
    group.bench_function("struct_serie_field_column", |b| {
        b.iter(|| black_box(points.child_serie_by(black_box("x"))))
    });

    // The assembly path: build one struct column from N row scalars — each row's
    // `to_arrow_scalar` plus a single `concat` — the cost paid once at construction,
    // measured apart from the row-scalar building (done in the untimed setup).
    group.bench_function("struct_serie_new", |b| {
        b.iter_batched(
            || {
                (0..N as i64)
                    .map(|value| {
                        RecordScalar::new(
                            point.clone(),
                            vec![
                                AnyScalar::from(Int64Scalar::new(value)),
                                AnyScalar::from(Int64Scalar::new(value + 1)),
                            ],
                        )
                        .unwrap()
                    })
                    .collect::<Vec<_>>()
            },
            |rows| black_box(yggdryl_scalar::TypedStructSerie::new(point.clone(), rows)),
            criterion::BatchSize::SmallInput,
        )
    });

    // Iterating the rows against the indexed `scalar_at` loop above: the iterator
    // reconstitutes the struct column once, then slices per row.
    group.bench_function("struct_serie_iter_records", |b| {
        b.iter(|| {
            for record in points.iter_records() {
                black_box(record);
            }
        })
    });

    group.finish();
}

criterion_group!(benches, scalar, accessor, optional, factory, record);
criterion_main!(benches);
