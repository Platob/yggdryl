//! `rust/src/decimal.rs`: the decimal text reader every door reads through.

use super::typed;

#[cfg(feature = "internals")]
mod internal {
    mod reading {
        use yggdryl::internals::decimal::from_decimal_text;
        use yggdryl::{DataType, Scalar};

        fn money() -> DataType {
            "decimal128(10, 2)".parse().unwrap()
        }

        /// The empty text is no spelling: the empty-cell rule sits above this
        /// reader, on the doors, and the reader itself keeps refusing it.
        #[test]
        fn the_empty_text_is_no_decimal_spelling() {
            assert!(from_decimal_text(&money(), "").is_err());
        }

        #[test]
        fn text_is_restated_exactly_at_the_declared_scale() {
            assert_eq!(
                from_decimal_text(&money(), "10.50").unwrap(),
                Scalar::d128(1_050, 2)
            );
            assert_eq!(
                from_decimal_text(&money(), "1.05e1").unwrap(),
                Scalar::d128(1_050, 2)
            );
            // A digit the scale cannot hold is refused rather than rounded away,
            // and that refusal is a reading rather than a parse failure, so no
            // other reader is allowed to round it either.
            let refused = from_decimal_text(&money(), "1.005").unwrap_err();
            assert!(
                matches!(refused, yggdryl::Error::InvalidRecord { .. }),
                "{refused:?}"
            );
            // Text that is not a decimal at all did not read, so it may be tried
            // by a wider reader.
            let unread = from_decimal_text(&money(), "ten").unwrap_err();
            assert!(matches!(unread, yggdryl::Error::Parse { .. }), "{unread:?}");
        }
    }
}

mod selection {

    use yggdryl::{DataType, Error};

    #[test]
    fn decimal_selects_the_smallest_arrow_representation() {
        assert_eq!(
            DataType::decimal(1, 0).unwrap(),
            DataType::Decimal32 {
                precision: 1,
                scale: 0,
            }
        );
        assert_eq!(
            DataType::decimal(10, 2).unwrap(),
            DataType::Decimal64 {
                precision: 10,
                scale: 2,
            }
        );
        assert_eq!(
            DataType::decimal(19, 2).unwrap(),
            DataType::Decimal128 {
                precision: 19,
                scale: 2,
            }
        );
        assert_eq!(
            DataType::decimal(38, 38).unwrap(),
            DataType::Decimal128 {
                precision: 38,
                scale: 38,
            }
        );
        assert_eq!(
            DataType::decimal(39, 39).unwrap(),
            DataType::Decimal256 {
                precision: 39,
                scale: 39,
            }
        );
        assert_eq!(
            DataType::decimal(76, -20).unwrap(),
            DataType::Decimal256 {
                precision: 76,
                scale: -20,
            }
        );
    }

    #[test]
    fn decimal_uses_the_selected_constructor_validation() {
        for (precision, scale, expected_kind) in [
            (0, 0, "Decimal32"),
            (12, 13, "Decimal64"),
            (39, 40, "Decimal256"),
            (77, 0, "Decimal256"),
        ] {
            let error = DataType::decimal(precision, scale).unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::InvalidDataType { kind, .. } if kind == expected_kind
                ),
                "unexpected error for decimal({precision},{scale}): {error}"
            );
        }
    }

    #[test]
    fn decimal_preserves_existing_negative_scale_rules() {
        assert_eq!(
            DataType::decimal(9, i8::MIN).unwrap(),
            DataType::decimal32(9, i8::MIN).unwrap()
        );
        assert_eq!(
            DataType::decimal(18, i8::MIN).unwrap(),
            DataType::decimal64(18, i8::MIN).unwrap()
        );
        assert_eq!(
            DataType::decimal(38, i8::MIN).unwrap(),
            DataType::decimal128(38, i8::MIN).unwrap()
        );
        assert_eq!(
            DataType::decimal(39, i8::MIN).unwrap(),
            DataType::decimal256(39, i8::MIN).unwrap()
        );
    }

    #[test]
    fn generic_decimal_parser_selects_and_round_trips_physical_storage() {
        for (expression, expected) in [
            (
                "decimal(9,2)",
                DataType::Decimal32 {
                    precision: 9,
                    scale: 2,
                },
            ),
            (
                "decimal(10,2)",
                DataType::Decimal64 {
                    precision: 10,
                    scale: 2,
                },
            ),
            (
                "decimal(38,18)",
                DataType::Decimal128 {
                    precision: 38,
                    scale: 18,
                },
            ),
            (
                "decimal(39,18)",
                DataType::Decimal256 {
                    precision: 39,
                    scale: 18,
                },
            ),
            (
                "numeric(76,20)",
                DataType::Decimal256 {
                    precision: 76,
                    scale: 20,
                },
            ),
        ] {
            let parsed = DataType::from_str(expression).unwrap();
            assert_eq!(parsed, expected, "{expression}");
            assert_eq!(DataType::from_str(&parsed.to_string()).unwrap(), parsed);
        }

        assert!(DataType::from_str("decimal(77,0)").is_err());
        assert!(DataType::from_str("decimal128(39,0)").is_err());
    }
}

