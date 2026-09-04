#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "io/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    io,
    benchmarks::record::round_trip_benchmarks,
    benchmarks::dimensions::dimension_benchmarks,
    benchmarks::write::write_surface_benchmarks,
    benchmarks::stream::byte_stream_benchmarks,
    benchmarks::value::structured_scalar_benchmarks,
    benchmarks::pushdown::projection_benchmarks,
    benchmarks::buffered::buffered_benchmarks,
    benchmarks::listing::listing_benchmarks
);
criterion_main!(io);
