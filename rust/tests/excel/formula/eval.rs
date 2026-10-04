//! `rust/src/excel/formula/eval.rs`: one pass preserves held caches and no-op revisions.

use yggdryl::Scalar;
use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};

#[test]
fn concatenation_operator_uses_shared_text_conversion_and_dependencies() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", "B2".parse().unwrap(), "5").unwrap();
    for (at, entry) in [
        ("C1", "=CLEAN(\"a\"&CHAR(9)&\"b\"&CHAR(10)&\"c\")"),
        ("C2", "=A1&\"x\""),
        ("C3", "=INDIRECT(\"Data!\"&\"B2\")"),
    ] {
        book.set_entry("Data", at.parse().unwrap(), entry).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (3, 0));
    let sheet = book.sheet("Data").unwrap();
    // These three spellings are recorded in the full native Excel oracle.
    assert_eq!(sheet.scalar(CellRef::new(0, 2)).as_str(), Some("abc"));
    assert_eq!(sheet.scalar(CellRef::new(1, 2)).as_str(), Some("x"));
    assert_eq!(sheet.scalar(CellRef::new(2, 2)).as_f64(), Some(5.0));
    book.set_entry("Data", "A1".parse().unwrap(), "q").unwrap();
    book.set_entry("Data", "B2".parse().unwrap(), "7").unwrap();
    let changed = book.recalculate().unwrap();
    assert_eq!((changed.evaluated, changed.uncomputed), (2, 0));
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(CellRef::new(1, 2))
            .as_str(),
        Some("qx")
    );
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(CellRef::new(2, 2))
            .as_f64(),
        Some(7.0)
    );
}

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

/// A source binary64 subnormal crosses the cached-number boundary without
/// a proven native MIN/MAX serialization rule. Keep the old formula cache.
#[test]
fn min_max_hold_subnormal_sources_while_count_still_counts_them() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet
        .set_cell(CellRef::new(0, 0), f64::from_bits(1))
        .unwrap();
    sheet
        .set_cell(CellRef::new(1, 0), -f64::from_bits(1))
        .unwrap();
    for (row, expression) in [
        (0, "MIN(A1:A2)"),
        (1, "MAX(A1:A2)"),
        (2, "COUNT(A1:A2)"),
        (3, "COUNTA(A1:A2)"),
    ] {
        let at = CellRef::new(row, 1);
        sheet
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(expression, at)),
            )
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (2, 2));
    for row in [0, 1] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(CellRef::new(row, 1)),
            Scalar::from(-777.0)
        );
    }
    for row in [2, 3] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(CellRef::new(row, 1)),
            Scalar::from(2.0)
        );
    }
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(CellRef::new(0, 0), 3.0)
        .unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(CellRef::new(1, 0), 4.0)
        .unwrap();
    let report = book.recalculate().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (4, 0));
    assert_eq!(
        book.sheet("Data").unwrap().scalar(CellRef::new(0, 1)),
        Scalar::from(3.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(CellRef::new(1, 1)),
        Scalar::from(4.0)
    );
}

#[test]
fn ordered_comparisons_cover_all_native_cases_and_hold_unmodeled_collation() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/comparisons_native.json")).unwrap();
    assert_eq!(fixture["native_observations"], 842);
    assert_eq!(fixture["native_cache_comparisons_equal"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut coverage = (0, 0);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        for sheet in ["Values", "CycleShape", "Cases"] {
            book.add_sheet(sheet).unwrap();
        }
        for (sheet, cells) in fixture["source_cells"].as_object().unwrap() {
            for (address, value) in cells.as_object().unwrap() {
                let value = if let Some(value) = value.as_bool() {
                    Scalar::from(value)
                } else if let Some(value) = value.as_str() {
                    Scalar::from(value)
                } else {
                    Scalar::from(value.as_f64().unwrap())
                };
                book.sheet_mut(sheet)
                    .unwrap()
                    .set_cell(address.parse().unwrap(), value)
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
            (349, 72, 0)
        );
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book
                .sheet(case["sheet"].as_str().unwrap())
                .unwrap()
                .cell(at)
                .unwrap();
            if case["rust_policy"] == "held_unmodeled_collation" {
                coverage.1 += 1;
                assert_eq!(cell.value(), &Scalar::from(-777.0), "{}", case["id"]);
                continue;
            }
            coverage.0 += 1;
            let cache = &case["saved_cache"];
            match cache["type"].as_str().unwrap() {
                "b" => assert_eq!(
                    cell.value().as_bool(),
                    Some(cache["value_text"] == "1"),
                    "{}",
                    case["id"]
                ),
                "str" => assert_eq!(
                    cell.value().as_str(),
                    Some(cache["value_text"].as_str().unwrap_or("")),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    cache["value_text"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("unexpected native cache type {other}"),
            }
        }
        let before =
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision());
        assert_eq!(book.calculate_all().unwrap().evaluated, 349);
        assert_eq!(
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision()),
            before
        );
        assert_eq!(book.recalculate().unwrap().evaluated, 0);
    }
    assert_eq!(coverage, (698, 144));
}

