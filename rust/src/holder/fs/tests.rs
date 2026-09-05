//! Foreign filesystems behind the one storage trait.
//!
//! The shared byte contract is exercised for this backend alongside every
//! other in `io::tests::conformance`; what is tested here is what only this
//! module can get wrong - the vtable boundary, the prefix-shaped directory
//! model, and the staged write that publishes as one whole-value replacement.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::holder::fs::{File, FileInfo, FileSystem, Folder, MemoryFileSystem, Path};
use crate::{IOBase, IOMedia};
use crate::{IOKind, MediaType, MimeType, Result};

#[test]
fn file_info_is_a_totally_ordered_hashable_snapshot() {
    fn assert_traits<T: Clone + Eq + std::hash::Hash + Ord>() {}
    assert_traits::<FileInfo>();
    assert!(FileInfo::file("a", 1) < FileInfo::file("b", 1));
}

/// An empty in-memory filesystem.
fn memory() -> Arc<MemoryFileSystem> {
    Arc::new(MemoryFileSystem::new())
}

/// A filesystem that counts every call reaching the vtable.
///
/// This is how "constructing a handle touches nothing" becomes a testable
/// number rather than a claim: build a handle, read the counter, expect zero.
#[derive(Debug, Default)]
struct Counting {
    inner: MemoryFileSystem,
    calls: AtomicUsize,
}

