use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixRegistry, Url};

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
    group.finish();

    for path in built {
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = std::fs::remove_dir_all(target);
    let _ = std::fs::remove_dir_all(mixed_root);
    let _ = std::fs::remove_dir_all(mixed_target);
}
