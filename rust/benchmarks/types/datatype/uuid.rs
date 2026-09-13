//! UUID packing without hashing or a clock; allocations are pinned separately.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::types::Uuid;

pub(crate) fn uuid_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("uuid");
    group.bench_function("from_v7", |bencher| {
        bencher.iter(|| {
            Uuid::from_v7(
                black_box(1_645_557_742_000_456),
                black_box(0xfedc_ba98_7654_3210),
            )
            .expect("an in-range microsecond instant")
        });
    });
    group.bench_function("from_v8", |bencher| {
        bencher.iter(|| Uuid::from_v8(black_box(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6)));
    });
    group.finish();
}
