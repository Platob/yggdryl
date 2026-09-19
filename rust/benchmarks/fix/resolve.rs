//! Registry lookups: fields by tag, name, alias, identifier and path; the
//! code sets a field carries, read borrowed.

use criterion::Criterion;
use std::collections::HashMap;
use std::hint::black_box;
use yggdryl::SequenceType;
use yggdryl::graph::{Element, Event};
use yggdryl::{
    DataType, Field, FieldPath, FixCode, FixCodeValue, FixCodec, FixId, FixKey, MimeType,
};

use super::{DIALECT_FIELDS, LARGE_FIELDS, generated, mixed_categories, seed, two_dialects, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
    assert!(matches!(
        registry.field_by_name("Parties").unwrap().dtype(),
        DataType::Sequence(SequenceType::List(_))
    ));
    assert_eq!(
        registry.field_by_counter(453).unwrap(),
        registry.field_by_name("Parties").unwrap()
    );
    assert_eq!(
        registry.field("LastShares").unwrap(),
        registry.field(32).unwrap()
    );
    assert!(registry.get_field_by_tag(i32::MAX).is_none());
    assert!(registry.get_field_by_name("absent").is_none());
    let mut alternate = registry.clone();
    let mut field = DataType::utf8().nullable_field("AlternateTagBenchmark");
    field.as_fix_mut().set_tag(9_000).unwrap();
    field.as_fix_mut().set_tags(&[9_001]).unwrap();
    alternate.insert(field).unwrap();
    assert_eq!(
        alternate.field(9_001).unwrap(),
        alternate.field(9_000).unwrap()
    );
    let mut group = criterion.benchmark_group("fix/resolve");

    let identifiers = registry.msgtype("D").unwrap();
    let identifier_message = FixCodec::new(std::sync::Arc::new(registry.clone()))
        .parse_fix_line(b"8=FIX.4.4|35=D|11=ORDER-1|37=VENUE-1|10=000|")
        .unwrap();
    group.bench_function("identifier_declaration", |bencher| {
        bencher.iter(|| {
            for name in black_box(identifiers).as_field().as_fix().identifiers() {
                black_box(name);
            }
        });
    });
    group.bench_function("identifier_selection", |bencher| {
        bencher.iter(|| {
            for value in black_box(identifiers).identifier_values(black_box(&identifier_message)) {
                black_box(value);
            }
        });
    });
    // The settled identity, read through the graph traits: four copies.
    group.bench_function("settled_identity_reads", |bencher| {
        bencher.iter(|| {
            let held = black_box(&identifier_message);
            black_box((
                held.get_currunix(),
                held.get_creatunix(),
                held.get_currhashcode(),
                held.get_crosshashcode(),
            ))
        });
    });

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
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("Parties")));
    });
    group.bench_function("group_counter_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_counter(black_box(453)));
    });
    group.bench_function("alternate_tag_hit", |bencher| {
        bencher.iter(|| black_box(&alternate).get_field_by_tag(black_box(9_001)));
    });
    group.bench_function("name_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("Symbol")));
    });
    group.bench_function("name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("SYMBOL")));
    });
    group.bench_function("alias_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("LastShares")));
    });
    group.bench_function("tag_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_tag(black_box(i32::MAX)));
    });
    group.bench_function("name_miss", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_name(black_box("absent")));
    });
    let fixml = b"8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|";
    group.bench_function("infer_fixml_protocol", |bencher| {
        bencher.iter(|| MimeType::infer_bytes(black_box(fixml)));
    });
    group.bench_function("infer_fixml_msgtype", |bencher| {
        bencher.iter(|| FixCodec::infer_msgtype_bytes(black_box(fixml)));
    });

    // The identifier's own derivation and render: an id is the fold of a tag
    // and a name, computed on every read of a field, and spelled as its
    // decimal digest wherever it crosses a boundary.
    let symbol_id = FixId::of(55, "Symbol").expect("a positive tag");
    assert_eq!(symbol_id, FixId::of(55, "SYMBOL").expect("a positive tag"));
    group.bench_function("id_render", |bencher| {
        bencher.iter(|| black_box(&symbol_id).to_string());
    });
    group.bench_function("id_of", |bencher| {
        bencher.iter(|| FixId::of(black_box(55), black_box("Symbol")).unwrap());
    });
    group.bench_function("id_of_folded", |bencher| {
        bencher.iter(|| FixId::of(black_box(35), black_box("Msg_Type")).unwrap());
    });
    group.bench_function("field_id", |bencher| {
        let symbol = registry.field(55).unwrap();
        bencher.iter(|| black_box(symbol).as_fix().id().unwrap().unwrap());
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

    // A path at one, two and three segments, each resolved once outside the
    // routine: a path is read where a caller states it, and what is measured
    // here is the walk it drives, not the parse that made it.
    let paths: Vec<FieldPath> = [
        "NoPartyIDs",
        "Parties.PartyID",
        "Parties.PtysSubGrp.PartySubID",
    ]
    .into_iter()
    .map(|spelling| FieldPath::from_str(spelling).expect("a path"))
    .collect();
    for path in &paths {
        assert!(registry.get_field_by_path(path).is_some(), "{path}");
    }
    for (segments, path) in paths.iter().enumerate() {
        group.bench_function(format!("path_{}_segments", segments + 1), |bencher| {
            bencher.iter(|| black_box(&registry).get_field_by_path(black_box(path)));
        });
    }

    // What reading one costs, measured on its own so the walk above is not
    // credited with it: a caller resolving per row pays this once a row.
    group.bench_function("path_parse_2_segments", |bencher| {
        bencher.iter(|| FieldPath::from_str(black_box("Parties.PartyID")).expect("a path"));
    });

    // The plain map the index structure has to beat: a tag map, and a
    // lowercase name map that must fold the query to probe it.
    let by_tag: HashMap<i32, Field> = registry
        .iter()
        .map(|field| (field.as_fix().tag().unwrap().unwrap(), field.clone()))
        .collect();
    let by_name: HashMap<String, Field> = registry
        .iter()
        .map(|field| (field.name().to_ascii_lowercase(), field.clone()))
        .collect();
    // The id baseline: the derived identity in a hash map, probed with the
    // id already in hand, which is what `get_field_by_id` has to earn itself
    // against. `HashMap<i32, Field>` above answers a strictly weaker
    // question - it cannot hold two fields on one tag at all.
    let by_id: HashMap<FixId, Field> = registry
        .iter()
        .map(|field| (field.as_fix().id().unwrap().unwrap(), field.clone()))
        .collect();
    group.bench_function("baseline_hashmap_tag_hit", |bencher| {
        bencher.iter(|| black_box(&by_tag).get(black_box(&55)));
    });
    group.bench_function("baseline_hashmap_id_hit", |bencher| {
        bencher.iter(|| black_box(&by_id).get(black_box(&symbol_id)));
    });
    group.bench_function("id_hit", |bencher| {
        bencher.iter(|| black_box(&registry).get_field_by_id(black_box(symbol_id)));
    });
    group.bench_function("baseline_hashmap_name_hit_folded", |bencher| {
        bencher.iter(|| black_box(&by_name).get(&black_box("SYMBOL").to_ascii_lowercase()));
    });

    // Two dictionaries in one registry: the venue's fields by id, name and
    // alias, its tags beside the standard ones, membership stamped and read
    // as provenance rather than consulted by any lookup.
    let venue = venue();
    let mixed = two_dialects(DIALECT_FIELDS);
    let dialect_middle = DIALECT_FIELDS / 2;
    let vendor_tag = i32::try_from(5_000 + dialect_middle).expect("the vendor tag fits i32");
    let vendor_name = format!("vendor{dialect_middle:05}");
    let vendor_alias = format!("VendorAlias{dialect_middle:05}");
    let vendor_id = FixId::of(vendor_tag, &vendor_name).expect("a vendor identifier");
    let vendor = mixed.field(vendor_id).expect("the venue field by its id");
    assert!(vendor.as_fix().has_branch(venue));
    assert!(!mixed.field(55).unwrap().as_fix().has_branch(venue));
    assert_eq!(mixed.dialects(), [venue.to_owned()]);
    group.bench_function("has_branch_vendor", |bencher| {
        bencher.iter(|| black_box(vendor).as_fix().has_branch(black_box(venue)));
    });
    group.bench_function("id_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_id(black_box(vendor_id)));
    });
    group.bench_function("name_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_name(black_box(&vendor_name)));
    });
    group.bench_function("alias_hit_vendor", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_name(black_box(&vendor_alias)));
    });
    group.bench_function("tag_hit_vendor_inferred", |bencher| {
        bencher.iter(|| black_box(&mixed).get_field_by_tag(black_box(vendor_tag)));
    });
    group.bench_function("tag_hit_two_dialects", |bencher| {
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
        bencher.iter(|| black_box(&large).get_field_by_name(black_box(&middle_name)));
    });
    group.bench_function(format!("alias_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| black_box(&large).get_field_by_name(black_box(&middle_alias)));
    });

    // One scalar in fifty counts a separately named repeating group; tag 5000
    // is the first counter and 5001 is the ordinary scalar beside it.
    let mut realistic = registry.clone();
    realistic
        .add_fields(mixed_categories(LARGE_FIELDS))
        .expect("the generated dictionary has no conflict");
    for index in (0..LARGE_FIELDS).step_by(50) {
        let item =
            yggdryl::DataType::from_fields([yggdryl::DataType::utf8().nullable_field("Member")])
                .unwrap()
                .required_field("item");
        let mut field = yggdryl::DataType::list(item).nullable_field(format!("Group{index:05}"));
        field
            .as_fix_mut()
            .set_counter(i32::try_from(5_000 + index).unwrap())
            .unwrap();
        realistic.insert(field).unwrap();
    }
    group.bench_function(format!("field_tag_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| black_box(&realistic).get_field_by_tag(black_box(5_001)));
    });
    group.bench_function(format!("group_name_hit_{LARGE_FIELDS}"), |bencher| {
        bencher.iter(|| black_box(&realistic).get_field_by_name(black_box("Group00000")));
    });
    group.finish();

    codes(criterion);
}

