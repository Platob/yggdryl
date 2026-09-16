//! Exact decimals: what they hold, how they compare, and how they restate.

mod representation {
    use yggdryl::Scalar;
    use yggdryl::i256;

    #[test]
    fn a_decimal_keeps_the_coefficient_and_the_scale_it_was_given() {
        let price = Scalar::d128(1_050, 2);

        assert_eq!(price.as_d128(), Some((1_050, 2)));
        assert!(price.is_decimal());
        assert_eq!(price.kind(), "d128");
        assert!(!Scalar::from(10.5).is_decimal());
    }

    #[test]
    fn a_decimal_holds_a_fraction_no_double_can_hold() {
        // The point of the variant: one tenth has no finite binary expansion,
        // so a float can only ever hold the nearest double to it.
        let exact = Scalar::d128(1, 1);
        assert_eq!(exact.as_d128(), Some((1, 1)));
        assert_ne!(f64::from(0.1_f32), 0.1_f64);

        // The full 128-bit coefficient survives, which is what a float cannot do.
        assert_eq!(Scalar::d128(i128::MAX, 0).as_d128(), Some((i128::MAX, 0)));
    }

    #[test]
    fn one_renderer_restores_the_exact_plain_text() {
        assert_eq!(
            Scalar::d128(1_050, 2).into_decimal_utf8().as_deref(),
            Some("10.50")
        );
        assert_eq!(
            Scalar::d128(-5, 3).into_decimal_utf8().as_deref(),
            Some("-0.005")
        );
        assert_eq!(
            Scalar::d128(12, -2).into_decimal_utf8().as_deref(),
            Some("1200")
        );
        assert_eq!(
            Scalar::d256(yggdryl::i256::from_i128(1_050), 2)
                .into_decimal_utf8()
                .as_deref(),
            Some("10.50")
        );
    }

    #[test]
    fn generic_decimal_selects_width_and_has_one_family_view() {
        let narrow = Scalar::from_decimal(i256::from_i128(i128::MAX), 2);
        let wide = Scalar::from_decimal(
            "170141183460469231731687303715884105728".parse().unwrap(),
            3,
        );
        let narrow_minimum = Scalar::from_decimal(i256::from_i128(i128::MIN), -2);
        let wide_negative = Scalar::from_decimal(
            "-170141183460469231731687303715884105729".parse().unwrap(),
            -3,
        );

        assert!(narrow.as_d128().is_some());
        assert!(wide.as_d256().is_some());
        assert!(narrow_minimum.as_d128().is_some());
        assert!(wide_negative.as_d256().is_some());
        assert_eq!(narrow.as_decimal(), Some((i256::from_i128(i128::MAX), 2)));
        assert_eq!(wide.as_decimal().map(|parts| parts.1), Some(3));
        assert_eq!(wide_negative.as_decimal().map(|parts| parts.1), Some(-3));
        assert!(Scalar::from(1).as_decimal().is_none());
    }
}

mod comparison {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use yggdryl::Scalar;

    fn hash(value: &Scalar) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn two_spellings_of_one_number_are_one_value() {
        for (left, right) in [
            (Scalar::d128(1_050, 2), Scalar::d128(105, 1)),
            (Scalar::d128(0, 7), Scalar::d128(0, -7)),
            (Scalar::d128(-1_050, 2), Scalar::d128(-105, 1)),
            (Scalar::d128(100, 0), Scalar::d128(1, -2)),
        ] {
            assert_eq!(left, right, "{left:?} == {right:?}");
            assert_eq!(hash(&left), hash(&right), "{left:?} hashes as {right:?}");
        }
    }

    #[test]
    fn decimals_order_by_the_number_they_name() {
        assert!(Scalar::d128(1, 1) < Scalar::d128(2, 1));
        assert!(Scalar::d128(-1, 0) < Scalar::d128(1, 5));
        // 0.001 is smaller than 1 even though its coefficient is not.
        assert!(Scalar::d128(1, 3) < Scalar::d128(1, 0));
        // A coefficient too wide to restate is by that fact the larger one.
        assert!(Scalar::d128(i128::MAX, 0) > Scalar::d128(1, -30));
        assert!(Scalar::d128(i128::MIN, 0) < Scalar::d128(-1, -30));
    }

    #[test]
    fn a_decimal_is_its_own_kind_and_never_an_integer() {
        // A decimal carries a scale, an integer does not, and Arrow keeps them
        // apart too - so equality does not quietly merge them.
        assert_ne!(Scalar::d128(1, 0), Scalar::from(1_i64));
        assert!(Scalar::from(1_i64) < Scalar::d128(1, 0));
    }
}

mod restating {
    use yggdryl::Scalar;

    #[test]
    fn restating_adds_digits_freely_and_drops_none() {
        let price = Scalar::d128(1_050, 2);

        assert_eq!(price.decimal_unscaled_at(2), Some(1_050));
        assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
        // 10.50 has a zero to spare, so scale 1 is exact...
        assert_eq!(price.decimal_unscaled_at(1), Some(105));
        // ... and scale 0 is not, because 10.5 is not a whole number.
        assert_eq!(price.decimal_unscaled_at(0), None);

        // A coefficient that would no longer fit is refused, not wrapped.
        assert_eq!(Scalar::d128(i128::MAX, 0).decimal_unscaled_at(1), None);
        // Only a decimal restates at all.
        assert_eq!(Scalar::from(105_i64).decimal_unscaled_at(1), None);
    }
}

