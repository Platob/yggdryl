//! `rust/src/local/file.rs`: the memory-mapped local file - what it creates,
//! what it resizes, and what it publishes.

mod local {
    mod mapped {
        use yggdryl::IOBase;
        use yggdryl::local::{LocalFile, LocalFolder};

        fn path(label: &str) -> std::path::PathBuf {
            let mut path = LocalFolder::temporary().unwrap().path().unwrap();
            path.push(format!("yggdryl-mmap-{label}-{}.bin", std::process::id()));
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
            path
        }

        #[test]
        fn a_mapped_file_round_trips_and_resizes() {
            let path = path("roundtrip");
            {
                let mut mapped = LocalFile::create(&path).unwrap();
                assert!(mapped.is_empty());

                mapped.pwrite(0, b"trade").unwrap();
                assert_eq!(mapped.size(), 5);

                // Growing past the mapping remaps rather than failing.
                let large = vec![7_u8; 256 * 1024];
                mapped.append_bytes(&large).unwrap();
                assert_eq!(mapped.size(), 5 + large.len() as u64);
                assert!(mapped.capacity() >= mapped.size());
                mapped.flush().unwrap();
            }

            // The on-disk length is the logical size, not the mapping capacity.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 5 + 256 * 1024);

            // Reopening sees the same bytes.
            let reopened = LocalFile::new(&path).unwrap();
            assert_eq!(reopened.size(), 5 + 256 * 1024);
            assert_eq!(reopened.read_range_bytes(0, 5).unwrap(), b"trade");
            assert_eq!(reopened.url().unwrap().extension(), Some("bin"));

            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }

        #[test]
        fn a_handle_for_a_missing_file_touches_nothing() {
            let path = path("lazy");
            assert!(!path.exists());

            let handle = LocalFile::new(&path).unwrap();
            // Constructing the handle must not create the file.
            assert!(!path.exists());
            assert!(!handle.exists());

            // A read of a missing file is empty, not an error.
            assert_eq!(handle.size(), 0);
            assert!(handle.is_empty());
            let mut probe = [0_u8; 8];
            assert_eq!(handle.pread(0, &mut probe).unwrap(), 0);
            assert!(handle.read_all_bytes().unwrap().is_empty());
            // Reading still did not create it.
            assert!(!path.exists());

            // The media type comes from the name, which exists even when the file
            // does not.
            assert_eq!(handle.url().unwrap().extension(), Some("bin"));
        }

        #[test]
        fn the_first_write_creates_the_file_and_its_parent() {
            let mut path = LocalFolder::temporary().unwrap().path().unwrap();
            path.push(format!("yggdryl-mmap-create-{}", std::process::id()));
            LocalFolder::new(&path)
                .expect("a local container")
                .remove(true)
                .expect("a removable tree");
            path.push("nested");
            path.push("trades.bin");

            let mut handle = LocalFile::new(&path).unwrap();
            assert!(!path.exists());

            handle.pwrite(0, b"created").unwrap();
            handle.flush().unwrap();

            assert!(path.exists());
            assert_eq!(handle.read_all_bytes().unwrap(), b"created");

            let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
        }

        #[test]
        fn a_write_into_a_missing_ancestry_creates_it_from_the_write_itself() {
            // No `mkdir` step runs first: the open fails with the typed absence,
            // the ancestry is repaired once, and the same open is retried once.
            let mut root = LocalFolder::temporary().unwrap().path().unwrap();
            root.push(format!("yggdryl-ancestry-{}", std::process::id()));
            yggdryl::local::LocalFolder::new(&root)
                .expect("a local folder")
                .remove(true)
                .expect("a removable folder");

            let deep = root.join("a").join("b").join("c").join("trades.bin");
            let mut leaf = LocalFile::new(&deep).expect("a local leaf");
            leaf.write_all_bytes(b"rows").expect("a created ancestry");

            assert_eq!(
                LocalFile::new(&deep)
                    .expect("a local leaf")
                    .read_all_bytes()
                    .expect("the written bytes"),
                b"rows"
            );

            yggdryl::local::LocalFolder::new(&root)
                .expect("a local folder")
                .remove(true)
                .expect("a removable folder");
        }

        #[test]
        fn a_read_of_a_missing_file_is_empty_rather_than_an_absence() {
            // The open *is* the existence question; nothing probes before it, and
            // a read that finds nothing is emptiness rather than a failure.
            let path = path("absent-read");
            let leaf = LocalFile::new(&path).expect("a local leaf");
            assert_eq!(leaf.size(), 0);
            assert!(leaf.read_all_bytes().expect("an empty read").is_empty());
        }

        #[test]
        fn a_complete_write_publishes_its_length_to_another_handle() {
            let path = path("published");

            let mut writer = LocalFile::new(&path).unwrap();
            writer.write_all_bytes(b"one\ntwo\n").unwrap();

            // The geometric growth is this handle's working state, not the value:
            // a second handle - or another process - must see the logical length,
            // or it would read the mapping's zero padding as content.
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 8);
            let second = LocalFile::new(&path).unwrap();
            assert_eq!(second.size(), 8);
            assert_eq!(second.read_all_bytes().unwrap(), b"one\ntwo\n");

            drop(writer);
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }

        #[cfg(unix)]
        #[test]
        fn closing_a_read_only_mapping_does_not_restore_replaced_bytes() {
            let path = path("read-only-close");
            std::fs::write(&path, b"before").unwrap();

            let mut reader = LocalFile::new(&path).unwrap();
            reader.open().unwrap();
            assert_eq!(reader.read_all_bytes().unwrap(), b"before");

            std::fs::write(&path, b"after!").unwrap();
            reader.close().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"after!");

            LocalFile::new(&path).unwrap().remove(false).unwrap();
        }

        #[test]
        fn a_mapped_file_zero_fills_a_write_gap() {
            let path = path("gap");
            let mut mapped = LocalFile::create(&path).unwrap();
            mapped.pwrite(0, b"ab").unwrap();
            mapped.pwrite(5, b"z").unwrap();

            assert_eq!(mapped.size(), 6);
            assert_eq!(mapped.read_all_bytes().unwrap(), b"ab\0\0\0z");

            drop(mapped);
            // Teardown through the abstraction: absence is a no-op success.
            LocalFile::new(&path)
                .expect("a local leaf")
                .remove(false)
                .expect("a removable leaf");
        }
    }
}
