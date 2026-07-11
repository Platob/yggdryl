//! Behavioural tests for the gzip compression codec and the shared codec traits.
//!
//! The whole file exercises `Gzip`, so it compiles only with the `gzip` feature.
#![cfg(feature = "gzip")]

use yggdryl_compression::{
    Compression, CompressionDecoder, CompressionEncoder, DecodeError, Decoder, EncodeError,
    Encoder, Gzip, TypedDecoder, TypedEncoder,
};

#[test]
fn gzip_round_trips_bytes() {
    let gzip = Gzip::new(6).unwrap();
    let original = b"the quick brown fox jumps over the lazy dog".repeat(16);

    let compressed = gzip.encode_byte_array(&original).unwrap();
    assert!(compressed.len() < original.len(), "should actually shrink");

    let restored = gzip.decode_byte_array(&compressed).unwrap();
    assert_eq!(restored, original);
}

#[test]
fn gzip_round_trips_empty_input() {
    let gzip = Gzip::default();
    let compressed = gzip.encode_byte_array(b"").unwrap();
    assert_eq!(gzip.decode_byte_array(&compressed).unwrap(), b"");
}

#[test]
fn typed_and_base_encode_agree() {
    let gzip = Gzip::new(9).unwrap();
    // `TypedEncoder<u8>::encode` is exactly the base `encode_byte_array`.
    assert_eq!(
        TypedEncoder::encode(&gzip, b"payload").unwrap(),
        gzip.encode_byte_array(b"payload").unwrap()
    );
    let encoded = TypedEncoder::encode(&gzip, b"payload").unwrap();
    assert_eq!(TypedDecoder::decode(&gzip, &encoded).unwrap(), b"payload");
}

#[test]
fn default_level_is_six() {
    assert_eq!(Gzip::default().level(), 6);
    assert_eq!(Gzip::DEFAULT_LEVEL, 6);
    assert_eq!(Gzip::new(6).unwrap().level(), 6);
}

#[test]
fn invalid_level_is_rejected() {
    assert_eq!(
        Gzip::new(10).unwrap_err(),
        EncodeError::InvalidLevel {
            level: 10,
            min: 0,
            max: 9
        }
    );
}

#[test]
fn zstd_round_trips_and_streams() {
    use yggdryl_compression::{ByteBuffer, IOBase, Whence, Zstd};

    let zstd = Zstd::default();
    assert_eq!(zstd.level(), 3);
    assert_eq!(zstd.name(), "zstd");

    let original = b"the quick brown fox ".repeat(200);
    let compressed = zstd.encode_byte_array(&original).unwrap();
    assert!(compressed.len() < original.len());
    assert_eq!(zstd.decode_byte_array(&compressed).unwrap(), original);

    // codec config round-trips through bytes
    assert_eq!(
        Zstd::deserialize_bytes(&Zstd::new(9).unwrap().serialize_bytes()).unwrap(),
        Zstd::new(9).unwrap()
    );

    // streaming between cursors
    let mut source = ByteBuffer::from_bytes(&original).byte_cursor();
    let mut packed = ByteBuffer::new().byte_cursor();
    zstd.compress_stream(&mut source, &mut packed).unwrap();
    assert_eq!(zstd.decode_byte_array(packed.as_bytes()).unwrap(), original);

    packed.byte_seek(0, Whence::Start).unwrap();
    let mut restored = ByteBuffer::new().byte_cursor();
    zstd.decompress_stream(&mut packed, &mut restored).unwrap();
    assert_eq!(restored.as_bytes(), original.as_slice());
}

#[test]
fn zstd_rejects_out_of_range_level() {
    use yggdryl_compression::Zstd;

    let (_min, max) = Zstd::level_range();
    assert!(Zstd::new(max + 1).is_err());
}

#[test]
fn compress_io_round_trips_with_both_codecs() {
    use yggdryl_compression::{ByteBuffer, CompressIO, IOBase, Whence, Zstd};

    for shrink in [true, false] {
        let original = if shrink {
            b"repeat ".repeat(200)
        } else {
            (0..137u8).collect() // small, incompressible-ish
        };
        // gzip and zstd both drive CompressIO the same way.
        let gzip = Gzip::default();
        let mut src = ByteBuffer::from_bytes(&original).byte_cursor();
        let mut compressed = src.compress(&gzip).unwrap();
        compressed.byte_seek(0, Whence::Start).unwrap();
        assert_eq!(compressed.decompress(&gzip).unwrap().as_bytes(), original);

        let zstd = Zstd::default();
        let mut src = ByteBuffer::from_bytes(&original).byte_cursor();
        let mut compressed = src.compress(&zstd).unwrap();
        compressed.byte_seek(0, Whence::Start).unwrap();
        assert_eq!(compressed.decompress(&zstd).unwrap().as_bytes(), original);
    }
}

