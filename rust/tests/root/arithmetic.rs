//! `rust/src/arithmetic.rs`: the scalar arithmetic dispatcher no caller can
//! name.
//!
//! `Arithmetic` is crate-private: it names which operation a binary walk is
//! performing, and a caller reaches it only through the operator traits.
//! What a caller can observe is beside it here; the scalar surface it rides
//! on is in `rust/tests/root/scalar.rs`.

use yggdryl::internals::arithmetic::{Arithmetic, checked_arithmetic};
use yggdryl::{Error, Float16, Float32, Float64, Scalar, TimeUnit, Timezone, i256};

#[test]
fn integer_arithmetic_preserves_width_and_promotes_without_loss() {
    assert_eq!(
        Scalar::from(3_i8).checked_add(&Scalar::from(4_i8)).unwrap(),
        Scalar::from(7_i8)
    );
    assert_eq!(
        Scalar::from(-1_i8)
            .checked_add(&Scalar::from(2_u8))
            .unwrap(),
        Scalar::from(1_i16)
    );
    assert!(matches!(
        Scalar::from(127_i8).checked_add(&Scalar::from(1_i8)),
        Err(Error::ArithmeticOverflow { kind: "i8", .. })
    ));
    assert!(matches!(
        Scalar::from(1_i128).checked_add(&Scalar::from(1_u128)),
        Err(Error::InvalidArithmetic { .. })
    ));
}

#[test]
fn float_arithmetic_promotes_width_and_uses_checked_zero() {
    let left = Scalar::from(Float16::from_f16(half::f16::from_f32(1.5)));
    let right = Scalar::from(Float32::from_f32(2.0));
    assert_eq!(
        left.checked_mul(&right).unwrap(),
        Scalar::from(Float32::from_f32(3.0))
    );
    assert!(matches!(
        right.checked_div(&Scalar::from(Float32::from_f32(0.0))),
        Err(Error::DivisionByZero { .. })
    ));
    assert!(matches!(
        right.checked_rem(&Scalar::from(Float32::from_f32(-0.0))),
        Err(Error::DivisionByZero { .. })
    ));
}

#[test]
fn float_wrappers_keep_their_width_for_native_arithmetic() {
    let mut half = Float16::from_f16(half::f16::from_f32(1.5));
    half += Float16::from_f16(half::f16::from_f32(0.5));
    assert_eq!(half, Float16::from_f16(half::f16::from_f32(2.0)));
    assert_eq!(
        Float32::from_f32(7.0) % Float32::from_f32(4.0),
        Float32::from_f32(3.0)
    );
    assert_eq!(-Float64::from_f64(2.5), Float64::from_f64(-2.5));
    assert_eq!(Float32::from_f32(-0.0).abs(), Float32::from_f32(0.0));
}

#[test]
fn decimal_arithmetic_is_scale_exact() {
    assert_eq!(
        Scalar::d128(105, 2)
            .checked_add(&Scalar::d128(2, 1))
            .unwrap(),
        Scalar::d128(125, 2)
    );
    assert_eq!(
        Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0)).unwrap(),
        Scalar::d128(5, 1)
    );
    assert_eq!(
        Scalar::d128(100, 2)
            .checked_div(&Scalar::d128(2, 0))
            .unwrap(),
        Scalar::d128(5, 1)
    );
    assert_eq!(
        Scalar::d128(1, 0)
            .checked_div(&Scalar::d128(128, 0))
            .unwrap(),
        Scalar::d128(78_125, 7)
    );
    assert!(matches!(
        Scalar::d128(1, 0).checked_div(&Scalar::d128(3, 0)),
        Err(Error::InexactArithmetic { .. })
    ));

    let maximum = Scalar::d128(i128::MAX, 0);
    assert_eq!(maximum.checked_div(&maximum).unwrap(), Scalar::d128(1, 0));
    let wide: i256 = "9999999999999999999999999999999999999999999999999999999999999999999999999999"
        .parse()
        .unwrap();
    let maximum = Scalar::d256(wide, 0);
    assert_eq!(
        maximum.checked_div(&maximum).unwrap(),
        Scalar::d256(i256::from_i128(1), 0)
    );
    assert_eq!(
        Scalar::d128(3, 0).checked_div(&Scalar::d128(6, 0)).unwrap(),
        Scalar::d128(5, 1)
    );

    let denominator = (0..75).fold(i256::from_i128(1), |value, _| {
        value.checked_mul(i256::from_i128(2)).unwrap()
    });
    let coefficient = (0..75).fold(i256::from_i128(1), |value, _| {
        value.checked_mul(i256::from_i128(5)).unwrap()
    });
    assert_eq!(
        Scalar::d256(i256::from_i128(1), 0)
            .checked_div(&Scalar::d256(denominator, 0))
            .unwrap(),
        Scalar::d256(coefficient, 75)
    );
}

