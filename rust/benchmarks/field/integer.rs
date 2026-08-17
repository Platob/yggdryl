use std::hint::black_box;

use criterion::Criterion;
use yggdryl::field::{Int64Field, integer};
use yggdryl::{DataType, Field};

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("typed/integer");
    group.bench_function("static_construct", |bencher| {
        bencher.iter(|| Int64Field::new(black_box("id"), black_box(false)));
    });
    group.bench_function("checked_borrow", |bencher| {
        let field = Field::new("id", DataType::Int64, false);
        bencher.iter(|| {
            black_box(&field)
                .try_as_typed::<integer::Int64>()
                .expect("the benchmark field has the checked marker")
        });
    });
    group.finish();
}
