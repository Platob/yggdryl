#[path = "datatype/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    datatype,
    benchmarks::floating::decimal_benchmarks,
    benchmarks::temporal::time_builder_benchmarks,
    benchmarks::temporal::time_unit_benchmarks,
    benchmarks::parser::parser_benchmarks,
    benchmarks::nested::value_benchmarks,
    benchmarks::default::default_and_compatibility_benchmarks,
    benchmarks::arrow::arrow_benchmarks,
    benchmarks::geospatial::geospatial_benchmarks
);
criterion_main!(datatype);
