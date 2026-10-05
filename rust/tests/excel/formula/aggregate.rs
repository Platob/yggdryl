//! `rust/src/excel/formula/aggregate.rs`: ordered numeric accumulation.

#[test]
fn aggregate_intersections_keep_reference_origin_distinct_from_scalar_origin() {
    use yggdryl::excel::{CellRef, ExcelError, Workbook};
    let mut book = Workbook::new();
    book.add_sheet("Data")
        .unwrap()
        .set_cell("A1".parse().unwrap(), true)
        .unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .set_cell("A2".parse().unwrap(), "2")
        .unwrap();
    for (row, formula) in ["SUM(@A1)", "SUM(@TRUE)", "SUM(@A2)", "AND(@A2)"]
        .into_iter()
        .enumerate()
    {
        book.set_entry("Data", CellRef::new(row as u32, 1), &format!("={formula}"))
            .unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 4);
    for (row, expected) in [0.0, 1.0, 0.0].into_iter().enumerate() {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row as u32, 1))
                .as_f64(),
            Some(expected)
        );
    }
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .cell(CellRef::new(3, 1))
            .unwrap()
            .error(),
        Some(ExcelError::Value)
    );
}

#[test]
fn basic_aggregates_recalculate_sparse_ranges_and_preserve_held_dependencies() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (row, value) in [
        (0, Scalar::from(2.0)),
        (1, Scalar::from(-3.0)),
        (2, Scalar::from(true)),
        (3, Scalar::from("text")),
    ] {
        sheet.set_cell(CellRef::new(row, 0), value).unwrap();
    }
    for (row, function) in ["COUNT", "COUNTA", "MIN", "MAX"].into_iter().enumerate() {
        let at = CellRef::new(row as u32, 1);
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(&format!("{function}(A:A)"), at)),
            )
            .unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 4);
    for (row, expected) in [2.0, 4.0, -3.0, 2.0].into_iter().enumerate() {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row as u32, 1)),
            Scalar::from(expected)
        );
    }
    let at = CellRef::new(1_048_575, 0);
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at, Scalar::from("old"), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("MYSTERY(1)", at)),
        )
        .unwrap();
    let held = book.recalculate().unwrap();
    assert_eq!((held.evaluated, held.uncomputed), (0, 5));
    for (row, expected) in [2.0, 4.0, -3.0, 2.0].into_iter().enumerate() {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row as u32, 1)),
            Scalar::from(expected)
        );
    }
    book.sheet_mut("Data").unwrap().set_cell(at, 9.0).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    for (row, expected) in [3.0, 5.0, -3.0, 9.0].into_iter().enumerate() {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row as u32, 1)),
            Scalar::from(expected)
        );
    }
    assert_eq!(book.sheet("Data").unwrap().cell_count(), 9);
}

#[cfg(feature = "internals")]
mod internal {
    use serde_json::Value;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;

    fn number(bits: &str) -> f64 {
        f64::from_bits(u64::from_str_radix(bits, 16).unwrap())
    }