#[test]
fn basic_math_native_scalar_calls_and_subnormal_hold() {
    // 154-case Excel 16.0 observation in math_native.json: 152 cache-equal;
    // TRUNC of a computed subnormal is raw/subnormal but saved as zero.
    for (formula, bits) in [
        ("SIGN(-0.001)", 0xbff0_0000_0000_0000_u64),
        ("INT(-3.2)", 0xc010_0000_0000_0000),
        ("TRUNC(-3.14159,3)", 0xc009_20c4_9ba5_e354),
        ("PI()", 0x4009_21fb_5444_2d18),
        ("SIGN(TRUE)", 0x3ff0_0000_0000_0000),
        ("INT(Inputs!A2)", 0xc010_0000_0000_0000),
        ("TRUNC(12.987,1.9)", 0x4029_cccc_cccc_cccd),
    ] {
        let mut book = Workbook::new();
        let input = book.add_sheet("Inputs").unwrap();
        input
            .insert_cell(
                Cell::from_scalar(
                    "A2".parse().unwrap(),
                    Scalar::from(-3.2),
                    DateSystem::Year1900,
                )
                .unwrap(),
            )
            .unwrap();
        let cases = book.add_sheet("Cases").unwrap();
        let at: CellRef = "B2".parse().unwrap();
        cases
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(99.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 1, "{formula}");
        assert_eq!(
            book.sheet("Cases")
                .unwrap()
                .scalar(at)
                .as_f64()
                .unwrap()
                .to_bits(),
            bits,
            "{formula}"
        );
    }

    let mut book = Workbook::new();
    let inputs = book.add_sheet("Inputs").unwrap();
    let at: CellRef = "A11".parse().unwrap();
    inputs
        .insert_cell(
            Cell::from_scalar(at, Scalar::from(0.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("3E-308-2.5E-308", at)),
        )
        .unwrap();
    let cases = book.add_sheet("Cases").unwrap();
    let result: CellRef = "B2".parse().unwrap();
    cases
        .insert_cell(
            Cell::from_scalar(result, Scalar::from(77.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("TRUNC(Inputs!A11)", result)),
        )
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert!(report.uncomputed >= 1);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(result),
        Scalar::from(77.0)
    );
}

#[test]
fn basic_math_all_154_native_vectors_replay_with_explicit_raw_subnormal_holds() {
    let native: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/basic_math_native.json")).unwrap();
    assert_eq!(
        native["source_sha256"],
        "ecc53230164fbecb0f4b74a78b3a85ee4b71898a15a7cad4a7e123a5522f8e12"
    );
    assert_eq!(native["native_observations"], 154);
    assert_eq!(native["native_cache_equal"], 152);
    assert_eq!(native["native_cache_different"], 2);
    assert_eq!(native["rust_exact_cases"], 148);
    assert_eq!(native["rust_held_raw_subnormal_dependency"], 6);
    assert_eq!(native["cleanup_completed"], true);
    assert_eq!(native["source_whole_run_passed"], false);
    let cases = native["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 154);

    let mut checked = (0, 0, 0);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        for source in native["source_cells"].as_array().unwrap() {
            let address: CellRef = source["cell"].as_str().unwrap().parse().unwrap();
            let kind = source["kind"].as_str().unwrap();
            if kind == "absent_blank" {
                continue;
            }
            let value = &source["value"];
            if kind.starts_with("error_") || kind == "computed_subnormal" {
                let text = value.as_str().unwrap();
                let formula = text.strip_prefix('=').unwrap();
                book.sheet_mut("Inputs")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(address, Scalar::from(0.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula, address)),
                    )
                    .unwrap();
            } else {
                let value = if let Some(value) = value.as_bool() {
                    Scalar::from(value)
                } else if let Some(value) = value.as_str() {
                    Scalar::from(value)
                } else {
                    Scalar::from(value.as_f64().unwrap())
                };
                book.sheet_mut("Inputs")
                    .unwrap()
                    .set_cell(address, value)
                    .unwrap();
            }
        }
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let address: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        book.calculate_all().unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let id = case["id"].as_str().unwrap();
            let address: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(address).unwrap();
            let cache = &case["saved_cache"];
            assert_eq!(cache["formula_text"], case["formula"], "{id}");
            assert_eq!(case["raw_value2"], case["after_save_value2"], "{id}");
            match case["rust_policy"].as_str().unwrap() {
                "held_raw_subnormal_dependency" => {
                    checked.1 += 1;
                    assert_eq!(cell.value(), &Scalar::from(-777.0), "{id}");
                    if case["native_cache_comparison"] == "different" {
                        checked.2 += 1;
                        assert_eq!(case["function"], "TRUNC", "{id}");
                        assert_eq!(
                            case["raw_value2"]["ieee754_hex"], "0003986b3c0cf46a",
                            "{id}"
                        );
                        assert_eq!(cache["type"], "n", "{id}");
                        assert_eq!(cache["value_text"], "0", "{id}");
                    }
                }
                "exact_saved_cache" => {
                    checked.0 += 1;
                    assert_eq!(case["native_cache_comparison"], "equal", "{id}");
                    match cache["type"].as_str().unwrap() {
                        "n" => {
                            let expected = u64::from_str_radix(
                                case["raw_value2"]["ieee754_hex"].as_str().unwrap(),
                                16,
                            )
                            .unwrap();
                            let saved = cache["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap();
                            assert_eq!(saved.to_bits(), expected, "{id}: source cache transport");
                            assert_eq!(cell.value().as_f64().unwrap().to_bits(), expected, "{id}");
                        }
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            cache["value_text"].as_str(),
                            "{id}"
                        ),
                        other => panic!("{id}: unexpected native cache type {other}"),
                    }
                }
                other => panic!("{id}: unknown replay policy {other}"),
            }
        }
    }
    assert_eq!(checked, (148, 6, 2));
}

#[test]
fn basic_math_all_190_native_near_integer_vectors_replay_exact_bits() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/basic_math_edge_native.json")).unwrap();
    assert_eq!(
        fixture["source_sha256"],
        "5262ff532e1cf48845fdc86b6f5e89eb12ec02223bb64b7ed38a2170d1788b1d"
    );
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_observations"], 190);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 190);
    let mut checked = 0;
    // Excel truncates long numeric lexemes when saving. Both the original
    // wire formula and its Excel-saved spelling must produce the same bits.
    for spelling in ["formula", "saved_formula"] {
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Inputs").unwrap();
            book.add_sheet("Cases").unwrap();
            let sources = &fixture["source_cells"][year]["cells"];
            for address in ["A1", "A2", "A3"] {
                let at: CellRef = address.parse().unwrap();
                let source = &sources[address];
                let bits = u64::from_str_radix(source["bits"].as_str().unwrap(), 16).unwrap();
                let cell = Cell::from_scalar(
                    at,
                    Scalar::from(if address == "A1" {
                        f64::from_bits(bits)
                    } else {
                        -777.0
                    }),
                    system,
                )
                .unwrap();
                let cell = if let Some(formula) = source["formula"].as_str() {
                    cell.with_formula(Formula::from_file(formula, at))
                } else {
                    cell
                };
                book.sheet_mut("Inputs").unwrap().insert_cell(cell).unwrap();
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                book.sheet_mut("Cases")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(case[spelling].as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!((report.uncomputed, report.circular_count), (0, 0));
            for address in ["A1", "A2", "A3"] {
                let expected =
                    u64::from_str_radix(sources[address]["bits"].as_str().unwrap(), 16).unwrap();
                let at: CellRef = address.parse().unwrap();
                assert_eq!(
                    book.sheet("Inputs")
                        .unwrap()
                        .scalar(at)
                        .as_f64()
                        .unwrap()
                        .to_bits(),
                    expected,
                    "{year}: source {address}"
                );
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                checked += 1;
                let id = case["id"].as_str().unwrap();
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                assert_eq!(case["saved_cache_type"], "n", "{id}");
                let expected = u64::from_str_radix(case["raw_bits"].as_str().unwrap(), 16).unwrap();
                let saved = case["saved_cache_text"]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                assert_eq!(saved.to_bits(), expected, "{id}: cache transport");
                assert_eq!(
                    book.sheet("Cases")
                        .unwrap()
                        .scalar(at)
                        .as_f64()
                        .unwrap()
                        .to_bits(),
                    expected,
                    "{id}"
                );
            }
        }
    }
    assert_eq!(checked, 380);
}

#[cfg(feature = "internals")]
#[test]
fn lazy_selectors_evaluator_preserves_selected_types_and_reference_origin() {
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;

    let mut evaluator = ContextEvaluator::default();
    for (text, expected, calls) in [
        (
            "IF(FALSE,MYSTERY(A1),7)",
            Some(Scalar::from(7.0)),
            [0, 0, 0],
        ),
        ("IF(TRUE,MYSTERY(A1),7)", None, [0, 0, 0]),
        ("IF(FALSE,9)", Some(Scalar::from(false)), [0, 0, 0]),
        ("IF(FALSE,9,)", Some(Scalar::from(0.0)), [0, 0, 0]),
        ("IFERROR(NA(),ABS(-9))", Some(Scalar::from(9.0)), [0, 0, 0]),
        ("IFNA(NA(),TRUE)", Some(Scalar::from(true)), [0, 0, 0]),
        (
            "CHOOSE(1.9,9,SUM(A1:A5))",
            Some(Scalar::from(9.0)),
            [0, 0, 0],
        ),
        (
            "CHOOSE(2.9,9,SUM(A1:A5))",
            Some(Scalar::from(7.0)),
            [1, 0, 5],
        ),
        (
            "SUM(IF(TRUE,A1:A5,MYSTERY(1)))",
            Some(Scalar::from(7.0)),
            [1, 0, 5],
        ),
        (
            "SUM(CHOOSE(2,MYSTERY(1),A1:A5))",
            Some(Scalar::from(7.0)),
            [1, 0, 5],
        ),
        ("IFERROR(A1,SUM(A1:A5))", Some(Scalar::from(2.0)), [1, 1, 0]),
        ("IFNA(A1,SUM(A1:A5))", Some(Scalar::from(2.0)), [1, 1, 0]),
    ] {
        let formula = Formula::from_file(text, CellRef::new(0, 1));
        assert_eq!(
            evaluator.evaluate(&formula, false, false).unwrap(),
            expected,
            "{text}"
        );
        assert_eq!(evaluator.calls(), calls, "{text}");
        assert!(evaluator.is_clear(), "{text}");
    }
    // The shape probe stays conservative while the selected path can compute.
    assert!(!Formula::from_file("IF(FALSE,MYSTERY(A1),7)", CellRef::new(0, 1)).is_computed());
}

