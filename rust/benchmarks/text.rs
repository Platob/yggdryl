#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "text/fixtures.rs"]
mod fixtures;
#[path = "text/json.rs"]
mod json;
#[path = "text/line.rs"]
mod line;
#[path = "text/placeholder.rs"]
mod placeholder;
#[path = "text/toml.rs"]
mod toml;
#[path = "text/value.rs"]
mod value;
#[path = "text/yaml.rs"]
mod yaml;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    text,
    value::value_benchmarks,
    placeholder::placeholder_benchmarks,
    line::text_options_benchmarks,
    line::text_records_benchmarks,
    json::format::json_benchmarks,
    toml::format::toml_benchmarks,
    yaml::format::yaml_benchmarks,
);
criterion_main!(text);
