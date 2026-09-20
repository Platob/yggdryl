#[path = "bench_profile.rs"]
mod bench_profile;

use criterion::{criterion_group, criterion_main};

#[path = "market/mod.rs"]
mod market_benches;

criterion_group!(
    market,
    market_benches::doors::benchmarks,
    market_benches::iterator::benchmarks,
    market_benches::rows::benchmarks,
);
criterion_main!(market);
