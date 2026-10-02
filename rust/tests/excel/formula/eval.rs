//! `rust/src/excel/formula/eval.rs`: one pass preserves held caches and no-op revisions.

use yggdryl::Scalar;
use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};

#[test]
fn held_formulas_keep_cached_results_and_identical_result_is_clean() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, expression, cached) in [
        ("B1", "A1+1", 1.0),
        ("C1", "MYSTERY(1)", 42.0),
        ("D1", "1+2", 3.0),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(cached), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(expression, at)),
            )
            .unwrap();
    }
    let before = workbook.sheet("Data").unwrap().revision();
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (2, 1));
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(sheet.revision(), before);
    for (address, expected) in [("B1", 1.0), ("C1", 42.0), ("D1", 3.0)] {
        assert_eq!(
            sheet.scalar(address.parse().unwrap()),
            Scalar::from(expected)
        );
    }
}

#[test]
fn date_styled_formula_recovers_from_invalid_numeric_cache_across_reopen() {
    use yggdryl::excel::{CellRange, NumberFormat, StylePatch};

    for (system, last, valid) in [
        (DateSystem::Year1900, 2_958_465, 45_292),
        (DateSystem::Year1904, 2_957_003, 43_830),
    ] {
        for (invalid_formula, invalid) in [
            ("-1".to_owned(), -1.0),
            (format!("{last}+1"), f64::from(last + 1)),
        ] {
            let mut workbook = Workbook::new();
            workbook.set_date_system(system);
            let at = CellRef::new(0, 0);
            workbook
                .add_sheet("Data")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::date32(19_723), system)
                        .unwrap()
                        .with_formula(Formula::from_entry(&invalid_formula, at).unwrap()),
                )
                .unwrap();
            workbook
                .set_style(
                    "Data",
                    &[CellRange::new(at, at)],
                    &StylePatch {
                        number_format: Some("m/d/yyyy".into()),
                        ..StylePatch::default()
                    },
                )
                .unwrap();
            assert_eq!(
                workbook.sheet("Data").unwrap().cell(at).unwrap().format(),
                NumberFormat::Date
            );

            assert_eq!(workbook.calculate_all().unwrap().evaluated, 1);
            let cell = workbook.sheet("Data").unwrap().cell(at).unwrap();
            assert_eq!(cell.value(), &Scalar::from(invalid));
            assert_eq!(cell.format(), NumberFormat::General);
            assert_eq!(
                workbook.cell_style("Data", at).unwrap().number_format,
                "m/d/yyyy"
            );
            let shown = workbook.display_text("Data", at).unwrap().unwrap();
            if system == DateSystem::Year1904 && invalid == -1.0 {
                assert_eq!(shown.text, "-1/2/1904");
                assert_eq!(shown.fill, None);
            } else {
                assert_eq!(shown.fill, Some(('#', 0)));
            }

            let mut reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
            let cell = reopened.sheet("Data").unwrap().cell(at).unwrap();
            assert_eq!(cell.value(), &Scalar::from(invalid));
            assert_eq!(cell.format(), NumberFormat::General);
            assert_eq!(
                reopened.cell_style("Data", at).unwrap().number_format,
                "m/d/yyyy"
            );
            let shown = reopened.display_text("Data", at).unwrap().unwrap();
            if system == DateSystem::Year1904 && invalid == -1.0 {
                assert_eq!(shown.text, "-1/2/1904");
                assert_eq!(shown.fill, None);
            } else {
                assert_eq!(shown.fill, Some(('#', 0)));
            }

            // Replacing only the formula leaves the numeric cache and authored
            // date style in place. Its next valid result must be typed by the
            // style, just as reopening that result would type it.
            let prior = reopened.sheet("Data").unwrap().cell(at).unwrap().clone();
            reopened
                .sheet_mut("Data")
                .unwrap()
                .insert_cell(
                    prior.with_formula(Formula::from_entry(&valid.to_string(), at).unwrap()),
                )
                .unwrap();
            assert_eq!(reopened.calculate_all().unwrap().evaluated, 1);
            let cell = reopened.sheet("Data").unwrap().cell(at).unwrap();
            assert_eq!(cell.value(), &Scalar::date32(19_723));
            assert_eq!(cell.format(), NumberFormat::Date);
            let roundtrip = Workbook::from_bytes(reopened.into_bytes().unwrap()).unwrap();
            assert_eq!(
                roundtrip.sheet("Data").unwrap().cell(at).unwrap().value(),
                &Scalar::date32(19_723)
            );
        }
    }
}

