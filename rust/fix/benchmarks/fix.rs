#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

use criterion::criterion_group;

#[path = "fix/mod.rs"]
mod fix_benches;

criterion_group!(
    fix,
    fix_benches::resolve::benchmarks,
    fix_benches::cblock::benchmarks,
    fix_benches::mutate::benchmarks,
    fix_benches::pipeline::benchmarks,
    fix_benches::pipeline::line_benchmarks,
    fix_benches::ulbridge::benchmarks,
    fix_benches::store::benchmarks,
);
fn main() {
    // The kinds and names the benchmarks read are claimed before any runs.
    yggdryl_fix::install().expect("yggdryl-fix installs");
    fix();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
