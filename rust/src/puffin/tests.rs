//! Unit tests for the Puffin container module.

use crate::io::{Buffer, IOBase};
use crate::{Limits, Value};

use super::bitmap::{Cursor, decode_portable_32, decode_portable_64, encode_portable_32};
use super::format::MAGIC;
use super::{
    APACHE_DATASKETCHES_THETA_V1, BlobMetadata, DELETION_VECTOR_V1, FileMetadata, Puffin,
    read_deletion_vector, read_deletion_vector_with_limits, write_deletion_vector,
};

/// Decode one bare 32-bit Roaring serialization completely.
fn decode_32(bytes: &[u8]) -> Vec<u64> {
    let mut cursor = Cursor::over(bytes);
    let mut positions = Vec::new();
    decode_portable_32(&mut cursor, 0, usize::MAX, &mut positions).unwrap();
    assert_eq!(cursor.consumed(), bytes.len(), "trailing bytes");
    positions
}

// ---------------------------------------------------------------- bitmap ---

#[test]
fn an_empty_vector_round_trips() {
    let blob = write_deletion_vector(&[]).unwrap();
    // Length prefix, magic, an eight-byte zero bucket count, CRC-32.
    assert_eq!(blob.len(), 4 + 4 + 8 + 4);
    assert_eq!(read_deletion_vector(&blob).unwrap(), Vec::<u64>::new());
}

#[test]
fn a_single_position_round_trips() {
    for position in [0_u64, 1, 65535, 65536, 4_294_967_295, 4_294_967_296] {
        let blob = write_deletion_vector(&[position]).unwrap();
        assert_eq!(read_deletion_vector(&blob).unwrap(), vec![position]);
    }
}

#[test]
fn the_maximum_position_round_trips() {
    let maximum = u64::try_from(i64::MAX).unwrap();
    let blob = write_deletion_vector(&[0, maximum]).unwrap();
    assert_eq!(read_deletion_vector(&blob).unwrap(), vec![0, maximum]);
}

#[test]
fn a_position_with_the_sign_bit_set_is_refused() {
    let message = write_deletion_vector(&[1 << 63]).unwrap_err().to_string();
    assert!(message.contains("most significant bit"), "{message}");
    assert!(message.contains("9223372036854775808"), "{message}");
}

#[test]
fn unsorted_positions_are_refused() {
    let message = write_deletion_vector(&[5, 5]).unwrap_err().to_string();
    assert!(
        message.contains("expected strictly increasing positions, got 5 after 5"),
        "{message}"
    );
}

#[test]
fn sparse_positions_choose_an_array_container() {
    // 100 scattered values in one 16-bit container: no run pays off.
    let values: Vec<u32> = (0..100).map(|index| index * 7).collect();
    let mut encoded = Vec::new();
    encode_portable_32(&values, true, &mut encoded);
    // Cookie + count + descriptive header + offsets + 2 bytes per value.
    assert_eq!(encoded.len(), 4 + 4 + 4 + 4 + 2 * values.len());
    assert_eq!(u32::from_le_bytes(encoded[0..4].try_into().unwrap()), 12346);
    let expected: Vec<u64> = values.iter().map(|value| u64::from(*value)).collect();
    assert_eq!(decode_32(&encoded), expected);
}

#[test]
fn dense_positions_choose_a_bitset_container() {
    // Cardinality 32768 with no adjacency: above 4096, runs pay nothing.
    let values: Vec<u32> = (0..32768).map(|index| index * 2).collect();
    let mut encoded = Vec::new();
    encode_portable_32(&values, true, &mut encoded);
    assert_eq!(encoded.len(), 4 + 4 + 4 + 4 + 8192);
    let expected: Vec<u64> = values.iter().map(|value| u64::from(*value)).collect();
    assert_eq!(decode_32(&encoded), expected);
}

