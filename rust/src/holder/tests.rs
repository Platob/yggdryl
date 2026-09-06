//! Composing a holder from the media type its name declares.

use super::{Buffer, Holder};
use crate::holder::buffered::BufferedOptions;
use crate::{Codec, IOBase, MimeType, Url};

const PLAIN: &[u8] = b"symbol,price\nAAPL,1\n";

/// A handle holding `bytes` under the media type `name` declares.
fn named(name: &str, bytes: Vec<u8>) -> (Holder, crate::MediaType) {
    let url = Url::from_str(&format!("file:///{name}")).unwrap();
    let media_type = url.media_type();
    let holder = Holder::buffer(Buffer::from_bytes(bytes).with_media_type(media_type.clone()));
    (holder, media_type)
}

/// The composed handle for `name` over the coding its suffix declares.
fn composed(name: &str) -> Holder {
    let codec = Codec::from_url(&Url::from_str(&format!("file:///{name}")).unwrap());
    let (holder, _) = named(name, codec.dump(PLAIN).unwrap());
    holder.into_declared_media()
}

#[test]
fn a_coding_and_a_text_base_compose_as_text_over_the_decoded_view() {
    for name in ["trades.txt.gz", "trades.txt.zst", "trades.txt.zz"] {
        let handle = composed(name);
        match &handle {
            Holder::Text(text) => assert!(
                matches!(text.handle(), Holder::Coded(_)),
                "{name} held {:?}",
                text.handle()
            ),
            other => panic!("expected text over a coding for {name}, got {other:?}"),
        }
        assert_eq!(handle.read_all_bytes().unwrap(), PLAIN, "{name}");
        assert_eq!(handle.media_type().base(), &MimeType::PLAIN_TEXT, "{name}");
        assert!(handle.media_type().encodings().is_empty(), "{name}");
    }
}

#[test]
fn a_coding_alone_composes_only_the_decoded_view() {
    let handle = composed("archive.bin.gz");
    assert!(matches!(handle, Holder::Coded(_)), "{handle:?}");
    assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
}

#[test]
fn a_record_encoding_alone_composes_only_the_media() {
    let (handle, _) = named("rows.arrows", Vec::new());
    assert!(matches!(handle.into_declared_media(), Holder::Media(_)));

    let (handle, _) = named("rows.parquet", Vec::new());
    assert!(matches!(handle.into_declared_media(), Holder::Media(_)));

    let (handle, _) = named("rows.txt", PLAIN.to_vec());
    assert!(matches!(handle.into_declared_media(), Holder::Text(_)));
}

#[test]
fn a_name_declaring_neither_is_returned_unchanged() {
    let (handle, _) = named("notes.json", PLAIN.to_vec());
    let handle = handle.into_declared_media();
    assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");
    assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
}

#[test]
fn repeating_the_composition_stacks_nothing() {
    let codec = Codec::Gzip;
    let (handle, _) = named("trades.txt.gz", codec.dump(PLAIN).unwrap());
    let once = handle.into_declared_media();
    let twice = once.into_declared_media();
    match &twice {
        Holder::Text(text) => match text.handle() {
            Holder::Coded(coded) => assert!(matches!(coded.handle(), Holder::Buffer(_))),
            other => panic!("expected one coding under the text, got {other:?}"),
        },
        other => panic!("expected text over a coding, got {other:?}"),
    }
    assert_eq!(twice.read_all_bytes().unwrap(), PLAIN);
}

#[test]
fn a_page_cache_stays_outside_the_composition() {
    let (handle, _) = named("trades.txt.gz", Codec::Gzip.dump(PLAIN).unwrap());
    let handle = handle
        .buffered(BufferedOptions::default())
        .into_declared_media();
    match &handle {
        Holder::Buffered(buffered) => match buffered.handle() {
            Holder::Text(text) => assert!(matches!(text.handle(), Holder::Coded(_))),
            other => panic!("expected text over a coding under the cache, got {other:?}"),
        },
        other => panic!("expected the cache outermost, got {other:?}"),
    }
    assert_eq!(handle.read_all_bytes().unwrap(), PLAIN);
}

