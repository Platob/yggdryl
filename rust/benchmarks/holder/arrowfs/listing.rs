//! Listing and glob expansion over a populated foreign tree.
//!
//! The walk itself is inherited from [`IOBase`], so what varies is how many
//! vtable calls the roles make to answer it: a flat listing is one `list`, a
//! recursive one is still one `list`, and a glob descends its fixed prefix
//! before listing rather than listing the whole tree and filtering.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::IOBase;
use yggdryl::holder::arrowfs::Folder;

use super::{local, local_location, memory, tree};

pub(crate) fn listing_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arrowfs_listing");

    let filesystem = memory();
    tree(filesystem.as_ref(), "lake");
    let lake = Folder::from_location(filesystem, "lake").expect("a valid location");

    group.bench_function("ls_flat/arrowfs_memory", |bencher| {
        bencher.iter(|| black_box(&lake).ls(false, false).count());
    });

    group.bench_function("ls_recursive/arrowfs_memory", |bencher| {
        bencher.iter(|| black_box(&lake).ls(true, false).count());
    });

    group.bench_function("glob_all/arrowfs_memory", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .glob("**/*.parquet", false)
                .expect("an expandable pattern")
                .count()
        });
    });

    // A fixed prefix is descended rather than listed and filtered, so this
    // leg should stay well under the whole-tree one.
    group.bench_function("glob_prefixed/arrowfs_memory", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .glob("year=2024/**/*.parquet", false)
                .expect("an expandable pattern")
                .count()
        });
    });

    group.bench_function("children_where/arrowfs_memory", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .children_where(&[("year", "2024")], false)
                .expect("a filterable container")
                .count()
        });
    });

    let (local_filesystem, root) = local();
    let location = local_location(&root, "lake");
    tree(local_filesystem.as_ref(), &location);
    let local_lake = Folder::from_location(local_filesystem, &location).expect("a valid location");

    group.bench_function("ls_recursive/arrowfs_local", |bencher| {
        bencher.iter(|| black_box(&local_lake).ls(true, false).count());
    });

    group.bench_function("ls_recursive/local_folder", |bencher| {
        let folder = yggdryl::holder::local::Folder::new(root.join("lake")).expect("a valid path");
        bencher.iter(|| black_box(&folder).ls(true, false).count());
    });

    group.finish();
    let _ = std::fs::remove_dir_all(&root);
}
