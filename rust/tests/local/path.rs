//! `rust/src/local/path.rs`: one local location, whatever it turns out to be.

mod local {
    /// One generic location resolves to the implementation it turns out to need.
    mod generic_path {
        use yggdryl::IOBase;
        use yggdryl::holder::Holder;
        use yggdryl::local::{LocalFolder, LocalPath};
        use yggdryl::{IOKind, MediaType, MimeType, Url};

        fn root(label: &str) -> std::path::PathBuf {
            let mut path = LocalFolder::temporary().unwrap().path().unwrap();
            path.push(format!("yggdryl-path-{label}-{}", std::process::id()));
            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
            path
        }

        #[test]
        fn a_location_reports_what_it_actually_is() {
            let path = root("kind");
            std::fs::create_dir_all(&path).unwrap();

            let directory = LocalPath::new(&path).unwrap();
            assert_eq!(directory.kind(), IOKind::Directory);
            assert!(directory.is_container());
            assert_eq!(directory.media_type().base(), &MimeType::DIRECTORY);

            // A location that does not exist has not decided what it is.
            let missing = LocalPath::new(path.join("absent.arrows")).unwrap();
            assert_eq!(missing.kind(), IOKind::Unknown);
            assert!(!missing.is_container());
            assert!(missing.read_all_bytes().unwrap().is_empty());
            assert_eq!(missing.size(), 0);

            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }

