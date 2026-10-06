//! `rust/src/iobase.rs`: the one byte-storage trait - what every backend must
//! answer, and what a positional read and write promise.

mod backends {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    use std::sync::Arc;

    /// Every backend under test, each as a freshly built empty handle.
    ///
    /// The handles are boxed because the battery is one function rather than
    /// one per backend; `IOBase` is implemented for the box, so the byte half
    /// of the contract forwards unchanged.
    fn backends(label: &str) -> Vec<(&'static str, Box<dyn IOBase>)> {
        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!(
            "yggdryl-conformance-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a writable temporary root");

        let memory = Arc::new(yggdryl::fs::MemoryFileSystem::new());
        yggdryl::fs::FileSystem::create_dir(memory.as_ref(), "bench", false)
            .expect("a writable memory root");
        vec![
            ("buffer", Box::new(Buffer::new()) as Box<dyn IOBase>),
            (
                "local::LocalFile",
                Box::new(
                    yggdryl::local::LocalFile::create(root.join(format!("{label}.bin")))
                        .expect("a valid path"),
                ),
            ),
            (
                "fs::FsFile",
                Box::new(
                    yggdryl::fs::FsFile::from_path(memory, format!("bench/{label}.bin"), None)
                        .expect("a valid location"),
                ),
            ),
            (
                "buffered",
                Box::new(
                    // Pages far smaller than the default, so even these short
                    // fixtures cross page boundaries and exercise the cache
                    // rather than living inside one page.
                    Buffer::new().buffered(
                        yggdryl::holder::buffered::BufferedOptions::default().with_page_size(4),
                    ),
                ),
            ),
        ]
    }

    /// Backends that provide arbitrary positional mutation rather than only the
    /// sequential Arrow output and append stream capabilities.
    fn positional_backends(label: &str) -> Vec<(&'static str, Box<dyn IOBase>)> {
        backends(label)
            .into_iter()
            .filter(|(name, _)| *name != "fs::FsFile")
            .collect()
    }

