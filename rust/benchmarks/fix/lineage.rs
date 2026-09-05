use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field, FixLineageEntry, FixPedigree, FixRegistry, Version};

use super::{LARGE_FIELDS, generated, seed};

/// The versions a dated dictionary is read at.
fn version(text: &str) -> Version {
    text.parse().expect("a valid version")
}

/// One field carrying the worked lineage: renamed once, retyped once.
///
/// The historical spelling is derived from `name`, because `set_lineage`
/// rewrites the aliases from it and one shared old name would make every
/// generated field claim the same alias.
fn dated(name: &str, tag: i32) -> Field {
    let was = format!("{name}Was");
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_tag(tag).expect("a static tag");
    field
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("2.7"), None))
                .with_name(&was)
                .with_dtype("int"),
            FixLineageEntry::new(FixPedigree::new(version("4.2"), Some(204)))
                .with_name(&was)
                .with_dtype("Qty"),
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None))
                .with_name(name)
                .with_dtype("utf8"),
        ])
        .expect("a lineage agreeing with its field");
    field
}

/// The tracked seed beside `count` dated fields.
fn dictionary(count: usize) -> FixRegistry {
    let dated = (0..count).map(|index| {
        dated(
            &format!("Dated{index:05}"),
            i32::try_from(5_000 + index).expect("a small tag"),
        )
    });
    FixRegistry::from_fields(seed().iter().cloned().chain(dated))
        .expect("the generated dictionary has no conflict")
}

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = dictionary(LARGE_FIELDS);
    let field = dated("LastQty", 32);
    let view = field.as_fix();
    let old = version("4.2");
    let newest = version("5.0SP2");
    let mut group = criterion.benchmark_group("fix/lineage");

    // The borrowed scan, against the undated read it costs more than.
    group.bench_function("name_at_newest", |bencher| {
        bencher.iter(|| black_box(&view).name_at(black_box(newest)));
    });
    group.bench_function("name_at_old", |bencher| {
        bencher.iter(|| black_box(&view).name_at(black_box(old)));
    });
    group.bench_function("baseline_name", |bencher| {
        bencher.iter(|| black_box(&field).name());
    });
    group.bench_function("defined_at", |bencher| {
        bencher.iter(|| black_box(&view).defined_at(black_box(old)));
    });
    group.bench_function("since", |bencher| {
        bencher.iter(|| black_box(&view).since());
    });
    // The two costs a one-entry read is made of, so the scan is separable
    // from the metadata lookup under it and from parsing a version.
    group.bench_function("baseline_property_get", |bencher| {
        bencher.iter(|| black_box(&view).description());
    });
    group.bench_function("baseline_version_parse", |bencher| {
        bencher.iter(|| black_box("5.0SP2").parse::<Version>());
    });
    // Resolving a datatype is what a lineage read costs when it must build
    // one, so the two are reported apart.
    group.bench_function("dtype_at", |bencher| {
        bencher.iter(|| black_box(&view).dtype_at(black_box(old)));
    });

    // A version filter over a whole dictionary, against the unfiltered read.
    group.bench_function("field_at_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_at(black_box(newest), black_box(5_000)));
    });
    group.bench_function("baseline_field_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field(black_box(5_000)));
    });
    // Both derived answers walk every lineage the dictionary holds, so they
    // are the cost a caller pays once rather than per read.
    group.bench_function("newest", |bencher| {
        bencher.iter(|| black_box(&registry).newest());
    });
    group.bench_function("versions", |bencher| {
        bencher.iter(|| black_box(&registry).versions());
    });

    // Writing derives the aliases and re-renders the document once.
    let entries: Vec<FixLineageEntry<'_>> = view
        .lineage()
        .map(|entry| entry.expect("canonical"))
        .collect();
    group.bench_function("set_lineage", |bencher| {
        bencher.iter_batched_ref(
            || generated(1).remove(0),
            |field| {
                field
                    .as_fix_mut()
                    .set_lineage(black_box(&entries))
                    .expect("a lineage stating no name");
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}