#[test]
fn percent_and_division_use_the_same_shared_float_kernel() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula) in [("A1", "25%"), ("B1", "25/100")] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_entry(formula, at).unwrap()),
            )
            .unwrap();
    }
    workbook.calculate_all().unwrap();
    let sheet = workbook.sheet("Data").unwrap();
    let percent = sheet.scalar("A1".parse().unwrap()).as_f64().unwrap();
    let divided = sheet.scalar("B1".parse().unwrap()).as_f64().unwrap();
    assert_eq!(percent.to_bits(), divided.to_bits());
}

#[test]
fn array_dynamic_and_data_table_anchors_keep_caches_and_metadata() {
    use crate::excel_package::{
        content_types, member, package, root_relationships, workbook, workbook_relationships,
        worksheet,
    };
    let types = content_types(1, false, false);
    let root = root_relationships();
    let book = workbook(&["Data"], false);
    let rels = workbook_relationships(1, false, false);
    let part = worksheet(
        "<row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A2\">1+1</f><v>77</v></c>\
         <c r=\"B1\" cm=\"1\"><f>1+1</f><v>88</v></c>\
         <c r=\"C1\"><f t=\"dataTable\" ref=\"C1:C2\" dtr=\"1\" r1=\"A1\">1+1</f><v>99</v></c></row>",
    );
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &part),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    let prior = opened.sheet("Data").unwrap().revision();
    let report = opened.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (0, 3));
    let sheet = opened.sheet("Data").unwrap();
    assert_eq!(sheet.revision(), prior);
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from(77.0));
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from(88.0));
    assert_eq!(sheet.scalar("C1".parse().unwrap()), Scalar::from(99.0));
    assert_eq!(member(&opened, "xl/worksheets/sheet1.xml"), part);
    let reopened = Workbook::from_bytes(opened.into_bytes().unwrap()).unwrap();
    let sheet = reopened.sheet("Data").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from(77.0));
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from(88.0));
    assert_eq!(sheet.scalar("C1".parse().unwrap()), Scalar::from(99.0));
}

#[test]
fn error_propagation_and_unsettled_coercion_keep_their_boundaries() {
    use yggdryl::excel::ExcelError;
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula) in [("A1", "1/0"), ("B1", "#N/A+2"), ("C1", "\"2\"+1")] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(99.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_entry(formula, at).unwrap()),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (3, 0));
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(
        sheet.cell("A1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::Div0)
    );
    assert_eq!(
        sheet.cell("B1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::NA)
    );
    assert_eq!(sheet.scalar("C1".parse().unwrap()), Scalar::from(3.0));
}

#[test]
fn a_later_lazy_source_error_publishes_no_earlier_calculation() {
    use crate::excel_package::{
        content_types, package, root_relationships, workbook, workbook_relationships, worksheet,
    };
    let types = content_types(2, false, false);
    let root = root_relationships();
    let book = workbook(&["Data", "Bad"], false);
    let rels = workbook_relationships(2, false, false);
    let good = worksheet("<row r=\"1\"><c r=\"A1\"><f>2+3</f><v>77</v></c></row>");
    let bad = worksheet("<row r=\"1\"><c r=\"A1\"><v>4</v></c>");
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &good),
        ("xl/worksheets/sheet2.xml", &bad),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    let prior = opened.sheet("Data").unwrap().revision();
    assert!(opened.calculate_all().is_err());
    let sheet = opened.sheet("Data").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from(77.0));
    assert_eq!(sheet.revision(), prior);
}

/// The desktop fixture's ABS(-7.25) is 7.25. Zero and error propagation are
/// typed controls; direct Boolean/text arguments use Entry-owned coercion.
#[test]
fn abs_uses_shared_scalar_magnitude_after_numeric_coercion() {
    use yggdryl::excel::ExcelError;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula, stale) in [
        ("A1", "ABS(-7.25)", 99.0),
        ("B1", "ABS(0)", 99.0),
        ("C1", "ABS(#N/A)", 99.0),
        ("D1", "ABS(TRUE)", 44.0),
        ("E1", "ABS(\"2\")", 55.0),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(stale), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_entry(formula, at).unwrap()),
            )
            .unwrap();
    }
    let omitted: CellRef = "F1".parse().unwrap();
    assert!(Formula::from_entry("ABS(,)", omitted).is_err());
    sheet
        .insert_cell(
            Cell::from_scalar(omitted, Scalar::from(66.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("ABS(,)", omitted)),
        )
        .unwrap();
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (5, 1));
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from(7.25));
    let zero = sheet.scalar("B1".parse().unwrap()).as_f64().unwrap();
    assert_eq!(zero.to_bits(), 0.0_f64.to_bits());
    assert_eq!(
        sheet.cell("C1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::NA)
    );
    assert_eq!(sheet.scalar("D1".parse().unwrap()), Scalar::from(1.0));
    assert_eq!(sheet.scalar("E1".parse().unwrap()), Scalar::from(2.0));
    assert_eq!(sheet.scalar(omitted), Scalar::from(66.0));
}