#[cfg(feature = "internals")]
#[test]
fn lazy_selectors_evaluator_cleans_a_selected_late_read_failure() {
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;

    let mut evaluator = ContextEvaluator::default();
    let formula = Formula::from_file("IF(TRUE,SUM(A1:A5),9)", CellRef::new(0, 1));
    let error = evaluator.evaluate(&formula, false, true).unwrap_err();
    assert!(
        matches!(error, yggdryl::Error::InvalidRecord { ref path, .. } if path == "context-fixture!A3")
    );
    assert_eq!(evaluator.calls(), [1, 0, 3]);
    assert!(evaluator.is_clear());
    let skipped = Formula::from_file("IF(FALSE,SUM(A1:A5),9)", CellRef::new(0, 1));
    assert_eq!(
        evaluator.evaluate(&skipped, false, true).unwrap(),
        Some(Scalar::from(9.0))
    );
    assert_eq!(evaluator.calls(), [0, 0, 0]);
    assert!(evaluator.is_clear());
}

#[cfg(feature = "internals")]
#[test]
fn multi_selectors_evaluator_visits_initial_inputs_and_only_the_chosen_result() {
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;
    let mut evaluator = ContextEvaluator::default();
    for (text, expected, calls) in [
        ("IFS(TRUE,11,A1,12)", Scalar::from(11.0), [1, 0, 0]),
        ("SWITCH(1,1,11,A1,12)", Scalar::from(11.0), [1, 0, 0]),
        ("IFS(FALSE,A1,TRUE,12)", Scalar::from(12.0), [0, 0, 0]),
        ("SWITCH(2,1,A1,2,12)", Scalar::from(12.0), [0, 0, 0]),
        (
            "IFS(TRUE,11,NA(),MYSTERY(1))",
            Scalar::from(11.0),
            [0, 0, 0],
        ),
        (
            "SWITCH(1,1,11,NA(),MYSTERY(1))",
            Scalar::from(11.0),
            [0, 0, 0],
        ),
        (
            "SUM(IFS(TRUE,A1:A5,TRUE,MYSTERY(1)))",
            Scalar::from(7.0),
            [1, 0, 5],
        ),
        (
            "SUM(SWITCH(1,1,A1:A5,MYSTERY(1)))",
            Scalar::from(7.0),
            [1, 0, 5],
        ),
        (
            "ISREF(IFS(TRUE,A1,TRUE,MYSTERY(1)))",
            Scalar::from(true),
            [1, 0, 0],
        ),
        (
            "ISREF(SWITCH(1,1,A1,MYSTERY(1)))",
            Scalar::from(true),
            [1, 0, 0],
        ),
    ] {
        let formula = Formula::from_file(text, CellRef::new(0, 1));
        assert_eq!(
            evaluator.evaluate(&formula, false, false).unwrap(),
            Some(expected),
            "{text}"
        );
        assert_eq!(evaluator.calls(), calls, "{text}");
        assert!(evaluator.is_clear(), "{text}");
    }
}

#[cfg(feature = "internals")]
#[test]
fn geometry_functions_read_reference_shape_without_scalar_or_range_values() {
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;
    let mut evaluator = ContextEvaluator::default();
    for (text, expected, calls) in [
        ("ROW(A1:A5)", Some(1.0), [1, 0, 0]),
        ("COLUMN(A1:A5)", Some(1.0), [1, 0, 0]),
        ("ROWS(A1:A5)", Some(5.0), [1, 0, 0]),
        ("COLUMNS(A1:A5)", Some(1.0), [1, 0, 0]),
        ("ROWS(@A1:A5)", Some(1.0), [1, 0, 0]),
        ("SUM(@A1:A5)", Some(2.0), [1, 1, 0]),
        ("ROWS({1,2;3,4})", Some(2.0), [0, 0, 0]),
        ("COLUMNS({1,2;3,4})", Some(2.0), [0, 0, 0]),
        ("ROWS(IF(TRUE,{1,2;3,4},0))", Some(2.0), [0, 0, 0]),
        ("ROWS(TRUE)", Some(1.0), [0, 0, 0]),
        // Native literal_arrays_native.json retains the exact numeric10.
        // Array evaluation adds no reference intake, scalarization or range read.
        ("SUM({1,2;3,4})", Some(10.0), [0, 0, 0]),
        ("ROW(7)", None, [0, 0, 0]),
    ] {
        let formula = Formula::from_entry(text, CellRef::new(0, 0)).unwrap();
        assert_eq!(
            evaluator.evaluate(&formula, false, false).unwrap(),
            expected.map(Scalar::from),
            "{text}"
        );
        assert_eq!(evaluator.calls(), calls, "{text}");
        assert!(evaluator.is_clear(), "{text}");
    }
}

/// The 248 native observations include these direct serial extractions.
#[test]
fn temporal_serial_extraction_matches_native_direct_cases() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/temporal_functions_native.json")).unwrap();
    assert_eq!(fixture["native_cache_equal"], 248);
    let mut covered = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let before_covered = covered;
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        let selected = fixture["cases"].as_array().unwrap().iter().filter(|case| {
            case["date_system"] == year
                && (case["group"] == "serial_direct" || case["group"] == "phantom_weekday")
                && matches!(
                    case["function"].as_str(),
                    Some("YEAR" | "MONTH" | "DAY" | "WEEKDAY")
                )
        });
        for case in selected {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
            covered += 1;
        }
        assert_eq!(
            covered - before_covered,
            42,
            "{year}: native fixture selection"
        );
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (42, 0), "{year}");
        for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
            case["date_system"] == year
                && (case["group"] == "serial_direct" || case["group"] == "phantom_weekday")
                && matches!(
                    case["function"].as_str(),
                    Some("YEAR" | "MONTH" | "DAY" | "WEEKDAY")
                )
        }) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
    assert_eq!(covered, 84);
}

/// A styled source cell projects serial 60 to the same typed day as 59 in
/// the 1900 system. Formula extraction still consumes its retained raw 60.
#[test]
fn temporal_serial_extraction_uses_styled_source_raw_serial() {
    use yggdryl::excel::{CellRange, StylePatch};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/temporal_functions_native.json")).unwrap();
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        for (row, serial) in [0.0, 1.0, 59.0, 60.0, 60.5, 61.0, 45_292.75]
            .into_iter()
            .enumerate()
        {
            book.sheet_mut("Inputs")
                .unwrap()
                .set_cell(CellRef::new((row + 1) as u32, 0), Scalar::from(serial))
                .unwrap();
        }
        book.set_style(
            "Inputs",
            &["A2:A8".parse::<CellRange>().unwrap()],
            &StylePatch {
                number_format: Some("mm-dd-yy".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
        book.add_sheet("Cases").unwrap();
        for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
            case["date_system"] == year
                && case["group"] == "styled_reference"
                && matches!(case["function"].as_str(), Some("YEAR" | "MONTH" | "DAY"))
        }) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (18, 0), "{year}");
        for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
            case["date_system"] == year
                && case["group"] == "styled_reference"
                && matches!(case["function"].as_str(), Some("YEAR" | "MONTH" | "DAY"))
        }) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .cell(at)
                    .unwrap()
                    .value()
                    .as_f64(),
                Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                "{}",
                case["id"]
            );
        }
    }
}

