//! `rust/src/excel/formula/number.rs`: finite result boundaries and the design's shared fifteen-digit equality; native arithmetic and ROUND observations are a separate pending gate.

#[cfg(feature = "internals")]
mod internal {
    use serde_json::Value;
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::{equal, finite, round};

    #[test]
    fn normalization_compares_significant_digits_without_rounding_each_operation() {
        for (left, right, expected) in [
            (0.1 + 0.2, 0.3, true),
            (1.0, 1.0 + 2.0_f64.powi(-49), true),
            (1.0, 1.0 + 2.0_f64.powi(-46), false),
            (123_456_789_012_345.0, 123_456_789_012_346.0, false),
            (-0.1 - 0.2, -0.3, true),
            (-0.3, 0.3, false),
            (0.0, -0.0, true),
            (0.0, f64::MIN_POSITIVE, false),
        ] {
            assert_eq!(equal(left, right), Ok(expected), "{left:?}, {right:?}");
            assert_eq!(
                equal(right, left),
                Ok(expected),
                "reverse {left:?}, {right:?}"
            );
        }
        // A value boundary preserves the binary result; normalization is
        // a comparison policy, not an extra rounding pass after addition.
        let binary = 0.1 + 0.2;
        assert_ne!(binary, 0.3);
        assert_eq!(finite(binary).unwrap().to_bits(), binary.to_bits());
    }

    #[test]
    fn finite_limits_do_not_overflow_when_only_the_decimal_is_compared() {
        let max_before = f64::from_bits(f64::MAX.to_bits() - 1);
        let min_after = f64::from_bits(f64::MIN_POSITIVE.to_bits() + 1);
        for (left, right) in [
            (f64::MAX, max_before),
            (-f64::MAX, -max_before),
            (f64::MIN_POSITIVE, min_after),
            (-f64::MIN_POSITIVE, -min_after),
        ] {
            assert_eq!(equal(left, right), Ok(true), "{left:?}, {right:?}");
        }
        assert_eq!(equal(f64::MAX, f64::MAX / 2.0), Ok(false));
        assert_eq!(equal(f64::MAX, -f64::MAX), Ok(false));
    }

    #[test]
    fn underflow_and_negative_zero_have_the_one_positive_zero_representation() {
        let last_subnormal = f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1);
        for value in [
            0.0,
            -0.0,
            f64::from_bits(1),
            -f64::from_bits(1),
            last_subnormal,
            -last_subnormal,
        ] {
            assert_eq!(finite(value).unwrap().to_bits(), 0_u64, "{value:?}");
            assert_eq!(equal(value, 0.0), Ok(true));
        }
        for value in [
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::MAX,
            -f64::MAX,
            1.25,
            -1.25,
        ] {
            assert_eq!(
                finite(value).unwrap().to_bits(),
                value.to_bits(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn nonfinite_values_are_numeric_errors_even_when_their_bits_match() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX * 2.0] {
            assert_eq!(finite(value), Err(ExcelError::Num));
            assert_eq!(equal(value, value), Err(ExcelError::Num));
            assert_eq!(equal(value, 1.0), Err(ExcelError::Num));
            assert_eq!(equal(1.0, value), Err(ExcelError::Num));
        }
    }

    fn number(bits: &str) -> f64 {
        f64::from_bits(u64::from_str_radix(bits, 16).unwrap())
    }

    #[test]
    fn native_round_results_match_bits_and_errors() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/round_native.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 42);
        for case in cases {
            let formula = case["formula"].as_str().unwrap();
            let input = case["argument_bits"].as_str().unwrap();
            let places = case["places_bits"].as_str().unwrap();
            let basis = case["argument_basis"]["kind"].as_str().unwrap();
            if basis == "observed_identity" {
                assert_eq!(
                    case["argument_basis"]["observed_bits"].as_str(),
                    Some(input)
                );
            } else {
                assert_eq!(basis, "direct_decimal_token_candidate");
            }
            let actual = round(number(input), number(places)).expect(formula);
            if let Some(expected) = case["expected"]["bits"].as_str() {
                assert_eq!(
                    actual.unwrap().to_bits(),
                    number(expected).to_bits(),
                    "{formula}"
                );
                assert_eq!(case["cache"]["type"].as_str(), Some("n"));
            } else {
                assert_eq!(case["expected"]["error"].as_str(), Some("#NUM!"));
                assert_eq!(actual, Err(ExcelError::Num), "{formula}");
                assert_eq!(case["cache"]["type"].as_str(), Some("e"));
            }
        }
    }
}

#[cfg(feature = "internals")]
mod native_arithmetic {
    use serde_json::Value;
    use yggdryl::internals::excel_formula_number::add;

    fn number(bits: &str) -> f64 {
        f64::from_bits(u64::from_str_radix(bits, 16).unwrap())
    }

    #[test]
    fn root_seven_ulp_residual_is_zero_but_eight_is_retained() {
        let first = 1.0;
        let near_seven = number("3feffffffffffff2");
        let near_eight = number("3feffffffffffff0");
        let raw_seven = first - near_seven;
        let raw_eight = first - near_eight;
        assert_eq!(add(first, -near_seven, true).unwrap().unwrap().to_bits(), 0);
        assert_eq!(
            add(first, -near_eight, true).unwrap().unwrap().to_bits(),
            raw_eight.to_bits()
        );
        assert_eq!(
            add(first, -near_seven, false).unwrap().unwrap().to_bits(),
            raw_seven.to_bits()
        );
    }

