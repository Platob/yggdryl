#[path = "bench_profile.rs"]
mod bench_profile;

use criterion::{criterion_group, criterion_main};

#[path = "graph/mod.rs"]
mod graph_benches;

criterion_group!(
    graph,
    graph_benches::book::benchmarks,
    graph_benches::view::benchmarks
);
criterion_main!(graph);