/// The future-prefixed spelling, rather than unprefixed DAYS, was calculated
/// by the guarded native run. DATE-dependent composition stays for DATE's slice.
#[test]
fn prefixed_days_matches_native_scalar_and_reference_cases() {
    use yggdryl::excel::{CellRange, StylePatch};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/days_weekday_native.json")).unwrap();
    assert_eq!(fixture["native_cache_equal"], 140);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        for (row, serial) in [0.0, 1.0, 59.0, 60.0, 60.5, 61.0].into_iter().enumerate() {
            book.sheet_mut("Inputs")
                .unwrap()
                .set_cell(CellRef::new((row + 1) as u32, 0), Scalar::from(serial))
                .unwrap();
        }
        book.set_style(
            "Inputs",
            &["A2:A7".parse::<CellRange>().unwrap()],
            &StylePatch {
                number_format: Some("mm-dd-yy".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
        book.add_sheet("Cases").unwrap();
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                case["date_system"] == year
                    && case["function"] == "DAYS"
                    && case["group"] != "construction"
            })
            .collect();
        assert_eq!(cases.len(), 17, "{year}: native DAYS selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (17, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

#[test]
fn weekday_return_modes_match_native_integer_fractional_and_error_codes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/days_weekday_native.json")).unwrap();
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
            .filter(|case| case["date_system"] == year && case["function"] == "WEEKDAY")
            .collect();
        assert_eq!(cases.len(), 52, "{year}: native WEEKDAY selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (52, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

#[test]
fn index_and_offset_native_references() {
    super::native_function_fixture(include_str!("../fixtures/indexed_offset_native.json"), 48);
    super::native_function_fixture(
        include_str!("../fixtures/indexed_offset_edges_native.json"),
        36,
    );
}

#[test]
fn index_reference_dependencies_follow_only_the_consumed_cells() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (at, text) in [
        ("A1", "=INDEX(A1:B1,1,2)"),
        ("B1", "7"),
        ("A2", "=SUM(INDEX(B1:C2,0,1))"),
        ("B2", "3"),
        ("C1", "=A1+1"),
        ("C2", "=A2+1"),
    ] {
        book.set_entry("Data", at.parse().unwrap(), text).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (4, 0, 0)
    );
    for (at, value) in [("A1", 7.0), ("A2", 10.0), ("C1", 8.0), ("C2", 11.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at.parse().unwrap()),
            Scalar::from(value)
        );
    }
    book.set_entry("Data", "B2".parse().unwrap(), "4").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Data").unwrap().scalar("C2".parse().unwrap()),
        Scalar::from(12.0)
    );
    book.set_entry("Data", "A1".parse().unwrap(), "=INDEX(A1:B1,1,1)")
        .unwrap();
    assert_eq!(book.recalculate().unwrap().circular_count, 1);
}

/// The documented WEEKDAY return-type table is closed; outside codes are #NUM!.
#[test]
fn weekday_out_of_table_modes_refuse_with_num() {
    for system in [DateSystem::Year1900, DateSystem::Year1904] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        for (index, code) in ["-1", "4", "19", "1E100", "-0.5", "17.9", "17"]
            .iter()
            .enumerate()
        {
            let at = CellRef::new((index + 1) as u32, 0);
            let formula = format!("WEEKDAY(61,{code})");
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(&formula, at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (7, 0), "{system}");
        for row in 1..=5 {
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .cell(CellRef::new(row, 0))
                    .unwrap()
                    .error()
                    .map(|e| e.as_str()),
                Some("#NUM!"),
                "{system} row {row}"
            );
        }
        // Fractional mode 17.9 truncates to 17, the documented Sunday-first mode.
        assert_eq!(
            book.sheet("Cases")
                .unwrap()
                .cell(CellRef::new(6, 0))
                .unwrap()
                .value()
                .as_f64(),
            book.sheet("Cases")
                .unwrap()
                .cell(CellRef::new(7, 0))
                .unwrap()
                .value()
                .as_f64(),
            "{system}"
        );
    }
}

/// DATE is the serial constructor, including the 1900 phantom and rollover.
#[test]
fn date_constructor_matches_native_phantom_and_rollover_cases() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/temporal_functions_native.json")).unwrap();
    assert_eq!(fixture["native_cache_equal"], 248);
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
            .filter(|case| case["date_system"] == year && case["function"] == "DATE")
            .collect();
        assert_eq!(cases.len(), 10, "{year}: native DATE selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (10, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

/// Native whole-second extraction stays tied to the raw serial, including phantom 60.
#[test]
fn clock_parts_match_native_rollover_and_source_cases() {
    use yggdryl::excel::{CellRange, StylePatch};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/clock_parts_native.json")).unwrap();
    assert_eq!(fixture["native_cache_equal"], 120);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        for (row, serial) in [0.0, 1.0, 59.0, 60.0, 60.5, 61.0].into_iter().enumerate() {
            book.sheet_mut("Inputs")
                .unwrap()
                .set_cell(CellRef::new((row + 2) as u32, 0), Scalar::from(serial))
                .unwrap();
        }
        book.set_style(
            "Inputs",
            &["A2:A7".parse::<CellRange>().unwrap()],
            &StylePatch {
                number_format: Some("mm-dd-yy hh:mm:ss.000".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
        book.add_sheet("Cases").unwrap();
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                case["date_system"] == year
                    && matches!(
                        case["function"].as_str(),
                        Some("HOUR" | "MINUTE" | "SECOND")
                    )
            })
            .collect();
        assert_eq!(cases.len(), 48, "{year}: native clock-part selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (48, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

/// TIME caches retain native floating bits after component truncation and wrap.
#[test]
fn time_constructor_matches_native_component_and_error_cases() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/clock_parts_native.json")).unwrap();
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
            .filter(|case| case["date_system"] == year && case["function"] == "TIME")
            .collect();
        assert_eq!(cases.len(), 12, "{year}: native TIME selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (12, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => {
                    let observed = cell.value().as_f64().unwrap();
                    let native =
                        u64::from_str_radix(case["value2_bits"].as_str().unwrap(), 16).unwrap();
                    assert_eq!(observed.to_bits(), native, "{}", case["id"]);
                }
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

/// EDATE preserves the source day with civil-month clamping; EOMONTH picks
/// the real civil month end, including beside the 1900 phantom serial.
#[test]
fn month_shift_matches_native_phantom_and_modern_cases() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/temporal_functions_native.json")).unwrap();
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
            .filter(|case| {
                case["date_system"] == year
                    && matches!(case["function"].as_str(), Some("EDATE" | "EOMONTH"))
            })
            .collect();
        assert_eq!(cases.len(), 10, "{year}: native month-shift selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (10, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

#[test]
fn month_shift_before_epoch_and_extreme_date_month_refuse_without_panicking() {
    for system in [DateSystem::Year1900, DateSystem::Year1904] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        let formulas = [
            "EDATE(0,-1)",
            "EDATE(1,-1)",
            "EDATE(0,-12)",
            "EDATE(1,-12)",
            "EOMONTH(0,-1)",
            "EOMONTH(1,-1)",
            "EOMONTH(0,-12)",
            "EOMONTH(1,-12)",
            "DATE(1900,-25769826575,1)",
        ];
        for (index, formula) in formulas.into_iter().enumerate() {
            let at = CellRef::new(index as u32, 0);
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (9, 0), "{system}");
        for row in 0..9 {
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .cell(CellRef::new(row, 0))
                    .unwrap()
                    .error()
                    .map(|error| error.as_str()),
                Some("#NUM!"),
                "{system} row {row}"
            );
        }
    }
}

/// Formula text dates pass the existing Entry grammar only where the workbook
/// need not guess a locale; locale-dependent slash dates keep their cache.
#[test]
fn parsed_temporal_text_uses_unambiguous_entry_contract() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/temporal_functions_native.json")).unwrap();
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
            .filter(|case| {
                case["date_system"] == year
                    && ((case["function"] == "DATEVALUE"
                        && matches!(
                            case["formula"].as_str(),
                            Some("DATEVALUE(\"2024-02-29\")" | "DATEVALUE(\"1900-02-29\")")
                        ))
                        || case["function"] == "TIMEVALUE")
            })
            .collect();
        assert_eq!(cases.len(), 6, "{year}: locale-free text selection");
        for case in &cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (6, 0), "{year}");
        for case in cases {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "n" => assert_eq!(
                    cell.value().as_f64(),
                    Some(case["cache"].as_str().unwrap().parse::<f64>().unwrap()),
                    "{}",
                    case["id"]
                ),
                "e" => assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    case["cache"].as_str(),
                    "{}",
                    case["id"]
                ),
                other => panic!("{}: unexpected cache type {other}", case["id"]),
            }
        }
    }
}

