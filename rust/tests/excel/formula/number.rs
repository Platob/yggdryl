//! `rust/src/excel/formula/number.rs`: finite result boundaries and the design's shared fifteen-digit equality; native arithmetic and ROUND observations are a separate pending gate.

#[cfg(feature = "internals")]
mod internal {
    use serde_json::Value;
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::{
        equal, finite, parity_round, quotient, round, round_direction,
    };

    #[test]
    fn directional_rounding_reuses_decimal_and_raw_quotient_boundaries() {
        assert_eq!(round_direction(3.2, -2.0, true), Some(Ok(100.0)));
        assert_eq!(round_direction(-3.2, -2.0, true), Some(Ok(-100.0)));
        assert_eq!(round_direction(3.2, -2.0, false), Some(Ok(0.0)));
        assert_eq!(round_direction(0.3, 0.1, true), Some(Ok(1.0)));
        assert_eq!(round_direction(0.3, 0.1, false), Some(Ok(0.0)));
        assert_eq!(
            round_direction(1.23456789012345, 1_000_000.0, true),
            Some(Ok(1.23456789012345))
        );
        assert_eq!(round_direction(f64::from_bits(1), 0.0, true), None);
        assert_eq!(parity_round(2.5, false), Some(Ok(4.0)));
        assert_eq!(parity_round(-2.5, true), Some(Ok(-3.0)));
        assert_eq!(parity_round(0.0, true), Some(Ok(1.0)));
        assert_eq!(quotient(0.3, 0.1), Some(Ok(2.0)));
        assert_eq!(quotient(-3.2, 2.0), Some(Ok(-1.0)));
        assert_eq!(quotient(1.0, 0.0), Some(Err(ExcelError::Div0)));
    }

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

#[cfg(feature = "internals")]
#[test]
fn ordered_comparisons_share_numeric_equality_and_finite_boundaries() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::compare;
    for (left, right, expected) in [
        (0.1 + 0.2, 0.3, Equal),
        (1.0, 1.0 + 2.0_f64.powi(-49), Equal),
        (1.0, 1.0 + 2.0_f64.powi(-46), Less),
        (-1.0, -2.0, Greater),
        (0.0, -0.0, Equal),
        (f64::from_bits(1), 0.0, Equal),
        (f64::MIN_POSITIVE, 0.0, Greater),
        (f64::MAX, f64::MAX / 2.0, Greater),
        (f64::MAX, f64::from_bits(f64::MAX.to_bits() - 1), Equal),
    ] {
        assert_eq!(compare(left, right), Ok(expected), "{left:?}/{right:?}");
        assert_eq!(compare(right, left), Ok(expected.reverse()));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(compare(value, 0.0), Err(ExcelError::Num));
        assert_eq!(compare(0.0, value), Err(ExcelError::Num));
    }
}

#[cfg(feature = "internals")]
#[test]
fn integer_floor_and_decimal_truncate_share_native_boundaries() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::{integer_floor, truncate};
    for (value, expected) in [
        (0.9999999999999998, 1.0_f64),
        (-0.9999999999999998, -1.0),
        (-1.2, -2.0),
        (123.456, 123.0),
    ] {
        assert_eq!(
            integer_floor(value).unwrap().unwrap().to_bits(),
            expected.to_bits()
        );
    }
    for (value, places, expected) in [
        (0.9999999999999998, 0.0, 1.0_f64),
        (-1.2, 0.0, -1.0),
        (-123.456, 1.0, -123.4),
        (123.456, -1.0, 120.0),
    ] {
        assert_eq!(
            truncate(value, places).unwrap().unwrap().to_bits(),
            expected.to_bits(),
            "{value:?}, {places}"
        );
    }
    assert_eq!(integer_floor(f64::INFINITY), Some(Err(ExcelError::Num)));
    assert_eq!(integer_floor(f64::from_bits(1)), None);
    assert_eq!(truncate(2.0, f64::NAN), None);
    assert_eq!(truncate(f64::from_bits(1), 0.0), None);
}

#[test]
fn native_rounding_family_replays_all_648_typed_source_and_function_cases() {
    super::native_function_fixture(include_str!("../fixtures/rounding_family_native.json"), 648);
}

#[test]
fn native_pure_math_first_slice_replays_164_typed_cases() {
    super::native_function_fixture(include_str!("../fixtures/pure_math_first_native.json"), 164);
}

#[test]
fn native_cosine_and_inverse_sine_replay_all_82_cases() {
    super::native_function_fixture(include_str!("../fixtures/pure_math_trig_native.json"), 82);
}

