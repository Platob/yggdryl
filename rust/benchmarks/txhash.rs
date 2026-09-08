#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "txhash/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    txhash,
    benchmarks::values::coupling_benchmarks,
    benchmarks::values::value_benchmarks,
    benchmarks::values::instant_benchmarks,
    benchmarks::arrow::column_benchmarks,
    benchmarks::arrow::holder_fill_benchmarks
);
criterion_main!(txhash);
