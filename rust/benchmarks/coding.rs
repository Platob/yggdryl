#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "coding/stream.rs"]
mod stream;

use criterion::{criterion_group, criterion_main};

criterion_group!(coding, stream::byte_stream_benchmarks);
criterion_main!(coding);
