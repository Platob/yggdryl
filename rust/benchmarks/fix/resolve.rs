use std::collections::HashMap;
use std::hint::black_box;

use criterion::Criterion;
use yggdryl::FixCodec;
use yggdryl::{DataType, Field, FixBranch, FixCategory, FixId, FixKey, MimeType};

use super::{BRANCH_FIELDS, LARGE_FIELDS, generated, mixed_categories, seed, two_branches, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let standard = FixBranch::STANDARD;
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
    assert!(matches!(
        registry
            .definition(FixCategory::Groups, "Parties", None)
            .unwrap()
            .dtype(),
        DataType::List(_)
    ));
    assert_eq!(
        registry.field("LastShares").unwrap(),
        registry.field(32).unwrap()
    );
    assert!(registry.get_field_by_tag(i32::MAX).is_none());
    assert!(
        registry
            .get_field_by_name("absent", Some(&standard))
            .is_none()
    );
    let mut alternate = registry.clone();
    let mut field = DataType::Utf8.nullable_field("AlternateTagBenchmark");
    field.as_fix_mut().set_tag(9_000).unwrap();
    field.as_fix_mut().set_tags(&[9_001]).unwrap();
    alternate.insert(field).unwrap();
    assert_eq!(
        alternate.field(9_001).unwrap(),
        alternate.field(9_000).unwrap()
    );
    let mut group = criterion.benchmark_group("fix/resolve");

    // The four outcomes a lookup has, over the tracked seed.
    group.bench_function("tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(55)));
    });
    // Counters are scalar fields; logical groups have their own name index.
    group.bench_function("scalar_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(55)));
    });
    group.bench_function("counter_tag_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(453)));
    });
    group.bench_function("group_name_hit", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_definition(
                FixCategory::Groups,
                black_box("Parties"),
                Some(&standard),
            )
        });
    });
    group.bench_function("alternate_tag_hit", |bencher| {
        bencher.iter(|| black_box(&alternate).get_field_by_tag(black_box(9_001)));
    });
    group.bench_function("name_hit", |bencher| {
        bencher
            .iter(|| black_box(&registry).get_field_by_name(black_box("Symbol"), Some(&standard)));
    });
    group.bench_function("name_hit_folded", |bencher| {
        bencher
            .iter(|| black_box(&registry).get_field_by_name(black_box("SYMBOL"), Some(&standard)));
    });
    group.bench_function("alias_hit", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_field_by_name(black_box("LastShares"), Some(&standard))
        });
    });
    group.bench_function("tag_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(i32::MAX)));
    });
    group.bench_function("name_miss", |bencher| {
        bencher
            .iter(|| black_box(&registry).get_field_by_name(black_box("absent"), Some(&standard)));
    });
    let fixml = b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|";
    group.bench_function("infer_fixml_protocol", |bencher| {
        bencher.iter(|| MimeType::infer_bytes(black_box(fixml)));
    });
    group.bench_function("infer_fixml_msgtype", |bencher| {
        bencher.iter(|| FixCodec::infer_msgtype_bytes(black_box(fixml)));
    });

    // The identifier's own render and parse, which is what a config file or a
    // binding boundary spells an identity as.
    let standard_id = FixId::standard(55);
    group.bench_function("id_render", |bencher| {
        bencher.iter(|| black_box(&standard_id).to_string());
    });
    group.bench_function("id_parse", |bencher| {
        bencher.iter(|| FixId::from_str(black_box("5001:cme")).unwrap());
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
    for path in [
        "NoPartyIDs",
        "Parties.PartyID",
        "Parties.PtysSubGrp.PartySubID",
    ] {
        assert!(
            registry.get_field_by_path(path, Some(&standard)).is_some(),
            "{path}"
        );
    }
    group.bench_function("path_1_segment", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_field_by_path(black_box("NoPartyIDs"), Some(&standard))
        });
    });
    group.bench_function("path_2_segments", |bencher| {
        bencher.iter(|| {
            black_box(&registry).get_field_by_path(black_box("Parties.PartyID"), Some(&standard))
        });
    });
    group.bench_function("path_3_segments", |bencher| {
        bencher.iter(|| {
            black_box(&registry)
                .get_field_by_path(black_box("Parties.PtysSubGrp.PartySubID"), Some(&standard))
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
    // branch. `HashMap<i32, Field>` above answers a strictly weaker
    // question - it cannot hold two branches at all.
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

    // Two branches in one registry: explicit venue identifiers/names and an
    // inferred venue tag.
    let venue = venue();
    let mixed = two_branches(BRANCH_FIELDS);
    let branch_middle = BRANCH_FIELDS / 2;
    let vendor_tag = i32::try_from(5_000 + branch_middle).expect("the vendor tag fits i32");
    let vendor_name = format!("vendor{branch_middle:05}");
    let vendor_alias = format!("VendorAlias{branch_middle:05}");
    let vendor_id = FixId::from_parts(&venue, vendor_tag).expect("a vendor identifier");
    group.bench_function("id_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_id(black_box(vendor_id)));
    });
    group.bench_function("name_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_name(black_box(&vendor_name), Some(&venue)));
    });
    group.bench_function("alias_hit_vendor", |bencher| {
        bencher
            .iter(|| black_box(&mixed).get_field_by_name(black_box(&vendor_alias), Some(&venue)));
    });
    group.bench_function("tag_hit_vendor_inferred", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_tag(black_box(vendor_tag)));
    });
    group.bench_function("tag_hit_two_branches", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_tag(black_box(55)));
    });

    // The same hits over the lightweight release corpus.
    let mut large = registry.clone();
    large
        .add_fields(generated(LARGE_FIELDS))
        .expect("the generated dictionary has no conflict");
    let middle = LARGE_FIELDS / 2;
    let middle_tag = i32::try_from(5_000 + middle).expect("the middle tag fits i32");
    let middle_name = format!("generated{middle:05}");
    let middle_alias = format!("GENERATEDALIAS{middle:05}");
    group.bench_function(format!("tag_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_tag(black_box(middle_tag)));
    });
    group.bench_function(format!("name_hit_{LARGE_FIELDS}"), |bencher| {
        bencher
            .iter(|| black_box(&large).get_field_by_name(black_box(&middle_name), Some(&standard)));
    });
    group.bench_function(format!("alias_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| {
            black_box(&large).get_field_by_name(black_box(&middle_alias), Some(&standard))
        });
    });

    // One scalar in fifty counts a separately named repeating group; tag 5000
    // is the first counter and 5001 is the ordinary scalar beside it.
    let mut realistic = registry.clone();
    realistic
        .add_fields(mixed_categories(LARGE_FIELDS))
        .expect("the generated dictionary has no conflict");
    for index in (0..LARGE_FIELDS).step_by(50) {
        let item =
            yggdryl::DataType::from_fields([yggdryl::DataType::Utf8.nullable_field("Member")])
                .unwrap()
                .required_field("item");
        let mut field = yggdryl::DataType::list(item).nullable_field(format!("Group{index:05}"));
        field
            .as_fix_mut()
            .set_counter(i32::try_from(5_000 + index).unwrap())
            .unwrap();
        realistic
            .insert_definition(yggdryl::FixCategory::Groups, field)
            .unwrap();
    }
    group.bench_function(format!("field_tag_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| black_box(&realistic).get_field_by_tag(black_box(5_001)));
    });
    group.bench_function(format!("group_name_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| {
            black_box(&realistic).get_definition(
                yggdryl::FixCategory::Groups,
                black_box("Group00000"),
                None,
            )
        });
    });
    group.finish();
}
