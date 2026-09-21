//! `rust/src/scalar.rs`: the scalar invariants no caller can reach.
//!
//! The scalar invariants an integration test cannot reach.
//!
//! Everything a caller can observe lives in `tests/types/scalar.rs`. These two
//! pin crate-private readers: the sign-and-magnitude reader every width
//! compares through, and the rank sweep that decides how two kinds sort.

use yggdryl::internals::scalar::{
    compare_integer_parts, integer_parts, leaf_display, point_empty_wkb, stable_hash_of,
    try_sequence, value_rank,
};
use yggdryl::{Error, Scalar, TimeUnit, Timezone, i256};

#[test]
fn a_fallible_sequence_skips_the_callback_when_empty() {
    let mut calls = 0;
    let sequence = try_sequence(0, |_| {
        calls += 1;
        Ok(Scalar::Null)
    })
    .unwrap();

    assert_eq!(calls, 0);
    assert_eq!(sequence.as_sequence(), Some([].as_slice()));
}

#[test]
fn a_fallible_sequence_keeps_every_input_in_index_order() {
    let inputs = vec![Scalar::from("first"), Scalar::from(2_i64), Scalar::Null];
    let retained = inputs.clone();
    let mut calls = Vec::new();
    let sequence = try_sequence(inputs.len(), |index| {
        calls.push(index);
        Ok(inputs[index].clone())
    })
    .unwrap();

    assert_eq!(calls, [0, 1, 2]);
    assert_eq!(sequence.as_sequence(), Some(retained.as_slice()));
    assert_eq!(inputs, retained);
}

#[test]
fn a_fallible_sequence_stops_at_its_first_refusal() {
    let mut calls = Vec::new();
    let error = try_sequence(5, |index| {
        calls.push(index);
        if index == 2 {
            return Err(Error::InvalidRecord {
                path: "$[2]".into(),
                reason: "refused child".into(),
            });
        }
        Ok(Scalar::from(i64::try_from(index).unwrap()))
    })
    .unwrap_err();

    assert_eq!(calls, [0, 1, 2]);
    assert!(error.to_string().contains("refused child"));
}

#[test]
fn the_integer_sign_and_magnitude_reader_answers_every_width() {
    use std::cmp::Ordering;

    let cases = [
        (Scalar::from(-7_i8), Some((true, 7))),
        (Scalar::from(-7_i16), Some((true, 7))),
        (Scalar::from(-7_i32), Some((true, 7))),
        (Scalar::from(-7_i64), Some((true, 7))),
        (
            Scalar::from(i128::MIN),
            Some((true, i128::MIN.unsigned_abs())),
        ),
        (Scalar::from(7_u8), Some((false, 7))),
        (Scalar::from(7_u16), Some((false, 7))),
        (Scalar::from(7_u32), Some((false, 7))),
        (Scalar::from(7_u64), Some((false, 7))),
        (Scalar::from(u128::MAX), Some((false, u128::MAX))),
        (Scalar::from(0_i32), Some((false, 0))),
        (Scalar::from(7.0), None),
        (Scalar::d128(7, 0), None),
        (Scalar::Null, None),
    ];
    for (value, expected) in &cases {
        assert_eq!(integer_parts(value), *expected, "{value:?}");
    }

    assert_eq!(compare_integer_parts((true, 1), (false, 0)), Ordering::Less);
    assert_eq!(compare_integer_parts((true, 2), (true, 1)), Ordering::Less);
    assert_eq!(
        compare_integer_parts((false, 2), (false, 1)),
        Ordering::Greater
    );
    assert_eq!(
        compare_integer_parts((false, 7), (false, 7)),
        Ordering::Equal
    );
}