/// A computed subnormal addend can affect a later normal result. Until raw
/// calculation and the distinct saved-cache spelling are both represented,
/// hold the formula's old cache instead of publishing a wrong normal value.
#[test]
fn subnormal_add_intermediates_hold_cache_before_normal_final() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula, cached) in [
        ("A1", "3E-308-2.5E-308+2.3E-308", 77.0),
        ("B1", "(3E-308-2.5E-308)+2.3E-308", 88.0),
        ("C1", "3E-308-2.5E-308", 99.0),
        ("D1", "3E-308*0.1", 111.0),
        ("E1", "3E-308/10", 222.0),
        ("F1", "3E-308%", 333.0),
        ("G1", "1+2*3", 0.0),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(cached), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_entry(formula, at).unwrap()),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (4, 3));
    let sheet = workbook.sheet("Data").unwrap();
    for (address, cached) in [("A1", 77.0), ("B1", 88.0), ("C1", 99.0)] {
        assert_eq!(
            sheet.scalar(address.parse().unwrap()),
            Scalar::from(cached),
            "{address}"
        );
    }
    for address in ["D1", "E1", "F1"] {
        let value = sheet.scalar(address.parse().unwrap()).as_f64().unwrap();
        assert_eq!(value.to_bits(), 0, "{address}");
    }
    assert_eq!(sheet.scalar("G1".parse().unwrap()), Scalar::from(7.0));
}

/// Direct numeric arguments use the ordered sum owner and the calibrated
/// ROUND primitive. Unsettled origins retain their stale caches.
#[test]
fn numeric_sum_and_round_calls_coerce_direct_values() {
    use yggdryl::excel::ExcelError;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula, stale) in [
        ("A1", "SUM(1,2,3)", 99.0),
        ("B1", "SUM(1+2,3)", 99.0),
        ("C1", "SUM(1/0,2)", 99.0),
        ("D1", "SUM(1,\"2\")", 44.0),
        ("E1", "SUM(1,TRUE)", 55.0),
        ("F1", "SUM(1,,2)", 66.0),
        ("G1", "ROUND(2.15,1)", 99.0),
        ("H1", "ROUND(-1.475,2)", 99.0),
        ("I1", "ROUND(#N/A,1)", 99.0),
        ("J1", "ROUND(\"2\",1)", 77.0),
        ("K1", "ROUND(1/0,1)", 99.0),
        ("L1", "SUM(#N/A,Z1)", 88.0),
        ("M1", "SUM(Z1,#N/A)", 89.0),
        ("Z1", "MYSTERY(1)", 77.0),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(stale), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (10, 4));
    let sheet = workbook.sheet("Data").unwrap();
    for (address, expected) in [
        ("A1", 6.0),
        ("B1", 6.0),
        ("D1", 3.0),
        ("E1", 2.0),
        ("J1", 2.0),
    ] {
        assert_eq!(
            sheet.scalar(address.parse().unwrap()),
            Scalar::from(expected)
        );
    }
    for (address, expected) in [
        ("G1", 0x4001_9999_9999_999a_u64),
        ("H1", 0xbff7_ae14_7ae1_47ae),
    ] {
        assert_eq!(
            sheet
                .scalar(address.parse().unwrap())
                .as_f64()
                .unwrap()
                .to_bits(),
            expected,
            "{address}"
        );
    }
    for (address, expected) in [
        ("C1", ExcelError::Div0),
        ("I1", ExcelError::NA),
        ("K1", ExcelError::Div0),
    ] {
        assert_eq!(
            sheet.cell(address.parse().unwrap()).unwrap().error(),
            Some(expected)
        );
    }
    for (address, stale) in [("F1", 66.0), ("L1", 88.0), ("M1", 89.0), ("Z1", 77.0)] {
        assert_eq!(sheet.scalar(address.parse().unwrap()), Scalar::from(stale));
    }
}

