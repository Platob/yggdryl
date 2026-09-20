use std::hint::black_box;

use criterion::Criterion;

use super::nested_field;

pub fn benchmarks(criterion: &mut Criterion) {
    let field = nested_field();
    let paths = [
        ("plain", "id"),
        ("nested_dotted", "events.value"),
        ("quoted_literal", r#""id""#),
        ("list_indexed", "events[0].value"),
    ];
    for (_, path) in paths {
        assert!(field.get_field_by_path(path).is_some(), "{path}");
    }

    let mut group = criterion.benchmark_group("field_path");
    for (name, path) in paths {
        group.bench_function(name, |bencher| {
            bencher.iter(|| black_box(&field).get_field_by_path(black_box(path)));
        });
    }
    group.finish();
}