#[test]
fn native_trigonometry_large_angles_refuse_unsettled_reduction_and_report_domain_errors() {
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/trig_limit_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| {
                c["date_system"] == year
                    && ["COS", "SIN", "TAN"].contains(&c["shape"]["function"].as_str().unwrap())
            })
            .collect();
        assert_eq!(cases.len(), 27);
        for c in &cases {
            let at: CellRef = c["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, yggdryl::Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(c["wire_formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (15, 12, 0)
        );
        for c in &cases {
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(c["cell"].as_str().unwrap().parse().unwrap())
                .unwrap();
            if c["saved_cache"]["type"] == "e" {
                assert_eq!(
                    cell.error().unwrap().as_str(),
                    c["saved_cache"]["value_text"].as_str().unwrap()
                );
            } else {
                assert_eq!(cell.value().as_f64(), Some(-777.0));
            }
        }
    }
}

#[test]
fn native_atan_and_atan2_replay_158_cases() {
    super::native_function_fixture(include_str!("../fixtures/atan_native.json"), 158);
}

#[test]
fn native_remaining_trig_bounded_replay_50_cases() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/trig_bounded_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 100);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 50);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        let epoch: Vec<_> = cases
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(epoch.len(), 25);
        for case in &epoch {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            at,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (17, 8, 0),
            "{year}"
        );
        for case in &epoch {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            if !case["expect_computed"].as_bool().unwrap() {
                assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                continue;
            }
            let cache = &case["saved_cache"];
            match cache["type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64().unwrap().to_bits(),
                    cache["value_text"]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap()
                        .to_bits(),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    cache["value_text"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("unexpected native cache kind {other}"),
            }
        }
    }
}

#[test]
fn native_asin_negative_endpoint_preserves_cache_until_kernel_is_known() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/asin_endpoint_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 4);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let case = cases
            .iter()
            .find(|case| case["date_system"] == year)
            .unwrap();
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        let at: CellRef = "A1".parse().unwrap();
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), system)
                    .unwrap()
                    .with_formula(Formula::from_file(
                        case["wire_formula"].as_str().unwrap(),
                        at,
                    )),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (0, 1, 0),
            "{year}"
        );
        let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
        assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
        assert_ne!(
            case["saved_cache"]["value_text"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                .to_bits(),
            (-777.0_f64).to_bits()
        );
    }
}

#[test]
fn native_logarithm_replays_62_scalar_and_48_base_cases() {
    super::native_function_fixture(include_str!("../fixtures/logarithm_native.json"), 62);
    super::native_function_fixture(include_str!("../fixtures/logarithm_base_native.json"), 48);
}

#[cfg(feature = "internals")]
#[test]
fn financial_annuity_numeric_domains_keep_errors_and_unsettled_underflow_distinct() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::annuity;
    assert_eq!(
        annuity(false, [-1.0, 10.0, 100.0, 0.0, 0.0]),
        Some(Err(ExcelError::Div0))
    );
    assert_eq!(
        annuity(true, [-1.0, 10.0, 100.0, 0.0, 0.0]),
        Some(Ok(-100.0))
    );
    assert_eq!(
        annuity(true, [-2.0, 2.5, 100.0, 0.0, 0.0]),
        Some(Err(ExcelError::Num))
    );
    assert_eq!(
        annuity(false, [100.0, 1000.0, 100.0, 0.0, 0.0]),
        Some(Err(ExcelError::Num))
    );
    assert_eq!(
        annuity(true, [f64::from_bits(1), 10.0, 1.0, 0.0, 0.0]),
        None
    );
    assert_eq!(
        annuity(false, [0.1, 0.0, 100.0, 0.0, 0.0])
            .unwrap()
            .unwrap()
            .to_bits(),
        0.0_f64.to_bits()
    );
}

#[cfg(feature = "internals")]
#[test]
fn payment_boundary_separates_zero_rate_from_unproved_discount_rounding() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_number::payment;
    assert_eq!(payment([0.0, 10.0, 100.0, 50.0, 1.0]), Some(Ok(-15.0)));
    assert_eq!(payment([0.0, -10.0, 100.0, 50.0, 1.0]), Some(Ok(15.0)));
    assert_eq!(
        payment([0.0, 0.0, 0.0, 0.0, 0.0]),
        Some(Err(ExcelError::Num))
    );
    assert_eq!(
        payment([-1.0, 10.0, 100.0, 0.0, 0.0]),
        Some(Err(ExcelError::Num))
    );
    assert_eq!(payment([0.1, 10.0, 100.0, 0.0, 0.0]), None);
}
