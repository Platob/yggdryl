//! `rust/src/record_header.rs`: one typed naming policy across record media.

use yggdryl::{Error, RecordHeader, Scalar, from_json_scalar};

#[test]
fn header_intake_is_wide_only_at_the_boundary() {
    for (input, expected) in [
        (Scalar::from(true), RecordHeader::Source),
        (Scalar::from(false), RecordHeader::None),
        (Scalar::Null, RecordHeader::None),
        (Scalar::from("source"), RecordHeader::Source),
        (Scalar::from("none"), RecordHeader::None),
        (Scalar::from("infer"), RecordHeader::Infer),
        (Scalar::from(1_i64), RecordHeader::Rows(1)),
        (Scalar::from(2_i64), RecordHeader::Rows(2)),
        (
            Scalar::from(u64::from(u32::MAX)),
            RecordHeader::Rows(u32::MAX),
        ),
    ] {
        assert_eq!(RecordHeader::from_scalar(&input).unwrap(), expected);
    }
    assert_eq!(RecordHeader::from(true), RecordHeader::Source);
    assert_eq!(RecordHeader::from(false), RecordHeader::None);
    assert_eq!(RecordHeader::default(), RecordHeader::Source);
}

#[test]
fn malformed_header_spelling_and_nonpositive_or_oversized_rows_are_located() {
    for literal in [
        "0",
        "-1",
        "4294967296",
        "[]",
        "{}",
        "\"\"",
        "\"Source\"",
        "\"true\"",
        "\"rows\"",
    ] {
        let value = from_json_scalar(literal).unwrap();
        let error = RecordHeader::from_scalar(&value).unwrap_err();
        let Error::InvalidRecord { path, reason } = error else {
            panic!("expected a located header refusal for {literal}, got {error}");
        };
        assert_eq!(path.as_str(), "$.header");
        assert!(reason.contains("expected"), "{reason}");
    }
}