#[test]
fn composing_never_resolves_the_location() {
    // The media type is the caller's, so composing never resolves what is
    // there: an absent location composes, reads empty, and stays lazy.
    let url = Url::from_str("file:///yggdryl-absent-composition/trades.txt.gz").unwrap();
    let handle = Holder::local(url.into_path().unwrap())
        .unwrap()
        .into_declared_media();
    assert!(matches!(handle, Holder::Text(_)), "{handle:?}");
    assert!(handle.read_all_bytes().unwrap().is_empty());
}

#[test]
fn every_wrapper_keeps_the_filesystem_location_it_stands_on() {
    use std::sync::Arc;

    use crate::holder::fs::{BoundLocation, FileSystem, MemoryFileSystem};

    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let stored = Codec::Gzip.dump(PLAIN).unwrap();
    let mut writer = filesystem
        .open_output_stream("trades.txt.gz", None)
        .unwrap();
    let mut offset = 0;
    while offset < stored.len() {
        offset += writer.write(&stored[offset..]).unwrap();
    }
    writer.close().unwrap();

    let location = BoundLocation::new(filesystem, "trades.txt.gz", None::<String>).unwrap();

    // A wrapper answers where the bytes live, or every filesystem accessor -
    // the raw path, the URI, the info call - goes blank the moment a handle is
    // composed.
    let composed = crate::holder::fs::located(location.clone()).into_declared_media();
    assert!(matches!(composed, Holder::Text(_)), "{composed:?}");
    assert_eq!(
        composed.bound_location().map(BoundLocation::path),
        Some("trades.txt.gz")
    );
    assert_eq!(composed.read_all_bytes().unwrap(), PLAIN);

    let cached = crate::holder::fs::located(location).buffered(BufferedOptions::default());
    assert_eq!(
        cached.bound_location().map(BoundLocation::path),
        Some("trades.txt.gz")
    );
}

#[cfg(feature = "parquet")]
#[test]
fn a_coding_around_parquet_is_left_for_the_writer_to_refuse() {
    // Parquet compresses internally, so a decoded view over `.parquet.gz`
    // would hide a name no other Parquet reader can open.
    let (handle, _) = named("trades.parquet.gz", Vec::new());
    let handle = handle.into_declared_media();
    assert!(matches!(handle, Holder::Buffer(_)), "{handle:?}");
    assert_eq!(handle.codec(), Codec::Gzip);
}

#[test]
fn a_composed_absent_location_is_still_absent() {
    // Every wrapper reports the storage role of what it stands on, so a
    // composed handle for a location that is not there answers `Unknown` and
    // reads empty rather than claiming to be a file.
    let url = Url::from_str("file:///yggdryl-absent-composition/trades.txt.gz").unwrap();
    let handle = Holder::local(url.into_path().unwrap())
        .unwrap()
        .into_declared_media();
    assert_eq!(handle.kind(), crate::IOKind::Unknown);
    assert!(handle.read_all_bytes().unwrap().is_empty());
}

#[test]
fn a_composed_whole_read_decodes_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A handle that counts how many times its bytes are streamed.
    #[derive(Debug)]
    struct Counted {
        handle: Buffer,
        streams: std::sync::Arc<AtomicUsize>,
    }

    impl crate::IOMedia for Counted {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Counted {
        crate::delegate_iobase!(handle: pread, pwrite, size, capacity, reserve, truncate, url,
            media_type, set_media_type, flush, kind);

        fn pstream_bytes(
            &self,
            position: u64,
            batch_size: usize,
        ) -> crate::Result<crate::ByteStream<'_>> {
            self.streams.fetch_add(1, Ordering::Relaxed);
            self.handle.pstream_bytes(position, batch_size)
        }
    }

    let streams = std::sync::Arc::new(AtomicUsize::new(0));
    let url = Url::from_str("file:///trades.txt.gz").unwrap();
    let source = Counted {
        handle: Buffer::from_bytes(Codec::Gzip.dump(PLAIN).unwrap())
            .with_media_type(url.media_type()),
        streams: std::sync::Arc::clone(&streams),
    };

    // Text over a coding answers the whole read through the coding rather than
    // through the trait's `size`-then-read default, which would decode the
    // value once to measure it and again to read it.
    let text = crate::media::text::Text::new(crate::coding::Coding::new(source, Codec::Gzip));
    assert_eq!(text.read_all_bytes().unwrap(), PLAIN);
    assert_eq!(streams.load(Ordering::Relaxed), 1);
}