#[test]
fn ambiguous_formula_date_text_keeps_its_cache_without_workbook_locale() {
    let mut book = Workbook::new();
    book.add_sheet("Cases").unwrap();
    let at = CellRef::new(0, 0);
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("DATEVALUE(\"1/2/2024\")", at)),
        )
        .unwrap();
    let before = book.sheet("Cases").unwrap().revision();
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (0, 1));
    assert_eq!(book.sheet("Cases").unwrap().revision(), before);
    assert_eq!(
        book.sheet("Cases")
            .unwrap()
            .cell(at)
            .unwrap()
            .value()
            .as_f64(),
        Some(-777.0)
    );
}

#[test]
fn gcd_lcm_replay_all_88_native_integer_math_observations() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/pure_math_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut covered = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Values").unwrap();
        book.add_sheet("Cases").unwrap();
        for (address, value) in fixture["source_cells"]["Values"].as_object().unwrap() {
            let value = if let Some(value) = value.as_bool() {
                Scalar::from(value)
            } else if let Some(value) = value.as_f64() {
                Scalar::from(value)
            } else {
                Scalar::from(value.as_str().unwrap())
            };
            let at: CellRef = address.parse().unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .insert_cell(Cell::from_scalar(at, value, system).unwrap())
                .unwrap();
        }
        for case in cases
            .iter()
            .filter(|case| case["date_system"] == year && case["shape"]["kind"] == "source")
        {
            book.set_entry(
                "Values",
                case["cell"].as_str().unwrap().parse().unwrap(),
                case["actual"]["formula"].as_str().unwrap(),
            )
            .unwrap();
        }
        let chosen: Vec<_> = cases
            .iter()
            .filter(|case| {
                case["date_system"] == year
                    && matches!(case["shape"]["function"].as_str(), Some("GCD" | "LCM"))
            })
            .collect();
        assert_eq!(
            chosen.len(),
            44,
            "{year}: every native GCD/LCM case must run"
        );
        for case in &chosen {
            book.set_entry(
                "Cases",
                case["cell"].as_str().unwrap().parse().unwrap(),
                case["actual"]["formula"].as_str().unwrap(),
            )
            .unwrap();
        }
        book.calculate_all().unwrap();
        let sheet = book.sheet("Cases").unwrap();
        for case in chosen {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = sheet.cell(at).unwrap();
            let cache = &case["saved_cache"];
            match cache["type"].as_str().unwrap_or("n") {
                "e" => assert_eq!(
                    cell.error(),
                    Some(ExcelError::from_text(cache["value_text"].as_str().unwrap())),
                    "{}",
                    case["id"]
                ),
                "n" => {
                    let expected: f64 = cache["value_text"].as_str().unwrap().parse().unwrap();
                    assert_eq!(
                        sheet.scalar(at).as_f64().unwrap().to_bits(),
                        expected.to_bits(),
                        "{}",
                        case["id"]
                    );
                }
                other => panic!("unexpected native cache type {other}: {}", case["id"]),
            }
            covered += 1;
        }
    }
    assert_eq!(covered, 88);
}

#[test]
fn gcd_lcm_replay_native_integer_boundaries() {
    use yggdryl::excel::ExcelError;

    let reports = [
        include_str!("../fixtures/integer_boundary_native.json"),
        include_str!("../fixtures/integer_direct_boundary_native.json"),
    ];
    let mut seen = 0;
    for report in reports {
        let fixture: serde_json::Value = serde_json::from_str(report).unwrap();
        assert_eq!(fixture["native_run_passed"], true);
        assert_eq!(fixture["cleanup_completed"], true);
        let cases = fixture["cases"].as_array().unwrap();
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Cases").unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                book.set_entry(
                    "Cases",
                    case["cell"].as_str().unwrap().parse().unwrap(),
                    case["formula"].as_str().unwrap(),
                )
                .unwrap();
            }
            book.calculate_all().unwrap();
            let sheet = book.sheet("Cases").unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let cell = sheet.cell(at).unwrap();
                match case["cache_type"].as_str().unwrap() {
                    "e" => assert_eq!(
                        cell.error(),
                        Some(ExcelError::from_text(case["cache_value"].as_str().unwrap())),
                        "{}",
                        case["id"]
                    ),
                    "n" => {
                        let expected: f64 = case["cache_value"].as_str().unwrap().parse().unwrap();
                        assert_eq!(
                            sheet.scalar(at).as_f64().unwrap().to_bits(),
                            expected.to_bits(),
                            "{}",
                            case["id"]
                        );
                    }
                    other => panic!("unexpected native cache {other}: {}", case["id"]),
                }
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 54);
}

#[test]
fn lcm_replays_native_intermediate_error_order() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/lcm_order_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut seen = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            book.set_entry(
                "Cases",
                case["cell"].as_str().unwrap().parse().unwrap(),
                case["formula"].as_str().unwrap(),
            )
            .unwrap();
        }
        book.calculate_all().unwrap();
        let sheet = book.sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = sheet.cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "e" => assert_eq!(
                    cell.error(),
                    Some(ExcelError::from_text(case["cache_value"].as_str().unwrap())),
                    "{}",
                    case["id"]
                ),
                "n" => {
                    let expected: f64 = case["cache_value"].as_str().unwrap().parse().unwrap();
                    assert_eq!(
                        sheet.scalar(at).as_f64().unwrap().to_bits(),
                        expected.to_bits(),
                        "{}",
                        case["id"]
                    );
                }
                other => panic!("unexpected native cache {other}: {}", case["id"]),
            }
            seen += 1;
        }
    }
    assert_eq!(seen, 16);
}

/// Replay one independent family from an unchanged native report. Its source
/// error cells remain present, while the count proves every selected case ran.
fn lookup_native_function_subset(text: &str, function: &str, expected: usize, a1_only: bool) {
    let mut fixture: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array_mut().unwrap();
    cases.retain(|case| {
        case["shape"]["kind"] == "source"
            || (case["shape"]["function"] == function
                && (!a1_only || !case["wire_formula"].as_str().unwrap().contains("FALSE")))
    });
    assert_eq!(cases.len(), expected, "{function}: native case selection");
    fixture["native_cache_comparisons_equal"] = serde_json::json!(expected * 2);
    if function == "INDIRECT" {
        // Eight volatile calls per epoch run again even with unchanged inputs;
        // the ninth fixture cell is an ordinary source formula.
        fixture["incremental_evaluated_per_epoch"] = serde_json::json!(8);
    }
    super::native_function_fixture(&fixture.to_string(), expected as u64);
}

#[test]
fn match_native_sorted_exact_and_wildcards() {
    lookup_native_function_subset(
        include_str!("../fixtures/lookup_functions_native.json"),
        "MATCH",
        72,
        false,
    );
}

#[test]
fn vlookup_native_sorted_exact_and_index_edges() {
    lookup_native_function_subset(
        include_str!("../fixtures/lookup_functions_native.json"),
        "VLOOKUP",
        42,
        false,
    );
}

#[test]
fn hlookup_native_sorted_exact_and_index_edges() {
    lookup_native_function_subset(
        include_str!("../fixtures/lookup_functions_native.json"),
        "HLOOKUP",
        42,
        false,
    );
}

#[test]
fn lookup_native_sorted_vector() {
    lookup_native_function_subset(
        include_str!("../fixtures/lookup_functions_native.json"),
        "LOOKUP",
        16,
        false,
    );
}

#[test]
fn xlookup_native_scalar_exact_approximate_and_wildcard() {
    lookup_native_function_subset(
        include_str!("../fixtures/lookup_functions_native.json"),
        "XLOOKUP",
        56,
        false,
    );
}

