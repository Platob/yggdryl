#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "types/datatype/mod.rs"]
mod datatype;
#[path = "types/enums.rs"]
mod enums;
#[path = "types/field.rs"]
mod field;

use criterion::criterion_group;
#[cfg(not(windows))]
use criterion::criterion_main;

criterion_group!(
    types,
    datatype::floating::decimal_benchmarks,
    datatype::temporal::time_builder_benchmarks,
    datatype::temporal::time_unit_benchmarks,
    datatype::value::value_benchmarks,
    datatype::version::version_benchmarks,
    datatype::parser::parser_benchmarks,
    datatype::nested::value_benchmarks,
    datatype::default::default_and_compatibility_benchmarks,
    datatype::arrow::arrow_benchmarks,
    datatype::geospatial::geospatial_benchmarks,
    datatype::ascii::ascii_benchmarks,
    field::field_benches::parser::benchmarks,
    field::field_benches::value::benchmarks,
    field::field_benches::integer::benchmarks,
    field::field_benches::comparison::benchmarks,
    field::field_benches::arrow::benchmarks,
    enums::mime_parsing,
    enums::media_inference,
    enums::write_modes_and_io_identity,
);
#[cfg(not(windows))]
criterion_main!(types);

#[cfg(windows)]
fn main() {
    // Windows executables default to a 1-MiB main stack. Keep the near-limit
    // parser corpus identical to other platforms by running the whole harness
    // once on the same 8-MiB stack Rust gives test workers.
    std::thread::Builder::new()
        .name("types-benchmarks".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(types)
        .expect("the types benchmark worker must start")
        .join()
        .expect("the types benchmark worker must complete");
}