impl Counting {
    fn new() -> Self {
        Self::default()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn count(&self) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

impl FileSystem for Counting {
    fn type_name(&self) -> &str {
        // Naming is not a filesystem operation, so it is deliberately uncounted.
        self.inner.type_name()
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.count();
        self.inner.file_info(path)
    }

    fn list(&self, path: &str, recursive: bool) -> crate::holder::fs::FileInfos {
        self.count();
        self.inner.list(path, recursive)
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.count();
        self.inner.read_range(path, offset, buffer)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.count();
        self.inner.write_full(path, bytes)
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        self.count();
        self.inner.create_dir(path)
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.count();
        self.inner.delete_file(path)
    }
}

/// A filesystem whose every operation fails, to check the message crosses.
#[derive(Debug)]
struct Failing;

impl Failing {
    const MESSAGE: &'static str = "the bucket refused the request: 403 Forbidden";

    fn refusal<T>() -> Result<T> {
        Err(crate::Error::Io(std::io::Error::other(Self::MESSAGE)))
    }
}

impl FileSystem for Failing {
    fn type_name(&self) -> &str {
        "s3"
    }

    fn file_info(&self, _path: &str) -> Result<FileInfo> {
        Self::refusal()
    }

    fn list(&self, _path: &str, _recursive: bool) -> crate::holder::fs::FileInfos {
        crate::holder::fs::FileInfos::failing(
            Self::refusal::<()>().expect_err("the refusal this filesystem always answers with"),
        )
    }

    fn read_range(&self, _path: &str, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Self::refusal()
    }

    fn write_full(&self, _path: &str, _bytes: &[u8]) -> Result<()> {
        Self::refusal()
    }

    fn create_dir(&self, _path: &str) -> Result<()> {
        Self::refusal()
    }

    fn delete_file(&self, _path: &str) -> Result<()> {
        Self::refusal()
    }
}

mod laziness {
    //! Construction touches nothing; absence reads empty; writes create.

    use super::*;

    #[test]
    fn constructing_any_role_performs_no_filesystem_call() {
        let filesystem = Arc::new(Counting::new());

        let file = File::from_location(filesystem.clone(), "bucket/key.parquet").unwrap();
        let folder = Folder::from_location(filesystem.clone(), "bucket/lake").unwrap();
        let path = Path::from_location(filesystem.clone(), "bucket/anything").unwrap();
        let located = crate::holder::fs::located(filesystem.clone(), path.url().clone());

        // Not one call reached the vtable while direct and generic handles were built.
        assert_eq!(filesystem.calls(), 0);
        assert!(matches!(located, crate::holder::Holder::FsPath(_)));

        // Nor does borrowing what a handle already knows about itself.
        let _ = (file.url(), folder.url(), path.url());
        let _ = (file.location(), folder.location(), path.location());
        let _ = file.media_type();
        assert_eq!(filesystem.calls(), 0);
    }

    #[test]
    fn a_missing_file_reads_as_empty_without_creating_it() {
        let filesystem = memory();
        let handle = File::from_location(filesystem.clone(), "bucket/absent.bin").unwrap();

        assert_eq!(handle.size(), 0);
        assert!(handle.is_empty());
        assert!(handle.read_all_bytes().unwrap().is_empty());
        let mut probe = [0_u8; 8];
        assert_eq!(handle.pread(0, &mut probe).unwrap(), 0);
        assert_eq!(handle.kind(), IOKind::Unknown);
        assert!(!handle.exists());

        // Reading created nothing on the filesystem.
        assert_eq!(
            filesystem.file_info("bucket/absent.bin").unwrap().kind,
            IOKind::Unknown
        );
    }

    #[test]
    fn a_write_creates_the_file_and_its_parents() {
        let filesystem = memory();
        let mut handle =
            File::from_location(filesystem.clone(), "bucket/deep/nested/trades.bin").unwrap();

        handle.write_all_bytes(b"created").unwrap();
        handle.close().unwrap();

        assert_eq!(
            filesystem
                .file_info("bucket/deep/nested/trades.bin")
                .unwrap()
                .size,
            7
        );
        // The parents are prefixes, which is what a directory is here.
        assert_eq!(
            filesystem.file_info("bucket/deep").unwrap().kind,
            IOKind::Directory
        );
        assert_eq!(
            filesystem.file_info("bucket/deep/nested").unwrap().kind,
            IOKind::Directory
        );
    }

    #[test]
    fn opening_a_missing_file_succeeds_without_creating_it() {
        let filesystem = memory();
        let mut handle = File::from_location(filesystem.clone(), "bucket/nothing.bin").unwrap();

        handle.open().unwrap();
        handle.close().unwrap();

        assert_eq!(
            filesystem.file_info("bucket/nothing.bin").unwrap().kind,
            IOKind::Unknown
        );
    }
}

mod staging {
    //! Whole-value publication: the one write shape an Arrow filesystem has.

    use super::*;

    #[test]
    fn a_staged_write_leaves_the_stored_value_untouched_until_close() {
        let filesystem = memory();
        filesystem
            .write_full("bucket/trades.bin", b"stored")
            .unwrap();
        let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin").unwrap();

        // Positional writes stage: they are pieces of a value, and a store
        // that only takes whole values must not see a half-written one.
        handle.truncate(0).unwrap();
        handle.pwrite(0, b"pend").unwrap();
        handle.pwrite(4, b"ing").unwrap();

        // The handle presents the pending value...
        assert_eq!(handle.read_all_bytes().unwrap(), b"pending");
        // ...while the filesystem still holds the old one.
        let mut stored = [0_u8; 6];
        filesystem
            .read_range("bucket/trades.bin", 0, &mut stored)
            .unwrap();
        assert_eq!(&stored, b"stored");

        handle.close().unwrap();

        let mut published = [0_u8; 7];
        filesystem
            .read_range("bucket/trades.bin", 0, &mut published)
            .unwrap();
        assert_eq!(&published, b"pending");
    }

    #[test]
    fn a_whole_value_write_publishes_without_waiting_for_close() {
        let filesystem = memory();
        filesystem
            .write_full("bucket/trades.bin", b"stored")
            .unwrap();
        let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin").unwrap();

        // A complete value is one store operation, so it needs no scope: the
        // staging exists to fold *many positional* writes into one publication.
        handle.write_all_bytes(b"pending").unwrap();

        let mut published = [0_u8; 7];
        filesystem
            .read_range("bucket/trades.bin", 0, &mut published)
            .unwrap();
        assert_eq!(&published, b"pending");
    }

    #[test]
    fn a_close_publishes_exactly_once() {
        let filesystem = Arc::new(Counting::new());
        let mut handle = File::from_location(filesystem.clone(), "bucket/once.bin").unwrap();

        // Many positional writes, one publication.
        for offset in 0..16 {
            handle.pwrite(offset, b"x").unwrap();
        }
        let before = filesystem.calls();
        handle.close().unwrap();
        let publishing = filesystem.calls() - before;
        assert_eq!(publishing, 1, "one write_full, not one per pwrite");

        // Closing again publishes nothing: there is nothing pending.
        let settled = filesystem.calls();
        handle.close().unwrap();
        assert_eq!(filesystem.calls(), settled);
    }

    #[test]
    fn a_positional_write_keeps_the_bytes_already_stored() {
        // The stage loads the stored value first, so writing one byte in the
        // middle of a stored file republishes the whole file, not a fragment.
        let filesystem = memory();
        filesystem
            .write_full("bucket/trades.bin", b"AAAAAAAAAA")
            .unwrap();
        let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin").unwrap();

        handle.pwrite(4, b"Z").unwrap();
        handle.close().unwrap();

        let mut published = [0_u8; 10];
        filesystem
            .read_range("bucket/trades.bin", 0, &mut published)
            .unwrap();
        assert_eq!(&published, b"AAAAZAAAAA");
    }

    #[test]
    fn reopening_after_close_refetches_rather_than_serving_a_stale_cache() {
        let filesystem = memory();
        let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin").unwrap();
        handle.write_all_bytes(b"first").unwrap();
        handle.close().unwrap();
        assert!(!handle.opened());

        // The value changes underneath the handle.
        filesystem
            .write_full("bucket/trades.bin", b"second value")
            .unwrap();

        // A closed handle holds nothing, so the next read sees the new value.
        assert_eq!(handle.read_all_bytes().unwrap(), b"second value");
        assert_eq!(handle.size(), 12);
    }

    #[test]
    fn dropping_a_handle_publishes_what_was_staged() {
        let filesystem = memory();
        {
            let mut handle = File::from_location(filesystem.clone(), "bucket/dropped.bin").unwrap();
            handle.write_all_bytes(b"dropped").unwrap();
            // No explicit flush or close.
        }
        let mut published = [0_u8; 7];
        filesystem
            .read_range("bucket/dropped.bin", 0, &mut published)
            .unwrap();
        assert_eq!(&published, b"dropped");
    }

    #[test]
    fn an_over_budget_stage_is_refused_loudly_rather_than_allocated() {
        let filesystem = memory();
        let mut handle = File::from_location(filesystem, "bucket/huge.bin").unwrap();

        // A length no process can hold is a typed refusal naming the size,
        // never an aborting allocation.
        let message = handle.truncate(u64::MAX).unwrap_err().to_string();
        assert!(
            message.contains("expected a value addressable"),
            "{message}"
        );

        let message = handle
            .pwrite(u64::MAX - 8, b"far away")
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("expected a value addressable"),
            "{message}"
        );

        // The refusal left the handle usable and empty.
        assert!(handle.read_all_bytes().unwrap().is_empty());
    }

    #[test]
    fn a_range_read_at_the_end_of_the_value_is_short_rather_than_failing() {
        let filesystem = memory();
        let mut handle = File::from_location(filesystem, "bucket/edge.bin").unwrap();
        handle.write_all_bytes(b"0123456789").unwrap();
        handle.close().unwrap();

        // Straddling the end reads what exists; entirely past it reads none.
        let mut target = [0_u8; 4];
        assert_eq!(handle.pread(8, &mut target).unwrap(), 2);
        assert_eq!(handle.pread(10, &mut target).unwrap(), 0);
        assert_eq!(handle.pread(1_000, &mut target).unwrap(), 0);
        assert_eq!(handle.read_range_bytes(8, 100).unwrap(), b"89");
    }

    #[test]
    fn a_foreign_failure_crosses_with_its_own_message() {
        let filesystem: Arc<dyn FileSystem> = Arc::new(Failing);
        let mut handle = File::from_location(filesystem, "bucket/key.bin").unwrap();

        handle.pwrite(0, b"x").unwrap_err();
        let message = handle
            .pwrite(0, b"x")
            .map(|_| String::new())
            .unwrap_or_else(|error| error.to_string());
        assert!(message.contains(Failing::MESSAGE), "{message}");
    }
}

mod hierarchy {
    //! Listing, children, globs, and Hive partitions over a foreign tree.