#[test]
fn indirect_native_a1_reference_identity_and_range() {
    lookup_native_function_subset(
        include_str!("../fixtures/reference_functions_native.json"),
        "INDIRECT",
        18,
        false,
    );
}

#[test]
fn address_native_a1_text_and_error_edges() {
    lookup_native_function_subset(
        include_str!("../fixtures/address_functions_native.json"),
        "ADDRESS",
        50,
        true,
    );
}

#[test]
fn lookup_native_lazy_fallback_and_later_error() {
    let fixture = include_str!("../fixtures/lookup_dependency_native.json");
    let document: serde_json::Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(document["native_run_passed"], true);
    assert_eq!(document["cleanup_completed"], true);
    assert_eq!(document["cases"].as_array().unwrap().len(), 18);
    super::native_function_fixture(fixture, 18);
}

#[test]
fn lookup_exact_prefix_avoids_later_key_cycle() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.add_sheet("Cases").unwrap();
    for (at, input) in [
        ("A1", "1"),
        ("A2", "3"),
        ("A3", "=Cases!B2+1"),
        ("B1", "10"),
        ("B2", "30"),
        ("B3", "50"),
        ("J1", "1"),
        ("K1", "3"),
        ("L1", "=Cases!B3+1"),
        ("J2", "10"),
        ("K2", "30"),
        ("L2", "50"),
    ] {
        book.set_entry("Data", at.parse().unwrap(), input).unwrap();
    }
    for (at, formula) in [
        ("B2", "=MATCH(1,Data!A1:A3,0)"),
        ("B3", "=HLOOKUP(1,Data!J1:L2,2,FALSE)"),
        ("B4", "=VLOOKUP(1,Data!A1:B3,2,FALSE)"),
        ("B5", "=_xlfn.XLOOKUP(1,Data!A1:A3,Data!B1:B3)"),
    ] {
        book.set_entry("Cases", at.parse().unwrap(), formula)
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        report.circular_count, 0,
        "unused key suffixes cannot create cycles"
    );
    for (at, expected) in [("B2", 1.0), ("B3", 10.0), ("B4", 10.0), ("B5", 10.0)] {
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at.parse().unwrap()),
            Scalar::from(expected),
            "{at}: an exact earlier match must finish before a later key formula"
        );
    }
}

#[test]
fn lcm_replays_native_u64_intermediate_order() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/lcm_u64_order_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut seen = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            book.set_entry(
                "Cases",
                case["cell"].as_str().unwrap().parse().unwrap(),
                case["formula"].as_str().unwrap(),
            )
            .unwrap();
        }
        book.calculate_all().unwrap();
        let sheet = book.sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = sheet.cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "e" => assert_eq!(
                    cell.error(),
                    Some(ExcelError::from_text(case["cache_value"].as_str().unwrap())),
                    "{}",
                    case["id"]
                ),
                "n" => {
                    let expected: f64 = case["cache_value"].as_str().unwrap().parse().unwrap();
                    assert_eq!(
                        sheet.scalar(at).as_f64().unwrap().to_bits(),
                        expected.to_bits(),
                        "{}",
                        case["id"]
                    );
                }
                other => panic!("unexpected native cache {other}: {}", case["id"]),
            }
            seen += 1;
        }
    }
    assert_eq!(seen, 16);
}

#[test]
fn lcm_fractional_zero_clears_provisional_overflow() {
    // Excel truncates nonnegative numeric arguments before the exact pair fold.
    // A later 0.5 is therefore the same annihilating integer as a later zero.
    let mut book = Workbook::new();
    book.add_sheet("Cases").unwrap();
    book.set_entry(
        "Cases",
        "A1".parse().unwrap(),
        "=LCM(POWER(2,52),POWER(2,52)-1,0.5)",
    )
    .unwrap();
    book.calculate_all().unwrap();
    assert_eq!(
        book.sheet("Cases")
            .unwrap()
            .scalar("A1".parse().unwrap())
            .as_f64(),
        Some(0.0)
    );
}

#[test]
fn factorial_replays_native_order_and_boundary() {
    use yggdryl::excel::ExcelError;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/factorial_order_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut seen = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            book.set_entry(
                "Cases",
                case["cell"].as_str().unwrap().parse().unwrap(),
                case["formula"].as_str().unwrap(),
            )
            .unwrap();
        }
        book.calculate_all().unwrap();
        let sheet = book.sheet("Cases").unwrap();
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = sheet.cell(at).unwrap();
            match case["cache_type"].as_str().unwrap() {
                "e" => assert_eq!(
                    cell.error(),
                    Some(ExcelError::from_text(case["cache_value"].as_str().unwrap())),
                    "{}",
                    case["id"]
                ),
                "n" => {
                    let expected: f64 = case["cache_value"].as_str().unwrap().parse().unwrap();
                    assert_eq!(
                        sheet.scalar(at).as_f64().unwrap().to_bits(),
                        expected.to_bits(),
                        "{}",
                        case["id"]
                    );
                }
                other => panic!("unexpected native cache {other}: {}", case["id"]),
            }
            seen += 1;
        }
    }
    assert_eq!(seen, 354);
}

#[test]
fn financial_annuities_keep_omitted_defaults_scalar_dependencies_and_errors() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", CellRef::new(0, 0), "1").unwrap();
    for (row, formula, expected) in [
        (0, "FV(A1,3,1,0)", -7.0),
        (1, "PV(A1,3,1,0)", -0.875),
        (2, "FV(A1,3,1,,1)", -14.0),
        (3, "FV(0,3,1,2)", -5.0),
    ] {
        let host = CellRef::new(row, 1);
        book.set_entry("Data", host, &format!("={formula}"))
            .unwrap();
        book.calculate_all().unwrap();
        assert_eq!(
            book.sheet("Data").unwrap().scalar(host).as_f64(),
            Some(expected)
        );
    }
    book.set_entry("Data", CellRef::new(0, 0), "0").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(CellRef::new(0, 1))
            .as_f64(),
        Some(-3.0)
    );
    book.set_entry("Data", CellRef::new(0, 0), "=1/0").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .cell(CellRef::new(0, 1))
            .unwrap()
            .error()
            .map(|v| v.as_str()),
        Some("#DIV/0!")
    );
}

#[test]
fn financial_npv_distinguishes_missing_slots_from_sparse_reference_blanks() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (row, formula, expected) in [
        (0, "NPV(1,1,,3)", 0.875),
        (1, "NPV(1,1,Z1,3)", 1.25),
        (2, "NPV(0,10000000000000000,1,-10000000000000000)", 0.0),
    ] {
        let host = CellRef::new(row, 0);
        book.set_entry("Data", host, &format!("={formula}"))
            .unwrap();
        book.calculate_all().unwrap();
        assert_eq!(
            book.sheet("Data").unwrap().scalar(host).as_f64(),
            Some(expected)
        );
    }
    book.set_entry("Data", CellRef::new(0, 25), "1").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(CellRef::new(1, 0))
            .as_f64(),
        Some(1.125)
    );
}

#[cfg(feature = "internals")]
#[test]
fn lookup_range_callback_count_is_prefix_and_resume_linear() {
    use yggdryl::internals::excel_formula_eval::ContextEvaluator;
    for length in [64_u64, 4_096] {
        let formula =
            Formula::from_entry(&format!("MATCH(1,A1:A{length},0)"), CellRef::new(0, 1)).unwrap();
        let mut evaluator = ContextEvaluator::default();
        assert_eq!(
            evaluator
                .evaluate_lookup(&formula, length, 0, None)
                .unwrap(),
            Some(Scalar::from(1.0)),
            "first match over {length}"
        );
        assert_eq!(evaluator.calls()[2], 1, "exact match visited a suffix");
        assert!(evaluator.is_clear());
        assert_eq!(
            evaluator
                .evaluate_lookup(&formula, length, length - 1, Some(length / 2))
                .unwrap(),
            Some(Scalar::from(length as f64)),
            "last match over {length}"
        );
        assert_eq!(
            evaluator.calls()[2],
            length as usize,
            "resumed lookup replayed a prefix instead of retaining its ordinal"
        );
        assert!(evaluator.is_clear());
    }
}