    /// Remove whatever the local backend left behind.
    fn cleanup(label: &str) {
        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!(
            "yggdryl-conformance-{label}-{}",
            std::process::id()
        ));
        // Teardown goes through the abstraction, not around it: a folder
        // handle already addresses this tree, and absence is a no-op success.
        if let Ok(mut folder) = yggdryl::local::LocalFolder::new(&root) {
            folder.remove(true).expect("a removable tree");
        }
    }

    #[test]
    fn every_backend_grows_and_zero_fills_a_write_gap() {
        for (name, mut handle) in positional_backends("gap") {
            handle.pwrite(0, b"trade").expect("a writable handle");
            assert_eq!(handle.size(), 5, "{name}");

            // Writing past the end grows the value and zero-fills the gap.
            handle.pwrite(8, b"!").expect("a writable handle");
            assert_eq!(handle.size(), 9, "{name}");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"trade\0\0\0!",
                "{name}"
            );
        }
        cleanup("gap");
    }

    #[test]
    fn every_backend_reads_positionally_without_a_shared_cursor() {
        for (name, mut handle) in backends("cursor") {
            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");

            // Two reads at different offsets, in either order.
            let mut tail = [0_u8; 3];
            handle.pread(7, &mut tail).expect("a readable handle");
            let mut head = [0_u8; 3];
            handle.pread(0, &mut head).expect("a readable handle");
            assert_eq!(&head, b"012", "{name}");
            assert_eq!(&tail, b"789", "{name}");

            // Entirely past the end is empty; straddling the end is short.
            let mut past = [0_u8; 4];
            assert_eq!(
                handle.pread(100, &mut past).expect("a readable handle"),
                0,
                "{name}"
            );
            assert_eq!(
                handle.pread(8, &mut past).expect("a readable handle"),
                2,
                "{name}"
            );
        }
        cleanup("cursor");
    }

    #[test]
    fn every_backend_reads_a_missing_resource_as_empty() {
        // The laziness contract: absence is emptiness on the read path, so a
        // caller probes a location without an existence check first.
        let memory = Arc::new(yggdryl::fs::MemoryFileSystem::new());
        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!("yggdryl-conformance-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let absent: Vec<(&str, Box<dyn IOBase>)> = vec![
            (
                "local::LocalFile",
                Box::new(
                    yggdryl::local::LocalFile::new(root.join("absent.bin")).expect("a valid path"),
                ),
            ),
            (
                "fs::FsFile",
                Box::new(
                    yggdryl::fs::FsFile::from_path(memory, "nowhere/absent.bin", None)
                        .expect("a valid location"),
                ),
            ),
        ];
        for (name, handle) in absent {
            assert_eq!(handle.size(), 0, "{name}");
            assert!(handle.is_empty(), "{name}");
            let mut probe = [0_u8; 8];
            assert_eq!(
                handle.pread(0, &mut probe).expect("a readable handle"),
                0,
                "{name}"
            );
            assert!(
                handle
                    .read_all_bytes()
                    .expect("a readable handle")
                    .is_empty(),
                "{name}"
            );
        }
        // Reading created nothing.
        assert!(!root.exists());
    }

    #[test]
    fn every_backend_truncates_shrinking_and_extending() {
        for (name, mut handle) in positional_backends("truncate") {
            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");

            handle.truncate(4).expect("a resizable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"0123",
                "{name}"
            );

            // Extending zero-fills rather than leaving stale bytes visible.
            handle.truncate(6).expect("a resizable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"0123\0\0",
                "{name}"
            );

            handle.clear().expect("a clearable handle");
            assert!(handle.is_empty(), "{name}");
        }
        cleanup("truncate");
    }

    #[test]
    fn every_backend_keeps_capacity_at_or_above_size() {
        for (name, mut handle) in positional_backends("capacity") {
            handle.reserve(4_096).expect("a reservable handle");
            assert!(handle.capacity() >= 4_096, "{name}");
            assert_eq!(handle.size(), 0, "{name}");

            handle.pwrite(0, b"x").expect("a writable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");

            // The invariant holds after a shrink too, not only after growth.
            handle
                .write_all_bytes(&vec![7_u8; 8_192])
                .expect("a writable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");
            handle.truncate(16).expect("a resizable handle");
            assert!(handle.capacity() >= handle.size(), "{name}");
        }
        cleanup("capacity");
    }

    #[test]
    fn every_backend_appends_where_it_says_it_did() {
        for (name, mut handle) in backends("append") {
            handle
                .write_all_bytes(b"")
                .expect("an initialized append target");
            assert_eq!(
                handle.append_bytes(b"first").expect("a writable handle"),
                0,
                "{name}"
            );
            assert_eq!(
                handle.append_bytes(b"second").expect("a writable handle"),
                5,
                "{name}"
            );
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"firstsecond",
                "{name}"
            );
        }
        cleanup("append");
    }

    #[test]
    fn arrow_filesystem_handles_reject_unavailable_random_mutation() {
        let filesystem = Arc::new(yggdryl::fs::MemoryFileSystem::new());
        yggdryl::fs::FileSystem::create_dir(filesystem.as_ref(), "bench", false)
            .expect("a writable memory root");
        let mut handle = yggdryl::fs::FsFile::from_path(filesystem, "bench/random.bin", None)
            .expect("a valid location");
        handle
            .write_all_bytes(b"value")
            .expect("a sequential output stream");

        assert!(handle.pwrite(1, b"x").unwrap_err().is_unsupported());
        assert!(handle.truncate(2).unwrap_err().is_unsupported());
        assert!(handle.reserve(16).unwrap_err().is_unsupported());
        assert_eq!(handle.read_all_bytes().unwrap(), b"value");
    }

    #[test]
    fn every_backend_replaces_the_whole_value_and_bounds_a_range_read() {
        for (name, mut handle) in backends("replace") {
            handle
                .write_all_bytes(b"a much longer previous value")
                .expect("a writable handle");
            handle.write_all_bytes(b"short").expect("a writable handle");
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"short",
                "{name}"
            );
            assert_eq!(handle.size(), 5, "{name}");

            handle
                .write_all_bytes(b"0123456789")
                .expect("a writable handle");
            assert_eq!(
                handle.read_range_bytes(2, 3).expect("a readable handle"),
                b"234",
                "{name}"
            );
            // Asking past the end yields what exists rather than failing.
            assert_eq!(
                handle.read_range_bytes(8, 100).expect("a readable handle"),
                b"89",
                "{name}"
            );
            assert!(
                handle
                    .read_range_bytes(50, 4)
                    .expect("a readable handle")
                    .is_empty(),
                "{name}"
            );
        }
        cleanup("replace");
    }

    #[test]
    fn every_backend_names_the_shortfall_of_an_exact_read() {
        for (name, mut handle) in backends("exact") {
            handle.write_all_bytes(b"abc").expect("a writable handle");
            let mut target = [0_u8; 8];
            let message = handle
                .pread_exact(0, &mut target)
                .expect_err("a short value cannot fill the buffer")
                .to_string();
            assert!(message.contains("expected 8 bytes"), "{name}: {message}");
            assert!(message.contains("got 3"), "{name}: {message}");
        }
        cleanup("exact");
    }

    #[test]
    fn every_backend_copies_into_every_other_one() {
        // The transfer is chunked through the trait alone, so a copy works
        // in every direction across backends without either side knowing
        // what the other is.
        for (source_name, mut source) in backends("copy-source") {
            source
                .write_all_bytes(b"symbol,price\nAAPL,1\n")
                .expect("a writable handle");
            for (target_name, mut target) in backends("copy-target") {
                target
                    .write_all_bytes(b"stale contents")
                    .expect("a writable handle");
                let copied = source.copy_into(target.as_mut()).expect("a copyable pair");

                assert_eq!(copied, source.size(), "{source_name} -> {target_name}");
                assert_eq!(
                    target.read_all_bytes().expect("a readable handle"),
                    source.read_all_bytes().expect("a readable handle"),
                    "{source_name} -> {target_name}"
                );
            }
            cleanup("copy-target");
        }
        cleanup("copy-source");
    }

    #[test]
    fn every_backend_streams_through_the_reader_and_writer_adapters() {
        use std::io::{Read, Write};

        for (name, mut handle) in backends("streams") {
            {
                let mut writer = handle.writer_at(0);
                writer.write_all(b"symbol,").expect("a writable adapter");
                writer.write_all(b"price").expect("a writable adapter");
                writer.flush().expect("a flushable adapter");
            }
            assert_eq!(
                handle.read_all_bytes().expect("a readable handle"),
                b"symbol,price",
                "{name}"
            );

            let mut text = String::new();
            handle
                .reader_at(0)
                .read_to_string(&mut text)
                .expect("a readable adapter");
            assert_eq!(text, "symbol,price", "{name}");

            // A reader can start anywhere without disturbing another.
            let mut tail = String::new();
            handle
                .reader_at(7)
                .read_to_string(&mut tail)
                .expect("a readable adapter");
            assert_eq!(tail, "price", "{name}");
        }
        cleanup("streams");
    }

    #[test]
    fn every_backend_round_trips_a_content_coding() {
        for (name, mut handle) in backends("coding") {
            let payload = "symbol,price\n".repeat(500).into_bytes();
            handle.write_all_bytes(&payload).expect("a writable handle");

            for codec in [
                yggdryl::Codec::Gzip,
                yggdryl::Codec::Zlib,
                yggdryl::Codec::Zstd,
            ] {
                let mut compressed = Buffer::new();
                handle
                    .compress_into(&mut compressed, codec)
                    .expect("an encodable value");
                assert!(compressed.size() < handle.size(), "{name}/{codec}");
                assert_eq!(compressed.codec(), codec, "{name}/{codec}");

                let mut restored = Buffer::new();
                compressed
                    .decompress_into(&mut restored)
                    .expect("a decodable value");
                assert_eq!(restored.as_slice(), payload.as_slice(), "{name}/{codec}");
            }
        }
        cleanup("coding");
    }
}

