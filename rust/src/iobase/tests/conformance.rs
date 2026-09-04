use super::{Buffer, IOBase};

use std::sync::Arc;

/// Every backend under test, each as a freshly built empty handle.
///
/// The handles are boxed because the battery is one function rather than
/// one per backend; `IOBase` is implemented for the box, so the byte half
/// of the contract forwards unchanged.
fn backends(label: &str) -> Vec<(&'static str, Box<dyn IOBase>)> {
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!(
        "yggdryl-conformance-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a writable temporary root");

    let memory = Arc::new(crate::holder::arrowfs::MemoryFileSystem::new());
    vec![
        ("buffer", Box::new(Buffer::new()) as Box<dyn IOBase>),
        (
            "local::File",
            Box::new(
                crate::holder::local::File::create(root.join(format!("{label}.bin")))
                    .expect("a valid path"),
            ),
        ),
        (
            "arrowfs::File",
            Box::new(
                crate::holder::arrowfs::File::from_location(memory, &format!("bench/{label}.bin"))
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
                    crate::holder::buffered::BufferedOptions::default().with_page_size(4),
                ),
            ),
        ),
    ]
}

/// Remove whatever the local backend left behind.
fn cleanup(label: &str) {
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!(
        "yggdryl-conformance-{label}-{}",
        std::process::id()
    ));
    // Teardown goes through the abstraction, not around it: a folder
    // handle already addresses this tree, and absence is a no-op success.
    if let Ok(mut folder) = crate::holder::local::Folder::new(&root) {
        folder.remove(true).expect("a removable tree");
    }
}

#[test]
fn every_backend_grows_and_zero_fills_a_write_gap() {
    for (name, mut handle) in backends("gap") {
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
    let memory = Arc::new(crate::holder::arrowfs::MemoryFileSystem::new());
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-conformance-absent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let absent: Vec<(&str, Box<dyn IOBase>)> = vec![
        (
            "local::File",
            Box::new(
                crate::holder::local::File::new(root.join("absent.bin")).expect("a valid path"),
            ),
        ),
        (
            "arrowfs::File",
            Box::new(
                crate::holder::arrowfs::File::from_location(memory, "nowhere/absent.bin")
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
    for (name, mut handle) in backends("truncate") {
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
    for (name, mut handle) in backends("capacity") {
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

        for codec in [crate::Codec::Gzip, crate::Codec::Zlib, crate::Codec::Zstd] {
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