#[test]
fn temporal_arithmetic_uses_durations_and_preserves_zone() {
    let at = Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap();
    let elapsed = Scalar::duration64(2, TimeUnit::Second).unwrap();
    assert_eq!(
        at.checked_add(&elapsed).unwrap(),
        Scalar::datetime64(3_000, TimeUnit::Millisecond, Timezone::UTC).unwrap()
    );
    assert_eq!(
        at.checked_sub(&Scalar::datetime64(500, TimeUnit::Millisecond, Timezone::UTC).unwrap())
            .unwrap(),
        Scalar::duration64(500, TimeUnit::Millisecond).unwrap()
    );
    assert_eq!(
        Scalar::date32(2).checked_sub(&Scalar::date32(1)).unwrap(),
        Scalar::duration64(1, TimeUnit::Day).unwrap()
    );
    assert_eq!(
        Scalar::date64(86_400_001)
            .checked_sub(&Scalar::date32(1))
            .unwrap(),
        Scalar::duration64(1, TimeUnit::Millisecond).unwrap()
    );
    assert_eq!(
        Scalar::datetime64(2, TimeUnit::Second, Timezone::NAIVE)
            .unwrap()
            .checked_sub(
                &Scalar::datetime64(1_000_000_000, TimeUnit::Nanosecond, Timezone::NAIVE,).unwrap(),
            )
            .unwrap(),
        Scalar::duration64(1_000_000_000, TimeUnit::Nanosecond).unwrap()
    );
}

#[test]
fn durations_scale_only_by_exact_integers() {
    let duration = Scalar::duration32(12, TimeUnit::Second).unwrap();
    assert_eq!(
        duration.checked_mul(&Scalar::from(-3_i16)).unwrap(),
        Scalar::duration32(-36, TimeUnit::Second).unwrap()
    );
    assert_eq!(
        Scalar::from(3_u8).checked_mul(&duration).unwrap(),
        Scalar::duration32(36, TimeUnit::Second).unwrap()
    );
    assert_eq!(
        duration.checked_div(&Scalar::from(3_i8)).unwrap(),
        Scalar::duration32(4, TimeUnit::Second).unwrap()
    );
    assert!(matches!(
        duration.checked_div(&Scalar::from(5_i8)),
        Err(Error::InexactArithmetic { .. })
    ));
    assert!(matches!(
        duration.checked_div(&Scalar::from(0_i8)),
        Err(Error::DivisionByZero { .. })
    ));
    assert!(Scalar::date32(1).checked_mul(&Scalar::from(2_i8)).is_err());
    assert!(Scalar::from(2_i8).checked_div(&duration).is_err());
}

#[test]
fn null_propagates_through_every_binary_operation() {
    for operation in [
        Arithmetic::Add,
        Arithmetic::Sub,
        Arithmetic::Mul,
        Arithmetic::Div,
        Arithmetic::Rem,
    ] {
        assert_eq!(
            checked_arithmetic(&Scalar::Null, &Scalar::from(0_i8), operation).unwrap(),
            Scalar::Null
        );
        assert_eq!(
            checked_arithmetic(&Scalar::from(0_i8), &Scalar::Null, operation).unwrap(),
            Scalar::Null
        );
    }
    assert_eq!(Scalar::Null.checked_neg().unwrap(), Scalar::Null);
    assert_eq!(Scalar::Null.checked_abs().unwrap(), Scalar::Null);
}

#[test]
fn operator_traits_are_checked_and_do_not_concatenate() {
    assert_eq!(
        (&Scalar::from(7_i16) + &Scalar::from(5_i16)).unwrap(),
        Scalar::from(12_i16)
    );
    assert_eq!((-Scalar::from(7_u8)).unwrap(), Scalar::from(-7_i16));
    assert_eq!(
        Scalar::from(-7_i16).checked_abs().unwrap(),
        Scalar::from(7_i16)
    );
    assert_eq!(
        Scalar::from(u128::MAX).checked_abs().unwrap(),
        Scalar::from(u128::MAX)
    );
    assert!(matches!(
        Scalar::from(i8::MIN).checked_abs(),
        Err(Error::ArithmeticOverflow { .. })
    ));
    assert_eq!(
        (Scalar::from("a") + Scalar::from("b")).unwrap(),
        Scalar::from("ab")
    );
    assert!((Scalar::from("a") - Scalar::from("b")).is_err());
}
