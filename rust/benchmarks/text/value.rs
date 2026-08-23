use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::Scalar;

pub fn value_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/value");

    let narrow = entries(8);
    group.bench_function("mapping_8", |bencher| {
        bencher.iter_batched(
            || narrow.clone(),
            |entries| black_box(Scalar::from_mapping(entries).unwrap()),
            BatchSize::SmallInput,
        );
    });

    let wide = entries(1_024);
    group.bench_function("mapping_1024", |bencher| {
        bencher.iter_batched(
            || wide.clone(),
            |entries| black_box(Scalar::from_mapping(entries).unwrap()),
            BatchSize::LargeInput,
        );
    });

    let nested = nested(64);
    group.bench_function("clone_shared_depth_64", |bencher| {
        bencher.iter(|| black_box(nested.clone()));
    });
    group.finish();
}

fn entries(length: u64) -> Vec<(Scalar, Scalar)> {
    (0..length)
        .map(|index| (Scalar::from(index), Scalar::from(index.to_string())))
        .collect()
}

fn nested(depth: usize) -> Scalar {
    (0..depth).fold(Scalar::Null, |value, _| Scalar::from_sequence([value]))
}
