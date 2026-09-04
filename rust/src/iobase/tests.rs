//! Behavior every [`IOBase`] implementation must share.

use std::io::{Read, Write};

use super::IOBase;
use crate::Codec;
use crate::holder::Buffer;
use crate::{Field, MediaType, MimeType, Scalar, Url};

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
fn structured_values_follow_the_declared_format_and_content_coding() {
    let expected = Scalar::from_record([
        ("quantity", Scalar::I64(2)),
        ("symbol", Scalar::from("AAPL")),
    ])
    .unwrap();

    for name in [
        "trade.json",
        "trade.json.gz",
        "trade.json.zz",
        "trade.json.zst",
        "trade.yaml",
        "trade.toml",
    ] {
        let media = Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type();
        let mut handle = Buffer::new().with_media_type(media);
        handle
            .write_scalar(&expected)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = handle
            .read_scalar(None)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn structured_value_fields_direct_parsing_and_casting() {
    let media = Url::from_str("file:///trade.json").unwrap().media_type();
    let source = Buffer::from_bytes(br#"{"quantity":2}"#.to_vec()).with_media_type(media);
    let field = Field::from_str("trade: struct<quantity: int32 not null> not null").unwrap();
    let expected = Scalar::from_sequence([Scalar::I64(2)]);

    assert_eq!(source.read_scalar(Some(&field)).unwrap(), expected);

    let invalid = Buffer::from_bytes(br#"{"quantity":"many"}"#.to_vec())
        .with_media_type(Url::from_str("file:///trade.json").unwrap().media_type());
    let message = invalid.read_scalar(Some(&field)).unwrap_err().to_string();
    assert!(message.contains("quantity"), "{message}");
    assert!(message.contains("int32"), "{message}");
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

#[test]
fn boxed_cursors_preserve_lifecycle_hierarchy_and_kind() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::holder::Holder;
    use crate::{IOKind, Result};
    use crate::{IOMedia, Listing};

    struct Probe {
        bytes: Buffer,
        opened: Arc<AtomicBool>,
    }

    impl IOMedia for Probe {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Probe {
        crate::delegate_iobase!(bytes: pread, pstream_bytes, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, clear, remove);

        fn open(&mut self) -> Result<()> {
            self.opened.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn opened(&self) -> bool {
            self.opened.load(Ordering::SeqCst)
        }

        fn close(&mut self) -> Result<()> {
            self.opened.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn parent(&self) -> Option<Holder> {
            Some(Holder::buffer(Buffer::from_bytes(b"parent".to_vec())))
        }

        fn child_by_path(&self, path: &str) -> Result<Holder> {
            Ok(Holder::buffer(Buffer::from_bytes(path.as_bytes().to_vec())))
        }

        fn ls(&self, recursive: bool, include_private: bool) -> Listing {
            let value = format!("{recursive}:{include_private}");
            Listing::new(std::iter::once(Ok(Holder::buffer(Buffer::from_bytes(
                value.into_bytes(),
            )))))
        }

        fn kind(&self) -> IOKind {
            IOKind::Directory
        }
    }

    let state = Arc::new(AtomicBool::new(false));
    let mut handle: Box<dyn IOBase> = Box::new(crate::Cursor::new(Probe {
        bytes: Buffer::new(),
        opened: Arc::clone(&state),
    }));

    assert_eq!(handle.kind(), IOKind::Directory);
    assert!(handle.is_container());
    assert_eq!(
        handle.parent().unwrap().read_all_bytes().unwrap(),
        b"parent"
    );
    assert_eq!(
        handle
            .child_by_path("nested/leaf")
            .unwrap()
            .read_all_bytes()
            .unwrap(),
        b"nested/leaf"
    );
    assert_eq!(
        handle
            .ls(true, true)
            .next()
            .unwrap()
            .unwrap()
            .read_all_bytes()
            .unwrap(),
        b"true:true"
    );

    handle.open().unwrap();
    assert!(handle.opened());
    assert!(state.load(Ordering::SeqCst));
    handle.close().unwrap();
    assert!(handle.closed());
    assert!(!state.load(Ordering::SeqCst));
}

mod applying;
mod buffered_handle;
mod conformance;
mod laziness;
mod lifecycle;
/// Any handle reads through one reader and writes through three explicit
/// intents. Held record batches are zero-copy adapters over those primitives.
#[cfg(feature = "arrow")]
#[cfg(feature = "arrow")]
mod records;
mod shape;
