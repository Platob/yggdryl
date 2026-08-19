#[path = "io/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    io,
    benchmarks::record::round_trip_benchmarks,
    benchmarks::pushdown::projection_benchmarks,
    benchmarks::lines::lines_arrow_benchmarks,
    benchmarks::lines::lines_gzip_benchmarks,
    benchmarks::buffered::buffered_benchmarks
);
criterion_main!(io);