#[test]
fn the_value_rank_sweep_is_unchanged() {
    use yggdryl::Side;

    let point = yggdryl::Geometry::new(point_empty_wkb()).unwrap();
    // One value of every variant, in declaration order, with the rank it has
    // always had. The rank is wire-visible: it orders dictionary values.
    let values = [
        (Scalar::Null, 0),
        (Scalar::from(true), 1),
        (Scalar::from(1_i8), 2),
        (Scalar::from(1_i16), 2),
        (Scalar::from(1_i32), 2),
        (Scalar::from(1_i64), 2),
        (Scalar::from(1_u8), 2),
        (Scalar::from(1_u16), 2),
        (Scalar::from(1_u32), 2),
        (Scalar::from(1_u64), 2),
        (Scalar::from(1_i128), 2),
        (Scalar::from(1_u128), 2),
        (Scalar::from(half::f16::from_f32(1.0)), 3),
        (Scalar::from(1.0_f32), 3),
        (Scalar::from(1.0_f64), 3),
        (Scalar::Decimal32(yggdryl::Decimal32::new(1, 0)), 4),
        (Scalar::Decimal64(yggdryl::Decimal64::new(1, 0)), 4),
        (Scalar::d128(1, 0), 4),
        (Scalar::d256(i256::from_i128(1), 0), 4),
        (Scalar::date32(1), 7),
        (Scalar::date64(86_400_000), 7),
        (
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            8,
        ),
        (
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            8,
        ),
        (
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
            9,
        ),
        (Scalar::duration32(1, TimeUnit::Second).unwrap(), 10),
        (Scalar::duration64(1, TimeUnit::Second).unwrap(), 10),
        (
            Scalar::Interval(yggdryl::Interval::new(1, 0, 0, TimeUnit::YearMonth).unwrap()),
            16,
        ),
        (Scalar::from("a"), 5),
        (Scalar::from(Side::new("1").unwrap()), 18),
        (
            Scalar::Uuid(
                yggdryl::uuid::Uuid::from_bytes(b"550e8400-e29b-41d4-a716-446655440000").unwrap(),
            ),
            17,
        ),
        (Scalar::from(yggdryl::Version::new(1, 2, 3)), 19),
        (
            Scalar::from(yggdryl::Url::from_str("https://example.com/a").unwrap()),
            20,
        ),
        // A vocabulary member is its name, so it ranks with the text it is.
        (Scalar::from(TimeUnit::Second), 5),
        (Scalar::from(b"a".as_slice()), 6),
        (Scalar::Geometry(point), 14),
        (Scalar::from_sequence([]), 11),
        (Scalar::from_mapping([]).unwrap(), 12),
        (
            Scalar::from_struct(Vec::<(&str, Scalar)>::new()).unwrap(),
            13,
        ),
    ];
    assert_eq!(values.len(), 38);
    for (value, rank) in &values {
        assert_eq!(value_rank(value), *rank, "{value:?}");
    }

    // Sorting any arrangement lays the kinds out in rank order.
    let mut sorted = values
        .iter()
        .rev()
        .map(|(value, _)| value.clone())
        .collect::<Vec<_>>();
    sorted.sort();
    let mut expected = values.iter().map(|(_, rank)| *rank).collect::<Vec<_>>();
    expected.sort_unstable();
    assert_eq!(sorted.iter().map(value_rank).collect::<Vec<_>>(), expected);
}

/// Deterministic hashes built over `Hash` keep the bytes the retired width
/// enums fed, pinned at the values the two-level representation produced.
#[test]
fn hash_derived_stable_hashes_keep_their_pre_flattening_values() {
    use yggdryl::interval::Interval;
    let interval = Interval::new(1, 2, 3_000_000, yggdryl::TimeUnit::MonthDayNano).unwrap();
    for (name, value, expected) in [
        (
            "sequence",
            Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("a")]),
            2_351_796_681_665_035_878_u64,
        ),
        (
            "mapping",
            Scalar::from_mapping([(Scalar::from("k"), Scalar::from(2_i64))]).unwrap(),
            17_364_630_997_768_761_460,
        ),
        (
            "record",
            Scalar::from_struct([("a", Scalar::from(1_i32))]).unwrap(),
            12_407_753_854_889_480_402,
        ),
        (
            "interval",
            Scalar::Interval(interval),
            196_150_670_316_405_394,
        ),
        ("i32", Scalar::from(7_i32), 13_767_510_565_555_144_141),
        ("f32", Scalar::from(1.5_f32), 6_394_485_071_238_434_244),
        ("d128", Scalar::d128(1250, 2), 9_433_506_932_114_274_648),
    ] {
        assert_eq!(stable_hash_of(&value), expected, "{name}");
    }
}

/// The leaf's own spelling is reachable without a per-width table.
///
/// `leaf_display` is crate-private - it is what the typed renderer walks
/// through - so what it answers per width is pinned here.
#[test]
fn a_width_variant_borrows_the_leaf_display_it_holds() {
    use std::sync::Arc;

    use yggdryl::{decimal, integer, sequence};

    let decimal = Scalar::Decimal32(decimal::Decimal32::new(1_250, 2));
    assert_eq!(leaf_display(&decimal).unwrap().to_string(), "12.50");
    assert_eq!(
        leaf_display(&Scalar::Int32(integer::Int32::new(7)))
            .unwrap()
            .to_string(),
        "7"
    );
    let held = sequence::Sequence::new(Arc::from([Scalar::from(1_i32)]));
    assert_eq!(
        leaf_display(&Scalar::Sequence(held.clone()))
            .unwrap()
            .to_string(),
        held.to_string()
    );
    // A value whose spelling is the caller's to choose answers nothing.
    assert!(leaf_display(&Scalar::from("12.50")).is_none());
    assert!(leaf_display(&Scalar::from(true)).is_none());
    assert!(leaf_display(&Scalar::Null).is_none());
}