mod fixed {
    use yggdryl::types::Decimal;
    use yggdryl::{DataType, Scalar};

    #[test]
    fn a_fixed_decimal_is_decimal128_at_eighteen_digits_already_applied() {
        let px: Decimal = "82.5".parse().unwrap();
        assert_eq!(px.units(), 82_500_000_000_000_000_000);
        assert_eq!(Decimal::from_units(px.units()), Some(px));
        assert_eq!(px.to_string(), "82.5");
        assert_eq!(Decimal::from_int(100).to_string(), "100");
        assert_eq!(Decimal::ZERO.to_string(), "0");
        assert_eq!(
            "-0.000000000000000001".parse::<Decimal>().unwrap().units(),
            -1
        );
        assert_eq!(Decimal::dtype(), DataType::decimal128(38, 18).unwrap());
        assert_eq!(DataType::DECIMAL, DataType::decimal128(38, 18).unwrap());
        // The scalar is the column's value, and reads back.
        let scalar = Scalar::from(px);
        assert_eq!(scalar.as_d128(), Some((px.units(), 18)));
        assert_eq!(Decimal::from_scalar(&scalar), Some(px));
        assert_eq!(Decimal::from_scalar(&Scalar::d128(825, 1)), Some(px));
        assert_eq!(Decimal::from_scalar(&Scalar::from(82.5_f64)), Some(px));
        assert_eq!(
            Decimal::from_scalar(&Scalar::from(100_i64)),
            Some(Decimal::from_int(100))
        );
        assert_eq!(Decimal::from_scalar(&Scalar::from("82.5")), Some(px));
        assert_eq!(px.to_f64(), 82.5);
        assert_eq!(Decimal::from_f64(82.5), Some(px));
        assert_eq!(Decimal::from_f64(f64::NAN), None);
    }

    #[test]
    fn fixed_arithmetic_is_exact_bounded_and_truncates_toward_zero() {
        let px: Decimal = "82.5".parse().unwrap();
        let qty = Decimal::from_int(1_000);
        assert_eq!((px * qty).to_string(), "82500");
        assert_eq!((px + Decimal::from_int(1)).to_string(), "83.5");
        assert_eq!((px - Decimal::from_int(100)).to_string(), "-17.5");
        assert_eq!((px / Decimal::from_int(4)).to_string(), "20.625");
        assert_eq!((-px).abs(), px);
        assert!(px.is_positive() && (-px).is_negative() && Decimal::ZERO.is_zero());
        // A third is truncated at the eighteenth digit, never rounded up.
        assert_eq!(
            (Decimal::ONE / Decimal::from_int(3)).to_string(),
            "0.333333333333333333"
        );
        assert_eq!(
            Decimal::parse("1.123456789")
                .unwrap()
                .truncated(4)
                .to_string(),
            "1.1234"
        );
        // Past thirty-eight digits there is no value.
        assert_eq!(Decimal::MAX.checked_add(Decimal::ONE), None);
        assert_eq!(Decimal::MAX.checked_mul(Decimal::from_int(2)), None);
        assert_eq!(Decimal::ONE.checked_div(Decimal::ZERO), None);
        assert_eq!(Decimal::MIN.checked_sub(Decimal::ONE), None);
        assert_eq!(Decimal::from_units(i128::MAX), None);
        // Sums fold as integers do.
        assert_eq!(
            [px, px, px].into_iter().sum::<Decimal>().to_string(),
            "247.5"
        );
        let mut held = px;
        held += Decimal::from_int(1);
        held *= Decimal::from_int(2);
        assert_eq!(held.to_string(), "167");
    }

    #[test]
    fn fixed_text_is_read_as_leniently_as_a_number_can_be() {
        for (text, expected) in [
            ("", "0"),
            ("  82.5\t", "82.5"),
            ("+1.50", "1.5"),
            ("-0.000000000000000001", "-0.000000000000000001"),
            (".5", "0.5"),
            ("5.", "5"),
            ("1,250,000.25", "1250000.25"),
            ("1_000", "1000"),
            ("1'000", "1000"),
            ("1 000", "1000"),
            ("2.5e3", "2500"),
            ("2.5E+3", "2500"),
            ("1E-2", "0.01"),
            ("125e-1", "12.5"),
            // Digits past the eighteenth fractional one are truncated.
            ("0.1234567890123456789", "0.123456789012345678"),
            ("-0.9999999999999999999", "-0.999999999999999999"),
            ("1e-30", "0"),
            (
                "99999999999999999999.999999999999999999",
                "99999999999999999999.999999999999999999",
            ),
        ] {
            assert_eq!(
                Decimal::parse(text).unwrap().to_string(),
                expected,
                "{text:?}"
            );
        }
        for refused in [
            "-",
            "+",
            ".",
            "1.2.3",
            "abc",
            "NaN",
            "inf",
            "1e",
            "1e+",
            "1e1.5",
            ",1",
            "1..",
            "100000000000000000000",
            "1e21",
            "1e400",
        ] {
            assert!(Decimal::parse(refused).is_err(), "{refused:?}");
        }
        // Text held as a scalar reads the same way.
        assert_eq!(
            Decimal::from_scalar(&Scalar::from("1,250.5"))
                .unwrap()
                .to_string(),
            "1250.5"
        );
        assert_eq!(Decimal::from_scalar(&Scalar::from("x")), None);
    }
}
