#[path = "text/fixtures.rs"]
mod fixtures;
#[path = "toml/format.rs"]
mod format;

use criterion::{criterion_group, criterion_main};

criterion_group!(toml, format::toml_benchmarks);
criterion_main!(toml);