    #[test]
    fn native_ordered_three_input_sum_matches_all_54_saved_results() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/sum_order_native.json")).unwrap();
        assert_eq!(fixture["native_cases"].as_u64(), Some(288));
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 54);
        for case in cases {
            let mut accumulator = Accumulator::default();
            let inputs = case["source_bits"].as_array().unwrap();
            assert_eq!(inputs.len(), 3);
            for input in inputs {
                accumulator
                    .push_number(number(input.as_str().unwrap()))
                    .unwrap();
            }
            let actual = accumulator.finish_sum().unwrap().unwrap().to_bits();
            let expected = number(case["expected_bits"].as_str().unwrap()).to_bits();
            assert_eq!(actual, expected, "{}", case["formula"].as_str().unwrap());
        }
    }

    #[test]
    fn empty_singleton_and_order_are_explicit() {
        assert_eq!(
            Accumulator::default()
                .finish_sum()
                .unwrap()
                .unwrap()
                .to_bits(),
            0
        );
        let mut one = Accumulator::default();
        one.push_number(7.25).unwrap();
        assert_eq!(
            one.finish_sum().unwrap().unwrap().to_bits(),
            7.25_f64.to_bits()
        );

        let n = -f64::from_bits(0x3fef_ffff_ffff_fffe);
        let tiny = f64::EPSILON;
        let mut natural = Accumulator::default();
        for value in [1.0, n, tiny] {
            natural.push_number(value).unwrap();
        }
        let mut permuted = Accumulator::default();
        for value in [1.0, tiny, n] {
            permuted.push_number(value).unwrap();
        }
        assert_eq!(
            natural.finish_sum().unwrap().unwrap().to_bits(),
            (2.0 * tiny).to_bits()
        );
        assert_eq!(permuted.finish_sum().unwrap().unwrap().to_bits(), 0);
    }

    #[test]
    fn a_refused_number_does_not_publish_a_partial_push() {
        use yggdryl::excel::ExcelError;

        let mut accumulator = Accumulator::default();
        accumulator.push_number(2.0).unwrap();
        assert_eq!(accumulator.push_number(f64::INFINITY), Err(ExcelError::Num));
        accumulator.push_number(3.0).unwrap();
        assert_eq!(
            accumulator.finish_sum().unwrap().unwrap().to_bits(),
            5.0_f64.to_bits()
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn native_subnormal_transport_requires_an_uncomputed_sum_boundary() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/sum_edges_native.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 40);
    assert_eq!(
        fixture["input_sha256"],
        "e8cde983212d3129fc47d6464dc4fecf4bf02cfc7ab80e4b8551a281178b5b83"
    );
    let native = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["formula"] == "SUM(C2,D2,E2)")
        .unwrap();
    assert_eq!(native["answer"], "0008000000000000");
    assert_eq!(native["cache"], "0");
    let mut sum = Accumulator::default();
    for bits in [
        0x0010_0000_0000_0000,
        0x8018_0000_0000_0000,
        0x0010_0000_0000_0000,
    ] {
        sum.push_number(f64::from_bits(bits)).unwrap();
    }
    assert_eq!(sum.finish_sum(), Ok(None));

    let mut ordinary = Accumulator::default();
    for value in [1.0, -(1.0 - f64::EPSILON), f64::EPSILON, 0.0] {
        ordinary.push_number(value).unwrap();
    }
    assert_eq!(ordinary.finish_sum(), Ok(Some(2.0 * f64::EPSILON)));

    let mut overflow = Accumulator::default();
    overflow.push_number(9e307).unwrap();
    overflow.push_number(9e307).unwrap();
    assert_eq!(overflow.finish_sum(), Err(ExcelError::Num));
}

#[test]
fn logical_reducers_match_native_values_and_keep_locale_text_explicitly_held() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/logical_reducers_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 380);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 190);
    let mut covered = (0, 0);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        for name in ["Values", "CycleShape", "Cases"] {
            book.add_sheet(name).unwrap();
        }
        for (name, cells) in fixture["source_cells"].as_object().unwrap() {
            for (address, value) in cells.as_object().unwrap() {
                let scalar = if let Some(value) = value.as_bool() {
                    Scalar::from(value)
                } else if let Some(value) = value.as_str() {
                    Scalar::from(value)
                } else {
                    Scalar::from(value.as_f64().unwrap())
                };
                book.sheet_mut(name)
                    .unwrap()
                    .set_cell(address.parse().unwrap(), scalar)
                    .unwrap();
            }
        }
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut(case["sheet"].as_str().unwrap())
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
            (83, 12, 0)
        );
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let cell = book
                .sheet(case["sheet"].as_str().unwrap())
                .unwrap()
                .cell(case["cell"].as_str().unwrap().parse().unwrap())
                .unwrap();
            if case["rust_policy"] == "held_locale_text" {
                covered.1 += 1;
                assert_eq!(cell.value(), &Scalar::from(-777.0), "{}", case["id"]);
                continue;
            }
            covered.0 += 1;
            let expected = &case["saved_cache"];
            match expected["type"].as_str().unwrap() {
                "b" => assert_eq!(
                    cell.value().as_bool(),
                    Some(expected["value_text"] == "1"),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    expected["value_text"].as_str(),
                    "{}",
                    case["id"]
                ),
                "str" => assert_eq!(
                    cell.value().as_str(),
                    Some(expected["value_text"].as_str().unwrap_or("")),
                    "{}",
                    case["id"]
                ),
                kind => panic!("unexpected cache type {kind}"),
            }
        }
        let before =
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision());
        assert_eq!(book.calculate_all().unwrap().evaluated, 83);
        assert_eq!(
            before,
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision())
        );
        let idle = book.recalculate().unwrap();
        assert_eq!((idle.evaluated, idle.uncomputed), (0, 12));
    }
    assert_eq!(covered, (166, 24));
}

