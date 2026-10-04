//! `rust/src/excel/formula/value.rs`: computed error operands land as error cells.

use yggdryl::Scalar;
use yggdryl::excel::{
    Cell, CellKind, CellRef, DateSystem, ExcelError, Formula, NumberFormat, StylePatch, Workbook,
};

#[test]
fn error_literal_replaces_only_the_formula_cache() {
    let at: CellRef = "B1".parse().unwrap();
    let mut workbook = Workbook::new();
    workbook
        .add_sheet("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_entry("#DIV/0!", at).unwrap()),
        )
        .unwrap();

    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (1, 0));
    let cell = workbook.sheet("Data").unwrap().cell(at).unwrap();
    assert_eq!(cell.kind(), CellKind::Error);
    assert_eq!(cell.error(), Some(ExcelError::Div0));
    assert_eq!(cell.value(), &Scalar::Null);
    assert_eq!(cell.formula().unwrap().at(at).to_string(), "#DIV/0!");
}

#[test]
fn styled_formula_numeric_cache_is_temporal_before_and_after_roundtrip() {
    let at: CellRef = "A1".parse().unwrap();
    let mut workbook = Workbook::new();
    workbook
        .add_sheet("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_entry("45292+0", at).unwrap()),
        )
        .unwrap();
    workbook
        .set_style(
            "Data",
            &["A1".parse().unwrap()],
            &StylePatch {
                number_format: Some("yyyy-mm-dd".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
    let style = workbook.sheet("Data").unwrap().cell(at).unwrap().style();
    assert_eq!(workbook.calculate_all().unwrap().evaluated, 1);
    let cell = workbook.sheet("Data").unwrap().cell(at).unwrap();
    assert_eq!(cell.format(), NumberFormat::Date);
    assert_eq!(cell.style(), style);
    assert_eq!(cell.value(), &Scalar::date32(19_723));
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    let cell = reopened.sheet("Data").unwrap().cell(at).unwrap();
    assert_eq!(cell.format(), NumberFormat::Date);
    assert_eq!(cell.value(), &Scalar::date32(19_723));
}

/// The cited individual Excel cases are stable across COM save and XML cache.
/// The enclosing 324-case run failed an unrelated localized TEXT calibration.
#[test]
fn direct_numeric_coercion_matches_native_observations() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/coercion_native.json")).unwrap();
    assert_eq!(fixture["full_run_passed"], false);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 12);
    let cases = fixture["cases"].as_array().unwrap();
    for (id, key, expected) in [
        ("arithmetic-text", "value2", "4010000000000000"),
        ("sum-direct-coercion", "value2", "4010000000000000"),
        ("arithmetic-bad-text", "cache_text", "#VALUE!"),
        ("sum-array-coercion", "cache_text", "1"),
        ("sum-reference-coercion", "cache_text", "5"),
    ] {
        let case = cases.iter().find(|case| case["id"] == id).unwrap();
        let observed = if key == "value2" {
            &case[key]["ieee754_hex"]
        } else {
            &case[key]
        };
        assert_eq!(observed.as_str(), Some(expected));
    }

    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (address, formula) in [
        ("A1", "\"3\"+TRUE"),
        ("B1", "\"abc\"+1"),
        ("C1", "SUM(TRUE,\"3\")"),
        ("D1", "1=\"1\""),
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
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (4, 0));
    let sheet = book.sheet("Data").unwrap();
    for address in ["A1", "C1"] {
        assert_eq!(
            sheet
                .scalar(address.parse().unwrap())
                .as_f64()
                .unwrap()
                .to_bits(),
            4.0_f64.to_bits()
        );
    }
    assert_eq!(
        sheet.cell("B1".parse().unwrap()).unwrap().error(),
        Some(ExcelError::Value)
    );
    assert_eq!(sheet.scalar("D1".parse().unwrap()), Scalar::from(false));
}

#[cfg(feature = "internals")]
#[test]
fn entry_owned_direct_number_intake_is_typed_once() {
    use yggdryl::internals::excel_formula_value::{number_blank, number_boolean, number_text};

    assert_eq!(number_blank(), Some(Ok(0.0)));
    assert_eq!(number_boolean(true), Some(Ok(1.0)));
    assert_eq!(number_boolean(false), Some(Ok(0.0)));
    for (text, expected) in [
        ("3", 3.0),
        (" 3 ", 3.0),
        ("12.5%", 0.125),
        ("1,234.50", 1234.5),
    ] {
        assert_eq!(
            number_text(text, DateSystem::Year1900),
            Some(Ok(expected)),
            "{text}"
        );
    }
    assert_eq!(
        number_text("abc", DateSystem::Year1900),
        Some(Err(ExcelError::Value))
    );
    assert_eq!(
        number_text("", DateSystem::Year1900),
        Some(Err(ExcelError::Value))
    );
    assert_eq!(
        number_text("=1+2", DateSystem::Year1900),
        Some(Err(ExcelError::Value))
    );
    assert_eq!(
        number_text("'3", DateSystem::Year1900),
        Some(Err(ExcelError::Value))
    );
    assert_eq!(
        number_text("2024-01-31", DateSystem::Year1900),
        Some(Ok(45_322.0))
    );
    assert_eq!(
        number_text("2024-01-31", DateSystem::Year1904),
        Some(Ok(43_860.0))
    );
    assert_eq!(
        number_text("23:45:30", DateSystem::Year1900)
            .unwrap()
            .unwrap()
            .to_bits(),
        0x3fef_ad82_d82d_82d8
    );
}

/// Excel 16.0 build 20430.0: all 230 cases are retained in the fixture.
/// This test executes only direct scalar formulas; range, reference and array
/// cases remain evidence for their separate operand-origin implementation.
#[test]
fn native_direct_coercion_preserves_typed_cache_and_f64_bits() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/coercion_full_native.json")).unwrap();
    assert_eq!(fixture["excel_version"], "16.0");
    assert_eq!(fixture["excel_build"], "20430.0");
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["execution_only"], true);
    assert_eq!(fixture["rust_equivalence_checked"], false);
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 230);
    let mut checked = 0;
    for (system_name, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        let sheet = book.add_sheet("Data").unwrap();
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| {
                if case["date_system"] != system_name {
                    return false;
                }
                let origin = case["parameters"]["origin"].as_str().unwrap();
                origin == "direct"
                    || (origin == "control"
                        && !case["formula"].as_str().unwrap().contains("Inputs!")
                        && !case["formula"].as_str().unwrap().contains('{'))
            })
            .collect();
        assert_eq!(cases.len(), 54);
        for (index, case) in cases.iter().enumerate() {
            let at: CellRef = format!("B{}", index + 1).parse().unwrap();
            sheet
                .insert_cell(
                    Cell::from_scalar(at, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(case["formula"].as_str().unwrap(), at)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed),
            (54, 0),
            "{system_name}"
        );
        let sheet = book.sheet("Data").unwrap();
        for (index, case) in cases.iter().enumerate() {
            let at: CellRef = format!("B{}", index + 1).parse().unwrap();
            let cell = sheet.cell(at).unwrap();
            assert_eq!(case["cache_comparison_actual"], "equal", "{}", case["id"]);
            assert_eq!(
                case["cache_comparison_after_save"], "equal",
                "{}",
                case["id"]
            );
            assert_eq!(
                case["actual_value2"], case["after_save_value2"],
                "{}",
                case["id"]
            );
            match case["cache_type"].as_str().unwrap() {
                "n" => {
                    let expected = u64::from_str_radix(
                        case["actual_value2"]["ieee754_hex"].as_str().unwrap(),
                        16,
                    )
                    .unwrap();
                    assert_eq!(
                        cell.value().as_f64().unwrap().to_bits(),
                        expected,
                        "{}",
                        case["id"]
                    );
                }
                "e" => assert_eq!(
                    cell.error(),
                    Some(ExcelError::from_text(
                        case["cache_value_text"].as_str().unwrap()
                    )),
                    "{}",
                    case["id"]
                ),
                "b" => {
                    assert_eq!(cell.kind(), CellKind::Boolean, "{}", case["id"]);
                    assert_eq!(
                        cell.value(),
                        &Scalar::from(case["actual_value2"]["value"].as_bool().unwrap()),
                        "{}",
                        case["id"]
                    );
                }
                "str" => {
                    assert_eq!(cell.kind(), CellKind::FormulaString, "{}", case["id"]);
                    assert_eq!(
                        cell.value(),
                        &Scalar::from(case["actual_value2"]["value"].as_str().unwrap()),
                        "{}",
                        case["id"]
                    );
                }
                other => panic!("unexpected native cache type {other}"),
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 108);
}

/// Unary plus preserves empty text as a string cache, including save/reopen.
#[test]
fn unary_plus_empty_text_keeps_formula_string_cache_through_roundtrip() {
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    let at: CellRef = "A1".parse().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("+\"\"", at)),
        )
        .unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    let cell = book.sheet("Data").unwrap().cell(at).unwrap();
    assert_eq!(cell.kind(), CellKind::FormulaString);
    assert_eq!(cell.value(), &Scalar::from(""));
    let bytes = book.into_bytes().unwrap();
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.clone(),
    ))));
    let xml = archive.read_member("xl/worksheets/sheet1.xml").unwrap();
    let xml = std::str::from_utf8(&xml).unwrap();
    assert!(xml.contains("t=\"str\""), "{xml}");
    assert!(xml.contains("<v></v>"), "{xml}");
    let reopened = Workbook::from_bytes(bytes).unwrap();
    let cell = reopened.sheet("Data").unwrap().cell(at).unwrap();
    assert_eq!(cell.kind(), CellKind::FormulaString);
    assert_eq!(cell.value(), &Scalar::from(""));
}