#[test]
fn run_heavy_positions_choose_run_containers() {
    // One contiguous run of 10000 values: 6 bytes of container body.
    let values: Vec<u32> = (100..10100).collect();
    let mut encoded = Vec::new();
    encode_portable_32(&values, true, &mut encoded);
    // Run cookie + run bitset + descriptive header + one run, no offsets
    // below the four-container threshold.
    assert_eq!(encoded.len(), 4 + 1 + 4 + 2 + 4);
    assert_eq!(
        u32::from_le_bytes(encoded[0..4].try_into().unwrap()) & 0xFFFF,
        12347
    );
    let expected: Vec<u64> = values.iter().map(|value| u64::from(*value)).collect();
    assert_eq!(decode_32(&encoded), expected);
}

#[test]
fn positions_spanning_several_high_keys_round_trip() {
    let positions = vec![
        0,
        65_536,
        u64::from(u32::MAX),
        1 << 32,
        (1 << 32) + 1,
        (5 << 32) + 123_456,
        (5 << 32) + 123_457,
        1 << 40,
    ];
    let blob = write_deletion_vector(&positions).unwrap();
    assert_eq!(read_deletion_vector(&blob).unwrap(), positions);
    // The vector names four buckets: high keys 0, 1, 5, and 256.
    let count = u64::from_le_bytes(blob[8..16].try_into().unwrap());
    assert_eq!(count, 4);
}

#[test]
fn official_interop_vectors_reencode_byte_for_byte() {
    // The value set documented in RoaringFormatSpec's testdata/README.md
    // (https://github.com/RoaringBitmap/RoaringFormatSpec): multiples of 1000
    // in [0, 100000), 3*k for k in [100000, 200000), and all of
    // [700000, 800000). The official files' byte lengths and CRC-32s are
    // pinned here, so a byte-identical re-encoding proves the codec against
    // Java's serialization without committing the binaries:
    //   testdata/bitmapwithruns.bin    48056 bytes, CRC-32 0x1052a898
    //   testdata/bitmapwithoutruns.bin 72616 bytes, CRC-32 0xef6dc26a
    let mut values: Vec<u32> = (0..100_000).step_by(1000).collect();
    values.extend((100_000..200_000).map(|k| 3 * k));
    values.extend(700_000..800_000);
    values.sort_unstable();
    values.dedup();

    let checksum = |bytes: &[u8]| {
        let mut crc = flate2::Crc::new();
        crc.update(bytes);
        crc.sum()
    };
    let expected: Vec<u64> = values.iter().map(|value| u64::from(*value)).collect();

    let mut with_runs = Vec::new();
    encode_portable_32(&values, true, &mut with_runs);
    assert_eq!(with_runs.len(), 48056);
    assert_eq!(checksum(&with_runs), 0x1052_a898);
    assert_eq!(decode_32(&with_runs), expected);

    let mut without_runs = Vec::new();
    encode_portable_32(&values, false, &mut without_runs);
    assert_eq!(without_runs.len(), 72616);
    assert_eq!(checksum(&without_runs), 0xef6d_c26a);
    assert_eq!(decode_32(&without_runs), expected);
}

#[test]
fn a_corrupted_crc_is_reported() {
    let mut blob = write_deletion_vector(&[1, 2, 3]).unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 0xFF;
    let message = read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(message.contains("expected CRC-32"), "{message}");
}

#[test]
fn a_corrupted_magic_is_reported() {
    let mut blob = write_deletion_vector(&[1]).unwrap();
    blob[4] = 0x00;
    let message = read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(
        message.contains("expected deletion-vector magic d1d33964"),
        "{message}"
    );
    assert!(message.contains("byte 4"), "{message}");
}

#[test]
fn a_wrong_combined_length_is_reported() {
    let mut blob = write_deletion_vector(&[1]).unwrap();
    blob[3] ^= 0x01;
    let message = read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(
        message.contains("expected a combined magic-and-vector length"),
        "{message}"
    );
}

#[test]
fn a_truncated_vector_is_reported() {
    let message = read_deletion_vector(&[0, 0, 0]).unwrap_err().to_string();
    assert!(
        message.contains("expected at least 12 bytes of deletion-vector framing, got 3"),
        "{message}"
    );
}

