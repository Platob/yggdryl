//! The column **access** surface added across every `Serie` family: `get` / `get_scalar`,
//! single `set` / `set_scalar`, and the bulk `set_range` / `set_scalars` / `set_values` — with the
//! edge cases each one has to get right (null↔value transitions, out-of-bounds, empty ranges, and,
//! for the variable-length column, a value that changes length and rewrites the offsets).

use yggdryl_core::io::fixed::{
    D128Scalar, D128Serie, FixedBinaryScalar, FixedBinarySerie, Scalar, Serie, D128,
};
use yggdryl_core::io::var::{Utf8Scalar, Utf8Serie};
use yggdryl_core::io::{Bytes, IOCursor, IoError, SerieType};

// -------------------------------------------------------------------------------------
// Fixed numeric Serie<T>
// -------------------------------------------------------------------------------------

#[test]
fn fixed_set_transitions_value_and_null() {
    let mut col = Serie::from_values(&[1i32, 2, 3, 4]);
    assert_eq!(col.get_scalar(0), Scalar::of(1));
    col.set(1, Some(20)).unwrap();
    col.set(2, None).unwrap(); // value -> null materializes validity
    assert_eq!(col.to_options(), [Some(1), Some(20), None, Some(4)]);
    assert_eq!(col.null_count(), 1);
    col.set(2, Some(30)).unwrap(); // null -> value clears the bit
    assert_eq!(col.get(2), Some(30));
    assert_eq!(col.null_count(), 0);
    col.set_scalar(0, &Scalar::null()).unwrap();
    assert_eq!(col.get(0), None);
}

#[test]
fn fixed_set_out_of_bounds_is_guided() {
    let mut col = Serie::from_values(&[1i32, 2]);
    let err = col.set(2, Some(9)).unwrap_err();
    assert!(matches!(
        err,
        IoError::IndexOutOfBounds { index: 2, len: 2 }
    ));
    assert!(err.to_string().contains("out of bounds"));
    // The column is untouched.
    assert_eq!(col.to_options(), [Some(1), Some(2)]);
}

#[test]
fn fixed_bulk_set_from_serie_scalars_and_values() {
    let mut col = Serie::from_values(&[0i32; 6]);
    // From another Serie (nulls included).
    col.set_range(1, &Serie::from_options(&[Some(7), None, Some(9)]))
        .unwrap();
    assert_eq!(
        col.to_options(),
        [Some(0), Some(7), None, Some(9), Some(0), Some(0)]
    );
    // From scalars.
    col.set_scalars(4, &[Scalar::of(40), Scalar::null()])
        .unwrap();
    assert_eq!(
        col.to_options(),
        [Some(0), Some(7), None, Some(9), Some(40), None]
    );
    // From native values.
    col.set_values(0, &[100, 200]).unwrap();
    assert_eq!(
        col.to_options(),
        [Some(100), Some(200), None, Some(9), Some(40), None]
    );
    // An empty bulk at the end is a no-op; a range past the end errors and leaves it unchanged.
    col.set_values(6, &[]).unwrap();
    assert!(col.set_range(5, &Serie::from_values(&[1i32, 2])).is_err());
    assert_eq!(col.len(), 6);
}

// -------------------------------------------------------------------------------------
// Fixed-size binary FixedSizeSerie<K>
// -------------------------------------------------------------------------------------

#[test]
fn fixed_size_set_overwrites_slot_in_place() {
    let mut col = FixedBinarySerie::new(4);
    for chunk in [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]] {
        col.push(Some(&chunk)).unwrap();
    }
    col.set(1, Some(&[0xaa; 4])).unwrap();
    assert_eq!(col.get_bytes(1), Some(&[0xaa; 4][..]));
    assert_eq!(col.get_scalar(0), FixedBinaryScalar::of(&[1, 2, 3, 4]));
    // A wrong-width value is refused.
    assert!(matches!(
        col.set(1, Some(&[1, 2, 3])),
        Err(IoError::CorruptLength { .. })
    ));
    // value -> null, then bulk from another column.
    col.set(2, None).unwrap();
    assert_eq!(col.get_bytes(2), None);
    let patch = FixedBinarySerie::from_values(4, &[Some(&[7u8; 4][..]), None]).unwrap();
    col.set_range(0, &patch).unwrap();
    assert_eq!(col.get_bytes(0), Some(&[7u8; 4][..]));
    assert_eq!(col.get_bytes(1), None);
}

