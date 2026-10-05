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
        Scalar::decimal128(105, 2)
            .checked_add(&Scalar::decimal128(2, 1))
            .unwrap(),
        Scalar::decimal128(125, 2)
    );
    assert_eq!(
        Scalar::decimal128(1, 0)
            .checked_div(&Scalar::decimal128(2, 0))
            .unwrap(),
        Scalar::decimal128(5, 1)
    );
    assert_eq!(
        Scalar::decimal128(100, 2)
            .checked_div(&Scalar::decimal128(2, 0))
            .unwrap(),
        Scalar::decimal128(5, 1)
    );
    assert_eq!(
        Scalar::decimal128(1, 0)
            .checked_div(&Scalar::decimal128(128, 0))
            .unwrap(),
        Scalar::decimal128(78_125, 7)
    );
    assert!(matches!(
        Scalar::decimal128(1, 0).checked_div(&Scalar::decimal128(3, 0)),
        Err(Error::InexactArithmetic { .. })
    ));

    let maximum = Scalar::decimal128(i128::MAX, 0);
    assert_eq!(
        maximum.checked_div(&maximum).unwrap(),
        Scalar::decimal128(1, 0)
    );
    let wide: i256 = "9999999999999999999999999999999999999999999999999999999999999999999999999999"
        .parse()
        .unwrap();
    let maximum = Scalar::decimal256(wide, 0);
    assert_eq!(
        maximum.checked_div(&maximum).unwrap(),
        Scalar::decimal256(i256::from_i128(1), 0)
    );
    assert_eq!(
        Scalar::decimal128(3, 0)
            .checked_div(&Scalar::decimal128(6, 0))
            .unwrap(),
        Scalar::decimal128(5, 1)
    );

    let denominator = (0..75).fold(i256::from_i128(1), |value, _| {
        value.checked_mul(i256::from_i128(2)).unwrap()
    });
    let coefficient = (0..75).fold(i256::from_i128(1), |value, _| {
        value.checked_mul(i256::from_i128(5)).unwrap()
    });
    assert_eq!(
        Scalar::decimal256(i256::from_i128(1), 0)
            .checked_div(&Scalar::decimal256(denominator, 0))
            .unwrap(),
        Scalar::decimal256(coefficient, 75)
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

#[test]
fn sqrt_private_kernel_requires_float64_and_preserves_ieee() {
    use yggdryl::internals::arithmetic::checked_sqrt;
    assert_eq!(checked_sqrt(&Scalar::Null).unwrap(), Scalar::Null);
    assert_eq!(
        checked_sqrt(&Scalar::from(4.0_f64)).unwrap(),
        Scalar::from(2.0_f64)
    );
    assert!(
        checked_sqrt(&Scalar::from(-1.0_f64))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_nan()
    );
    assert_eq!(
        checked_sqrt(&Scalar::from(f64::INFINITY)).unwrap(),
        Scalar::from(f64::INFINITY)
    );
    assert!(matches!(
        checked_sqrt(&Scalar::from(4_i64)),
        Err(Error::InvalidArithmetic {
            operation: "square root",
            ..
        })
    ));
}

#[cfg(feature = "internals")]
#[test]
fn power_private_kernel_requires_bound_float64_and_preserves_ieee() {
    use yggdryl::internals::arithmetic::checked_pow;
    assert_eq!(
        checked_pow(&Scalar::from(2.0_f64), &Scalar::from(3.0_f64)).unwrap(),
        Scalar::from(8.0_f64)
    );
    assert_eq!(
        checked_pow(&Scalar::Null, &Scalar::from(3.0_f64)).unwrap(),
        Scalar::Null
    );
    assert_eq!(
        checked_pow(&Scalar::from(0.0_f64), &Scalar::from(0.0_f64)).unwrap(),
        Scalar::from(1.0_f64)
    );
    assert!(
        checked_pow(&Scalar::from(-8.0_f64), &Scalar::from(0.5_f64))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_nan()
    );
    assert!(matches!(
        checked_pow(&Scalar::from(2_i64), &Scalar::from(3.0_f64)),
        Err(Error::InvalidArithmetic {
            operation: "power",
            ..
        })
    ));
}

#[cfg(feature = "internals")]
#[test]
fn float64_sign_floor_and_truncate_share_exact_scalar_kernels() {
    use yggdryl::internals::arithmetic::{checked_floor, checked_sign, checked_trunc};

    assert_eq!(checked_sign(&Scalar::Null).unwrap(), Scalar::Null);
    for (input, sign, floor, trunc) in [
        (-3.2, -1.0, -4.0, -3.0),
        // The shared IEEE kernel retains signed zero; Excel normalizes it later.
        (-0.001, -1.0, -1.0, -0.0),
        (0.0, 0.0, 0.0, 0.0),
        (3.2, 1.0, 3.0, 3.0),
    ] {
        let value = Scalar::from(input);
        assert_eq!(checked_sign(&value).unwrap(), Scalar::from(sign));
        assert_eq!(checked_floor(&value).unwrap(), Scalar::from(floor));
        assert_eq!(checked_trunc(&value).unwrap(), Scalar::from(trunc));
    }
    assert_eq!(
        checked_sign(&Scalar::from(-0.0)).unwrap(),
        Scalar::from(0.0)
    );
    assert!(matches!(
        checked_floor(&Scalar::from(3_i64)),
        Err(Error::InvalidArithmetic { .. })
    ));
}

#[cfg(feature = "internals")]
#[test]
fn pure_math_private_kernels_require_bound_float64() {
    use yggdryl::internals::arithmetic::{
        checked_degrees, checked_exp, checked_ln, checked_log10, checked_radians,
    };
    for (name, kernel, expected) in [
        (
            "exp",
            checked_exp as fn(&Scalar) -> yggdryl::Result<Scalar>,
            1.0_f64.exp(),
        ),
        ("ln", checked_ln, 1.0_f64.ln()),
        ("log10", checked_log10, 1.0_f64.log10()),
        ("degrees", checked_degrees, 1.0_f64.to_degrees()),
        ("radians", checked_radians, 1.0_f64.to_radians()),
    ] {
        assert_eq!(kernel(&Scalar::Null).unwrap(), Scalar::Null, "{name}");
        assert_eq!(
            kernel(&Scalar::from(1.0_f64))
                .unwrap()
                .as_f64()
                .unwrap()
                .to_bits(),
            expected.to_bits(),
            "{name}"
        );
        assert!(
            matches!(
                kernel(&Scalar::from(1_i64)),
                Err(Error::InvalidArithmetic { .. })
            ),
            "{name}"
        );
    }
    assert!(
        checked_ln(&Scalar::from(-1.0_f64))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_nan()
    );
    assert!(
        checked_log10(&Scalar::from(-1.0_f64))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_nan()
    );
}

#[cfg(feature = "internals")]
#[test]
fn trigonometry_private_kernels_require_bound_float64() {
    use yggdryl::internals::arithmetic::{checked_asin, checked_cos};
    for (kernel, expected) in [
        (
            checked_cos as fn(&Scalar) -> yggdryl::Result<Scalar>,
            0.5_f64.cos(),
        ),
        (checked_asin, 0.5_f64.asin()),
    ] {
        assert_eq!(kernel(&Scalar::Null).unwrap(), Scalar::Null);
        assert_eq!(
            kernel(&Scalar::from(0.5))
                .unwrap()
                .as_f64()
                .unwrap()
                .to_bits(),
            expected.to_bits()
        );
        assert!(matches!(
            kernel(&Scalar::from(1_i64)),
            Err(Error::InvalidArithmetic { .. })
        ));
    }
    assert!(
        checked_asin(&Scalar::from(2.0))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_nan()
    );
}

#[test]
fn exact_integer_gcd_lcm_handle_signed_minimum_and_overflow() {
    assert_eq!(
        Scalar::from(-12_i8)
            .checked_gcd(&Scalar::from(18_u64))
            .unwrap(),
        Scalar::from(6_u64)
    );
    assert_eq!(
        Scalar::from(12_i64)
            .checked_lcm(&Scalar::from(18_u64))
            .unwrap(),
        Scalar::from(36_u64)
    );
    assert_eq!(
        Scalar::from(0_i8).checked_gcd(&Scalar::from(0_u8)).unwrap(),
        Scalar::from(0_u64)
    );
    assert_eq!(
        Scalar::from(0_i8)
            .checked_lcm(&Scalar::from(18_u8))
            .unwrap(),
        Scalar::from(0_u64)
    );
    assert_eq!(
        Scalar::from(i128::MIN)
            .checked_gcd(&Scalar::from(2_u8))
            .unwrap(),
        Scalar::from(2_u64)
    );
    assert!(matches!(
        Scalar::from(i128::MIN).checked_gcd(&Scalar::from(0_u8)),
        Err(Error::ArithmeticOverflow {
            operation: "greatest common divisor",
            kind: "u64"
        })
    ));
    assert!(matches!(
        Scalar::from(u128::MAX).checked_lcm(&Scalar::from(2_u8)),
        Err(Error::ArithmeticOverflow {
            operation: "least common multiple",
            kind: "u64"
        })
    ));
    assert!(matches!(
        Scalar::from(2.0_f64).checked_gcd(&Scalar::from(2_i64)),
        Err(Error::InvalidArithmetic {
            operation: "greatest common divisor",
            ..
        })
    ));
    assert_eq!(
        Scalar::Null.checked_lcm(&Scalar::from(2_u8)).unwrap(),
        Scalar::Null
    );
}

#[test]
fn factorial_private_kernel_uses_bound_float64_only() {
    use yggdryl::internals::arithmetic::checked_factorial;
    assert_eq!(checked_factorial(&Scalar::Null).unwrap(), Scalar::Null);
    for (input, expected) in [
        (0.0, 1.0),
        (5.0, 120.0),
        (20.0, 2_432_902_008_176_640_000.0),
    ] {
        assert_eq!(
            checked_factorial(&Scalar::from(input)).unwrap().as_f64(),
            Some(expected)
        );
    }
    assert_eq!(
        checked_factorial(&Scalar::from(171.0_f64))
            .unwrap()
            .as_f64(),
        Some(f64::INFINITY)
    );
    for input in [
        Scalar::from(-1.0_f64),
        Scalar::from(5.5_f64),
        Scalar::from(5_i64),
    ] {
        assert!(matches!(
            checked_factorial(&input),
            Err(Error::InvalidArithmetic {
                operation: "factorial",
                ..
            })
        ));
    }
}

#[cfg(feature = "internals")]
#[test]
fn annuity_factors_share_growth_with_dyadic_and_zero_rate_boundaries() {
    use yggdryl::internals::arithmetic::annuity_factors;
    assert_eq!(annuity_factors(1.0, 3.0, false), (8.0, 7.0));
    assert_eq!(annuity_factors(1.0, 3.0, true), (8.0, 14.0));
    assert_eq!(annuity_factors(1.0, -3.0, false), (0.125, -0.875));
    assert_eq!(annuity_factors(0.0, 3.0, true), (1.0, 3.0));
    assert_eq!(annuity_factors(-2.0, 3.0, false), (-1.0, 1.0));
    assert!(annuity_factors(-2.0, 0.5, false).0.is_nan());
    assert!(annuity_factors(100.0, 1000.0, false).0.is_infinite());
}

#[cfg(feature = "internals")]
#[test]
fn shared_trig_kernels_report_exact_native_bit_boundaries() {
    use std::collections::BTreeMap;
    use yggdryl::internals::arithmetic::{
        checked_acos, checked_atan, checked_atan2, checked_sin, checked_tan,
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../excel/fixtures/trig_kernel_native.json")).unwrap();
    assert_eq!(fixture["kind"], "trig_kernel_native_controls");
    assert_eq!(fixture["native_1900_and_1904_equal"], true);
    let rows = fixture["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 14);
    let mut exact = BTreeMap::<&str, usize>::new();
    let mut different = BTreeMap::<&str, Vec<&str>>::new();
    for row in rows {
        let function = row["function"].as_str().unwrap();
        let id = row["id"].as_str().unwrap();
        let args: Vec<f64> = row["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| {
                let bits = u64::from_str_radix(arg["bits"].as_str().unwrap(), 16).unwrap();
                f64::from_bits(bits)
            })
            .collect();
        let actual = match function {
            "ACOS" => checked_acos(&Scalar::from(args[0])),
            "ATAN" => checked_atan(&Scalar::from(args[0])),
            "ATAN2" => checked_atan2(&Scalar::from(args[1]), &Scalar::from(args[0])),
            "SIN" => checked_sin(&Scalar::from(args[0])),
            "TAN" => checked_tan(&Scalar::from(args[0])),
            other => panic!("unexpected native function {other}"),
        }
        .unwrap()
        .as_f64()
        .unwrap();
        let expected =
            u64::from_str_radix(row["expected_ieee754_hex"].as_str().unwrap(), 16).unwrap();
        if actual.to_bits() == expected {
            *exact.entry(function).or_default() += 1;
        } else {
            different.entry(function).or_default().push(id);
        }
    }
    assert_eq!(exact["ATAN"], 2, "{different:?}");
    assert_eq!(exact["ATAN2"], 3, "{different:?}");
    // ACOS/SIN/TAN differences are diagnostics for this target, not a
    // portable contract on the host math library's last bit.
    assert_eq!(
        exact.values().sum::<usize>() + different.values().map(Vec::len).sum::<usize>(),
        14
    );
}
