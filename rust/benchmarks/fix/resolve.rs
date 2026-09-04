use std::collections::HashMap;
use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{Field, FixKey, FixRegistry};

use super::{generated, seed};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let mut group = criterion.benchmark_group("fix/resolve");

    // The four outcomes a lookup has, over the tracked seed.
    group.bench_function("tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(55)));
    });
    group.bench_function("alternate_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(20)));
    });
    group.bench_function("name_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("Symbol")));
    });
    group.bench_function("name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("SYMBOL")));
    });
    group.bench_function("alias_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("ticker")));
    });
    group.bench_function("tag_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(7)));
    });
    group.bench_function("name_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("absent")));
    });

    // The generic pair against the specialized one it redirects to.
    group.bench_function("generic_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field(black_box(FixKey::Tag(55))));
    });
    group.bench_function("generic_name_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field(black_box("Symbol")));
    });
    group.bench_function("field_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).field(black_box(55)).unwrap());
    });

    // A path at one, two and three segments.
    group.bench_function("path_1_segment", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_path(black_box("NoPartyIDs")));
    });
    group.bench_function("path_2_segments", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_path(black_box("NoPartyIDs.PartyID")));
    });
    group.bench_function("path_3_segments", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_field_by_path(black_box("NoPartyIDs.item.PartyRole"))
        });
    });

    // The plain map the index structure has to beat: a tag map with no
    // tiers, and a lowercase name map that must fold the query to probe it.
    let by_tag: HashMap<i32, Field> = registry
        .iter()
        .map(|field| (field.as_fix().tag().unwrap().unwrap(), field.clone()))
        .collect();
    let by_name: HashMap<String, Field> = registry
        .iter()
        .map(|field| (field.name().to_ascii_lowercase(), field.clone()))
        .collect();
    group.bench_function("baseline_hashmap_tag_hit", |bencher| {
        bencher.iter(|| black_box(&by_tag).get(black_box(&55)));
    });
    group.bench_function("baseline_hashmap_name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&by_name).get(&black_box("SYMBOL").to_ascii_lowercase()));
    });

    // The same hits over a few thousand fields.
    let large = FixRegistry::from_fields(registry.iter().cloned().chain(generated(4_000)))
        .expect("the generated dictionary has no conflict");
    group.bench_function("tag_hit_4000", |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_tag(black_box(7_000)));
    });
    group.bench_function("name_hit_4000", |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_name(black_box("generated02000")));
    });
    group.bench_function("alias_hit_4000", |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_name(black_box("GENERATEDALIAS02000")));
    });
    group.finish();
}