#[test]
fn an_unknown_roaring_cookie_is_reported() {
    // Valid framing around a vector whose inner bitmap opens with a cookie
    // that is neither serialization.
    let mut vector = Vec::new();
    vector.extend_from_slice(&1_u64.to_le_bytes());
    vector.extend_from_slice(&0_u32.to_le_bytes());
    vector.extend_from_slice(&99_u32.to_le_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(&super::bitmap::DELETION_VECTOR_MAGIC);
    crc.update(&vector);
    let mut blob = Vec::new();
    blob.extend_from_slice(&u32::try_from(4 + vector.len()).unwrap().to_be_bytes());
    blob.extend_from_slice(&super::bitmap::DELETION_VECTOR_MAGIC);
    blob.extend_from_slice(&vector);
    blob.extend_from_slice(&crc.sum().to_be_bytes());
    let message = read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(
        message.contains("expected Roaring cookie 12346 or 12347, got 99"),
        "{message}"
    );
}

#[test]
fn a_vector_beyond_the_byte_budget_is_refused() {
    // 65536 contiguous positions decode fine under default limits but exceed
    // a 1 KiB budget: the run form is tiny, the decoded set is not.
    let positions: Vec<u64> = (0..65536).collect();
    let blob = write_deletion_vector(&positions).unwrap();
    assert_eq!(read_deletion_vector(&blob).unwrap().len(), 65536);
    let limits = Limits::new(128, 1024, 1_000_000, 1_024);
    let message = read_deletion_vector_with_limits(&blob, limits)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("expected at most 128 decoded positions"),
        "{message}"
    );
}

#[test]
fn trailing_bytes_after_the_vector_are_reported() {
    let mut vector = 0_u64.to_le_bytes().to_vec();
    vector.push(0xAA);
    let mut crc = flate2::Crc::new();
    crc.update(&super::bitmap::DELETION_VECTOR_MAGIC);
    crc.update(&vector);
    let mut blob = Vec::new();
    blob.extend_from_slice(&u32::try_from(4 + vector.len()).unwrap().to_be_bytes());
    blob.extend_from_slice(&super::bitmap::DELETION_VECTOR_MAGIC);
    blob.extend_from_slice(&vector);
    blob.extend_from_slice(&crc.sum().to_be_bytes());
    let message = read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(message.contains("got 1 trailing bytes"), "{message}");
}

#[test]
fn sixty_four_bit_buckets_wrap_the_thirty_two_bit_codec() {
    // Hand-build a portable 64-bit stream and decode it: one bucket under
    // key 2 whose inner bitmap is the codec's own encoding.
    let mut stream = Vec::new();
    stream.extend_from_slice(&1_u64.to_le_bytes());
    stream.extend_from_slice(&2_u32.to_le_bytes());
    encode_portable_32(&[10, 11, 12], true, &mut stream);
    let mut cursor = Cursor::over(&stream);
    let positions = decode_portable_64(&mut cursor, Limits::default()).unwrap();
    assert_eq!(
        positions,
        vec![(2 << 32) | 10, (2 << 32) | 11, (2 << 32) | 12]
    );
}

// ------------------------------------------------------------- container ---

#[test]
fn an_empty_handle_reads_as_an_empty_footer() {
    let file = Puffin::new(Buffer::new());
    let footer = file.footer().unwrap();
    assert!(footer.blobs.is_empty());
    assert!(footer.properties.is_empty());
}

#[test]
fn appended_blobs_round_trip_through_the_footer() {
    let mut file = Puffin::new(Buffer::new());
    file.set_file_property("created-by", "yggdryl tests")
        .unwrap();
    let raw = file
        .append_blob(
            BlobMetadata::new("some-index", vec![1, 2], 77, 3),
            b"raw payload",
        )
        .unwrap();
    let packed = file
        .append_blob(
            BlobMetadata::new("some-index", vec![3], 77, 3).with_compression_codec("zstd"),
            b"a compressible payload a compressible payload",
        )
        .unwrap();
    file.finish().unwrap();

    let reread = Puffin::new(file.into_handle());
    let footer = reread.footer().unwrap();
    assert_eq!(footer.get_property("created-by"), Some("yggdryl tests"));
    assert_eq!(footer.blobs.len(), 2);
    assert_eq!(footer.blobs[0], raw);
    assert_eq!(footer.blobs[1], packed);
    assert_eq!(reread.read_blob(&footer.blobs[0]).unwrap(), b"raw payload");
    assert_eq!(
        reread.read_blob(&footer.blobs[1]).unwrap(),
        b"a compressible payload a compressible payload"
    );
    assert_eq!(footer.blobs[0].snapshot_id(), 77);
    assert_eq!(footer.blobs[0].sequence_number(), 3);
    assert_eq!(footer.blobs[0].fields(), &[1, 2]);
}