#[test]
fn literal_arrays_native_leaf_publication_and_aggregate_origin_are_exact() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/literal_arrays_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 210);
    assert_eq!(
        cases
            .iter()
            .filter(|case| case["initial_leaf_slice"] == true)
            .count(),
        134
    );
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Cases").unwrap();
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| case["date_system"] == year && case["initial_leaf_slice"] == true)
                .collect();
            for case in &selected {
                let host: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut("Cases")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(host, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), host)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (67, 0, 0),
                "{year} saved={saved}"
            );
            for case in selected {
                let host: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let cell = book.sheet("Cases").unwrap().cell(host).unwrap();
                let expected = &case["saved_cache"];
                match expected["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            expected["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap()
                                .to_bits()
                        ),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    "b" => assert_eq!(
                        cell.value().as_bool(),
                        Some(expected["value_text"] == "1"),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    "str" => assert_eq!(
                        cell.value().as_str(),
                        expected["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    "e" => assert_eq!(
                        cell.error().map(|v| v.as_str()),
                        expected["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native array cache {other}"),
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 268);
}

#[test]
fn literal_arrays_keep_numeric_text_and_boolean_aggregate_rules_distinct() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (row, (formula, expected, error)) in [
        ("AVERAGEA({TRUE,FALSE})", None, Some("#DIV/0!")),
        ("AVERAGEA({\"2\",\"3\"})", Some(0.0), None),
        ("MINA({1,\"3\"})", Some(1.0), None),
        ("MINA({-1,\"3\"})", Some(-1.0), None),
        ("MAXA({-1,\"3\"})", Some(-1.0), None),
        ("GCD({1,TRUE})", None, Some("#VALUE!")),
        ("GCD({1,\"3\"})", Some(1.0), None),
        ("LCM({1,TRUE})", None, Some("#VALUE!")),
        ("LCM({1,\"3\"})", Some(3.0), None),
    ]
    .into_iter()
    .enumerate()
    {
        let host = CellRef::new(row as u32, 0);
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(host, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, host)),
            )
            .unwrap();
        // Every value is also retained in the independent wrapped native run.
        let report = book.calculate_all().unwrap();
        assert_eq!(report.uncomputed, 0, "{formula}");
        let cell = book.sheet("Data").unwrap().cell(host).unwrap();
        assert_eq!(cell.error().map(|v| v.as_str()), error, "{formula}");
        if error.is_none() {
            assert_eq!(cell.value().as_f64(), expected, "{formula}");
        }
    }
}

#[test]
fn reference_operators_native_unions_intersections_and_ranges() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/reference_operators_native.json")).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for run in fixture["runs"].as_array().unwrap() {
        assert_eq!(run["native_run_passed"], true);
        assert_eq!(run["cleanup_completed"], true);
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            for saved in [false, true] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                book.add_sheet("Values").unwrap();
                book.add_sheet("Cases").unwrap();
                for (address, value) in run["source_cells"]["Values"].as_object().unwrap() {
                    let value = if let Some(text) = value.as_str() {
                        Scalar::from(text)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut("Values")
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
                let cases: Vec<_> = run["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|case| case["date_system"] == year)
                    .collect();
                for case in &cases {
                    let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                    let text = if saved {
                        &case["saved_cache"]["formula_text"]
                    } else {
                        &case["wire_formula"]
                    };
                    book.sheet_mut("Cases")
                        .unwrap()
                        .insert_cell(
                            Cell::from_scalar(at, Scalar::from(-777.0), system)
                                .unwrap()
                                .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                        )
                        .unwrap();
                }
                let report = book.calculate_all().unwrap();
                assert_eq!(report.circular_count, 0);
                for case in &cases {
                    let at = case["cell"].as_str().unwrap().parse().unwrap();
                    let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
                    let cache = &case["saved_cache"];
                    let text = cache["value_text"].as_str().unwrap();
                    let equal = match cache["type"].as_str().unwrap() {
                        "e" => cell.error().is_some_and(|error| error.as_str() == text),
                        "b" => {
                            cell.error().is_none() && cell.value().as_bool() == Some(text == "1")
                        }
                        "n" => {
                            cell.error().is_none()
                                && cell.value().as_f64().map(f64::to_bits)
                                    == Some(text.parse::<f64>().unwrap().to_bits())
                        }
                        other => panic!("unexpected oracle type {other}"),
                    };
                    if !equal {
                        failures.push(format!(
                            "{} saved={saved}: {} expected {text}, got {:?}",
                            case["id"], case["wire_formula"], cell
                        ));
                    }
                    checked += 1;
                }
            }
        }
    }
    // 116 original + 24 colon shapes, each replayed as authored and saved.
    assert_eq!(checked, 280);
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn literal_arrays_explicit_projection_keeps_native_scalar_only_boundaries() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (address, formula, expected) in [
        ("A1", "SUM(_xlfn.SINGLE({1,2}))", Scalar::from(1.0)),
        ("A2", "CONCATENATE({\"a\",\"b\"},\"x\")", Scalar::from("ax")),
        ("A3", "CONCATENATE({\"a\",\"b\"})", Scalar::from("a")),
    ] {
        let host: CellRef = address.parse().unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(host, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, host)),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(report.uncomputed, 0, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(host),
            expected,
            "{formula}"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn literal_arrays_context_lends_the_original_arena_without_reference_reads() {
    let mut evaluator = yggdryl::internals::excel_formula_eval::ContextEvaluator::default();
    for (formula, expected) in [
        ("{1,TRUE,\"3\"}", Scalar::from(1.0)),
        ("SUM(_xlfn.SINGLE({1,2}))", Scalar::from(1.0)),
        ("CONCATENATE({\"a\",\"b\"})", Scalar::from("a")),
    ] {
        let formula = Formula::from_file(formula, CellRef::new(0, 0));
        assert_eq!(
            evaluator.evaluate(&formula, false, false).unwrap(),
            Some(expected)
        );
        assert_eq!(evaluator.calls(), [0, 0, 0]);
        assert!(evaluator.is_clear());
    }
}

#[test]
fn reference_operators_admit_only_consumed_geometry() {
    use yggdryl::{
        Scalar,
        excel::{CellRef, Workbook},
    };
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", at("A2"), "2").unwrap();
    book.set_entry("Data", at("A3"), "3").unwrap();
    for (address, text) in [
        ("A1", "=SUM(A1:A3 A2:A3)"),
        ("B1", "=INDEX((B1,A2),1,1,2)"),
        ("C1", "=_xlfn.CONCAT((C1,A2))"),
        ("D1", "=COUNTBLANK((D1,A2))"),
        ("E1", "=SUMPRODUCT((E1,A2))"),
        ("F1", "=SUMIF((F1,A2),\">0\")"),
        ("G1", "=ROWS((G1,A2))"),
        ("H1", "=ISREF((H1,A2))"),
        ("I1", "=SUM((A2,A2))"),
    ] {
        book.set_entry("Data", at(address), text).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (9, 0, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")),
        Scalar::from(2.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("H1")),
        Scalar::from(true)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("I1")),
        Scalar::from(4.0)
    );
    for (address, error) in [
        ("C1", "#VALUE!"),
        ("D1", "#VALUE!"),
        ("E1", "#VALUE!"),
        ("F1", "#VALUE!"),
        ("G1", "#REF!"),
    ] {
        assert_eq!(
            book.sheet("Data")
                .unwrap()
                .cell(at(address))
                .unwrap()
                .error()
                .map(|error| error.as_str()),
            Some(error),
            "{address}"
        );
    }
    book.set_entry("Data", at("A2"), "7").unwrap();
    let report = book.recalculate().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (3, 0, 0)
    );
    for (address, value) in [("A1", 10.0), ("B1", 7.0), ("I1", 14.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(address)),
            Scalar::from(value)
        );
    }
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn reference_operators_preserve_real_union_cycles() {
    use yggdryl::excel::{CellRef, Workbook};
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", CellRef::new(0, 0), "=SUM((A1,A2))")
        .unwrap();
    book.set_entry("Data", CellRef::new(1, 0), "2").unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 1, 1)
    );
    assert_eq!(report.circular, [("Data".into(), CellRef::new(0, 0))]);
}

