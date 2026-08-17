use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Field;

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("parse");
    group.bench_function("field_canonical", |bencher| {
        let canonical = nested_field().to_string();
        bencher.iter(|| {
            Field::from_str(black_box(&canonical))
                .expect("canonical display output must round-trip")
        });
    });
    group.bench_function("field_display_parse_round_trip", |bencher| {
        let field = nested_field();
        bencher.iter(|| {
            let canonical = black_box(&field).to_string();
            Field::from_str(black_box(&canonical))
                .expect("canonical display output must round-trip")
        });
    });
    group.finish();
}