/// Excel 16.0 build 20430.0: explicit argument errors beat provisional
/// aggregate overflow; transient subnormal arithmetic needs a raw/cache split.
#[test]
fn native_sum_edge_cases_preserve_errors_and_uncertain_cache() {
    use yggdryl::excel::ExcelError;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula) in [
        ("A1", "SUM(9E307,9E307)"),
        ("B1", "SUM(9E307,9E307,1/0)"),
        ("C1", "SUM(9E307,9E307,#N/A)"),
        ("D1", "SUM(3E-308,-2.5E-308,2.3E-308)"),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (3, 1));
    let sheet = workbook.sheet("Data").unwrap();
    for (address, expected) in [
        ("A1", ExcelError::Num),
        ("B1", ExcelError::Div0),
        ("C1", ExcelError::NA),
    ] {
        assert_eq!(
            sheet.cell(address.parse().unwrap()).unwrap().error(),
            Some(expected)
        );
    }
    assert_eq!(sheet.scalar("D1".parse().unwrap()), Scalar::from(77.0));
}

/// SQRT(2) is pinned to the independently observed Excel 16.0 result. The
/// overall source run had an unrelated locale TEXT calibration failure.
#[test]
fn sqrt_native_bits_domain_errors_and_direct_coercion() {
    use yggdryl::excel::ExcelError;
    let native: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/sqrt_native.json")).unwrap();
    assert_eq!(native["formula"].as_str(), Some("SQRT(2)"));
    assert_eq!(native["before_bits"], native["after_bits"]);
    assert_eq!(native["cache_type"].as_str(), Some("n"));
    let expected = u64::from_str_radix(native["before_bits"].as_str().unwrap(), 16).unwrap();
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula, stale) in [
        ("A1", "SQRT(2)", 99.0),
        ("B1", "SQRT(0)", 99.0),
        ("C1", "SQRT(-1)", 99.0),
        ("D1", "SQRT(#N/A)", 99.0),
        ("E1", "SQRT(1/0)", 99.0),
        ("F1", "SQRT(\"4\")", 77.0),
        ("G1", "SQRT(MYSTERY(1))", 78.0),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(stale), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (6, 1));
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(
        sheet
            .scalar("A1".parse().unwrap())
            .as_f64()
            .unwrap()
            .to_bits(),
        expected
    );
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from(0.0));
    for (address, error) in [
        ("C1", ExcelError::Num),
        ("D1", ExcelError::NA),
        ("E1", ExcelError::Div0),
    ] {
        assert_eq!(
            sheet.cell(address.parse().unwrap()).unwrap().error(),
            Some(error)
        );
    }
    assert_eq!(sheet.scalar("F1".parse().unwrap()), Scalar::from(2.0));
    assert_eq!(sheet.scalar("G1".parse().unwrap()), Scalar::from(78.0));
}

/// Individual Excel 16.0 build 20430.0 MOD observations have stable
/// before/after Value2; the encompassing run failed an unrelated TEXT probe.
#[test]
fn native_mod_sign_zero_error_and_held_cases() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/math_mod_native.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 6);
    assert_eq!(
        fixture["manifest_sha256"],
        "3a1b341d1e66691c04e2f33d6803aada608cbd7a648c94de03b411c63eabafc5"
    );
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula) in [
        ("A1", "MOD(-7,-3)"),
        ("B1", "MOD(-7,3)"),
        ("C1", "MOD(7,-3)"),
        ("D1", "MOD(7,3)"),
        ("E1", "MOD(7,0)"),
        ("F1", "MOD(#N/A,3)"),
        ("G1", "MOD(\"7\",3)"),
        ("H1", "MOD(MYSTERY(1),3)"),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (7, 1));
    let sheet = workbook.sheet("Data").unwrap();
    for (address, expected) in [("A1", -1.0), ("B1", 2.0), ("C1", -2.0), ("D1", 1.0)] {
        assert_eq!(
            sheet.scalar(address.parse().unwrap()),
            Scalar::from(expected)
        );
    }
    for (address, expected) in [("E1", ExcelError::Div0), ("F1", ExcelError::NA)] {
        assert_eq!(
            sheet.cell(address.parse().unwrap()).unwrap().error(),
            Some(expected)
        );
    }
    assert_eq!(sheet.scalar("G1".parse().unwrap()), Scalar::from(1.0));
    assert_eq!(sheet.scalar("H1".parse().unwrap()), Scalar::from(77.0));
}

