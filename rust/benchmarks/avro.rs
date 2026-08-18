#[path = "avro/container.rs"]
mod container;

use criterion::{criterion_group, criterion_main};

criterion_group!(avro, container::avro_benchmarks);
criterion_main!(avro);
