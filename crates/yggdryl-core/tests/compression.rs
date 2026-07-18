//! Functional tests for the [`Compression`](yggdryl_core::compression) codecs and their
//! [`IOBase`] integration — round-trips, edge cases, the recursive magic inference, and the
//! zero-copy `compress_into` / `decompress_into` path. **Feature-gated**: run with
//! `cargo test -p yggdryl-core --features compression --test compression`.
#![cfg(feature = "compression")]

use yggdryl_core::compression::{codec_for, codec_for_mime, Compression, Gzip, Lzma, Zlib, Zstd};
use yggdryl_core::io::memory::{Heap, IOBase, IoError};
use yggdryl_core::mediatype::MediaType;
use yggdryl_core::mimetype::MimeType;

/// A compressible corpus (repetitive, so every codec shrinks it well).
fn corpus() -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..4096u32 {
        data.extend_from_slice(format!("record {i:08} the quick brown fox; ").as_bytes());
    }
    data
}

#[test]
fn every_codec_round_trips_and_shrinks() {
    let data = corpus();
    let codecs: Vec<Box<dyn Compression>> = vec![
        Box::new(Gzip::new()),
        Box::new(Zlib::new()),
        Box::new(Zstd::new()),
        Box::new(Lzma::new()),
    ];
    for codec in &codecs {
        let packed = codec.compress(&data).unwrap();
        assert!(packed.len() < data.len(), "{} should shrink", codec.name());
        assert_eq!(
            codec.decompress(&packed).unwrap(),
            data,
            "{} round-trip",
            codec.name()
        );

        // Empty input round-trips too.
        let empty = codec.compress(b"").unwrap();
        assert_eq!(codec.decompress(&empty).unwrap(), b"");

        // Corrupt input is a guided error, not a panic.
        let err = codec.decompress(b"not compressed at all").unwrap_err();
        assert!(matches!(
            err,
            IoError::Compression {
                op: "decompress",
                ..
            }
        ));
    }
}

#[test]
fn codec_resolves_from_mime_and_essence() {
    assert_eq!(codec_for("application/gzip").unwrap().name(), "gzip");
    assert_eq!(codec_for("application/zstd").unwrap().name(), "zstd");
    assert_eq!(codec_for("application/x-xz").unwrap().name(), "xz");
    assert!(codec_for("application/json").is_none()); // not a compression
    assert_eq!(
        codec_for_mime(&MimeType::from_extension("zst").unwrap())
            .unwrap()
            .name(),
        "zstd"
    );
}

#[test]
fn iobase_compress_into_decompress_into_round_trip() {
    let data = corpus();
    let src = Heap::from_slice(&data);
    let codec = Gzip::new();

    // compress_into a fresh sink, then decompress_into another — end-to-end through IOBase.
    let mut packed = Heap::new();
    let n = src.compress_into(&codec, &mut packed).unwrap();
    assert_eq!(n, packed.byte_size());
    assert!(packed.byte_size() < src.byte_size());

    let mut restored = Heap::new();
    packed.decompress_into(&codec, &mut restored).unwrap();
    assert_eq!(restored.as_slice(), data.as_slice());

    // Zero-copy read side: a Heap exposes its bytes, so no intermediate copy is made.
    assert!(src.as_bytes().is_some());
}

#[test]
fn iobase_decompress_infers_the_codec_from_the_media_type() {
    let data = corpus();
    let packed = Zstd::new().compress(&data).unwrap();

    // A Heap addressed by a .zst name infers the zstd codec from its media type.
    let mut src = Heap::from_slice(&packed);
    src.headers_mut().set_content_type("application/zstd");
    assert!(src.compression().is_some());
    assert_eq!(src.decompress().unwrap(), data);

    // A non-compression source gives a guided error naming the fix.
    let plain = Heap::from_slice(b"hello");
    let err = plain.decompress().unwrap_err();
    assert!(matches!(
        err,
        IoError::Compression {
            op: "decompress",
            ..
        }
    ));
    assert!(err.to_string().contains("compression"));
}

#[test]
fn recursive_magic_inference_peels_compression_layers() {
    // An inner payload that carries a magic signature (PDF), so peeling the gzip layer finds
    // it (a magicless inner like JSON would infer octet-stream).
    let inner = b"%PDF-1.7\nfake pdf body ".repeat(200);
    let gzipped = Gzip::new().compress(&inner).unwrap();

    // The compressed bytes' magic is gzip; peeling the layer finds the inner pdf.
    let media = MediaType::infer_from_head(&gzipped, None);
    assert_eq!(media.primary().unwrap().essence(), "application/gzip");
    assert!(media.essences().contains(&"application/pdf"));

    // Through IOBase (positioned read of the head; the cursor is not moved).
    let mut src = Heap::from_slice(&gzipped);
    src.set_position(5); // put the cursor somewhere
    let via_source = src.infer_media_type();
    assert_eq!(via_source.primary().unwrap().essence(), "application/gzip");
    assert_eq!(src.position(), 5, "infer must not seek the cursor");
    // Magic inference of the single primary type, cursor untouched.
    assert_eq!(src.infer_mime_type().essence(), "application/gzip");
    assert_eq!(src.position(), 5);
}

#[test]
fn iobase_compress_in_place_round_trips_and_syncs_headers() {
    // A .gz-addressed heap packs itself with the media-type codec, then unpacks back.
    let original = corpus();
    let mut heap = Heap::from_slice(&original);
    heap.set_headers(
        yggdryl_core::headers::Headers::new().with("Content-Type", "application/gzip"),
    );
    let packed_from = heap.byte_size();
    heap.compress_in_place(None).unwrap(); // codec defaults to the media-type codec (gzip)
    assert!(
        heap.byte_size() < packed_from,
        "compression should shrink the corpus"
    );
    assert_eq!(heap.headers().content_type(), Some("application/gzip"));

    heap.decompress_in_place().unwrap();
    assert_eq!(heap.as_slice(), &original[..]); // exact inverse
    assert_eq!(heap.byte_size(), packed_from);

    // compress_in_place on a NON-compression media type is a guided error naming the fix.
    let mut plain = Heap::from_slice(b"not compressible by type");
    plain.set_headers(yggdryl_core::headers::Headers::new().with("Content-Type", "text/plain"));
    let err = plain.compress_in_place(None).unwrap_err().to_string();
    assert!(err.contains("codec") || err.contains("compression"));
}