mod exact {
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
        use yggdryl::Decimal18;
        use yggdryl::{DataType, Scalar};

        #[test]
        fn a_fixed_decimal_is_decimal128_at_eighteen_digits_already_applied() {
            let px: Decimal18 = "82.5".parse().unwrap();
            assert_eq!(px.units(), 82_500_000_000_000_000_000);
            assert_eq!(Decimal18::from_units(px.units()), Some(px));
            assert_eq!(px.to_string(), "82.5");
            assert_eq!(Decimal18::from_int(100).to_string(), "100");
            assert_eq!(Decimal18::ZERO.to_string(), "0");
            assert_eq!(
                "-0.000000000000000001"
                    .parse::<Decimal18>()
                    .unwrap()
                    .units(),
                -1
            );
            assert_eq!(Decimal18::dtype(), DataType::decimal128(38, 18).unwrap());
            assert_eq!(DataType::DECIMAL, DataType::decimal128(38, 18).unwrap());
            // The scalar is the column's value, and reads back.
            let scalar = Scalar::from(px);
            assert_eq!(scalar.as_d128(), Some((px.units(), 18)));
            assert_eq!(Decimal18::from_scalar(&scalar), Some(px));
            assert_eq!(Decimal18::from_scalar(&Scalar::d128(825, 1)), Some(px));
            assert_eq!(Decimal18::from_scalar(&Scalar::from(82.5_f64)), Some(px));
            assert_eq!(
                Decimal18::from_scalar(&Scalar::from(100_i64)),
                Some(Decimal18::from_int(100))
            );
            assert_eq!(Decimal18::from_scalar(&Scalar::from("82.5")), Some(px));
            assert_eq!(px.to_f64(), 82.5);
            assert_eq!(Decimal18::from_f64(82.5), Some(px));
            assert_eq!(Decimal18::from_f64(f64::NAN), None);
        }

        #[test]
        fn fixed_arithmetic_is_exact_bounded_and_truncates_toward_zero() {
            let px: Decimal18 = "82.5".parse().unwrap();
            let qty = Decimal18::from_int(1_000);
            assert_eq!((px * qty).to_string(), "82500");
            assert_eq!((px + Decimal18::from_int(1)).to_string(), "83.5");
            assert_eq!((px - Decimal18::from_int(100)).to_string(), "-17.5");
            assert_eq!((px / Decimal18::from_int(4)).to_string(), "20.625");
            assert_eq!((-px).abs(), px);
            assert!(px.is_positive() && (-px).is_negative() && Decimal18::ZERO.is_zero());
            // A third is truncated at the eighteenth digit, never rounded up.
            assert_eq!(
                (Decimal18::ONE / Decimal18::from_int(3)).to_string(),
                "0.333333333333333333"
            );
            assert_eq!(
                Decimal18::parse("1.123456789")
                    .unwrap()
                    .truncated(4)
                    .to_string(),
                "1.1234"
            );
            // Past thirty-eight digits there is no value.
            assert_eq!(Decimal18::MAX.checked_add(Decimal18::ONE), None);
            assert_eq!(Decimal18::MAX.checked_mul(Decimal18::from_int(2)), None);
            assert_eq!(Decimal18::ONE.checked_div(Decimal18::ZERO), None);
            assert_eq!(Decimal18::MIN.checked_sub(Decimal18::ONE), None);
            assert_eq!(Decimal18::from_units(i128::MAX), None);
            // Sums fold as integers do.
            assert_eq!(
                [px, px, px].into_iter().sum::<Decimal18>().to_string(),
                "247.5"
            );
            let mut held = px;
            held += Decimal18::from_int(1);
            held *= Decimal18::from_int(2);
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
                    Decimal18::parse(text).unwrap().to_string(),
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
                assert!(Decimal18::parse(refused).is_err(), "{refused:?}");
            }
            // Text held as a scalar reads the same way.
            assert_eq!(
                Decimal18::from_scalar(&Scalar::from("1,250.5"))
                    .unwrap()
                    .to_string(),
                "1250.5"
            );
            assert_eq!(Decimal18::from_scalar(&Scalar::from("x")), None);
        }
    }

    mod family {
        use yggdryl::{DataType, DataTypeKind, FamilyValue, Scalar, i256};
        use yggdryl::{Decimal, Decimal18, Decimal32, Decimal64, Decimal128, Decimal256};

        #[test]
        fn the_decimal_family_stands_for_every_width() {
            crate::scalar::assert_family_round_trip(
                vec![
                    crate::family_leaf!(Decimal::Decimal32, Decimal32::new(1_250, 2)),
                    crate::family_leaf!(Decimal::Decimal64, Decimal64::new(-7, 1)),
                    crate::family_leaf!(Decimal::Decimal128, Decimal128::new(125, 1)),
                    crate::family_leaf!(
                        Decimal::Decimal256,
                        Decimal256::new(i256::from_i128(-125), 3)
                    ),
                ],
                DataTypeKind::Decimal,
                &Scalar::from(3_i64),
            );

            // The fixed-scale money value is one decimal128 leaf of the family,
            // and `as_decimal` stays the coefficient-and-scale reader beside it.
            // The value's own datatype holds the digits it has at scale 18; the
            // column a `Decimal18` declares, `DataType::DECIMAL`, is the wider
            // decimal128 it is stored in.
            let money = Scalar::from(Decimal18::from_int(3));
            let held = Decimal::from_scalar(&money).unwrap();
            assert!(matches!(held, Decimal::Decimal128(_)), "{held:?}");
            assert_eq!(held.dtype().unwrap(), DataType::decimal128(19, 18).unwrap());
            assert_eq!(
                money.as_decimal(),
                Some((i256::from_i128(3_000_000_000_000_000_000), 18))
            );
        }
    }
}

