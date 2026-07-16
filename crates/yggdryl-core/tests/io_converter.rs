//! The **type converter** — the compile-time-generic `cast` on the fixed value types plus the
//! universal UTF-8 / binary bridges. Focus: range checks (integers), truncation & non-finite
//! handling (floats), null passthrough, the same-type no-copy fast path, and the string / byte
//! bridges that reach anything from anything.

use yggdryl_core::io::fixed::{f16, Buffer, Scalar, Serie};
use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar};
use yggdryl_core::io::{CastError, Converter};

#[test]
fn integer_casts_are_range_checked() {
    // Widen and narrow-in-range are exact.
    assert_eq!(
        Scalar::of(300i32).cast::<i64>().unwrap(),
        Scalar::of(300i64)
    );
    assert_eq!(
        Scalar::of(300i64).cast::<i16>().unwrap(),
        Scalar::of(300i16)
    );
    // Out-of-range narrowing is a guided error naming the value.
    assert!(matches!(
        Scalar::of(300i32).cast::<u8>(),
        Err(CastError::OutOfRange { .. })
    ));
    // A negative into an unsigned type fails.
    assert!(Scalar::of(-1i32).cast::<u32>().is_err());
    // u64 -> i64 boundary.
    assert!(Scalar::of(u64::MAX).cast::<i64>().is_err());
    assert_eq!(Scalar::of(255u64).cast::<u8>().unwrap(), Scalar::of(255u8));
}

#[test]
fn float_and_int_crossovers() {
    // int -> float, float -> int (truncating toward zero).
    assert_eq!(Scalar::of(3i32).cast::<f64>().unwrap(), Scalar::of(3.0f64));
    assert_eq!(Scalar::of(3.9f64).cast::<i32>().unwrap(), Scalar::of(3i32));
    assert_eq!(
        Scalar::of(-3.9f64).cast::<i32>().unwrap(),
        Scalar::of(-3i32)
    );
    // float -> float (lossy narrowing is allowed).
    assert_eq!(
        Scalar::of(1.5f64).cast::<f32>().unwrap(),
        Scalar::of(1.5f32)
    );
    // half precision participates.
    assert_eq!(
        Scalar::of(f16::from_f32(1.5)).cast::<f32>().unwrap(),
        Scalar::of(1.5f32)
    );
    // A non-finite float cannot become an integer; out-of-range truncation is rejected.
    assert!(matches!(
        Scalar::of(f64::NAN).cast::<i32>(),
        Err(CastError::NotFinite { .. })
    ));
    assert!(matches!(
        Scalar::of(1e30f64).cast::<i32>(),
        Err(CastError::OutOfRange { .. })
    ));
}

#[test]
fn inexact_float_to_int_gate_rejects_the_boundary_power_of_two() {
    // REGRESSION: `<int>::MAX as f64` rounds UP for the wide types (`i64::MAX` -> 2^63,
    // `u64::MAX` -> 2^64, `i128::MAX` -> 2^127), so the old `t <= MAX as f64` gate let those exact
    // powers through and then `t as int` SATURATED to MAX with no error. Each must now be rejected.
    assert!(matches!(
        Scalar::of(2f64.powi(63)).cast::<i64>(),
        Err(CastError::OutOfRange { .. })
    ));
    assert!(Serie::from_values(&[2f64.powi(63)]).cast::<i64>().is_err());
    assert!(Scalar::of(2f64.powi(64)).cast::<u64>().is_err()); // 2^64 into u64
    assert!(Scalar::of(2f64.powi(127)).cast::<i128>().is_err()); // 2^127 into i128

    // In-range values still cast exactly (the correct behavior is unchanged).
    let just_under = 2f64.powi(62); // well inside i64
    assert_eq!(
        Serie::from_values(&[just_under])
            .cast::<i64>()
            .unwrap()
            .to_options(),
        [Some(just_under as i64)]
    );
    assert_eq!(
        Scalar::of(9.0e18f64).cast::<i64>().unwrap(), // < 2^63, fits
        Scalar::of(9_000_000_000_000_000_000i64)
    );
}