#[test]
fn literal_arrays_mapped_operations_match_native_elements_and_scalar_boundaries() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/mapped_arrays_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 258);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            // Distinct native corpora reuse coordinates, so each retains its own workbook.
            for corpus in ["literal", "wrapped", "mapping"] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                book.add_sheet("Values").unwrap();
                book.add_sheet("Cases").unwrap();
                book.set_entry("Values", CellRef::new(0, 0), "10").unwrap();
                book.set_entry("Values", CellRef::new(1, 0), "20").unwrap();
                let selected: Vec<_> = cases
                    .iter()
                    .filter(|case| {
                        case["date_system"] == year
                            && case["corpus"] == corpus
                            && case["mapped_operations_slice"] == true
                    })
                    .collect();
                for case in &selected {
                    let host: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                    let formula = if saved {
                        &case["saved_cache"]["formula_text"]
                    } else {
                        &case["wire_formula"]
                    };
                    book.sheet_mut("Cases")
                        .unwrap()
                        .insert_cell(
                            Cell::from_scalar(host, Scalar::from(-777.0), system)
                                .unwrap()
                                .with_formula(Formula::from_file(formula.as_str().unwrap(), host)),
                        )
                        .unwrap();
                }
                let report = book.calculate_all().unwrap();
                assert_eq!(
                    (report.evaluated, report.uncomputed, report.circular_count),
                    (selected.len() as u64, 0, 0),
                    "{year} {corpus} saved={saved}"
                );
                for case in selected {
                    let host: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                    let cell = book.sheet("Cases").unwrap().cell(host).unwrap();
                    let expected = &case["saved_cache"];
                    match expected["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(
                                expected["value_text"]
                                    .as_str()
                                    .unwrap()
                                    .parse::<f64>()
                                    .unwrap()
                                    .to_bits()
                            ),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|v| v.as_str()),
                            expected["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected mapped native cache {other}"),
                    }
                    checked += 1;
                }
                assert_eq!(book.recalculate().unwrap().evaluated, 0);
            }
        }
    }
    assert_eq!(checked, 108);
}

#[cfg(feature = "internals")]
#[test]
fn literal_arrays_capture_scalar_references_once_before_broadcast() {
    let mut evaluator = yggdryl::internals::excel_formula_eval::ContextEvaluator::default();
    let formula = Formula::from_file("SUM({1,2,3,4}+A1)", CellRef::new(0, 1));
    // Fixture A1 is 2; four elements add it, but intake/scalarization occur once.
    assert_eq!(
        evaluator.evaluate(&formula, false, false).unwrap(),
        Some(Scalar::from(18.0))
    );
    assert_eq!(evaluator.calls(), [1, 1, 0]);
    assert!(evaluator.is_clear());
}

#[test]
fn literal_arrays_sumproduct_uses_native_array_elements_and_selected_shapes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/mapped_arrays_native.json")).unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
        matches!(
            case["wire_formula"].as_str(),
            Some(
                "SUMPRODUCT({1,2},{3,4})"
                    | "SUMPRODUCT({1,TRUE,\"3\"})"
                    | "SUMPRODUCT({1,2}+1,{3,4})"
                    | "SUMPRODUCT(IF(TRUE,{1,2},{3,4}),{3,4})"
            )
        )
    }) {
        for saved in [false, true] {
            let system = if case["date_system"] == "1904" {
                DateSystem::Year1904
            } else {
                DateSystem::Year1900
            };
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Cases").unwrap();
            let host: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let formula = if saved {
                &case["saved_cache"]["formula_text"]
            } else {
                &case["wire_formula"]
            };
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(host, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula.as_str().unwrap(), host)),
                )
                .unwrap();
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed),
                (1, 0),
                "{} saved={saved}",
                case["id"]
            );
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .scalar(host)
                    .as_f64()
                    .map(f64::to_bits),
                Some(
                    case["saved_cache"]["value_text"]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap()
                        .to_bits()
                ),
                "{} saved={saved}",
                case["id"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 16);
}

#[test]
fn literal_arrays_sumproduct_keeps_composed_scalar_mapping_eligible() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (row, formula, expected) in [
        (0, "SUMPRODUCT(ABS({-1,2})+1)", 5.0),
        (1, "SUMPRODUCT(ROUND({1.2,2.8},0)+1)", 6.0),
    ] {
        let at = CellRef::new(row, 0);
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(report.uncomputed, 0, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at).as_f64(),
            Some(expected),
            "{formula}"
        );
    }
}

#[test]
fn literal_arrays_selected_positions_match_native_broadcast_and_error_boundaries() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/mapped_arrays_native.json")).unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
        matches!(
            case["wire_formula"].as_str(),
            Some(
                "SUM(IF({TRUE,FALSE},{1,2},{3,4}))"
                    | "SUM(IF({TRUE,FALSE},{1,2},{3,4})+1)"
                    | "SUM(IF({TRUE,FALSE},1,4))"
                    | "SUM(IF({TRUE;FALSE},{1,2},{3,4}))"
                    | "SUM(IF({TRUE,FALSE},Values!A1,Values!A2))"
                    | "SUM(CHOOSE({1,2},{1,2},{3,4}))"
                    | "SUM(IFERROR({1,#N/A},{3,4}))"
            )
        )
    }) {
        for saved in [false, true] {
            let system = if case["date_system"] == "1904" {
                DateSystem::Year1904
            } else {
                DateSystem::Year1900
            };
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Values").unwrap();
            book.add_sheet("Cases").unwrap();
            book.set_entry("Values", CellRef::new(0, 0), "10").unwrap();
            book.set_entry("Values", CellRef::new(1, 0), "20").unwrap();
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let formula = if saved {
                &case["saved_cache"]["formula_text"]
            } else {
                &case["wire_formula"]
            };
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                )
                .unwrap();
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (1, 0, 0),
                "{} saved={saved}",
                case["id"]
            );
            assert_eq!(
                book.sheet("Cases")
                    .unwrap()
                    .scalar(at)
                    .as_f64()
                    .map(f64::to_bits),
                Some(
                    case["saved_cache"]["value_text"]
                        .as_str()
                        .unwrap()
                        .parse::<f64>()
                        .unwrap()
                        .to_bits()
                ),
                "{} saved={saved}",
                case["id"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 28);
}

#[cfg(feature = "internals")]
#[test]
fn literal_arrays_mapped_storage_depends_on_plan_not_output_grid() {
    let horizontal = std::iter::repeat_n("1", 64).collect::<Vec<_>>().join(",");
    let mut capacities = Vec::new();
    for rows in [1, 64] {
        let vertical = std::iter::repeat_n("1", rows).collect::<Vec<_>>().join(";");
        let formula = Formula::from_file(
            &format!("SUM({{{horizontal}}}+{{{vertical}}})"),
            CellRef::new(0, 0),
        );
        let mut evaluator = yggdryl::internals::excel_formula_eval::ContextEvaluator::default();
        assert_eq!(
            evaluator.evaluate(&formula, false, false).unwrap(),
            Some(Scalar::from((rows * 128) as f64))
        );
        capacities.push(evaluator.array_capacity());
        assert!(evaluator.is_clear());
    }
    assert_eq!(
        capacities[0], capacities[1],
        "64 vs4096 output elements retained a value grid: {capacities:?}"
    );
}
