#[path = "text/line.rs"]
mod line;
#[path = "text/value.rs"]
mod value;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    text,
    value::value_benchmarks,
    line::lines_arrow_benchmarks,
    line::lines_gzip_benchmarks,
    line::lines_shape_benchmarks
);
criterion_main!(text);