        #[test]
        fn holder_local_retains_the_unresolved_path_role() {
            let path = root("holder-local");
            std::fs::create_dir_all(&path).unwrap();
            let existing = Holder::local(&path).unwrap();
            let missing = Holder::local(path.join("absent.arrows")).unwrap();

            assert!(matches!(&existing, Holder::LocalPath(_)));
            assert!(matches!(&missing, Holder::LocalPath(_)));
            assert_eq!(missing.media_type().base(), &MimeType::ARROW_STREAM);
            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_write_decides_an_undecided_location() {
            let path = root("write");
            std::fs::create_dir_all(&path).unwrap();

            let mut leaf = LocalPath::new(path.join("trades.bin")).unwrap();
            assert_eq!(leaf.kind(), IOKind::Unknown);

            leaf.write_all_bytes(b"AAPL").unwrap();
            leaf.flush().unwrap();

            // Writing created a file, and reading it goes through that file.
            assert_eq!(leaf.kind(), IOKind::File);
            assert_eq!(leaf.read_all_bytes().unwrap(), b"AAPL");

            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }

        #[test]
        fn a_generic_leaf_keeps_media_inference_and_declared_overrides() {
            let path = root("media-type");
            let mut leaf = LocalPath::new(path.with_extension("arrows")).unwrap();

            assert_eq!(leaf.media_type().base(), &MimeType::ARROW_STREAM);
            leaf.set_media_type(MediaType::from(MimeType::CSV));
            assert_eq!(leaf.media_type().base(), &MimeType::CSV);
            assert!(leaf.is_tabular());
            assert!(!leaf.is_atomic());
            assert_eq!(leaf.as_file().unwrap().media_type().base(), &MimeType::CSV);
        }

        #[test]
        fn clearing_a_generic_leaf_discards_its_retained_mapping() {
            let path = root("clear-retained");
            std::fs::create_dir_all(&path).unwrap();
            let file = path.join("staged.bin");
            let mut leaf = LocalPath::new(&file).unwrap();

            leaf.pwrite(0, b"must-not-return").unwrap();
            leaf.clear().unwrap();
            leaf.close().unwrap();

            assert_eq!(std::fs::read(&file).unwrap(), b"");
            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_directory_location_lists_and_a_leaf_location_does_not() {
            let path = root("hierarchy");
            std::fs::create_dir_all(path.join("nested")).unwrap();
            std::fs::write(path.join("a.bin"), b"a").unwrap();

            let directory = LocalPath::new(&path).unwrap();
            assert_eq!(
                directory
                    .ls(false, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                directory
                    .ls(true, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .len(),
                2
            );

            let leaf = LocalPath::new(path.join("a.bin")).unwrap();
            assert_eq!(leaf.kind(), IOKind::File);
            assert!(
                leaf.ls(true, false)
                    .collect::<yggdryl::Result<Vec<_>>>()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(leaf.read_all_bytes().unwrap(), b"a");

            // Children resolve as further generic locations.
            let child = directory.child_by_path("a.bin").unwrap();
            assert!(matches!(&child, Holder::LocalPath(_)));
            assert_eq!(child.read_all_bytes().unwrap(), b"a");
            let parent = child.parent().unwrap();
            assert!(matches!(&parent, Holder::LocalPath(_)));
            assert_eq!(parent.kind(), IOKind::Directory);

            let message = leaf
                .child_by_path("deeper")
                .expect_err("a file cannot resolve a child")
                .to_string();
            assert!(message.contains("expected a container"), "{message}");

            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }

        /// A tree of leaves under `path`: two logs at the top - the second
        /// without a final newline - a note, a folder holding a log, and a
        /// private log.
        fn logs(path: &std::path::Path) {
            std::fs::create_dir_all(path.join("sub")).unwrap();
            std::fs::write(path.join("a.log"), b"a1\na2\n").unwrap();
            std::fs::write(path.join("b.log"), b"b1\nb2").unwrap();
            std::fs::write(path.join("notes.txt"), b"n1\n").unwrap();
            std::fs::write(path.join("sub").join("c.log"), b"c1\n").unwrap();
            std::fs::write(path.join(".hidden.log"), b"private\n").unwrap();
        }

        /// What a location streams from `position`, four bytes a chunk.
        fn streamed(location: &LocalPath, position: u64) -> Vec<u8> {
            let chunks = location
                .pstream_bytes(position, 4)
                .unwrap()
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap();
            assert!(chunks.iter().all(|chunk| !chunk.is_empty()), "{chunks:?}");
            chunks.concat()
        }

        /// A location that resolves to a container answers its leaves' stream
        /// through every byte read, and holds no positional bytes of its own.
        fn streams_as_container(location: &LocalPath, content: &[u8]) {
            assert!(location.is_container());
            assert_eq!(streamed(location, 0), content);
            assert_eq!(streamed(location, 3), &content[3..]);
            assert_eq!(location.read_all_bytes().unwrap(), content);
            assert_eq!(location.read_range_bytes(2, 7).unwrap(), &content[2..9]);
            assert_eq!(location.read_range_bytes(9, 64).unwrap(), &content[9..]);
            assert!(location.read_range_bytes(64, 4).unwrap().is_empty());
            assert_eq!(location.pread(0, &mut [0_u8; 8]).unwrap(), 0);
            assert_eq!(location.size(), 0);
            assert!(!location.is_atomic());
        }

        #[test]
        fn a_directory_location_streams_every_leaf_beneath_it() {
            let path = root("stream-directory");
            logs(&path);

            // Sorted per directory and walked depth first, nothing between two
            // leaves - `b2` runs into `n1` - and the private name left out.
            let directory = LocalPath::new(&path).unwrap();
            assert_eq!(directory.kind(), IOKind::Directory);
            streams_as_container(&directory, b"a1\na2\nb1\nb2n1\nc1\n");

            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_location_ending_in_a_slash_streams_the_leaves_beneath_it() {
            let path = root("stream-slash");
            logs(&path);

            let spelled = LocalPath::new(format!("{}/", path.display())).unwrap();
            streams_as_container(&spelled, b"a1\na2\nb1\nb2n1\nc1\n");

            // Absent, the same spelling is a container holding nothing, and
            // streaming it creates nothing.
            let absent = path.join("absent");
            let missing = LocalPath::new(format!("{}/", absent.display())).unwrap();
            assert!(missing.is_container());
            assert!(streamed(&missing, 0).is_empty());
            assert!(missing.read_all_bytes().unwrap().is_empty());
            assert!(missing.read_range_bytes(0, 4).unwrap().is_empty());
            assert!(!absent.exists());

            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_one_level_pattern_streams_only_the_leaves_it_matches() {
            let path = root("stream-glob");
            logs(&path);

            // `*` stays in one name: the nested log and the note are not
            // matched, and the private log is matched and left out.
            let url = Url::from_path(&path).unwrap().joinpath("*.log").unwrap();
            let pattern = LocalPath::from_url(url).unwrap();
            streams_as_container(&pattern, b"a1\na2\nb1\nb2");

            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_recursive_pattern_streams_every_leaf_it_matches() {
            let path = root("stream-deep-glob");
            logs(&path);

            // `**` spans any number of levels, none included.
            let url = Url::from_path(&path)
                .unwrap()
                .joinpath("**")
                .unwrap()
                .joinpath("*.log")
                .unwrap();
            let pattern = LocalPath::from_url(url).unwrap();
            streams_as_container(&pattern, b"a1\na2\nb1\nb2c1\n");

            // A fixed prefix that is not there matches nothing.
            let url = Url::from_path(path.join("absent"))
                .unwrap()
                .joinpath("**")
                .unwrap()
                .joinpath("*.log")
                .unwrap();
            let nowhere = LocalPath::from_url(url).unwrap();
            assert!(streamed(&nowhere, 0).is_empty());
            assert!(nowhere.read_all_bytes().unwrap().is_empty());

            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn an_absent_location_streams_nothing() {
            let path = root("stream-absent");
            std::fs::create_dir_all(&path).unwrap();

            let missing = LocalPath::new(path.join("absent.log")).unwrap();
            assert_eq!(missing.kind(), IOKind::Unknown);
            assert!(streamed(&missing, 0).is_empty());
            assert!(missing.read_range_bytes(0, 4).unwrap().is_empty());
            assert!(!path.join("absent.log").exists());

            LocalFolder::new(&path).unwrap().remove(true).unwrap();
        }

        #[test]
        fn a_location_can_be_addressed_as_a_directory_before_it_exists() {
            let path = root("as-directory");
            let location = LocalPath::new(&path).unwrap();
            assert_eq!(location.kind(), IOKind::Unknown);

            // Truncating to zero is the write that brings a directory into being.
            location.as_directory().unwrap().create().unwrap();
            assert_eq!(location.kind(), IOKind::Directory);

            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
        }
    }
}
