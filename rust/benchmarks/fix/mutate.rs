use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::{DataType, Field, FixCode, FixField, FixFieldMut, FixRegistry, StructType};

use super::{LARGE_FIELDS, generated, seed, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let mut large = registry.clone();
    large
        .add_fields(generated(LARGE_FIELDS))
        .expect("the generated dictionary has no conflict");
    let mut group = criterion.benchmark_group("fix/mutate");

    // A spelling the word pairs reach, and one they do not: the lookup a
    // desk's own name costs where the dictionary names it otherwise.
    group.bench_function("word_alias_lookup", |bencher| {
        bencher.iter(|| {
            black_box((
                registry.get_field_by_name(black_box("AskPrice")),
                registry.get_field_by_name(black_box("HedgePriceQty")),
            ))
        });
    });

    // One insert into a dictionary of each size. The clone is outside the
    // timer, and so is the drop: every routine hands the registry back as its
    // output rather than letting it fall at the end of the timed closure.
    let mut incoming = DataType::utf8().nullable_field("Incoming");
    FixFieldMut::new(&mut incoming).set_tag(9_000).unwrap();
    FixFieldMut::new(&mut incoming)
        .set_names(["IncomingAlias"])
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

    // Building a whole dictionary, which is what a load costs above I/O:
    // the scalar fields, because a registry's iteration lists definitions
    // after the scalars in name order, and a message inserted before the
    // component it references is refused. The vocabularies stay behind: a
    // dictionary built from bare fields states no code set, and a field
    // naming one nothing states is refused.
    let fields: Vec<_> = large
        .iter()
        .filter(|field| !field.dtype().is_nested())
        .map(|field| {
            let mut field = field.clone();
            FixFieldMut::new(&mut field).remove_codeset();
            field
        })
        .collect();
    group.bench_function(format!("from_fields_{LARGE_FIELDS}"), |bencher| {
        bencher.iter_batched(
            || fields.clone(),
            |fields| black_box(FixRegistry::from_fields(fields).unwrap()),
            BatchSize::SmallInput,
        );
    });

    // A merge that adds an alias and an alternate tag to a stored field.
    let mut update = DataType::utf8().nullable_field("Symbol");
    FixFieldMut::new(&mut update).set_tag(55).unwrap();
    FixFieldMut::new(&mut update).set_tags(&[9_001]).unwrap();
    FixFieldMut::new(&mut update).set_names(["Sym"]).unwrap();
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
    FixFieldMut::new(&mut renamed).set_tag(9_001).unwrap();
    FixFieldMut::new(&mut renamed)
        .set_names(["Ticker"])
        .unwrap();
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
    let nested = StructType::from_fields([DataType::utf8().nullable_field("VenueSymbol")])
        .map(DataType::from)
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
    FixFieldMut::new(&mut venue_symbol).set_tag(9_010).unwrap();
    seeded.add_field(venue_symbol).unwrap();
    let mut member = seeded.field(9_010).unwrap().clone();
    FixFieldMut::new(&mut member)
        .set_field_ref("VenueSymbol")
        .unwrap();
    let mut instrument = seeded.field_by_name("Instrument").unwrap().clone();
    instrument
        .set_dtype(DataType::from(
            StructType::from_fields(instrument.fields().iter().cloned().chain([member])).unwrap(),
        ))
        .unwrap();
    group.bench_function("add_field_extends_component", |bencher| {
        bencher.iter_batched(
            || (seeded.clone(), instrument.clone()),
            |(mut registry, field)| {
                black_box(registry.add_field(field).unwrap());
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
    FixFieldMut::new(&mut movable).set_tag(9_000).unwrap();
    group.bench_function("set_tag", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                FixFieldMut::new(&mut field).set_tag(9_000).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_sources_one", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                FixFieldMut::new(&mut field).set_sources([venue]).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_sources_folded", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                FixFieldMut::new(&mut field)
                    .set_sources(["plugin", "CME", venue, "eurex"])
                    .unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut member = movable.clone();
    FixFieldMut::new(&mut member).set_sources([venue]).unwrap();
    group.bench_function("add_source", |bencher| {
        bencher.iter_batched(
            || member.clone(),
            |mut field| {
                FixFieldMut::new(&mut field).add_source("eurex").unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut reserved = DataType::utf8().nullable_field("Reserved");
    FixFieldMut::new(&mut reserved).set_tag(35).unwrap();
    group.bench_function("set_tag_standard", |bencher| {
        bencher.iter_batched(
            || reserved.clone(),
            |mut field| {
                FixFieldMut::new(&mut field)
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
    let stored = merge_source("the stored wording", "lastshares", LASTQTY_CODESET);
    let incoming = merge_source("the incoming wording", "qty", VENUE_LASTQTY_CODESET);
    group.bench_function("merge_with", |bencher| {
        bencher.iter_batched(
            || incoming.clone(),
            |mut field| {
                FixFieldMut::new(&mut field)
                    .merge_with(&FixField::new(&stored))
                    .expect("two definitions of one tag");
                field
            },
            BatchSize::SmallInput,
        );
    });
    // The same fold through the registry, which adds the generic metadata
    // half, the vocabularies the two sides name differently, and the
    // reindexing a stored field needs.
    let mut merging = FixRegistry::new();
    merging
        .set_codeset(
            LASTQTY_CODESET,
            &[
                FixCode::new("Shared", "1").with_description("the stored reading"),
                FixCode::new("Other", "2"),
            ],
        )
        .expect("a valid code set");
    merging
        .set_codeset(
            VENUE_LASTQTY_CODESET,
            &[
                FixCode::new("Shared", "1").with_description("the incoming reading"),
                FixCode::new("Other", "2"),
            ],
        )
        .expect("a valid code set");
    merging.insert(stored.clone()).expect("one field");
    group.bench_function("update_merging", |bencher| {
        bencher.iter(|| {
            black_box(&mut merging)
                .update(black_box(incoming.clone()))
                .expect("the same identity")
        });
    });

    let target = coded_catalog();
    let mut source = target.clone();
    // The other dictionary's statement of the one set its field reads by:
    // a member the target does not hold, which is the fold a merge pays.
    source
        .set_codeset(PARTY_CODESET, &[FixCode::new("Client", "C")])
        .unwrap();
    group.bench_function("merge_catalog_codesets", |bencher| {
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

/// The set `PartyID` reads by, named as a store files it: the folded field
/// name and `codeset`.
const PARTY_CODESET: &str = "partyidcodeset";

fn coded_catalog() -> FixRegistry {
    let mut party = DataType::utf8().nullable_field("PartyID");
    FixFieldMut::new(&mut party).set_tag(448).unwrap();
    FixFieldMut::new(&mut party)
        .set_codeset(PARTY_CODESET)
        .unwrap();
    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    FixFieldMut::new(&mut counter).set_tag(453).unwrap();
    // The vocabulary first: a registry refuses a field naming a set it does
    // not hold.
    let mut registry = FixRegistry::new();
    registry
        .set_codeset(PARTY_CODESET, &[FixCode::new("Broker", "B")])
        .unwrap();
    for field in [party.clone(), counter] {
        registry.insert(field).unwrap();
    }
    FixFieldMut::new(&mut party)
        .set_field_ref("PartyID")
        .unwrap();
    let component = StructType::from_fields([party])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
    registry.insert(component.clone()).unwrap();
    let mut group = DataType::serie(component).nullable_field("Parties");
    FixFieldMut::new(&mut group).set_counter(453).unwrap();
    FixFieldMut::new(&mut group).set_component("Party").unwrap();
    registry.insert(group).unwrap();
    let mut group = registry.field_by_name("Parties").unwrap().clone();
    FixFieldMut::new(&mut group).set_group("Parties").unwrap();
    // A group is its list alone: the counter is the dictionary's field and
    // no member of the message beside it.
    let mut message = StructType::from_fields([group])
        .map(DataType::from)
        .unwrap()
        .required_field("Order");
    FixFieldMut::new(&mut message).set_msgtype("D").unwrap();
    registry.insert(message).unwrap();
    registry
}

/// The two names the sources of tag 32 file its vocabulary under: the
/// specification's own, and a venue's naming after the field.
const LASTQTY_CODESET: &str = "lastqtycodeset";
const VENUE_LASTQTY_CODESET: &str = "venuelastqtycodeset";

/// One realistic definition of tag 32: coded, described, aliased.
fn merge_source(wording: &str, alias: &str, codeset: &str) -> Field {
    let mut field = DataType::utf8().nullable_field("LastQty");
    FixFieldMut::new(&mut field)
        .set_tag(32)
        .expect("a static tag");
    FixFieldMut::new(&mut field)
        .set_tags(&[65, 66])
        .expect("static alternate tags");
    FixFieldMut::new(&mut field)
        .set_names([alias])
        .expect("a spelling the field does not already take");
    FixFieldMut::new(&mut field)
        .set_description(wording)
        .expect("a description");
    FixFieldMut::new(&mut field)
        .set_codeset(codeset)
        .expect("the set its dictionary holds");
    field
}
