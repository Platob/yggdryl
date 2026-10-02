//! `rust/src/floating.rs`.

use yggdryl::floating;
use yggdryl::{DataType, Scalar};

use super::typed::assert_typed_marker;

#[test]
fn floating_markers_cover_every_width() {
    assert_typed_marker::<floating::Float16Type>(DataType::Float16);
    assert_typed_marker::<floating::Float32Type>(DataType::Float32);
    assert_typed_marker::<floating::Float64Type>(DataType::Float64);
}

#[test]
fn scalar_float_arithmetic_keeps_native_bits() {
    let left = 1.0_f64;
    let right = -(1.0 - 2.0_f64.powi(-52));
    let left_scalar = Scalar::from(left);
    let right_scalar = Scalar::from(right);
    for (actual, expected) in [
        (left_scalar.checked_add(&right_scalar), left + right),
        (left_scalar.checked_sub(&right_scalar), left - right),
        (left_scalar.checked_mul(&right_scalar), left * right),
        (left_scalar.checked_div(&right_scalar), left / right),
        (left_scalar.checked_rem(&right_scalar), left % right),
    ] {
        assert_eq!(
            actual.unwrap().as_f64().unwrap().to_bits(),
            expected.to_bits()
        );
    }
    let tiny = Scalar::from(f64::MIN_POSITIVE);
    assert_eq!(
        tiny.checked_div(&Scalar::from(2.0))
            .unwrap()
            .as_f64()
            .unwrap()
            .to_bits(),
        (f64::MIN_POSITIVE / 2.0).to_bits(),
    );
    assert!(
        Scalar::from(f64::MAX)
            .checked_mul(&Scalar::from(2.0))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_infinite(),
    );
}