/// Three owned Excel16 runs establish a bounded quotient region for MOD.
/// The mixed region above 2^40 retains the stale cache until its exact
/// refusal boundary is known; results outside it use observed native cases.
#[test]
fn mod_uses_native_quotient_and_subnormal_boundaries() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/math_mod_boundary_native.json")).unwrap();
    assert_eq!(fixture["excel_version"], "16.0");
    assert_eq!(fixture["excel_build"], "20430.0");
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 252);
    for (formula, expected) in [
        ("MOD(0.3,0.1)", "3fb9999999999998"),
        ("MOD(9E307,3)", "#NUM!"),
        ("MOD(3E-308,2.5E-308)", "#NUM!"),
        ("MOD(2.5E-308,3E-308)", "0011fa182c40c60d"),
        ("MOD(9E307,9E307)", "0000000000000000"),
        ("MOD(1E12,1)", "0000000000000000"),
    ] {
        let case = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["formula"] == formula)
            .unwrap();
        if expected == "#NUM!" {
            assert_eq!(case["cache_type"], "e");
            assert_eq!(case["cache_text"], expected);
        } else {
            assert_eq!(case["cache_type"], "n");
            assert_eq!(case["value2"]["ieee754_hex"], expected);
        }
    }
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (address, formula) in [
        ("A1", "MOD(0.3,0.1)"),
        ("B1", "MOD(9E307,3)"),
        ("C1", "MOD(3E-308,2.5E-308)"),
        ("D1", "MOD(2.5E-308,3E-308)"),
        ("E1", "MOD(9E307,9E307)"),
        ("F1", "MOD(1E12,1)"),
        ("G1", "MOD(1.5E12,1)"),
    ] {
        let at: CellRef = address.parse().unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (6, 1));
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(
        sheet
            .scalar("A1".parse().unwrap())
            .as_f64()
            .unwrap()
            .to_bits(),
        0x3fb9_9999_9999_9998
    );
    assert_eq!(
        sheet.cell("B1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::Num)
    );
    assert_eq!(
        sheet.cell("C1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::Num)
    );
    assert_eq!(
        sheet
            .scalar("D1".parse().unwrap())
            .as_f64()
            .unwrap()
            .to_bits(),
        0x0011_fa18_2c40_c60d
    );
    for address in ["E1", "F1"] {
        assert_eq!(
            sheet
                .scalar(address.parse().unwrap())
                .as_f64()
                .unwrap()
                .to_bits(),
            0
        );
    }
    assert_eq!(sheet.scalar("G1".parse().unwrap()), Scalar::from(77.0));
}

#[cfg(feature = "internals")]
mod context {
    use super::*;
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;

    fn formula(text: &str) -> Formula {
        Formula::from_entry(text, CellRef::new(0, 1)).unwrap()
    }

    #[test]
    fn evaluator_context_preserves_range_origin_and_scalarizes_only_where_required() {
        let mut evaluator = ContextEvaluator::default();
        for (text, value, calls) in [
            (r#"SUM(A1:A5,TRUE,"3")"#, 11.0, [1, 0, 5]),
            ("SUM((A1:A5))", 7.0, [1, 0, 5]),
            ("A1", 2.0, [1, 1, 0]),
            ("SUM(A1:A5+1)", 3.0, [1, 1, 0]),
            ("ROUND(A1,0)", 2.0, [1, 1, 0]),
        ] {
            assert_eq!(
                evaluator.evaluate(&formula(text), false, false).unwrap(),
                Some(Scalar::from(value)),
                "{text}"
            );
            assert_eq!(evaluator.calls(), calls, "{text}");
            assert!(evaluator.is_clear(), "{text}");
        }
    }

    #[test]
    fn evaluator_context_holds_a_blocked_range_member_and_cleans_after_late_refusal() {
        let mut evaluator = ContextEvaluator::default();
        let expression = formula("SUM(A1:A5)");
        assert_eq!(evaluator.evaluate(&expression, true, false).unwrap(), None);
        assert_eq!(evaluator.calls(), [1, 0, 5]);
        assert!(evaluator.is_clear());
        let error = evaluator.evaluate(&expression, false, true).unwrap_err();
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { ref path, .. } if path == "context-fixture!A3")
        );
        assert_eq!(evaluator.calls(), [1, 0, 3]);
        assert!(evaluator.is_clear());
        assert_eq!(
            evaluator.evaluate(&expression, false, false).unwrap(),
            Some(Scalar::from(7.0))
        );
        assert!(evaluator.is_clear());
    }
}
