#[path = "text/fixtures.rs"]
mod fixtures;
#[path = "yaml/format.rs"]
mod format;

use criterion::{criterion_group, criterion_main};

criterion_group!(yaml, format::yaml_benchmarks);
criterion_main!(yaml);