/// A code set of `count` members, each carrying a description tier 3 reads.
fn vocabulary(count: usize) -> Field {
    let codes: Vec<FixCode> = (0..count)
        .map(|index| {
            FixCode::new(format!("Member{index:04}"), format!("{index:04}"))
                .with_description(format!("Member number {index} (M{index:04})"))
        })
        .collect();
    let mut field = DataType::utf8().nullable_field("Vocabulary");
    field.as_fix_mut().set_tag(9995).expect("a static tag");
    field
        .as_fix_mut()
        .set_codes(&codes)
        .expect("a valid code set");
    field
}

/// A field's code set read borrowed, at three sizes, against a map.
fn codes(criterion: &mut Criterion) {
    // Three sizes, because a FIX code set is usually small and occasionally
    // not: `Side` has ten members and `PartyRole` has hundreds.
    let small = vocabulary(10);
    let medium = vocabulary(60);
    let large = vocabulary(300);
    let mut group = criterion.benchmark_group("fix/codes");

    for (label, field) in [("10", &small), ("60", &medium), ("300", &large)] {
        let view = field.as_fix();
        let last = format!("{:04}", field.as_fix().codes().count() - 1);

        // Tier 1 stops at the match, so the first and last member bracket it.
        group.bench_function(format!("{label}/value_first"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box("0000")));
        });
        group.bench_function(format!("{label}/value_last"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box(last.as_str())));
        });
        group.bench_function(format!("{label}/value_miss"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box("absent")));
        });
        // Tier 2 runs the whole set whatever it finds, because two codes
        // folding to one spelling must answer nothing rather than the first.
        group.bench_function(format!("{label}/name_folded"), |bencher| {
            bencher.iter(|| black_box(&view).code_value(black_box("member_0000")));
        });

        // The baseline: the same document as a map. Built once and read many
        // times it wins, which is what the two rows are for - a caller
        // resolving one spelling pays the build, and one resolving a million
        // should build the map itself.
        let map: HashMap<&str, FixCodeValue<'_>> = view
            .codes()
            .map(|code| {
                let code = code.expect("a canonical document");
                (code.value(), code)
            })
            .collect();
        group.bench_function(format!("{label}/baseline_map_hit"), |bencher| {
            bencher.iter(|| black_box(&map).get(black_box("0000")));
        });
        group.bench_function(format!("{label}/baseline_map_build_and_hit"), |bencher| {
            bencher.iter(|| {
                let built: HashMap<&str, FixCodeValue<'_>> = black_box(&view)
                    .codes()
                    .filter_map(|code| code.ok().map(|code| (code.value(), code)))
                    .collect();
                black_box(built.get(black_box("0000")).copied())
            });
        });
    }

    let view = large.as_fix();
    // Writing renders the whole document once, which is what a generator pays.
    let codes: Vec<FixCode> = view
        .codes()
        .map(|code| FixCode::from(code.expect("a canonical document")))
        .collect();
    group.bench_function("300/set_codes", |bencher| {
        bencher.iter_batched_ref(
            || DataType::utf8().nullable_field("Vocabulary"),
            |field| {
                field
                    .as_fix_mut()
                    .set_codes(black_box(&codes))
                    .expect("a valid code set");
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}
