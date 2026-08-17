#[path = "text/fixtures.rs"]
mod fixtures;
#[path = "json/format.rs"]
mod format;

use criterion::{criterion_group, criterion_main};

criterion_group!(json, format::json_benchmarks);
criterion_main!(json);