#[test]
fn the_file_magic_frames_the_head_and_the_footer() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();
    // The wrapping handle mirrors bytes, so the raw layout is reachable
    // without unwrapping.
    let bytes = file.read_all_bytes().unwrap();
    assert_eq!(&bytes[0..4], &MAGIC);
    assert_eq!(&bytes[bytes.len() - 4..], &MAGIC);
    let payload_size =
        u32::from_le_bytes(bytes[bytes.len() - 12..bytes.len() - 8].try_into().unwrap());
    let footer_magic = bytes.len() - 16 - payload_size as usize;
    assert_eq!(&bytes[footer_magic..footer_magic + 4], &MAGIC);
    // The four flag bytes are zero: the footer is written uncompressed.
    assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], &[0, 0, 0, 0]);
}

#[test]
fn a_missing_head_magic_is_reported() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();
    let mut bytes = file.into_handle().read_all_bytes().unwrap();
    bytes[0] = b'Q';
    let broken = Puffin::new(Buffer::from_bytes(bytes));
    let message = broken.footer().unwrap_err().to_string();
    assert!(
        message.contains("expected magic \"PFA1\" at the head of the file"),
        "{message}"
    );
}

#[test]
fn a_compressed_footer_is_refused_naming_lz4() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();
    let mut bytes = file.into_handle().read_all_bytes().unwrap();
    let flag = bytes.len() - 8;
    bytes[flag] |= 0x01;
    let compressed = Puffin::new(Buffer::from_bytes(bytes));
    let message = compressed.footer().unwrap_err().to_string();
    assert!(message.contains("\"lz4\""), "{message}");
    assert!(
        message.contains("expected an uncompressed footer"),
        "{message}"
    );
}

#[test]
fn a_reserved_footer_flag_is_reported() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();
    let mut bytes = file.into_handle().read_all_bytes().unwrap();
    let flag = bytes.len() - 7;
    bytes[flag] = 0x80;
    let unknown = Puffin::new(Buffer::from_bytes(bytes));
    let message = unknown.footer().unwrap_err().to_string();
    assert!(
        message.contains("expected zero reserved footer flags"),
        "{message}"
    );
}

#[test]
fn a_truncated_file_is_reported() {
    let truncated = Puffin::new(Buffer::from_bytes(b"PFA1abc".to_vec()));
    let message = truncated.footer().unwrap_err().to_string();
    assert!(
        message.contains("expected a Puffin file of at least 20 bytes, got 7"),
        "{message}"
    );
}

#[test]
fn an_lz4_blob_is_refused_by_name() {
    let file = Puffin::new(Buffer::new());
    let blob = BlobMetadata::new("some-index", vec![], 1, 1).with_compression_codec("lz4");
    let message = file.read_blob(&blob).unwrap_err().to_string();
    assert!(
        message.contains(
            "expected a Puffin compression codec this build implements (zstd), got \"lz4\""
        ),
        "{message}"
    );
}

#[test]
fn an_lz4_blob_is_refused_on_write_too() {
    let mut file = Puffin::new(Buffer::new());
    let blob = BlobMetadata::new("some-index", vec![], 1, 1).with_compression_codec("lz4");
    let message = file.append_blob(blob, b"x").unwrap_err().to_string();
    assert!(message.contains("got \"lz4\""), "{message}");
    // The refusal happened before anything was staged or written.
    assert!(file.handle().is_empty());
}