#[test]
fn null_and_column_casts_preserve_nulls() {
    // A null casts to a null of the target.
    assert_eq!(Scalar::<i32>::null().cast::<f64>().unwrap(), Scalar::null());
    // A whole column converts element-for-element, nulls preserved.
    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(
        col.cast::<i64>().unwrap().to_options(),
        [Some(1i64), None, Some(3)]
    );
    // One out-of-range element fails the whole cast.
    assert!(Serie::from_values(&[1i32, 300]).cast::<u8>().is_err());
    // The Converter trait can be driven directly, too.
    let doubled = <i32 as Converter<f64>>::cast_serie(&col).unwrap();
    assert_eq!(doubled.to_options(), [Some(1.0f64), None, Some(3.0)]);
}

#[test]
fn same_type_cast_is_a_no_copy_identity() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    let same = col.cast::<i32>().unwrap(); // same type — shares the buffer, no conversion
    assert_eq!(same, col);
    let buf = Buffer::from_slice(&[1i32, 2, 3]);
    assert_eq!(buf.cast::<i32>().unwrap().to_vec(), buf.to_vec());
    // Buffer cross-width cast.
    assert_eq!(buf.cast::<i64>().unwrap().to_vec(), vec![1i64, 2, 3]);
}

#[test]
fn utf8_bridge_reaches_anything() {
    // any -> utf8 (Display) and utf8 -> any (parse).
    assert_eq!(Scalar::of(42i32).to_utf8().as_str(), Some("42"));
    assert_eq!(
        Utf8Scalar::of("42").parse_to::<i32>().unwrap(),
        Scalar::of(42)
    );
    assert_eq!(
        Utf8Scalar::of("  -7 ").parse_to::<i64>().unwrap(),
        Scalar::of(-7i64)
    );
    assert_eq!(
        Utf8Scalar::of("2.5").parse_to::<f64>().unwrap(),
        Scalar::of(2.5)
    );
    // Round-trip a value type -> utf8 -> a different value type.
    let text = Scalar::of(1000u16).to_utf8();
    assert_eq!(text.parse_to::<i64>().unwrap(), Scalar::of(1000i64));
    // Bad text is a guided parse error; a null passes through.
    assert!(matches!(
        Utf8Scalar::of("nope").parse_to::<i32>(),
        Err(CastError::Parse { .. })
    ));
    assert_eq!(
        Utf8Scalar::null().parse_to::<i32>().unwrap(),
        Scalar::null()
    );
}

#[test]
fn binary_bridge_round_trips_bytes() {
    // any -> binary (canonical LE bytes) and binary -> any.
    let bin = Scalar::of(0x0102_0304i32).to_binary();
    assert_eq!(bin.value_bytes(), Some(&[0x04, 0x03, 0x02, 0x01][..])); // little-endian
    assert_eq!(bin.read_to::<i32>().unwrap(), Scalar::of(0x0102_0304i32));
    // A wrong-width blob for the target is a guided error.
    assert!(matches!(
        BinaryScalar::of(&[1, 2, 3]).read_to::<i32>(),
        Err(CastError::WidthMismatch { .. })
    ));
    // Nulls pass through both directions.
    assert_eq!(Scalar::<i64>::null().to_binary(), BinaryScalar::null());
    assert_eq!(
        BinaryScalar::null().read_to::<i64>().unwrap(),
        Scalar::null()
    );
}

#[test]
fn cast_error_messages_are_guided() {
    let err = Scalar::of(300i32).cast::<u8>().unwrap_err();
    assert_eq!(err.to_string(), "value 300 is out of range for `u8`");
    let err = BinaryScalar::of(&[1, 2, 3]).read_to::<i32>().unwrap_err();
    assert_eq!(
        err.to_string(),
        "binary value is 3 bytes, but `i32` needs exactly 4"
    );
}
