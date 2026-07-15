//! The fixed-size byte family (`io::fixed::binary` = `FixedBinary`, `io::fixed::string` =
//! `FixedUtf8`): runtime-`N` values that are all exactly `N` bytes. Focus: the width-enforcing
//! `push`, UTF-8 validation, zero-copy accessors, round-trips, and the dual category
//! classification (fixed-width **and** binary/utf8).

use yggdryl_core::io::fixed::{
    FixedBinaryScalar, FixedBinarySerie, FixedBinaryType, FixedUtf8Scalar, FixedUtf8Serie,
    FixedUtf8Type,
};
use yggdryl_core::io::{Bytes, DataType, DataTypeId, IOCursor, IoError};

#[test]
fn fixed_binary_descriptor_classifies_as_fixed_width_binary() {
    let dt = FixedBinaryType::new(16);
    assert_eq!(dt.name(), "fixed_binary");
    assert_eq!(dt.byte_width(), 16);
    assert_eq!(dt.type_id(), DataTypeId::FixedBinary);
    // Dual classification: it is BOTH fixed-width AND binary (the id-range predicates handle it).
    assert!(dt.is_fixed_width() && dt.is_binary());
    assert!(!dt.is_variable_length() && !dt.is_numeric() && !dt.is_utf8());
}

#[test]
fn fixed_binary_scalar_round_trips() {
    let s = FixedBinaryScalar::of(&[0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(s.width(), 4);
    assert_eq!(s.value_bytes(), Some(&[0xde, 0xad, 0xbe, 0xef][..]));
    let mut sink = Bytes::new();
    s.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(FixedBinaryScalar::read_from(&mut sink).unwrap(), s);

    // A null keeps its declared width.
    let n = FixedBinaryScalar::null(4);
    assert!(n.is_null());
    let mut sink = Bytes::new();
    n.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(FixedBinaryScalar::read_from(&mut sink).unwrap(), n);
}

#[test]
fn fixed_binary_serie_enforces_width_and_round_trips() {
    let mut col = FixedBinarySerie::new(2);
    col.push(Some(&[1, 2])).unwrap();
    col.push(None).unwrap();
    col.push(Some(&[3, 4])).unwrap();
    assert_eq!(col.len(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get_bytes(0), Some(&[1, 2][..]));
    assert_eq!(col.get_bytes(1), None);
    assert_eq!(col.get_bytes(2), Some(&[3, 4][..]));

    // Wrong width is rejected.
    let err = col.push(Some(&[9, 9, 9])).unwrap_err();
    assert!(matches!(err, IoError::CorruptLength { len: 3, width: 2 }));

    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(FixedBinarySerie::read_from(&mut sink).unwrap(), col);
}

#[test]
fn fixed_utf8_validates_and_reads_back_as_str() {
    let dt = FixedUtf8Type::new(3);
    assert!(dt.is_fixed_width() && dt.is_utf8());
    assert_eq!(dt.type_id(), DataTypeId::FixedUtf8);

    let s = FixedUtf8Scalar::of("abc");
    assert_eq!(s.as_str(), Some("abc"));

    let mut col = FixedUtf8Serie::new(2);
    col.push(Some("ab".as_bytes())).unwrap();
    col.push(None).unwrap();
    col.push(Some("cd".as_bytes())).unwrap();
    assert_eq!(col.get_str(0), Some("ab"));
    assert_eq!(col.get_str(1), None);
    assert_eq!(col.get_str(2), Some("cd"));

    // Invalid UTF-8 of the right width is still rejected.
    let err = col.push(Some(&[0xff, 0xfe])).unwrap_err();
    assert!(matches!(err, IoError::InvalidUtf8 { .. }));

    // Round-trip preserves the UTF-8 values.
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    let back = FixedUtf8Serie::read_from(&mut sink).unwrap();
    assert_eq!(back.get_str(2), Some("cd"));
}

#[test]
fn fixed_utf8_read_validates_each_slot_not_just_the_blob() {
    // A hostile frame whose 2 one-byte slots split a multi-byte code point (`é` = C3 A9): the
    // whole blob is valid UTF-8, but each slot is not — read must reject it per-slot, not accept
    // it and later silently return "".
    let mut frame = Bytes::new();
    frame.write_all(&2u64.to_le_bytes()).unwrap(); // len = 2
    frame.write_all(&1u64.to_le_bytes()).unwrap(); // width = 1
    frame.write_all(&[0u8]).unwrap(); // flags: no validity
    frame.write_all(&[0xC3, 0xA9]).unwrap(); // two slots splitting one code point
    frame.rewind();
    assert!(matches!(
        FixedUtf8Serie::read_from(&mut frame).unwrap_err(),
        IoError::InvalidUtf8 { .. }
    ));
}

#[cfg(feature = "arrow")]
#[test]
fn fixed_size_maps_to_arrow_fixed_size_binary() {
    use arrow_schema::DataType as A;
    assert_eq!(FixedBinaryType::new(16).to_arrow(), A::FixedSizeBinary(16));
    // Arrow has no fixed-size UTF-8, so it maps to FixedSizeBinary(N) too.
    assert_eq!(FixedUtf8Type::new(4).to_arrow(), A::FixedSizeBinary(4));
}
