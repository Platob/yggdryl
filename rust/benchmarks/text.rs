#[path = "text/value.rs"]
mod value;

use criterion::{criterion_group, criterion_main};

criterion_group!(text, value::value_benchmarks);
criterion_main!(text);