#[test]
fn deletion_vector_blobs_round_trip() {
    let positions: Vec<u64> = vec![0, 5, 6, 7, 1_000_000, (3 << 32) + 4];
    let mut file = Puffin::new(Buffer::new());
    let blob = file
        .append_deletion_vector("data/part-000.parquet", &positions)
        .unwrap();
    file.finish().unwrap();

    assert_eq!(blob.blob_type(), DELETION_VECTOR_V1);
    assert_eq!(blob.snapshot_id(), -1);
    assert_eq!(blob.sequence_number(), -1);
    assert_eq!(blob.compression_codec(), None);
    assert_eq!(
        blob.get_property("referenced-data-file"),
        Some("data/part-000.parquet")
    );
    assert_eq!(blob.get_property("cardinality"), Some("6"));

    let reread = Puffin::new(file.into_handle());
    let footer = reread.footer().unwrap();
    assert_eq!(footer.blobs.len(), 1);
    assert_eq!(
        reread.read_deletion_vector(&footer.blobs[0]).unwrap(),
        positions
    );
}

#[test]
fn a_deletion_vector_with_a_codec_is_refused() {
    let mut file = Puffin::new(Buffer::new());
    let blob = BlobMetadata::deletion_vector("data/a.parquet", 1).with_compression_codec("zstd");
    let message = file.append_blob(blob, b"x").unwrap_err().to_string();
    assert!(
        message.contains(
            "expected an uncompressed deletion-vector-v1 blob, got compression-codec \"zstd\""
        ),
        "{message}"
    );
}

#[test]
fn a_deletion_vector_without_its_properties_is_refused() {
    let file = Puffin::new(Buffer::new());
    let blob = BlobMetadata::new(DELETION_VECTOR_V1, vec![], -1, -1);
    let message = file.read_deletion_vector(&blob).unwrap_err().to_string();
    assert!(
        message.contains("expected a \"referenced-data-file\" property"),
        "{message}"
    );
}

#[test]
fn a_deletion_vector_with_a_snapshot_id_is_refused() {
    let file = Puffin::new(Buffer::new());
    let blob = BlobMetadata::deletion_vector("data/a.parquet", 1);
    let mut wrong = BlobMetadata::new(DELETION_VECTOR_V1, vec![], 42, -1);
    for (key, value) in blob.properties() {
        wrong = wrong.with_property(key.clone(), value.clone());
    }
    let message = file.read_deletion_vector(&wrong).unwrap_err().to_string();
    assert!(
        message.contains("expected snapshot-id -1 on a deletion-vector-v1 blob, got 42"),
        "{message}"
    );
}

#[test]
fn a_wrong_cardinality_property_is_refused() {
    let mut file = Puffin::new(Buffer::new());
    let blob = file
        .append_deletion_vector("data/a.parquet", &[1, 2, 3])
        .unwrap();
    file.finish().unwrap();
    let wrong = blob.with_property("cardinality", "2");
    let message = file.read_deletion_vector(&wrong).unwrap_err().to_string();
    assert!(
        message.contains("expected the declared cardinality 2, got 3 positions"),
        "{message}"
    );
}

#[test]
fn a_sketch_blob_is_preserved_across_a_rewrite() {
    // A file holding a Theta sketch gains a deletion vector: the sketch's
    // bytes and metadata survive untouched, and are never re-encoded.
    let sketch_bytes = b"opaque theta sketch bytes";
    let mut file = Puffin::new(Buffer::new());
    let sketch = file
        .append_blob(
            BlobMetadata::new(APACHE_DATASKETCHES_THETA_V1, vec![4], 11, 2)
                .with_property("ndv", "123"),
            sketch_bytes,
        )
        .unwrap();
    file.finish().unwrap();

    let mut rewriter = Puffin::new(file.into_handle());
    rewriter
        .append_deletion_vector("data/a.parquet", &[9])
        .unwrap();
    rewriter.finish().unwrap();

    let reread = Puffin::new(rewriter.into_handle());
    let footer = reread.footer().unwrap();
    assert_eq!(footer.blobs.len(), 2);
    assert_eq!(footer.blobs[0], sketch);
    assert_eq!(footer.blobs[0].get_property("ndv"), Some("123"));
    assert_eq!(reread.read_blob(&footer.blobs[0]).unwrap(), sketch_bytes);
}