#[test]
fn logical_reducers_retain_held_dependencies_before_filtering_old_caches() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (row, cache) in [(0, Scalar::from(false)), (1, Scalar::from("old text"))] {
        let at = CellRef::new(row, 0);
        sheet
            .insert_cell(
                Cell::from_scalar(at, cache, DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file("MYSTERY(1)", at)),
            )
            .unwrap();
    }
    for (row, formula) in [
        (0, "OR(TRUE,A1:A2)"),
        (1, "AND(FALSE,A1:A2)"),
        (2, "XOR(TRUE,A1:A2)"),
    ] {
        let at = CellRef::new(row, 1);
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let first = book.calculate_all().unwrap();
    assert_eq!((first.evaluated, first.uncomputed), (0, 5));
    for row in 0..3 {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(CellRef::new(row, 1)),
            Scalar::from(77.0)
        );
    }
    let sheet = book.sheet_mut("Data").unwrap();
    sheet.set_cell(CellRef::new(0, 0), false).unwrap();
    sheet.set_cell(CellRef::new(1, 0), "text").unwrap();
    let next = book.recalculate().unwrap();
    assert_eq!((next.evaluated, next.uncomputed), (3, 0));
    for (row, value) in [(0, true), (1, false), (2, true)] {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row, 1))
                .as_bool(),
            Some(value)
        );
    }
}

#[test]
fn logical_reducers_visit_whole_columns_sparsely_and_follow_new_occupancy() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, ExcelError, Formula, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (row, function) in ["AND", "OR", "XOR"].into_iter().enumerate() {
        let at = CellRef::new(row as u32, 1);
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(&format!("{function}(A:A)"), at)),
            )
            .unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 3);
    for row in 0..3 {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .cell(CellRef::new(row, 1))
                .unwrap()
                .error(),
            Some(ExcelError::Value)
        );
    }
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(CellRef::new(1_048_575, 0), true)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    for row in 0..3 {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .scalar(CellRef::new(row, 1))
                .as_bool(),
            Some(true)
        );
    }
    assert_eq!(book.sheet("Data").unwrap().cell_count(), 4);
}

#[test]
fn basic_aggregates_match_all_native_scalar_and_reference_controls() {
    super::native_function_fixture(
        include_str!("../fixtures/basic_aggregates_native.json"),
        296,
    );
}

#[test]
fn basic_aggregates_match_all_native_explicit_intersection_controls() {
    super::native_function_fixture(
        include_str!("../fixtures/aggregate_intersection_native.json"),
        84,
    );
}

#[test]
fn averages_extrema_and_product_match_all_408_native_cases() {
    super::native_function_fixture(include_str!("../fixtures/average_product_native.json"), 408);
}

#[cfg(feature = "internals")]
#[test]
fn product_prefix_refusal_preserves_the_last_finite_state() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    let mut product = Accumulator::default();
    product.push_product(9e307).unwrap();
    assert_eq!(product.push_product(2.0), Err(ExcelError::Num));
    product.push_product(1e-307).unwrap();
    assert_eq!(
        product.finish_product().to_bits(),
        (9e307_f64 * 1e-307_f64).to_bits()
    );
    let mut zero = Accumulator::default();
    zero.push_product(0.0).unwrap();
    zero.push_product(9e307).unwrap();
    zero.push_product(9e307).unwrap();
    assert_eq!(zero.finish_product(), 0.0);
}

