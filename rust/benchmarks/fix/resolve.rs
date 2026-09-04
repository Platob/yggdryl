use std::collections::HashMap;
use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{Field, FixId, FixKey, FixNamespace, FixRegistry};

use super::{generated, seed, two_namespaces, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let standard = FixNamespace::STANDARD;
    let mut group = criterion.benchmark_group("fix/resolve");

    // The four outcomes a lookup has, over the tracked seed.
    group.bench_function("tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(55)));
    });
    group.bench_function("alternate_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(20)));
    });
    group.bench_function("name_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(&standard, black_box("Symbol")));
    });
    group.bench_function("name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(&standard, black_box("SYMBOL")));
    });
    group.bench_function("alias_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(&standard, black_box("ticker")));
    });
    group.bench_function("tag_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(7)));
    });
    group.bench_function("name_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(&standard, black_box("absent")));
    });

    // The identifier's own render and parse, which is what a config file or a
    // binding boundary spells an identity as.
    let standard_id = FixId::standard(55);
    group.bench_function("id_render", |bencher| {
        bencher.iter(|| black_box(&standard_id).to_string());
    });
    group.bench_function("id_parse", |bencher| {
        bencher.iter(|| FixId::from_str(black_box("cme:5001")).unwrap());
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
        bencher.iter(|| black_box(&registry).get_field_by_path(&standard, black_box("NoPartyIDs")));
    });
    group.bench_function("path_2_segments", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_field_by_path(&standard, black_box("NoPartyIDs.PartyID"))
        });
    });
    group.bench_function("path_3_segments", |bencher| {
        bencher.iter(|| {
            black_box(&registry)
                .get_field_by_path(&standard, black_box("NoPartyIDs.item.PartyRole"))
        });
    });

    // The plain map the index structure has to beat: a tag map with no
    // tiers, and a lowercase name map that must fold the query to probe it.
    // Re-measured here because the tag index is now keyed by `FixId`, so the
    // baseline the tag hit is compared against had to be run again.
    let by_tag: HashMap<i32, Field> = registry
        .iter()
        .map(|field| (field.as_fix().tag().unwrap().unwrap(), field.clone()))
        .collect();
    let by_name: HashMap<String, Field> = registry
        .iter()
        .map(|field| (field.name().to_ascii_lowercase(), field.clone()))
        .collect();
    // The equivalent baseline: the same composite key in a hash map, which is
    // what the ordered map has to earn itself against now that it carries a
    // namespace. `HashMap<i32, Field>` above answers a strictly weaker
    // question - it cannot hold two namespaces at all.
    let by_id: HashMap<FixId, Field> = registry
        .iter()
        .map(|field| (field.as_fix().id().unwrap().unwrap(), field.clone()))
        .collect();
    group.bench_function("baseline_hashmap_tag_hit", |bencher| {
        bencher.iter(|| black_box(&by_tag).get(black_box(&55)));
    });
    group.bench_function("baseline_hashmap_id_hit", |bencher| {
        bencher.iter(|| black_box(&by_id).get(&FixId::standard(black_box(55))));
    });
    group.bench_function("baseline_hashmap_name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&by_name).get(&black_box("SYMBOL").to_ascii_lowercase()));
    });

    // Two namespaces in one registry: a venue identifier, a venue name, and
    // the cross-namespace miss a bare tag must be.
    let venue = venue();
    let mixed = two_namespaces(1_000);
    let vendor_id = FixId::from_parts(venue.clone(), 5_500).expect("a vendor identifier");
    group.bench_function("id_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_id(black_box(&vendor_id)));
    });
    group.bench_function("name_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_name(&venue, black_box("vendor00500")));
    });
    group.bench_function("alias_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_name(&venue, black_box("VendorAlias00500")));
    });
    group.bench_function("cross_namespace_miss", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_tag(black_box(5_500)));
    });
    group.bench_function("tag_hit_two_namespaces", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_tag(black_box(55)));
    });

    // The same hits over a few thousand fields.
    let large = FixRegistry::from_fields(registry.iter().cloned().chain(generated(4_000)))
        .expect("the generated dictionary has no conflict");
    group.bench_function("tag_hit_4000", |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_tag(black_box(7_000)));
    });
    group.bench_function("name_hit_4000", |bencher| {
        bencher
            .iter(|| black_box(&large).get_field_by_name(&standard, black_box("generated02000")));
    });
    group.bench_function("alias_hit_4000", |bencher| {
        bencher.iter(|| {
            black_box(&large).get_field_by_name(&standard, black_box("GENERATEDALIAS02000"))
        });
    });
    group.finish();
}
