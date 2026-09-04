//! Time to first entry beside the full drain, over synthetic wide folders.
//!
//! A listing benchmark that only measures the drain hides the exact property
//! the listing contract exists for: a caller that wants three entries out of a
//! hundred thousand should pay for three. The two legs are reported together so
//! the shape of that answer is visible rather than asserted.

use std::cell::OnceCell;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::io::IOBase;
use yggdryl::local::Folder;

/// The folder widths the two legs are measured at.
const WIDTHS: [usize; 3] = [10, 1_000, 100_000];

/// Build a folder of `width` leaves, and return it with its root path.
fn wide(width: usize) -> (std::path::PathBuf, Folder) {
    let root = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-bench-listing-{width}"));
    let mut folder = Folder::new(&root).expect("a valid path");
    folder.remove(true).ok();
    folder.create().expect("a creatable folder");
    for leaf in 0..width {
        let mut child = folder
            .child_by_path(&format!("part-{leaf:06}.parquet"))
            .expect("a child");
        child.write_all_bytes(b"PAR1").expect("a written leaf");
    }
    (root, folder)
}

/// Build a folder of `width` leaves split across ten subdirectories.
fn deep(width: usize) -> (std::path::PathBuf, Folder) {
    let root = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-bench-listing-deep-{width}"));
    let mut folder = Folder::new(&root).expect("a valid path");
    folder.remove(true).ok();
    folder.create().expect("a creatable folder");
    for leaf in 0..width {
        let mut child = folder
            .child_by_path(&format!("part={:02}/part-{leaf:06}.parquet", leaf % 10))
            .expect("a child");
        child.write_all_bytes(b"PAR1").expect("a written leaf");
    }
    (root, folder)
}

pub(crate) fn listing_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("io_listing");

    for width in WIDTHS {
        // Criterion still calls this benchmark-registration function when a
        // different group is selected by a command-line filter. Keep the
        // 100,000-file fixture behind the timed case so a focused run does not
        // eagerly build an unrelated directory tree.
        let fixture = OnceCell::new();
        group.throughput(Throughput::Elements(width as u64));

        // The property the contract exists for: one entry costs one entry.
        group.bench_with_input(
            BenchmarkId::new("first_entry/flat", width),
            &width,
            |bencher, _| {
                let (_, folder) = fixture.get_or_init(|| wide(width));
                bencher.iter(|| black_box(folder).ls(false, false).next().is_some());
            },
        );

        group.bench_with_input(
            BenchmarkId::new("drain/flat", width),
            &width,
            |bencher, _| {
                let (_, folder) = fixture.get_or_init(|| wide(width));
                bencher.iter(|| black_box(folder).ls(false, false).count());
            },
        );

        if let Some((root, folder)) = fixture.into_inner() {
            drop(folder);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    for width in WIDTHS {
        let fixture = OnceCell::new();
        group.throughput(Throughput::Elements(width as u64));

        group.bench_with_input(
            BenchmarkId::new("first_entry/recursive", width),
            &width,
            |bencher, _| {
                let (_, folder) = fixture.get_or_init(|| deep(width));
                bencher.iter(|| black_box(folder).ls(true, false).next().is_some());
            },
        );

        group.bench_with_input(
            BenchmarkId::new("drain/recursive", width),
            &width,
            |bencher, _| {
                let (_, folder) = fixture.get_or_init(|| deep(width));
                bencher.iter(|| black_box(folder).ls(true, false).count());
            },
        );

        if let Some((root, folder)) = fixture.into_inner() {
            drop(folder);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    group.finish();
}
