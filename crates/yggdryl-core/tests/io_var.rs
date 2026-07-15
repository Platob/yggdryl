//! Tests for the `io::var` variable-length family — the `Utf8` / `Binary` byte suites built on
//! offsets + data + validity. Focus: the empty-string-vs-null distinction, arbitrary binary
//! bytes, UTF-8 validation (a guided error), zero-copy `&str` / `&[u8]` accessors, the category
//! drill-down predicates, value semantics (equality / hashing), and serialization round-trips
//! through the `IOCursor` byte sink.

use std::collections::HashSet;

use yggdryl_core::io::var::{
    Binary, BinaryScalar, BinarySerie, ByteField, ByteScalar, ByteType, Utf8, Utf8Field,
    Utf8Scalar, Utf8Serie, VarScalar, VarSerie,
};
use yggdryl_core::io::{Bytes, DataType, FieldType, IOCursor, IoError, ScalarType, SerieType};

// -------------------------------------------------------------------------------------
// Descriptors + the category drill-down (no `match` at the call site)
// -------------------------------------------------------------------------------------

#[test]
fn utf8_data_type_categorizes_as_variable_length_string() {
    let dt = <ByteType<Utf8>>::new();
    assert_eq!(dt.name(), "utf8");
    assert_eq!(dt.byte_width(), 4); // one 32-bit offset — the fixed portion
    assert!(dt.is_utf8());
    assert!(dt.is_variable_length());
    assert!(!dt.is_fixed_width());
    assert!(!dt.is_binary() && !dt.is_numeric() && !dt.is_integer());
    assert_eq!(ByteType::<Utf8>::NAME, "utf8");
}

#[test]
fn binary_data_type_categorizes_as_variable_length_opaque() {
    let dt = <ByteType<Binary>>::new();
    assert_eq!(dt.name(), "binary");
    assert!(dt.is_binary());
    assert!(dt.is_variable_length() && !dt.is_fixed_width());
    assert!(!dt.is_utf8());
}

#[test]
fn field_drills_down_over_its_category() {
    let f: Utf8Field = ByteField::new("city", true);
    assert_eq!(f.name(), "city");
    assert_eq!(f.type_name(), "utf8");
    assert_eq!(f.byte_width(), 4);
    assert!(f.nullable());
    assert!(f.is_utf8() && f.is_variable_length());

    // The erased runtime field keeps the category, so `is_*` still works without the descriptor.
    let erased = f.erase();
    assert_eq!(erased.type_name(), "utf8");
    assert!(erased.is_variable_length());
    assert!(!erased.is_fixed_width());
}

// -------------------------------------------------------------------------------------
// Scalars: null-object, value semantics, UTF-8 validation, round-trip
// -------------------------------------------------------------------------------------

#[test]
fn utf8_scalar_present_and_null() {
    let s = Utf8Scalar::of("héllo"); // multi-byte code points
    assert!(!s.is_null());
    assert_eq!(s.as_str(), Some("héllo"));
    assert_eq!(VarScalar::value_bytes(&s), Some("héllo".as_bytes()));
    assert_eq!(s.data_type().name(), "utf8");

    let null = Utf8Scalar::null();
    assert!(null.is_null());
    assert_eq!(null.as_str(), None);
    assert!(ScalarType::is_null(&null));
}

#[test]
fn utf8_scalar_rejects_invalid_utf8_with_a_guided_error() {
    let err = <ByteScalar<Utf8>>::from_bytes(&[0x66, 0xff, 0x6f]).unwrap_err();
    assert_eq!(err, IoError::InvalidUtf8 { position: 1 });
    let text = err.to_string();
    assert!(text.contains("invalid UTF-8 at byte 1"));
    assert!(text.contains("binary type")); // points at the fix
}

#[test]
fn binary_scalar_holds_arbitrary_bytes() {
    let s = BinaryScalar::of(&[0xff, 0x00, 0xfe]);
    assert!(!s.is_null());
    assert_eq!(s.value_bytes(), Some(&[0xff, 0x00, 0xfe][..]));
    assert_eq!(s.data_type().name(), "binary");
}

#[test]
fn scalar_round_trips_through_a_byte_sink() {
    for scalar in [Utf8Scalar::of("round trip"), Utf8Scalar::null()] {
        let mut sink = Bytes::new();
        scalar.write_to(&mut sink).unwrap();
        sink.rewind();
        assert_eq!(Utf8Scalar::read_from(&mut sink).unwrap(), scalar);
    }

    let blob = BinaryScalar::of(&[0, 1, 2, 255]);
    let mut sink = Bytes::new();
    blob.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(BinaryScalar::read_from(&mut sink).unwrap(), blob);
}

#[test]
fn scalar_is_hashable_and_equatable_as_a_set_key() {
    let mut set = HashSet::new();
    set.insert(Utf8Scalar::of("a"));
    set.insert(Utf8Scalar::of("a")); // duplicate value
    set.insert(Utf8Scalar::of("b"));
    set.insert(Utf8Scalar::null());
    set.insert(Utf8Scalar::null()); // duplicate null
    assert_eq!(set.len(), 3); // {"a", "b", null}
    assert!(set.contains(&Utf8Scalar::of("a")));
}

// -------------------------------------------------------------------------------------
// Series: the empty-string-vs-null distinction, offsets, nulls, accessors
// -------------------------------------------------------------------------------------