#[test]
fn ordered_comparisons_keep_contextual_blanks_identical_text_and_incremental_edges() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet.set_cell(CellRef::new(0, 0), 1.0).unwrap();
    sheet.set_cell(CellRef::new(0, 1), 2.0).unwrap();
    let operators = ["=", "<>", "<", ">", "<=", ">="];
    for (index, operator) in operators.iter().enumerate() {
        for (column, left, right) in [(2, "A1", "B1"), (3, "A9", "FALSE"),
            (4, "\"a-b\"", "\"a-b\""), (5, "\"\u{e9}\"", "\"\u{e9}\"")] {
            let at = CellRef::new(index as u32, column);
            sheet.insert_cell(Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900).unwrap()
                .with_formula(Formula::from_file(&format!("{left}{operator}{right}"), at))).unwrap();
        }
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 24);
    let sheet = book.sheet("Data").unwrap();
    for (index, expected) in [false, true, true, false, true, false].into_iter().enumerate() {
        assert_eq!(sheet.scalar(CellRef::new(index as u32, 2)).as_bool(), Some(expected));
    }
    for (index, expected) in [true, false, false, false, true, true].into_iter().enumerate() {
        for column in [3, 4, 5] {
            assert_eq!(sheet.scalar(CellRef::new(index as u32, column)).as_bool(), Some(expected));
        }
    }
    book.sheet_mut("Data").unwrap().set_cell(CellRef::new(0, 0), 3.0).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 6);
    for (index, expected) in [false, true, false, true, false, true].into_iter().enumerate() {
        assert_eq!(book.sheet("Data").unwrap().scalar(CellRef::new(index as u32, 2)).as_bool(), Some(expected));
    }
    book.sheet_mut("Data").unwrap().set_cell(CellRef::new(8, 0), true).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 6);
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}