mod fields {
    use yggdryl::{DataType, DataTypeId, Scalar};
    use yggdryl::{DataTypeValue, DecimalField, DecimalType};

    use super::typed::assert_typed_marker;

    #[test]
    fn the_decimal_marker_covers_every_backing_width() {
        for dtype in [
            DataType::decimal32(9, 2).unwrap(),
            DataType::decimal64(18, 2).unwrap(),
            DataType::decimal128(38, 2).unwrap(),
            DataType::decimal256(76, 2).unwrap(),
        ] {
            assert_typed_marker::<DecimalType>(dtype);
        }
    }

    #[test]
    fn a_leaf_is_the_width_and_the_family_is_what_the_number_means() {
        // The leaf says which integer holds the coefficient; the precision and
        // the scale say what the column means, and both are read without a match.
        let narrow = DecimalType::Decimal32 {
            precision: 9,
            scale: 2,
        };
        assert_eq!(narrow.precision(), 9);
        assert_eq!(narrow.scale(), 2);
        assert_eq!(narrow.maximum(), 9);
        assert_eq!(narrow.id(), DataTypeId::Decimal32);
        assert_eq!(narrow.to_string(), "decimal32(9,2)");

        // `decimal` picks the narrowest width that holds the digits, and the four
        // named constructors are the same rule with the width already chosen.
        for (precision, width) in [
            (1_u8, DataTypeId::Decimal32),
            (9, DataTypeId::Decimal32),
            (10, DataTypeId::Decimal64),
            (18, DataTypeId::Decimal64),
            (19, DataTypeId::Decimal128),
            (38, DataTypeId::Decimal128),
            (39, DataTypeId::Decimal256),
            (76, DataTypeId::Decimal256),
        ] {
            assert_eq!(DataType::decimal(precision, 0).unwrap().id(), width);
            assert_eq!(DecimalType::narrowest(precision, 0).id(), width);
        }

        // Each width states its own maximum, and a precision past it is refused
        // by the name of the width that could not hold it.
        assert!(DataType::decimal32(10, 0).is_err());
        assert!(DataType::decimal64(19, 0).is_err());
        assert!(DataType::decimal128(39, 0).is_err());
        assert!(DataType::decimal256(77, 0).is_err());
        let refused = DataType::decimal32(10, 0).unwrap_err().to_string();
        assert!(refused.contains("Decimal32"), "{refused}");
        // A positive scale cannot exceed the digits it is taken out of.
        assert!(DataType::decimal128(4, 5).is_err());
        // The variants stay public, so an unchecked one is caught at the boundary.
        assert!(
            DataTypeValue::validate(&DecimalType::Decimal32 {
                precision: 10,
                scale: 0
            })
            .is_err()
        );
    }

    #[test]
    fn a_decimal_field_holds_its_own_leaf_and_the_root_redirects_to_it() {
        let field =
            DecimalField::try_new("price", DataType::decimal128(38, 18).unwrap(), false).unwrap();
        assert_eq!(field.typed_dtype_ref().precision(), 38);
        assert_eq!(field.typed_dtype_ref().scale(), 18);
        assert_eq!(field.dtype(), &DataType::decimal128(38, 18).unwrap());
        assert_eq!(field.dtype().id(), DataTypeId::Decimal128);

        // A datatype from another family is refused by name.
        assert!(DecimalField::try_new("price", DataType::Int64, false).is_err());

        // The value door is the family's, whichever width the column is.
        let held = field.to_field().scalar(Scalar::from("1.5")).unwrap();
        assert_eq!(held.id(), DataTypeId::Decimal128);
    }
}