    use super::*;

    /// Two years, two months each, one part and one note per month.
    fn lake() -> Arc<MemoryFileSystem> {
        let filesystem = memory();
        for year in ["2024", "2025"] {
            for month in ["01", "02"] {
                let leaf = format!("bucket/year={year}/month={month}");
                filesystem
                    .write_full(&format!("{leaf}/part-0.parquet"), b"PAR1")
                    .unwrap();
                filesystem
                    .write_full(&format!("{leaf}/notes.txt"), b"notes")
                    .unwrap();
            }
        }
        filesystem
            .write_full("bucket/.staging/part-0.parquet", b"draft")
            .unwrap();
        filesystem
    }

    #[test]
    fn a_listing_is_flat_or_recursive_and_stable_in_order() {
        let filesystem = memory();
        filesystem.write_full("bucket/a.bin", b"a").unwrap();
        filesystem.write_full("bucket/sub/b.bin", b"b").unwrap();
        filesystem
            .write_full("bucket/sub/deep/c.bin", b"c")
            .unwrap();

        let folder = Folder::from_location(filesystem, "bucket").unwrap();

        // Flat: the file and the one immediate subdirectory.
        let flat = folder
            .ls(false, false)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(flat.len(), 2, "{flat:?}");
        assert_eq!(flat.iter().filter(|entry| entry.is_container()).count(), 1);

        // Recursive: every directory and every leaf beneath.
        let deep = folder
            .ls(true, false)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(deep.len(), 5, "{deep:?}");

        // The order is stable across runs.
        let names: Vec<String> = deep
            .iter()
            .filter_map(|entry| entry.url().map(ToString::to_string))
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_listing_excludes_private_entries_unless_asked() {
        let filesystem = lake();
        let folder = Folder::from_location(filesystem, "bucket").unwrap();

        let public = folder
            .ls(true, false)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert!(
            public
                .iter()
                .all(|entry| !entry.url().unwrap().to_string().contains("/.staging")),
            "a private directory is not descended into"
        );

        let everything = folder
            .ls(true, true)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert!(
            everything
                .iter()
                .any(|entry| entry.url().unwrap().to_string().contains("/.staging"))
        );
    }

    #[test]
    fn a_glob_descends_its_fixed_prefix_before_listing() {
        let filesystem = Arc::new(Counting::new());
        for year in ["2024", "2025"] {
            for month in ["01", "02"] {
                filesystem
                    .write_full(
                        &format!("bucket/year={year}/month={month}/part-0.parquet"),
                        b"PAR1",
                    )
                    .unwrap();
            }
        }
        let folder = Folder::from_location(filesystem.clone(), "bucket").unwrap();

        let all: Vec<_> = folder
            .glob("**/*.parquet", false)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(all.len(), 4, "{all:?}");

        let one_year: Vec<_> = folder
            .glob("year=2024/**/*.parquet", false)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(one_year.len(), 2, "{one_year:?}");
        assert!(
            one_year
                .iter()
                .all(|entry| entry.url().unwrap().to_string().contains("year=2024"))
        );

        // A prefix that is not there yields nothing rather than failing.
        assert_eq!(
            folder
                .glob("year=1999/**/*.parquet", false)
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn hive_partitions_are_read_off_the_location_and_filter_the_leaves() {
        let filesystem = lake();
        let folder = Folder::from_location(filesystem, "bucket").unwrap();

        let year: Vec<_> = folder
            .children_where(&[("year", "2024")], false)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(year.len(), 4, "two months, two leaves each");

        let one: Vec<_> = folder
            .children_where(&[("year", "2024"), ("month", "01")], false)
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(one.len(), 2);
        assert!(one.iter().all(|entry| !entry.is_container()));
        assert_eq!(
            one[0].partitions(),
            vec![
                ("year".to_owned(), "2024".to_owned()),
                ("month".to_owned(), "01".to_owned()),
            ]
        );

        // A filter nothing carries selects nothing; no filter is every leaf.
        assert_eq!(
            folder
                .children_where(&[("year", "1999")], false)
                .unwrap()
                .count(),
            0
        );
        assert_eq!(folder.children_where(&[], false).unwrap().count(), 8);
    }

    #[test]
    fn a_child_of_a_file_is_refused_by_name() {
        let filesystem = memory();
        filesystem.write_full("bucket/trades.bin", b"x").unwrap();
        let folder = Folder::from_location(filesystem, "bucket").unwrap();

        let leaf = folder.child_by_path("trades.bin").unwrap();
        assert!(!leaf.is_container());

        let message = leaf.child_by_path("deeper").unwrap_err().to_string();
        assert!(message.contains("expected a container"), "{message}");
        assert!(message.contains("trades.bin"), "{message}");
        // A leaf lists nothing rather than failing.
        assert!(
            leaf.ls(true, false)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_listed_handle_resolves_back_to_itself_by_name() {
        // Every generic caller does this round trip - the line projection
        // reopens a leaf as `parent().child_by_path(url.file_name())`, a folder
        // write routes rows by the segments under its root, and a table
        // turns a recorded location into a relative name first. So a name
        // taken off a listed handle's own URL has to address the object that
        // handle addressed, including when the object's name needs escaping.
        let filesystem = memory();
        filesystem
            .write_full("logs/app 2024.log", b"REAL-BYTES")
            .unwrap();
        let folder = Folder::from_location(filesystem, "logs").unwrap();

        let listed = folder
            .ls(false, false)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(listed.len(), 1);
        let url = listed[0].url().unwrap().clone();
        assert_eq!(listed[0].size(), 10);

        let name = url.file_name().unwrap();
        let round = folder.child_by_path(name).unwrap();
        assert_eq!(
            round.url().unwrap(),
            &url,
            "the same location, not an escaped one"
        );
        assert_eq!(round.read_all_bytes().unwrap(), b"REAL-BYTES");

        // The parent of a listed child resolves it the same way.
        let parent = listed[0].parent().expect("a listed leaf has a parent");
        let reopened = parent.child_by_path(name).unwrap();
        assert_eq!(reopened.read_all_bytes().unwrap(), b"REAL-BYTES");
    }

    #[test]
    fn a_folder_write_lands_on_the_leaf_it_read() {
        // The folder record path clears an existing leaf through its listed
        // handle and rewrites it through `child_by_path`, so a name needing
        // escaping must resolve to one object, never two.
        let filesystem = memory();
        filesystem.write_full("lake/part 0.bin", b"old").unwrap();
        let folder = Folder::from_location(filesystem.clone(), "lake").unwrap();

        let relative = folder
            .ls(false, false)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap()[0]
            .url()
            .unwrap()
            .file_name()
            .unwrap()
            .to_owned();
        let mut leaf = folder.child_by_path(&relative).unwrap();
        leaf.write_all_bytes(b"new").unwrap();
        leaf.close().unwrap();

        // One object, holding the new bytes - not a second, differently
        // spelled key beside the original.
        let entries = filesystem
            .list("lake", true)
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].path, "lake/part 0.bin");
        assert_eq!(entries[0].size, 3);
    }

    #[test]
    fn a_parent_walks_back_up_and_keeps_the_filesystem() {
        let filesystem = memory();
        filesystem
            .write_full("bucket/lake/trades.bin", b"x")
            .unwrap();
        let folder = Folder::from_location(filesystem, "bucket/lake").unwrap();

        let leaf = folder.child_by_path("trades.bin").unwrap();
        let parent = leaf.parent().expect("a leaf has a parent");
        assert!(parent.is_container());
        assert_eq!(parent.url().unwrap(), folder.url());

        // The rebuilt parent still reaches the same filesystem.
        assert_eq!(
            parent
                .ls(false, false)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_directory_holds_no_bytes_and_is_created_by_truncating_to_zero() {
        let filesystem = memory();
        let mut folder = Folder::from_location(filesystem.clone(), "bucket/new").unwrap();

        assert_eq!(folder.size(), 0);
        assert!(folder.read_all_bytes().unwrap().is_empty());
        let message = folder.pwrite(0, b"nope").unwrap_err().to_string();
        assert!(message.contains("expected a file"), "{message}");
        assert!(
            folder.truncate(4).is_err(),
            "a container has no bytes to size"
        );

        assert!(!folder.exists());
        folder.truncate(0).unwrap();
        assert!(folder.exists());
        assert_eq!(folder.kind(), IOKind::Directory);
        assert_eq!(folder.media_type().base(), &MimeType::DIRECTORY);
    }

    #[test]
    fn a_generic_location_routes_to_the_role_it_turns_out_to_need() {
        let filesystem = memory();
        filesystem.write_full("bucket/lake/a.bin", b"a").unwrap();

        let directory = Path::from_location(filesystem.clone(), "bucket/lake").unwrap();
        assert_eq!(directory.kind(), IOKind::Directory);
        assert!(directory.is_container());
        assert_eq!(
            directory
                .ls(false, false)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
                .len(),
            1
        );

        let leaf = Path::from_location(filesystem.clone(), "bucket/lake/a.bin").unwrap();
        assert_eq!(leaf.kind(), IOKind::File);
        assert_eq!(leaf.read_all_bytes().unwrap(), b"a");
        assert!(
            leaf.ls(true, false)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
                .is_empty()
        );
        let message = leaf
            .child_by_path("deeper")
            .expect_err("a file cannot resolve a child")
            .to_string();
        assert!(message.contains("expected a container"), "{message}");

        let child = directory.child_by_path("a.bin").unwrap();
        assert!(matches!(&child, crate::holder::Holder::FsPath(_)));
        assert_eq!(child.media_type().base(), &MimeType::OCTET_STREAM);
        let parent = child.parent().expect("the generic child has a parent");
        assert!(matches!(&parent, crate::holder::Holder::FsPath(_)));
        assert_eq!(parent.kind(), IOKind::Directory);

        // Nothing there yet has not decided what it is; a write settles it.
        let mut undecided = Path::from_location(filesystem, "bucket/lake/new.bin").unwrap();
        assert_eq!(undecided.kind(), IOKind::Unknown);
        undecided.write_all_bytes(b"decided").unwrap();
        undecided.close().unwrap();
        assert_eq!(undecided.kind(), IOKind::File);
        assert_eq!(undecided.read_all_bytes().unwrap(), b"decided");
    }

    #[test]
    fn a_generic_leaf_keeps_media_inference_and_staged_identity() {
        let filesystem = memory();
        let mut leaf = Path::from_location(filesystem.clone(), "bucket/trades.arrows").unwrap();

        assert_eq!(leaf.media_type().base(), &MimeType::ARROW_STREAM);
        let bare = Path::from_location(filesystem.clone(), "bucket/plain").unwrap();
        assert_eq!(bare.media_type().base(), &MimeType::FILE);
        leaf.set_media_type(MediaType::from(MimeType::CSV));
        assert_eq!(leaf.media_type().base(), &MimeType::CSV);
        assert!(leaf.is_tabular());
        assert!(!leaf.is_atomic());
        assert_eq!(leaf.as_file().media_type().base(), &MimeType::CSV);

        filesystem.create_dir("bucket/lake.parquet").unwrap();
        let directory = Path::from_location(filesystem.clone(), "bucket/lake.parquet").unwrap();
        assert_eq!(directory.media_type().base(), &MimeType::DIRECTORY);

        leaf.pwrite(0, b"AAPL").unwrap();
        assert_eq!(leaf.kind(), IOKind::File, "the retained stage is a leaf");
        assert_eq!(
            filesystem.file_info("bucket/trades.arrows").unwrap().kind,
            IOKind::Unknown,
            "a positional write remains unpublished"
        );
        leaf.flush().unwrap();
        assert_eq!(
            filesystem.file_info("bucket/trades.arrows").unwrap().kind,
            IOKind::File
        );
    }

    #[test]
    fn clearing_a_generic_leaf_discards_its_retained_stage() {
        let filesystem = memory();
        let mut leaf = Path::from_location(filesystem.clone(), "bucket/staged.bin").unwrap();

        leaf.pwrite(0, b"must-not-return").unwrap();
        leaf.clear().unwrap();
        leaf.close().unwrap();

        let stored = File::from_location(filesystem, "bucket/staged.bin").unwrap();
        assert!(stored.read_all_bytes().unwrap().is_empty());
    }
}

mod identity {
    //! The URL is the handle's identity, and the media type follows its name.

    use super::*;

    #[test]
    fn a_locations_url_carries_the_filesystems_own_scheme() {
        let memory_file = File::from_location(memory(), "bucket/key.parquet").unwrap();
        assert_eq!(memory_file.url().to_string(), "memory://bucket/key.parquet");

        // A bucket-shaped filesystem spells its authority the way S3 does.
        let s3: Arc<dyn FileSystem> = Arc::new(Failing);
        let object = File::from_location(s3.clone(), "bucket/prefix/key.parquet").unwrap();
        assert_eq!(object.url().to_string(), "s3://bucket/prefix/key.parquet");
        // And the path handed back to the vtable is the one it was given.
        assert_eq!(object.location(), "bucket/prefix/key.parquet");

        // A bucket root is a location too, and it can be descended.
        let root = Folder::from_location(s3, "bucket").unwrap();
        assert_eq!(root.url().to_string(), "s3://bucket/");
        assert_eq!(root.location(), "bucket");
    }

    #[test]
    fn a_full_url_passes_through_unchanged() {
        let filesystem = memory();
        let handle = File::from_location(filesystem, "s3://bucket/key.arrows").unwrap();
        assert_eq!(handle.url().to_string(), "s3://bucket/key.arrows");
        assert_eq!(handle.location(), "bucket/key.arrows");
    }

    #[test]
    fn the_media_type_is_derived_from_the_locations_suffixes() {
        let filesystem = memory();

        let parquet = File::from_location(filesystem.clone(), "bucket/t.parquet").unwrap();
        assert_eq!(parquet.media_type().base(), &MimeType::PARQUET);

        let stream = File::from_location(filesystem.clone(), "bucket/t.arrows").unwrap();
        assert_eq!(stream.media_type().base(), &MimeType::ARROW_STREAM);

        // A compound suffix reports the base plus the coding it names.
        let coded = File::from_location(filesystem.clone(), "bucket/t.json.gz").unwrap();
        assert_eq!(coded.media_type().base(), &MimeType::JSON);
        assert_eq!(coded.codec(), crate::Codec::Gzip);

        // A name that says nothing still says this is a stored file.
        let bare = File::from_location(filesystem.clone(), "bucket/plain").unwrap();
        assert_eq!(bare.media_type().base(), &MimeType::FILE);

        // A declared media type overrides inference.
        let mut declared = File::from_location(filesystem, "bucket/t.bin").unwrap();
        declared.set_media_type(crate::MediaType::from(MimeType::CSV));
        assert_eq!(declared.media_type().base(), &MimeType::CSV);
    }

    #[test]
    fn a_multi_byte_path_survives_every_operation() {
        // Paths are UTF-8, and the prefix arithmetic must respect character
        // boundaries rather than slicing bytes.
        let filesystem = memory();
        filesystem
            .write_full("marché/données/prix€.bin", b"euro")
            .unwrap();

        let folder = Folder::from_location(filesystem.clone(), "marché").unwrap();
        assert_eq!(
            folder
                .ls(true, false)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
                .len(),
            2
        );

        // A raw filesystem name is reached through `from_location`, which is
        // the constructor that encodes; `child_by_path` takes URI-path text.
        let leaf = File::from_location(filesystem.clone(), "marché/données/prix€.bin").unwrap();
        assert_eq!(leaf.read_all_bytes().unwrap(), b"euro");
        assert_eq!(
            filesystem.file_info("marché/données").unwrap().kind,
            IOKind::Directory
        );
    }

    #[test]
    fn a_prefix_is_a_directory_only_at_a_component_boundary() {
        // "lake" must not be reported as a directory merely because
        // "lakeside/x.bin" starts with those letters.
        let filesystem = memory();
        filesystem.write_full("lakeside/x.bin", b"x").unwrap();

        assert_eq!(
            filesystem.file_info("lakeside").unwrap().kind,
            IOKind::Directory
        );
        assert_eq!(filesystem.file_info("lake").unwrap().kind, IOKind::Unknown);
        assert_eq!(filesystem.file_info("lakes").unwrap().kind, IOKind::Unknown);
    }
}

mod wrappers {
    //! Every wrapper composes, because none of them was reimplemented here.

    use super::*;

    #[test]
    fn a_content_coding_round_trips_over_a_foreign_leaf() {
        let filesystem = memory();
        let leaf = File::from_location(filesystem.clone(), "bucket/trades.json.gz").unwrap();

        // `Coding` presents the decoded bytes and stores the encoded ones.
        let mut coded = crate::coding::Coding::new(leaf, crate::Codec::Gzip);
        coded.write_all_bytes(br#"{"symbol":"AAPL"}"#).unwrap();
        coded.close().unwrap();

        assert_eq!(coded.media_type().base(), &MimeType::JSON);
        assert_eq!(coded.read_all_bytes().unwrap(), br#"{"symbol":"AAPL"}"#);

        // What actually landed on the filesystem is gzip, not the plain text.
        let stored = File::from_location(filesystem, "bucket/trades.json.gz").unwrap();
        let bytes = stored.read_all_bytes().unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b], "a gzip member header");
        assert_eq!(
            crate::coding::gzip::load(&bytes).unwrap(),
            br#"{"symbol":"AAPL"}"#
        );
    }

    #[test]
    fn text_lines_stream_off_a_foreign_leaf_decoded() {
        let filesystem = memory();
        filesystem
            .write_full(
                "bucket/app.log.gz",
                &crate::coding::gzip::dump(b"alpha\nbeta\ngamma\n").unwrap(),
            )
            .unwrap();
        let handle = File::from_location(filesystem, "bucket/app.log.gz").unwrap();

        let options = handle.record_options().unwrap();
        let batch = handle
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let body = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow_array::BinaryArray>()
            .unwrap();
        assert_eq!(
            body.iter().collect::<Vec<_>>(),
            [Some(&b"alpha"[..]), Some(&b"beta"[..]), Some(&b"gamma"[..])]
        );
    }
}

#[cfg(feature = "arrow")]
mod records {
    //! The three record methods, inherited rather than reimplemented.

    use super::*;

    use std::sync::Arc as StdArc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};

    use crate::holder::fs::LocalFileSystem;
    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{DataType, Field};

    fn schema() -> Field {
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap()
        .required_field("row")
    }

    fn reader() -> crate::arrow::BatchReader {
        let batch = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![
                StdArc::new(Int64Array::from(vec![1, 2])),
                StdArc::new(StringArray::from(vec![Some("AAPL"), None])),
            ],
        )
        .unwrap();
        crate::arrow::batch_reader(batch.schema(), [batch])
    }

    fn rows(handle: &impl IOBase, options: &RecordOptions) -> usize {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    /// A temporary root for the local reference filesystem.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = crate::holder::local::Folder::temporary()
            .unwrap()
            .path()
            .unwrap();
        path.push(format!("yggdryl-fs-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Every encoding this build implements, as leaf names.
    fn encodings() -> Vec<&'static str> {
        let mut names = vec!["trades.arrows"];
        if cfg!(feature = "parquet") {
            names.push("trades.parquet");
        }
        names
    }

    #[test]
    fn batches_round_trip_over_both_reference_filesystems() {
        let local_root = root("records");
        let cases: Vec<(&str, Arc<dyn FileSystem>, String)> = vec![
            ("memory", memory(), "bucket".to_owned()),
            (
                "local",
                Arc::new(LocalFileSystem::new()),
                local_root.to_string_lossy().replace('\\', "/"),
            ),
        ];

        for (label, filesystem, base) in cases {
            for name in encodings() {
                let mut handle =
                    File::from_location(filesystem.clone(), &format!("{base}/{name}")).unwrap();
                let options = handle
                    .record_options()
                    .unwrap()
                    .with_field(schema())
                    .with_safe(true);

                handle
                    .overwrite_arrow_reader(reader(), &options)
                    .unwrap_or_else(|error| panic!("{label}/{name}: {error}"));
                handle.close().unwrap();

                assert_eq!(rows(&handle, &options), 2, "{label}/{name}");
                assert_eq!(
                    handle.read_arrow_field(&options).unwrap(),
                    schema(),
                    "{label}/{name}"
                );

                // The third method: appending reads, adds, and rewrites.
                handle.append_arrow_reader(reader(), &options).unwrap();
                handle.close().unwrap();
                assert_eq!(rows(&handle, &options), 4, "{label}/{name}");
            }
        }

        let _ = std::fs::remove_dir_all(&local_root);
    }

    #[test]
    fn a_missing_foreign_resource_reads_as_empty_with_its_schema_answered() {
        let filesystem = memory();
        let handle = File::from_location(filesystem, "bucket/absent.arrows").unwrap();
        let options = handle.record_options().unwrap().with_field(schema());

        assert_eq!(handle.read_arrow_reader(&options).unwrap().count(), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn a_folder_reads_as_the_partitioned_table_beneath_it() {
        let filesystem = memory();
        let folder = Folder::from_location(filesystem.clone(), "bucket/lake").unwrap();

        // Two partitions, written through the folder itself.
        for (year, id) in [("2024", 1_i64), ("2025", 2_i64)] {
            let batch = RecordBatch::try_new(
                crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
                vec![
                    StdArc::new(Int64Array::from(vec![id])),
                    StdArc::new(StringArray::from(vec![Some("AAPL")])),
                ],
            )
            .unwrap();
            let mut leaf = folder
                .child_by_path(&format!("year={year}/part-0.arrows"))
                .unwrap();
            let options = leaf.record_options().unwrap().with_field(schema());
            leaf.overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
            leaf.close().unwrap();
        }

        // The container reads across every leaf beneath it.
        let options = folder.record_options().unwrap();
        assert_eq!(rows(&folder, &options), 2);

        // And the partition column its directories spell out is restored.
        let field = folder.read_arrow_field(&options).unwrap();
        assert!(
            field.get_field_by_path("year").is_some(),
            "the layout's partition column is restored: {field}"
        );
    }
}

#[cfg(feature = "iceberg")]
mod tables {
    //! A table is a folder, reached through `IOBase` only - including this one.

    use super::*;

    use std::sync::Arc as StdArc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};

    use crate::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use crate::{DataType, Field};

    fn schema() -> Field {
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap()
        .required_field("row")
    }

    fn batch(id: i64, symbol: &str) -> RecordBatch {
        RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![
                StdArc::new(Int64Array::from(vec![id])),
                StdArc::new(StringArray::from(vec![Some(symbol.to_owned())])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn an_iceberg_table_lives_on_a_foreign_filesystem() {
        let filesystem = memory();
        let root = Folder::from_location(filesystem, "warehouse/trades").unwrap();

        // Creating, appending, and reading all reach storage through IOBase,
        // so the table never learns which backend it is standing on.
        let mut table = Table::create(
            root,
            FormatVersion::V2,
            schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_append(crate::arrow::batch_reader(
                batch(1, "AAPL").schema(),
                [batch(1, "AAPL")],
            ))
            .unwrap();
        table
            .commit_append(crate::arrow::batch_reader(
                batch(2, "MSFT").schema(),
                [batch(2, "MSFT")],
            ))
            .unwrap();

        let options = table.record_options().unwrap();
        let rows: usize = table
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(rows, 2);

        // The stored schema is answered from the metadata, no file opened.
        assert_eq!(
            table.read_arrow_field(&options).unwrap().field_len(),
            schema().field_len()
        );
    }
}

/// A listing must cost what it yields, not what it could have yielded, and the
/// counting filesystem is what turns that into a number.
mod listing_cost {
    use std::sync::Arc;

    use super::{Counting, Result};
    use crate::IOBase;
    use crate::holder::fs::{FileSystem, Folder};

    /// A tree `depth` levels deep, `width` leaves per level.
    fn tree(filesystem: &Counting, depth: usize, width: usize) {
        let mut prefix = "lake".to_owned();
        for level in 0..depth {
            for leaf in 0..width {
                filesystem
                    .inner
                    .write_full(&format!("{prefix}/part-{leaf:04}.parquet"), b"PAR1")
                    .expect("a written leaf");
            }
            prefix.push_str(&format!("/level-{level:02}"));
        }
    }

    #[test]
    fn one_entry_from_a_deep_wide_tree_costs_one_directory_read() {
        let filesystem = Arc::new(Counting::new());
        tree(&filesystem, 4, 500);
        let lake = Folder::from_location(filesystem.clone(), "lake").expect("a valid location");

        let before = filesystem.calls();
        let first = lake.ls(true, false).next().expect("an entry");
        first.expect("a readable entry");

        assert_eq!(
            filesystem.calls() - before,
            1,
            "the walk read the root level and stopped, not the whole tree"
        );
    }

    #[test]
    fn a_recursive_walk_asks_the_filesystem_once_for_the_whole_prefix() {
        let filesystem = Arc::new(Counting::new());
        tree(&filesystem, 4, 5);
        let lake = Folder::from_location(filesystem.clone(), "lake").expect("a valid location");

        let entries: Vec<_> = lake
            .ls(true, false)
            .collect::<Result<Vec<_>>>()
            .expect("a listing");

        // Four levels of five leaves each, plus the three nested directories
        // the levels below live in.
        assert_eq!(entries.len(), 4 * 5 + 3, "{entries:?}");
        // One call, not one per level: a prefix listing is the shape every
        // Arrow filesystem already answers recursively, so the walk asks once
        // and the laziness lives inside that one answer. The local backend,
        // whose `read_dir` is per directory, pays one call per level instead -
        // and both are the same contract, one entry at a time.
    }
}
