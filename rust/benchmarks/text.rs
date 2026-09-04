#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "text/line.rs"]
mod line;
#[path = "text/placeholder.rs"]
mod placeholder;
#[path = "text/value.rs"]
mod value;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    text,
    value::value_benchmarks,
    placeholder::placeholder_benchmarks,
    line::text_options_benchmarks,
    line::text_records_benchmarks
);
criterion_main!(text);
