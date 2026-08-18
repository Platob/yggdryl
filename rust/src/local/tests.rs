//! Local directories and mapped files, exercised against a real temp tree.

mod mapped {
    use crate::io::IOBase;
    use crate::local::{File, Folder};

    fn path(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-mmap-{label}-{}.bin", std::process::id()));
        // Teardown through the abstraction: absence is a no-op success.
        File::new(&path)
            .expect("a local leaf")
            .remove(false)
            .expect("a removable leaf");
        path
    }

    #[test]
    fn a_mapped_file_round_trips_and_resizes() {
        let path = path("roundtrip");
        {
            let mut mapped = File::create(&path).unwrap();
            assert!(mapped.is_empty());

            mapped.pwrite(0, b"trade").unwrap();
            assert_eq!(mapped.size(), 5);

            // Growing past the mapping remaps rather than failing.
            let large = vec![7_u8; 256 * 1024];
            mapped.append(&large).unwrap();
            assert_eq!(mapped.size(), 5 + large.len() as u64);
            assert!(mapped.capacity() >= mapped.size());
            mapped.flush().unwrap();
        }

        // The on-disk length is the logical size, not the mapping capacity.
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 5 + 256 * 1024);

        // Reopening sees the same bytes.
        let reopened = File::new(&path).unwrap();
        assert_eq!(reopened.size(), 5 + 256 * 1024);
        assert_eq!(reopened.read_range(0, 5).unwrap(), b"trade");
        assert_eq!(reopened.url().unwrap().extension(), Some("bin"));

        // Teardown through the abstraction: absence is a no-op success.
        File::new(&path)
            .expect("a local leaf")
            .remove(false)
            .expect("a removable leaf");
    }

    #[test]
    fn a_handle_for_a_missing_file_touches_nothing() {
        let path = path("lazy");
        assert!(!path.exists());

        let handle = File::new(&path).unwrap();
        // Constructing the handle must not create the file.
        assert!(!path.exists());
        assert!(!handle.exists());

        // A read of a missing file is empty, not an error.
        assert_eq!(handle.size(), 0);
        assert!(handle.is_empty());
        let mut probe = [0_u8; 8];
        assert_eq!(handle.pread(0, &mut probe).unwrap(), 0);
        assert!(handle.read_all().unwrap().is_empty());
        // Reading still did not create it.
        assert!(!path.exists());

        // The media type comes from the name, which exists even when the file
        // does not.
        assert_eq!(handle.url().unwrap().extension(), Some("bin"));
    }

    #[test]
    fn the_first_write_creates_the_file_and_its_parent() {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-mmap-create-{}", std::process::id()));
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path.push("nested");
        path.push("trades.bin");

        let mut handle = File::new(&path).unwrap();
        assert!(!path.exists());

        handle.pwrite(0, b"created").unwrap();
        handle.flush().unwrap();

        assert!(path.exists());
        assert_eq!(handle.read_all().unwrap(), b"created");

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_mapped_file_zero_fills_a_write_gap() {
        let path = path("gap");
        let mut mapped = File::create(&path).unwrap();
        mapped.pwrite(0, b"ab").unwrap();
        mapped.pwrite(5, b"z").unwrap();

        assert_eq!(mapped.size(), 6);
        assert_eq!(mapped.read_all().unwrap(), b"ab\0\0\0z");

        drop(mapped);
        // Teardown through the abstraction: absence is a no-op success.
        File::new(&path)
            .expect("a local leaf")
            .remove(false)
            .expect("a removable leaf");
    }
}