// -------------------------------------------------------------------------------------
// Variable-length Utf8Serie (the expensive offset-rewriting set)
// -------------------------------------------------------------------------------------

#[test]
fn var_set_rewrites_offsets_on_length_change() {
    let mut col = Utf8Serie::from_strs(&[Some("a"), Some("bb"), Some("ccc"), Some("dddd")]);
    col.set_str(1, Some("longer")).unwrap(); // grow -> trailing offsets shift up
    col.set_str(2, Some("x")).unwrap(); //        shrink -> trailing offsets shift down
    col.set_str(3, None).unwrap(); //             null -> slot shrinks to empty
    assert_eq!(col.to_strs(), [Some("a"), Some("longer"), Some("x"), None]);
    assert_eq!(col.null_count(), 1);
    // The offsets stay consistent — a serialize/deserialize round-trip (which validates offsets)
    // reproduces the column exactly.
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(Utf8Serie::read_from(&mut sink).unwrap(), col);
    // A null slot then set back to a present value.
    col.set_str(3, Some("back")).unwrap();
    assert_eq!(col.get_str(3), Some("back"));
    assert_eq!(col.null_count(), 0);
}

#[test]
fn var_set_edge_cases() {
    let mut col = Utf8Serie::from_strs(&[Some("x"), Some("y")]);
    // Out of bounds leaves the column unchanged.
    assert!(matches!(
        col.set_str(2, Some("z")),
        Err(IoError::IndexOutOfBounds { index: 2, len: 2 })
    ));
    // Invalid UTF-8 through the binary-facing setter leaves the column unchanged.
    assert!(matches!(
        col.set_bytes(0, Some(&[0xff, 0xfe])),
        Err(IoError::InvalidUtf8 { .. })
    ));
    assert_eq!(col.to_strs(), [Some("x"), Some("y")]);
    // Set from scalars, then bulk-replace a range with differing lengths.
    col.set_scalar(0, &Utf8Scalar::of("hello")).unwrap();
    col.set_range(0, &Utf8Serie::from_strs(&[Some("i"), Some("jj")]))
        .unwrap();
    assert_eq!(col.to_strs(), [Some("i"), Some("jj")]);
}

// -------------------------------------------------------------------------------------
// Decimal DecimalSerie<B>
// -------------------------------------------------------------------------------------

#[test]
fn decimal_set_and_bulk() {
    let mut col = D128Serie::from_options(
        20,
        2,
        &[
            Some(D128::new(100, 2).unwrap()),
            None,
            Some(D128::new(200, 2).unwrap()),
        ],
    )
    .unwrap();
    assert_eq!(
        col.get_scalar(0),
        D128Scalar::of(D128::new(100, 2).unwrap())
    );
    // Single set re-expresses at the column scale; null transition works.
    col.set(1, Some(D128::new(5, 0).unwrap())).unwrap(); // 5 -> 5.00 at scale 2
    assert_eq!(col.get(1).unwrap().to_string(), "5.00");
    col.set(0, None).unwrap();
    assert_eq!(col.null_count(), 1);
    // A value that does not fit the column's scale/precision is refused (column unchanged).
    assert!(col.set(2, Some(D128::new(12345, 3).unwrap())).is_err()); // scale 3 into scale 2
                                                                      // Bulk from another column.
    let patch = D128Serie::from_values(
        20,
        2,
        &[D128::new(700, 2).unwrap(), D128::new(800, 2).unwrap()],
    )
    .unwrap();
    col.set_range(1, &patch).unwrap();
    assert_eq!(col.get(1).unwrap().to_string(), "7.00");
    assert_eq!(col.get(2).unwrap().to_string(), "8.00");
    // Out of bounds is guided and leaves the column unchanged.
    assert!(col
        .set_values(2, &[D128::new(1, 2).unwrap(), D128::new(2, 2).unwrap()])
        .is_err());
    assert_eq!(SerieType::len(&col), 3);
}