#[test]
fn utf8_serie_distinguishes_empty_string_from_null() {
    let mut col = Utf8Serie::new();
    col.push_str(Some("a"));
    col.push_str(None); // null
    col.push_str(Some("")); // present, empty
    col.push_str(Some("cd"));

    assert_eq!(col.len(), 4);
    assert_eq!(col.null_count(), 1);
    assert!(col.has_nulls());
    assert_eq!(col.data_len(), 3); // "a" + "" + "cd" = 3 bytes, nulls contribute nothing

    assert_eq!(col.get_str(0), Some("a"));
    assert_eq!(col.get_str(1), None); // null
    assert_eq!(col.get_str(2), Some("")); // empty, NOT null
    assert_eq!(col.get_str(3), Some("cd"));
    assert_eq!(col.get_str(4), None); // out of range

    // The null and the empty string are genuinely different states.
    assert!(col.get_scalar(1).is_null());
    assert_eq!(col.get_scalar(2).as_str(), Some(""));
}

#[test]
fn utf8_serie_from_and_to_strs() {
    let values = [Some("x"), None, Some("yz")];
    let col = Utf8Serie::from_strs(&values);
    assert_eq!(col.to_strs(), values);
    // The generic `SerieType::get` hands back an owned copy.
    assert_eq!(SerieType::get(&col, 2).as_deref(), Some(&b"yz"[..]));
    assert_eq!(VarSerie::value_bytes(&col, 0), Some(&b"x"[..]));
}

#[test]
fn no_nulls_means_no_validity_overhead_but_still_correct() {
    let col = Utf8Serie::from_strs(&[Some("one"), Some("two")]);
    assert_eq!(col.null_count(), 0);
    assert!(!col.has_nulls());
    assert_eq!(col.get_str(0), Some("one"));
}

#[test]
fn binary_serie_holds_arbitrary_bytes_and_round_trips() {
    let col =
        BinarySerie::from_byte_values(&[Some(&[0xff, 0x00][..]), None, Some(&[0x01][..])]).unwrap();
    assert_eq!(col.len(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get_bytes(0), Some(&[0xff, 0x00][..]));
    assert_eq!(col.get_bytes(1), None);
    assert_eq!(col.get_bytes(2), Some(&[0x01][..]));

    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(BinarySerie::read_from(&mut sink).unwrap(), col);
}

#[test]
fn utf8_serie_round_trips_including_nulls_and_empty() {
    let col = Utf8Serie::from_strs(&[Some("α"), None, Some(""), Some("longer value here")]);
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    let back = Utf8Serie::read_from(&mut sink).unwrap();
    assert_eq!(back, col);
    assert_eq!(back.get_str(0), Some("α"));
    assert_eq!(back.get_str(1), None);
    assert_eq!(back.get_str(2), Some(""));
}

#[test]
fn empty_serie_round_trips() {
    let col = Utf8Serie::new();
    assert!(col.is_empty());
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(Utf8Serie::read_from(&mut sink).unwrap(), col);
}

#[test]
fn corrupt_offsets_are_a_guided_error_not_a_panic() {
    // A hostile frame whose last offset (100) runs past the empty data buffer must be rejected
    // on read — never handed back as a Serie whose `get_bytes` would slice out of bounds.
    let mut frame = Bytes::new();
    frame.write_all(&1u64.to_le_bytes()).unwrap(); // len = 1
    frame.write_all(&[0u8]).unwrap(); // flags: no validity
    frame.write_all(&0i32.to_le_bytes()).unwrap(); // offsets[0] = 0
    frame.write_all(&100i32.to_le_bytes()).unwrap(); // offsets[1] = 100 (past empty data)
    frame.write_all(&0u64.to_le_bytes()).unwrap(); // data_len = 0
    frame.rewind();
    let err = Utf8Serie::read_from(&mut frame).unwrap_err();
    assert!(matches!(err, IoError::CorruptOffsets { .. }));

    // A non-monotonic offset pair is likewise a guided error, not a panic.
    let mut frame = Bytes::new();
    frame.write_all(&1u64.to_le_bytes()).unwrap();
    frame.write_all(&[0u8]).unwrap();
    frame.write_all(&5i32.to_le_bytes()).unwrap(); // offsets[0] = 5 (must be 0)
    frame.write_all(&5i32.to_le_bytes()).unwrap();
    frame.write_all(&5u64.to_le_bytes()).unwrap();
    frame.write_all(b"hello").unwrap();
    frame.rewind();
    assert!(matches!(
        Utf8Serie::read_from(&mut frame).unwrap_err(),
        IoError::CorruptOffsets { .. }
    ));
}

#[test]
fn from_byte_values_validates_utf8() {
    // A `Utf8` column rejects invalid bytes up front.
    let err = Utf8Serie::from_byte_values(&[Some(&[0xff][..])]).unwrap_err();
    assert!(matches!(err, IoError::InvalidUtf8 { position: 0 }));

    // The same bytes are fine as `Binary`.
    assert!(BinarySerie::from_byte_values(&[Some(&[0xff][..])]).is_ok());
}

#[test]
fn serie_field_infers_nullability() {
    let with_nulls = Utf8Serie::from_strs(&[Some("a"), None]);
    assert!(with_nulls.to_field("c").nullable());

    let no_nulls = Utf8Serie::from_strs(&[Some("a"), Some("b")]);
    assert!(!no_nulls.to_field("c").nullable());
    assert!(no_nulls.field("c", true).nullable()); // explicit override
}
