#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

use criterion::criterion_group;

#[path = "graph/mod.rs"]
mod graph_benches;

criterion_group!(
    graph,
    graph_benches::book::benchmarks,
    graph_benches::candle::benchmarks,
    graph_benches::identifier::benchmarks,
    graph_benches::isin_registry::benchmarks,
    graph_benches::view::benchmarks
);
fn main() {
    // The kinds and names the benchmarks read are claimed before any runs.
    yggdryl_market::install().expect("yggdryl-market installs");
    graph();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
