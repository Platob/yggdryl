#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "arrowfs/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    arrowfs,
    benchmarks::bytes::byte_benchmarks,
    benchmarks::record::record_benchmarks,
    benchmarks::listing::listing_benchmarks
);
criterion_main!(arrowfs);
