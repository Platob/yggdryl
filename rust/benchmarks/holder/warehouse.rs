//! What a warehouse costs: resolving a registered path, and listing a folder
//! catalog of N leaves - time to the first table beside the full drain, as
//! the listing benchmarks measure a folder.
//!
//! Resolution of a registered path touches no store, so its row is the floor
//! of the abstraction: the walk down the memory levels and the clone of the
//! object answered. A folder catalog's listing is one store listing plus
//! one classification per entry, lazily, so the first table costs one entry
//! and the drain costs them all.

use std::cell::OnceCell;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::local::LocalFolder;
use yggdryl::{Catalog, FolderCatalog, IOBase, MediaTable, NamespaceValue, Table, Url, Warehouse};

/// The folder widths the listing is measured at.
const WIDTHS: [usize; 3] = [
    10,
    crate::bench_profile::corpus(1_000, 100),
    crate::bench_profile::corpus(100_000, 1_000),
];

/// A folder of `width` CSV leaves, as a catalog over it, with its root path.
fn wide(width: usize) -> (std::path::PathBuf, Catalog) {
    let root = LocalFolder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-bench-warehouse-{width}"));
    let mut folder = LocalFolder::new(&root).expect("a valid path");
    folder.remove(true).ok();
    folder.create().expect("a creatable folder");
    for leaf in 0..width {
        let mut child = folder
            .child_by_path(&format!("part-{leaf:06}.csv"))
            .expect("a child");
        child
            .write_all_bytes(b"symbol,price\nAAPL,1\n")
            .expect("a written leaf");
    }
    let catalog = Catalog::from(FolderCatalog::bound(
        "market",
        yggdryl::holder::Holder::LocalFolder(folder),
    ));
    (root, catalog)
}

/// A warehouse of one memory catalog holding `width` registered tables under
/// one namespace.
fn registered(width: usize) -> Warehouse {
    let mut warehouse = Warehouse::new();
    for table in 0..width {
        warehouse
            .register(Table::from(
                MediaTable::new(
                    ["lake", "eu", &format!("t{table}")],
                    Url::from_str(&format!("file:///lake/eu/t{table}.csv")).expect("a URL"),
                )
                .expect("a table"),
            ))
            .expect("registered");
    }
    warehouse
}

pub(crate) fn warehouse_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("warehouse");

    // Resolving a registered path: a walk of the memory levels and nothing
    // asked of any store, whatever the level holds.
    for width in [1, crate::bench_profile::corpus(1_000, 100)] {
        let warehouse = registered(width);
        let last = format!("lake.eu.t{}", width - 1);
        group.bench_with_input(
            BenchmarkId::new("resolve/registered", width),
            &width,
            |bencher, _| {
                bencher.iter(|| {
                    black_box(&warehouse)
                        .table(black_box(last.as_str()))
                        .expect("the table")
                });
            },
        );
    }

    for width in WIDTHS {
        // Criterion still calls this function when another group is
        // selected, so the wide fixture is built behind the timed case.
        let fixture = OnceCell::new();
        group.throughput(Throughput::Elements(width as u64));

        group.bench_with_input(
            BenchmarkId::new("first_table/folder", width),
            &width,
            |bencher, _| {
                let (_, catalog) = fixture.get_or_init(|| wide(width));
                bencher.iter(|| black_box(catalog).children().next().is_some());
            },
        );

        group.bench_with_input(
            BenchmarkId::new("drain/folder", width),
            &width,
            |bencher, _| {
                let (_, catalog) = fixture.get_or_init(|| wide(width));
                bencher.iter(|| black_box(catalog).children().count());
            },
        );

        if let Some((root, catalog)) = fixture.into_inner() {
            drop(catalog);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    group.finish();
}
