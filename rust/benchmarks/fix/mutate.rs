use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::{
    DataType, Field, FixCategory, FixCode, FixLineageEntry, FixPedigree, FixRegistry, Version,
};

use super::{LARGE_FIELDS, generated, seed, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let mut large = registry.clone();
    large
        .add_fields(generated(LARGE_FIELDS))
        .expect("the generated dictionary has no conflict");
    let mut group = criterion.benchmark_group("fix/mutate");

    // One insert into a dictionary of each size. The clone is outside the
    // timer, and so is the drop: every routine hands the registry back as its
    // output rather than letting it fall at the end of the timed closure.
    let mut incoming = DataType::utf8().nullable_field("Incoming");
    incoming.as_fix_mut().set_tag(9_000).unwrap();
    incoming
        .as_fix_mut()
        .set_aliases(["IncomingAlias"])
        .unwrap();
    group.bench_function("insert_into_seed", |bencher| {
        bencher.iter_batched(
            || (registry.clone(), incoming.clone()),
            |(mut registry, field)| {
                black_box(registry.insert(field).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function(format!("insert_into_{LARGE_FIELDS}"), |bencher| {
        bencher.iter_batched(
            || (large.clone(), incoming.clone()),
            |(mut registry, field)| {
                black_box(registry.insert(field).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });

    // Building a whole dictionary, which is what a load costs above I/O.
    let fields: Vec<_> = large.iter().cloned().collect();
    group.bench_function(format!("from_fields_{LARGE_FIELDS}"), |bencher| {
        bencher.iter_batched(
            || fields.clone(),
            |fields| black_box(FixRegistry::from_fields(fields).unwrap()),
            BatchSize::SmallInput,
        );
    });

    // A merge that adds an alias and an alternate tag to a stored field.
    let mut update = DataType::utf8().nullable_field("Symbol");
    update.as_fix_mut().set_tag(55).unwrap();
    update.as_fix_mut().set_tags(&[9_001]).unwrap();
    update.as_fix_mut().set_aliases(["Sym"]).unwrap();
    group.bench_function("update_in_seed", |bencher| {
        bencher.iter_batched(
            || (registry.clone(), update.clone()),
            |(mut registry, field)| {
                registry.update(field).unwrap();
                registry
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function(format!("update_in_{LARGE_FIELDS}"), |bencher| {
        bencher.iter_batched(
            || (large.clone(), update.clone()),
            |(mut registry, field)| {
                registry.update(field).unwrap();
                registry
            },
            BatchSize::SmallInput,
        );
    });
    // The lenient verb, one staged mutation each: a scalar folding into the
    // field its name reaches, a Struct redirected to the components, and a
    // component gaining a member that every reference to it then carries -
    // which re-resolves the seed's whole catalog, and is the honest cost.
    let mut renamed = DataType::utf8().nullable_field("symbol");
    renamed.as_fix_mut().set_tag(9_001).unwrap();
    renamed.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    group.bench_function("add_field_same_name_merge", |bencher| {
        bencher.iter_batched(
            || (registry.clone(), renamed.clone()),
            |(mut registry, field)| {
                black_box(registry.add_field(field).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });
    let nested = DataType::from_fields([DataType::utf8().nullable_field("VenueSymbol")])
        .unwrap()
        .required_field("VenueInstrument");
    group.bench_function("add_field_nested_redirect", |bencher| {
        bencher.iter_batched(
            || (registry.clone(), nested.clone()),
            |(mut registry, field)| {
                black_box(registry.add_field(field).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });
    let mut seeded = registry.clone();
    let mut venue_symbol = DataType::utf8().nullable_field("VenueSymbol");
    venue_symbol.as_fix_mut().set_tag(9_010).unwrap();
    seeded.add_field(venue_symbol).unwrap();
    let mut member = seeded.field(9_010).unwrap().clone();
    member.as_fix_mut().set_field_ref("VenueSymbol").unwrap();
    let mut instrument = seeded
        .definition(FixCategory::Components, "Instrument")
        .unwrap()
        .clone();
    instrument
        .set_dtype(
            DataType::from_fields(instrument.fields().iter().cloned().chain([member])).unwrap(),
        )
        .unwrap();
    group.bench_function("add_definition_extends_component", |bencher| {
        bencher.iter_batched(
            || (seeded.clone(), instrument.clone()),
            |(mut registry, field)| {
                black_box(
                    registry
                        .add_definition(FixCategory::Components, field)
                        .unwrap(),
                );
                registry
            },
            BatchSize::SmallInput,
        );
    });
    let middle_tag = i32::try_from(5_000 + LARGE_FIELDS / 2).expect("the middle tag fits i32");
    group.bench_function(format!("remove_from_{LARGE_FIELDS}"), |bencher| {
        bencher.iter_batched(
            || large.clone(),
            |mut registry| {
                black_box(registry.remove(middle_tag).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });

    // The identity and membership setters: a tag on any field, the FIX
    // specification's own included, and the membership a dictionary stamps -
    // one name, and a list that is folded, deduplicated and sorted.
    let venue = venue();
    let mut movable = DataType::utf8().nullable_field("Movable");
    movable.as_fix_mut().set_tag(9_000).unwrap();
    group.bench_function("set_tag", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_tag(9_000).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_branches_one", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_branches([venue]).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_branches_folded", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field
                    .as_fix_mut()
                    .set_branches(["ulbridge", "CME", venue, "eurex"])
                    .unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut member = movable.clone();
    member.as_fix_mut().set_branches([venue]).unwrap();
    group.bench_function("add_branch", |bencher| {
        bencher.iter_batched(
            || member.clone(),
            |mut field| {
                field.as_fix_mut().add_branch("eurex").unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut reserved = DataType::utf8().nullable_field("Reserved");
    reserved.as_fix_mut().set_tag(35).unwrap();
    group.bench_function("set_tag_standard", |bencher| {
        bencher.iter_batched(
            || reserved.clone(),
            |mut field| {
                field
                    .as_fix_mut()
                    .set_tag(35)
                    .expect("nothing gates a tag on its dictionary");
                field
            },
            BatchSize::SmallInput,
        );
    });
    // The one FIX-aware merge, over two realistic definitions of one tag: a
    // generator folds several sources into every field it writes, so this is
    // what a regeneration costs per tag.
    let stored = merge_source("the stored wording", "2.7", "the stored reading");
    let incoming = merge_source("the incoming wording", "5.0.2", "the incoming reading");
    group.bench_function("merge_with", |bencher| {
        bencher.iter_batched(
            || incoming.clone(),
            |mut field| {
                field
                    .as_fix_mut()
                    .merge_with(&stored.as_fix())
                    .expect("two definitions of one tag");
                field
            },
            BatchSize::SmallInput,
        );
    });
    // The same fold through the registry, which adds the generic metadata
    // half and the reindexing a stored field needs.
    let mut merging = FixRegistry::from_fields([stored.clone()]).expect("one field");
    group.bench_function("update_merging", |bencher| {
        bencher.iter(|| {
            black_box(&mut merging)
                .update(black_box(incoming.clone()))
                .expect("the same identity")
        });
    });

    let target = coded_catalog();
    let mut source = target.clone();
    let mut incoming = source.field(448).unwrap().clone();
    incoming
        .as_fix_mut()
        .set_codes(&[FixCode::new("Client", "C")])
        .unwrap();
    source.insert(incoming).unwrap();
    group.bench_function("merge_catalog_inline_codes", |bencher| {
        bencher.iter_batched(
            || target.clone(),
            |mut target| {
                black_box(target.merge_with(black_box(&source)).unwrap());
                target
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn coded_catalog() -> FixRegistry {
    let mut party = DataType::utf8().nullable_field("PartyID");
    party.as_fix_mut().set_tag(448).unwrap();
    party
        .as_fix_mut()
        .set_codes(&[FixCode::new("Broker", "B")])
        .unwrap();
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453).unwrap();
    let mut registry = FixRegistry::from_fields([party.clone(), counter.clone()]).unwrap();
    party.as_fix_mut().set_field_ref("PartyID").unwrap();
    let component = DataType::from_fields([party])
        .unwrap()
        .required_field("Party");
    registry
        .create_definition(FixCategory::Components, component.clone())
        .unwrap();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry
        .create_definition(FixCategory::Groups, group)
        .unwrap();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group.as_fix_mut().set_group("Parties").unwrap();
    counter.as_fix_mut().set_field_ref("NoPartyIDs").unwrap();
    let mut message = DataType::from_fields([counter, group])
        .unwrap()
        .required_field("Order");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .create_definition(FixCategory::Messages, message)
        .unwrap();
    registry
}

/// One realistic definition of tag 32: dated, coded, described, aliased.
fn merge_source(wording: &str, dated: &str, reading: &str) -> Field {
    let version: Version = dated.parse().expect("a valid version");
    let mut field = DataType::utf8().nullable_field("LastQty");
    field.as_fix_mut().set_tag(32).expect("a static tag");
    field
        .as_fix_mut()
        .set_tags(&[65, 66])
        .expect("static alternate tags");
    field
        .as_fix_mut()
        .set_description(wording)
        .expect("a description");
    field
        .as_fix_mut()
        .set_lineage(&[FixLineageEntry::new(FixPedigree::new(version, None)).with_name("LastQty")])
        .expect("a lineage agreeing with its field");
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Shared", "1").with_description(reading),
            FixCode::new("Other", "2"),
        ])
        .expect("a valid code set");
    field
}
