#[path = "bench_profile.rs"]
mod bench_profile;

use criterion::{criterion_group, criterion_main};

#[path = "fix/mod.rs"]
mod fix_benches;

criterion_group!(
    fix,
    fix_benches::resolve::benchmarks,
    fix_benches::mutate::benchmarks,
    fix_benches::store::benchmarks,
);
criterion_main!(fix);