    #[test]
    fn transient_subnormal_addition_requires_raw_value_and_cache_policy() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/subnormal_native.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 16);
        for case in cases {
            assert_eq!(case["value2_before_bits"], case["value2_after_bits"]);
            assert_eq!(case["cache_type"], "n");
            assert_eq!(case["cache_formula"], case["formula"]);
        }
        let observed = |context: &str| {
            cases
                .iter()
                .find(|case| case["context"] == context)
                .unwrap()
        };
        let bits =
            |context: &str| number(observed(context)["value2_before_bits"].as_str().unwrap());
        let left = bits("first");
        let right = bits("second");
        assert_eq!((left + right).to_bits(), bits("pair-direct").to_bits());
        assert!(bits("pair-direct").is_subnormal());
        assert_eq!(observed("pair-direct")["cache_text"], "0");
        assert_eq!(
            (left + right + bits("third")).to_bits(),
            bits("fold-direct").to_bits()
        );
        assert!(bits("fold-direct").is_normal());
        for context in ["multiply", "divide", "percent"] {
            assert_eq!(bits(context).to_bits(), 0);
            assert_eq!(observed(context)["cache_text"], "0");
        }
        assert!(add(left, right, false).is_none());
        assert!(add(f64::from_bits(1), 1.0, true).is_none());
        assert_eq!(add(-0.0, 0.0, true).unwrap().unwrap().to_bits(), 0);
    }

    #[test]
    fn native_binary_cases_match_exact_bits() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/binary_native.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 1380);
        let mut mismatches = Vec::new();
        for case in cases {
            let left = number(case["left_bits"].as_str().unwrap());
            let right = number(case["right_bits"].as_str().unwrap());
            let right = match case["operator"].as_str().unwrap() {
                "add" => right,
                "sub" => -right,
                other => panic!("unexpected operator {other}"),
            };
            let root = match case["context"].as_str().unwrap() {
                "root" => true,
                "outer_group" | "nested_multiply_by_one" => false,
                other => panic!("unexpected context {other}"),
            };
            let actual = add(left, right, root)
                .expect("native normal case")
                .unwrap()
                .to_bits();
            let expected = number(case["expected_bits"].as_str().unwrap()).to_bits();
            if actual != expected {
                mismatches.push(format!(
                    "{}: {actual:016x} != {expected:016x}",
                    case["formula"].as_str().unwrap()
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} mismatches of {}: {}",
            mismatches.len(),
            cases.len(),
            mismatches
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn literal_limit_does_not_narrow_native_computed_finite_results() {
    use yggdryl::internals::excel_formula_number::finite;

    let native: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/literal_precision_excel.json")).unwrap();
    for case in native["computed_controls"].as_array().unwrap() {
        let bits = u64::from_str_radix(case["bits"].as_str().unwrap(), 16).unwrap();
        let value = f64::from_bits(bits);
        assert_eq!(value.abs(), f64::MAX);
        assert_eq!(finite(value).unwrap().to_bits(), bits, "{}", case["id"]);
    }
}

#[cfg(feature = "internals")]
#[test]
fn native_modulus_fixture_covers_every_observed_operand_pair() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::modulus;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/math_mod_boundary_native.json")).unwrap();
    assert_eq!(fixture["excel_version"], "16.0");
    assert_eq!(fixture["excel_build"], "20430.0");
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["groups"].as_array().unwrap().len(), 3);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 252);
    let (mut numeric, mut num_error, mut div_zero, mut held) = (0, 0, 0, 0);
    for case in cases {
        let formula = case["formula"].as_str().unwrap();
        // Excel canonicalizes literals (including subnormals to zero). The
        // independently observed operand bits below, not formula spelling,
        // are the input to this primitive comparison.
        let native_formula = case["native_formula"].as_str().unwrap();
        assert!(native_formula.starts_with("MOD(") && native_formula.ends_with(')'));
        let bits = |key: &str| {
            f64::from_bits(u64::from_str_radix(case[key].as_str().unwrap(), 16).unwrap())
        };
        let (left, right) = (bits("left_bits"), bits("right_bits"));
        let quotient = (left / right).abs();
        let actual = modulus(left, right);
        if right != 0.0 && (left.is_subnormal() || right.is_subnormal()) {
            assert_eq!(actual, None, "{formula}");
            held += 1;
            continue;
        }
        if quotient > 1_099_511_627_776.0 && quotient < 2_199_023_255_552.0 {
            assert_eq!(actual, None, "{formula}");
            held += 1;
            continue;
        }
        match case["cache_type"].as_str().unwrap() {
            "n" => {
                let expected_bits =
                    u64::from_str_radix(case["value2"]["ieee754_hex"].as_str().unwrap(), 16)
                        .unwrap();
                assert_eq!(
                    actual.unwrap().unwrap().to_bits(),
                    expected_bits,
                    "{formula}"
                );
                numeric += 1;
            }
            "e" => {
                let expected = match case["cache_text"].as_str().unwrap() {
                    "#NUM!" => {
                        num_error += 1;
                        ExcelError::Num
                    }
                    "#DIV/0!" => {
                        div_zero += 1;
                        ExcelError::Div0
                    }
                    other => panic!("unexpected native MOD error {other}: {formula}"),
                };
                assert_eq!(actual, Some(Err(expected)), "{formula}");
            }
            other => panic!("unexpected native MOD cache {other}: {formula}"),
        }
    }
    // Thirty uncertain quotient cases and one subnormal-input case are held.
    assert_eq!((numeric, num_error, div_zero, held), (148, 71, 2, 31));
}
