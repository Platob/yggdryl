//! `rust/src/zip/archive.rs`: `ZipArchive::copy_member_from` - a member moved between archives as it is stored, its encoded bytes, digest, sizes and restart map carried and nothing decoded, and each refusal naming the member.

use std::sync::Arc;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;
use yggdryl::{Codec, Error, IOBase};

/// A payload long enough to hold several restart strides.
fn payload(size: usize) -> Vec<u8> {
    b"symbol,price,venue\nAAPL,187.23,XNAS\n"
        .iter()
        .copied()
        .cycle()
        .take(size)
        .collect()
}

/// An archive over an empty buffer.
fn empty() -> Arc<ZipArchive> {
    Arc::new(ZipArchive::new(Holder::buffer(Buffer::new())))
}

/// The published bytes of `archive`, which it gives up.
fn published(archive: Arc<ZipArchive>) -> Vec<u8> {
    let archive = Arc::into_inner(archive).expect("the only holder");
    archive
        .into_handle()
        .expect("the archive publishes")
        .read_all_bytes()
        .expect("the bytes")
}

#[test]
fn a_member_the_source_does_not_hold_or_the_archive_root_is_refused_by_name() {
    let source = empty();
    source.write_member("held.csv", b"a").unwrap();
    let target = empty();
    let error = target.copy_member_from(&source, "absent.csv").unwrap_err();
    assert!(matches!(error, Error::Absent { .. }), "{error}");
    assert!(error.to_string().contains("absent.csv"), "{error}");
    let error = target.copy_member_from(&source, "").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected a member to copy, got the archive root"),
        "{error}"
    );
    // Nothing was written, and nothing is owed.
    assert_eq!(target.handle_writes(), 0);
    assert!(target.entries().unwrap().is_empty());
}

#[test]
fn a_copied_member_is_its_source_record_and_reads_back_byte_for_byte() {
    let data = payload(200 * 1024);
    let source =
        Arc::new(ZipArchive::new(Holder::buffer(Buffer::new())).with_restart_stride(16 * 1024));
    let original = source
        .write_member_with("trades/eu.csv", &data, Codec::Deflate)
        .unwrap();
    source
        .write_member_with("trades/stored.csv", b"stored as is", Codec::Identity)
        .unwrap();
    assert!(!original.restarts().is_empty());

    let target = empty();
    let copied = target.copy_member_from(&source, "trades/eu.csv").unwrap();
    assert_eq!(copied.name(), original.name());
    assert_eq!(copied.crc32(), original.crc32());
    assert_eq!(copied.compressed_size(), original.compressed_size());
    assert_eq!(copied.size(), original.size());
    assert_eq!(copied.codec().unwrap(), Codec::Deflate);
    assert_eq!(copied.restarts(), original.restarts());
    assert_eq!(copied.modified(), original.modified());
    let stored = target
        .copy_member_from(&source, "trades/stored.csv")
        .unwrap();
    assert_eq!(stored.codec().unwrap(), Codec::Identity);

    // Read here, and again from the published bytes by a fresh mount: the
    // record's header and its map describe the bytes that moved.
    assert_eq!(target.read_member("trades/eu.csv").unwrap(), data);
    let reopened = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        published(target),
    ))));
    assert_eq!(reopened.read_member("trades/eu.csv").unwrap(), data);
    assert_eq!(
        reopened.read_member("trades/stored.csv").unwrap(),
        b"stored as is"
    );
    let entry = reopened.get_entry("trades/eu.csv").unwrap().unwrap();
    assert_eq!(entry.restarts(), original.restarts());
    assert_eq!(entry.crc32(), original.crc32());
}

#[test]
fn a_copy_replaces_a_member_of_the_same_name_and_a_member_copied_into_its_own_archive_stands() {
    let source = empty();
    source.write_member("a.txt", b"from the source").unwrap();
    let target = empty();
    target.write_member("a.txt", b"already here").unwrap();
    target.copy_member_from(&source, "a.txt").unwrap();
    assert_eq!(target.read_member("a.txt").unwrap(), b"from the source");
    assert_eq!(target.entries().unwrap().len(), 1);

    let writes = source.handle_writes();
    let itself = source.copy_member_from(&source, "a.txt").unwrap();
    assert_eq!(itself, source.get_entry("a.txt").unwrap().unwrap());
    assert_eq!(source.handle_writes(), writes);
}

#[test]
fn copies_in_both_directions_at_once_finish() {
    let left = empty();
    let right = empty();
    left.write_member("left.bin", &payload(64 * 1024)).unwrap();
    right
        .write_member("right.bin", &payload(32 * 1024))
        .unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| left.copy_member_from(&right, "right.bin").unwrap());
            scope.spawn(|| right.copy_member_from(&left, "left.bin").unwrap());
        }
    });
    assert_eq!(left.read_member("right.bin").unwrap(), payload(32 * 1024));
    assert_eq!(right.read_member("left.bin").unwrap(), payload(64 * 1024));
}

#[cfg(feature = "internals")]
mod internal {
    use std::sync::Arc;

    use yggdryl::Error;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::internals::{zip_entry, zip_format as format};
    use yggdryl::zip::ZipArchive;

    #[test]
    fn an_encrypted_member_is_refused_by_name() {
        let entry = zip_entry::Draft::new("secret.txt", 0, 0)
            .with_flags(format::FLAG_UTF8 | format::FLAG_ENCRYPTED)
            .with_content(0, 3, 3)
            .entry();
        let mut image = Vec::new();
        format::write_local_with(&entry, false, &mut image);
        image.extend_from_slice(b"abc");
        let directory_offset = image.len() as u64;
        let mut trailer = Vec::new();
        format::write_central(&entry, &mut trailer);
        let directory_size = trailer.len() as u64;
        format::write_end(directory_offset, directory_size, 1, b"", &mut trailer);
        image.extend_from_slice(&trailer);

        let source = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(image))));
        let target = ZipArchive::new(Holder::buffer(Buffer::new()));
        let error = target.copy_member_from(&source, "secret.txt").unwrap_err();
        assert!(matches!(error, Error::Unsupported { .. }), "{error}");
        assert!(error.to_string().contains("secret.txt"), "{error}");
    }
}
