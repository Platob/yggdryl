//! Behavioural + edge-case tests for the fixed-width decimals.

use yggdryl_core::{i256, Decimal, Decimal128, Decimal256, Decimal32, Decimal64, DecimalError};

#[test]
fn construct_and_numeric_conversions() {
    let d = Decimal64::new(123_456, 3); // 123.456
    assert_eq!(d.mantissa(), 123_456);
    assert_eq!(d.scale(), 3);
    assert!((d.to_f64() - 123.456).abs() < 1e-9);
    assert_eq!(d.to_i128(), Some(123)); // truncates toward zero
    assert_eq!(Decimal32::from_f64(1.5, 1), Decimal32::new(15, 1));
    assert_eq!(
        Decimal128::from_integer(1000, 0).unwrap().to_i128(),
        Some(1000)
    );
}

#[test]
fn byte_round_trip_and_value_semantics() {
    let d = Decimal32::new(-4200, 2); // -42.00
    let bytes = d.serialize_bytes();
    assert_eq!(bytes.len(), 5); // 4 mantissa + 1 scale
    assert_eq!(Decimal32::deserialize_bytes(&bytes).unwrap(), d);

    // Equal iff bytes equal — same value, different scale is NOT equal (rule 7).
    let a = Decimal64::new(10, 1); // 1.0
    let b = Decimal64::new(1, 0); //  1
    assert_ne!(a.serialize_bytes(), b.serialize_bytes());
    assert_ne!(a, b);
    assert!((a.to_f64() - b.to_f64()).abs() < 1e-12); // but the values match

    // A wrong length is a guided error.
    assert!(matches!(
        Decimal32::deserialize_bytes(&[0, 0, 0]),
        Err(DecimalError::InvalidByteLength {
            expected: 5,
            len: 3
        })
    ));
}

#[test]
fn rescale_and_overflow() {
    let d = Decimal64::new(123, 0); // 123
    assert_eq!(d.rescale(2).unwrap(), Decimal64::new(12_300, 2)); // 123.00
    assert_eq!(d.rescale(2).unwrap().rescale(0).unwrap(), d); // exact round-trip

    // Rescaling past the mantissa width overflows with a guided error.
    let big = Decimal32::new(2_000_000_000, 0);
    assert!(matches!(
        big.rescale(2),
        Err(DecimalError::Overflow { bits: 32 })
    ));

    // Negative scale multiplies out (mantissa × 10^-scale).
    let scaled = Decimal64::new(5, -2); // 500
    assert_eq!(scaled.to_i128(), Some(500));
}

#[test]
fn cross_width_conversions() {
    // Widen: 32 -> 64 -> 128 -> 256 keeps the value.
    let d32 = Decimal32::new(12_345, 2); // 123.45
    let d64: Decimal64 = d32.into();
    let d128: Decimal128 = d64.into();
    assert_eq!(d64, Decimal64::new(12_345, 2));
    assert_eq!(d128, Decimal128::new(12_345, 2));
    assert_eq!(
        d32.to_decimal256(),
        Decimal256::new(i256::from_i128(12_345), 2)
    );

    // Narrow 256 -> 128 when it fits.
    let big = Decimal256::new(i256::from_i128(999), 1);
    assert_eq!(big.try_to_decimal128().unwrap(), Decimal128::new(999, 1));

    // Narrow fails (guided) when the mantissa exceeds the target width.
    let huge = Decimal256::new(
        i256::from_i128(i128::MAX)
            .checked_mul(i256::from_i128(2))
            .unwrap(),
        0,
    );
    assert!(matches!(
        huge.try_to_decimal128(),
        Err(DecimalError::Overflow { bits: 128 })
    ));
}

#[test]
fn decimal256_over_i128() {
    // A mantissa beyond i128, held exactly and byte-round-tripped.
    let mantissa = i256::from_i128(i128::MAX)
        .checked_mul(i256::from_i128(10))
        .unwrap();
    let d = Decimal256::new(mantissa, 0);
    assert_eq!(d.to_i128(), None); // integer part exceeds i128
    assert_eq!(d.serialize_bytes().len(), 33); // 32 mantissa + 1 scale
    assert_eq!(
        Decimal256::deserialize_bytes(&d.serialize_bytes()).unwrap(),
        d
    );
    assert_eq!(<Decimal256 as Decimal>::bits(&d), 256);
}

#[test]
fn display_formats_the_scaled_value() {
    assert_eq!(Decimal64::new(123_456, 3).to_string(), "123.456");
    assert_eq!(Decimal64::new(-5, 2).to_string(), "-0.05");
    assert_eq!(Decimal64::new(42, 0).to_string(), "42");
    assert_eq!(Decimal64::new(100, 2).to_string(), "1.00");
}
