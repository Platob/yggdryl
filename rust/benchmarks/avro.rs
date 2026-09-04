#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "avro/codecs.rs"]
mod codecs;
#[path = "avro/container.rs"]
mod container;
#[path = "avro/format.rs"]
mod format;
#[path = "avro/projection.rs"]
mod projection;
#[path = "avro/resolution.rs"]
mod resolution;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    avro,
    container::avro_benchmarks,
    format::format_benchmarks,
    codecs::codec_benchmarks,
    projection::projection_benchmarks,
    resolution::resolution_benchmarks,
);
criterion_main!(avro);