#[cfg(feature = "internals")]
#[test]
fn ranked_accumulator_order_statistics() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    fn ranks(values: &[f64]) -> Accumulator {
        let mut result = Accumulator::ranked();
        for &value in values {
            result.push_number(value).unwrap();
        }
        result
    }
    let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    assert_eq!(ranks(&values).finish_kth(1.9, true), Ok(Some(7.0)));
    assert_eq!(ranks(&values).finish_kth(2.9, true), Ok(Some(6.0)));
    assert_eq!(ranks(&values).finish_kth(1.9, false), Ok(Some(1.0)));
    assert_eq!(ranks(&values).finish_kth(2.9, false), Ok(Some(2.0)));
    assert_eq!(ranks(&values).finish_kth(0.9, true), Err(ExcelError::Num));
    assert_eq!(ranks(&values).finish_kth(8.1, false), Err(ExcelError::Num));
    assert_eq!(
        ranks(&[0.1, 0.2, 0.3]).finish_percentile(0.3),
        Ok(Some(0.16000000000000003))
    );
    assert_eq!(
        ranks(&[0.1, 0.2, 0.3]).finish_percentile(0.9),
        Ok(Some(0.27999999999999997))
    );
    assert_eq!(
        ranks(&[1.0, 2.0, 3.0]).finish_percentile(-0.1),
        Err(ExcelError::Num)
    );
    assert_eq!(ranks(&[]).finish_percentile(0.5), Err(ExcelError::Num));
    assert_eq!(
        ranks(&[2.0, 3.0, 3.0, 4.0]).finish_rank(3.0, false),
        Ok(Some(2.0))
    );
    assert_eq!(
        ranks(&[2.0, 3.0, 3.0, 4.0]).finish_rank(3.0, true),
        Ok(Some(2.0))
    );
    assert_eq!(
        ranks(&[2.0, 3.0, 4.0]).finish_rank(5.0, false),
        Err(ExcelError::NA)
    );
    assert_eq!(ranks(&[f64::from_bits(1)]).finish_kth(1.0, true), Ok(None));
    assert_eq!(ranks(&[f64::from_bits(1)]).finish_percentile(0.5), Ok(None));
    assert_eq!(
        ranks(&[f64::from_bits(1)]).finish_rank(0.0, false),
        Ok(None)
    );
}

#[cfg(feature = "internals")]
#[test]
fn sparse_count_uses_one_checked_multiplicity() {
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    let mut count = Accumulator::default();
    count.push_count_n(1_048_576).unwrap();
    count.push_present().unwrap();
    assert_eq!(count.finish_counta(), Some(1_048_577.0));
    let mut out_of_cache_range = Accumulator::default();
    out_of_cache_range.push_count_n((1_u64 << 53) + 1).unwrap();
    assert_eq!(out_of_cache_range.finish_counta(), None);
}

