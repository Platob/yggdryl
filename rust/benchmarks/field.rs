use criterion::{criterion_group, criterion_main};

#[path = "field/mod.rs"]
mod field_benches;

criterion_group!(
    field,
    field_benches::parser::benchmarks,
    field_benches::value::benchmarks,
    field_benches::integer::benchmarks,
    field_benches::comparison::benchmarks,
    field_benches::arrow::benchmarks,
);
criterion_main!(field);