mod positional {
    use std::io::{Read, Write};
    use yggdryl::Codec;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType, Url};

    #[test]
    fn positional_writes_grow_and_zero_fill_the_gap() {
        let mut buffer = Buffer::new();
        assert!(buffer.is_empty());

        buffer.pwrite(0, b"trade").unwrap();
        assert_eq!(buffer.size(), 5);

        // Writing past the end grows the value and zero-fills what was skipped.
        buffer.pwrite(8, b"!").unwrap();
        assert_eq!(buffer.size(), 9);
        assert_eq!(buffer.as_slice(), b"trade\0\0\0!");
    }

    #[test]
    fn positional_reads_do_not_share_a_cursor() {
        let buffer = Buffer::from_bytes(b"0123456789".to_vec());

        // Two independent reads at different offsets, in any order.
        let mut tail = [0_u8; 3];
        buffer.pread(7, &mut tail).unwrap();
        let mut head = [0_u8; 3];
        buffer.pread(0, &mut head).unwrap();

        assert_eq!(&head, b"012");
        assert_eq!(&tail, b"789");

        // A read entirely past the end is empty rather than an error.
        let mut past = [0_u8; 4];
        assert_eq!(buffer.pread(100, &mut past).unwrap(), 0);

        // A read straddling the end is short.
        assert_eq!(buffer.pread(8, &mut past).unwrap(), 2);
    }

    #[test]
    fn exact_reads_name_the_shortfall() {
        let buffer = Buffer::from_bytes(b"abc".to_vec());
        let mut target = [0_u8; 8];
        let message = buffer.pread_exact(0, &mut target).unwrap_err().to_string();
        assert!(message.contains("expected 8 bytes"), "{message}");
        assert!(message.contains("got 3"), "{message}");
    }

    #[test]
    fn truncate_shrinks_and_extends() {
        let mut buffer = Buffer::from_bytes(b"0123456789".to_vec());

        buffer.truncate(4).unwrap();
        assert_eq!(buffer.as_slice(), b"0123");

        // Extending zero-fills rather than leaving stale bytes visible.
        buffer.truncate(6).unwrap();
        assert_eq!(buffer.as_slice(), b"0123\0\0");

        buffer.clear().unwrap();
        assert!(buffer.is_empty());
    }

    #[test]
    fn reserve_grows_capacity_without_changing_size() {
        let mut buffer = Buffer::new();
        buffer.reserve(4_096).unwrap();

        assert!(buffer.capacity() >= 4_096);
        assert_eq!(buffer.size(), 0);
        // Capacity is never below size, for every implementation.
        buffer.pwrite(0, b"x").unwrap();
        assert!(buffer.capacity() >= buffer.size());
    }

    #[test]
    fn append_reports_where_the_bytes_landed() {
        let mut buffer = Buffer::new();
        assert_eq!(buffer.append_bytes(b"first").unwrap(), 0);
        assert_eq!(buffer.append_bytes(b"second").unwrap(), 5);
        assert_eq!(buffer.as_slice(), b"firstsecond");
    }

    #[test]
    fn a_declared_media_type_overrides_inference() {
        let url = Url::from_str("file:///trades.json.gz").unwrap();
        let buffer = Buffer::new().with_media_type(url.media_type());

        assert_eq!(buffer.media_type().base(), &MimeType::JSON);
        assert_eq!(buffer.codec(), Codec::Gzip);

        // Setting one later replaces whatever was inferred.
        let mut plain = Buffer::from_bytes(b"{\"a\":1}".to_vec());
        assert_eq!(plain.media_type().base(), &MimeType::JSON);
        plain.set_media_type(MediaType::from(MimeType::CSV));
        assert_eq!(plain.media_type().base(), &MimeType::CSV);
    }

    #[test]
    fn an_undeclared_media_type_is_inferred_from_content() {
        // A buffer has no filename, so its representation comes from its bytes.
        let json = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
        assert_eq!(json.media_type().base(), &MimeType::JSON);

        let parquet = Buffer::from_bytes(b"PAR1payload".to_vec());
        assert_eq!(parquet.media_type().base(), &MimeType::PARQUET);

        // Opaque bytes stay opaque rather than guessing.
        let opaque = Buffer::from_bytes(vec![0xAB, 0xCD, 0xEF]);
        assert_eq!(opaque.media_type().base(), &MimeType::OCTET_STREAM);
        assert!(Buffer::new().media_type().base() == &MimeType::OCTET_STREAM);
    }

    #[test]
    fn inference_is_redone_after_the_bytes_change() {
        let mut buffer = Buffer::from_bytes(br#"{"a":1}"#.to_vec());
        assert_eq!(buffer.media_type().base(), &MimeType::JSON);

        // Replacing the content replaces the inferred representation.
        buffer.write_all_bytes(b"PAR1payload").unwrap();
        assert_eq!(buffer.media_type().base(), &MimeType::PARQUET);

        buffer.clear().unwrap();
        assert_eq!(buffer.media_type().base(), &MimeType::OCTET_STREAM);
    }

    #[test]
    fn copy_into_moves_bytes_and_media_type() {
        let source = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
            .with_media_type(Url::from_str("file:///trades.csv").unwrap().media_type());
        let mut target = Buffer::from_bytes(b"stale contents".to_vec());

        let copied = source.copy_into(&mut target).unwrap();

        assert_eq!(copied, source.size());
        assert_eq!(target.as_slice(), source.as_slice());
        assert_eq!(target.media_type().base(), &MimeType::CSV);
    }

    #[test]
    fn move_into_moves_the_bytes_and_leaves_no_source() {
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-transfer-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // Local to local: neither side is bound, so the move is the copy then
        // the removal, and the source is gone once the target holds the value.
        let mut source = LocalFile::new(root.join("source.csv")).unwrap();
        source.write_all_bytes(b"symbol,price\nAAPL,1\n").unwrap();
        let mut target = LocalFile::new(root.join("target.csv")).unwrap();
        target.write_all_bytes(b"stale contents").unwrap();
        let moved = source.move_into(&mut target).unwrap();
        assert_eq!(moved, 20);
        assert_eq!(target.read_all_bytes().unwrap(), b"symbol,price\nAAPL,1\n");
        assert!(!source.exists());
        assert!(!root.join("source.csv").exists());

        // Local to buffer: the same rule whatever the pair, the media type
        // travelling with the bytes.
        let mut buffer = Buffer::from_bytes(b"stale".to_vec());
        let moved = target.move_into(&mut buffer).unwrap();
        assert_eq!(moved, 20);
        assert_eq!(buffer.as_slice(), b"symbol,price\nAAPL,1\n");
        assert_eq!(buffer.media_type().base(), &MimeType::CSV);
        assert!(!target.exists());
        assert!(!root.join("target.csv").exists());

        // And back: a buffer moves into a local file and holds nothing after.
        let mut landed = LocalFile::new(root.join("landed.csv")).unwrap();
        let moved = buffer.move_into(&mut landed).unwrap();
        assert_eq!(moved, 20);
        assert_eq!(landed.read_all_bytes().unwrap(), b"symbol,price\nAAPL,1\n");
        assert_eq!(buffer.size(), 0);

        LocalFolder::new(&root).unwrap().remove(true).unwrap();
    }

    #[test]
    fn a_move_onto_its_own_location_keeps_the_bytes() {
        use yggdryl::holder::Holder;
        use yggdryl::holder::counted::Counted;
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-transfer-self-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // Two handles on one location: a rename onto itself is a no-op, so the
        // move moves nothing, answers the value's size and leaves it there.
        let mut source = LocalFile::new(root.join("self.csv")).unwrap();
        source.write_all_bytes(b"symbol,price\nAAPL,1\n").unwrap();
        let mut target = LocalFile::new(root.join("self.csv")).unwrap();
        assert_eq!(source.move_into(&mut target).unwrap(), 20);
        assert_eq!(target.read_all_bytes().unwrap(), b"symbol,price\nAAPL,1\n");
        assert!(source.exists());
        assert_eq!(
            std::fs::read(root.join("self.csv")).unwrap(),
            b"symbol,price\nAAPL,1\n"
        );

        // A handle that answers no local URL - a wrapper here, an object on
        // a store - is refused by the location instead: one spelling may
        // name two stores, and a copy onto itself would end in the removal.
        let mut source = Counted::new(Holder::file(root.join("self.csv")).unwrap());
        let mut target = Counted::new(Holder::file(root.join("self.csv")).unwrap());
        let error = source.move_into(&mut target).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Conflict { .. }), "{error}");
        assert!(error.to_string().contains("self.csv"), "{error}");
        assert_eq!(
            std::fs::read(root.join("self.csv")).unwrap(),
            b"symbol,price\nAAPL,1\n"
        );

        LocalFolder::new(&root).unwrap().remove(true).unwrap();
    }

    #[test]
    fn a_move_of_an_absent_source_refuses_by_name_and_leaves_the_target() {
        use yggdryl::holder::Holder;
        use yggdryl::local::{LocalFile, LocalFolder};

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!(
            "yggdryl-transfer-absent-move-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // Nothing at the source: refused by name before the target is read or
        // written, never an empty value copied over it.
        let mut source = LocalFile::new(root.join("missing.csv")).unwrap();
        let mut target = LocalFile::new(root.join("target.csv")).unwrap();
        target.write_all_bytes(b"kept").unwrap();
        let error = source.move_into(&mut target).unwrap_err();
        assert!(error.is_absent(), "{error}");
        assert!(error.to_string().contains("missing.csv"), "{error}");
        assert_eq!(target.read_all_bytes().unwrap(), b"kept");
        assert_eq!(std::fs::read(root.join("target.csv")).unwrap(), b"kept");

        // The same through holders, the pair that would otherwise rename.
        let mut source = Holder::file(root.join("missing.csv")).unwrap();
        let mut target = Holder::file(root.join("target.csv")).unwrap();
        let error = source.move_into(&mut target).unwrap_err();
        assert!(error.is_absent(), "{error}");
        assert_eq!(target.read_all_bytes().unwrap(), b"kept");
        assert!(!root.join("missing.csv").exists());

        LocalFolder::new(&root).unwrap().remove(true).unwrap();
    }

    #[test]
    fn a_local_move_between_holders_is_a_rename() {
        use std::sync::Arc;
        use std::time::{Duration, SystemTime};

        use yggdryl::holder::Holder;
        use yggdryl::holder::counted::Counted;
        use yggdryl::local::LocalFolder;

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-transfer-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // A payload past one transfer chunk, stamped with an instant no write
        // of today could produce: a rename keeps the file and its instant
        // where a copy would write a new file stamped now.
        let payload: Vec<u8> = (0..4 << 20).map(|index| (index % 251) as u8).collect();
        let stamped = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let mut source = Holder::file(root.join("source.bin")).unwrap();
        source.write_all_bytes(&payload).unwrap();
        source.close().unwrap();
        std::fs::File::options()
            .write(true)
            .open(root.join("source.bin"))
            .unwrap()
            .set_modified(stamped)
            .unwrap();

        // Into a folder that is not there yet: the move creates it, as a
        // write at the destination would.
        let landed = root.join("moved").join("target.bin");
        let mut target = Holder::file(&landed).unwrap();
        let moved = source.move_into(&mut target).unwrap();
        assert_eq!(moved, payload.len() as u64);
        assert_eq!(
            std::fs::metadata(&landed).unwrap().modified().unwrap(),
            stamped,
            "a rename keeps the file's own instant"
        );
        assert_eq!(target.read_all_bytes().unwrap(), payload);
        assert!(!source.exists());
        assert!(!root.join("source.bin").exists());

        // The location roles answer the same: a path onto a path renames, and
        // the target handle reads what landed under it.
        let mut source = Holder::local(&landed).unwrap();
        let again = root.join("again.bin");
        let mut target = Holder::local(&again).unwrap();
        let moved = source.move_into(&mut target).unwrap();
        assert_eq!(moved, payload.len() as u64);
        assert_eq!(
            std::fs::metadata(&again).unwrap().modified().unwrap(),
            stamped
        );
        assert_eq!(target.read_all_bytes().unwrap(), payload);
        assert!(!source.exists());
        assert!(!landed.exists());

        // A record configuration over local storage passes the bytes through
        // unchanged, so two of them rename too: the file keeps its instant.
        let rows = root.join("rows.arrows");
        let mut source = Holder::file(&rows).unwrap().into_declared_media();
        assert!(matches!(source, Holder::Media(_)), "{source:?}");
        source.write_all_bytes(&payload).unwrap();
        source.close().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&rows)
            .unwrap()
            .set_modified(stamped)
            .unwrap();
        let shelved = root.join("shelved").join("rows.arrows");
        let mut target = Holder::file(&shelved).unwrap().into_declared_media();
        assert!(matches!(target, Holder::Media(_)), "{target:?}");
        assert_eq!(source.move_into(&mut target).unwrap(), payload.len() as u64);
        assert_eq!(
            std::fs::metadata(&shelved).unwrap().modified().unwrap(),
            stamped,
            "a record configuration over local storage renames"
        );
        assert!(!rows.exists());

        // A cache or a counting wrapper over a local target is never renamed
        // beneath: it answers no local URL, so the move is the copy then the
        // removal, every byte written through the wrapper where its tally
        // sees it, and the file that lands is a new one stamped now.
        let mut source = Holder::local(&again).unwrap();
        let wrapped = root.join("wrapped.bin");
        let mut target = Counted::new(Holder::file(&wrapped).unwrap());
        let calls = Arc::clone(target.calls());
        assert_eq!(source.move_into(&mut target).unwrap(), payload.len() as u64);
        let tally = calls.snapshot().to_string();
        assert!(tally.contains("pwrite="), "{tally}");
        assert_eq!(std::fs::read(&wrapped).unwrap(), payload);
        assert_ne!(
            std::fs::metadata(&wrapped).unwrap().modified().unwrap(),
            stamped,
            "a copy lands a new file"
        );
        assert!(!again.exists());

        LocalFolder::new(&root).unwrap().remove(true).unwrap();
    }

    #[test]
    fn a_move_refuses_a_container_before_touching_either_side() {
        use yggdryl::Error;
        use yggdryl::local::LocalFolder;

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!(
            "yggdryl-transfer-move-container-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.csv"), b"symbol\nAAPL\n").unwrap();

        let mut folder = LocalFolder::new(&root).unwrap();
        let mut target = Buffer::from_bytes(b"kept".to_vec());
        let error = folder.move_into(&mut target).unwrap_err();
        assert!(
            matches!(
                error,
                Error::NotAtomic {
                    operation: "move",
                    kind: "directory",
                    ..
                }
            ),
            "{error}"
        );
        // Refused before anything moved: the folder's leaf and the target's
        // bytes are what they were.
        assert_eq!(
            std::fs::read(root.join("a.csv")).unwrap(),
            b"symbol\nAAPL\n"
        );
        assert_eq!(target.as_slice(), b"kept");

        folder.remove(true).unwrap();
    }

    #[test]
    fn compression_round_trips_and_tracks_the_coding() {
        let payload = "symbol,price\n".repeat(500).into_bytes();
        let source = Buffer::from_bytes(payload.clone())
            .with_media_type(Url::from_str("file:///trades.csv").unwrap().media_type());

        for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
            let mut compressed = Buffer::new();
            let written = source.compress_into(&mut compressed, codec).unwrap();

            assert_eq!(written, compressed.size());
            assert!(compressed.size() < source.size(), "{codec}");
            // The target records the coding, so decoding needs no extra argument.
            assert_eq!(compressed.codec(), codec, "{codec}");
            assert_eq!(compressed.media_type().base(), &MimeType::CSV, "{codec}");

            let mut restored = Buffer::new();
            compressed.decompress_into(&mut restored).unwrap();
            assert_eq!(restored.as_slice(), payload.as_slice(), "{codec}");
            // The coding is gone once decoded.
            assert_eq!(restored.codec(), Codec::Identity, "{codec}");
        }
    }

    #[test]
    fn a_coding_transfer_refuses_a_decoded_view_before_touching_it() {
        use yggdryl::coding::Coded;
        use yggdryl::holder::Holder;

        let plain = Buffer::from_bytes(b"symbol,price\n".to_vec());
        let mut stored = Buffer::new();
        plain.compress_into(&mut stored, Codec::Gzip).unwrap();
        let gzipped = stored.as_slice().to_vec();

        // A coded target would code the zstd bytes a second time.
        let mut view = Holder::from(Coded::wrap(Buffer::new(), Codec::Gzip));
        assert_eq!(view.applied_codec(), Codec::Gzip);
        let refused = plain
            .compress_into(&mut view, Codec::Zstd)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a target presenting its stored bytes"),
            "{refused}"
        );
        assert!(refused.contains("gzip view"), "{refused}");
        assert_eq!(view.size(), 0);

        // A coded source has already decoded what the transfer would decode.
        let source = Coded::wrap(Buffer::from_bytes(gzipped), Codec::Gzip);
        let mut target = Buffer::from_bytes(b"kept".to_vec());
        let refused = source
            .decompress_into_with(&mut target, Codec::Gzip)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a source presenting its stored bytes"),
            "{refused}"
        );
        assert_eq!(target.as_slice(), b"kept");

        // Through the view, `copy_into` is the call that stores the coded form.
        let mut view = Coded::wrap(Buffer::new(), Codec::Gzip);
        plain.copy_into(&mut view).unwrap();
        assert_eq!(view.read_all_bytes().unwrap(), b"symbol,price\n");
    }

    #[test]
    fn a_value_transfer_refuses_a_container_before_touching_the_target() {
        use yggdryl::Error;
        use yggdryl::local::{LocalFolder, LocalPath};

        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-transfer-container-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.csv"), b"symbol\nAAPL\n").unwrap();
        std::fs::write(
            root.join("b.csv.gz"),
            Codec::Gzip.dump(b"symbol\nMSFT\n").unwrap(),
        )
        .unwrap();

        let pattern = Url::from_path(&root).unwrap().joinpath("*.csv").unwrap();
        let sources: [(&str, Box<dyn IOBase>); 2] = [
            ("folder", Box::new(LocalFolder::new(&root).unwrap())),
            ("pattern", Box::new(LocalPath::from_url(pattern).unwrap())),
        ];
        /// One transfer of a source's value into a target.
        type Transfer = fn(&dyn IOBase, &mut Buffer) -> yggdryl::Result<u64>;

        let transfers: [(&str, Transfer); 4] = [
            ("copy", |source, target| source.copy_into(target)),
            ("compress", |source, target| {
                source.compress_into(target, Codec::Gzip)
            }),
            ("decompress", |source, target| {
                source.decompress_into(target)
            }),
            ("decompress", |source, target| {
                source.decompress_into_with(target, Codec::Gzip)
            }),
        ];
        for (name, source) in &sources {
            // A container's stream has bytes - its leaves', end to end - and
            // that concatenation is what no transfer of one value may take
            // for one.
            assert!(!source.read_all_bytes().unwrap().is_empty(), "{name}");
            for (operation, transfer) in transfers {
                let mut target = Buffer::from_bytes(b"kept".to_vec())
                    .with_media_type(MediaType::from(MimeType::JSON));
                let error = transfer(source.as_ref(), &mut target).unwrap_err();
                assert!(
                    matches!(
                        &error,
                        Error::NotAtomic { operation: refused, kind: "directory", .. }
                            if *refused == operation
                    ),
                    "{name} {operation}: {error}"
                );
                assert!(error.to_string().contains("got a directory"), "{error}");
                // Refused before the target was touched: its bytes and the
                // media type it declares are what they were.
                assert_eq!(target.as_slice(), b"kept", "{name} {operation}");
                assert_eq!(
                    target.media_type().base(),
                    &MimeType::JSON,
                    "{name} {operation}"
                );
            }
        }

        LocalFolder::new(&root).unwrap().remove(true).unwrap();
    }

    #[test]
    fn a_copy_between_two_filesystem_locations_refuses_a_container_too() {
        use std::sync::Arc;

        use yggdryl::Error;
        use yggdryl::fs::{FileSystem, FsFile, FsFolder, MemoryFileSystem};

        // Both ends bound to one filesystem is the one copy the store makes
        // itself; a container is still no value to make one of.
        let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
        let mut leaf = FsFile::from_path(Arc::clone(&filesystem), "logs/a.csv", None).unwrap();
        leaf.write_all_bytes(b"symbol\nAAPL\n").unwrap();
        let mut target = FsFile::from_path(Arc::clone(&filesystem), "out.json", None).unwrap();
        target.write_all_bytes(b"kept").unwrap();

        let folder = FsFolder::from_path(Arc::clone(&filesystem), "logs", None).unwrap();
        assert_eq!(folder.read_all_bytes().unwrap(), b"symbol\nAAPL\n");
        let error = folder.copy_into(&mut target).unwrap_err();
        assert!(
            matches!(
                error,
                Error::NotAtomic {
                    operation: "copy",
                    kind: "directory",
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(target.read_all_bytes().unwrap(), b"kept");
        assert_eq!(target.media_type().base(), &MimeType::JSON);
    }

    #[test]
    fn streaming_adapters_advance_their_own_offset() {
        let mut buffer = Buffer::new();
        {
            let mut writer = buffer.writer_at(0);
            writer.write_all(b"symbol,").unwrap();
            writer.write_all(b"price").unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(buffer.as_slice(), b"symbol,price");

        let mut text = String::new();
        buffer.reader_at(0).read_to_string(&mut text).unwrap();
        assert_eq!(text, "symbol,price");

        // A reader can start anywhere without disturbing another.
        let mut tail = String::new();
        buffer.reader_at(7).read_to_string(&mut tail).unwrap();
        assert_eq!(tail, "price");
    }

    #[test]
    fn read_range_is_bounded_by_the_value() {
        let buffer = Buffer::from_bytes(b"0123456789".to_vec());
        assert_eq!(buffer.read_range_bytes(2, 3).unwrap(), b"234");
        // Asking past the end yields what exists rather than failing.
        assert_eq!(buffer.read_range_bytes(8, 100).unwrap(), b"89");
        assert!(buffer.read_range_bytes(50, 4).unwrap().is_empty());
    }

    #[test]
    fn write_all_bytes_replaces_the_whole_value() {
        let mut buffer = Buffer::from_bytes(b"a much longer previous value".to_vec());
        buffer.write_all_bytes(b"short").unwrap();
        assert_eq!(buffer.as_slice(), b"short");
        assert_eq!(buffer.size(), 5);
    }
}

mod identity {

    use yggdryl::holder::Buffer;
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, Uri, Url};

    /// A handle says what it is addressed by, and a location is one kind of
    /// address rather than the only one: `uri` is what every backend answers,
    /// and `url` is that identifier when it names a place.
    #[test]
    fn a_handle_is_addressed_by_an_identifier_and_a_location_is_one() {
        let root = LocalFolder::temporary().unwrap().path().unwrap();
        let folder = LocalFolder::new(&root).unwrap();

        // The two accessors read one value, never two.
        assert_eq!(
            IOBase::uri(&folder),
            IOBase::url(&folder).map(AsRef::<Uri>::as_ref),
            "a located handle answers one identifier"
        );
        assert_eq!(IOBase::uri(&folder).unwrap().scheme().as_str(), "file");

        // A buffer is not stored anywhere, so its address is an identity; it is
        // still an identifier, and still the same one through both doors.
        let buffer = Buffer::from_bytes(b"symbol\n".to_vec());
        let identity = buffer.uri().cloned().unwrap();
        assert_eq!(identity.scheme().as_str(), "mem");
        assert_eq!(buffer.url().map(Url::to_string), Some(identity.to_string()));

        // Composition addresses the same thing it wrapped: a wrapper forwards
        // the identity rather than inventing one.
        let wrapped = buffer.buffered(BufferedOptions::default());
        assert_eq!(wrapped.uri(), Some(&identity));
        assert_eq!(
            wrapped.url().map(Url::to_string),
            Some(identity.to_string())
        );
    }
}

/// `IOBase::create_bytes`: the value written where nothing is, and the
/// conflict - from the one attempt - where something already is.
mod create {
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    use yggdryl::fs::{FileSystem, FsFile, MemoryFileSystem};
    use yggdryl::holder::Buffer;
    use yggdryl::local::{LocalFile, LocalFolder};
    use yggdryl::{Error, IOBase};

    /// A fresh path under the temporary folder, nothing at it.
    fn fresh(label: &str) -> PathBuf {
        let mut root = LocalFolder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-create-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root.join("value.bin")
    }

    /// Remove what a local case left behind, through the abstraction.
    fn cleanup(path: &std::path::Path) {
        if let Some(parent) = path.parent()
            && let Ok(mut folder) = LocalFolder::new(parent)
        {
            folder.remove(true).expect("a removable tree");
        }
    }

    /// A memory filesystem with the parent of `bench/{label}.bin` in place.
    fn memory_file(label: &str) -> (Arc<MemoryFileSystem>, FsFile) {
        let memory = Arc::new(MemoryFileSystem::new());
        memory
            .create_dir("bench", false)
            .expect("a writable memory root");
        let file = FsFile::from_path(
            Arc::clone(&memory) as Arc<dyn FileSystem>,
            format!("bench/{label}.bin"),
            None,
        )
        .expect("a valid location");
        (memory, file)
    }

    #[test]
    fn a_create_on_nothing_writes_and_publishes() {
        let mut buffer = Buffer::new();
        buffer
            .create_bytes(b"AAPL,187.23")
            .expect("an empty buffer");
        assert_eq!(buffer.read_all_bytes().unwrap(), b"AAPL,187.23");

        // A second handle on the location reads the value on return: no
        // flush or close is left to the caller.
        let path = fresh("publish");
        LocalFile::new(&path)
            .unwrap()
            .create_bytes(b"AAPL,187.23")
            .expect("nothing at the path, its parent created");
        assert_eq!(std::fs::read(&path).unwrap(), b"AAPL,187.23");
        assert_eq!(
            LocalFile::new(&path).unwrap().read_all_bytes().unwrap(),
            b"AAPL,187.23"
        );
        cleanup(&path);

        let (memory, mut file) = memory_file("publish");
        file.create_bytes(b"AAPL,187.23")
            .expect("nothing at the path");
        let other =
            FsFile::from_path(memory as Arc<dyn FileSystem>, "bench/publish.bin", None).unwrap();
        assert_eq!(other.read_all_bytes().unwrap(), b"AAPL,187.23");
    }

    #[test]
    fn a_create_on_a_value_is_a_conflict_naming_the_location_and_leaving_the_value() {
        fn refused(handle: &mut dyn IOBase, location: &str) {
            let error = handle
                .create_bytes(b"MSFT,410.10")
                .expect_err("a value is already there");
            assert!(
                matches!(&error, Error::Conflict { path, .. } if path.as_str() == location),
                "{error:?} does not carry path {location:?}"
            );
            assert!(error.is_conflict());
            let quoted_location = format!("{location:?}");
            assert!(
                error.to_string().contains(&quoted_location),
                "{error} names {quoted_location}"
            );
        }

        let mut buffer = Buffer::from_bytes(b"AAPL,187.23".to_vec());
        let identity = buffer.url().unwrap().to_string();
        refused(&mut buffer, &identity);
        assert_eq!(buffer.read_all_bytes().unwrap(), b"AAPL,187.23");

        let path = fresh("conflict");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"AAPL,187.23").unwrap();
        let mut local = LocalFile::new(&path).unwrap();
        refused(&mut local, &path.display().to_string());
        assert_eq!(std::fs::read(&path).unwrap(), b"AAPL,187.23");
        // A handle that already mapped the file asks the path all the same.
        local.open().unwrap();
        refused(&mut local, &path.display().to_string());
        assert_eq!(local.read_all_bytes().unwrap(), b"AAPL,187.23");
        drop(local);
        assert_eq!(std::fs::read(&path).unwrap(), b"AAPL,187.23");
        cleanup(&path);

        let (_memory, mut file) = memory_file("conflict");
        file.write_all_bytes(b"AAPL,187.23").unwrap();
        refused(&mut file, "bench/conflict.bin");
        assert_eq!(file.read_all_bytes().unwrap(), b"AAPL,187.23");
    }

    #[test]
    fn of_sixteen_racing_creators_of_one_local_path_exactly_one_succeeds() {
        const CREATORS: usize = 16;
        let path = fresh("race");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let barrier = Arc::new(Barrier::new(CREATORS));
        let outcomes: Vec<(Vec<u8>, yggdryl::Result<()>)> = std::thread::scope(|scope| {
            let creators: Vec<_> = (0..CREATORS)
                .map(|creator| {
                    let barrier = Arc::clone(&barrier);
                    let path = &path;
                    scope.spawn(move || {
                        let payload = format!("creator-{creator:02}").into_bytes();
                        let mut handle = LocalFile::new(path).unwrap();
                        barrier.wait();
                        let outcome = handle.create_bytes(&payload);
                        (payload, outcome)
                    })
                })
                .collect();
            creators
                .into_iter()
                .map(|creator| creator.join().unwrap())
                .collect()
        });
        let winners: Vec<&Vec<u8>> = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.is_ok())
            .map(|(payload, _)| payload)
            .collect();
        assert_eq!(winners.len(), 1, "exactly one creator wins");
        for (_, outcome) in &outcomes {
            if let Err(error) = outcome {
                assert!(
                    matches!(error, Error::Conflict { .. }),
                    "every other creator is the conflict, got {error}"
                );
            }
        }
        assert_eq!(&std::fs::read(&path).unwrap(), winners[0]);
        cleanup(&path);
    }
}
