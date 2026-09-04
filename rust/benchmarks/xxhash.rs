#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "xxhash/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    xxhash,
    benchmarks::oneshot::size_benchmarks,
    benchmarks::oneshot::wrapper_benchmarks,
    benchmarks::oneshot::streaming_benchmarks,
    benchmarks::handles::handle_benchmarks,
    benchmarks::values::value_benchmarks,
    benchmarks::values::stable_hash_benchmarks,
    benchmarks::arrow::row_digest_benchmarks
);
criterion_main!(xxhash);
