use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixCategory, FixRegistry, Url};

use super::{BRANCH_FIELDS, scratch, seed, seed_root, two_branches};

/// A folder holding `shards` shards of ten fields each, built outside the
/// timer.
fn sharded(shards: i32) -> (std::path::PathBuf, Folder) {
    let fields = (0..shards).flat_map(|shard| {
        (0..10).map(move |offset| {
            let tag = shard * 100 + offset;
            let mut field = DataType::Int64.nullable_field(format!("Field{tag}"));
            field.as_fix_mut().set_tag(tag).unwrap();
            field
        })
    });
    let registry = FixRegistry::from_fields(fields).expect("distinct generated tags");
    let path = scratch(&format!("shards-{shards}"));
    let mut folder = Folder::new(&path).expect("a local folder");
    registry
        .write_into(&mut folder)
        .expect("the shards written");
    (path, folder)
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fix/store");

    // Open and full load against shard count.
    let mut built = Vec::new();
    let shard_counts = [
        1_i32,
        i32::try_from(crate::bench_profile::corpus(10, 3)).unwrap(),
        i32::try_from(crate::bench_profile::corpus(100, 10)).unwrap(),
    ];
    for shards in shard_counts {
        let (path, folder) = sharded(shards);
        group.throughput(Throughput::Elements(u64::try_from(shards * 10).unwrap()));
        group.bench_function(format!("from_handle_{shards}_shards"), |bencher| {
            bencher.iter(|| black_box(FixRegistry::from_handle(black_box(&folder)).unwrap()));
        });
        built.push(path);
    }

    // The tracked seed, and the whole write of a hundred shards.
    let seed_folder = Folder::new(seed_root()).expect("the seed folder");
    group.throughput(Throughput::Elements(u64::try_from(seed().len()).unwrap()));
    group.bench_function("from_handle_seed", |bencher| {
        bencher.iter(|| black_box(FixRegistry::from_handle(black_box(&seed_folder)).unwrap()));
    });
    let hundred = FixRegistry::from_handle(&Folder::new(&built[2]).unwrap()).unwrap();
    let target = scratch("write");
    let mut target_folder = Folder::new(&target).expect("a local folder");
    group.throughput(Throughput::Elements(u64::try_from(hundred.len()).unwrap()));
    group.bench_function(
        format!("write_into_{}_shards", shard_counts[2]),
        |bencher| {
            bencher.iter(|| black_box(&hundred).write_into(&mut target_folder).unwrap());
        },
    );

    // The branched layout: a registry holding two dictionaries opens, loads
    // and writes them as separate folders of shards.
    let mixed = two_branches(BRANCH_FIELDS);
    let mixed_root = scratch("two-branches");
    let mut mixed_folder = Folder::new(&mixed_root).expect("a local folder");
    mixed
        .write_into(&mut mixed_folder)
        .expect("the shards written");
    group.throughput(Throughput::Elements(u64::try_from(mixed.len()).unwrap()));
    group.bench_function("from_handle_two_branches", |bencher| {
        bencher.iter(|| black_box(FixRegistry::from_handle(black_box(&mixed_folder)).unwrap()));
    });
    let mixed_target = scratch("two-branches-write");
    let mut mixed_target_folder = Folder::new(&mixed_target).expect("a local folder");
    group.bench_function("write_into_two_branches", |bencher| {
        bencher.iter(|| {
            black_box(&mixed)
                .write_into(&mut mixed_target_folder)
                .unwrap();
        });
    });

    // The first-call cost of the default resolved from an explicit location:
    // the URL parse, the folder handle, and the load it redirects to.
    let location = seed_root().to_string_lossy().into_owned();
    group.throughput(Throughput::Elements(u64::try_from(seed().len()).unwrap()));
    group.bench_function("autoload_location_seed", |bencher| {
        bencher.iter(|| {
            let location = black_box(location.as_str());
            let url = Url::from_str(location)
                .or_else(|_| Url::from_path(location))
                .unwrap();
            let folder = Folder::from_url(url).unwrap();
            assert!(folder.exists());
            black_box(FixRegistry::from_handle(&folder).unwrap())
        });
    });
    let catalog = seed();
    let snapshot = catalog.into_json().expect("the complete catalog snapshot");
    group.throughput(Throughput::Bytes(snapshot.len() as u64));
    group.bench_function("from_json_seed", |bencher| {
        bencher.iter(|| black_box(FixRegistry::from_json(black_box(&snapshot)).unwrap()));
    });
    group.bench_function("into_json_seed", |bencher| {
        bencher.iter(|| black_box(catalog.into_json().unwrap()));
    });
    group.bench_function("stable_hash_seed_one_state_allocation", |bencher| {
        bencher.iter(|| black_box(catalog.stable_hash()));
    });
    group.throughput(Throughput::Elements(1));
    group.bench_function("definition_group", |bencher| {
        bencher.iter(|| {
            black_box(
                catalog
                    .definition(FixCategory::Groups, black_box("Parties"), None)
                    .unwrap(),
            )
        });
    });
    group.bench_function("definitions_components_first", |bencher| {
        bencher.iter(|| black_box(catalog.definitions(FixCategory::Components).next()));
    });
    group.bench_function("field_code_resolve", |bencher| {
        let field = catalog.field(54).unwrap();
        bencher.iter(|| black_box(field.as_fix().code_name(black_box("1"))));
    });
    group.bench_function("msgtype_code", |bencher| {
        bencher.iter(|| black_box(catalog.get_msgtype(black_box("D"), None)));
    });
    group.bench_function("msgtype_stable_hash_one_state_allocation", |bencher| {
        let message = catalog.msgtype("D", None).unwrap();
        bencher.iter(|| black_box(message.stable_hash()));
    });
    group.bench_function("msgtype_scoped_group", |bencher| {
        let message = catalog.msgtype("D", None).unwrap();
        bencher.iter(|| {
            black_box(message.get_group_by_counter(black_box(yggdryl::FixId::standard(453))))
        });
    });
    let codec = yggdryl::FixCodec::new(std::sync::Arc::new(catalog.clone()));
    group.bench_function("numeric_group_cached_plan", |bencher| {
        bencher.iter(|| {
            black_box(
                codec
                    .transform_fix_line(black_box(b"35=D|453=1|448=broker|447=D|452=1|"), false)
                    .unwrap(),
            )
        });
    });
    group.finish();

    for path in built {
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(mixed_root);
    let _ = std::fs::remove_dir_all(mixed_target);
}
