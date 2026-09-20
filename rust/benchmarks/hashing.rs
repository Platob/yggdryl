#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "hashing/mod.rs"]
mod benchmarks;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    hashing,
    benchmarks::xxhash::oneshot::size_benchmarks,
    benchmarks::xxhash::oneshot::wrapper_benchmarks,
    benchmarks::xxhash::oneshot::streaming_benchmarks,
    benchmarks::xxhash::handles::handle_benchmarks,
    benchmarks::xxhash::values::value_benchmarks,
    benchmarks::xxhash::values::stable_hash_benchmarks,
    benchmarks::variant::variant_benchmarks,
    benchmarks::xxhash::arrow::row_digest_benchmarks,
    benchmarks::xxhash::arrow::holder_fill_benchmarks,
    benchmarks::txhash::values::coupling_benchmarks,
    benchmarks::txhash::values::value_benchmarks,
    benchmarks::txhash::values::instant_benchmarks,
    benchmarks::txhash::arrow::column_benchmarks,
    benchmarks::txhash::arrow::holder_fill_benchmarks
);
criterion_main!(hashing);