#[test]
fn compress_io_edge_cases() {
    use yggdryl_compression::{ByteBuffer, CompressIO, DecodeError, IOBase, Whence};

    let gzip = Gzip::default();

    // Empty input round-trips.
    let mut empty = ByteBuffer::new().byte_cursor();
    let mut c = empty.compress(&gzip).unwrap();
    c.byte_seek(0, Whence::Start).unwrap();
    assert!(c.decompress(&gzip).unwrap().as_bytes().is_empty());

    // compress() only takes bytes from the current position onward.
    let mut cursor = ByteBuffer::from_bytes(b"skip-me-KEEP").byte_cursor();
    cursor.byte_seek(8, Whence::Start).unwrap(); // past "skip-me-"
    let mut tail = cursor.compress(&gzip).unwrap();
    tail.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(tail.decompress(&gzip).unwrap().as_bytes(), b"KEEP");

    // Decompressing garbage is a DecodeError.
    let mut garbage = ByteBuffer::from_bytes(b"not compressed").byte_cursor();
    assert!(matches!(
        garbage.decompress(&gzip).unwrap_err(),
        DecodeError::InvalidData(_)
    ));
}

#[test]
fn name_is_gzip() {
    assert_eq!(Gzip::new(1).unwrap().name(), "gzip");
}

#[test]
fn corrupt_input_is_invalid_data() {
    let err = Gzip::default()
        .decode_byte_array(b"not a gzip stream")
        .unwrap_err();
    assert!(matches!(err, DecodeError::InvalidData(_)), "got {err:?}");
}

#[test]
fn codec_config_round_trips_through_bytes() {
    for level in 0..=9 {
        let gzip = Gzip::new(level).unwrap();
        let bytes = gzip.serialize_bytes();
        assert_eq!(Gzip::deserialize_bytes(&bytes).unwrap(), gzip);
    }
}

#[test]
fn deserialize_bytes_validates() {
    assert!(matches!(
        Gzip::deserialize_bytes(&[]).unwrap_err(),
        DecodeError::InvalidData(_)
    ));
    assert!(matches!(
        Gzip::deserialize_bytes(&[6, 6]).unwrap_err(),
        DecodeError::InvalidData(_)
    ));
    assert!(matches!(
        Gzip::deserialize_bytes(&[42]).unwrap_err(),
        DecodeError::InvalidData(_)
    ));
}

/// Generic over the trait bounds to prove the marker traits are usable as bounds.
fn round_trip<C: CompressionEncoder + CompressionDecoder>(codec: &C, data: &[u8]) -> Vec<u8> {
    let encoded = codec.encode_byte_array(data).unwrap();
    codec.decode_byte_array(&encoded).unwrap()
}

#[test]
fn compression_marker_traits_are_usable_bounds() {
    assert_eq!(
        round_trip(&Gzip::default(), b"marker traits"),
        b"marker traits"
    );
}

#[test]
fn gzip_streams_between_byte_buffers() {
    use yggdryl_compression::{ByteBuffer, IOBase, Whence};

    let gzip = Gzip::new(6).unwrap();
    let original = b"stream me ".repeat(500);

    let mut source = ByteBuffer::from_bytes(&original).byte_cursor();
    let mut compressed = ByteBuffer::new().byte_cursor();
    let written = gzip.compress_stream(&mut source, &mut compressed).unwrap();
    // `byte_size` is the *remaining* bytes; the total written is the cursor's bytes.
    assert_eq!(written, compressed.as_bytes().len() as u64);
    assert!(
        compressed.as_bytes().len() < original.len(),
        "streaming should shrink"
    );

    // The streamed output is an ordinary gzip stream: the one-shot decoder reads it.
    assert_eq!(
        gzip.decode_byte_array(compressed.as_bytes()).unwrap(),
        original
    );

    // Round-trip fully through the streaming decoder.
    compressed.byte_seek(0, Whence::Start).unwrap();
    let mut restored = ByteBuffer::new().byte_cursor();
    let out = gzip
        .decompress_stream(&mut compressed, &mut restored)
        .unwrap();
    assert_eq!(out, original.len() as u64);
    assert_eq!(restored.as_bytes(), original.as_slice());
}

