use std::hint::black_box;

use criterion::{BatchSize, Criterion};
use yggdryl::{DataType, FixId, FixRegistry};

use super::{generated, seed, venue};

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = seed();
    let large = FixRegistry::from_fields(registry.iter().cloned().chain(generated(4_000)))
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
    group.bench_function("insert_into_4000", |bencher| {
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
    group.bench_function("from_fields_4000", |bencher| {
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
    group.bench_function("update_in_4000", |bencher| {
        bencher.iter_batched(
            || (large.clone(), update.clone()),
            |(mut registry, field)| {
                registry.update(field).unwrap();
                registry
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("remove_from_4000", |bencher| {
        bencher.iter_batched(
            || large.clone(),
            |mut registry| {
                black_box(registry.remove(7_000).unwrap());
                registry
            },
            BatchSize::SmallInput,
        );
    });

    // The identity setters, and the refusal path a caller pays for a tag the
    // FIX specification assigns.
    let venue = venue();
    let vendor = FixId::from_parts(venue.clone(), 9_000).expect("a vendor identifier");
    let mut movable = DataType::Utf8.nullable_field("Movable");
    movable.as_fix_mut().set_tag(9_000).unwrap();
    group.bench_function("set_namespace", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_namespace(&venue).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("set_id", |bencher| {
        bencher.iter_batched(
            || movable.clone(),
            |mut field| {
                field.as_fix_mut().set_id(&vendor).unwrap();
                field
            },
            BatchSize::SmallInput,
        );
    });
    let mut reserved = DataType::Utf8.nullable_field("Reserved");
    reserved.as_fix_mut().set_tag(35).unwrap();
    group.bench_function("set_namespace_refused", |bencher| {
        bencher.iter_batched(
            || reserved.clone(),
            |mut field| {
                black_box(field.as_fix_mut().set_namespace(&venue).unwrap_err());
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
                    .set_id(&FixId::standard(35))
                    .expect("the standard namespace holds any tag");
                field
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}
