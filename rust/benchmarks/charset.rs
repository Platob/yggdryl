#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "charset/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    charset,
    benchmarks::decode::decode_benchmarks,
    benchmarks::decode::borrow_benchmarks,
    benchmarks::encode::encode_benchmarks,
    benchmarks::streaming::streaming_benchmarks,
);
criterion_main!(charset);
