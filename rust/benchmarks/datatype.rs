#[path = "datatype/mod.rs"]
mod benchmarks;

use criterion::criterion_group;
#[cfg(not(windows))]
use criterion::criterion_main;

criterion_group!(
    datatype,
    benchmarks::floating::decimal_benchmarks,
    benchmarks::temporal::time_builder_benchmarks,
    benchmarks::temporal::time_unit_benchmarks,
    benchmarks::value::value_benchmarks,
    benchmarks::parser::parser_benchmarks,
    benchmarks::nested::value_benchmarks,
    benchmarks::default::default_and_compatibility_benchmarks,
    benchmarks::arrow::arrow_benchmarks,
    benchmarks::geospatial::geospatial_benchmarks
);
#[cfg(not(windows))]
criterion_main!(datatype);

#[cfg(windows)]
fn main() {
    // Windows executables default to a 1-MiB main stack. Keep the near-limit
    // parser corpus identical to other platforms by running the whole harness
    // once on the same 8-MiB stack Rust gives test workers.
    std::thread::Builder::new()
        .name("datatype-benchmarks".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(datatype)
        .expect("the datatype benchmark worker must start")
        .join()
        .expect("the datatype benchmark worker must complete");
}
