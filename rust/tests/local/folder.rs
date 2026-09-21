//! `rust/src/local/folder.rs`: a local directory as a container - its children,
//! its listings, its globs, and the well-known roots the environment names.

mod local {
    mod hierarchy {
        use yggdryl::IOBase;
        use yggdryl::holder::Holder;
        use yggdryl::local::Folder;

        fn root(label: &str) -> std::path::PathBuf {
            let mut path = Folder::temporary().unwrap().path().unwrap();
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
            assert!(
                directory
                    .ls(false, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .is_empty()
            );
            assert!(
                directory
                    .ls(true, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .is_empty()
            );
            assert!(!path.exists());
        }

        #[test]
        fn children_resolve_to_directories_and_mapped_leaves() {
            let path = root("children");
            let directory = Folder::new(&path).unwrap();
            directory.create().unwrap();

            // A write through a child creates the leaf.
            let mut leaf = directory.child_by_path("trades.arrows").unwrap();
            leaf.pwrite(0, b"payload").unwrap();
            leaf.flush().unwrap();
            assert!(matches!(leaf, Holder::File(_)));
            assert!(!leaf.is_container());

            // A nested child creates its parent directory on write.
            let mut nested = directory.child_by_path("sub/inner.bin").unwrap();
            nested.pwrite(0, b"deep").unwrap();
            nested.flush().unwrap();

            let listed = directory
                .ls(false, false)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(listed.len(), 2, "{listed:?}");
            assert!(listed.iter().any(Holder::is_container));
            assert!(listed.iter().any(|entry| !entry.is_container()));

            // Recursion reaches the nested leaf.
            let deep = directory
                .ls(true, false)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
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

            let mut leaf = directory.child_by_path("leaf.bin").unwrap();
            leaf.pwrite(0, b"x").unwrap();
            leaf.flush().unwrap();

            // A leaf's parent is the directory holding it.
            let parent = leaf.parent().expect("a mapped file has a parent");
            assert!(parent.is_container());
            assert_eq!(parent.url().unwrap(), directory.url());

            // A buffer has no location, so it has no parent.
            assert!(yggdryl::holder::Buffer::new().parent().is_none());
        }

        #[test]
        fn a_relative_child_resolves_dot_segments() {
            let path = root("relative");
            let directory = Folder::new(&path).unwrap();
            let sideways = directory.child_by_path("sub/../beside.bin").unwrap();

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
            assert!(directory.read_all_bytes().unwrap().is_empty());

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

            let mut leaf = directory.child_by_path("cached.bin").unwrap();
            assert!(!leaf.opened());

            leaf.pwrite(0, b"cached").unwrap();
            assert!(leaf.opened());

            // Closing publishes and releases; the handle stays usable.
            leaf.close().unwrap();
            assert!(!leaf.opened());
            assert_eq!(leaf.read_all_bytes().unwrap(), b"cached");

            // Opening a handle for a missing file caches nothing and creates nothing.
            let mut absent = directory.child_by_path("absent.bin").unwrap();
            absent.open().unwrap();
            assert!(!absent.opened());

            Folder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }
    }

    /// A listing skips private names unless a caller asks for them.
    mod privacy {
        use yggdryl::holder::Holder;
        use yggdryl::local::{Folder, Path};
        use yggdryl::{IOBase, IOKind, MimeType, Url};

        fn root(label: &str) -> std::path::PathBuf {
            let mut path = Folder::temporary().unwrap().path().unwrap();
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
            // A container spelled with its slash is judged by the same segment.
            assert!(Url::from_str("file:///project/.git/").unwrap().is_private());
        }

        #[test]
        fn a_trailing_slash_names_a_folder_before_anything_is_looked_up() {
            let path = root("trailing");
            let absent = path.join("not-yet");
            // Nothing exists, so a plain name is undecided...
            assert_eq!(Path::new(&absent).unwrap().kind(), IOKind::Unknown);
            // ...while the same name with a slash is a container, with no probe.
            let spelled = Path::new(format!("{}/", absent.display())).unwrap();
            assert_eq!(spelled.kind(), IOKind::Directory);
            assert!(spelled.is_container());
            assert!(!spelled.is_atomic());
            assert_eq!(spelled.media_type().base(), &MimeType::DIRECTORY);
            assert!(!absent.exists(), "asking created nothing");

            // A child resolved with a slash is a folder handle outright.
            let folder = Folder::new(&path).unwrap();
            assert!(matches!(
                folder.child_by_path("sub/").unwrap(),
                Holder::Folder(_)
            ));
            assert!(matches!(
                folder.child_by_path("sub").unwrap(),
                Holder::File(_)
            ));
            assert!(!path.join("sub").exists(), "resolving created nothing");

            // Truncating the spelled container to zero brings the directory into
            // being, exactly as it does for an explicit folder.
            let mut spelled = spelled;
            spelled.truncate(0).unwrap();
            assert!(absent.is_dir());
            Folder::new(&path).unwrap().remove(true).unwrap();
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

            let public = folder
                .ls(false, false)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(public.len(), 2, "{public:?}");

            let everything = folder
                .ls(false, true)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(everything.len(), 4, "{everything:?}");

            // A private directory is not descended into either.
            let deep_public = folder
                .ls(true, false)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(deep_public.len(), 2, "{deep_public:?}");
            let deep_all = folder
                .ls(true, true)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert!(deep_all.len() >= 5, "{deep_all:?}");

            Folder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }
    }

    /// A pattern is a location, so listing one expands it.
    mod globbing {
        use yggdryl::IOBase;
        use yggdryl::local::{Folder, Path};
        use yggdryl::{IOKind, Url};

        /// Build a small lake: two years, two months each, one part per month.
        fn lake(label: &str) -> std::path::PathBuf {
            let mut root = Folder::temporary().unwrap().path().unwrap();
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

        fn names(entries: &[yggdryl::holder::Holder]) -> Vec<String> {
            entries
                .iter()
                .filter_map(|entry| entry.url().map(ToString::to_string))
                .collect()
        }

        #[test]
        fn a_pattern_selects_the_leaves_it_names() {
            let root = lake("select");
            let folder = Folder::new(&root).unwrap();

            let parts = folder
                .glob("**/*.parquet", false)
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(parts.len(), 4, "{:?}", names(&parts));
            assert!(names(&parts).iter().all(|url| url.ends_with(".parquet")));

            // One plain segment stays at one level, where there are no leaves.
            assert_eq!(folder.glob("*.parquet", false).unwrap().count(), 0);

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn a_fixed_prefix_is_descended_rather_than_filtered() {
            let root = lake("prefix");
            let folder = Folder::new(&root).unwrap();

            let selected = folder
                .glob("year=2024/**/*.parquet", false)
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(selected.len(), 2, "{:?}", names(&selected));
            assert!(names(&selected).iter().all(|url| url.contains("year=2024")));

            // A prefix that is not there yields nothing rather than failing.
            assert!(
                folder
                    .glob("year=1999/**/*.parquet", false)
                    .unwrap()
                    .collect::<yggdryl::Result<Vec<_>>>()
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
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(found.len(), 1, "{:?}", names(&found));
            assert!(
                folder
                    .glob("year=2025/month=02/absent.parquet", false)
                    .unwrap()
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .is_empty()
            );

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn a_glob_listing_honours_the_privacy_filter() {
            let root = lake("privacy");
            let folder = Folder::new(&root).unwrap();

            let public = folder
                .glob("**/*.parquet", false)
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(public.len(), 4, "{:?}", names(&public));

            let everything = folder
                .glob("**/*.parquet", true)
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
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

            let listed = path
                .ls(false, false)
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(listed.len(), 4, "{:?}", names(&listed));

            // The same holds for a folder handle built straight on the pattern.
            let folder = Folder::from_url(url).unwrap();
            assert_eq!(
                folder
                    .ls(true, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .len(),
                4
            );

            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn partition_filters_select_the_leaves_that_carry_them() {
            let root = lake("partitions");
            let folder = Folder::new(&root).unwrap();

            let year: Vec<_> = folder
                .children_where(&[("year", "2024")], false)
                .unwrap()
                .collect::<yggdryl::Result<_>>()
                .unwrap();
            assert_eq!(year.len(), 4, "{:?}", names(&year));

            let one: Vec<_> = folder
                .children_where(&[("year", "2024"), ("month", "01")], false)
                .unwrap()
                .collect::<yggdryl::Result<_>>()
                .unwrap();
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
}

mod roots {
    use std::ffi::OsString;

    use yggdryl::IOBase;
    use yggdryl::local::Folder;

    #[test]
    fn home_and_config_follow_the_environment_and_create_nothing() {
        let original: (Option<OsString>, Option<OsString>) =
            (std::env::var_os("HOME"), std::env::var_os("USERPROFILE"));

        // A fresh directory of this test's own, never the developer's real home.
        let home = Folder::temporary()
            .expect("the temporary directory")
            .path()
            .expect("a platform path")
            .join(format!("yggdryl-local-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("a fresh home");

        // SAFETY: `set_var` is `unsafe` because another thread reading the
        // environment concurrently is a data race. This binary holds only this
        // test, so it runs on one thread, and the test spawns none.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
        }

        let resolved = Folder::home().expect("a home from the environment");
        assert_eq!(resolved.path().expect("a platform path"), home);

        let config = Folder::config().expect("a configuration directory");
        assert_eq!(
            config.path().expect("a platform path"),
            home.join(".config")
        );

        // Neither call created anything: the handle is the whole effect.
        assert!(!config.exists());
        assert!(!home.join(".config").exists());
        assert_eq!(
            Folder::new(&home)
                .expect("a local folder")
                .ls(false, true)
                .count(),
            0
        );

        // SAFETY: the same reasoning as above.
        unsafe {
            std::env::remove_var("HOME");
            std::env::remove_var("USERPROFILE");
        }

        let error = Folder::home().expect_err("no home without either variable");
        assert!(error.is_absent());
        let message = error.to_string();
        assert!(message.contains("HOME"), "{message}");
        assert!(message.contains("USERPROFILE"), "{message}");
        assert!(
            Folder::config()
                .expect_err("no config without a home")
                .is_absent()
        );

        // SAFETY: the same reasoning as above.
        unsafe {
            match original.0 {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match original.1 {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
