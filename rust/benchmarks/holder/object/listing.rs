//! Listing a prefix, against `object_store` on the same store.
//!
//! Time to first entry is reported beside the full drain, because the listing
//! contract exists for the first of those: a caller who wants three entries
//! out of two thousand should pay for one page, not for the walk.

use std::hint::black_box;

use criterion::Criterion;
use futures::StreamExt as _;
use object_store::ObjectStore as _;
use yggdryl::IOBase;

use super::{LEAVES, baseline, baseline_path, location, options, runtime, store, tree};

pub(crate) fn listing_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("object_listing");

    let store = store();
    tree(&store, "lake", LEAVES);
    let lake = yggdryl::holder::object::folder_with(&location("lake/"), options(&store))
        .expect("a prefix handle");
    let runtime = runtime();
    let external = baseline(&store);
    let prefix = baseline_path("lake");

    // The property the contract exists for: three entries cost one page.
    group.bench_function("first_entries/yggdryl", |bencher| {
        bencher.iter(|| black_box(&lake).ls(true, false).take(3).count());
    });
    group.bench_function("first_entries/object_store", |bencher| {
        bencher.iter(|| {
            runtime.block_on(async {
                let mut listing = external.list(Some(black_box(&prefix)));
                let mut seen = 0;
                while seen < 3 {
                    if listing.next().await.is_none() {
                        break;
                    }
                    seen += 1;
                }
                black_box(seen)
            })
        });
    });

    // The whole subtree. Yggdryl yields the containers each key implies as
    // well, which `object_store` does not report at all, so its count is the
    // leaves alone - the tables say so rather than pretending they match.
    group.bench_function("drain_recursive/yggdryl", |bencher| {
        bencher.iter(|| black_box(&lake).ls(true, false).count());
    });
    group.bench_function("drain_recursive/object_store", |bencher| {
        bencher.iter(|| {
            runtime.block_on(async {
                let mut listing = external.list(Some(black_box(&prefix)));
                let mut seen = 0_usize;
                while let Some(entry) = listing.next().await {
                    entry.expect("an entry");
                    seen += 1;
                }
                black_box(seen)
            })
        });
    });

    // One level, which is a delimited listing on both sides.
    group.bench_function("drain_level/yggdryl", |bencher| {
        bencher.iter(|| black_box(&lake).ls(false, false).count());
    });
    group.bench_function("drain_level/object_store", |bencher| {
        bencher.iter(|| {
            runtime.block_on(async {
                let result = external
                    .list_with_delimiter(Some(black_box(&prefix)))
                    .await
                    .expect("a level");
                black_box(result.objects.len() + result.common_prefixes.len())
            })
        });
    });

    group.finish();
}