#[cfg(feature = "internals")]
#[test]
fn compact_zero_runs_match_ordered_additions() {
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    for prefix in [
        &[][..],
        &[1.0][..],
        &[-0.0][..],
        &[1.0e16, 1.0, -1.0e16][..],
    ] {
        for zeros in [0, 1, 2, 3, 4_096] {
            let mut literal = Accumulator::default();
            let mut compact = Accumulator::default();
            for &value in prefix {
                literal.push_number(value).unwrap();
                compact.push_number(value).unwrap();
            }
            for _ in 0..zeros {
                literal.push_number(0.0).unwrap();
            }
            compact.push_zero_n(zeros).unwrap();
            assert_eq!(
                literal.finish_sum().unwrap().map(f64::to_bits),
                compact.finish_sum().unwrap().map(f64::to_bits),
                "prefix={prefix:?} zeros={zeros}"
            );
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn ranked_accumulator_median_mode_and_refusal() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    let mut median = Accumulator::ranked();
    let mut mode = Accumulator::ranked();
    for value in [1.0, 2.0, 2.0, 3.0, 3.0] {
        median.push_number(value).unwrap();
        mode.push_number(value).unwrap();
    }
    assert_eq!(
        median.finish_median().unwrap().unwrap().to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(
        mode.finish_mode().unwrap().unwrap().to_bits(),
        2.0_f64.to_bits()
    );
    let mut even = Accumulator::ranked();
    for value in [1.0, 2.0, 3.0, 4.0] {
        even.push_number(value).unwrap();
    }
    assert_eq!(
        even.finish_median().unwrap().unwrap().to_bits(),
        2.5_f64.to_bits()
    );
    let mut distinct = Accumulator::ranked();
    for value in [1.0, 2.0, 3.0] {
        distinct.push_number(value).unwrap();
    }
    assert_eq!(distinct.finish_mode(), Err(ExcelError::NA));
    let mut reverse_tie = Accumulator::ranked();
    for value in [3.0, 3.0, 2.0, 2.0] {
        reverse_tie.push_number(value).unwrap();
    }
    assert_eq!(reverse_tie.finish_mode(), Ok(Some(3.0)));
    assert_eq!(Accumulator::ranked().finish_median(), Err(ExcelError::Num));
    assert_eq!(Accumulator::ranked().finish_mode(), Err(ExcelError::NA));
    let mut tiny_midpoint = Accumulator::ranked();
    for value in [3.0e-308, 6.0e-308] {
        tiny_midpoint.push_number(value).unwrap();
    }
    assert_eq!(tiny_midpoint.finish_median(), Ok(Some(3.0e-308)));
    let mut overflowing_difference = Accumulator::ranked();
    for value in [-9.0e307, 9.0e307] {
        overflowing_difference.push_number(value).unwrap();
    }
    assert_eq!(overflowing_difference.finish_median(), Err(ExcelError::Num));
    let mut unsettled = Accumulator::ranked();
    unsettled.push_number(f64::from_bits(1)).unwrap();
    assert_eq!(unsettled.finish_median(), Ok(None));
}

#[cfg(feature = "internals")]
#[test]
fn discounted_fold_keeps_native_order_and_bounds_unproven_denominators() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::discounted;
    assert_eq!(discounted(1.0, &[1.0, 0.0, 3.0]), Ok(Some(0.875)));
    assert_eq!(discounted(0.0, &[1e16, 1.0, -1e16]), Ok(Some(0.0)));
    assert_eq!(discounted(-1.0, &[]), Err(ExcelError::Div0));
    assert_eq!(discounted(-1.0, &[0.0]), Err(ExcelError::Div0));
    assert_eq!(discounted(1e200, &[1.0, 2.0, 3.0]), Ok(None));
    assert_eq!(discounted(0.0, &[f64::MIN_POSITIVE / 2.0]), Ok(None));
    assert_eq!(discounted(0.0, &[f64::MAX, f64::MAX]), Err(ExcelError::Num));
}

#[cfg(feature = "internals")]
#[test]
fn exact_integral_variance_rejects_lossy_moments_before_binary_rounding() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    let finish = |values: &[f64], sample| {
        let mut accumulator = Accumulator::variance();
        for &value in values {
            accumulator.push_number(value).unwrap();
        }
        accumulator.finish_variance(sample)
    };
    assert_eq!(finish(&[], false), Err(ExcelError::Div0));
    assert_eq!(finish(&[2.0], true), Err(ExcelError::Div0));
    assert_eq!(finish(&[2.0], false), Ok(Some(0.0)));
    assert_eq!(finish(&[7.0, 7.0, 7.0], true), Ok(Some(0.0)));
    assert_eq!(finish(&[1.0, 2.0, 3.0], true), Ok(Some(1.0)));
    assert_eq!(finish(&[-2.0, 0.0, 2.0], false), Ok(Some(8.0 / 3.0)));
    assert_eq!(finish(&[499999.0, 500000.0, 500001.0], true), Ok(Some(1.0)));
    assert_eq!(finish(&[1.0, 2.0], true), Ok(Some(0.5)));
    assert_eq!(finish(&[1.0, 2.0, 2.0, 3.0, 3.0], true), Ok(None));
    assert_eq!(finish(&[0.1, 0.2, 0.3], true), Ok(None));
    assert_eq!(finish(&[-47453133.0, 47453133.0], true), Ok(None));
    assert_eq!(finish(&[9007199254740992.0, 1.0], true), Ok(None));
}

#[cfg(feature = "internals")]
#[test]
fn exact_integral_variance_failed_push_keeps_previous_moments() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;
    for invalid in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut accumulator = Accumulator::variance();
        accumulator.push_number(1.0).unwrap();
        accumulator.push_number(3.0).unwrap();
        assert_eq!(accumulator.push_number(invalid), Err(ExcelError::Num));
        assert_eq!(accumulator.finish_variance(true), Ok(Some(2.0)));
    }
}
