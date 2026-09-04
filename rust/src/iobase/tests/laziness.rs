use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::{IOBase, Listing};
use crate::holder::Holder;
use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

/// A container of `width` synthetic leaves that counts what it produces.
///
/// `opened` counts directory reads, `produced` counts entries actually
/// materialized. A lazy listing moves the second number by exactly what the
/// caller drained; an eager one moves it by `width` whatever the caller did.
#[derive(Debug, Clone)]
struct Wide {
    url: Url,
    width: usize,
    /// Directories this handle was asked to read.
    opened: Arc<AtomicUsize>,
    /// Entries this handle actually materialized.
    produced: Arc<AtomicUsize>,
    /// The zero-based entry index the listing fails at, if any.
    fails_at: Option<usize>,
}

impl Wide {
    fn new(width: usize) -> Self {
        Self {
            url: Url::from_str("memory://wide").expect("a valid location"),
            width,
            opened: Arc::new(AtomicUsize::new(0)),
            produced: Arc::new(AtomicUsize::new(0)),
            fails_at: None,
        }
    }

    fn failing_at(mut self, index: usize) -> Self {
        self.fails_at = Some(index);
        self
    }

    fn opened(&self) -> usize {
        self.opened.load(Ordering::Relaxed)
    }

    fn produced(&self) -> usize {
        self.produced.load(Ordering::Relaxed)
    }
}

impl crate::IOMedia for Wide {
    crate::impl_default_iomedia!();
}

impl IOBase for Wide {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        static DIRECTORY: std::sync::OnceLock<MediaType> = std::sync::OnceLock::new();
        DIRECTORY.get_or_init(|| MediaType::from(MimeType::DIRECTORY))
    }

    fn set_media_type(&mut self, _media_type: MediaType) {}

    fn kind(&self) -> IOKind {
        IOKind::Directory
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> Listing {
        let opened = Arc::clone(&self.opened);
        let produced = Arc::clone(&self.produced);
        let fails_at = self.fails_at;
        let root = self.url.clone();
        let width = self.width;
        // Deferred exactly as a real backend's is: the directory read
        // happens on the first `next`, not when the listing is built.
        Listing::new(std::iter::once(()).flat_map(move |()| {
            opened.fetch_add(1, Ordering::Relaxed);
            let produced = Arc::clone(&produced);
            let root = root.clone();
            Listing::new((0..width).map(move |index| {
                produced.fetch_add(1, Ordering::Relaxed);
                if fails_at == Some(index) {
                    return Err(Error::absent("file", format!("{root}/part-{index}")));
                }
                Ok(Holder::from(crate::holder::Buffer::new()))
            }))
        }))
    }
}

#[test]
fn building_a_listing_touches_nothing() {
    let wide = Wide::new(10_000);
    let listing = wide.ls(false, false);
    assert_eq!(wide.opened(), 0, "no directory read yet");
    assert_eq!(wide.produced(), 0, "no entry materialized yet");
    drop(listing);
    assert_eq!(wide.opened(), 0, "a listing nobody drained costs nothing");
}

#[test]
fn taking_three_entries_costs_three_entries() {
    let wide = Wide::new(10_000);
    let taken: Vec<_> = wide
        .ls(false, false)
        .take(3)
        .collect::<Result<Vec<_>>>()
        .expect("three entries");

    assert_eq!(taken.len(), 3);
    assert_eq!(wide.opened(), 1, "one directory read");
    assert_eq!(
        wide.produced(),
        3,
        "exactly the three entries the caller asked for, not ten thousand"
    );
}

#[test]
fn a_failing_entry_ends_the_listing_without_discarding_what_came_before() {
    let wide = Wide::new(10).failing_at(2);
    let entries: Vec<_> = wide.ls(false, false).collect();

    assert_eq!(entries.len(), 3, "two entries, then the failure");
    assert!(entries[0].is_ok());
    assert!(entries[1].is_ok());
    assert!(entries[2].as_ref().is_err_and(Error::is_absent));
    assert_eq!(
        wide.produced(),
        3,
        "the listing stopped at the failing entry rather than draining past it"
    );
}

#[test]
fn the_same_listing_over_the_same_state_yields_the_same_order_twice() {
    let root = crate::holder::local::Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-order-{}", std::process::id()));
    let mut folder = crate::holder::local::Folder::new(&root).expect("a local folder");
    folder.remove(true).ok();
    for name in ["c.bin", "a.bin", "b.bin"] {
        let mut leaf = folder.child_by_path(name).expect("a child");
        leaf.write_all_bytes(b"x").expect("a write");
    }

    let names = || -> Vec<String> {
        folder
            .ls(true, false)
            .map(|entry| {
                Ok(entry?
                    .url()
                    .and_then(|url| url.file_name())
                    .unwrap_or_default()
                    .to_owned())
            })
            .collect::<Result<Vec<_>>>()
            .expect("a listing")
    };
    assert_eq!(names(), names());
    assert_eq!(names(), ["a.bin", "b.bin", "c.bin"]);

    folder.remove(true).expect("a removable folder");
}

#[test]
fn a_glob_whose_fixed_prefix_loses_lists_nothing_beneath_it() {
    let root = crate::holder::local::Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-prefix-{}", std::process::id()));
    let mut folder = crate::holder::local::Folder::new(&root).expect("a local folder");
    folder.remove(true).ok();
    let mut leaf = folder
        .child_by_path("year=2024/month=01/part-0.parquet")
        .expect("a child");
    leaf.write_all_bytes(b"PAR1").expect("a write");

    // The pattern is built but never drained, so nothing is read at all.
    let listing = folder
        .glob("year=1999/**/*.parquet", false)
        .expect("an expandable pattern");
    drop(listing);

    // Drained, it descends the fixed prefix, finds nothing there, and never
    // looks at `year=2024`.
    assert_eq!(
        folder
            .glob("year=1999/**/*.parquet", false)
            .expect("an expandable pattern")
            .count(),
        0
    );
    assert_eq!(
        folder
            .glob("year=2024/**/*.parquet", false)
            .expect("an expandable pattern")
            .count(),
        1
    );

    folder.remove(true).expect("a removable folder");
}

#[test]
fn a_recursive_walk_descends_one_level_at_a_time() {
    // A deep, narrow tree: sixteen levels, each holding one directory and
    // one leaf. The walk yields an entry before the subtree under it, and
    // what it retains is one level's cursor per *open* level - the
    // frontier - never the thirty-two entries it will eventually yield.
    let root = crate::holder::local::Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-deep-{}", std::process::id()));
    let mut folder = crate::holder::local::Folder::new(&root).expect("a local folder");
    folder.remove(true).ok();
    let mut path = String::new();
    for level in 0..16 {
        path.push_str(&format!("level-{level:02}/"));
        let mut leaf = folder
            .child_by_path(&format!("{path}leaf.bin"))
            .expect("a child");
        leaf.write_all_bytes(b"x").expect("a write");
    }

    let mut seen = 0_usize;
    for entry in folder.ls(true, false) {
        entry.expect("an entry");
        seen += 1;
    }
    assert_eq!(seen, 32, "sixteen directories and sixteen leaves");

    folder.remove(true).expect("a removable folder");
}