#[test]
fn stream_count_is_the_bytes_written_not_the_absolute_position() {
    // The streamed byte count comes from how far the sink cursor advances, so it must
    // report the bytes written *this call* — a delta — even when the sink already holds
    // content and is positioned past it (writes append after the prefix).
    use yggdryl_compression::{ByteBuffer, IOBase, Whence};

    let gzip = Gzip::new(6).unwrap();
    let original = b"append after me ".repeat(64);

    let mut source = ByteBuffer::from_bytes(&original).byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    let prefix = b"PREFIX-BYTES";
    sink.pwrite_byte_array(prefix, Whence::Start).unwrap(); // cursor now at prefix end

    let written = gzip.compress_stream(&mut source, &mut sink).unwrap();

    // The count is the compressed size (delta), strictly less than the total sink length.
    assert!(written > 0);
    assert_eq!(written, sink.as_bytes().len() as u64 - prefix.len() as u64);
    assert!((written as usize) < sink.as_bytes().len());

    // The sink is prefix followed by the gzip stream, which decodes to the original.
    assert_eq!(&sink.as_bytes()[..prefix.len()], prefix);
    assert_eq!(
        gzip.decode_byte_array(&sink.as_bytes()[prefix.len()..])
            .unwrap(),
        original
    );
}

#[test]
fn streaming_matches_one_shot_encoding() {
    use yggdryl_compression::{ByteBuffer, IOBase, Whence};

    let gzip = Gzip::new(9).unwrap();
    let data = b"identical output ".repeat(100);

    let mut source = ByteBuffer::from_bytes(&data).byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    gzip.compress_stream(&mut source, &mut sink).unwrap();

    assert_eq!(sink.as_bytes(), gzip.encode_byte_array(&data).unwrap());

    // A streamed decode of a one-shot encode also round-trips.
    let mut encoded = ByteBuffer::from_bytes(&gzip.encode_byte_array(&data).unwrap()).byte_cursor();
    let mut restored = ByteBuffer::new().byte_cursor();
    encoded.byte_seek(0, Whence::Start).unwrap();
    gzip.decompress_stream(&mut encoded, &mut restored).unwrap();
    assert_eq!(restored.as_bytes(), data.as_slice());
}

/// A trivial store-only codec (no compression) used to exercise the trait-default
/// `compress_stream` / `decompress_stream`, which `Gzip` overrides.
#[derive(Default)]
struct Store;

impl Encoder for Store {
    fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError> {
        Ok(bytes.to_vec())
    }
}

impl Decoder for Store {
    fn decode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
        Ok(bytes.to_vec())
    }
}

impl CompressionEncoder for Store {}
impl CompressionDecoder for Store {}

#[test]
fn default_stream_impl_round_trips() {
    use yggdryl_compression::{ByteBuffer, IOBase, Whence};

    let store = Store;
    let data = b"no compression, just IO plumbing".to_vec();

    let mut source = ByteBuffer::from_bytes(&data).byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    let written = store.compress_stream(&mut source, &mut sink).unwrap();
    assert_eq!(written, data.len() as u64);
    assert_eq!(sink.as_bytes(), data.as_slice());

    sink.byte_seek(0, Whence::Start).unwrap();
    let mut restored = ByteBuffer::new().byte_cursor();
    store.decompress_stream(&mut sink, &mut restored).unwrap();
    assert_eq!(restored.as_bytes(), data.as_slice());
}

#[test]
fn equality_agrees_with_serialize_bytes() {
    let a = Gzip::new(6).unwrap();
    let b = Gzip::default();
    assert_eq!(a, b);
    assert_eq!(a.serialize_bytes(), b.serialize_bytes());
    assert_ne!(a, Gzip::new(9).unwrap());
    assert_ne!(a.serialize_bytes(), Gzip::new(9).unwrap().serialize_bytes());
}

#[test]
fn hashable_as_set_key() {
    use std::collections::HashSet;

    let set: HashSet<Gzip> = [
        Gzip::new(1).unwrap(),
        Gzip::new(1).unwrap(),
        Gzip::new(9).unwrap(),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 2);
    assert!(set.contains(&Gzip::new(1).unwrap()));
}
