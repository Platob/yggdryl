//! `rust/src/valuestream.rs`: the value wire format, byte for byte.

use yggdryl::internals::valuestream::{UNCOMPRESSED, ZSTD};
use yggdryl::{COMPRESS_FROM, DataTypeId, Scalar, VALUE_STREAM_VERSION};

#[test]
fn a_number_is_its_identifier_and_its_bytes() {
    let bytes = Scalar::from(7_i32).into_value_bytes();
    assert_eq!(
        bytes,
        [VALUE_STREAM_VERSION, DataTypeId::Int32.as_u8(), 7, 0, 0, 0]
    );
    assert_eq!(
        Scalar::decode_value_bytes(&bytes).unwrap(),
        Scalar::from(7_i32)
    );
}

#[test]
fn a_text_states_its_compression_and_its_size() {
    let bytes = Scalar::from("abc").into_value_bytes();
    assert_eq!(
        bytes,
        [
            VALUE_STREAM_VERSION,
            DataTypeId::Utf8String.as_u8(),
            UNCOMPRESSED,
            3,
            b'a',
            b'b',
            b'c'
        ]
    );
    let long = "x".repeat(COMPRESS_FROM + 1);
    let bytes = Scalar::from(long.as_str()).into_value_bytes();
    assert_eq!(bytes[2], ZSTD);
    assert!(bytes.len() < 64, "{}", bytes.len());
    assert_eq!(
        Scalar::decode_value_bytes(&bytes).unwrap(),
        Scalar::from(long.as_str())
    );
}

#[test]
fn a_tree_streams_one_leaf_per_chunk_and_reads_back_whole() {
    let value = Scalar::from_struct([
        ("id", Scalar::from(1_i64)),
        (
            "tags",
            Scalar::from_sequence([Scalar::from("a"), Scalar::Null]),
        ),
    ])
    .unwrap();
    let chunks: Vec<Vec<u8>> = value.encode_value_stream_bytes().collect();
    assert_eq!(chunks.len(), 7, "{chunks:?}");
    assert_eq!(chunks.concat(), value.into_value_bytes());
    assert_eq!(Scalar::decode_value_stream_bytes(&chunks).unwrap(), value);
}

#[test]
fn the_refusals_name_the_byte() {
    let error = Scalar::decode_value_bytes(&[1, 0]).unwrap_err().to_string();
    assert!(error.contains("version 1"), "{error}");
    let error = Scalar::decode_value_bytes(&[0, 0x10])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("placeholder") && error.contains("integer"),
        "{error}"
    );
    let error = Scalar::decode_value_bytes(&[0, DataTypeId::Int32.as_u8(), 1])
        .unwrap_err()
        .to_string();
    assert!(error.contains("4 bytes announced"), "{error}");
    let error = Scalar::decode_value_bytes(&[0, 0, 0])
        .unwrap_err()
        .to_string();
    assert!(error.contains("1 bytes left"), "{error}");
}
