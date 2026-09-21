//! `rust/src/iceberg/value.rs`: iceberg's portable single-value bytes.
//!
//! The encoding a bound is written in and read back under is what every
//! manifest summary and every pruning decision stands on, and no caller can
//! name it, so it is reached through `yggdryl::internals`. What a caller can
//! observe of those bounds is pinned in `rust/tests/iceberg/manifest.rs` and
//! `rust/tests/iceberg/scan.rs`.

use yggdryl::internals::iceberg_value::{compare_single, single_to_value, single_value};
use yggdryl::{DataType, Scalar};

#[test]
fn promoted_bounds_decode_under_the_current_type() {
    let int = 37_i32.to_le_bytes();
    assert_eq!(
        single_to_value(&int, &DataType::Int64),
        Some(Scalar::from(37))
    );
    assert_eq!(
        compare_single(&int, &38_i64.to_le_bytes(), &DataType::Int64),
        Some(std::cmp::Ordering::Less)
    );

    let float = 1.5_f32.to_le_bytes();
    assert_eq!(
        single_to_value(&float, &DataType::Float64).and_then(|value| value.as_f64()),
        Some(1.5)
    );
    assert_eq!(
        compare_single(&float, &2.0_f64.to_le_bytes(), &DataType::Float64),
        Some(std::cmp::Ordering::Less)
    );
}

#[test]
fn malformed_bounds_are_unknown_instead_of_zero() {
    for bytes in [vec![], vec![0; 3], vec![0; 5], vec![0; 7], vec![0; 9]] {
        assert!(single_to_value(&bytes, &DataType::Int32).is_none());
        assert!(single_to_value(&bytes, &DataType::Int64).is_none());
        assert!(single_to_value(&bytes, &DataType::Float32).is_none());
        assert!(single_to_value(&bytes, &DataType::Float64).is_none());
        assert!(compare_single(&bytes, &0_i64.to_le_bytes(), &DataType::Int64).is_none());
    }
    assert!(single_to_value(&[], &DataType::Boolean).is_none());
    assert!(single_to_value(&[0, 1], &DataType::Boolean).is_none());
    assert!(single_to_value(&[2], &DataType::Boolean).is_none());
    assert!(single_to_value(&[0; 3], &DataType::fixed_binary(4).unwrap()).is_none());
    assert!(single_to_value(&[0; 5], &DataType::fixed_binary(4).unwrap()).is_none());
    assert!(single_to_value(&[0xff], &DataType::utf8()).is_none());
}

#[test]
fn nan_is_never_encoded_or_decoded_as_a_bound() {
    let f32_nan = f32::NAN.to_le_bytes();
    let f64_nan = f64::NAN.to_le_bytes();

    assert!(single_value(&Scalar::from(f32::NAN), &DataType::Float32).is_none());
    assert!(single_value(&Scalar::from(f64::NAN), &DataType::Float64).is_none());
    assert!(single_to_value(&f32_nan, &DataType::Float32).is_none());
    assert!(single_to_value(&f64_nan, &DataType::Float64).is_none());
    assert!(compare_single(&f64_nan, &1.5_f64.to_le_bytes(), &DataType::Float64).is_none());

    // Signed zero and infinities are valid Iceberg bounds.
    for value in [-0.0_f64, 0.0, f64::NEG_INFINITY, f64::INFINITY] {
        let scalar = Scalar::from(value);
        let encoded = single_value(&scalar, &DataType::Float64).unwrap();
        assert_eq!(single_to_value(&encoded, &DataType::Float64), Some(scalar));
    }
}

#[test]
fn official_single_value_bytes_round_trip_supported_values() {
    let cases = [
        (Scalar::from(true), DataType::Boolean),
        (Scalar::from(-7), DataType::Int32),
        (Scalar::from(9), DataType::Int64),
        (Scalar::from("é"), DataType::utf8()),
        (Scalar::from([0_u8, 1, 2].as_slice()), DataType::binary()),
    ];
    for (value, dtype) in cases {
        let bytes = single_value(&value, &dtype).expect("a supported value");
        assert_eq!(single_to_value(&bytes, &dtype), Some(value));
    }
}

#[test]
fn decoded_bounds_keep_the_declared_scalar_identity() {
    let cases = [
        (Scalar::from("value"), DataType::large_utf8()),
        (Scalar::from("value"), DataType::utf8_view()),
        (Scalar::from("USD"), DataType::fixed_ascii(4).unwrap()),
        (Scalar::from("USD"), DataType::Currency),
        (
            Scalar::from("00112233-4455-6677-8899-aabbccddeeff"),
            DataType::Uuid,
        ),
        (
            Scalar::from([1_u8, 2, 3].as_slice()),
            DataType::large_binary(),
        ),
        (
            Scalar::from([1_u8, 2, 3].as_slice()),
            DataType::binary_view(),
        ),
        (
            Scalar::from([1_u8, 2, 3].as_slice()),
            DataType::fixed_binary(3).unwrap(),
        ),
    ];
    for (natural, dtype) in cases {
        let canonical = dtype.scalar(natural).unwrap();
        let bytes = single_value(&canonical, &dtype)
            .unwrap_or_else(|| panic!("{dtype} should have a portable bound for {canonical:?}"));
        let decoded = single_to_value(&bytes, &dtype).expect("a decoded bound");
        assert_eq!(decoded.dtype().unwrap(), dtype);
        assert_eq!(decoded, canonical);
    }
}
