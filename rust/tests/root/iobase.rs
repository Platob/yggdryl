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
        let mut root = yggdryl::local::Folder::temporary().unwrap().path().unwrap();
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
                "local::File",
                Box::new(
                    yggdryl::local::File::create(root.join(format!("{label}.bin")))
                        .expect("a valid path"),
                ),
            ),
            (
                "fs::File",
                Box::new(
                    yggdryl::fs::File::from_path(memory, format!("bench/{label}.bin"), None)
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
            .filter(|(name, _)| *name != "fs::File")
            .collect()
    }

    /// Remove whatever the local backend left behind.
    fn cleanup(label: &str) {
        let mut root = yggdryl::local::Folder::temporary().unwrap().path().unwrap();
        root.push(format!(
            "yggdryl-conformance-{label}-{}",
            std::process::id()
        ));
        // Teardown goes through the abstraction, not around it: a folder
        // handle already addresses this tree, and absence is a no-op success.
        if let Ok(mut folder) = yggdryl::local::Folder::new(&root) {
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
        let mut root = yggdryl::local::Folder::temporary().unwrap().path().unwrap();
        root.push(format!("yggdryl-conformance-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let absent: Vec<(&str, Box<dyn IOBase>)> = vec![
            (
                "local::File",
                Box::new(yggdryl::local::File::new(root.join("absent.bin")).expect("a valid path")),
            ),
            (
                "fs::File",
                Box::new(
                    yggdryl::fs::File::from_path(memory, "nowhere/absent.bin", None)
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
        let mut handle = yggdryl::fs::File::from_path(filesystem, "bench/random.bin", None)
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
    fn a_buffer_reports_a_mem_identity_rather_than_a_location() {
        let buffer = Buffer::from_bytes(b"bytes".to_vec());
        let identity = buffer.url().expect("a buffer always has an identity");

        // The bytes are not stored anywhere, so this names the process and the
        // allocation rather than a place on disk.
        assert_eq!(identity.scheme().as_str(), "mem");
        assert_eq!(
            identity.authority().as_str(),
            std::process::id().to_string()
        );
        assert!(identity.path().as_str().contains("0x"), "{identity}");

        // The identity is stable for one handle.
        assert_eq!(buffer.url(), Some(identity));

        // A distinct buffer is distinguishable from it.
        let other = Buffer::from_bytes(b"bytes".to_vec());
        assert_ne!(other.url(), Some(identity));
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