#[test]
fn open_caches_the_footer_and_close_releases_it() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();

    let mut handle = Puffin::new(file.into_handle());
    assert!(!handle.opened());
    handle.open().unwrap();
    assert!(handle.opened());
    assert_eq!(handle.footer().unwrap().blobs.len(), 1);
    handle.close().unwrap();
    assert!(!handle.opened());
}

#[test]
fn close_publishes_staged_blobs() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    // No finish: close is the publish.
    file.close().unwrap();
    let reread = Puffin::new(file.into_handle());
    assert_eq!(reread.footer().unwrap().blobs.len(), 1);
}

#[test]
fn remove_drops_staged_writes_so_a_flush_cannot_resurrect_them() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.remove(false).unwrap();
    file.flush().unwrap();
    assert!(file.handle().is_empty());
}

#[test]
fn the_footer_payload_document_round_trips() {
    let metadata = FileMetadata {
        blobs: vec![
            BlobMetadata::new("some-index", vec![7], 5, 2).with_property("k", "v"),
            BlobMetadata::deletion_vector("data/a.parquet", 3).with_property("x", "y"),
        ],
        properties: vec![("created-by".into(), "tests".into())],
    };
    let document = metadata.to_json().unwrap();
    // The payload crosses as plain JSON through the shared codec.
    let bytes = crate::json::to_vec(&document).unwrap();
    let reparsed = crate::json::from_slice(&bytes).unwrap();
    assert_eq!(FileMetadata::from_json(&reparsed).unwrap(), metadata);
}

#[test]
fn a_blob_entry_error_is_located_by_index() {
    let document = crate::json::from_str(
        r#"{"blobs":[
            {"type":"a","fields":[],"snapshot-id":1,"sequence-number":1,"offset":4,"length":1},
            {"type":"b","fields":[],"snapshot-id":1,"sequence-number":1,"offset":-4,"length":1}
        ]}"#,
    )
    .unwrap();
    let message = FileMetadata::from_json(&document).unwrap_err().to_string();
    assert!(
        message.contains("blobs[1]: expected a non-negative blob \"offset\", got -4"),
        "{message}"
    );
}

#[test]
fn a_footer_without_a_blobs_list_is_reported() {
    let document = crate::json::from_str(r#"{"properties":{}}"#).unwrap();
    let message = FileMetadata::from_json(&document).unwrap_err().to_string();
    assert!(
        message.contains("expected a \"blobs\" list in the footer payload"),
        "{message}"
    );
}

#[test]
fn a_non_string_property_is_reported() {
    let document = crate::json::from_str(r#"{"blobs":[],"properties":{"n":1}}"#).unwrap();
    let message = FileMetadata::from_json(&document).unwrap_err().to_string();
    assert!(message.contains("expected string"), "{message}");
}

#[test]
fn blob_metadata_json_uses_the_spec_key_spellings() {
    let blob = BlobMetadata::new("some-index", vec![1], 2, 3).with_compression_codec("zstd");
    let document = blob.to_json().unwrap();
    for key in [
        "type",
        "fields",
        "snapshot-id",
        "sequence-number",
        "offset",
        "length",
        "compression-codec",
    ] {
        assert!(document.get_key_str(key).is_some(), "missing {key}");
    }
    assert_eq!(
        document
            .get_key_str("compression-codec")
            .and_then(Value::as_str),
        Some("zstd")
    );
}

#[test]
fn the_wrapping_handle_mirrors_bytes() {
    let mut file = Puffin::new(Buffer::new());
    file.append_blob(BlobMetadata::new("some-index", vec![], 1, 1), b"x")
        .unwrap();
    file.finish().unwrap();
    let raw = file.handle().read_all_bytes().unwrap();
    assert_eq!(file.read_all_bytes().unwrap(), raw);
    assert_eq!(file.size(), raw.len() as u64);
    // A blob container holds one byte value, not rows.
    assert!(file.is_atomic());
    assert!(!file.is_tabular());
}
