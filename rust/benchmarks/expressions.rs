use criterion::{criterion_group, criterion_main};

#[path = "expressions/mod.rs"]
mod expression_benches;

criterion_group!(
    expressions,
    expression_benches::parse::benchmarks,
    expression_benches::bind::benchmarks,
    expression_benches::eval::benchmarks,
    expression_benches::prune::benchmarks,
);
criterion_main!(expressions);
