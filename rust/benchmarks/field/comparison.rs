use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field, OwnedDifferences};

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("comparison");
    let field = nested_field();
    group.bench_function("equals_without_metadata", |bencher| {
        let right = field
            .clone()
            .try_with_metadata("transport", "arrow")
            .expect("the static metadata is valid");
        bencher.iter(|| black_box(&field).equals(black_box(&right), false));
    });
    group.bench_function("diff_iterator_setup", |bencher| {
        let right = field.clone().with_name("renamed");
        bencher.iter(|| black_box(&field).show_diffs(black_box(&right), true, false));
    });
    group.bench_function("diff_consume", |bencher| {
        let right = field
            .clone()
            .with_name("renamed")
            .try_with_metadata("source", "changed")
            .expect("the static metadata is valid");
        bencher.iter(|| {
            black_box(&field)
                .show_diffs(black_box(&right), true, false)
                .for_each(|line| {
                    black_box(line);
                });
        });
    });
    let deep = |leaf: DataType| {
        let mut data_type = leaf;
        for _ in 0..64 {
            data_type = DataType::list(Field::new("item", data_type, true));
        }
        Field::new("root", data_type, false)
    };
    let deep_left = deep(DataType::Utf8);
    let deep_right = deep(DataType::Int64);
    group.bench_function("diff_first_deep_64", |bencher| {
        bencher.iter(|| {
            black_box(&deep_left)
                .show_diffs(black_box(&deep_right), true, false)
                .next()
        });
    });
    let wide =
        |prefix: &str| {
            Field::new(
                "root",
                DataType::from_fields((0..1_024).map(|index| {
                    Field::new(format!("{prefix}_{index:04}"), DataType::Int64, false)
                }))
                .expect("the generated field names are unique"),
                false,
            )
        };
    let wide_left = wide("left");
    let wide_right = wide("right");
    group.bench_function("diff_first_wide_struct_1024", |bencher| {
        bencher.iter(|| {
            black_box(&wide_left)
                .show_diffs(black_box(&wide_right), true, false)
                .next()
        });
    });
    group.bench_function("diff_owned_setup_wide_struct_1024", |bencher| {
        bencher.iter(|| {
            OwnedDifferences::from_fields(
                black_box(&wide_left),
                black_box(&wide_right),
                true,
                false,
            )
        });
    });
    group.bench_function("diff_owned_first_wide_struct_1024", |bencher| {
        bencher.iter(|| {
            OwnedDifferences::from_fields(
                black_box(&wide_left),
                black_box(&wide_right),
                true,
                false,
            )
            .next()
        });
    });
    group.finish();
}
