use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::{
    DataType, Field, FixBranch, FixCode, FixLineageEntry, FixPedigree, FixRegistry, Version,
};

use super::{LARGE_FIELDS, generated, seed, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let large = FixRegistry::from_fields(registry.iter().cloned().chain(generated(LARGE_FIELDS)))
        .expect("the generated dictionary has no conflict");
    let mut group = criterion.benchmark_group("fix/mutate");

    // One insert into a dictionary of each size. The clone is outside the
    // timer, and so is the drop: every routine hands the registry back as its
    // output rather than letting it fall at the end of the timed closure.
    let mut incoming = DataType::Utf8.nullable_field("Incoming");
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
    let mut update = DataType::Utf8.nullable_field("Symbol");
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

    // The identity setters, and the refusal path a caller pays for a tag the
    // FIX specification assigns.
    let venue = venue();
    let mut movable = DataType::Utf8.nullable_field("Movable");
    movable.as_fix_mut().set_tag(9_000).unwrap();
    group.bench_function("set_branch", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_branch(&venue).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_id", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_id(&venue, 9_000).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut reserved = DataType::Utf8.nullable_field("Reserved");
    reserved.as_fix_mut().set_tag(35).unwrap();
    group.bench_function("set_branch_refused", |bencher| {
        bencher.iter_batched(
            || reserved.clone(),
            |mut field| {
                black_box(field.as_fix_mut().set_branch(&venue).unwrap_err());
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_id_standard", |bencher| {
        bencher.iter_batched(
            || reserved.clone(),
            |mut field| {
                field
                    .as_fix_mut()
                    .set_id(&FixBranch::STANDARD, 35)
                    .expect("the standard branch holds any tag");
                field
            },
            BatchSize::SmallInput,
        );
    });
    // The one FIX-aware merge, over two realistic definitions of one tag: a
    // generator folds several sources into every field it writes, so this is
    // what a regeneration costs per tag.
    let stored = merge_source("the stored wording", "2.7", "the stored reading");
    let incoming = merge_source("the incoming wording", "5.0SP2", "the incoming reading");
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

    group.finish();
}

/// One realistic definition of tag 32: dated, coded, described, aliased.
fn merge_source(wording: &str, dated: &str, reading: &str) -> Field {
    let version: Version = dated.parse().expect("a valid version");
    let mut field = DataType::Utf8.nullable_field("LastQty");
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