mod hierarchy {
    use crate::generic::Holder;
    use crate::io::IOBase;
    use crate::local::Folder;

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-tree-{label}-{}", std::process::id()));
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path
    }

    #[test]
    fn a_directory_handle_touches_nothing_until_used() {
        let path = root("lazy");
        let directory = Folder::new(&path).unwrap();

        assert!(!path.exists());
        assert!(!directory.exists());
        assert!(directory.is_container());
        assert_eq!(directory.size(), 0);

        // Listing a directory that does not exist is empty, not an error.
        assert!(directory.ls(false, false).unwrap().is_empty());
        assert!(directory.ls(true, false).unwrap().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn children_resolve_to_directories_and_mapped_leaves() {
        let path = root("children");
        let directory = Folder::new(&path).unwrap();
        directory.create().unwrap();

        // A write through a child creates the leaf.
        let mut leaf = directory.child_by("trades.arrows").unwrap();
        leaf.pwrite(0, b"payload").unwrap();
        leaf.flush().unwrap();
        assert!(matches!(leaf, Holder::File(_)));
        assert!(!leaf.is_container());

        // A nested child creates its parent directory on write.
        let mut nested = directory.child_by("sub/inner.bin").unwrap();
        nested.pwrite(0, b"deep").unwrap();
        nested.flush().unwrap();

        let listed = directory.ls(false, false).unwrap();
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert!(listed.iter().any(Holder::is_container));
        assert!(listed.iter().any(|entry| !entry.is_container()));

        // Recursion reaches the nested leaf.
        let deep = directory.ls(true, false).unwrap();
        assert_eq!(deep.len(), 3, "{deep:?}");

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn parents_walk_back_up_the_tree() {
        let path = root("parents");
        let directory = Folder::new(&path).unwrap();
        directory.create().unwrap();

        let mut leaf = directory.child_by("leaf.bin").unwrap();
        leaf.pwrite(0, b"x").unwrap();
        leaf.flush().unwrap();

        // A leaf's parent is the directory holding it.
        let parent = leaf.parent().expect("a mapped file has a parent");
        assert!(parent.is_container());
        assert_eq!(parent.url().unwrap(), directory.url());

        // A buffer has no location, so it has no parent.
        assert!(crate::io::Buffer::new().parent().is_none());
    }

    #[test]
    fn a_relative_child_resolves_dot_segments() {
        let path = root("relative");
        let directory = Folder::new(&path).unwrap();
        let sideways = directory.child_by("sub/../beside.bin").unwrap();

        let url = sideways.url().unwrap().to_string();
        assert!(url.ends_with("/beside.bin"), "{url}");
        assert!(!url.contains(".."), "{url}");
    }

    #[test]
    fn a_directory_rejects_byte_writes_with_the_reason() {
        let path = root("bytes");
        let mut directory = Folder::new(&path).unwrap();

        let message = directory.pwrite(0, b"nope").unwrap_err().to_string();
        assert!(message.contains("expected a file"), "{message}");
        // Reads are empty rather than an error.
        assert!(directory.read_all().unwrap().is_empty());

        // Truncating to zero is how a directory is brought into being.
        directory.truncate(0).unwrap();
        assert!(path.exists());
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn open_and_close_bracket_cached_state() {
        let path = root("context");
        let directory = Folder::new(&path).unwrap();
        directory.create().unwrap();

        let mut leaf = directory.child_by("cached.bin").unwrap();
        assert!(!leaf.opened());

        leaf.pwrite(0, b"cached").unwrap();
        assert!(leaf.opened());

        // Closing publishes and releases; the handle stays usable.
        leaf.close().unwrap();
        assert!(!leaf.opened());
        assert_eq!(leaf.read_all().unwrap(), b"cached");

        // Opening a handle for a missing file caches nothing and creates nothing.
        let mut absent = directory.child_by("absent.bin").unwrap();
        absent.open().unwrap();
        assert!(!absent.opened());

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }
}

/// One generic location resolves to the implementation it turns out to need.
mod generic_path {
    use crate::io::IOBase;
    use crate::local::{Folder, Path};
    use crate::{IOKind, MimeType};

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-path-{label}-{}", std::process::id()));
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path
    }

    #[test]
    fn a_location_reports_what_it_actually_is() {
        let path = root("kind");
        std::fs::create_dir_all(&path).unwrap();

        let directory = Path::new(&path).unwrap();
        assert_eq!(directory.kind(), IOKind::Directory);
        assert!(directory.is_container());
        assert_eq!(directory.media_type().base(), &MimeType::DIRECTORY);

        // A location that does not exist has not decided what it is.
        let missing = Path::new(path.join("absent.arrows")).unwrap();
        assert_eq!(missing.kind(), IOKind::Unknown);
        assert!(!missing.is_container());
        assert!(missing.read_all().unwrap().is_empty());
        assert_eq!(missing.size(), 0);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn a_write_decides_an_undecided_location() {
        let path = root("write");
        std::fs::create_dir_all(&path).unwrap();

        let mut leaf = Path::new(path.join("trades.bin")).unwrap();
        assert_eq!(leaf.kind(), IOKind::Unknown);

        leaf.write_all_bytes(b"AAPL").unwrap();
        leaf.flush().unwrap();

        // Writing created a file, and reading it goes through that file.
        assert_eq!(leaf.kind(), IOKind::File);
        assert_eq!(leaf.read_all().unwrap(), b"AAPL");

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn a_directory_location_lists_and_a_leaf_location_does_not() {
        let path = root("hierarchy");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("a.bin"), b"a").unwrap();

        let directory = Path::new(&path).unwrap();
        assert_eq!(directory.ls(false, false).unwrap().len(), 2);
        assert_eq!(directory.ls(true, false).unwrap().len(), 2);

        let leaf = Path::new(path.join("a.bin")).unwrap();
        assert_eq!(leaf.kind(), IOKind::File);
        assert!(leaf.ls(true, false).unwrap().is_empty());
        assert_eq!(leaf.read_all().unwrap(), b"a");

        // Children resolve as further generic locations.
        let child = directory.child_by("a.bin").unwrap();
        assert_eq!(child.read_all().unwrap(), b"a");
        assert_eq!(child.parent().unwrap().kind(), IOKind::Directory);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn a_location_can_be_addressed_as_a_directory_before_it_exists() {
        let path = root("as-directory");
        let location = Path::new(&path).unwrap();
        assert_eq!(location.kind(), IOKind::Unknown);

        // Truncating to zero is the write that brings a directory into being.
        location.as_directory().unwrap().create().unwrap();
        assert_eq!(location.kind(), IOKind::Directory);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }
}

/// The three roles are what a backend implements; `local` is the reference.
mod roles {
    use crate::io::{IOBase, IOFile, IOFolder, IOPath};
    use crate::local::{File, Folder, Path};
    use crate::{IOKind, MimeType};

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-roles-{label}-{}", std::process::id()));
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path
    }

    #[test]
    fn the_folder_role_supplies_the_byte_half_of_the_contract() {
        let path = root("folder");
        let mut folder = Folder::new(&path).unwrap();

        // A container holds no bytes, refuses byte writes, and is created by
        // truncating it to zero - all of that comes from the role.
        assert_eq!(folder.size(), 0);
        assert!(folder.read_all().unwrap().is_empty());
        let message = folder.pwrite(0, b"x").unwrap_err().to_string();
        assert!(message.contains("got the directory"), "{message}");
        assert!(folder.truncate(4).is_err());

        assert!(!folder.folder_exists());
        folder.truncate(0).unwrap();
        assert!(folder.folder_exists());
        assert_eq!(folder.kind(), IOKind::Directory);
        assert_eq!(folder.media_type().base(), &MimeType::DIRECTORY);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn the_file_role_supplies_the_container_half_of_the_contract() {
        let path = root("file");
        std::fs::create_dir_all(&path).unwrap();
        let leaf = File::new(path.join("trades.bin")).unwrap();

        // A leaf contains nothing and resolves no children.
        assert!(leaf.file_ls().unwrap().is_empty());
        let message = leaf.file_child_by("child").unwrap_err().to_string();
        assert!(message.contains("got the file"), "{message}");

        // Its kind follows from whether it exists yet.
        assert_eq!(leaf.file_kind(), IOKind::Unknown);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }

    #[test]
    fn the_path_role_answers_by_looking() {
        let path = root("path");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("a.bin"), b"a").unwrap();

        let folder = Path::new(&path).unwrap();
        assert!(folder.is_folder());
        assert!(!folder.is_file());
        assert_eq!(folder.path_kind(), IOKind::Directory);
        assert_eq!(folder.path_media_type().base(), &MimeType::DIRECTORY);

        let leaf = Path::new(path.join("a.bin")).unwrap();
        assert!(leaf.is_file());
        assert_eq!(leaf.path_kind(), IOKind::File);

        let absent = Path::new(path.join("absent.arrows")).unwrap();
        assert!(!absent.path_exists());
        assert_eq!(absent.path_kind(), IOKind::Unknown);
        // An undecided location still reports what its name says it holds.
        assert_eq!(absent.path_media_type().base(), &MimeType::ARROW_STREAM);

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }
}

/// A listing skips private names unless a caller asks for them.
mod privacy {
    use crate::Url;
    use crate::io::IOBase;
    use crate::local::Folder;

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("yggdryl-private-{label}-{}", std::process::id()));
        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
        path
    }

    #[test]
    fn a_dot_prefixed_name_is_private() {
        assert!(Url::from_str("file:///project/.git").unwrap().is_private());
        assert!(
            Url::from_str("file:///project/.DS_Store")
                .unwrap()
                .is_private()
        );
        assert!(
            !Url::from_str("file:///project/trades.arrows")
                .unwrap()
                .is_private()
        );
        // A dot inside the name is not a prefix.
        assert!(
            !Url::from_str("file:///project/a.b.json")
                .unwrap()
                .is_private()
        );
    }

    #[test]
    fn a_listing_excludes_private_entries_by_default() {
        let path = root("listing");
        std::fs::create_dir_all(path.join(".git")).unwrap();
        std::fs::create_dir_all(path.join("data")).unwrap();
        std::fs::write(path.join(".env"), b"SECRET=1").unwrap();
        std::fs::write(path.join("trades.arrows"), b"x").unwrap();
        std::fs::write(path.join(".git").join("HEAD"), b"ref").unwrap();

        let folder = Folder::new(&path).unwrap();

        let public = folder.ls(false, false).unwrap();
        assert_eq!(public.len(), 2, "{public:?}");

        let everything = folder.ls(false, true).unwrap();
        assert_eq!(everything.len(), 4, "{everything:?}");

        // A private directory is not descended into either.
        let deep_public = folder.ls(true, false).unwrap();
        assert_eq!(deep_public.len(), 2, "{deep_public:?}");
        let deep_all = folder.ls(true, true).unwrap();
        assert!(deep_all.len() >= 5, "{deep_all:?}");

        Folder::new(&path)
            .expect("a local container")
            .remove(true)
            .expect("a removable tree");
    }
}

/// A pattern is a location, so listing one expands it.
mod globbing {
    use crate::io::IOBase;
    use crate::local::{Folder, Path};
    use crate::{IOKind, Url};

    /// Build a small lake: two years, two months each, one part per month.
    fn lake(label: &str) -> std::path::PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!("yggdryl-glob-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for year in ["2024", "2025"] {
            for month in ["01", "02"] {
                let leaf = root
                    .join(format!("year={year}"))
                    .join(format!("month={month}"));
                std::fs::create_dir_all(&leaf).unwrap();
                std::fs::write(leaf.join("part-0.parquet"), b"parquet").unwrap();
                std::fs::write(leaf.join("notes.txt"), b"notes").unwrap();
            }
        }
        std::fs::create_dir_all(root.join(".staging")).unwrap();
        std::fs::write(root.join(".staging").join("part-0.parquet"), b"draft").unwrap();
        root
    }

    fn names(entries: &[crate::generic::Holder]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| entry.url().map(ToString::to_string))
            .collect()
    }

    #[test]
    fn a_pattern_selects_the_leaves_it_names() {
        let root = lake("select");
        let folder = Folder::new(&root).unwrap();

        let parts = folder.glob("**/*.parquet", false).unwrap();
        assert_eq!(parts.len(), 4, "{:?}", names(&parts));
        assert!(names(&parts).iter().all(|url| url.ends_with(".parquet")));

        // One plain segment stays at one level, where there are no leaves.
        assert!(folder.glob("*.parquet", false).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fixed_prefix_is_descended_rather_than_filtered() {
        let root = lake("prefix");
        let folder = Folder::new(&root).unwrap();

        let selected = folder.glob("year=2024/**/*.parquet", false).unwrap();
        assert_eq!(selected.len(), 2, "{:?}", names(&selected));
        assert!(names(&selected).iter().all(|url| url.contains("year=2024")));

        // A prefix that is not there yields nothing rather than failing.
        assert!(
            folder
                .glob("year=1999/**/*.parquet", false)
                .unwrap()
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pattern_without_wildcards_names_one_existing_location() {
        let root = lake("exact");
        let folder = Folder::new(&root).unwrap();

        let found = folder
            .glob("year=2025/month=02/part-0.parquet", false)
            .unwrap();
        assert_eq!(found.len(), 1, "{:?}", names(&found));
        assert!(
            folder
                .glob("year=2025/month=02/absent.parquet", false)
                .unwrap()
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_glob_listing_honours_the_privacy_filter() {
        let root = lake("privacy");
        let folder = Folder::new(&root).unwrap();

        let public = folder.glob("**/*.parquet", false).unwrap();
        assert_eq!(public.len(), 4, "{:?}", names(&public));

        let everything = folder.glob("**/*.parquet", true).unwrap();
        assert_eq!(everything.len(), 5, "{:?}", names(&everything));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pattern_location_lists_as_the_children_it_selects() {
        let root = lake("location");
        let url = Url::from_path(&root)
            .unwrap()
            .joinpath("**")
            .unwrap()
            .joinpath("*.parquet")
            .unwrap();

        // The pattern is folder-like before anything touches the file system.
        let path = Path::from_url(url.clone()).unwrap();
        assert_eq!(path.kind(), IOKind::Directory);
        assert!(path.is_container());

        let listed = path.ls(false, false).unwrap();
        assert_eq!(listed.len(), 4, "{:?}", names(&listed));

        // The same holds for a folder handle built straight on the pattern.
        let folder = Folder::from_url(url).unwrap();
        assert_eq!(folder.ls(true, false).unwrap().len(), 4);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn partition_filters_select_the_leaves_that_carry_them() {
        let root = lake("partitions");
        let folder = Folder::new(&root).unwrap();

        let year: Vec<_> = folder
            .children_where(&[("year", "2024")], false)
            .unwrap()
            .collect();
        assert_eq!(year.len(), 4, "{:?}", names(&year));

        let one: Vec<_> = folder
            .children_where(&[("year", "2024"), ("month", "01")], false)
            .unwrap()
            .collect();
        assert_eq!(one.len(), 2, "{:?}", names(&one));
        assert!(one.iter().all(|entry| !entry.is_container()));
        assert_eq!(
            one[0].partitions(),
            vec![
                ("year".to_owned(), "2024".to_owned()),
                ("month".to_owned(), "01".to_owned()),
            ]
        );

        // A filter nothing carries selects nothing.
        assert_eq!(
            folder
                .children_where(&[("year", "1999")], false)
                .unwrap()
                .count(),
            0
        );
        // No filter is every leaf.
        assert_eq!(folder.children_where(&[], false).unwrap().count(), 8);

        let _ = std::fs::remove_dir_all(&root);
    }
}
