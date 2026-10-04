//! `rust/src/excel/workbook.rs`: `Workbook` - lazy sheet access, package ownership and metadata transfer, atomic structural edits, and saving with every untouched part carried over.

use std::sync::Arc;

use yggdryl::excel::{
    Cell, CellRef, DateSystem, NumberFormat, Paste, Sheet, SheetKey, SheetKind, SheetState,
    StyleId, Workbook,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::local::LocalFolder;
use yggdryl::zip::ZipArchive;
use yggdryl::{Codec, Error, Scalar, TimeUnit, Timezone};

use crate::excel_package::{
    NS, R_NS, REPORT_SHARED, Related, book, content_types, costs_cut_part, cut_table_part, member,
    normalized, package, package_coded, rich_package, rich_parts, root_relationships,
    shared_strings, sheet, sheets_book, stationary_cut_table, styles, table_formula_part,
    table_member_map, with_cut_tables, workbook, workbook_relationships, worksheet,
};

const WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const CHARTSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet";
const STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";

/// The cells of `sheet` with the style each holds set aside.
fn unstyled(sheet: &Sheet) -> Vec<Cell> {
    sheet
        .cells()
        .map(|cell| cell.clone().with_style(StyleId::DEFAULT))
        .collect()
}

/// Literal arithmetic exercises the one formula evaluation path and keeps
/// each cell's formula while replacing only its cached result.
#[test]
fn calculate_all_constant_arithmetic() {
    use yggdryl::excel::Formula;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    for (reference, expression) in [("B1", "1+2*3"), ("B2", "(1+2)*3")] {
        let reference = at(reference);
        sheet
            .insert_cell(
                Cell::from_scalar(reference, Scalar::Null, DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_entry(expression, reference).unwrap()),
            )
            .unwrap();
    }
    assert_eq!(
        workbook.sheet("Data").unwrap().scalar(at("B1")),
        Scalar::Null
    );
    assert_eq!(
        workbook.sheet("Data").unwrap().scalar(at("B2")),
        Scalar::Null
    );

    let calculated = workbook.calculate_all().unwrap();
    assert_eq!(calculated.evaluated, 2);
    assert_eq!(calculated.uncomputed, 0);
    assert_eq!(calculated.circular_count, 0);
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(sheet.scalar(at("B1")), Scalar::from(7.0));
    assert_eq!(sheet.scalar(at("B2")), Scalar::from(9.0));
    for (reference, expression) in [("B1", "1+2*3"), ("B2", "(1+2)*3")] {
        let reference = at(reference);
        assert_eq!(
            sheet
                .cell(reference)
                .unwrap()
                .formula()
                .unwrap()
                .at(reference)
                .to_string(),
            expression
        );
    }
}

fn at(reference: &str) -> CellRef {
    reference.parse().unwrap()
}

/// A worksheet part holding the number `value` at A1.
fn number_sheet(value: u32) -> String {
    worksheet(&format!(
        "<row r=\"1\"><c r=\"A1\"><v>{value}</v></c></row>"
    ))
}

/// A workbook relationships part stating `entries`, each (id, type, target).
fn relationships(entries: &[(&str, &str, &str)]) -> String {
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in entries {
        text.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    text.push_str("</Relationships>");
    text
}

/// A workbook part around `inner`: its `workbookPr`, if any, and `<sheets>`.
fn workbook_with(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\">{inner}</workbook>"
    )
}

/// `Sheet1`..`Sheet3` holding 1, 2 and 3 at A1, with no shared strings and
/// no styles, plus the `extra` members.
fn three_sheets_with(extra: &[(&str, &str)]) -> Vec<u8> {
    let types = content_types(3, false, false);
    let root = root_relationships();
    let rels = workbook_relationships(3, false, false);
    let book = workbook(&["Sheet1", "Sheet2", "Sheet3"], false);
    let (one, two, three) = (number_sheet(1), number_sheet(2), number_sheet(3));
    let mut parts: Vec<(&str, &str)> = vec![
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &one),
        ("xl/worksheets/sheet2.xml", &two),
        ("xl/worksheets/sheet3.xml", &three),
    ];
    parts.extend_from_slice(extra);
    package(&parts)
}

fn three_sheets() -> Vec<u8> {
    three_sheets_with(&[])
}

/// A one-sheet package whose workbook part is `book`.
fn one_sheet_under(book: &str) -> Vec<u8> {
    let types = content_types(1, false, false);
    let root = root_relationships();
    let rels = workbook_relationships(1, false, false);
    let sheet = number_sheet(1);
    package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book),
        ("xl/_rels/workbook.xml.rels", rels.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
    ])
}

/// `Data` (7 at A1) then the chart sheet `Chart`.
fn with_chartsheet() -> Vec<u8> {
    let types = content_types(1, false, false);
    let root = root_relationships();
    let book = workbook(&["Data", "Chart"], false);
    let rels = relationships(&[
        ("rId1", WORKSHEET, "worksheets/sheet1.xml"),
        ("rId2", CHARTSHEET, "chartsheets/sheet1.xml"),
    ]);
    let data = number_sheet(7);
    let chart = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <chartsheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews></chartsheet>"
    );
    package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &data),
        ("xl/chartsheets/sheet1.xml", &chart),
    ])
}

fn archive(bytes: &[u8]) -> Arc<ZipArchive> {
    Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))))
}

fn member_names(bytes: &[u8]) -> Vec<String> {
    archive(bytes)
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect()
}

fn member_text(bytes: &[u8], name: &str) -> String {
    String::from_utf8(archive(bytes).read_member(name).unwrap()).unwrap()
}

#[test]
fn a_new_workbook_and_one_opened_over_nothing_hold_no_sheet_under_the_1900_date_system() {
    let built = Workbook::new();
    assert_eq!(built.len(), 0);
    assert!(built.is_empty());
    assert!(built.sheet_names().is_empty());
    assert_eq!(built.date_system(), DateSystem::Year1900);
    assert_eq!(built.handle_reads(), 0);
    assert_eq!(Workbook::default().date_system(), DateSystem::Year1900);
    assert!(Workbook::default().is_empty());

    // A handle holding nothing is an empty workbook, not a malformed package.
    for opened in [
        Workbook::open(Buffer::new()).unwrap(),
        Workbook::from_bytes(Vec::new()).unwrap(),
    ] {
        assert!(opened.is_empty());
        assert!(opened.sheet_names().is_empty());
        assert_eq!(opened.date_system(), DateSystem::Year1900);
        assert_eq!(opened.handle_reads(), 0);
    }
}

#[test]
fn biff_bytes_are_refused_as_an_unsupported_workbook() {
    let mut biff = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    biff.extend([0_u8; 504]);
    let refusal = Workbook::from_bytes(biff).unwrap_err();
    assert!(
        matches!(
            &refusal,
            Error::Unsupported {
                operation: "reading a BIFF or encrypted workbook",
                ..
            }
        ),
        "{refusal:?}"
    );
    assert!(
        refusal
            .to_string()
            .ends_with("does not support reading a BIFF or encrypted workbook"),
        "{refusal}"
    );
}

#[test]
fn bytes_that_are_not_a_zip_package_are_a_codec_refusal_naming_xlsx() {
    let refusal = Workbook::from_bytes(b"not a spreadsheet".to_vec()).unwrap_err();
    assert!(
        matches!(
            &refusal,
            Error::Codec {
                format: "xlsx",
                position: 0,
                ..
            }
        ),
        "{refusal:?}"
    );
    assert!(
        refusal.to_string().starts_with(
            "invalid xlsx data at byte 0: expected a ZIP package \
             (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), got: "
        ),
        "{refusal}"
    );
}

#[test]
fn a_package_without_its_workbook_part_is_refused_listing_the_members() {
    let types = content_types(0, false, false);
    let root = root_relationships();
    let bytes = package(&[("[Content_Types].xml", &types), ("_rels/.rels", &root)]);
    let refusal = Workbook::from_bytes(bytes).unwrap_err();
    assert!(
        matches!(&refusal, Error::InvalidRecord { .. }),
        "{refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        "invalid record value at xl/workbook.xml: expected the workbook part in the package, \
         got the members [[Content_Types].xml, _rels/.rels]"
    );
}

#[test]
fn the_workbook_part_is_where_the_root_relationships_point_else_the_conventional_part() {
    // The office document named by `_rels/.rels`, with its relationships
    // beside it under `_rels/`.
    let root = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/book.xml\"/>\
                </Relationships>";
    let book = workbook(&["Only"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = number_sheet(5);
    let bytes = package(&[
        ("_rels/.rels", root),
        ("xl/book.xml", &book),
        ("xl/_rels/book.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet_names(), ["Only"]);
    assert_eq!(
        opened.sheet("Only").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );

    // Written back, the package keeps its workbook part where it was.
    opened
        .sheet_mut("Only")
        .unwrap()
        .set_cell(at("B1"), "more")
        .unwrap();
    let written = opened.into_bytes().unwrap();
    let names = member_names(&written);
    assert!(names.iter().any(|name| name == "xl/book.xml"), "{names:?}");
    assert!(
        !names.iter().any(|name| name == "xl/workbook.xml"),
        "{names:?}"
    );
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(
        reopened.sheet("Only").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );
    assert_eq!(
        reopened.sheet("Only").unwrap().scalar(at("B1")),
        Scalar::from("more")
    );

    // A package stating no root relationships reads `xl/workbook.xml`.
    let book = workbook(&["Plain"], false);
    let rels = workbook_relationships(1, false, false);
    let sheet = number_sheet(9);
    let bare = package(&[
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let plain = Workbook::from_bytes(bare).unwrap();
    assert_eq!(plain.sheet_names(), ["Plain"]);
    assert_eq!(
        plain.sheet("Plain").unwrap().scalar(at("A1")),
        Scalar::from(9.0)
    );
}

#[test]
fn sheets_are_listed_in_workbook_order_each_reading_the_part_its_relationship_names() {
    let types = content_types(3, false, false);
    let root = root_relationships();
    let book = workbook(&["Zeta", "Alpha", "Mid"], false);
    let rels = relationships(&[
        ("rId1", WORKSHEET, "worksheets/sheet3.xml"),
        ("rId2", WORKSHEET, "worksheets/sheet1.xml"),
        ("rId3", WORKSHEET, "/xl/worksheets/sheet2.xml"),
    ]);
    let (one, two, three) = (number_sheet(1), number_sheet(2), number_sheet(3));
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &one),
        ("xl/worksheets/sheet2.xml", &two),
        ("xl/worksheets/sheet3.xml", &three),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.len(), 3);
    assert!(!opened.is_empty());
    assert_eq!(opened.sheet_names(), ["Zeta", "Alpha", "Mid"]);
    assert_eq!(
        opened.sheet("Zeta").unwrap().scalar(at("A1")),
        Scalar::from(3.0)
    );
    assert_eq!(
        opened.sheet("Alpha").unwrap().scalar(at("A1")),
        Scalar::from(1.0)
    );
    // An absolute target names the member from the package root.
    assert_eq!(
        opened.sheet("Mid").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
}

#[test]
fn a_sheet_whose_relationship_names_no_part_is_refused_at_open() {
    let book = workbook_with(
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
         <sheet name=\"Ghost\" sheetId=\"2\" r:id=\"rId9\"/></sheets>",
    );
    let refusal = Workbook::from_bytes(one_sheet_under(&book)).unwrap_err();
    assert!(
        matches!(&refusal, Error::InvalidRecord { .. }),
        "{refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        "invalid record value at xl/workbook.xml: expected the relationship rId9 of sheet \"Ghost\" \
         to name a part in xl/_rels/workbook.xml.rels, got [rId1]"
    );
}

#[test]
fn sheets_are_found_by_name_without_case_and_by_tab_position() {
    let opened = Workbook::from_bytes(three_sheets()).unwrap();
    assert_eq!(opened.sheet("sheet2").unwrap().name(), "Sheet2");
    assert_eq!(
        opened.sheet("SHEET2").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
    let found = opened
        .get_sheet("sHeEt3")
        .unwrap()
        .expect("the sheet is there");
    assert_eq!(found.name(), "Sheet3");
    assert_eq!(found.scalar(at("A1")), Scalar::from(3.0));
    let first = opened.sheet_at(0).unwrap().expect("a first tab");
    assert_eq!(first.name(), "Sheet1");
    assert_eq!(opened.sheet_at(2).unwrap().unwrap().name(), "Sheet3");
    assert_eq!(opened.sheet_kind("SHEET1"), Some(SheetKind::Worksheet));
}

#[test]
fn an_absent_sheet_is_refused_listing_the_sheets_while_the_get_forms_answer_none() {
    let opened = Workbook::from_bytes(three_sheets()).unwrap();
    let reads = opened.handle_reads();
    let refusal = opened.sheet("Nope").unwrap_err();
    assert!(
        matches!(
            &refusal,
            Error::Absent {
                expected: "worksheet",
                ..
            }
        ),
        "{refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        "expected a worksheet at \"Nope (the workbook holds [Sheet1, Sheet2, Sheet3])\", got nothing"
    );
    assert!(opened.get_sheet("Nope").unwrap().is_none());
    assert!(opened.sheet_at(3).unwrap().is_none());
    assert!(opened.sheet_at(usize::MAX).unwrap().is_none());
    assert_eq!(opened.sheet_kind("Nope"), None);
    // Looking for what is not there parses nothing.
    assert_eq!(opened.handle_reads(), reads);
}

#[test]
fn a_chartsheet_is_listed_with_its_kind_and_refused_where_a_worksheet_is_asked_for() {
    let mut opened = Workbook::from_bytes(with_chartsheet()).unwrap();
    assert_eq!(opened.sheet_names(), ["Data", "Chart"]);
    assert_eq!(opened.sheet_kind("Data"), Some(SheetKind::Worksheet));
    assert_eq!(opened.sheet_kind("chart"), Some(SheetKind::Chartsheet));
    assert_eq!(SheetKind::Chartsheet.as_str(), "chartsheet");
    assert_eq!(SheetKind::Worksheet.as_str(), "worksheet");
    assert_eq!(SheetKind::Dialogsheet.as_str(), "dialogsheet");

    let expected = "invalid record value at $.Chart: expected a worksheet, got the chartsheet `Chart`, \
                    which holds no cells";
    assert_eq!(opened.sheet("Chart").unwrap_err().to_string(), expected);
    assert_eq!(opened.get_sheet("CHART").unwrap_err().to_string(), expected);
    assert_eq!(opened.sheet_at(1).unwrap_err().to_string(), expected);
    assert_eq!(opened.sheet_mut("Chart").unwrap_err().to_string(), expected);
    // A chart sheet holds no cells for a sheet to replace.
    let refusal = opened
        .insert_sheet(Sheet::new("chart").unwrap())
        .unwrap_err();
    assert_eq!(refusal.to_string(), expected);
    assert_eq!(opened.len(), 2);

    // Removed, it answers no sheet.
    assert!(opened.remove_sheet("Chart").unwrap().is_none());
    assert_eq!(opened.sheet_names(), ["Data"]);
}

#[test]
fn a_chartsheet_and_its_part_survive_a_write() {
    let original = with_chartsheet();
    let mut opened = Workbook::from_bytes(original.clone()).unwrap();
    opened
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("A2"), 8.5)
        .unwrap();
    let written = opened.into_bytes().unwrap();
    assert_eq!(
        member_text(&written, "xl/chartsheets/sheet1.xml"),
        member_text(&original, "xl/chartsheets/sheet1.xml")
    );
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["Data", "Chart"]);
    assert_eq!(reopened.sheet_kind("Chart"), Some(SheetKind::Chartsheet));
    assert_eq!(
        reopened.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
    assert_eq!(
        reopened.sheet("Data").unwrap().scalar(at("A2")),
        Scalar::from(8.5)
    );
}

#[test]
fn opening_reads_only_the_package_documents_and_a_sheet_is_parsed_once_on_first_access() {
    let opened = Workbook::from_bytes(three_sheets()).unwrap();
    // The mount (size and tail) and three documents - `_rels/.rels`, the
    // workbook relationships and the workbook part - at two reads each.
    assert_eq!(opened.handle_reads(), 8);
    opened.sheet("Sheet2").unwrap();
    // One sheet part: its two reads, and no other sheet's.
    assert_eq!(opened.handle_reads(), 10);
    // Held until drop: asked again, nothing is read.
    opened.sheet("sheet2").unwrap();
    opened.get_sheet("SHEET2").unwrap();
    opened.sheet_at(1).unwrap();
    assert_eq!(opened.handle_reads(), 10);
    opened.sheet_at(2).unwrap();
    assert_eq!(opened.handle_reads(), 12);
}

#[test]
fn shared_strings_and_styles_are_read_once_on_the_first_sheet_that_needs_them() {
    let types = content_types(2, true, true);
    let root = root_relationships();
    let book = workbook(&["Sheet1", "Sheet2"], false);
    let rels = workbook_relationships(2, true, true);
    let one = worksheet(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" s=\"1\"><v>45292</v></c></row>",
    );
    let two = worksheet("<row r=\"1\"><c r=\"A1\" t=\"s\"><v>1</v></c></row>");
    let sst = shared_strings(&["alpha", "beta"]);
    let css = styles(&[], &[0, 14]);
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &one),
        ("xl/worksheets/sheet2.xml", &two),
        ("xl/sharedStrings.xml", &sst),
        ("xl/styles.xml", &css),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.handle_reads(), 8);
    let first = opened.sheet("Sheet1").unwrap();
    // The sheet part, the shared strings and the styles, two reads each.
    assert_eq!(opened.handle_reads(), 14);
    assert_eq!(first.scalar(at("A1")), Scalar::from("alpha"));
    let day = first.cell(at("B1")).unwrap();
    assert_eq!(day.format(), NumberFormat::Date);
    assert_eq!(day.value().into_json().unwrap(), "\"2024-01-01\"");
    // The second sheet reads its own part and nothing shared again.
    let second = opened.sheet("Sheet2").unwrap();
    assert_eq!(opened.handle_reads(), 16);
    assert_eq!(second.scalar(at("A1")), Scalar::from("beta"));
}

#[test]
fn sheet_mut_parses_the_part_then_mutates_the_held_sheet() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    let sheet = opened.sheet_mut("SHEET1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(1.0));
    sheet.set_cell(at("B1"), "added").unwrap();
    assert_eq!(opened.handle_reads(), 10);
    // The same held sheet answers the shared borrow.
    let held = opened.sheet("Sheet1").unwrap();
    assert_eq!(held.scalar(at("A1")), Scalar::from(1.0));
    assert_eq!(held.scalar(at("B1")), Scalar::from("added"));

    let reopened = Workbook::from_bytes(opened.into_bytes().unwrap()).unwrap();
    let sheet = reopened.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(1.0));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("added"));
}

#[test]
fn add_sheet_appends_an_empty_worksheet_under_the_workbook_date_system_to_fill() {
    let mut built = Workbook::new();
    built.set_date_system(DateSystem::Year1904);
    let trades = built.add_sheet("Trades").unwrap();
    assert_eq!(trades.name(), "Trades");
    assert!(trades.is_empty());
    assert_eq!(trades.state(), SheetState::Visible);
    assert_eq!(trades.date_system(), DateSystem::Year1904);
    trades.set_cell(at("A1"), "symbol").unwrap();
    built.add_sheet("Quotes").unwrap();
    assert_eq!(built.sheet_names(), ["Trades", "Quotes"]);
    assert_eq!(built.len(), 2);
    assert_eq!(built.sheet_kind("quotes"), Some(SheetKind::Worksheet));
    assert_eq!(
        built.sheet("trades").unwrap().scalar(at("A1")),
        Scalar::from("symbol")
    );
    assert!(built.sheet("Quotes").unwrap().is_empty());
}

#[test]
fn add_sheet_refuses_a_name_another_sheet_has_compared_without_case() {
    let mut built = Workbook::new();
    built
        .add_sheet("Trades")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    let refusal = built.add_sheet("TRADES").unwrap_err();
    match &refusal {
        Error::Conflict {
            expected,
            actual,
            path,
        } => {
            assert_eq!(*expected, "sheet name no other sheet has");
            assert_eq!(*actual, "sheet of that name, compared without case");
            assert_eq!(path.as_str(), "TRADES");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    // Nothing changed: the sheet and its cell are still the one there.
    assert_eq!(built.sheet_names(), ["Trades"]);
    assert_eq!(
        built.sheet("Trades").unwrap().scalar(at("A1")),
        Scalar::from(1.0)
    );
}

#[test]
fn add_sheet_refuses_a_name_excel_refuses_and_adds_nothing() {
    let mut built = Workbook::new();
    for (name, reason) in [
        ("", "expected a sheet name, got the empty text"),
        (
            "a/b",
            "expected a sheet name without any of \\ / ? * [ ] :, got '/' in \"a/b\"",
        ),
        (
            "history",
            "expected a sheet name other than the reserved `History`",
        ),
        (
            "'quoted'",
            "expected a sheet name that neither opens nor closes with an apostrophe, got \"'quoted'\"",
        ),
        (
            "abcdefghijklmnopqrstuvwxyzABCDEF",
            "expected a sheet name of at most 31 characters, got 32 in \"abcdefghijklmnopqrstuvwxyzABCDEF\"",
        ),
    ] {
        let refusal = built.add_sheet(name).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            format!("invalid record value at $.sheet: {reason}"),
            "{name:?}"
        );
    }
    assert!(built.is_empty());
    // Thirty-one characters is the limit, not past it.
    built.add_sheet("abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
    assert_eq!(built.len(), 1);
}

#[test]
fn insert_sheet_replaces_a_same_named_sheet_in_place_or_appends_a_new_one() {
    let mut built = Workbook::new();
    built.add_sheet("First").unwrap();
    built
        .add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    built.add_sheet("Last").unwrap();

    let mut replacement = Sheet::new("DATA").unwrap();
    replacement.set_cell(at("A1"), 2.0).unwrap();
    let previous = built
        .insert_sheet(replacement)
        .unwrap()
        .expect("the sheet it replaced");
    assert_eq!(previous.name(), "Data");
    assert_eq!(previous.scalar(at("A1")), Scalar::from(1.0));
    // The tab keeps its place and takes the inserted sheet's name.
    assert_eq!(built.sheet_names(), ["First", "DATA", "Last"]);
    assert_eq!(
        built.sheet("data").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );

    // A new name goes after the last tab, under the workbook's date system.
    built.set_date_system(DateSystem::Year1904);
    assert!(
        built
            .insert_sheet(Sheet::new("Extra").unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(built.sheet_names(), ["First", "DATA", "Last", "Extra"]);
    assert_eq!(
        built.sheet("Extra").unwrap().date_system(),
        DateSystem::Year1904
    );
}

#[test]
fn remove_sheet_answers_the_sheet_parsed_and_none_for_a_name_no_sheet_has() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    let removed = opened
        .remove_sheet("SHEET2")
        .unwrap()
        .expect("the removed sheet");
    assert_eq!(removed.name(), "Sheet2");
    assert_eq!(removed.scalar(at("A1")), Scalar::from(2.0));
    assert_eq!(opened.sheet_names(), ["Sheet1", "Sheet3"]);
    assert_eq!(opened.len(), 2);
    assert!(opened.remove_sheet("Sheet2").unwrap().is_none());
    assert!(opened.get_sheet("Sheet2").unwrap().is_none());
    assert_eq!(opened.len(), 2);
}

#[test]
fn rename_sheet_keeps_the_place_and_the_part_and_refuses_a_taken_name() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    opened.rename_sheet("sheet2", "Renamed").unwrap();
    assert_eq!(opened.sheet_names(), ["Sheet1", "Renamed", "Sheet3"]);

    let taken = opened.rename_sheet("Renamed", "SHEET1").unwrap_err();
    assert!(
        matches!(&taken, Error::Conflict { path, .. } if path == "SHEET1"),
        "{taken:?}"
    );
    let absent = opened.rename_sheet("Nope", "Other").unwrap_err();
    assert!(matches!(&absent, Error::Absent { .. }), "{absent:?}");
    let invalid = opened.rename_sheet("Renamed", "a:b").unwrap_err();
    assert_eq!(
        invalid.to_string(),
        "invalid record value at $.sheet: expected a sheet name without any of \\ / ? * [ ] :, got ':' in \"a:b\""
    );
    // A sheet takes its own name in another case.
    opened.rename_sheet("sheet1", "SHEET1").unwrap();
    assert_eq!(opened.sheet_names(), ["SHEET1", "Renamed", "Sheet3"]);

    let written = opened.into_bytes().unwrap();
    let names = member_names(&written);
    assert!(
        !names.iter().any(|name| name == "xl/worksheets/sheet4.xml"),
        "{names:?}"
    );
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["SHEET1", "Renamed", "Sheet3"]);
    assert_eq!(
        reopened.sheet("Renamed").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
}

#[test]
fn a_workbook_with_no_visible_worksheet_is_not_written() {
    let expected = "invalid record value at $: expected at least one visible worksheet, which Excel \
                    requires of a workbook";
    assert_eq!(
        Workbook::new().into_bytes().unwrap_err().to_string(),
        expected
    );

    let mut hidden = Workbook::new();
    hidden
        .insert_sheet(Sheet::new("Secret").unwrap().with_state(SheetState::Hidden))
        .unwrap();
    assert_eq!(hidden.into_bytes().unwrap_err().to_string(), expected);

    hidden.add_sheet("Shown").unwrap();
    assert!(hidden.into_bytes().is_ok());
}

#[test]
fn hidden_and_very_hidden_states_are_read_from_the_workbook_part_and_written_back() {
    let types = content_types(3, false, false);
    let root = root_relationships();
    let book = workbook_with(
        "<sheets><sheet name=\"Shown\" sheetId=\"1\" r:id=\"rId1\"/>\
         <sheet name=\"Tucked\" sheetId=\"2\" r:id=\"rId2\" state=\"hidden\"/>\
         <sheet name=\"Locked\" sheetId=\"3\" r:id=\"rId3\" state=\"veryHidden\"/></sheets>",
    );
    let rels = workbook_relationships(3, false, false);
    let (one, two, three) = (number_sheet(1), number_sheet(2), number_sheet(3));
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &one),
        ("xl/worksheets/sheet2.xml", &two),
        ("xl/worksheets/sheet3.xml", &three),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(opened.sheet("Shown").unwrap().state(), SheetState::Visible);
    assert_eq!(opened.sheet("Tucked").unwrap().state(), SheetState::Hidden);
    assert_eq!(
        opened.sheet("Locked").unwrap().state(),
        SheetState::VeryHidden
    );

    // A sheet added changes the list, so the workbook part is spliced.
    opened.add_sheet("Added").unwrap();
    let written = opened.into_bytes().unwrap();
    let part = member_text(&written, "xl/workbook.xml");
    // Every tab keeps its number and its relationship; the added one takes
    // the next of each.
    assert!(
        part.contains("<sheet name=\"Tucked\" sheetId=\"2\" r:id=\"rId2\" state=\"hidden\"/>"),
        "{part}"
    );
    assert!(
        part.contains("<sheet name=\"Locked\" sheetId=\"3\" r:id=\"rId3\" state=\"veryHidden\"/>"),
        "{part}"
    );
    assert!(
        part.contains("<sheet name=\"Added\" sheetId=\"4\" r:id=\"rId4\"/>"),
        "{part}"
    );
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(
        reopened.sheet("Tucked").unwrap().state(),
        SheetState::Hidden
    );
    assert_eq!(
        reopened.sheet("Locked").unwrap().state(),
        SheetState::VeryHidden
    );
    assert_eq!(
        reopened.sheet("Added").unwrap().state(),
        SheetState::Visible
    );

    // A built workbook writes the state its sheet was inserted with.
    let mut built = Workbook::new();
    built.add_sheet("Front").unwrap();
    built
        .insert_sheet(
            Sheet::new("Back")
                .unwrap()
                .with_state(SheetState::VeryHidden),
        )
        .unwrap();
    let reopened = Workbook::from_bytes(built.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("Back").unwrap().state(),
        SheetState::VeryHidden
    );

    // A state the schema does not list is refused naming the part.
    let refusal = Workbook::from_bytes(one_sheet_under(&workbook_with(
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\" state=\"gone\"/></sheets>",
    )))
    .unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "invalid record value at xl/workbook.xml: invalid sheet state expression at byte 0: \
         expected visible, hidden or veryHidden for a sheet's state, got \"gone\""
    );
}

#[test]
fn the_date1904_attribute_reads_one_true_zero_or_false_and_refuses_anything_else() {
    let sheets = "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets>";
    for (value, system) in [
        ("1", DateSystem::Year1904),
        ("true", DateSystem::Year1904),
        ("0", DateSystem::Year1900),
        ("false", DateSystem::Year1900),
    ] {
        let book = workbook_with(&format!("<workbookPr date1904=\"{value}\"/>{sheets}"));
        let opened = Workbook::from_bytes(one_sheet_under(&book)).unwrap();
        assert_eq!(opened.date_system(), system, "{value}");
    }
    let book = workbook_with(&format!("<workbookPr date1904=\"yes\"/>{sheets}"));
    let refusal = Workbook::from_bytes(one_sheet_under(&book)).unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "invalid record value at xl/workbook.xml#workbookPr/@date1904: expected 1, true, 0 or false \
         for date1904, got \"yes\""
    );
}

#[test]
fn phantom_temporal_serial_displays_1900_february_29() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>60</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let workbook = Workbook::from_bytes(source).unwrap();
    let phantom = workbook.display_text("Sheet1", at("A1")).unwrap().unwrap();
    assert!(phantom.text.contains("2/29"), "{phantom:?}");
}

#[test]
fn the_1904_date_system_is_written_and_a_date_cell_round_trips_under_it() {
    let mut built = Workbook::new();
    built.set_date_system(DateSystem::Year1904);
    assert_eq!(built.date_system(), DateSystem::Year1904);
    // 2024-01-01, which is 1904 serial 43830 (1900 serial 45292 less 1462).
    built
        .add_sheet("Days")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_723))
        .unwrap();
    let written = built.into_bytes().unwrap();
    assert!(member_text(&written, "xl/workbook.xml").contains("<workbookPr date1904=\"1\"/>"));
    let part = member_text(&written, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<c r=\"A1\" s=\"1\"><v>43830</v></c>"),
        "{part}"
    );

    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.date_system(), DateSystem::Year1904);
    let sheet = reopened.sheet("Days").unwrap();
    assert_eq!(sheet.date_system(), DateSystem::Year1904);
    let day = sheet.cell(at("A1")).unwrap();
    assert_eq!(day.format(), NumberFormat::Date);
    assert_eq!(day.value(), &Scalar::date32(19_723));
}

#[test]
fn a_built_workbook_round_trips_every_sheet_cell_by_cell() {
    let mut built = Workbook::new();
    let prices = built.add_sheet("Prices").unwrap();
    prices.set_cell(at("A1"), "symbol").unwrap();
    prices.set_cell(at("B1"), "price").unwrap();
    prices.set_cell(at("A2"), "AAPL").unwrap();
    prices.set_cell(at("B2"), 187.25).unwrap();
    prices.set_cell(at("C2"), true).unwrap();
    prices.set_cell(at("D2"), Scalar::date32(19_723)).unwrap();
    prices
        .set_cell(
            at("E2"),
            Scalar::datetime64(1_704_110_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        )
        .unwrap();
    let notes = built.add_sheet("Notes").unwrap();
    notes.set_cell(at("A1"), "a & b <c>").unwrap();
    notes.set_cell(at("B3"), -0.5).unwrap();
    notes.set_cell(at("C3"), false).unwrap();

    let reopened = Workbook::from_bytes(built.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.sheet_names(), built.sheet_names());
    let styles = reopened.style_sheet().unwrap();
    for name in built.sheet_names() {
        assert_eq!(
            unstyled(reopened.sheet(name).unwrap()),
            unstyled(built.sheet(name).unwrap()),
            "{name}"
        );
        // A temporal cell built in memory holds the default style, which
        // does not read as its format, so it is written under the style the
        // save interned for the format; every other cell under the default.
        for cell in reopened.sheet(name).unwrap().cells() {
            let code = &styles.style(cell.style()).unwrap().number_format;
            match cell.format().code() {
                Some(expected) => assert_eq!(code, expected, "{}", cell.reference()),
                None => assert_eq!(cell.style(), StyleId::DEFAULT, "{}", cell.reference()),
            }
        }
    }
    assert_eq!(
        reopened
            .sheet("Prices")
            .unwrap()
            .scalar(at("E2"))
            .into_json()
            .unwrap(),
        "\"2024-01-01T12:00:00.000\""
    );
}

#[test]
fn an_opened_package_carries_its_untouched_sheets_and_parts_over_unparsed() {
    let app = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties><Application>Test</Application></Properties>";
    let theme = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><a:theme xmlns:a=\"urn:theme\" name=\"Office\"/>";
    let original = three_sheets_with(&[("docProps/app.xml", app), ("xl/theme/theme1.xml", theme)]);
    let mut opened = Workbook::from_bytes(original).unwrap();
    opened
        .sheet_mut("Sheet2")
        .unwrap()
        .set_cell(at("B1"), "edited")
        .unwrap();
    // Only the sheet edited was parsed.
    assert_eq!(opened.handle_reads(), 10);

    let written = opened.into_bytes().unwrap();
    assert_eq!(
        member_text(&written, "xl/worksheets/sheet1.xml"),
        number_sheet(1)
    );
    assert_eq!(
        member_text(&written, "xl/worksheets/sheet3.xml"),
        number_sheet(3)
    );
    assert_eq!(member_text(&written, "docProps/app.xml"), app);
    assert_eq!(member_text(&written, "xl/theme/theme1.xml"), theme);
    assert_ne!(
        member_text(&written, "xl/worksheets/sheet2.xml"),
        number_sheet(2)
    );

    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["Sheet1", "Sheet2", "Sheet3"]);
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::from(1.0)
    );
    assert_eq!(
        reopened.sheet("Sheet2").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
    assert_eq!(
        reopened.sheet("Sheet2").unwrap().scalar(at("B1")),
        Scalar::from("edited")
    );
    assert_eq!(
        reopened.sheet("Sheet3").unwrap().scalar(at("A1")),
        Scalar::from(3.0)
    );
}

#[test]
fn a_rewritten_sheet_drops_the_stale_calculation_chain() {
    let chain = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                 <calcChain xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><c r=\"A1\" i=\"1\"/></calcChain>";
    let original = three_sheets_with(&[("xl/calcChain.xml", chain)]);
    let mut opened = Workbook::from_bytes(original).unwrap();
    opened
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A1"), 10.0)
        .unwrap();
    let written = opened.into_bytes().unwrap();
    let names = member_names(&written);
    assert!(
        !names.iter().any(|name| name == "xl/calcChain.xml"),
        "{names:?}"
    );
    assert!(
        names.iter().any(|name| name == "xl/worksheets/sheet3.xml"),
        "{names:?}"
    );
}

#[test]
fn a_sheet_added_to_an_opened_package_takes_a_part_number_past_the_existing_ones() {
    // Two sheets at parts one and five: the next part is six, not three.
    let types = content_types(1, false, false);
    let root = root_relationships();
    let book = workbook(&["First", "Second"], false);
    let rels = relationships(&[
        ("rId1", WORKSHEET, "worksheets/sheet1.xml"),
        ("rId2", WORKSHEET, "worksheets/sheet5.xml"),
    ]);
    let (one, five) = (number_sheet(1), number_sheet(5));
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &one),
        ("xl/worksheets/sheet5.xml", &five),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    opened
        .add_sheet("Third")
        .unwrap()
        .set_cell(at("A1"), "x")
        .unwrap();
    let written = opened.into_bytes().unwrap();

    let names = member_names(&written);
    assert!(
        names.iter().any(|name| name == "xl/worksheets/sheet6.xml"),
        "{names:?}"
    );
    let types = member_text(&written, "[Content_Types].xml");
    assert!(
        types.contains(&format!(
            "<Override PartName=\"/xl/worksheets/sheet6.xml\" ContentType=\"{WORKSHEET_TYPE}\"/>"
        )),
        "{types}"
    );
    // The relationships already there stay; the new sheet's takes the id
    // past them, and the strings and styles it created the ones after.
    let rels = member_text(&written, "xl/_rels/workbook.xml.rels");
    assert!(
        rels.contains(&format!(
            "<Relationship Id=\"rId2\" Type=\"{WORKSHEET}\" Target=\"worksheets/sheet5.xml\"/>\
             <Relationship Id=\"rId3\" Type=\"{WORKSHEET}\" Target=\"worksheets/sheet6.xml\"/>"
        )),
        "{rels}"
    );
    let part = member_text(&written, "xl/workbook.xml");
    assert!(
        part.contains("<sheet name=\"Third\" sheetId=\"3\" r:id=\"rId3\"/>"),
        "{part}"
    );

    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["First", "Second", "Third"]);
    assert_eq!(
        reopened.sheet("Second").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );
    assert_eq!(
        reopened.sheet("Third").unwrap().scalar(at("A1")),
        Scalar::from("x")
    );
}

#[test]
fn a_sheet_removed_from_an_opened_package_leaves_the_workbook_documents() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    opened.remove_sheet("Sheet2").unwrap();
    let written = opened.into_bytes().unwrap();

    let types = member_text(&written, "[Content_Types].xml");
    assert!(!types.contains("/xl/worksheets/sheet2.xml"), "{types}");
    assert!(types.contains("/xl/worksheets/sheet3.xml"), "{types}");
    let rels = member_text(&written, "xl/_rels/workbook.xml.rels");
    assert!(!rels.contains("worksheets/sheet2.xml"), "{rels}");
    // The tabs left keep their numbers and relationships.
    let part = member_text(&written, "xl/workbook.xml");
    assert!(
        part.contains(
            "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
             <sheet name=\"Sheet3\" sheetId=\"3\" r:id=\"rId3\"/></sheets>"
        ),
        "{part}"
    );
    // The part the removed tab held goes with it.
    assert!(
        !member_names(&written)
            .iter()
            .any(|name| name == "xl/worksheets/sheet2.xml")
    );

    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["Sheet1", "Sheet3"]);
    assert_eq!(
        reopened.sheet("Sheet3").unwrap().scalar(at("A1")),
        Scalar::from(3.0)
    );
}

#[test]
fn a_workbook_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Workbook>();
    assert_send::<SheetKind>();
}

/// Each member of the package `bytes`: its name, how it is stored, its
/// checksum and stored size, and its content - in the order the package
/// lists them.
fn stored_members(bytes: &[u8]) -> Vec<(String, Codec, u32, u64, Vec<u8>)> {
    let archive = archive(bytes);
    archive
        .entries()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry.name().to_owned(),
                entry.codec().unwrap(),
                entry.crc32(),
                entry.compressed_size(),
                archive.read_member(entry.name()).unwrap(),
            )
        })
        .collect()
}

/// The encoded bytes of one member. The archive supplies its validated offset
/// and size; the ZIP local header supplies only the filename and extra lengths.
fn compressed_member<'a>(bytes: &'a [u8], name: &str) -> &'a [u8] {
    let source = archive(bytes);
    let entry = source
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name() == name)
        .unwrap();
    let header = usize::try_from(entry.header_offset()).unwrap();
    assert_eq!(&bytes[header..header + 4], b"PK\x03\x04");
    let filename = u16::from_le_bytes(bytes[header + 26..header + 28].try_into().unwrap());
    let extra = u16::from_le_bytes(bytes[header + 28..header + 30].try_into().unwrap());
    let start = header + 30 + usize::from(filename) + usize::from(extra);
    let end = start + usize::try_from(entry.compressed_size()).unwrap();
    &bytes[start..end]
}

/// A directory of its own for a test that writes files.
fn scratch(label: &str) -> std::path::PathBuf {
    let mut folder = LocalFolder::temporary().unwrap().path().unwrap();
    folder.push(format!("yggdryl-excel-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Every fixture this file opens, its members stored uncompressed, so a
/// member a save re-encoded instead of copying would come back deflated.
fn stored_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let app = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties><Application>Test</Application></Properties>";
    let chain = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                 <calcChain xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><c r=\"A1\" i=\"1\"/></calcChain>";
    let restored = |bytes: Vec<u8>| {
        let archive = archive(&bytes);
        let members: Vec<(String, String)> = archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    String::from_utf8(archive.read_member(entry.name()).unwrap()).unwrap(),
                )
            })
            .collect();
        let parts: Vec<(&str, &str)> = members
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        package_coded(&parts, Codec::Identity)
    };
    let types = content_types(2, true, true);
    let root = root_relationships();
    let book = workbook(&["Sheet1", "Sheet2"], true);
    let rels = workbook_relationships(2, true, true);
    let one = worksheet(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" s=\"1\"><v>45292</v></c></row>",
    );
    let two = worksheet("<row r=\"1\"><c r=\"A1\" t=\"s\"><v>1</v></c></row>");
    let sst = shared_strings(&["alpha", "beta"]);
    let css = styles(&[], &[0, 14]);
    vec![
        (
            "three sheets",
            restored(three_sheets_with(&[
                ("docProps/app.xml", app),
                ("xl/calcChain.xml", chain),
            ])),
        ),
        ("a chart sheet", restored(with_chartsheet())),
        (
            "strings, styles and 1904",
            restored(package(&[
                ("[Content_Types].xml", &types),
                ("_rels/.rels", &root),
                ("xl/workbook.xml", &book),
                ("xl/_rels/workbook.xml.rels", &rels),
                ("xl/worksheets/sheet1.xml", &one),
                ("xl/worksheets/sheet2.xml", &two),
                ("xl/sharedStrings.xml", &sst),
                ("xl/styles.xml", &css),
            ])),
        ),
    ]
}

#[test]
fn a_read_workbook_writes_back_what_it_read() {
    let mut fixtures = stored_fixtures();
    fixtures.push(("the rich package", rich_package()));
    for (label, original) in fixtures {
        let opened = Workbook::from_bytes(original.clone()).unwrap();
        // Every sheet parsed, nothing edited: nothing is dirty, so every
        // member is copied as it is stored - never inflated, never deflated.
        opened.parse_all().unwrap();
        assert!(!opened.is_dirty(), "{label}");
        let written = opened.into_bytes().unwrap();
        assert_eq!(
            stored_members(&written),
            stored_members(&original),
            "{label}"
        );
    }
}

/// Excel 16 saved this package with a chart, legacy comment, registered
/// pivot, table, metadata and extension payloads. The fixture was sanitized
/// only to remove local path and author identifiers.
const RICH_EXCEL: &[u8] = include_bytes!("fixtures/rich_excel.xlsx");

#[test]
fn an_excel_saved_rich_workbook_roundtrips_every_member_without_an_edit() {
    let opened = Workbook::from_bytes(RICH_EXCEL.to_vec()).unwrap();
    opened.parse_all().unwrap();
    assert!(!opened.is_dirty());
    let written = opened.into_bytes().unwrap();
    let original = stored_members(RICH_EXCEL);
    assert_eq!(stored_members(&written), original);
    for member in &original {
        assert_eq!(
            compressed_member(&written, &member.0),
            compressed_member(RICH_EXCEL, &member.0),
            "{} encoded bytes",
            member.0
        );
    }
}

#[test]
fn editing_an_excel_saved_rich_workbook_carries_opaque_features() {
    let mut opened = Workbook::from_bytes(RICH_EXCEL.to_vec()).unwrap();
    opened
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("C3"), 99.0)
        .unwrap();
    let written = opened.into_bytes().unwrap();
    if let Some(path) = std::env::var_os("YGGDRYL_EXCEL_RICH_EDITED_OUT") {
        std::fs::write(path, &written).unwrap();
    }
    let before = stored_members(RICH_EXCEL);
    let after = stored_members(&written);
    assert!(before.iter().any(|part| part.0 == "xl/calcChain.xml"));
    assert!(!after.iter().any(|part| part.0 == "xl/calcChain.xml"));
    let mut expected_names: Vec<&str> = before
        .iter()
        .map(|part| part.0.as_str())
        .filter(|name| *name != "xl/calcChain.xml")
        .collect();
    let mut actual_names: Vec<&str> = after.iter().map(|part| part.0.as_str()).collect();
    expected_names.sort_unstable();
    actual_names.sort_unstable();
    assert_eq!(actual_names, expected_names);
    for member in &before {
        let name = member.0.as_str();
        if name == "xl/calcChain.xml" {
            continue;
        }
        let current = after.iter().find(|part| part.0 == name).unwrap();
        if [
            "xl/worksheets/sheet1.xml",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
            "[Content_Types].xml",
        ]
        .contains(&name)
        {
            continue;
        }
        assert_eq!(current, member, "{name} decoded content and record facts");
        assert_eq!(
            compressed_member(&written, name),
            compressed_member(RICH_EXCEL, name),
            "{name} encoded bytes"
        );
    }
    // The rewritten sheet is identical after the contract's normalization
    // and the one cell edit: full feature attributes, references and payloads
    // are compared, rather than only counting their host elements.
    let original_data = member_text(RICH_EXCEL, "xl/worksheets/sheet1.xml");
    let old_cell = "<c r=\"C3\"><v>20</v></c>";
    let new_cell = "<c r=\"C3\"><v>99</v></c>";
    assert_eq!(original_data.matches(old_cell).count(), 1);
    let expected_data = original_data.replacen(old_cell, new_cell, 1);
    assert_eq!(
        normalized(&member_text(&written, "xl/worksheets/sheet1.xml"), &[]),
        normalized(&expected_data, &[]),
        "Data worksheet beyond C3"
    );
    // The three regenerated package owners have only the stated calculation
    // changes; all their unrelated extensions, types and relationships stay.
    for (name, removed, inserted) in [
        (
            "xl/workbook.xml",
            "<calcPr calcId=\"191029\"/>",
            "<calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/>",
        ),
        (
            "[Content_Types].xml",
            "<Override PartName=\"/xl/calcChain.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml\"/>",
            "",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<Relationship Id=\"rId9\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain\" Target=\"calcChain.xml\"/>",
            "",
        ),
    ] {
        let original = member_text(RICH_EXCEL, name);
        assert_eq!(original.matches(removed).count(), 1, "{name} fixture");
        let expected = original.replacen(removed, inserted, 1);
        assert_eq!(
            normalized(&member_text(&written, name), &[]),
            normalized(&expected, &[]),
            "{name} beyond the calculation change"
        );
    }
    let reopened = Workbook::from_bytes(written).unwrap();
    reopened.parse_all().unwrap();
    assert_eq!(
        reopened.sheet("Data").unwrap().scalar(at("C3")),
        Scalar::from(99.0)
    );
}

#[test]
fn an_edited_cell_changes_only_its_sheet_and_the_documents_a_written_sheet_asks_for() {
    let (_, original) = stored_fixtures().remove(0);
    let mut opened = Workbook::from_bytes(original.clone()).unwrap();
    opened
        .sheet_mut("Sheet2")
        .unwrap()
        .set_cell(at("B1"), "edited")
        .unwrap();
    let written = opened.into_bytes().unwrap();
    let before = stored_members(&original);
    let after = stored_members(&written);
    let find = |members: &[(String, Codec, u32, u64, Vec<u8>)], name: &str| {
        members
            .iter()
            .find(|member| member.0 == name)
            .cloned()
            .unwrap()
    };
    for name in [
        "_rels/.rels",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet3.xml",
        "docProps/app.xml",
    ] {
        assert_eq!(find(&after, name), find(&before, name), "{name}");
    }
    // The sheet written, the calculation chain gone from the package, its
    // relationship and its content type - and the workbook part asking
    // Excel to recalculate.
    assert_ne!(
        find(&after, "xl/worksheets/sheet2.xml").4,
        find(&before, "xl/worksheets/sheet2.xml").4
    );
    assert!(!after.iter().any(|member| member.0 == "xl/calcChain.xml"));
    assert!(
        member_text(&written, "xl/workbook.xml")
            .ends_with("<calcPr fullCalcOnLoad=\"1\"/></workbook>")
    );
    // The text written is the package's first, so the shared strings are
    // created; so are the styles, which a written sheet's cells index.
    let mut names: Vec<&str> = after.iter().map(|member| member.0.as_str()).collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = before
        .iter()
        .map(|member| member.0.as_str())
        .filter(|name| *name != "xl/calcChain.xml")
        .chain(["xl/sharedStrings.xml", "xl/styles.xml"])
        .collect();
    expected.sort_unstable();
    assert_eq!(names, expected);
}

/// Hold `written` - the rich package saved after the sheets `rewritten`
/// were written again from their cells - to the design's closed list: each
/// rewritten worksheet equal to what it read after normalization, `edited`
/// replacing a cell's spelling in it; the workbook part asking Excel to
/// recalculate; the calculation chain gone with its relationship and its
/// content type; every other member the member the package stored, byte for
/// byte and stored the same way.
fn assert_only_the_closed_list_changed(
    written: &[u8],
    rewritten: &[&str],
    edited: Option<(&str, &str, &str)>,
) {
    let original = rich_package();
    let before = stored_members(&original);
    let after = stored_members(written);
    let find = |members: &[(String, Codec, u32, u64, Vec<u8>)], name: &str| {
        members.iter().find(|member| member.0 == name).cloned()
    };
    let text = |member: Option<(String, Codec, u32, u64, Vec<u8>)>| {
        String::from_utf8(member.expect("the member").4).unwrap()
    };
    let parts = rich_parts();
    let sheet_of = |name: &str| -> &'static str {
        match name {
            "Data" => "xl/worksheets/sheet1.xml",
            "Report" => "xl/worksheets/sheet2.xml",
            _ => "xl/worksheets/sheet3.xml",
        }
    };
    let rewritten_parts: Vec<&str> = rewritten.iter().map(|name| sheet_of(name)).collect();
    for (name, stated) in &parts {
        let name = *name;
        if name == "xl/calcChain.xml" {
            assert!(find(&after, name).is_none(), "{name} is dropped");
            continue;
        }
        if name == "xl/printerSettings/printerSettings1.bin" {
            assert_eq!(find(&after, name), find(&before, name), "{name}");
            continue;
        }
        let stated = std::str::from_utf8(stated).unwrap();
        let expected = match name {
            "xl/workbook.xml" => stated.replace(
                "<calcPr calcId=\"191029\"/>",
                "<calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/>",
            ),
            "[Content_Types].xml" => stated.replace(
                "<Override PartName=\"/xl/calcChain.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml\"/>",
                "",
            ),
            "xl/_rels/workbook.xml.rels" => stated.replace(
                "<Relationship Id=\"rId7\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain\" Target=\"calcChain.xml\"/>",
                "",
            ),
            part if rewritten_parts.contains(&part) => {
                let mut stated = stated.to_owned();
                if let Some((sheet, from, to)) = edited
                    && sheet_of(sheet) == part
                {
                    assert!(stated.contains(from), "{from} in {part}");
                    stated = stated.replace(from, to);
                }
                let shared: &[(&str, &str)] = if part == "xl/worksheets/sheet2.xml" {
                    &REPORT_SHARED
                } else {
                    &[]
                };
                assert_eq!(
                    normalized(&text(find(&after, part)), &[]),
                    normalized(&stated, shared),
                    "{part}"
                );
                continue;
            }
            _ => {
                assert_eq!(find(&after, name), find(&before, name), "{name}");
                continue;
            }
        };
        assert_eq!(
            normalized(&text(find(&after, name)), &[]),
            normalized(&expected, &[]),
            "{name}"
        );
    }
    assert_eq!(after.len(), before.len() - 1, "no member is added");
}

#[test]
fn a_workbook_whose_every_sheet_is_written_again_states_what_it_read() {
    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    for (name, cell) in [("Data", "A1"), ("Report", "A1"), ("Pivot", "A3")] {
        // Borrowing a cell counts as a change: the sheet is written from
        // its cells.
        drop(opened.sheet_mut(name).unwrap().cell_mut(at(cell)).unwrap());
    }
    let written = opened.into_bytes().unwrap();
    assert_only_the_closed_list_changed(&written, &["Data", "Report", "Pivot"], None);
}

#[test]
fn an_edited_cell_changes_only_its_cell() {
    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    opened
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("C3"), 99.0)
        .unwrap();
    let written = opened.into_bytes().unwrap();
    assert_only_the_closed_list_changed(
        &written,
        &["Data"],
        Some((
            "Data",
            "<c r=\"C3\"><v>20</v></c>",
            "<c r=\"C3\"><v>99</v></c>",
        )),
    );
    // What the package held reads back, the edit included.
    let reopened = Workbook::from_bytes(written).unwrap();
    let data = reopened.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("C3")), Scalar::from(99.0));
    assert_eq!(data.scalar(at("A3")), Scalar::from("beta"));
    assert_eq!(data.scalar(at("C4")), Scalar::from("Apple"));
    assert_eq!(data.scalar(at("A5")), Scalar::from("inline "));
}

#[test]
fn a_calculation_chain_goes_with_its_relationship_and_content_type_on_a_save_that_wrote_a_sheet() {
    let chain = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                 <calcChain xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><c r=\"A1\" i=\"1\"/></calcChain>";
    let types = content_types(1, false, false).replace(
        "</Types>",
        "<Override PartName=\"/xl/calcChain.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml\"/></Types>",
    );
    let rels = relationships(&[
        ("rId1", WORKSHEET, "worksheets/sheet1.xml"),
        (
            "rId2",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain",
            "calcChain.xml",
        ),
    ]);
    let book = workbook_with(
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
         <calcPr calcId=\"191029\"/><extLst/>",
    );
    let sheet = number_sheet(1);
    let original = package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", rels.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ("xl/calcChain.xml", chain),
    ]);

    // A save that wrote nothing keeps the chain, which still holds.
    let unchanged = Workbook::from_bytes(original.clone())
        .unwrap()
        .into_bytes()
        .unwrap();
    assert_eq!(stored_members(&unchanged), stored_members(&original));

    let mut opened = Workbook::from_bytes(original).unwrap();
    opened
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A2"), 2.0)
        .unwrap();
    let written = opened.into_bytes().unwrap();
    assert!(
        !member_names(&written)
            .iter()
            .any(|name| name == "xl/calcChain.xml")
    );
    let rels = member_text(&written, "xl/_rels/workbook.xml.rels");
    assert!(!rels.contains("calcChain"), "{rels}");
    assert!(rels.contains("worksheets/sheet1.xml"), "{rels}");
    let types = member_text(&written, "[Content_Types].xml");
    assert!(!types.contains("calcChain"), "{types}");
    // The calculation properties Excel stated stay, recalculation added.
    let part = member_text(&written, "xl/workbook.xml");
    assert!(
        part.contains("<calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/><extLst/>"),
        "{part}"
    );
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("A2")),
        Scalar::from(2.0)
    );
}

#[test]
fn a_workbook_part_stating_no_calculation_properties_gains_them_where_the_schema_puts_them() {
    let book = workbook_with(
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
         <pivotCaches/><extLst/>",
    );
    let mut opened = Workbook::from_bytes(one_sheet_under(&book)).unwrap();
    opened
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A2"), 2.0)
        .unwrap();
    let part = member_text(&opened.into_bytes().unwrap(), "xl/workbook.xml");
    assert!(
        part.contains("</sheets><calcPr fullCalcOnLoad=\"1\"/><pivotCaches/><extLst/>"),
        "{part}"
    );
}

#[test]
fn a_workbook_is_dirty_exactly_while_it_holds_what_its_package_does_not() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    assert!(!opened.is_dirty());
    // Reading, parsing and borrowing mutably without a change are not edits.
    opened.sheet("Sheet1").unwrap();
    opened.parse_all().unwrap();
    opened.sheet_mut("Sheet2").unwrap();
    assert!(!opened.is_dirty());

    opened
        .sheet_mut("Sheet2")
        .unwrap()
        .set_cell(at("B1"), 1.0)
        .unwrap();
    assert!(opened.is_dirty());
    let mut target = Buffer::new();
    opened.write_into(&mut target).unwrap();
    assert!(!opened.is_dirty());

    // Each change of the documents counts.
    for change in [
        |book: &mut Workbook| book.rename_sheet("Sheet1", "First").unwrap(),
        |book: &mut Workbook| book.set_date_system(DateSystem::Year1904),
        |book: &mut Workbook| {
            book.add_sheet("Added").unwrap();
        },
        |book: &mut Workbook| {
            book.remove_sheet("Sheet3").unwrap();
        },
    ] {
        let mut book = Workbook::from_bytes(three_sheets()).unwrap();
        change(&mut book);
        assert!(book.is_dirty());
        book.write_into(&mut Buffer::new()).unwrap();
        assert!(!book.is_dirty());
    }
    // A new workbook is its template until a sheet is added.
    let mut built = Workbook::new();
    assert!(!built.is_dirty());
    built.add_sheet("Only").unwrap();
    assert!(built.is_dirty());
}

#[test]
fn a_renamed_sheet_is_the_workbook_part_s_change_and_its_own_part_is_copied() {
    let original = stored_fixtures().remove(0).1;
    let mut opened = Workbook::from_bytes(original.clone()).unwrap();
    opened.sheet("Sheet2").unwrap();
    opened.rename_sheet("Sheet2", "Renamed").unwrap();
    assert_eq!(opened.sheet("Renamed").unwrap().name(), "Renamed");
    let written = opened.into_bytes().unwrap();
    let find = |members: Vec<(String, Codec, u32, u64, Vec<u8>)>| {
        members
            .into_iter()
            .find(|member| member.0 == "xl/worksheets/sheet2.xml")
            .unwrap()
    };
    assert_eq!(
        find(stored_members(&written)),
        find(stored_members(&original))
    );
    assert!(
        member_text(&written, "xl/workbook.xml")
            .contains("<sheet name=\"Renamed\" sheetId=\"2\" r:id=\"rId2\"/>")
    );
}

#[test]
fn sheet_keys_stay_with_their_sheet_and_are_never_given_to_another() {
    let mut opened = Workbook::from_bytes(three_sheets()).unwrap();
    let keys: Vec<SheetKey> = ["Sheet1", "Sheet2", "Sheet3"]
        .iter()
        .map(|name| opened.sheet_key(name).unwrap())
        .collect();
    assert_eq!(
        keys.iter().map(|key| key.as_u32()).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(opened.sheet_key("SHEET2"), Some(keys[1]));
    assert_eq!(opened.sheet_key("Nope"), None);

    opened.rename_sheet("Sheet2", "Middle").unwrap();
    assert_eq!(opened.sheet_by_key(keys[1]), Some("Middle"));
    opened.remove_sheet("Sheet3").unwrap();
    assert_eq!(opened.sheet_by_key(keys[2]), None);
    // A sheet added under a gone sheet's name is another sheet.
    opened.add_sheet("Sheet3").unwrap();
    let added = opened.sheet_key("Sheet3").unwrap();
    assert!(!keys.contains(&added));
    // So is a sheet put in place of one.
    opened.insert_sheet(Sheet::new("Sheet1").unwrap()).unwrap();
    let replaced = opened.sheet_key("Sheet1").unwrap();
    assert!(!keys.contains(&replaced) && replaced != added);
    assert_eq!(opened.sheet_by_key(keys[0]), None);
    // A save keeps them.
    opened.write_into(&mut Buffer::new()).unwrap();
    assert_eq!(opened.sheet_by_key(keys[1]), Some("Middle"));
    assert_eq!(opened.sheet_key("Sheet3"), Some(added));
}

#[test]
fn a_saved_workbook_reopens_in_place() {
    let folder = scratch("in-place");
    let path = folder.join("book.xlsx");
    std::fs::write(&path, three_sheets()).unwrap();

    // Opened over the file, only the first sheet read and edited, then
    // saved over the very file it reads from.
    let mut book = Workbook::open(Holder::file(&path).unwrap()).unwrap();
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), "first save")
        .unwrap();
    book.write_into(&mut Holder::file(&path).unwrap()).unwrap();
    assert!(!book.is_dirty());

    // The sheets it had not read come from what it wrote - the file it was
    // opened over moved under them - and a second edit saves the same way.
    assert_eq!(
        book.sheet("Sheet3").unwrap().scalar(at("A1")),
        Scalar::from(3.0)
    );
    book.sheet_mut("Sheet2")
        .unwrap()
        .set_cell(at("B2"), 22.0)
        .unwrap();
    book.write_into(&mut Holder::file(&path).unwrap()).unwrap();

    let reopened = Workbook::from_bytes(std::fs::read(&path).unwrap()).unwrap();
    let cell = |sheet: &str, reference: &str| reopened.sheet(sheet).unwrap().scalar(at(reference));
    assert_eq!(cell("Sheet1", "A1"), Scalar::from(1.0));
    assert_eq!(cell("Sheet1", "B1"), Scalar::from("first save"));
    assert_eq!(cell("Sheet2", "A1"), Scalar::from(2.0));
    assert_eq!(cell("Sheet2", "B2"), Scalar::from(22.0));
    assert_eq!(cell("Sheet3", "A1"), Scalar::from(3.0));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_failed_write_leaves_the_workbook_dirty() {
    let folder = scratch("failed-write");
    // A file where the target's folder would have to be: no write lands.
    let blocker = folder.join("blocker");
    std::fs::write(&blocker, b"a file, not a folder").unwrap();
    let mut target = Holder::file(blocker.join("book.xlsx")).unwrap();

    let mut book = Workbook::from_bytes(three_sheets()).unwrap();
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), "kept")
        .unwrap();
    book.rename_sheet("Sheet3", "Third").unwrap();
    assert!(book.write_into(&mut target).is_err());
    assert!(book.is_dirty());

    // What it holds is what it held: the next save writes all of it.
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.sheet_names(), ["Sheet1", "Sheet2", "Third"]);
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("B1")),
        Scalar::from("kept")
    );
    assert_eq!(
        reopened.sheet("Sheet2").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
    let mut written = Buffer::new();
    book.write_into(&mut written).unwrap();
    assert!(!book.is_dirty());
    assert_eq!(
        Workbook::from_bytes(written.into_bytes())
            .unwrap()
            .sheet("Third")
            .unwrap()
            .scalar(at("A1")),
        Scalar::from(3.0)
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn rebase_adopts_a_package_the_caller_wrote_and_an_edit_made_since_stays_unsaved() {
    let mut book = Workbook::from_bytes(three_sheets()).unwrap();
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 1.0)
        .unwrap();
    let package = book.into_package().unwrap();
    // Building a package changes nothing about the workbook.
    assert!(book.is_dirty());
    assert_eq!(package.as_bytes(), book.into_bytes().unwrap().as_slice());
    // An edit made while the caller writes it.
    book.sheet_mut("Sheet2")
        .unwrap()
        .set_cell(at("B1"), 2.0)
        .unwrap();
    book.rebase(package).unwrap();
    // Adopting reads nothing: the package is what the workbook reads from.
    assert_eq!(book.handle_reads(), 0);
    assert!(book.is_dirty());
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("B1")),
        Scalar::from(1.0)
    );
    assert_eq!(
        reopened.sheet("Sheet2").unwrap().scalar(at("B1")),
        Scalar::from(2.0)
    );

    let other = Workbook::from_bytes(three_sheets())
        .unwrap()
        .into_package()
        .unwrap();
    let refusal = book.rebase(other).unwrap_err();
    assert!(matches!(refusal, Error::Conflict { .. }), "{refusal:?}");
    assert_eq!(
        refusal.to_string(),
        "expected to create a saved state of this workbook at \"$\", got an existing package of \
         another workbook"
    );
}

#[test]
fn a_sheet_from_another_workbook_keeps_its_values_and_takes_this_workbook_s_styles() {
    let types = content_types(1, false, true);
    let sheet = worksheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c><c r=\"B1\" s=\"2\"><v>0.5</v></c>\
         <c r=\"C1\" vm=\"1\" ph=\"1\"><v>3</v></c></row>",
    );
    let css = styles(&[(164, "0.00%")], &[0, 14, 164]);
    let source = Workbook::from_bytes(package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/styles.xml", &css),
    ]))
    .unwrap();
    let read = source.sheet("Sheet1").unwrap().clone();
    assert_eq!(read.cell(at("B1")).unwrap().style(), StyleId::new(2));

    let mut other = Workbook::new();
    other.insert_sheet(read.clone()).unwrap();
    let held = other.sheet("Sheet1").unwrap();
    // The values and what they mean stay; the styles are this workbook's.
    for reference in ["A1", "B1", "C1"] {
        let cell = held.cell(at(reference)).unwrap();
        assert_eq!(cell.style(), StyleId::DEFAULT, "{reference}");
        assert_eq!(cell.value(), read.cell(at(reference)).unwrap().value());
    }
    assert_eq!(held.cell(at("A1")).unwrap().format(), NumberFormat::Date);
    let written = other.into_bytes().unwrap();
    let part = member_text(&written, "xl/worksheets/sheet1.xml");
    // The date is written under the date style this workbook interned; the
    // value metadata named the other workbook's part and goes, the
    // phonetic flag is the cell's own and stays.
    assert!(
        part.contains("<c r=\"A1\" s=\"1\"><v>45292</v></c>"),
        "{part}"
    );
    assert!(part.contains("<c r=\"B1\"><v>0.5</v></c>"), "{part}");
    assert!(part.contains("<c r=\"C1\" ph=\"1\"><v>3</v></c>"), "{part}");
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(
        reopened
            .style_sheet()
            .unwrap()
            .style(StyleId::new(1))
            .unwrap()
            .number_format,
        "yyyy-mm-dd"
    );

    // Put back in the workbook it came from, it keeps them.
    let mut source = source;
    source.insert_sheet(read).unwrap();
    let part = member_text(&source.into_bytes().unwrap(), "xl/worksheets/sheet2.xml");
    assert!(
        part.contains("<c r=\"A1\" s=\"1\"><v>45292</v></c>"),
        "{part}"
    );
    assert!(
        part.contains("<c r=\"B1\" s=\"2\"><v>0.5</v></c>"),
        "{part}"
    );
    assert!(
        part.contains("<c r=\"C1\" vm=\"1\" ph=\"1\"><v>3</v></c>"),
        "{part}"
    );
}

#[test]
fn a_tab_keeps_its_sheet_id_and_a_new_tab_takes_one_past_every_id_it_had() {
    let book = workbook_with(
        "<sheets><sheet name=\"A\" sheetId=\"4\" r:id=\"rId1\"/><sheet name=\"B\" sheetId=\"9\" r:id=\"rId2\"/></sheets>",
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(2, false, false)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &book),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(2, false, false),
        ),
        ("xl/worksheets/sheet1.xml", &number_sheet(1)),
        ("xl/worksheets/sheet2.xml", &number_sheet(2)),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    // The highest is gone, and still not given again.
    opened.remove_sheet("B").unwrap();
    opened.add_sheet("C").unwrap();
    let part = member_text(&opened.into_bytes().unwrap(), "xl/workbook.xml");
    assert!(
        part.contains(
            "<sheets><sheet name=\"A\" sheetId=\"4\" r:id=\"rId1\"/><sheet name=\"C\" sheetId=\"10\" r:id=\"rId3\"/></sheets>"
        ),
        "{part}"
    );
}

/// `Sheet1` and `Sheet2`, each holding 45292 - 2024-01-01 under the 1900
/// date system - at A1 under the built-in date format.
fn two_dated_sheets() -> Vec<u8> {
    let dated = worksheet("<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c></row>");
    package(&[
        ("[Content_Types].xml", &content_types(2, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1", "Sheet2"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(2, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &dated),
        ("xl/worksheets/sheet2.xml", &dated),
        ("xl/styles.xml", &styles(&[], &[0, 14])),
    ])
}

#[test]
fn a_date_system_set_keeps_the_day_of_every_sheet_parsed_or_not() {
    let day = Scalar::date32(19_723);
    // Nothing parsed, one sheet parsed, both parsed: the switch keeps each
    // date's day whichever sheets were read before it.
    for parsed in [&[][..], &["Sheet1"][..], &["Sheet1", "Sheet2"][..]] {
        let mut book = Workbook::from_bytes(two_dated_sheets()).unwrap();
        for name in parsed {
            assert_eq!(book.sheet(name).unwrap().scalar(at("A1")), day);
        }
        book.set_date_system(DateSystem::Year1904);
        assert!(book.is_dirty());
        // A sheet read after the switch reads its part under the system the
        // part was written in.
        for name in ["Sheet1", "Sheet2"] {
            assert_eq!(
                book.sheet(name).unwrap().scalar(at("A1")),
                day,
                "{parsed:?} {name}"
            );
        }
        let mut saved = Buffer::new();
        book.write_into(&mut saved).unwrap();
        assert!(!book.is_dirty(), "{parsed:?}");
        let bytes = saved.into_bytes();
        assert!(
            member_text(&bytes, "xl/workbook.xml").contains("date1904=\"1\""),
            "{parsed:?}"
        );
        // Every sheet is written again, its serial counted from 1904.
        for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            assert!(
                member_text(&bytes, part).contains("<c r=\"A1\" s=\"1\"><v>43830</v></c>"),
                "{parsed:?} {part}"
            );
        }
        let reopened = Workbook::from_bytes(bytes).unwrap();
        assert_eq!(reopened.date_system(), DateSystem::Year1904);
        for name in ["Sheet1", "Sheet2"] {
            assert_eq!(
                reopened.sheet(name).unwrap().scalar(at("A1")),
                day,
                "{parsed:?} {name}"
            );
        }
    }
}

#[test]
fn a_package_built_before_the_one_last_adopted_is_refused() {
    let mut book = Workbook::from_bytes(three_sheets()).unwrap();
    let early = book.into_package().unwrap();
    book.add_sheet("Late")
        .unwrap()
        .set_cell(at("A1"), "late")
        .unwrap();
    let late = book.into_package().unwrap();
    // Two saves finishing out of order: the later one lands first.
    book.rebase(late).unwrap();
    assert!(!book.is_dirty());
    let refusal = book.rebase(early).unwrap_err();
    assert!(matches!(refusal, Error::Conflict { .. }), "{refusal:?}");
    assert_eq!(
        refusal.to_string(),
        "expected to create a saved state newer than the one last adopted at \"$\", got an \
         existing package built before it"
    );
    // The workbook still reads from the later package, the added sheet in
    // it, and saves it whole.
    assert!(!book.is_dirty());
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet_names(),
        ["Sheet1", "Sheet2", "Sheet3", "Late"]
    );
    assert_eq!(
        reopened.sheet("Late").unwrap().scalar(at("A1")),
        Scalar::from("late")
    );

    // A save in place is an adoption too: a package built before it is
    // refused alike.
    let stale = book.into_package().unwrap();
    book.sheet_mut("Late")
        .unwrap()
        .set_cell(at("A2"), 2.0)
        .unwrap();
    book.write_into(&mut Buffer::new()).unwrap();
    assert!(matches!(
        book.rebase(stale).unwrap_err(),
        Error::Conflict { .. }
    ));
}

/// A one-sheet package whose workbook part prefixes its elements, the root
/// and `<x:sheets>` stating `root` and `sheets` beside their own names and
/// its one `<x:sheet>` naming its relationship as `id` spells it.
fn prefixed_workbook(root: &str, sheets: &str, id: &str) -> Vec<u8> {
    one_sheet_under(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <x:workbook xmlns:x=\"{NS}\"{root}><x:sheets{sheets}>\
         <x:sheet name=\"Sheet1\" sheetId=\"1\" {id}/>\
         </x:sheets></x:workbook>"
    ))
}

#[test]
fn a_prefixed_workbook_part_is_edited_under_its_own_prefixes() {
    let bound = format!(" xmlns:r=\"{R_NS}\"");
    for (root, sheets, id, reused) in [
        // The relationships namespace bound on each sheet alone.
        (
            String::new(),
            String::new(),
            format!("r:id=\"rId1\"{bound}"),
            None,
        ),
        // Bound on the root under a prefix of its own, which new sheets use.
        (
            format!(" xmlns:rel=\"{R_NS}\""),
            String::new(),
            "rel:id=\"rId1\"".to_owned(),
            Some("rel"),
        ),
        // Bound on the root and bound again to another namespace where the
        // sheets are: new sheets bind it themselves.
        (
            bound.clone(),
            " xmlns:r=\"urn:other\"".to_owned(),
            "r:id=\"rId1\"".to_owned(),
            None,
        ),
    ] {
        let label = format!("{root}|{sheets}|{id}");
        let mut book = Workbook::from_bytes(prefixed_workbook(&root, &sheets, &id)).unwrap();
        book.rename_sheet("Sheet1", "Data").unwrap();
        book.add_sheet("Added").unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at("B1"), "edited")
            .unwrap();
        let bytes = book.into_bytes().unwrap();
        let text = member_text(&bytes, "xl/workbook.xml");

        // Read in its namespaces, every element is SpreadsheetML and every
        // sheet names its relationship in the relationships namespace.
        let document = yggdryl::xml::from_bytes(text.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        assert!(root.is(Some(NS), "workbook"), "{label}: {text}");
        let sheets = root.child(Some(NS), "sheets").expect("the sheets");
        let listed: Vec<(String, String)> = sheets
            .children()
            .map(|sheet| {
                assert!(sheet.is(Some(NS), "sheet"), "{label}: {text}");
                let name = sheet.attribute_in(None, "name").unwrap().to_owned();
                let id = sheet
                    .attribute_in(Some(R_NS), "id")
                    .unwrap_or_else(|| panic!("{label}: no relationship id in {text}"))
                    .to_owned();
                (name, id)
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("Data".to_owned(), "rId1".to_owned()),
                ("Added".to_owned(), "rId2".to_owned())
            ],
            "{label}"
        );
        let calculation = root
            .child(Some(NS), "calcPr")
            .expect("calcPr, in the namespace");
        assert_eq!(
            calculation.attribute_in(None, "fullCalcOnLoad"),
            Some("1"),
            "{label}"
        );
        if let Some(prefix) = reused {
            // The root's binding is reused rather than declared again.
            assert!(
                text.contains(&format!(
                    "<x:sheet name=\"Added\" sheetId=\"2\" {prefix}:id=\"rId2\"/>"
                )),
                "{text}"
            );
        }

        let reopened = Workbook::from_bytes(bytes).unwrap();
        assert_eq!(reopened.sheet_names(), ["Data", "Added"]);
        assert_eq!(
            reopened.sheet("Data").unwrap().scalar(at("B1")),
            Scalar::from("edited")
        );
    }
}

#[test]
fn a_tab_numbered_the_highest_there_is_leaves_new_tabs_the_lowest_free_number() {
    let book = workbook_with(&format!(
        "<sheets><sheet name=\"Top\" sheetId=\"{}\" r:id=\"rId1\"/>\
         <sheet name=\"Unnumbered\" r:id=\"rId2\"/></sheets>",
        u32::MAX
    ));
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(2, false, false)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &book),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(2, false, false),
        ),
        ("xl/worksheets/sheet1.xml", &number_sheet(1)),
        ("xl/worksheets/sheet2.xml", &number_sheet(2)),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    opened.add_sheet("Added").unwrap();
    let part = member_text(&opened.into_bytes().unwrap(), "xl/workbook.xml");
    assert!(
        part.contains(&format!(
            "<sheets><sheet name=\"Top\" sheetId=\"{}\" r:id=\"rId1\"/>\
             <sheet name=\"Unnumbered\" sheetId=\"1\" r:id=\"rId2\"/>\
             <sheet name=\"Added\" sheetId=\"2\" r:id=\"rId3\"/></sheets>",
            u32::MAX
        )),
        "{part}"
    );
}

#[test]
fn a_relationship_numbered_the_highest_there_is_refuses_a_new_one_by_name() {
    let top = format!("rId{}", usize::MAX);
    let book = workbook_with(&format!(
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"{top}\"/></sheets>"
    ));
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &book),
        (
            "xl/_rels/workbook.xml.rels",
            &relationships(&[
                (top.as_str(), WORKSHEET, "worksheets/sheet1.xml"),
                ("rId2", STYLES, "styles.xml"),
            ]),
        ),
        ("xl/worksheets/sheet1.xml", &number_sheet(1)),
        ("xl/styles.xml", &styles(&[], &[0])),
    ]);
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    // A save that adds no relationship is untouched by it.
    opened
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 2.0)
        .unwrap();
    opened.into_bytes().unwrap();
    opened.add_sheet("Added").unwrap();
    assert_eq!(
        opened.into_bytes().unwrap_err().to_string(),
        format!(
            "invalid record value at xl/_rels/workbook.xml.rels: expected a relationship id \
             past every one the part states, got none left past {top}"
        )
    );
}

#[test]
fn a_part_numbered_the_highest_there_is_leaves_a_new_sheet_the_lowest_free_part() {
    let last = format!("xl/worksheets/sheet{}.xml", usize::MAX);
    let mut opened = Workbook::from_bytes(three_sheets_with(&[(
        last.as_str(),
        number_sheet(9).as_str(),
    )]))
    .unwrap();
    opened.add_sheet("Added").unwrap();
    let bytes = opened.into_bytes().unwrap();
    let names = member_names(&bytes);
    assert!(
        names.contains(&"xl/worksheets/sheet4.xml".to_owned()),
        "{names:?}"
    );
    assert!(names.contains(&last), "{names:?}");
    assert_eq!(
        Workbook::from_bytes(bytes).unwrap().sheet_names(),
        ["Sheet1", "Sheet2", "Sheet3", "Added"]
    );
}

/// The members of `bytes` the rich package held and `bytes` no longer does.
fn gone_from_rich(bytes: &[u8]) -> Vec<String> {
    let kept = member_names(bytes);
    let mut gone: Vec<String> = rich_parts()
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .filter(|name| !kept.contains(name))
        .collect();
    gone.sort();
    gone
}

/// The formula each cell of `cells` holds on `sheet`, in the file spelling.
fn formulas(sheet: &Sheet, cells: &[&str]) -> Vec<String> {
    cells
        .iter()
        .map(|reference| {
            let cell = sheet.cell(at(reference)).unwrap();
            cell.formula()
                .map(|formula| formula.at(cell.reference()).to_string())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn a_removed_sheet_takes_every_part_only_it_reached_and_their_content_types() {
    let original = rich_package();
    let mut opened = Workbook::from_bytes(original.clone()).unwrap();
    opened.remove_sheet("Data").unwrap();
    let written = opened.into_bytes().unwrap();
    // The drawing, native-styled chart, comments and VML drawing, table,
    // and printer settings went with Data; so did the stale calc chain.
    // Excel rejected the old synthetic chart style/color relationship
    // parts, so this fixture no longer pins their deletion.
    assert_eq!(
        gone_from_rich(&written),
        [
            "xl/calcChain.xml",
            "xl/charts/chart1.xml",
            "xl/comments1.xml",
            "xl/drawings/_rels/drawing1.xml.rels",
            "xl/drawings/drawing1.xml",
            "xl/drawings/vmlDrawing1.vml",
            "xl/printerSettings/printerSettings1.bin",
            "xl/tables/table1.xml",
            "xl/worksheets/_rels/sheet1.xml.rels",
            "xl/worksheets/sheet1.xml",
        ]
    );
    let types = member_text(&written, "[Content_Types].xml");
    for part in [
        "/xl/worksheets/sheet1.xml",
        "/xl/drawings/drawing1.xml",
        "/xl/charts/chart1.xml",
        "/xl/tables/table1.xml",
        "/xl/comments1.xml",
        "/xl/calcChain.xml",
    ] {
        assert!(!types.contains(part), "{part} in {types}");
    }
    // What the tabs left reach stays: the pivot table and its cache.
    for part in [
        "/xl/pivotTables/pivotTable1.xml",
        "/xl/pivotCache/pivotCacheDefinition1.xml",
        "/xl/pivotCache/pivotCacheRecords1.xml",
        "/xl/metadata.xml",
    ] {
        assert!(types.contains(part), "{part} in {types}");
    }
    let book = member_text(&written, "xl/workbook.xml");
    assert!(book.contains("activeTab=\"0\""), "{book}");
    assert!(
        book.contains("<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId8\"/></pivotCaches>"),
        "{book}"
    );
    // The formulas naming the sheet name nothing.
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["Report", "Pivot"]);
    assert_eq!(reopened.active_tab(), 0);
    assert_eq!(
        formulas(reopened.sheet("Report").unwrap(), &["A1", "C1", "A2", "A3"]),
        [
            "SUM(#REF!B2:B4)",
            "_xlfn.CONCAT(#REF!A2,\"-\",#REF!E2)",
            "#REF!B2*#REF!C2",
            "#REF!B3*#REF!C3",
        ]
    );
}

#[test]
fn a_pivot_cache_goes_with_the_last_pivot_table_built_on_it() {
    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    opened.remove_sheet("Pivot").unwrap();
    let written = opened.into_bytes().unwrap();
    assert_eq!(
        gone_from_rich(&written),
        [
            "xl/calcChain.xml",
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            "xl/pivotCache/pivotCacheDefinition1.xml",
            "xl/pivotCache/pivotCacheRecords1.xml",
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            "xl/pivotTables/pivotTable1.xml",
            "xl/worksheets/_rels/sheet3.xml.rels",
            "xl/worksheets/sheet3.xml",
        ]
    );
    let book = member_text(&written, "xl/workbook.xml");
    assert!(!book.contains("pivotCache"), "{book}");
    // The tab the workbook opened on is still the one it shows.
    assert!(book.contains("activeTab=\"1\""), "{book}");
    let rels = member_text(&written, "xl/_rels/workbook.xml.rels");
    assert!(!rels.contains("pivotCacheDefinition"), "{rels}");
    let types = member_text(&written, "[Content_Types].xml");
    assert!(!types.contains("pivot"), "{types}");
    // The sheets left are copied as they are stored: none named the one
    // removed.
    let before = stored_members(&rich_package());
    let after = stored_members(&written);
    for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        assert_eq!(
            after.iter().find(|member| member.0 == part),
            before.iter().find(|member| member.0 == part),
            "{part}"
        );
    }
}

#[test]
fn a_removed_active_tab_shows_the_nearest_visible_sheet_after_it_else_before_it() {
    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    assert_eq!(opened.active_tab(), 1);
    opened.remove_sheet("Report").unwrap();
    assert_eq!(opened.active_tab(), 1);
    assert_eq!(opened.sheet_names()[opened.active_tab()], "Pivot");

    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    opened
        .sheet_mut("Pivot")
        .unwrap()
        .set_state(SheetState::Hidden);
    opened.remove_sheet("Report").unwrap();
    assert_eq!(opened.active_tab(), 0);
    let book = member_text(&opened.into_bytes().unwrap(), "xl/workbook.xml");
    assert!(book.contains("activeTab=\"0\""), "{book}");
}

#[test]
fn a_rename_rewrites_the_formulas_naming_the_sheet_and_leaves_every_other_sheet_stored() {
    let original = rich_package();
    let mut opened = Workbook::from_bytes(original.clone()).unwrap();
    opened.rename_sheet("Data", "Q1 Data").unwrap();
    let report = opened.sheet("Report").unwrap();
    assert_eq!(
        formulas(report, &["A1", "B1", "C1", "A2", "A4", "B2"]),
        [
            "SUM('Q1 Data'!B2:B4)",
            "Rate*A1",
            "_xlfn.CONCAT('Q1 Data'!A2,\"-\",'Q1 Data'!E2)",
            "'Q1 Data'!B2*'Q1 Data'!C2",
            "'Q1 Data'!B4*'Q1 Data'!C4",
            "'Q1 Data'!B2:B4*2",
        ]
    );
    // The dependents of the shared group still hold its one shape.
    assert_eq!(
        report.cell(at("A3")).unwrap().formula(),
        report.cell(at("A2")).unwrap().formula()
    );
    let written = opened.into_bytes().unwrap();
    let part = member_text(&written, "xl/worksheets/sheet2.xml");
    assert!(
        part.contains("<c r=\"A1\"><f>SUM('Q1 Data'!B2:B4)</f><v>4.25</v></c>"),
        "{part}"
    );
    // What the renamed sheet carries names it by its new name: its
    // sparkline's data; so do the chart over it and the pivot cache reading
    // it, rewritten beside the sheets.
    let data = member_text(&written, "xl/worksheets/sheet1.xml");
    assert!(data.contains("<xm:f>'Q1 Data'!B2:C2</xm:f>"), "{data}");
    assert!(data.contains("location=\"Report!A1\""), "{data}");
    let chart = member_text(&written, "xl/charts/chart1.xml");
    assert!(chart.contains("<c:f>'Q1 Data'!$B$2:$B$4</c:f>"), "{chart}");
    let cache = member_text(&written, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(
        cache.contains("<worksheetSource ref=\"A1:B2\" sheet=\"Q1 Data\"/>"),
        "{cache}"
    );
    // The pivot sheet names no sheet: it is the member the package stored.
    let before = stored_members(&original);
    let after = stored_members(&written);
    for part in ["xl/worksheets/sheet3.xml", "xl/drawings/drawing1.xml"] {
        assert_eq!(
            after.iter().find(|member| member.0 == part),
            before.iter().find(|member| member.0 == part),
            "{part}"
        );
    }
}

#[test]
fn a_moved_sheet_takes_its_names_and_the_views_follow_the_sheets_they_showed() {
    let mut opened = Workbook::from_bytes(rich_package()).unwrap();
    // Refused before anything changes.
    assert!(matches!(
        opened.move_sheet("Data", 3),
        Err(Error::InvalidRecord { .. })
    ));
    assert!(matches!(
        opened.move_sheet("Missing", 0),
        Err(Error::Absent { .. })
    ));
    assert!(!opened.is_dirty());
    opened.move_sheet("Pivot", 0).unwrap();
    assert_eq!(opened.sheet_names(), ["Pivot", "Data", "Report"]);
    assert_eq!(opened.active_tab(), 2);
    let data = opened.sheet_key("Data");
    assert_eq!(opened.defined_names().next().unwrap().scope(), data);
    let written = opened.into_bytes().unwrap();
    let book = member_text(&written, "xl/workbook.xml");
    for stated in [
        "<sheets><sheet name=\"Pivot\" sheetId=\"4\" r:id=\"rId3\"/>\
         <sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/>\
         <sheet name=\"Report\" sheetId=\"2\" r:id=\"rId2\"/></sheets>",
        "activeTab=\"2\"",
        "<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"1\" hidden=\"1\">Data!$A$1:$C$4</definedName>",
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"2\">Report!$A$1:$E$4</definedName>",
        "<definedName name=\"Rate\">0.07</definedName>",
    ] {
        assert!(book.contains(stated), "{stated} in {book}");
    }
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(reopened.sheet_names(), ["Pivot", "Data", "Report"]);
    assert_eq!(
        reopened.defined_names().nth(1).unwrap().scope(),
        reopened.sheet_key("Report")
    );
    // A move to where the sheet stands changes nothing.
    let mut still = Workbook::from_bytes(rich_package()).unwrap();
    still.move_sheet("report", 1).unwrap();
    assert!(!still.is_dirty());
}

/// The rich package with a second pivot sheet, `Pivot2`, whose table is
/// built on a cache of its own.
fn two_pivot_caches() -> Vec<u8> {
    let mut parts = rich_parts();
    let text = |parts: &[(&str, Vec<u8>)], name: &str| {
        parts
            .iter()
            .find(|(part, _)| *part == name)
            .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
            .unwrap()
    };
    let replace = |parts: &mut Vec<(&str, Vec<u8>)>, name: &str, from: &str, to: &str| {
        let part = parts.iter_mut().find(|(part, _)| *part == name).unwrap();
        let text = std::str::from_utf8(&part.1).unwrap();
        assert!(text.contains(from), "{from} in {name}");
        part.1 = text.replacen(from, to, 1).into_bytes();
    };
    let copies = [
        ("xl/worksheets/sheet4.xml", "xl/worksheets/sheet3.xml", None),
        (
            "xl/worksheets/_rels/sheet4.xml.rels",
            "xl/worksheets/_rels/sheet3.xml.rels",
            Some(("pivotTable1", "pivotTable2")),
        ),
        (
            "xl/pivotTables/pivotTable2.xml",
            "xl/pivotTables/pivotTable1.xml",
            Some(("cacheId=\"1\"", "cacheId=\"2\"")),
        ),
        (
            "xl/pivotTables/_rels/pivotTable2.xml.rels",
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            Some(("pivotCacheDefinition1", "pivotCacheDefinition2")),
        ),
        (
            "xl/pivotCache/pivotCacheDefinition2.xml",
            "xl/pivotCache/pivotCacheDefinition1.xml",
            None,
        ),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition2.xml.rels",
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            Some(("pivotCacheRecords1", "pivotCacheRecords2")),
        ),
        (
            "xl/pivotCache/pivotCacheRecords2.xml",
            "xl/pivotCache/pivotCacheRecords1.xml",
            None,
        ),
    ];
    for (name, from, change) in copies {
        let mut copied = text(&parts, from);
        if let Some((old, new)) = change {
            copied = copied.replace(old, new);
        }
        parts.push((name, copied.into_bytes()));
    }
    let main = "application/vnd.openxmlformats-officedocument";
    replace(
        &mut parts,
        "[Content_Types].xml",
        "</Types>",
        &format!(
            "<Override PartName=\"/xl/worksheets/sheet4.xml\" ContentType=\"{main}.spreadsheetml.worksheet+xml\"/>\
             <Override PartName=\"/xl/pivotTables/pivotTable2.xml\" ContentType=\"{main}.spreadsheetml.pivotTable+xml\"/>\
             <Override PartName=\"/xl/pivotCache/pivotCacheDefinition2.xml\" ContentType=\"{main}.spreadsheetml.pivotCacheDefinition+xml\"/>\
             <Override PartName=\"/xl/pivotCache/pivotCacheRecords2.xml\" ContentType=\"{main}.spreadsheetml.pivotCacheRecords+xml\"/></Types>"
        ),
    );
    replace(
        &mut parts,
        "xl/_rels/workbook.xml.rels",
        "</Relationships>",
        "<Relationship Id=\"rId10\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet4.xml\"/>\
         <Relationship Id=\"rId11\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition\" Target=\"pivotCache/pivotCacheDefinition2.xml\"/></Relationships>",
    );
    replace(
        &mut parts,
        "xl/workbook.xml",
        "</sheets>",
        "<sheet name=\"Pivot2\" sheetId=\"5\" r:id=\"rId10\"/></sheets>",
    );
    replace(
        &mut parts,
        "xl/workbook.xml",
        "</pivotCaches>",
        "<pivotCache cacheId=\"2\" r:id=\"rId11\"/></pivotCaches>",
    );
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    package(&parts)
}

#[test]
fn the_pivot_cache_list_goes_with_its_last_cache_across_saves() {
    let mut book = Workbook::from_bytes(two_pivot_caches()).unwrap();
    book.remove_sheet("Pivot").unwrap();
    let mut first = Buffer::new();
    book.write_into(&mut first).unwrap();
    let first = first.into_bytes();
    let listed = member_text(&first, "xl/workbook.xml");
    assert!(
        listed.contains("<pivotCaches><pivotCache cacheId=\"2\" r:id=\"rId11\"/></pivotCaches>"),
        "{listed}"
    );
    assert!(!member_names(&first).contains(&"xl/pivotCache/pivotCacheDefinition1.xml".to_owned()));
    // The second save reads the part the first one wrote: the cache it
    // dropped is no cache left.
    book.remove_sheet("Pivot2").unwrap();
    let second = book.into_bytes().unwrap();
    let listed = member_text(&second, "xl/workbook.xml");
    assert!(!listed.contains("pivotCache"), "{listed}");
    let names = member_names(&second);
    assert!(
        !names.iter().any(|name| name.contains("pivot")),
        "{names:?}"
    );

    // Adopted rather than written in place, the same.
    let mut book = Workbook::from_bytes(two_pivot_caches()).unwrap();
    book.remove_sheet("Pivot2").unwrap();
    let package = book.into_package().unwrap();
    book.rebase(package).unwrap();
    book.remove_sheet("Pivot").unwrap();
    let listed = member_text(&book.into_bytes().unwrap(), "xl/workbook.xml");
    assert!(!listed.contains("pivotCache"), "{listed}");
}

#[test]
fn a_removed_sheet_takes_the_content_type_a_writer_stated_for_its_relationships() {
    // LibreOffice states an override for each relationships part.
    let rels_type = "application/vnd.openxmlformats-package.relationships+xml";
    let parts: Vec<(&str, Vec<u8>)> = rich_parts()
        .into_iter()
        .map(|(name, bytes)| {
            if name == "[Content_Types].xml" {
                let overrides = format!(
                    "<Override PartName=\"/xl/worksheets/_rels/sheet1.xml.rels\" ContentType=\"{rels_type}\"/>\
                     <Override PartName=\"/xl/worksheets/_rels/sheet3.xml.rels\" ContentType=\"{rels_type}\"/></Types>"
                );
                (name, std::str::from_utf8(&bytes).unwrap().replace("</Types>", &overrides).into_bytes())
            } else {
                (name, bytes)
            }
        })
        .collect();
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let mut opened = Workbook::from_bytes(package(&parts)).unwrap();
    opened.remove_sheet("Pivot").unwrap();
    let types = member_text(&opened.into_bytes().unwrap(), "[Content_Types].xml");
    assert!(!types.contains("sheet3.xml"), "{types}");
    assert!(
        types.contains("PartName=\"/xl/worksheets/_rels/sheet1.xml.rels\""),
        "{types}"
    );
}

/// A style patched between building a package and rebasing onto it is
/// kept: the package's own temporal style took the index the patch's was
/// appended at, so the patched cell names its style at its new index, and
/// the next save writes it.
#[test]
fn rebase_keeps_the_styles_appended_after_the_package_was_built() {
    use yggdryl::excel::StylePatch;

    let mut book = Workbook::new();
    book.add_sheet("Sheet1").unwrap();
    // A date whose style the save interns: the package appends it.
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_723))
        .unwrap();
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 5.0)
        .unwrap();
    let held = book.style_sheet().unwrap().len();
    let package = book.into_package().unwrap();
    // An edit made while the caller writes it appends at the same index.
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    book.set_style("Sheet1", &["B1".parse().unwrap()], &bold)
        .unwrap();
    assert_eq!(
        book.sheet("Sheet1")
            .unwrap()
            .cell(at("B1"))
            .unwrap()
            .style(),
        StyleId::new(u16::try_from(held).unwrap())
    );
    book.rebase(package).unwrap();
    // The package's style kept its index; the patched one moved past it.
    let moved = book
        .sheet("Sheet1")
        .unwrap()
        .cell(at("B1"))
        .unwrap()
        .style();
    assert_eq!(moved, StyleId::new(u16::try_from(held + 1).unwrap()));
    assert!(book.cell_style("Sheet1", at("B1")).unwrap().font.bold);
    assert!(book.is_dirty());
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert!(reopened.cell_style("Sheet1", at("B1")).unwrap().font.bold);
    assert!(!reopened.cell_style("Sheet1", at("A1")).unwrap().font.bold);
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    assert_eq!(
        reopened
            .cell_style("Sheet1", at("A1"))
            .unwrap()
            .number_format,
        "yyyy-mm-dd"
    );
}

/// The first package appends a temporal format at the index an edit made
/// while it is being written can also take. Its later adoption moves that
/// edit's style, including every opaque inverse still held by a caller.
fn retained_style_snapshot() -> (Workbook, yggdryl::excel::Package) {
    let mut book = Workbook::new();
    book.add_sheet("A").unwrap();
    book.add_sheet("Keep").unwrap();
    book.sheet_mut("A")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_723))
        .unwrap();
    book.sheet_mut("A")
        .unwrap()
        .set_cell(at("B1"), 5.0)
        .unwrap();
    let older = book.into_package().unwrap();
    (book, older)
}

fn retain_bold_style(book: &mut Workbook, range: &str) {
    book.set_style(
        "A",
        &[range.parse().unwrap()],
        &yggdryl::excel::StylePatch {
            bold: Some(true),
            ..yggdryl::excel::StylePatch::default()
        },
    )
    .unwrap();
}

fn assert_retained_bold_style(book: &Workbook, cell: &str) {
    let style = book.cell_style("A", at(cell)).unwrap();
    assert!(style.font.bold, "{cell}: {style:?}");
    assert_eq!(style.number_format, "General", "{cell}");
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    let style = reopened.cell_style("A", at(cell)).unwrap();
    assert!(style.font.bold, "saved {cell}: {style:?}");
    assert_eq!(style.number_format, "General", "saved {cell}");
    assert_eq!(
        reopened.sheet("A").unwrap().scalar(at("B1")),
        Scalar::from(5.0)
    );
}

#[test]
fn rebase_keeps_style_bindings_in_a_removed_sheet_inverse() {
    use yggdryl::excel::Edit;

    let (mut book, older) = retained_style_snapshot();
    retain_bold_style(&mut book, "B1");
    assert!(book.cell_style("A", at("B1")).unwrap().font.bold);
    let undo = book
        .apply(Edit::RemoveSheet { name: "A".into() })
        .unwrap()
        .inverse
        .unwrap();
    book.rebase(older).unwrap();
    book.apply(undo).unwrap();
    assert_retained_bold_style(&book, "B1");
}

#[test]
fn rebase_keeps_style_bindings_in_a_cell_range_inverse() {
    use yggdryl::excel::{Clear, Edit};

    let (mut book, older) = retained_style_snapshot();
    retain_bold_style(&mut book, "B1");
    assert!(book.cell_style("A", at("B1")).unwrap().font.bold);
    let undo = book
        .apply(Edit::Clear {
            sheet: "A".into(),
            ranges: vec!["B1".parse().unwrap()],
            what: Clear::All,
        })
        .unwrap()
        .inverse
        .unwrap();
    book.rebase(older).unwrap();
    book.apply(undo).unwrap();
    assert_retained_bold_style(&book, "B1");
}

#[test]
fn rebase_keeps_style_bindings_in_a_row_format_inverse() {
    use yggdryl::excel::Edit;

    let (mut book, older) = retained_style_snapshot();
    // An empty cell inherits only the row's style, so a remapped cell style
    // cannot hide the stale row-format ID restored by RowHeight's inverse.
    retain_bold_style(&mut book, "2:2");
    assert!(book.cell_style("A", at("B2")).unwrap().font.bold);
    let undo = book
        .apply(Edit::RowHeight {
            sheet: "A".into(),
            start: 1,
            count: 1,
            height: Some(28.0),
        })
        .unwrap()
        .inverse
        .unwrap();
    book.rebase(older).unwrap();
    assert!(book.cell_style("A", at("B2")).unwrap().font.bold);
    book.apply(undo).unwrap();
    assert_retained_bold_style(&book, "B2");
}

#[test]
fn rebase_keeps_style_bindings_in_a_column_format_inverse() {
    use yggdryl::excel::Edit;

    let (mut book, older) = retained_style_snapshot();
    retain_bold_style(&mut book, "C:C");
    assert!(book.cell_style("A", at("C2")).unwrap().font.bold);
    let undo = book
        .apply(Edit::ColumnWidth {
            sheet: "A".into(),
            start: 2,
            count: 1,
            width: Some(16.0),
        })
        .unwrap()
        .inverse
        .unwrap();
    book.rebase(older).unwrap();
    assert!(book.cell_style("A", at("C2")).unwrap().font.bold);
    book.apply(undo).unwrap();
    assert_retained_bold_style(&book, "C2");
}

#[test]
fn rebase_keeps_style_bindings_across_overlapping_snapshots() {
    let (mut book, first) = retained_style_snapshot();
    retain_bold_style(&mut book, "B1");
    // This package assigns bold before date; the older one does the reverse.
    let second = book.into_package().unwrap();
    book.rebase(first).unwrap();
    book.sheet_mut("A")
        .unwrap()
        .set_cell(at("C1"), 5.0)
        .unwrap();
    book.set_style(
        "A",
        &["C1".parse().unwrap()],
        &yggdryl::excel::StylePatch {
            number_format: Some("yyyy-mm-dd".into()),
            ..yggdryl::excel::StylePatch::default()
        },
    )
    .unwrap();
    let expected = book.cell_style("A", at("C1")).unwrap();
    assert_eq!(expected.number_format, "yyyy-mm-dd");
    assert!(!expected.font.bold);
    book.rebase(second).unwrap();
    assert_eq!(book.cell_style("A", at("C1")).unwrap(), expected);
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.cell_style("A", at("C1")).unwrap(), expected);
    assert!(reopened.cell_style("A", at("B1")).unwrap().font.bold);
    assert_eq!(
        reopened.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
}

#[test]
fn rebase_keeps_style_bindings_in_a_clean_removed_worksheet_part() {
    use yggdryl::excel::{Edit, StylePatch};

    let (mut book, first) = retained_style_snapshot();
    book.sheet_mut("Keep")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    book.set_style(
        "Keep",
        &["A1".parse().unwrap()],
        &StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    )
    .unwrap();
    let second = book.into_package().unwrap();
    book.rebase(first).unwrap();
    // A is now clean. Its parsed date still has DEFAULT plus NumberFormat,
    // while its serialized root names the first package's temporal XF 1.
    let undo = book
        .apply(Edit::RemoveSheet { name: "A".into() })
        .unwrap()
        .inverse
        .unwrap();
    book.rebase(second).unwrap();
    book.apply(undo).unwrap();
    assert_eq!(
        book.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    assert!(!book.cell_style("A", at("A1")).unwrap().font.bold);
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    assert!(!reopened.cell_style("A", at("A1")).unwrap().font.bold);
    assert_eq!(
        reopened.cell_style("A", at("A1")).unwrap().number_format,
        "yyyy-mm-dd"
    );
    assert!(reopened.cell_style("Keep", at("A1")).unwrap().font.bold);
}

/// A package that merely copied its styles part still owns that part's
/// interpretation: an outstanding earlier save can reorder the live table
/// between its build and adoption without changing either sheet's values.
#[test]
fn rebase_keeps_style_bindings_when_a_clean_snapshot_copied_its_styles() {
    use yggdryl::excel::StylePatch;

    let (mut book, first) = retained_style_snapshot();
    retain_bold_style(&mut book, "B1");
    book.sheet_mut("Keep")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_723))
        .unwrap();
    let second = book.into_package().unwrap();
    let second_styles = member_text(second.as_bytes(), "xl/styles.xml");
    book.rebase(first).unwrap();
    // First adoption puts date before bold; this outstanding package keeps
    // that order, but its sheets have the same revisions as `second`.
    let outstanding = book.into_package().unwrap();
    book.rebase(second).unwrap();
    assert!(!book.is_dirty());
    assert_retained_bold_style(&book, "B1");
    // Clean sheets and unchanged styles are copied raw. Adoption still
    // needs the copied table's interpretation after another save intervenes.
    let copied = book.into_package().unwrap();
    assert_eq!(
        member_text(copied.as_bytes(), "xl/styles.xml"),
        second_styles
    );
    book.rebase(outstanding).unwrap();
    book.rebase(copied).unwrap();
    assert_retained_bold_style(&book, "B1");
    for name in ["A", "Keep"] {
        assert_eq!(
            book.sheet(name).unwrap().scalar(at("A1")),
            Scalar::date32(19_723)
        );
    }
    // A later style append writes the live style table while Keep's clean
    // worksheet stays raw. It must not reinterpret Keep's date as bold
    // General merely because those two saves used opposite XF orders.
    book.set_style(
        "A",
        &["C1".parse().unwrap()],
        &StylePatch {
            italic: Some(true),
            ..StylePatch::default()
        },
    )
    .unwrap();
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    for name in ["A", "Keep"] {
        assert_eq!(
            reopened.sheet(name).unwrap().scalar(at("A1")),
            Scalar::date32(19_723),
            "{name}"
        );
        let style = reopened.cell_style(name, at("A1")).unwrap();
        assert_eq!(style.number_format, "yyyy-mm-dd", "{name}");
        assert!(!style.font.bold, "{name}: {style:?}");
    }
    assert_retained_bold_style(&reopened, "B1");
}

/// RestoreSheet has already placed its clean raw worksheet in the active
/// overrides before the outstanding package changes the mutable XF suffix.
/// Rebinding a later opaque undo alone cannot repair this retained image.
#[test]
fn rebase_keeps_style_bindings_in_an_already_restored_worksheet_override() {
    use yggdryl::excel::Edit;

    let (mut book, first) = retained_style_snapshot();
    retain_bold_style(&mut book, "B1");
    let second = book.into_package().unwrap();
    let second_part = member_text(second.as_bytes(), "xl/worksheets/sheet1.xml");
    book.rebase(first).unwrap();
    let outstanding = book.into_package().unwrap();
    book.rebase(second).unwrap();
    assert!(!book.is_dirty());
    let undo = book
        .apply(Edit::RemoveSheet { name: "A".into() })
        .unwrap()
        .inverse
        .unwrap();
    book.apply(undo).unwrap();
    let restored = member_text(&book.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert_eq!(restored, second_part);
    assert_retained_bold_style(&book, "B1");
    assert_eq!(
        book.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    book.rebase(outstanding).unwrap();
    assert_retained_bold_style(&book, "B1");
    assert_eq!(
        book.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    assert!(!book.cell_style("A", at("A1")).unwrap().font.bold);
    let package = book.into_package().unwrap();
    let reopened = Workbook::from_bytes(package.as_bytes().to_vec()).unwrap();
    assert_eq!(
        reopened.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    let date = reopened.cell_style("A", at("A1")).unwrap();
    assert!(!date.font.bold, "{date:?}");
    assert_eq!(date.number_format, "yyyy-mm-dd");
    assert_retained_bold_style(&reopened, "B1");
    book.rebase(package).unwrap();
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("A").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    assert!(!reopened.cell_style("A", at("A1")).unwrap().font.bold);
    assert_retained_bold_style(&reopened, "B1");
}

/// A saved workbook patched again appends past what it saved, and a save
/// with no sheet written still writes the styles a patch appended.
#[test]
fn a_workbook_saved_in_place_and_patched_again_writes_its_new_styles() {
    use yggdryl::IOBase;
    use yggdryl::excel::StylePatch;

    let mut book = Workbook::new();
    book.add_sheet("Sheet1").unwrap();
    book.set_entry("Sheet1", at("A1"), "12%").unwrap();
    let mut target = Buffer::new();
    book.write_into(&mut target).unwrap();
    assert!(!book.is_dirty());
    let italic = StylePatch {
        italic: Some(true),
        ..StylePatch::default()
    };
    book.set_style("Sheet1", &["A1".parse().unwrap()], &italic)
        .unwrap();
    assert!(book.is_dirty());
    book.write_into(&mut target).unwrap();
    let reopened = Workbook::from_bytes(target.read_all_bytes().unwrap()).unwrap();
    let style = reopened.cell_style("Sheet1", at("A1")).unwrap();
    assert!(style.font.italic);
    assert_eq!(style.number_format, "0%");
}

/// A rebase whose styles cannot take a style appended since the package
/// was built is refused before anything is adopted: the workbook keeps
/// reading what it read, the appended style where it was.
#[test]
fn a_rebase_whose_styles_cannot_take_a_style_appended_since_adopts_nothing() {
    use yggdryl::excel::{MAX_CELL_FORMATS, StylePatch};

    let bytes = package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &worksheet("")),
        (
            "xl/styles.xml",
            &styles(&[], &vec![0; MAX_CELL_FORMATS - 1]),
        ),
    ]);
    let mut book = Workbook::from_bytes(bytes).unwrap();
    book.sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_723))
        .unwrap();
    // The save's date style takes the last cell format there is.
    let package = book.into_package().unwrap();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    book.set_style("Sheet1", &["B1".parse().unwrap()], &bold)
        .unwrap();
    let reads = book.handle_reads();
    let refusal = book.rebase(package).unwrap_err().to_string();
    assert!(
        refusal.contains("expected at most 64000 cell formats"),
        "{refusal}"
    );
    assert!(book.cell_style("Sheet1", at("B1")).unwrap().font.bold);
    assert_eq!(book.style_sheet().unwrap().len(), MAX_CELL_FORMATS);
    assert!(book.is_dirty());
    assert_eq!(book.handle_reads(), reads);
}

#[test]
fn inserted_rows_inherit_the_preceding_rows_cell_formats_and_height() {
    use yggdryl::excel::StylePatch;

    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Data").unwrap();
    sheet.set_cell(at("B2"), 0.25).unwrap();
    sheet.set_cell(at("C2"), 10.0).unwrap();
    sheet.set_row_height(1..2, Some(24.0)).unwrap();
    for (range, code) in [("B2", "0.00%"), ("C2", "\"$\"#,##0.00")] {
        workbook
            .set_style(
                "Data",
                &[range.parse().unwrap()],
                &StylePatch {
                    number_format: Some(code.into()),
                    ..StylePatch::default()
                },
            )
            .unwrap();
    }
    // Excel 16.0 build 20430 confirmed these formats and 24-point height
    // on the inserted row in the openpyxl structural exchange fixture.
    workbook.insert_rows("Data", 2, 1).unwrap();
    let sheet = workbook.sheet("Data").unwrap();
    assert_eq!(sheet.row_height(2), 24.0);
    for (previous, inserted) in [("B2", "B3"), ("C2", "C3")] {
        assert_eq!(sheet.scalar(at(inserted)), Scalar::Null);
        assert_eq!(
            workbook.cell_style("Data", at(inserted)).unwrap(),
            workbook.cell_style("Data", at(previous)).unwrap()
        );
    }
}

#[test]
fn inserting_rows_moves_every_row_from_the_point_down_and_refuses_pushing_one_off_the_grid() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Moves").unwrap();
    sheet.set_cell(at("A1"), "a").unwrap();
    sheet.set_cell(at("A2"), "b").unwrap();
    sheet.set_cell(at("B3"), "c").unwrap();
    workbook.insert_rows("Moves", 1, 2).unwrap();
    let sheet = workbook.sheet("Moves").unwrap();
    let references: Vec<String> = sheet
        .cells()
        .map(|cell| cell.reference().to_string())
        .collect();
    assert_eq!(references, ["A1", "A4", "B5"]);
    assert_eq!(sheet.scalar(at("A4")), Scalar::from("b"));
    assert_eq!(sheet.row(3).unwrap().index(), 3);
    assert_eq!(sheet.cell(at("B5")).unwrap().row(), 4);

    let unchanged = workbook.sheet("Moves").unwrap().clone();
    workbook.insert_rows("Moves", 0, 0).unwrap();
    workbook.remove_rows("Moves", 3..3).unwrap();
    assert_eq!(workbook.sheet("Moves").unwrap(), &unchanged);

    let mut full = Workbook::new();
    let sheet = full.add_sheet("Full").unwrap();
    sheet.set_cell(at("A1"), 1.0).unwrap();
    sheet.set_cell(at("A1048576"), 2.0).unwrap();
    let before = full.sheet("Full").unwrap().clone();
    assert_eq!(
        full.insert_rows("Full", 0, 1).unwrap_err().to_string(),
        "invalid record value at Full!A1048576: expected the moved rows to stay within 1048576 \
         rows, got A1048576 moving by 1"
    );
    assert_eq!(full.sheet("Full").unwrap(), &before);
    assert_eq!(
        full.insert_rows("Full", 5, u32::MAX)
            .unwrap_err()
            .to_string(),
        format!(
            "invalid record value at Full!rows: expected rows opening within the 1048576 of the \
             grid, got {} at 6",
            u32::MAX
        )
    );
    assert_eq!(
        full.remove_rows("Full", 0..1_048_577)
            .unwrap_err()
            .to_string(),
        "invalid record value at Full!rows: expected rows within the 1048576 of the grid, got up \
         to 1048577"
    );
    assert!(matches!(
        full.insert_rows("Missing", 0, 1),
        Err(Error::Absent { .. })
    ));
    assert_eq!(full.sheet("Full").unwrap(), &before);
}

#[test]
fn removing_rows_drops_them_and_moves_every_row_below_up() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Moves").unwrap();
    for (reference, text) in [("A1", "a"), ("A2", "b"), ("A3", "c"), ("B5", "d")] {
        sheet.set_cell(at(reference), text).unwrap();
    }
    workbook.remove_rows("Moves", 1..3).unwrap();
    let sheet = workbook.sheet("Moves").unwrap();
    let references: Vec<String> = sheet
        .cells()
        .map(|cell| cell.reference().to_string())
        .collect();
    assert_eq!(references, ["A1", "B3"]);
    assert_eq!(sheet.scalar(at("B3")), Scalar::from("d"));
    assert_eq!(sheet.dimension(), Some("A1:B3".parse().unwrap()));
    let unchanged = sheet.clone();
    workbook.remove_rows("Moves", 10..20).unwrap();
    assert_eq!(workbook.sheet("Moves").unwrap(), &unchanged);
}

/// The file spelling of the formula each of `cells` of the sheet `sheet`
/// holds.
fn spelled(workbook: &Workbook, sheet: &str, cells: &[&str]) -> Vec<String> {
    formulas(workbook.sheet(sheet).unwrap(), cells)
}

#[test]
fn inserting_a_row_moves_every_reference_the_package_states() {
    let mut workbook = Workbook::from_bytes(rich_package()).unwrap();
    workbook.insert_rows("Data", 2, 1).unwrap();
    // Every formula over the sheet, the shared group's dependents and the
    // array's included.
    assert_eq!(
        spelled(
            &workbook,
            "Report",
            &[
                "A1", "B1", "C1", "A2", "A3", "A4", "B2", "C2", "E2", "E3", "E4"
            ]
        ),
        [
            "SUM(Data!B2:B5)",
            "Rate*A1",
            "_xlfn.CONCAT(Data!A2,\"-\",Data!E2)",
            "Data!B2*Data!C2",
            "Data!B4*Data!C4",
            "Data!B5*Data!C5",
            "Data!B2:B5*2",
            "_xlfn._xlws.SORT(Data!B2:B5)",
            "A2*2",
            "A3*2",
            "A4*2",
        ]
    );
    let names: Vec<(String, String)> = workbook
        .defined_names()
        .map(|name| (name.name().to_owned(), name.text()))
        .collect();
    assert_eq!(
        names,
        [
            (
                "_xlnm._FilterDatabase".to_owned(),
                "Data!$A$1:$C$5".to_owned()
            ),
            ("_xlnm.Print_Area".to_owned(), "Report!$A$1:$E$4".to_owned()),
            ("Prices".to_owned(), "Data!$B$2:$B$5".to_owned()),
            ("Rate".to_owned(), "0.07".to_owned()),
        ]
    );
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("B4")), Scalar::from(2.25));
    assert!(data.is_row_hidden(5) && !data.is_row_hidden(4));
    assert_eq!(data.row_height(7), 30.0);
    assert_eq!(
        data.merges()
            .map(|merge| merge.to_string())
            .collect::<Vec<_>>(),
        ["A7:C7"]
    );
    let written = workbook.into_bytes().unwrap();
    let part = member_text(&written, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<autoFilter ref=\"A1:C5\"",
        "<sortState xmlns:xlrd2=\"http://schemas.microsoft.com/office/spreadsheetml/2017/richdata2\" ref=\"A2:C5\">",
        "<sortCondition ref=\"B2:B5\"/>",
        "<conditionalFormatting sqref=\"B2:B5\">",
        "sqref=\"C2:C5\"",
        "<hyperlink ref=\"A2\" r:id=\"rId1\"",
        "<hyperlink ref=\"A4\" location=\"Report!A1\"",
        "<xm:sqref>C2:C5</xm:sqref>",
        "<xm:f>Data!B2:C2</xm:f><xm:sqref>D2</xm:sqref>",
        "<mergeCell ref=\"A7:C7\"/>",
        "<row r=\"6\" s=\"1\" customFormat=\"1\" hidden=\"1\"",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
    let table = member_text(&written, "xl/tables/table1.xml");
    assert!(table.contains("ref=\"E1:F4\" totalsRowShown"), "{table}");
    assert!(table.contains("<autoFilter ref=\"E1:F4\"/>"), "{table}");
    let chart = member_text(&written, "xl/charts/chart1.xml");
    assert!(chart.contains("<c:f>Data!$B$2:$B$5</c:f>"), "{chart}");
    let vml = member_text(&written, "xl/drawings/vmlDrawing1.vml");
    // Excel moves the entire note box by its owner cell's delta. B2 is above
    // the inserted third row, so both corners of this native anchor stay put.
    assert!(vml.contains("2, 12, 0, 11, 4, 6, 5, 8</x:Anchor>"), "{vml}");
    assert!(vml.contains("<x:Row>1</x:Row>"), "{vml}");
    let comments = member_text(&written, "xl/comments1.xml");
    assert!(comments.contains("<comment ref=\"B2\""), "{comments}");
    // What the edit did not reach is the member the package stored.
    let before = stored_members(&rich_package());
    let after = stored_members(&written);
    // The chart is anchored `editAs="oneCell"` above the row: it keeps its
    // place and its size.
    for part in [
        "xl/worksheets/sheet3.xml",
        "xl/pivotCache/pivotCacheDefinition1.xml",
        "xl/pivotTables/pivotTable1.xml",
        "xl/comments1.xml",
        "xl/drawings/drawing1.xml",
    ] {
        assert_eq!(
            after.iter().find(|member| member.0 == part),
            before.iter().find(|member| member.0 == part),
            "{part}"
        );
    }
    // And the package opens again as the edit left it.
    let reopened = Workbook::from_bytes(written).unwrap();
    assert_eq!(
        spelled(&reopened, "Report", &["A1", "A4"]),
        ["SUM(Data!B2:B5)", "Data!B5*Data!C5"]
    );
}

#[test]
fn removing_rows_makes_what_named_them_ref_and_shrinks_what_they_cut() {
    let mut workbook = Workbook::from_bytes(rich_package()).unwrap();
    // Row 3 of `Data`: a data row of the filter, inside the ranges the
    // formulas, the rules and the chart name.
    workbook.remove_rows("Data", 2..3).unwrap();
    assert_eq!(
        spelled(&workbook, "Report", &["A1", "A2", "A3", "A4", "B2"]),
        [
            "SUM(Data!B2:B3)",
            "Data!B2*Data!C2",
            "Data!#REF!*Data!#REF!",
            "Data!B3*Data!C3",
            "Data!B2:B3*2",
        ]
    );
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("B3")), Scalar::from(0.5));
    let written = workbook.into_bytes().unwrap();
    let part = member_text(&written, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<autoFilter ref=\"A1:C3\"",
        "<conditionalFormatting sqref=\"B2:B3\">",
        "<hyperlink ref=\"A2\" r:id=\"rId1\"",
        "<mergeCell ref=\"A5:C5\"/>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
    // The hyperlink on the removed row goes with it.
    assert!(!part.contains("location=\"Report!A1\""), "{part}");
    let chart = member_text(&written, "xl/charts/chart1.xml");
    assert!(chart.contains("<c:f>Data!$B$2:$B$3</c:f>"), "{chart}");
}

#[test]
fn clearing_takes_out_what_it_names_and_keeps_the_rest() {
    use yggdryl::excel::{Clear, StylePatch};
    let book = || {
        let mut workbook = Workbook::new();
        workbook.add_sheet("S").unwrap();
        for (cell, text) in [("A1", "1/2/2024"), ("B1", "text"), ("C1", "=A1")] {
            workbook.set_entry("S", at(cell), text).unwrap();
        }
        let bold = StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        };
        workbook
            .set_style(
                "S",
                &["A1:C1".parse().unwrap(), "3:3".parse().unwrap()],
                &bold,
            )
            .unwrap();
        workbook
    };
    let mut all = book();
    all.clear("S", &["A1:B1".parse().unwrap()], Clear::All)
        .unwrap();
    assert!(all.sheet("S").unwrap().cell(at("A1")).is_none());
    assert!(all.sheet("S").unwrap().cell(at("C1")).is_some());
    let mut contents = book();
    contents
        .clear("S", &["A1:C1".parse().unwrap()], Clear::Contents)
        .unwrap();
    let sheet = contents.sheet("S").unwrap();
    assert!(sheet.cell(at("C1")).unwrap().formula().is_none());
    assert_eq!(sheet.scalar(at("B1")), Scalar::Null);
    assert!(contents.cell_style("S", at("B1")).unwrap().font.bold);
    let mut formats = book();
    formats
        .clear(
            "S",
            &["A1:C1".parse().unwrap(), "3:3".parse().unwrap()],
            Clear::Formats,
        )
        .unwrap();
    // The date is its serial again, the text keeps its value, the row its
    // style no more.
    assert_eq!(
        formats.sheet("S").unwrap().scalar(at("A1")),
        Scalar::from(45_293.0)
    );
    assert!(!formats.cell_style("S", at("B1")).unwrap().font.bold);
    assert_eq!(formats.sheet("S").unwrap().row_style(2), None);
}

#[test]
fn a_paste_puts_values_formulas_or_formats_as_asked_and_refuses_what_it_cannot_land() {
    use yggdryl::excel::{Paste, StylePatch};
    let mut workbook = Workbook::new();
    workbook.add_sheet("S").unwrap();
    workbook.set_entry("S", at("A1"), "7").unwrap();
    let host = at("B1");
    workbook
        .sheet_mut("S")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(host, 14.0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(yggdryl::excel::Formula::from_file("A1*2", host)),
        )
        .unwrap();
    let percent = StylePatch {
        number_format: Some("0%".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("S", &["D5:E5".parse().unwrap()], &percent)
        .unwrap();
    let source = "A1:B1".parse().unwrap();
    // Values: the formula's result, the cells' format kept.
    workbook
        .paste(("S", source), ("S", at("D5")), Paste::Values, false)
        .unwrap();
    assert_eq!(
        workbook.display_text("S", at("E5")).unwrap().unwrap().text,
        "1400%"
    );
    assert!(
        workbook
            .sheet("S")
            .unwrap()
            .cell(at("E5"))
            .unwrap()
            .formula()
            .is_none()
    );
    // Formulas: translated, the format kept.
    workbook
        .paste(("S", source), ("S", at("D5")), Paste::Formulas, false)
        .unwrap();
    assert_eq!(
        workbook.entry_text("S", at("E5")).unwrap().as_deref(),
        Some("=D5*2")
    );
    // Formats: the style alone.
    workbook
        .paste(
            ("S", "D5:D5".parse().unwrap()),
            ("S", at("A1")),
            Paste::Formats,
            false,
        )
        .unwrap();
    assert_eq!(
        workbook.display_text("S", at("A1")).unwrap().unwrap().text,
        "700%"
    );
    // All: a reference carried off the grid is `#REF!`.
    workbook
        .paste(("S", source), ("S", at("A1048576")), Paste::All, false)
        .unwrap();
    assert_eq!(
        workbook.entry_text("S", at("B1048576")).unwrap().as_deref(),
        Some("=A1048576*2")
    );
    workbook
        .paste(
            ("S", "B1".parse().unwrap()),
            ("S", at("A1")),
            Paste::All,
            false,
        )
        .unwrap();
    assert_eq!(
        workbook.entry_text("S", at("A1")).unwrap().as_deref(),
        Some("=#REF!*2")
    );
    for (outcome, expected) in [
        (
            workbook.paste(("S", source), ("S", at("XFD1")), Paste::All, false),
            "expected the pasted cells to land on the grid",
        ),
        (
            workbook.paste(("S", source), ("S", at("F1")), Paste::Values, true),
            "expected a cut to paste everything",
        ),
    ] {
        let error = outcome.unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    // Content under a merge shows only at its first cell: a paste over a
    // merge - part of it or all of it - lands on none.
    workbook
        .sheet_mut("S")
        .unwrap()
        .merge("G2:H2".parse().unwrap())
        .unwrap();
    for target in ["F2", "G2"] {
        let before = workbook.sheet("S").unwrap().clone();
        let error = workbook
            .paste(("S", source), ("S", at(target)), Paste::All, false)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected a paste over no merge, got G2:H2"),
            "{error}"
        );
        assert_eq!(workbook.sheet("S").unwrap(), &before);
    }
}

#[test]
fn a_paste_costs_the_cells_its_ranges_hold_never_the_grid() {
    use yggdryl::excel::Paste;
    // Whole columns and the whole grid, from a sheet of a few cells: a
    // paste walking every coordinate would take minutes.
    let book = || {
        let mut workbook = Workbook::new();
        workbook.add_sheet("From").unwrap();
        workbook.add_sheet("To").unwrap();
        for (cell, text) in [("A1", "id"), ("A900000", "7"), ("B5", "=A1&\"!\"")] {
            workbook.set_entry("From", at(cell), text).unwrap();
        }
        for (cell, text) in [("A2", "gone"), ("XFD1048576", "far"), ("C3", "kept")] {
            workbook.set_entry("To", at(cell), text).unwrap();
        }
        workbook
    };
    for (range, what) in [("A:B", Paste::All), ("A1:XFD1048576", Paste::All)] {
        let mut pasted = book();
        pasted
            .paste(
                ("From", range.parse().unwrap()),
                ("To", at("A1")),
                what,
                false,
            )
            .unwrap();
        let sheet = pasted.sheet("To").unwrap();
        assert_eq!(sheet.scalar(at("A900000")), Scalar::from(7.0));
        assert_eq!(sheet.scalar(at("A1")), Scalar::from("id"));
        assert!(sheet.cell(at("A2")).is_none());
        assert_eq!(
            pasted.entry_text("To", at("B5")).unwrap().as_deref(),
            Some("=A1&\"!\"")
        );
        // What the block holds nothing for is empty after, inside it; the
        // rest of the sheet is as it was.
        let whole = range == "A1:XFD1048576";
        assert_eq!(sheet.cell(at("XFD1048576")).is_none(), whole);
        assert_eq!(sheet.cell(at("C3")).is_none(), whole);
    }
    // Values keep the formats pasted over and clear what the block holds
    // no content for, cell by cell of what the ranges hold.
    let mut workbook = book();
    workbook
        .paste(
            ("From", "A:XFD".parse().unwrap()),
            ("To", at("A1")),
            Paste::Values,
            false,
        )
        .unwrap();
    let sheet = workbook.sheet("To").unwrap();
    assert!(sheet.cell(at("C3")).is_none());
    assert_eq!(sheet.scalar(at("B5")), Scalar::Null);
    // Formats walk the landing, which is bounded.
    let error = workbook
        .paste(
            ("From", "A:C".parse().unwrap()),
            ("To", at("A1")),
            Paste::Formats,
            false,
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("expected at most 2097152 cells formatted at once, got 3145728"),
        "{error}"
    );
}

#[test]
fn pasted_text_reads_excel_s_clipboard_quoting_cell_by_cell() {
    let mut workbook = Workbook::new();
    workbook.add_sheet("S").unwrap();
    workbook.set_entry("S", at("C3"), "gone").unwrap();
    let range = workbook
        .paste_text(
            "S",
            at("B2"),
            "a\t\"two\tcells\"\r\n12%\t\t\"line\nbreak\"\n=1+1",
        )
        .unwrap();
    assert_eq!(range.to_string(), "B2:D4");
    let sheet = workbook.sheet("S").unwrap();
    assert_eq!(sheet.scalar(at("C2")), Scalar::from("two\tcells"));
    assert_eq!(sheet.scalar(at("B3")), Scalar::from(0.12));
    // An empty field clears its cell.
    assert!(sheet.cell(at("C3")).is_none());
    assert_eq!(sheet.scalar(at("D3")), Scalar::from("line\nbreak"));
    assert_eq!(
        workbook.entry_text("S", at("B4")).unwrap().as_deref(),
        Some("=1+1")
    );
    assert!(workbook.paste_text("S", at("XFD1"), "a\tb").is_err());
}

#[test]
fn a_sort_orders_rows_as_excel_does_and_moves_each_whole() {
    use yggdryl::excel::SortKey;
    let mut workbook = Workbook::new();
    workbook.add_sheet("S").unwrap();
    for (row, (key, second)) in [
        ("name", "n"),
        ("pear", "1"),
        ("", "2"),
        ("10", "3"),
        ("TRUE", "4"),
        ("Apple", "5"),
        ("#N/A", "6"),
        ("2", "7"),
        ("FALSE", "8"),
    ]
    .iter()
    .enumerate()
    {
        let row = row as u32;
        if !key.is_empty() {
            workbook.set_entry("S", CellRef::new(row, 0), key).unwrap();
        }
        workbook
            .set_entry("S", CellRef::new(row, 1), second)
            .unwrap();
    }
    let host = at("C3");
    workbook.set_entry("S", host, "=B3*10").unwrap();
    let keys = [SortKey {
        column: 0,
        descending: false,
    }];
    workbook
        .sort("S", "A1:C9".parse().unwrap(), &keys, true)
        .unwrap();
    let column = |workbook: &Workbook, column: u32| -> Vec<String> {
        (1..9)
            .map(|row| {
                workbook
                    .display_text("S", CellRef::new(row, column))
                    .unwrap()
                    .map_or_else(String::new, |shown| shown.text.to_string())
            })
            .collect()
    };
    assert_eq!(
        column(&workbook, 0),
        ["2", "10", "Apple", "pear", "FALSE", "TRUE", "#N/A", ""]
    );
    assert_eq!(
        column(&workbook, 1),
        ["7", "3", "5", "1", "8", "4", "6", "2"]
    );
    // The formula moved with its row, translated to it.
    assert_eq!(
        workbook.entry_text("S", at("C9")).unwrap().as_deref(),
        Some("=B9*10")
    );
    // Descending, blanks stay last.
    let keys = [SortKey {
        column: 0,
        descending: true,
    }];
    workbook
        .sort("S", "A2:C9".parse().unwrap(), &keys, false)
        .unwrap();
    assert_eq!(
        column(&workbook, 0),
        ["#N/A", "TRUE", "FALSE", "pear", "Apple", "10", "2", ""]
    );
    let key = [SortKey {
        column: 5,
        descending: false,
    }];
    assert!(
        workbook
            .sort("S", "A1:C9".parse().unwrap(), &key, false)
            .unwrap_err()
            .to_string()
            .contains("expected sort keys inside the range's columns, got column F")
    );
}

#[test]
fn a_sheet_is_shown_and_hidden_and_the_last_visible_worksheet_stays() {
    let mut workbook = Workbook::new();
    workbook.add_sheet("One").unwrap();
    workbook.add_sheet("Two").unwrap();
    assert_eq!(workbook.sheet_state("Two"), Some(SheetState::Visible));
    workbook
        .set_sheet_state("Two", SheetState::VeryHidden)
        .unwrap();
    assert_eq!(workbook.sheet_state("two"), Some(SheetState::VeryHidden));
    assert_eq!(
        workbook
            .set_sheet_state("One", SheetState::Hidden)
            .unwrap_err()
            .to_string(),
        "invalid record value at One: expected another visible worksheet to be left, got none"
    );
    assert!(
        workbook
            .set_sheet_state("Missing", SheetState::Hidden)
            .is_err()
    );
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.sheet_state("Two"), Some(SheetState::VeryHidden));
}

/// Save and adopt, so removed members really leave the workbook's source.
fn save_removed_sheet_book(book: &mut Workbook) -> Vec<u8> {
    let package = book.into_package().unwrap();
    let bytes = package.as_bytes().to_vec();
    book.rebase(package).unwrap();
    bytes
}

/// A fresh ZIP may arrange local records differently while storing the same
/// members. Atomic refusals preserve the complete part map and logical state.
fn removal_member_map(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let archive = archive(bytes);
    archive
        .entries()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry.name().to_owned(),
                archive.read_member(entry.name()).unwrap(),
            )
        })
        .collect()
}

/// Metadata entries, including unknown attributes, independent of their order.
fn removal_metadata_entries(xml: &str, element: &[u8]) -> Vec<Vec<(String, String)>> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    let mut entries = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) | Event::Empty(start) if start.local_name().as_ref() == element => {
                let mut attributes: Vec<_> = start
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute.unwrap();
                        (
                            String::from_utf8(attribute.key.as_ref().to_vec()).unwrap(),
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                // A stale calculation chain is deliberately retired by every save.
                if attributes
                    .iter()
                    .any(|(_, value)| value == "/xl/calcChain.xml" || value.ends_with("/calcChain"))
                {
                    continue;
                }
                attributes.sort();
                entries.push(attributes);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    entries.sort();
    entries
}

/// Every member returns, including unmodelled descendants and registrations.
fn assert_removed_sheet_package_restored(before: &[u8], after: &[u8]) {
    let names = |bytes: &[u8]| {
        let mut names = member_names(bytes);
        names.retain(|name| name != "xl/calcChain.xml");
        names.sort();
        names
    };
    assert_eq!(names(before), names(after), "the complete package graph");
    let original = archive(before);
    let restored = archive(after);
    for name in names(before) {
        if [
            "[Content_Types].xml",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let expected = original.read_member(&name).unwrap();
        let actual = restored.read_member(&name).unwrap();
        if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
            assert_eq!(
                normalized(std::str::from_utf8(&expected).unwrap(), &REPORT_SHARED),
                normalized(std::str::from_utf8(&actual).unwrap(), &REPORT_SHARED),
                "{name} under the worksheet fidelity contract"
            );
        } else {
            assert_eq!(expected, actual, "{name}: sidecar bytes");
        }
    }
    for (part, element) in [
        ("[Content_Types].xml", b"Override".as_slice()),
        ("[Content_Types].xml", b"Default".as_slice()),
        ("xl/_rels/workbook.xml.rels", b"Relationship".as_slice()),
        ("xl/workbook.xml", b"sheet".as_slice()),
        ("xl/workbook.xml", b"pivotCache".as_slice()),
    ] {
        assert_eq!(
            removal_metadata_entries(&member_text(before, part), element),
            removal_metadata_entries(&member_text(after, part), element),
            "{part}: {} declarations",
            String::from_utf8_lossy(element)
        );
    }
}

#[test]
fn remove_sheet_undo_restores_every_related_part_and_registration_across_saves() {
    use yggdryl::excel::Edit;

    let parts: Vec<_> = rich_parts().into_iter().map(|(name, mut bytes)| {
        if name == "[Content_Types].xml" {
            bytes = std::str::from_utf8(&bytes).unwrap().replace("</Types>", concat!(
                "<Override PartName=\"/xl/worksheets/_rels/sheet1.xml.rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
                "<Override PartName=\"/xl/worksheets/_rels/sheet3.xml.rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/></Types>"
            )).into_bytes();
        }
        (name, bytes)
    }).collect();
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let source = package(&borrowed);
    for name in ["Data", "Report", "Pivot"] {
        let mut book = Workbook::from_bytes(source.clone()).unwrap();
        let before = save_removed_sheet_book(&mut book);
        let removed = book.apply(Edit::RemoveSheet { name: name.into() }).unwrap();
        let inverse = removed.inverse.unwrap();
        let deleted = save_removed_sheet_book(&mut book);
        assert!(
            !Workbook::from_bytes(deleted.clone())
                .unwrap()
                .sheet_names()
                .contains(&name)
        );
        let restored = book.apply(inverse).unwrap();
        let after = save_removed_sheet_book(&mut book);
        assert_removed_sheet_package_restored(&before, &after);
        let redone = book.apply(restored.inverse.unwrap()).unwrap();
        let deleted_again = save_removed_sheet_book(&mut book);
        assert_removed_sheet_package_restored(&deleted, &deleted_again);
        book.apply(redone.inverse.unwrap()).unwrap();
        assert_removed_sheet_package_restored(&before, &save_removed_sheet_book(&mut book));
    }
}

/// Report shares Data's drawing and has another pivot on Pivot's cache.
fn removal_shared_parts_package() -> Vec<u8> {
    let mut parts = rich_parts();
    let pivot = parts
        .iter()
        .find(|(name, _)| *name == "xl/pivotTables/pivotTable1.xml")
        .unwrap()
        .1
        .as_slice();
    let pivot = std::str::from_utf8(pivot)
        .unwrap()
        .replace("PivotTable1", "PivotTable2")
        .into_bytes();
    let pivot_rels = parts
        .iter()
        .find(|(name, _)| *name == "xl/pivotTables/_rels/pivotTable1.xml.rels")
        .unwrap()
        .1
        .clone();
    for (name, text) in &mut parts {
        if *name == "[Content_Types].xml" {
            *text = std::str::from_utf8(text).unwrap().replace("</Types>",
                "<Override PartName=\"/xl/pivotTables/pivotTable2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml\"/></Types>"
            ).into_bytes();
        } else if *name == "xl/worksheets/sheet2.xml" {
            *text = std::str::from_utf8(text).unwrap().replace("</worksheet>",
                "<drawing r:id=\"rId1\"/><pivotTableParts count=\"1\"><pivotTablePart r:id=\"rId2\"/></pivotTableParts></worksheet>"
            ).into_bytes();
        }
    }
    parts.push(("xl/pivotTables/pivotTable2.xml", pivot));
    parts.push(("xl/pivotTables/_rels/pivotTable2.xml.rels", pivot_rels));
    parts.push((
        "xl/worksheets/_rels/sheet2.xml.rels",
        relationships(&[
            (
                "rId1",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing",
                "../drawings/drawing1.xml",
            ),
            (
                "rId2",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable",
                "../pivotTables/pivotTable2.xml",
            ),
        ])
        .into_bytes(),
    ));
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    package(&borrowed)
}

#[test]
fn remove_sheet_undo_retains_shared_dependencies_with_and_without_intermediate_saves() {
    use yggdryl::excel::Edit;

    for removed in [["Data", "Report"], ["Pivot", "Report"]] {
        for between in [false, true] {
            let mut book = Workbook::from_bytes(removal_shared_parts_package()).unwrap();
            let before = save_removed_sheet_book(&mut book);
            let first = book
                .apply(Edit::RemoveSheet {
                    name: removed[0].into(),
                })
                .unwrap();
            if between {
                save_removed_sheet_book(&mut book);
            }
            let second = book
                .apply(Edit::RemoveSheet {
                    name: removed[1].into(),
                })
                .unwrap();
            let deleted = save_removed_sheet_book(&mut book);
            let vanished = if removed[0] == "Data" {
                "xl/charts/chart1.xml"
            } else {
                "xl/pivotCache/pivotCacheRecords1.xml"
            };
            assert!(
                !member_names(&deleted).iter().any(|name| name == vanished),
                "{vanished}"
            );
            book.apply(second.inverse.unwrap()).unwrap();
            save_removed_sheet_book(&mut book);
            book.apply(first.inverse.unwrap()).unwrap();
            assert_removed_sheet_package_restored(&before, &save_removed_sheet_book(&mut book));
        }
    }
}

#[test]
fn remove_sheet_undo_keeps_prior_unsaved_part_edits_and_counts_retained_bytes() {
    use yggdryl::excel::Edit;

    let mut book = Workbook::from_bytes(rich_package()).unwrap();
    book.rename_sheet("Data", "Trades").unwrap();
    let expected = book.into_bytes().unwrap();
    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Trades".into(),
        })
        .unwrap();
    let inverse = removed.inverse.unwrap();
    let deleted = save_removed_sheet_book(&mut book);
    let retained_bytes: usize = member_names(&expected)
        .iter()
        .filter(|name| {
            name.as_str() != "xl/calcChain.xml" && !member_names(&deleted).contains(name)
        })
        .map(|name| archive(&expected).read_member(name).unwrap().len())
        .sum();
    assert!(
        inverse.byte_size() >= retained_bytes,
        "journal {} omits retained package bytes {retained_bytes}",
        inverse.byte_size()
    );
    book.apply(inverse).unwrap();
    assert_removed_sheet_package_restored(&expected, &save_removed_sheet_book(&mut book));
}

#[test]
fn remove_sheet_undo_refuses_a_reused_part_without_changing_the_workbook() {
    use yggdryl::excel::Edit;

    let mut book = Workbook::from_bytes(three_sheets_with(&[])).unwrap();
    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Sheet3".into(),
        })
        .unwrap();
    save_removed_sheet_book(&mut book);
    book.add_sheet("Replacement")
        .unwrap()
        .set_cell(at("A1"), 99)
        .unwrap();
    let before = save_removed_sheet_book(&mut book);
    // The old highest part is now another tab's, even though sheetId/key grew.
    assert!(member_text(&before, "xl/worksheets/sheet3.xml").contains("99"));
    let error = book.apply(removed.inverse.unwrap()).unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }), "{error}");
    assert_eq!(book.sheet_names(), ["Sheet1", "Sheet2", "Replacement"]);
    assert!(!book.is_dirty());
    assert_eq!(
        removal_member_map(&before),
        removal_member_map(&book.into_bytes().unwrap()),
        "a refused inverse is atomic"
    );
}

#[test]
fn remove_sheet_undo_refuses_a_reused_relationship_without_changing_the_workbook() {
    use yggdryl::excel::Edit;

    let original = three_sheets_with(&[("xl/custom.xml", "<kept/>")]);
    let parts: Vec<_> = member_names(&original).into_iter().map(|name| {
        let mut text = member_text(&original, &name);
        if name == "xl/workbook.xml" {
            text = text.replace("r:id=\"rId1\"", "r:id=\"rId9\"");
        } else if name == "xl/_rels/workbook.xml.rels" {
            text = text.replace("Id=\"rId1\"", "Id=\"rId9\"").replace("</Relationships>",
                "<Relationship Id=\"rId8\" Type=\"urn:example:kept\" Target=\"custom.xml\"/></Relationships>"
            );
        }
        (name, text)
    }).collect();
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let mut book = Workbook::from_bytes(package(&borrowed)).unwrap();
    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Sheet1".into(),
        })
        .unwrap();
    save_removed_sheet_book(&mut book);
    book.add_sheet("Replacement")
        .unwrap()
        .set_cell(at("A1"), 99)
        .unwrap();
    let before = save_removed_sheet_book(&mut book);
    assert!(!member_names(&before).contains(&"xl/worksheets/sheet1.xml".to_owned()));
    assert!(member_text(&before, "xl/_rels/workbook.xml.rels")
        .contains("Id=\"rId9\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet4.xml\""));
    let error = book.apply(removed.inverse.unwrap()).unwrap_err();
    assert!(matches!(error, Error::Conflict { .. }), "{error}");
    assert_eq!(book.sheet_names(), ["Sheet2", "Sheet3", "Replacement"]);
    assert!(!book.is_dirty());
    assert_eq!(
        removal_member_map(&before),
        removal_member_map(&book.into_bytes().unwrap()),
        "a refused inverse is atomic"
    );
}

#[test]
fn remove_sheet_undo_keeps_the_namespace_of_a_cache_registration() {
    use quick_xml::events::Event;
    use quick_xml::name::{Namespace, ResolveResult};
    use yggdryl::excel::Edit;

    let mut parts = rich_parts();
    let workbook = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/workbook.xml")
        .unwrap()
        .1;
    let anchor = "<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId8\"/></pivotCaches>";
    let workbook_text = std::str::from_utf8(workbook).unwrap();
    assert_eq!(workbook_text.matches(anchor).count(), 1);
    *workbook = workbook_text.replace(anchor, &format!(
        "<pivotCaches xmlns:pc=\"{R_NS}\"><pivotCache cacheId=\"1\" pc:id=\"rId8\"/></pivotCaches>"
    )).into_bytes();
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let mut book = Workbook::from_bytes(package(&borrowed)).unwrap();
    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Pivot".into(),
        })
        .unwrap();
    let deleted = save_removed_sheet_book(&mut book);
    assert!(!member_text(&deleted, "xl/workbook.xml").contains("pivotCaches"));
    book.apply(removed.inverse.unwrap()).unwrap();
    let restored = save_removed_sheet_book(&mut book);
    let workbook = member_text(&restored, "xl/workbook.xml");
    let mut reader = quick_xml::NsReader::from_str(&workbook);
    let mut found = false;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) | Event::Empty(start)
                if start.local_name().as_ref() == b"pivotCache" =>
            {
                let id = start
                    .attributes()
                    .map(Result::unwrap)
                    .find(|attribute| attribute.key.local_name().as_ref() == b"id")
                    .unwrap();
                assert_eq!(
                    reader.resolver().resolve_attribute(id.key).0,
                    ResolveResult::Bound(Namespace(R_NS.as_bytes())),
                    "{workbook}"
                );
                assert_eq!(id.value.as_ref(), b"rId8");
                found = true;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert!(found, "the restored cache registration is present");
}

#[test]
fn remove_sheet_undo_escapes_inherited_namespace_quotes_and_entities_once() {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;
    use yggdryl::excel::Edit;

    let mut parts = rich_parts();
    let workbook = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/workbook.xml")
        .unwrap()
        .1;
    let anchor = "<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId8\"/></pivotCaches>";
    let workbook_text = std::str::from_utf8(workbook).unwrap();
    assert_eq!(workbook_text.matches(anchor).count(), 1);
    *workbook = workbook_text.replace(anchor,
        "<pivotCaches xmlns:vendor='urn:vendor:\"quoted\"?a=1&amp;b=2'><pivotCache cacheId=\"1\" r:id=\"rId8\" vendor:flag=\"yes\"/></pivotCaches>").into_bytes();
    let borrowed: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let mut book = Workbook::from_bytes(package(&borrowed)).unwrap();
    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Pivot".into(),
        })
        .unwrap();
    save_removed_sheet_book(&mut book);
    book.apply(removed.inverse.unwrap()).unwrap();
    let restored = save_removed_sheet_book(&mut book);
    let workbook = member_text(&restored, "xl/workbook.xml");
    let mut reader = quick_xml::NsReader::from_str(&workbook);
    let mut found = false;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) | Event::Empty(start)
                if start.local_name().as_ref() == b"pivotCache" =>
            {
                let flag = start
                    .attributes()
                    .map(Result::unwrap)
                    .find(|attribute| attribute.key.local_name().as_ref() == b"flag")
                    .unwrap();
                let ResolveResult::Bound(namespace) =
                    reader.resolver().resolve_attribute(flag.key).0
                else {
                    panic!("the unknown attribute lost its namespace: {workbook}");
                };
                assert_eq!(
                    quick_xml::escape::unescape(std::str::from_utf8(namespace.as_ref()).unwrap())
                        .unwrap(),
                    "urn:vendor:\"quoted\"?a=1&b=2"
                );
                assert_eq!(flag.value.as_ref(), b"yes");
                found = true;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert!(found, "the restored unknown attribute is present");
}

#[test]
fn remove_sheet_undo_refuses_missing_shared_dependencies_atomically() {
    use yggdryl::excel::{Clock, Edit};

    for settled in [false, true] {
        let mut book = Workbook::from_bytes(removal_shared_parts_package())
            .unwrap()
            .with_clock(Clock::fixed(0, Timezone::UTC, 73));
        let report_key = book.sheet_key("Report").unwrap();
        // A settled baseline makes the first removal's receipt contain no
        // incidental Report cache replacements (including its volatile TODAY).
        // The cold arm retains the new, earlier receipt-owner refusal as well.
        if settled {
            book.calculate_all().unwrap();
        }
        let original = save_removed_sheet_book(&mut book);
        let first = book
            .apply(Edit::RemoveSheet {
                name: "Pivot".into(),
            })
            .unwrap();
        let second = book
            .apply(Edit::RemoveSheet {
                name: "Report".into(),
            })
            .unwrap();
        let before = save_removed_sheet_book(&mut book);
        assert!(
            !member_names(&before).contains(&"xl/pivotCache/pivotCacheDefinition1.xml".to_owned())
        );
        let inverse = first.inverse.unwrap();
        let error = book.apply(inverse.clone()).unwrap_err();
        if settled {
            assert!(matches!(error, Error::Conflict { .. }), "{error}");
            assert!(
                error.to_string().contains("pivotCacheDefinition1.xml"),
                "{error}"
            );
        } else {
            // The receipt must restore Report's old caches before its paired
            // sheet inverse; that owner is absent after the second removal.
            match &error {
                Error::Absent { expected, path } => {
                    assert_eq!(*expected, "worksheet");
                    assert_eq!(path.as_str(), report_key.as_u32().to_string());
                }
                _ => panic!("expected missing receipt worksheet, got {error}"),
            }
        }
        assert_eq!(book.sheet_names(), ["Data"]);
        assert!(!book.is_dirty());
        assert_eq!(
            removal_member_map(&before),
            removal_member_map(&book.into_bytes().unwrap())
        );
        // The refused inverse is still usable once the other inverse restores
        // the shared cache which its removal, as the final owner, retained.
        book.apply(second.inverse.unwrap()).unwrap();
        save_removed_sheet_book(&mut book);
        book.apply(inverse).unwrap();
        assert_removed_sheet_package_restored(&original, &save_removed_sheet_book(&mut book));
    }
}

#[test]
fn table_formulas_refuse_two_worksheet_owners_atomically() {
    let table = table_formula_part(
        "E2:F6",
        "E2:F5",
        "Inputs!B3+Costs[[#This Row],[Cost]]",
        "SUM(Inputs!B3:B5)+Costs[Cost]",
    );
    let mut workbook = sheets_book(
        &[
            (
                "Data",
                sheet(
                    "",
                    "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                ),
                &[("rIdTable", "table", "../tables/table1.xml")],
            ),
            (
                "Other",
                sheet(
                    "",
                    "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                ),
                &[("rIdTable", "table", "../tables/table1.xml")],
            ),
            ("Inputs", sheet("", ""), &[]),
        ],
        &[],
        &[("xl/tables/table1.xml", table.clone())],
    );
    let error = workbook.rename_sheet("Inputs", "Renamed").unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at xl/tables/table1.xml: expected one worksheet owning the table part, got Data and Other"
    );
    assert_eq!(workbook.sheet_names(), ["Data", "Other", "Inputs"]);
    assert!(!workbook.is_dirty());
    assert_eq!(member(&workbook, "xl/tables/table1.xml"), table);
}

#[test]
fn table_owner_uses_only_the_listed_relationship_id() {
    let table = table_formula_part("E2:F6", "E2:F5", "Inputs!B3", "SUM(Inputs!B3:B5)");
    let other = table
        .replacen("id=\"1\"", "id=\"2\"", 1)
        .replace("\"Costs\"", "\"OtherCosts\"");
    let mut workbook = sheets_book(
        &[
            (
                "Data",
                sheet(
                    "",
                    "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                ),
                &[("rIdTable", "table", "../tables/table1.xml")],
            ),
            (
                "Other",
                sheet(
                    "",
                    "<tableParts count=\"1\"><tablePart xmlns:x=\"urn:unrelated\" x:id=\"rIdOrphan\" r:id=\"rIdOwn\"/></tableParts>",
                ),
                &[
                    ("rIdOwn", "table", "../tables/table2.xml"),
                    ("rIdOrphan", "table", "../tables/table1.xml"),
                ],
            ),
            ("Inputs", sheet("", ""), &[]),
        ],
        &[],
        &[
            ("xl/tables/table1.xml", table.clone()),
            ("xl/tables/table2.xml", other.clone()),
        ],
    );
    let relationships = member(&workbook, "xl/worksheets/_rels/sheet2.xml.rels");
    workbook.rename_sheet("Inputs", "Renamed").unwrap();
    assert_eq!(workbook.sheet_names(), ["Data", "Other", "Renamed"]);
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table.replace("Inputs!", "Renamed!")
    );
    assert_eq!(
        member(&workbook, "xl/tables/table2.xml"),
        other.replace("Inputs!", "Renamed!")
    );
    assert_eq!(
        member(&workbook, "xl/worksheets/_rels/sheet2.xml.rels"),
        relationships
    );
}

#[test]
fn table_owner_ignores_an_unlisted_table_at_a_cut_landing() {
    let moving = stationary_cut_table("Costs", 1, "E2:F6");
    let orphan = stationary_cut_table("Unlisted", 2, "J8:K12");
    let mut workbook = book(
        &sheet(
            "<row r=\"3\"><c r=\"E3\"><v>7</v></c></row>",
            "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
        ),
        &[
            ("rIdTable", "table", "../tables/table1.xml"),
            ("rIdOrphan", "table", "../tables/table2.xml"),
        ],
        &[
            ("xl/tables/table1.xml", &moving),
            ("xl/tables/table2.xml", &orphan),
        ],
    );
    workbook
        .paste(
            ("Sheet1", "E2:F6".parse().unwrap()),
            ("Sheet1", at("J8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        stationary_cut_table("Costs", 1, "J8:K12")
    );
    assert_eq!(member(&workbook, "xl/tables/table2.xml"), orphan);
    assert!(workbook.sheet("Sheet1").unwrap().cell(at("E3")).is_none());
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("J9")),
        7.0.into()
    );
    // The band check's already-read parts must use the same active IDs.
    workbook.insert_rows("Sheet1", 0, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        stationary_cut_table("Costs", 1, "J9:K13")
    );
    assert_eq!(member(&workbook, "xl/tables/table2.xml"), orphan);
}

#[test]
fn table_owner_resolves_scoped_and_encoded_namespaces() {
    let canonical = sheet(
        "",
        "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
    );
    let cases = [
        (
            "Strict",
            canonical
                .replace(NS, "http://purl.oclc.org/ooxml/spreadsheetml/main")
                .replace(
                    R_NS,
                    "http://purl.oclc.org/ooxml/officeDocument/relationships",
                ),
        ),
        (
            "alternate and local prefixes",
            format!(
                "<s:worksheet xmlns:s=\"{NS}\" xmlns:r=\"urn:custom\"><s:sheetData/><s:tableParts count=\"1\"><s:tablePart xmlns:link=\"{R_NS}\" r:id=\"rIdOrphan\" link:id=\"rIdTable\"/></s:tableParts></s:worksheet>"
            ),
        ),
        (
            "encoded namespace names",
            canonical
                .replace(NS, &NS.replace("2006", "&#x32;006"))
                .replace(R_NS, &R_NS.replace("relationships", "relation&#115;hips")),
        ),
    ];
    for (case, worksheet) in cases {
        let table = table_formula_part("E2:F6", "E2:F5", "Other!B3", "SUM(Other!B3:B5)");
        let orphan = table
            .replacen("id=\"1\"", "id=\"2\"", 1)
            .replace("\"Costs\"", "\"Unlisted\"");
        let mut workbook = sheets_book(
            &[
                (
                    "Data",
                    worksheet,
                    &[
                        ("rIdTable", "table", "../tables/table1.xml"),
                        ("rIdOrphan", "table", "../tables/table2.xml"),
                    ],
                ),
                ("Other", sheet("", ""), &[]),
            ],
            &[],
            &[
                ("xl/tables/table1.xml", table.clone()),
                ("xl/tables/table2.xml", orphan.clone()),
            ],
        );
        let relationships = member(&workbook, "xl/worksheets/_rels/sheet1.xml.rels");
        workbook.rename_sheet("Other", "Renamed").unwrap();
        assert_eq!(
            member(&workbook, "xl/tables/table1.xml"),
            table.replace("Other!", "Renamed!"),
            "{case}"
        );
        assert_eq!(member(&workbook, "xl/tables/table2.xml"), orphan, "{case}");
        assert_eq!(
            member(&workbook, "xl/worksheets/_rels/sheet1.xml.rels"),
            relationships,
            "{case}"
        );
    }
}

#[test]
fn active_table_relationship_refuses_a_foreign_namespace_table_part_atomically() {
    let foreign = table_formula_part("E2:F6", "E2:F5", "Data!B3", "SUM(Data!B3:B5)")
        .replace(NS, "urn:custom");
    refuse_table_part_without_root(foreign);
}

#[test]
fn active_table_relationship_refuses_a_nested_table_under_a_foreign_root_atomically() {
    let table = table_formula_part("E2:F6", "E2:F5", "Data!B3", "SUM(Data!B3:B5)");
    refuse_table_part_without_root(format!("<wrapper xmlns=\"urn:custom\">{table}</wrapper>"));
}

fn refuse_table_part_without_root(foreign: String) {
    let mut workbook = sheets_book(
        &[(
            "Data",
            sheet(
                "",
                "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
            ),
            &[("rIdTable", "table", "../tables/table1.xml")],
        )],
        &[],
        &[("xl/tables/table1.xml", foreign.clone())],
    );
    let before = table_member_map(&workbook);
    let error = workbook.insert_rows("Data", 0, 1).unwrap_err().to_string();
    assert!(error.contains("xl/tables/table1.xml"), "{error}");
    assert_eq!(table_member_map(&workbook), before);
    assert!(!workbook.is_dirty());
}

#[test]
fn table_owner_ignores_foreign_shadowed_and_nested_elements() {
    for (after, blocking) in [
        (
            "<x:tableParts xmlns:x=\"urn:custom\"><x:tablePart r:id=\"rIdTable\"/></x:tableParts>",
            Some("x:tableParts"),
        ),
        (
            "<tableParts><x:tablePart xmlns:x=\"urn:custom\" r:id=\"rIdTable\"/></tableParts>",
            None,
        ),
        (
            "<tableParts><tablePart xmlns=\"urn:custom\" r:id=\"rIdTable\"/></tableParts>",
            None,
        ),
        (
            "<tableParts xmlns=\"urn:custom\"><tablePart r:id=\"rIdTable\"/></tableParts>",
            Some("tableParts"),
        ),
        (
            "<tableParts><extLst><tablePart r:id=\"rIdTable\"/></extLst></tableParts>",
            None,
        ),
    ] {
        for operation in ["insert", "rename", "cut"] {
            let orphan = table_formula_part("E2:F6", "E2:F5", "Other!B3", "SUM(Other!B3:B5)");
            let mut workbook = sheets_book(
                &[
                    (
                        "Data",
                        sheet("<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>", after),
                        &[("rIdTable", "table", "../tables/table1.xml")],
                    ),
                    ("Other", sheet("", ""), &[]),
                ],
                &[],
                &[("xl/tables/table1.xml", orphan.clone())],
            );
            let relationships = member(&workbook, "xl/worksheets/_rels/sheet1.xml.rels");
            let before = table_member_map(&workbook);
            let result = match operation {
                "insert" => workbook.insert_rows("Data", 0, 1),
                "rename" => workbook.rename_sheet("Other", "Renamed"),
                "cut" => workbook
                    .paste(
                        ("Data", "A1".parse().unwrap()),
                        ("Data", at("C3")),
                        Paste::All,
                        true,
                    )
                    .map(|_| ()),
                _ => unreachable!(),
            };
            if let Some(element) = blocking.filter(|_| operation != "rename") {
                // A foreign root child is unknown carried XML, even when
                // its local name resembles tableParts. Namespace validation
                // now blocks moving its cells rather than calling it modelled.
                let error = result.unwrap_err();
                assert!(
                    matches!(&error, Error::Unsupported { filesystem, .. }
                    if filesystem == &format!("xl/worksheets/sheet1.xml#{element}")),
                    "{error}"
                );
                assert_eq!(table_member_map(&workbook), before);
                assert!(!workbook.is_dirty());
            } else {
                result.unwrap();
            }
            assert_eq!(
                member(&workbook, "xl/tables/table1.xml"),
                orphan,
                "{operation}: {after}"
            );
            assert_eq!(
                member(&workbook, "xl/worksheets/_rels/sheet1.xml.rels"),
                relationships,
                "{operation}: {after}"
            );
            assert!(
                member(&workbook, "xl/worksheets/sheet1.xml").contains(after),
                "{operation}: {after}"
            );
        }
    }
}

fn assert_table_owner_refusal(
    after: &str,
    relationships: &[Related<'_>],
    location: &str,
    ambiguous: bool,
) {
    for operation in ["insert", "rename", "cut"] {
        let table = table_formula_part("E2:F6", "E2:F5", "Other!B3", "SUM(Other!B3:B5)");
        let other = table
            .replacen("id=\"1\"", "id=\"2\"", 1)
            .replace("\"Costs\"", "\"OtherCosts\"");
        let mut workbook = sheets_book(
            &[
                (
                    "Data",
                    sheet("<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>", after),
                    relationships,
                ),
                ("Other", sheet("", ""), &[]),
            ],
            &[],
            &[
                ("xl/tables/table1.xml", table),
                ("xl/tables/table2.xml", other),
            ],
        );
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let result = match operation {
            "insert" => workbook.insert_rows("Data", 0, 1),
            "rename" => workbook.rename_sheet("Other", "Renamed"),
            "cut" => workbook
                .paste(
                    ("Data", "A1".parse().unwrap()),
                    ("Data", at("C3")),
                    Paste::All,
                    true,
                )
                .map(drop),
            _ => unreachable!(),
        };
        match result.unwrap_err() {
            Error::InvalidRecord { path, reason } => {
                assert!(path.contains("xl/worksheets/sheet1.xml"), "{path}");
                assert!(path.contains(location), "{path}");
                let reason = reason.to_ascii_lowercase();
                assert!(
                    reason.contains("relationship") || reason.contains("id"),
                    "{reason}"
                );
                if ambiguous {
                    assert!(
                        reason.contains("duplicate") || reason.contains("ambiguous"),
                        "{reason}"
                    );
                } else {
                    assert!(
                        reason.contains("missing") || reason.contains("absent"),
                        "{reason}"
                    );
                }
            }
            error => panic!("expected a located table ownership refusal, got {error}"),
        }
        assert_eq!(workbook.sheet_names(), ["Data", "Other"]);
        assert!(!workbook.is_dirty());
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.sheet("Data").unwrap().scalar(at("A1")), 7.0.into());
        assert_eq!(table_member_map(&workbook), before, "{operation}: {after}");
    }
}

#[test]
fn table_owner_refuses_duplicate_active_relationship_ids_atomically() {
    for second in ["../tables/table1.xml", "../tables/table2.xml"] {
        assert_table_owner_refusal(
            "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
            &[
                ("rIdTable", "table", "../tables/table1.xml"),
                ("rIdTable", "table", second),
            ],
            "rIdTable",
            true,
        );
    }
}

#[test]
fn table_owner_refuses_missing_relationship_namespace_ids_atomically() {
    for attributes in [
        "",
        " id=\"rIdTable\"",
        " xmlns:x=\"urn:custom\" x:id=\"rIdTable\"",
        " xmlns:r=\"urn:custom\" r:id=\"rIdTable\"",
    ] {
        assert_table_owner_refusal(
            &format!("<tableParts count=\"1\"><tablePart{attributes}/></tableParts>"),
            &[("rIdTable", "table", "../tables/table1.xml")],
            "tablePart",
            false,
        );
    }
    assert_table_owner_refusal(
        "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
        &[],
        "rIdTable",
        false,
    );
}

#[test]
fn table_owner_refuses_unresolved_or_wrong_kind_targets_atomically() {
    for (case, kind, present, id, mode) in [
        ("missing part", "table", false, "rIdTable", ""),
        ("wrong kind", "chart", true, "rIdTable", ""),
        ("missing relationship", "table", true, "rIdOther", ""),
        (
            "external target",
            "table",
            true,
            "rIdTable",
            " TargetMode=\"External\"",
        ),
    ] {
        for operation in ["insert", "rename", "cut"] {
            let mut parts = if present {
                vec![(
                    "xl/tables/table1.xml",
                    table_formula_part("E2:F6", "E2:F5", "Other!B3", "SUM(Other!B3:B5)"),
                )]
            } else {
                Vec::new()
            };
            parts.push((
                "xl/worksheets/_rels/sheet1.xml.rels",
                format!(
                    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"../tables/table1.xml\"{mode}/></Relationships>"
                ),
            ));
            let mut workbook = sheets_book(
                &[
                    (
                        "Data",
                        sheet(
                            "<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>",
                            "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                        ),
                        &[],
                    ),
                    ("Other", sheet("", ""), &[]),
                ],
                &[],
                &parts,
            );
            let before = table_member_map(&workbook);
            let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
            let result = match operation {
                "insert" => workbook.insert_rows("Data", 0, 1),
                "rename" => workbook.rename_sheet("Other", "Renamed"),
                "cut" => workbook
                    .paste(
                        ("Data", "A1".parse().unwrap()),
                        ("Data", at("C3")),
                        Paste::All,
                        true,
                    )
                    .map(drop),
                _ => unreachable!(),
            };
            match result.unwrap_err() {
                Error::InvalidRecord { path, reason } => {
                    assert!(path.contains("xl/worksheets/sheet1.xml"), "{path}");
                    assert!(path.contains("rIdTable"), "{path}");
                    let reason = reason.to_ascii_lowercase();
                    match case {
                        "wrong kind" => {
                            assert!(reason.contains("table"), "{reason}");
                            assert!(reason.contains("chart"), "{reason}");
                        }
                        "external target" => assert!(reason.contains("external"), "{reason}"),
                        _ => {
                            assert!(
                                reason.contains("missing") || reason.contains("absent"),
                                "{reason}"
                            );
                            let expected = if case == "missing part" {
                                "xl/tables/table1.xml"
                            } else {
                                "relationship"
                            };
                            assert!(reason.contains(expected), "{reason}");
                        }
                    }
                }
                error => panic!("expected a located table relationship refusal, got {error}"),
            }
            assert_eq!(workbook.sheet_names(), ["Data", "Other"]);
            assert!(!workbook.is_dirty());
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert_eq!(workbook.sheet("Data").unwrap().scalar(at("A1")), 7.0.into());
            assert_eq!(
                table_member_map(&workbook),
                before,
                "case={case}, operation={operation}"
            );
        }
    }
}

fn cross_table_reopen(parts: &std::collections::BTreeMap<String, Vec<u8>>) -> Workbook {
    let parts: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (name.as_str(), std::str::from_utf8(bytes).unwrap()))
        .collect();
    Workbook::from_bytes(package(&parts)).unwrap()
}

/// Resolve membership IDs in their actual inherited namespace, independent
/// of the prefix or relationship ID the transfer chose.
fn cross_table_ids(xml: &[u8]) -> Vec<String> {
    use quick_xml::events::Event;
    use quick_xml::name::{Namespace, ResolveResult};

    let mut reader = quick_xml::NsReader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut ids = Vec::new();
    let mut counts = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer).unwrap() {
            Event::Start(start) | Event::Empty(start) => {
                if start.local_name().as_ref() == b"tableParts" {
                    let count = start
                        .attributes()
                        .map(Result::unwrap)
                        .find(|attribute| attribute.key.as_ref() == b"count")
                        .unwrap();
                    counts.push(
                        count
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap()
                            .parse::<usize>()
                            .unwrap(),
                    );
                } else if start.local_name().as_ref() == b"tablePart" {
                    let listed: Vec<_> = start
                        .attributes()
                        .map(Result::unwrap)
                        .filter(|attribute| {
                            let (namespace, local) =
                                reader.resolver().resolve_attribute(attribute.key);
                            local.as_ref() == b"id"
                                && namespace == ResolveResult::Bound(Namespace(R_NS.as_bytes()))
                        })
                        .map(|attribute| {
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned()
                        })
                        .collect();
                    assert_eq!(
                        listed.len(),
                        1,
                        "each membership needs one resolved relationship ID"
                    );
                    ids.extend(listed);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if ids.is_empty() {
        assert!(
            counts.is_empty(),
            "the last table leaves no empty tableParts container"
        );
    } else {
        assert_eq!(counts, [ids.len()]);
    }
    ids
}

fn cross_table_relationships(xml: &[u8]) -> Vec<std::collections::BTreeMap<String, String>> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut rows = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) | Event::Empty(start)
                if start.local_name().as_ref() == b"Relationship" =>
            {
                rows.push(
                    start
                        .attributes()
                        .map(|attribute| {
                            let attribute = attribute.unwrap();
                            (
                                std::str::from_utf8(attribute.key.as_ref())
                                    .unwrap()
                                    .to_owned(),
                                attribute
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .unwrap()
                                    .into_owned(),
                            )
                        })
                        .collect(),
                );
            }
            Event::Eof => break,
            _ => {}
        }
    }
    rows
}

fn cross_table_expected() -> String {
    cut_table_part(
        "Costs",
        11,
        ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
        [
            "J9+$J$9+Data!$A$1+Other!B3+Costs[[#This Row],[Cost]]",
            "SUM(J9:J11)+SUM($J$9:$J$11)+Costs[Cost]+Data!$A$1",
        ],
    )
}

fn assert_cross_table_owner(
    workbook: &Workbook,
    destination: &str,
    part: &str,
    rels: &str,
    target: &str,
) -> String {
    let parts = table_member_map(workbook);
    assert_eq!(
        std::str::from_utf8(&parts["xl/tables/table1.xml"]).unwrap(),
        cross_table_expected()
    );
    assert_eq!(
        parts
            .keys()
            .filter(|name| name.starts_with("xl/tables/"))
            .count(),
        1
    );
    assert!(cross_table_ids(&parts["xl/worksheets/sheet1.xml"]).is_empty());
    let ids = cross_table_ids(&parts[part]);
    assert_eq!(ids.len(), 1);
    if let Some(source) = parts.get("xl/worksheets/_rels/sheet1.xml.rels") {
        assert!(
            cross_table_relationships(source)
                .iter()
                .all(|entry| entry["Type"] != format!("{R_NS}/table"))
        );
    }
    let relationships = cross_table_relationships(&parts[rels]);
    let unique: std::collections::BTreeSet<_> =
        relationships.iter().map(|entry| &entry["Id"]).collect();
    assert_eq!(unique.len(), relationships.len());
    let tables: Vec<_> = relationships
        .iter()
        .filter(|entry| entry["Type"] == format!("{R_NS}/table"))
        .collect();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0]["Id"], ids[0]);
    assert_eq!(tables[0]["Target"], target);
    assert!(!tables[0].contains_key("TargetMode"));
    let data = workbook.sheet("Data").unwrap();
    assert!(data.cell(at("E3")).is_none());
    assert_eq!(data.scalar(at("A1")), 9.0.into());
    assert_eq!(data.scalar(at("G4")), 44.0.into());
    let moved = workbook.sheet(destination).unwrap();
    assert_eq!(moved.scalar(at("J9")), 3.0.into());
    assert_eq!(
        moved
            .cell(at("K9"))
            .unwrap()
            .formula()
            .unwrap()
            .at(at("K9"))
            .to_string(),
        "J9+Data!$A$1"
    );
    ids[0].clone()
}

#[test]
fn table_cut_cross_sheet_refuses_multiple_relationship_roots_atomically() {
    let original = with_cut_tables(&[("Data", costs_cut_part())]);
    let mut parts = table_member_map(&original);
    let rels = "xl/worksheets/_rels/sheet1.xml.rels";
    let extra_at = parts[rels].len();
    parts.get_mut(rels).unwrap().extend_from_slice(b"<extra/>");
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    match workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        )
        .unwrap_err()
    {
        Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "xlsx");
            assert_eq!(position, extra_at);
            assert!(reason.contains("one metadata root"), "{reason}");
        }
        error => panic!("expected a located duplicate-root refusal, got {error}"),
    }
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert!(!workbook.is_dirty());
}

#[test]
fn table_cut_cross_sheet_creates_membership_in_a_loaded_destination() {
    let original = with_cut_tables(&[("Data", costs_cut_part())]);
    let mut parts = table_member_map(&original);
    let destination = "xl/custom/deep/other.xml";
    let other = parts.remove("xl/worksheets/sheet2.xml").unwrap();
    assert!(cross_table_ids(&other).is_empty());
    assert!(!parts.contains_key("xl/worksheets/_rels/sheet2.xml.rels"));
    parts.insert(destination.into(), other);
    for (name, before, after) in [
        (
            "xl/_rels/workbook.xml.rels",
            "worksheets/sheet2.xml",
            "custom/deep/other.xml",
        ),
        (
            "[Content_Types].xml",
            "/xl/worksheets/sheet2.xml",
            "/xl/custom/deep/other.xml",
        ),
    ] {
        let text = String::from_utf8(parts[name].clone()).unwrap();
        if name.ends_with(".rels") {
            assert_eq!(text.matches(before).count(), 1);
        }
        parts.insert(name.into(), text.replace(before, after).into_bytes());
    }
    let mut workbook = cross_table_reopen(&parts);
    workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_cross_table_owner(
        &workbook,
        "Other",
        destination,
        "xl/custom/deep/_rels/other.xml.rels",
        "../../tables/table1.xml",
    );
    // Before any save, the reference index must already assign the table
    // to Other. Its new owner moves both range and formula hosts once.
    workbook.insert_rows("Other", 0, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        cut_table_part(
            "Costs",
            11,
            ["J9:K13", "J9:K12", "J10:K12", "K10:K12"],
            [
                "J10+$J$10+Data!$A$1+Other!B4+Costs[[#This Row],[Cost]]",
                "SUM(J10:J12)+SUM($J$10:$J$12)+Costs[Cost]+Data!$A$1",
            ]
        )
    );
}

#[test]
fn table_cut_cross_sheet_creates_membership_in_a_fresh_destination() {
    let mut workbook = with_cut_tables(&[("Data", costs_cut_part())]);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    assert!(cross_table_ids(&before["xl/worksheets/sheet3.xml"]).is_empty());
    assert!(!before.contains_key("xl/worksheets/_rels/sheet3.xml.rels"));
    workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Fresh", at("J8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_cross_table_owner(
        &workbook,
        "Fresh",
        "xl/worksheets/sheet3.xml",
        "xl/worksheets/_rels/sheet3.xml.rels",
        "../tables/table1.xml",
    );
    assert!(cross_table_ids(&table_member_map(&workbook)["xl/worksheets/sheet2.xml"]).is_empty());
}

#[test]
fn table_cut_cross_sheet_saved_undo_redo_restores_membership_and_relationships_exactly() {
    use yggdryl::excel::Edit;

    let original = with_cut_tables(&[("Data", costs_cut_part())]);
    let mut parts = table_member_map(&original);
    // The destination already uses the source membership's ID for another
    // relationship. Preserve that entry and allocate a distinct table ID.
    let collision = format!(
        "<Relationship Id=\"rIdTable1\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/kept\" TargetMode=\"External\"/>"
    );
    let rels = "xl/worksheets/_rels/sheet2.xml.rels";
    parts.insert(rels.into(), format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{collision}</Relationships>").into_bytes());
    for fresh in [false, true] {
        let mut workbook = cross_table_reopen(&parts);
        let (destination, destination_part, destination_rels) = if fresh {
            workbook.add_sheet("Fresh").unwrap();
            (
                "Fresh",
                "xl/worksheets/sheet3.xml",
                "xl/worksheets/_rels/sheet3.xml.rels",
            )
        } else {
            ("Other", "xl/worksheets/sheet2.xml", rels)
        };
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "E2:F6".parse().unwrap()),
                to: (destination.into(), at("J8")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let id = assert_cross_table_owner(
            &workbook,
            destination,
            destination_part,
            destination_rels,
            "../tables/table1.xml",
        );
        if !fresh {
            assert_ne!(id, "rIdTable1");
        }
        assert!(member(&workbook, rels).contains(&collision));
        let moved = table_member_map(&workbook);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let mut undo = applied.inverse.unwrap();
        for _ in 0..2 {
            let undone = workbook.apply(undo).unwrap();
            assert_eq!(table_member_map(&workbook), before, "fresh={fresh}");
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
            let redone = workbook.apply(undone.inverse.unwrap()).unwrap();
            assert_eq!(table_member_map(&workbook), moved, "fresh={fresh}");
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
            undo = redone.inverse.unwrap();
        }
    }
}

#[test]
fn table_cut_cross_sheet_preserves_qualified_memberships_and_relationship_fragments() {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;
    use yggdryl::excel::Edit;

    let mut parts = table_member_map(&with_cut_tables(&[("Data", costs_cut_part())]));
    let source = String::from_utf8(parts["xl/worksheets/sheet1.xml"].clone()).unwrap()
        .replace("<worksheet ", "<worksheet xmlns:q='urn:source' ")
        .replace("<tableParts count=\"1\"><tablePart r:id=\"rIdTable1\"/></tableParts>",
            &format!("<x:tableParts xmlns:x='{NS}' xmlns:link='{}' count='1'><x:tablePart q:id='opaque' link:id='rIdTable1' q:flag='&quot; &amp;'><q:child xml:space='preserve'> α </q:child></x:tablePart></x:tableParts>", R_NS.replace("relationships", "relation&#115;hips")));
    parts.insert("xl/worksheets/sheet1.xml".into(), source.into_bytes());
    let destination = String::from_utf8(parts["xl/worksheets/sheet2.xml"].clone()).unwrap()
        .replace("<worksheet ", "<worksheet xmlns:q='urn:destination' xmlns:x='urn:destination-elements' xmlns:link='urn:destination-links' ");
    parts.insert("xl/worksheets/sheet2.xml".into(), destination.into_bytes());
    let source_rels = format!(
        "<?xml version='1.0'?><p:Relationships xmlns:p='http://schemas.openxmlformats.org/package/2006/relationships' xmlns:q='urn:relationship-source' q:flag='keep'><p:Relationship Id='rIdTable1' Type='{R_NS}/table' Target='../tables/table1.xml' q:Id='opaque-id'><q:child/></p:Relationship><!--tail--></p:Relationships><?after keep?>"
    );
    parts.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        source_rels.as_bytes().to_vec(),
    );
    let foreign = format!(
        "<p:Relationship Id='rIdTable1' Type='{R_NS}/hyperlink' Target='https://example.test/' TargetMode='External' q:flag='&quot; &amp;'/>"
    );
    let destination_rels = format!(
        "<?xml version='1.0'?><p:Relationships xmlns:p='http://schemas.openxmlformats.org/package/2006/relationships' xmlns:q='urn:relationship-destination'>{foreign}<!--destination--></p:Relationships><?after destination?>"
    );
    parts.insert(
        "xl/worksheets/_rels/sheet2.xml.rels".into(),
        destination_rels.as_bytes().to_vec(),
    );
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "E2:F6".parse().unwrap()),
            to: ("Other".into(), at("J8")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let moved = table_member_map(&workbook);
    let xml = std::str::from_utf8(&moved["xl/worksheets/sheet2.xml"]).unwrap();
    for literal in [
        "q:id='opaque'",
        "q:flag='&quot; &amp;'",
        "<q:child xml:space='preserve'> α </q:child>",
    ] {
        assert!(xml.contains(literal), "{xml}");
    }
    let mut reader = quick_xml::NsReader::from_reader(xml.as_bytes());
    let mut key = None;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) if start.local_name().as_ref() == b"tablePart" => {
                let (namespace, _) = reader.resolver().resolve_element(start.name());
                assert!(
                    matches!(namespace, ResolveResult::Bound(namespace) if namespace.as_ref() == NS.as_bytes())
                );
                for attribute in start.attributes().map(Result::unwrap) {
                    let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
                    if local.as_ref() != b"id" {
                        continue;
                    }
                    let ResolveResult::Bound(namespace) = namespace else {
                        panic!("unbound membership attribute");
                    };
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap();
                    if namespace.as_ref() == R_NS.as_bytes() {
                        key = Some(value.into_owned());
                    } else {
                        assert_eq!(namespace.as_ref(), b"urn:source");
                        assert_eq!(value, "opaque");
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert_ne!(key.as_deref(), Some("rIdTable1"));
    assert!(key.is_some());
    let destination_rels =
        std::str::from_utf8(&moved["xl/worksheets/_rels/sheet2.xml.rels"]).unwrap();
    for literal in [
        foreign.as_str(),
        "q:Id='opaque-id'",
        "<q:child/>",
        "<!--destination-->",
        "<?after destination?>",
    ] {
        assert!(destination_rels.contains(literal), "{destination_rels}");
    }
    assert!(destination_rels.contains("xmlns:q=\"urn:relationship-source\""));
    let source_rels = std::str::from_utf8(&moved["xl/worksheets/_rels/sheet1.xml.rels"]).unwrap();
    assert!(source_rels.contains("q:flag='keep'"));
    assert!(source_rels.ends_with("<!--tail--></p:Relationships><?after keep?>"));
    assert!(!source_rels.contains("<p:Relationship "));
    workbook.rebase(workbook.into_package().unwrap()).unwrap();
    let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
    assert_eq!(table_member_map(&workbook), before);
    workbook.rebase(workbook.into_package().unwrap()).unwrap();
    workbook.apply(undone.inverse.unwrap()).unwrap();
    assert_eq!(table_member_map(&workbook), moved);
}

const MCE_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

fn table_context_fixture(
    source: &str,
    container: &str,
    membership: &str,
    destination: &str,
) -> Workbook {
    let mut parts = table_member_map(&with_cut_tables(&[("Data", costs_cut_part())]));
    let text = String::from_utf8(parts["xl/worksheets/sheet1.xml"].clone())
        .unwrap()
        .replace("<worksheet ", &format!("<worksheet {source} "))
        .replace(
            "<tableParts count=\"1\">",
            &format!("<tableParts count=\"1\" {container}>"),
        )
        .replace("<tablePart r:id=\"rIdTable1\"/>", membership);
    parts.insert("xl/worksheets/sheet1.xml".into(), text.into_bytes());
    let text = String::from_utf8(parts["xl/worksheets/sheet2.xml"].clone())
        .unwrap()
        .replace("<worksheet ", &format!("<worksheet {destination} "));
    parts.insert("xl/worksheets/sheet2.xml".into(), text.into_bytes());
    cross_table_reopen(&parts)
}

#[test]
fn table_cut_cross_sheet_preserves_resolved_mce_context_and_authored_hints() {
    use quick_xml::events::Event;
    use quick_xml::name::{QName, ResolveResult};
    use std::collections::BTreeSet;

    let mut workbook = table_context_fixture(
        &format!(
            "xmlns:mc=\"{MCE_NS}\" xmlns:zz=\"{MCE_NS}\" xmlns:u=\"urn:old&amp;amp;\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:keep\" mc:MustUnderstand=\"u\""
        ),
        "xmlns:u=\"urn:new\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:also\"",
        "<tablePart xmlns:w='urn:third' xmlns:custom='urn:custom' custom:id='opaque' r:id='rIdTable1' zz:Ignorable='w' zz:ProcessContent='w:last' mc:PreserveAttributes='w:flag' w:flag='&quot; &amp;'><u:also><w:last/></u:also></tablePart>",
        &format!("xmlns:u=\"urn:destination\" xmlns:mc=\"{MCE_NS}\" xmlns:zz=\"{MCE_NS}\""),
    );
    workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        )
        .unwrap();
    let xml = member(&workbook, "xl/worksheets/sheet2.xml");
    assert!(xml.contains("custom:id='opaque'"), "{xml}");
    assert!(xml.contains("mc:PreserveAttributes='w:flag'"), "{xml}");
    assert!(xml.contains("w:flag='&quot; &amp;'"), "{xml}");
    assert!(xml.contains("<u:also><w:last/></u:also>"), "{xml}");
    let mut reader = quick_xml::NsReader::from_reader(xml.as_bytes());
    let mut checked = false;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(start) | Event::Empty(start)
                if start.local_name().as_ref() == b"tablePart" =>
            {
                let mut ignored = BTreeSet::new();
                let mut process = BTreeSet::new();
                let mut must_understand = false;
                let mut directive_names = BTreeSet::new();
                for attribute in start.attributes().map(Result::unwrap) {
                    let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
                    let ResolveResult::Bound(namespace) = namespace else {
                        continue;
                    };
                    if namespace.as_ref() != MCE_NS.as_bytes() {
                        continue;
                    }
                    assert!(
                        directive_names.insert(local.as_ref().to_vec()),
                        "duplicate expanded MC attribute in {xml}"
                    );
                    if local.as_ref() == b"Ignorable" || local.as_ref() == b"ProcessContent" {
                        assert!(
                            attribute.key.as_ref().starts_with(b"zz:"),
                            "authored QName must remain unchanged"
                        );
                    }
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap();
                    for token in value.split_whitespace() {
                        if local.as_ref() == b"MustUnderstand" {
                            must_understand = true;
                        }
                        if local.as_ref() != b"Ignorable" && local.as_ref() != b"ProcessContent" {
                            continue;
                        }
                        let (prefix, name) = token.split_once(':').unwrap_or((token, ""));
                        let qualified = format!("{prefix}:_");
                        let ResolveResult::Bound(namespace) = reader
                            .resolver()
                            .resolve_prefix(QName(qualified.as_bytes()).prefix(), false)
                        else {
                            panic!("unbound MCE prefix {prefix}");
                        };
                        let namespace = quick_xml::escape::unescape(
                            std::str::from_utf8(namespace.as_ref()).unwrap(),
                        )
                        .unwrap()
                        .into_owned();
                        if local.as_ref() == b"Ignorable" {
                            ignored.insert(namespace);
                        } else {
                            process.insert((namespace, name.to_owned()));
                        }
                    }
                }
                assert_eq!(
                    ignored,
                    ["urn:old&amp;", "urn:new", "urn:third"]
                        .map(str::to_owned)
                        .into()
                );
                assert_eq!(
                    process,
                    [
                        ("urn:old&amp;", "keep"),
                        ("urn:new", "also"),
                        ("urn:third", "last")
                    ]
                    .map(|(namespace, name)| (namespace.to_owned(), name.to_owned()))
                    .into()
                );
                assert!(
                    !must_understand,
                    "an ancestor's MustUnderstand is not inherited"
                );
                checked = true;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    assert!(checked);
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(member(&reopened, "xl/worksheets/sheet2.xml"), xml);
}

#[test]
fn table_cut_cross_sheet_refuses_unproven_destination_mce_context() {
    for directive in ["u:child", "u:*", "u:absent"] {
        let mut workbook = table_context_fixture(
            &format!(
                "xmlns:mc=\"{MCE_NS}\" xmlns:u=\"urn:vendor\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:known\""
            ),
            "",
            "<tablePart r:id=\"rIdTable1\"><u:known/><u:child/></tablePart>",
            &format!(
                "xmlns:mc=\"{MCE_NS}\" xmlns:u=\"urn:vendor\" mc:Ignorable=\"u\" mc:ProcessContent=\"{directive}\""
            ),
        );
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let result = workbook.paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        );
        if directive == "u:absent" {
            result.unwrap();
            assert_eq!(
                member(&workbook, "xl/tables/table1.xml"),
                cross_table_expected()
            );
        } else {
            match result.unwrap_err() {
                Error::InvalidRecord { path, reason } => {
                    assert!(path.contains("rIdTable1") && path.contains("MCE"), "{path}");
                    assert!(
                        reason.contains("destination") && reason.contains("ProcessContent"),
                        "{reason}"
                    );
                }
                error => panic!("expected a located MCE conflict, got {error}"),
            }
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert!(!workbook.is_dirty());
        }
    }
}

#[test]
fn table_cut_cross_sheet_refuses_inherited_non_2015_mce_hints_atomically() {
    let mut workbook = table_context_fixture(
        &format!(
            "xmlns:mc=\"{MCE_NS}\" xmlns:u=\"urn:vendor\" mc:Ignorable=\"u\" mc:PreserveElements=\"u:child\""
        ),
        "",
        "<tablePart r:id=\"rIdTable1\"><u:child/></tablePart>",
        "",
    );
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    let error = workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        )
        .unwrap_err();
    match error {
        Error::Unsupported {
            operation,
            filesystem,
        } => {
            assert!(operation.contains("2015"), "{operation}");
            assert!(
                filesystem.contains("rIdTable1") && filesystem.contains("mc"),
                "{filesystem}"
            );
        }
        error => panic!("expected a located inherited-MCE refusal, got {error}"),
    }
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert!(!workbook.is_dirty());
}

#[test]
fn table_cut_cross_sheet_refuses_malformed_mce_tokens_atomically() {
    for directive in [
        "mc:ProcessContent=':child'",
        "mc:ProcessContent='u:child:again'",
        "mc:ProcessContent='u:'",
        "mc:ProcessContent='u:1child'",
        "mc:ProcessContent='missing:child'",
        "mc:ProcessContent='v:child'",
        "mc:ProcessContent='mc:child'",
        "mc:Ignorable='u:bad'",
        "mc:Ignorable='mc'",
        "mc:Ignorable='u\u{a0}v'",
    ] {
        let mut workbook = table_context_fixture(
            &format!(
                "xmlns:mc=\"{MCE_NS}\" xmlns:u=\"urn:vendor\" xmlns:v=\"urn:not-ignorable\" mc:Ignorable=\"u\""
            ),
            "",
            &format!("<tablePart r:id='rIdTable1' {directive}/>"),
            "",
        );
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let error = workbook
            .paste(
                ("Data", "E2:F6".parse().unwrap()),
                ("Other", at("J8")),
                Paste::All,
                true,
            )
            .unwrap_err();
        match error {
            Error::Codec {
                format,
                position,
                reason,
            } => {
                assert_eq!(format, "xlsx");
                assert!(
                    position > 0,
                    "the malformed membership must have a byte location: {directive}"
                );
                assert!(
                    reason.contains("MCE") || reason.contains("ProcessContent"),
                    "{directive}: {reason}"
                );
            }
            error => {
                panic!("expected a typed located MCE syntax refusal for {directive}, got {error}")
            }
        }
        assert_eq!(table_member_map(&workbook), before, "{directive}");
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert!(!workbook.is_dirty(), "{directive}");
    }
}

#[test]
fn table_cut_cross_sheet_keeps_the_strict_namespace_family_in_fresh_destinations() {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    const STRICT_MAIN: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    const STRICT_REL: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let original = with_cut_tables(&[("Data", costs_cut_part())]);
    let mut parts = table_member_map(&original);
    // This fixture uses only core SpreadsheetML; replace both document and
    // OfficeDocument relationship namespaces throughout the complete package.
    // Package relationships/content types keep their OPC namespace.
    for bytes in parts.values_mut() {
        let text = std::str::from_utf8(bytes).unwrap();
        *bytes = text
            .replace(NS, STRICT_MAIN)
            .replace(R_NS, STRICT_REL)
            .into_bytes();
    }
    for (fresh, save_first) in [(false, false), (true, false), (true, true)] {
        let mut workbook = cross_table_reopen(&parts);
        let (destination, part, rels) = if fresh {
            workbook.add_sheet("Fresh").unwrap();
            (
                "Fresh",
                "xl/worksheets/sheet3.xml",
                "xl/worksheets/_rels/sheet3.xml.rels",
            )
        } else {
            (
                "Other",
                "xl/worksheets/sheet2.xml",
                "xl/worksheets/_rels/sheet2.xml.rels",
            )
        };
        if save_first {
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
        }
        workbook
            .paste(
                ("Data", "E2:F6".parse().unwrap()),
                (destination, at("J8")),
                Paste::All,
                true,
            )
            .unwrap();
        let saved = table_member_map(&workbook);
        let mut reader = quick_xml::NsReader::from_reader(saved["xl/workbook.xml"].as_slice());
        let mut sheet_ids = std::collections::BTreeSet::new();
        let mut sheet_relationships = std::collections::BTreeSet::new();
        let mut workbook_id = None;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(start) | Event::Empty(start)
                    if start.local_name().as_ref() == b"sheet" =>
                {
                    let (namespace, _) = reader.resolver().resolve_element(start.name());
                    let ResolveResult::Bound(namespace) = namespace else {
                        panic!("unbound workbook sheet element");
                    };
                    assert_eq!(namespace.as_ref(), STRICT_MAIN.as_bytes());
                    let mut name = None;
                    let mut id = None;
                    for attribute in start.attributes().map(Result::unwrap) {
                        let value = attribute
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap()
                            .into_owned();
                        match attribute.key.as_ref() {
                            b"name" => name = Some(value),
                            b"sheetId" => {
                                assert!(sheet_ids.insert(value), "duplicate sheetId");
                            }
                            _ => {
                                let (namespace, local) =
                                    reader.resolver().resolve_attribute(attribute.key);
                                if local.as_ref() == b"id" {
                                    let ResolveResult::Bound(namespace) = namespace else {
                                        panic!("unbound workbook relationship ID");
                                    };
                                    assert_eq!(namespace.as_ref(), STRICT_REL.as_bytes());
                                    id = Some(value);
                                }
                            }
                        }
                    }
                    let id = id.unwrap();
                    assert!(
                        sheet_relationships.insert(id.clone()),
                        "duplicate workbook relationship ID"
                    );
                    if name.as_deref() == Some(destination) {
                        workbook_id = Some(id);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        let workbook_rels = cross_table_relationships(&saved["xl/_rels/workbook.xml.rels"]);
        let relation = workbook_rels
            .iter()
            .find(|entry| Some(entry["Id"].as_str()) == workbook_id.as_deref())
            .unwrap();
        assert_eq!(
            relation["Type"],
            format!("{STRICT_REL}/worksheet"),
            "fresh={fresh}, save_first={save_first}"
        );
        assert_eq!(relation["Target"], part.strip_prefix("xl/").unwrap());
        let mut reader = quick_xml::NsReader::from_reader(saved[part].as_slice());
        let mut seen = Vec::new();
        let mut membership = None;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(start) | Event::Empty(start) => {
                    let (namespace, local) = reader.resolver().resolve_element(start.name());
                    if matches!(local.as_ref(), b"worksheet" | b"tableParts" | b"tablePart") {
                        let ResolveResult::Bound(namespace) = namespace else {
                            panic!("unbound core element in {part}, fresh={fresh}");
                        };
                        assert_eq!(
                            std::str::from_utf8(namespace.as_ref()).unwrap(),
                            STRICT_MAIN,
                            "Strict workbook must not mix a canonical Transitional frame with a Strict table membership: fresh={fresh}"
                        );
                        seen.push(local.as_ref().to_vec());
                    }
                    if local.as_ref() == b"tablePart" {
                        for attribute in start.attributes().map(Result::unwrap) {
                            let (namespace, name) =
                                reader.resolver().resolve_attribute(attribute.key);
                            if name.as_ref() == b"id" {
                                let ResolveResult::Bound(namespace) = namespace else {
                                    panic!("unbound relationship ID");
                                };
                                assert_eq!(
                                    std::str::from_utf8(namespace.as_ref()).unwrap(),
                                    STRICT_REL
                                );
                                membership = Some(
                                    attribute
                                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                        .unwrap()
                                        .into_owned(),
                                );
                            }
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(
            seen,
            [
                b"worksheet".to_vec(),
                b"tableParts".to_vec(),
                b"tablePart".to_vec()
            ]
        );
        let relations = cross_table_relationships(&saved[rels]);
        let entry = relations
            .iter()
            .find(|entry| Some(entry["Id"].as_str()) == membership.as_deref())
            .unwrap();
        assert_eq!(entry["Type"], format!("{STRICT_REL}/table"));
        assert_eq!(entry["Target"], "../tables/table1.xml");
        assert!(
            std::str::from_utf8(&saved["xl/tables/table1.xml"])
                .unwrap()
                .contains(&format!("xmlns=\"{STRICT_MAIN}\""))
        );
        let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
        assert_eq!(member(&reopened, part).as_bytes(), saved[part]);
    }
}

#[test]
fn table_cut_cross_sheet_keeps_inherited_ignorable_while_reparsing_authored_process_content() {
    use quick_xml::events::Event;
    use quick_xml::name::{QName, ResolveResult};

    for extra_destination_policy in [false, true] {
        let mut workbook = table_context_fixture(
            &format!("xmlns:mc=\"{MCE_NS}\" xmlns:u=\"urn:vendor\" mc:Ignorable=\"u\""),
            "",
            "<tablePart r:id='rIdTable1' mc:ProcessContent='u:keep'><u:keep/></tablePart>",
            &if extra_destination_policy {
                format!("xmlns:mc=\"{MCE_NS}\" xmlns:v=\"urn:other\" mc:Ignorable=\"v\"")
            } else {
                String::new()
            },
        );
        workbook.paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Other", at("J8")),
            Paste::All,
            true,
        ).unwrap_or_else(|error| panic!("valid inherited Ignorable was lost while reparsing a fragment (extra destination policy={extra_destination_policy}): {error}"));
        let xml = member(&workbook, "xl/worksheets/sheet2.xml");
        assert!(xml.contains("mc:ProcessContent='u:keep'"), "{xml}");
        assert!(xml.contains("<u:keep/>"), "{xml}");
        let mut reader = quick_xml::NsReader::from_reader(xml.as_bytes());
        let mut inherited_closed = false;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(start) | Event::Empty(start)
                    if start.local_name().as_ref() == b"tablePart" =>
                {
                    for attribute in start.attributes().map(Result::unwrap) {
                        let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                        if name.as_ref() != b"Ignorable" {
                            continue;
                        }
                        let ResolveResult::Bound(namespace) = namespace else {
                            continue;
                        };
                        if namespace.as_ref() != MCE_NS.as_bytes() {
                            continue;
                        }
                        let value = attribute
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap();
                        inherited_closed |= value.split_whitespace().any(|prefix| {
                            let qualified = format!("{prefix}:_");
                            matches!(reader.resolver().resolve_prefix(QName(qualified.as_bytes()).prefix(), false), ResolveResult::Bound(namespace) if namespace.as_ref() == b"urn:vendor")
                        });
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        assert!(
            inherited_closed,
            "source inherited Ignorable must be explicit in its new scope: {xml}"
        );
        let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
        assert_eq!(member(&reopened, "xl/worksheets/sheet2.xml"), xml);
    }
}

fn note_cut_fixture(other_note: bool) -> Workbook {
    use crate::excel_package::{
        content_types, rich_parts, root_relationships, workbook_relationships,
    };

    let rich = rich_parts();
    let original = |name: &str| {
        rich.iter()
            .find(|(part, _)| *part == name)
            .unwrap()
            .1
            .clone()
    };
    let original = |name| String::from_utf8(original(name)).unwrap();
    let comments = original("xl/comments1.xml")
        .replace("</authors>", "<author>Guest</author></authors>")
        .replace("</commentList>", "<comment ref=\"D4\" authorId=\"1\"><text><t>stationary</t></text></comment></commentList>");
    let original_vml = original("xl/drawings/vmlDrawing1.vml");
    let start = original_vml.find("<v:shape ").unwrap();
    let end = original_vml.find("</v:shape>").unwrap() + "</v:shape>".len();
    let stationary = original_vml[start..end]
        .replace("_x0000_s1025", "_x0000_s1026")
        .replace("2, 12, 0, 11, 4, 6, 5, 8", "4, 15, 2, 2, 6, 15, 6, 16")
        .replace("<x:Row>1</x:Row>", "<x:Row>3</x:Row>")
        .replace("<x:Column>1</x:Column>", "<x:Column>3</x:Column>");
    let source_vml = original_vml.replace("</xml>", &format!("{stationary}</xml>"));
    let mut types = content_types(2, false, false).replace(
        "<Override PartName=\"/xl/workbook.xml\"",
        "<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/><Override PartName=\"/xl/workbook.xml\"",
    );
    let mut extra = String::new();
    for number in 1..=(if other_note { 2 } else { 1 }) {
        extra.push_str(&format!("<Override PartName=\"/xl/comments{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml\"/>"));
    }
    types = types.replace("</Types>", &format!("{extra}</Types>"));
    let note_rels = |number| {
        format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdNote\" Type=\"{R_NS}/comments\" Target=\"../comments{number}.xml\"/><Relationship Id=\"rIdVml\" Type=\"{R_NS}/vmlDrawing\" Target=\"../drawings/vmlDrawing{number}.vml\"/></Relationships>"
        )
    };
    let frame = |notes| {
        crate::excel_package::worksheet("").replace(
            "</worksheet>",
            if notes {
                "<legacyDrawing r:id=\"rIdVml\"/></worksheet>"
            } else {
                "</worksheet>"
            },
        )
    };
    let mut parts: std::collections::BTreeMap<String, Vec<u8>> = [
        ("[Content_Types].xml", types),
        ("_rels/.rels", root_relationships()),
        (
            "xl/workbook.xml",
            crate::excel_package::workbook(&["Data", "Other"], false),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(2, false, false),
        ),
        ("xl/worksheets/sheet1.xml", frame(true)),
        ("xl/worksheets/sheet2.xml", frame(other_note)),
        ("xl/worksheets/_rels/sheet1.xml.rels", note_rels(1)),
        ("xl/comments1.xml", comments),
        ("xl/drawings/vmlDrawing1.vml", source_vml),
    ]
    .into_iter()
    .map(|(name, text)| (name.into(), text.into_bytes()))
    .collect();
    if other_note {
        parts.insert(
            "xl/worksheets/_rels/sheet2.xml.rels".into(),
            note_rels(2).into_bytes(),
        );
        parts.insert("xl/comments2.xml".into(), format!(
            "<comments xmlns=\"{NS}\"><authors><author>Bob</author></authors><commentList><comment ref=\"J10\" authorId=\"0\"><text><t>destination</t></text></comment></commentList></comments>"
        ).into_bytes());
        let destination = original_vml
            .replace("data=\"1\"", "data=\"3\"")
            .replace("_x0000_s1025", "_x0000_s3073")
            .replace("2, 12, 0, 11, 4, 6, 5, 8", "10, 15, 8, 2, 12, 15, 12, 16")
            .replace("<x:Row>1</x:Row>", "<x:Row>9</x:Row>")
            .replace("<x:Column>1</x:Column>", "<x:Column>9</x:Column>");
        parts.insert(
            "xl/drawings/vmlDrawing2.vml".into(),
            destination.into_bytes(),
        );
    }
    // Normalize writer metadata before exact-package inverse comparisons, while
    // leaving EVERY cell absent. In particular, B2 is only a note, not a Cell.
    let mut workbook = cross_table_reopen(&parts);
    for name in ["Data", "Other"] {
        let sheet = workbook.sheet_mut(name).unwrap();
        sheet.set_cell(at("A1"), 1.0).unwrap();
        sheet.remove_cell(at("A1"));
    }
    Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap()
}

/// The relationship's ID and resolved member; allocation names are not pinned.
fn note_cut_related(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
    kind: &str,
) -> Option<(String, String)> {
    let (folder, file) = sheet.rsplit_once('/').unwrap();
    let bytes = parts.get(&format!("{folder}/_rels/{file}.rels"))?;
    let rows = cross_table_relationships(bytes);
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|row| &row["Id"]).collect();
    assert_eq!(ids.len(), rows.len(), "duplicate relationship IDs: {sheet}");
    let related: Vec<_> = rows
        .iter()
        .filter(|row| row["Type"] == format!("{R_NS}/{kind}"))
        .collect();
    assert!(related.len() <= 1, "duplicate {kind} relationship: {sheet}");
    let row = related.first()?;
    assert!(
        !row.contains_key("TargetMode"),
        "internal note part: {sheet}"
    );
    let mut target: Vec<&str> = if row["Target"].starts_with('/') {
        Vec::new()
    } else {
        folder.split('/').collect()
    };
    for segment in row["Target"].split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                assert!(target.pop().is_some());
            }
            value => target.push(value),
        }
    }
    let target = target.join("/");
    assert!(
        parts.contains_key(&target),
        "missing {kind} target: {target}"
    );
    Some((row["Id"].clone(), target))
}

fn note_cut_text(element: yggdryl::xml::Element<'_>) -> String {
    let mut text = element.text().unwrap_or("").to_owned();
    for child in element.children() {
        text.push_str(&note_cut_text(child));
    }
    text
}

fn note_cut_assert_comments(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
    authors: &[&str],
    expected: &[(&str, usize, &str)],
) {
    let (_, member) = note_cut_related(parts, sheet, "comments").expect("comments owner");
    let document = yggdryl::xml::from_bytes(&parts[&member]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    assert!(root.is(Some(NS), "comments"));
    let list = root.child(Some(NS), "authors").unwrap();
    let actual: Vec<_> = list
        .children()
        .map(|author| {
            assert!(author.is(Some(NS), "author"));
            author.text().unwrap().to_owned()
        })
        .collect();
    assert_eq!(actual, authors, "{member}: author index domain");
    let list = root.child(Some(NS), "commentList").unwrap();
    let actual: std::collections::BTreeMap<_, _> = list
        .children()
        .map(|comment| {
            assert!(comment.is(Some(NS), "comment"));
            let reference = comment.attribute_in(None, "ref").unwrap().to_owned();
            let author: usize = comment
                .attribute_in(None, "authorId")
                .unwrap()
                .parse()
                .unwrap();
            assert!(author < authors.len());
            (
                reference,
                (
                    author,
                    note_cut_text(comment.child(Some(NS), "text").unwrap()),
                ),
            )
        })
        .collect();
    assert_eq!(
        list.children().count(),
        actual.len(),
        "duplicate comment refs"
    );
    let expected: std::collections::BTreeMap<_, _> = expected
        .iter()
        .map(|(reference, author, text)| ((*reference).to_owned(), (*author, (*text).to_owned())))
        .collect();
    assert_eq!(actual, expected, "{sheet}: note ownership and payload");
    if expected
        .values()
        .any(|(_, text)| text == "Author: list price")
    {
        let original = crate::excel_package::rich_parts()
            .into_iter()
            .find(|(name, _)| *name == "xl/comments1.xml")
            .unwrap()
            .1;
        let original = std::str::from_utf8(&original).unwrap();
        let start = original.find("<text>").unwrap();
        let end = original.find("</text>").unwrap() + "</text>".len();
        assert!(
            std::str::from_utf8(&parts[&member])
                .unwrap()
                .contains(&original[start..end]),
            "rich runs and xml:space remain byte-exact"
        );
    }
}

fn note_cut_assert_vml(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
    expected: &[((u32, u32), [i64; 8])],
) {
    const VML: &str = "urn:schemas-microsoft-com:vml";
    const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
    let (id, member) = note_cut_related(parts, sheet, "vmlDrawing").expect("VML owner");
    let document = yggdryl::xml::from_bytes(&parts[sheet]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let drawings = root.children_in(Some(NS), "legacyDrawing");
    assert_eq!(drawings.len(), 1);
    assert_eq!(
        drawings[0].attribute_in(Some(R_NS), "id"),
        Some(id.as_str())
    );
    let document = yggdryl::xml::from_bytes(&parts[&member]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let types: std::collections::BTreeSet<_> = root
        .children_in(Some(VML), "shapetype")
        .iter()
        .map(|shape| shape.attribute_in(None, "id").unwrap().to_owned())
        .collect();
    let shapes = root.children_in(Some(VML), "shape");
    let mut ids = std::collections::BTreeSet::new();
    let mut actual = std::collections::BTreeMap::new();
    for shape in &shapes {
        assert!(
            ids.insert(shape.attribute_in(None, "id").unwrap().to_owned()),
            "unique VML shape IDs"
        );
        let kind = shape
            .attribute_in(None, "type")
            .unwrap()
            .strip_prefix('#')
            .unwrap();
        assert!(types.contains(kind), "VML shape type resolves");
        let data = shape.child(Some(EXCEL), "ClientData").unwrap();
        assert_eq!(data.attribute_in(None, "ObjectType"), Some("Note"));
        let number = |name| {
            data.child(Some(EXCEL), name)
                .unwrap()
                .text()
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap()
        };
        let anchor = data.child(Some(EXCEL), "Anchor").unwrap();
        let values: Vec<i64> = anchor
            .text()
            .unwrap()
            .split(',')
            .map(|value| value.trim().parse().unwrap())
            .collect();
        let values: [i64; 8] = values.try_into().unwrap();
        assert!(
            actual
                .insert((number("Row"), number("Column")), values)
                .is_none(),
            "one shape per note owner"
        );
    }
    let expected: std::collections::BTreeMap<_, _> = expected.iter().copied().collect();
    assert_eq!(
        actual, expected,
        "{sheet}: full owner and both anchor corners"
    );
}

fn note_cut_assert_kept_shape(
    before: &std::collections::BTreeMap<String, Vec<u8>>,
    after: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
    id: &str,
) {
    let (_, original) = note_cut_related(before, sheet, "vmlDrawing").unwrap();
    let (_, current) = note_cut_related(after, sheet, "vmlDrawing").unwrap();
    let text = std::str::from_utf8(&before[&original]).unwrap();
    let id_at = text.find(&format!("id=\"{id}\"")).unwrap();
    let start = text[..id_at].rfind("<v:shape ").unwrap();
    let end = id_at + text[id_at..].find("</v:shape>").unwrap() + "</v:shape>".len();
    assert!(
        std::str::from_utf8(&after[&current])
            .unwrap()
            .contains(&text[start..end]),
        "unmoved shape remains byte-exact: {id}"
    );
}

/// All retained note/VML members have an owning worksheet and content type.
fn note_cut_assert_graph(parts: &std::collections::BTreeMap<String, Vec<u8>>, sheets: &[&str]) {
    let mut comments = std::collections::BTreeSet::new();
    let mut vml = std::collections::BTreeSet::new();
    for sheet in sheets {
        if let Some((_, member)) = note_cut_related(parts, sheet, "comments") {
            assert!(comments.insert(member));
        }
        if let Some((_, member)) = note_cut_related(parts, sheet, "vmlDrawing") {
            assert!(vml.insert(member));
        }
        let document = yggdryl::xml::from_bytes(&parts[*sheet]).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        assert!(
            root.children_in(Some(NS), "comments").is_empty(),
            "comments use implicit relationships"
        );
        assert!(root.children_in(Some(NS), "threadedComments").is_empty());
        if note_cut_related(parts, sheet, "vmlDrawing").is_none() {
            assert!(root.children_in(Some(NS), "legacyDrawing").is_empty());
        }
    }
    let document = yggdryl::xml::from_bytes(&parts["[Content_Types].xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let listed: std::collections::BTreeSet<_> = root
        .children()
        .filter(|entry| {
            entry.attribute_in(None, "ContentType")
                == Some("application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml")
        })
        .map(|entry| {
            entry
                .attribute_in(None, "PartName")
                .unwrap()
                .trim_start_matches('/')
                .to_owned()
        })
        .collect();
    assert_eq!(
        listed, comments,
        "no missing or orphaned comments content type"
    );
    let present: std::collections::BTreeSet<_> = parts
        .keys()
        .filter(|name| name.ends_with(".vml"))
        .cloned()
        .collect();
    assert_eq!(present, vml, "no orphaned VML member");
    for (name, bytes) in parts {
        if name.ends_with(".xml") {
            let document = yggdryl::xml::from_bytes(bytes).unwrap();
            let element = yggdryl::xml::Element::root(&document).unwrap();
            if element.is(Some(NS), "comments") {
                assert!(comments.contains(name), "orphaned comments member: {name}");
            }
        }
    }
    assert!(
        root.children()
            .any(|entry| entry.attribute_in(None, "Extension") == Some("vml")
                && entry.attribute_in(None, "ContentType")
                    == Some("application/vnd.openxmlformats-officedocument.vmlDrawing"))
    );
}

fn note_cut_saved_inverse(
    workbook: &mut Workbook,
    before: &std::collections::BTreeMap<String, Vec<u8>>,
    mut undo: yggdryl::excel::Edit,
) {
    let moved = table_member_map(workbook);
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    assert_eq!(table_member_map(workbook), moved);
    assert_eq!(table_member_map(&cross_table_reopen(&moved)), moved);
    for _ in 0..2 {
        let undone = workbook.apply(undo).unwrap();
        assert_eq!(&table_member_map(workbook), before);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        assert_eq!(
            &table_member_map(workbook),
            before,
            "saved inverse restores every byte and member absence"
        );
        let redone = workbook.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(workbook), moved);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        assert_eq!(table_member_map(workbook), moved);
        undo = redone.inverse.unwrap();
    }
}

#[test]
fn note_cut_retained_redo_refuses_parts_reused_by_another_owner_atomically() {
    let mut workbook = note_cut_fixture(false);
    workbook.add_sheet("Fresh").unwrap();
    let save = |workbook: &mut Workbook| {
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
    };
    save(&mut workbook);
    let first = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    save(&mut workbook);
    let first_parts = table_member_map(&workbook);
    let first_comments = note_cut_related(&first_parts, "xl/worksheets/sheet3.xml", "comments")
        .unwrap()
        .1;
    let first_drawing = note_cut_related(&first_parts, "xl/worksheets/sheet3.xml", "vmlDrawing")
        .unwrap()
        .1;
    let undone = workbook.apply(first.inverse.unwrap()).unwrap();
    let retained_redo = undone.inverse.unwrap();
    save(&mut workbook);
    let absent = table_member_map(&workbook);
    assert!(!absent.contains_key(&first_comments));
    assert!(!absent.contains_key(&first_drawing));

    // The released names now belong to a different worksheet and note.
    workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "D4".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    save(&mut workbook);
    let before = table_member_map(&workbook);
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet2.xml", "comments")
            .unwrap()
            .1,
        first_comments
    );
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet2.xml", "vmlDrawing")
            .unwrap()
            .1,
        first_drawing
    );
    note_cut_assert_comments(
        &before,
        "xl/worksheets/sheet2.xml",
        &["Author", "Guest"],
        &[("J10", 1, "stationary")],
    );
    let revisions = ["Data", "Other", "Fresh"].map(|name| workbook.sheet(name).unwrap().revision());
    assert!(!workbook.is_dirty());
    match workbook.apply(retained_redo).unwrap_err() {
        Error::Conflict { path, .. } => assert!(
            path.contains(&first_comments) || path.contains(&first_drawing),
            "the refusal must locate a reused package identity: {path}",
        ),
        error => panic!("expected a reused-part ownership conflict, got {error}"),
    }
    assert_eq!(workbook.sheet_names(), ["Data", "Other", "Fresh"]);
    assert_eq!(
        ["Data", "Other", "Fresh"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions,
    );
    assert!(!workbook.is_dirty());
    assert_eq!(
        table_member_map(&workbook),
        before,
        "a refused historical redo leaves all package bytes and membership unchanged"
    );
}

#[test]
fn note_cut_retained_undo_refuses_parts_reused_by_another_owner_atomically() {
    let mut workbook = note_cut_fixture(false);
    workbook.add_sheet("Fresh").unwrap();
    let mut initial = table_member_map(&workbook);
    // Keep Fresh's relationship part present even after its notes are undone:
    // a presence-only guard cannot mistake this for proof of note ownership.
    initial.insert("xl/worksheets/_rels/sheet3.xml.rels".into(), format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdKeep\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/kept\" TargetMode=\"External\"/></Relationships>"
    ).into_bytes());
    let mut workbook = cross_table_reopen(&initial);
    let save = |workbook: &mut Workbook| {
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
    };
    save(&mut workbook);
    let first = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    save(&mut workbook);
    let first_parts = table_member_map(&workbook);
    let first_comments = note_cut_related(&first_parts, "xl/worksheets/sheet3.xml", "comments")
        .unwrap()
        .1;
    let first_drawing = note_cut_related(&first_parts, "xl/worksheets/sheet3.xml", "vmlDrawing")
        .unwrap()
        .1;
    let retained_undo = first.inverse.unwrap();
    workbook.apply(retained_undo.clone()).unwrap();
    save(&mut workbook);
    let absent = table_member_map(&workbook);
    assert!(!absent.contains_key(&first_comments));
    assert!(!absent.contains_key(&first_drawing));

    // The released names now belong to a different worksheet and note.
    workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "D4".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    save(&mut workbook);
    let before = table_member_map(&workbook);
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet2.xml", "comments")
            .unwrap()
            .1,
        first_comments
    );
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet2.xml", "vmlDrawing")
            .unwrap()
            .1,
        first_drawing
    );
    note_cut_assert_comments(
        &before,
        "xl/worksheets/sheet2.xml",
        &["Author", "Guest"],
        &[("J10", 1, "stationary")],
    );
    let revisions = ["Data", "Other", "Fresh"].map(|name| workbook.sheet(name).unwrap().revision());
    assert!(!workbook.is_dirty());
    match workbook.apply(retained_undo).unwrap_err() {
        Error::Conflict { path, .. } => assert!(
            path.contains(&first_comments) || path.contains(&first_drawing),
            "the refusal must locate a reused package identity: {path}",
        ),
        error => panic!("expected a reused-part ownership conflict, got {error}"),
    }
    assert_eq!(workbook.sheet_names(), ["Data", "Other", "Fresh"]);
    assert_eq!(
        ["Data", "Other", "Fresh"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions,
    );
    assert!(!workbook.is_dirty());
    assert_eq!(
        table_member_map(&workbook),
        before,
        "a refused historical undo leaves all package bytes and membership unchanged"
    );
}

#[test]
fn note_cut_moves_an_unstored_same_sheet_note_and_its_whole_box() {
    let mut workbook = note_cut_fixture(false);
    assert!(workbook.sheet("Data").unwrap().cell(at("B2")).is_none());
    assert!(workbook.sheet("Data").unwrap().cell(at("D5")).is_none());
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Data".into(), at("D5")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D5", 0, "Author: list price"), ("D4", 1, "stationary")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet1.xml",
        &[
            // Native B2 anchor +3 rows/+2 columns to D5; offsets are unchanged.
            ((4, 3), [4, 12, 3, 11, 6, 6, 8, 8]),
            ((3, 3), [4, 15, 2, 2, 6, 15, 6, 16]),
        ],
    );
    note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet1.xml", "_x0000_s1026");
    assert_eq!(
        parts["xl/worksheets/_rels/sheet1.xml.rels"],
        before["xl/worksheets/_rels/sheet1.xml.rels"]
    );
    assert!(workbook.sheet("Data").unwrap().cell(at("B2")).is_none());
    assert!(workbook.sheet("Data").unwrap().cell(at("D5")).is_none());
    note_cut_assert_graph(
        &parts,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_partitions_parts_and_remaps_per_part_authors() {
    let mut workbook = note_cut_fixture(true);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D4", 1, "stationary")],
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet2.xml",
        &["Bob", "Author"],
        &[("J10", 0, "destination"), ("F6", 1, "Author: list price")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet1.xml",
        &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet2.xml",
        &[
            ((9, 9), [10, 15, 8, 2, 12, 15, 12, 16]),
            // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
            ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
        ],
    );
    note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet1.xml", "_x0000_s1026");
    note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet2.xml", "_x0000_s3073");
    assert!(workbook.sheet("Data").unwrap().cell(at("B2")).is_none());
    assert!(workbook.sheet("Other").unwrap().cell(at("F6")).is_none());
    note_cut_assert_graph(
        &parts,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_moves_all_notes_into_a_fresh_sheet_and_restores_absence() {
    let mut workbook = note_cut_fixture(false);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    assert!(!before.contains_key("xl/worksheets/_rels/sheet3.xml.rels"));
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2:D4".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "comments").is_none());
    assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "vmlDrawing").is_none());
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet3.xml",
        &["Author", "Guest"],
        &[("F6", 0, "Author: list price"), ("H8", 1, "stationary")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet3.xml",
        &[
            // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
            ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
            ((7, 7), [8, 15, 6, 2, 10, 15, 10, 16]),
        ],
    );
    assert!(workbook.sheet("Fresh").unwrap().cell(at("F6")).is_none());
    assert!(workbook.sheet("Fresh").unwrap().cell(at("H8")).is_none());
    note_cut_assert_graph(
        &parts,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_merges_all_notes_and_prunes_the_retired_source_parts() {
    let mut workbook = note_cut_fixture(true);
    let before = table_member_map(&workbook);
    let (_, old_comments) =
        note_cut_related(&before, "xl/worksheets/sheet1.xml", "comments").unwrap();
    let (_, old_vml) = note_cut_related(&before, "xl/worksheets/sheet1.xml", "vmlDrawing").unwrap();
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2:D4".parse().unwrap()),
            to: ("Other".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "comments").is_none());
    assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "vmlDrawing").is_none());
    assert!(
        !parts.contains_key(&old_comments),
        "source comments became orphaned"
    );
    assert!(
        !parts.contains_key(&old_vml),
        "source drawing became orphaned"
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet2.xml",
        &["Bob", "Author", "Guest"],
        &[
            ("J10", 0, "destination"),
            ("F6", 1, "Author: list price"),
            ("H8", 2, "stationary"),
        ],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet2.xml",
        &[
            ((9, 9), [10, 15, 8, 2, 12, 15, 12, 16]),
            // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
            ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
            ((7, 7), [8, 15, 6, 2, 10, 15, 10, 16]),
        ],
    );
    note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet2.xml", "_x0000_s3073");
    note_cut_assert_graph(
        &parts,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_splits_notes_into_new_parts_and_restores_their_absence() {
    let mut workbook = note_cut_fixture(false);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    let (_, source_comments) =
        note_cut_related(&before, "xl/worksheets/sheet1.xml", "comments").unwrap();
    let (_, source_vml) =
        note_cut_related(&before, "xl/worksheets/sheet1.xml", "vmlDrawing").unwrap();
    assert!(!before.contains_key("xl/worksheets/_rels/sheet3.xml.rels"));
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    let (_, new_comments) =
        note_cut_related(&parts, "xl/worksheets/sheet3.xml", "comments").unwrap();
    let (_, new_vml) = note_cut_related(&parts, "xl/worksheets/sheet3.xml", "vmlDrawing").unwrap();
    assert_ne!(new_comments, source_comments);
    assert_ne!(new_vml, source_vml);
    assert!(!before.contains_key(&new_comments));
    assert!(!before.contains_key(&new_vml));
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D4", 1, "stationary")],
    );
    // A split copies the author's existing index domain; it need not reindex the source.
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet3.xml",
        &["Author", "Guest"],
        &[("F6", 0, "Author: list price")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet1.xml",
        &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet3.xml",
        // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
        &[((5, 5), [6, 12, 4, 11, 8, 6, 9, 8])],
    );
    note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet1.xml", "_x0000_s1026");
    assert!(workbook.sheet("Fresh").unwrap().cell(at("F6")).is_none());
    note_cut_assert_graph(
        &parts,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_selected_note_replaces_destination_note_same_and_cross_sheet() {
    for same in [true, false] {
        let mut workbook = note_cut_fixture(!same);
        let before = table_member_map(&workbook);
        let destination = if same { "Data" } else { "Other" };
        let target = if same { "D4" } else { "J10" };
        let applied = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: ("Data".into(), "B2".parse().unwrap()),
                to: (destination.into(), at(target)),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let parts = table_member_map(&workbook);
        if same {
            note_cut_assert_comments(
                &parts,
                "xl/worksheets/sheet1.xml",
                &["Author", "Guest"],
                &[("D4", 0, "Author: list price")],
            );
            // Native rich_vml.xml anchors B2 at [2,12,0,11,4,6,5,8].
            // Its move to D4 adds two to both corner indices; offsets stay exact.
            note_cut_assert_vml(
                &parts,
                "xl/worksheets/sheet1.xml",
                &[((3, 3), [4, 12, 2, 11, 6, 6, 7, 8])],
            );
        } else {
            note_cut_assert_comments(
                &parts,
                "xl/worksheets/sheet1.xml",
                &["Author", "Guest"],
                &[("D4", 1, "stationary")],
            );
            note_cut_assert_comments(
                &parts,
                "xl/worksheets/sheet2.xml",
                &["Bob", "Author"],
                &[("J10", 1, "Author: list price")],
            );
            note_cut_assert_vml(
                &parts,
                "xl/worksheets/sheet1.xml",
                &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
            );
            // The incoming B2 note uses the native anchor, not the replaced
            // synthetic J10 note: add eight to its four corner indices only.
            note_cut_assert_vml(
                &parts,
                "xl/worksheets/sheet2.xml",
                &[((9, 9), [10, 12, 8, 11, 12, 6, 13, 8])],
            );
        }
        assert!(
            workbook
                .sheet(destination)
                .unwrap()
                .cell(at(target))
                .is_none()
        );
        note_cut_assert_graph(
            &parts,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn note_cut_clearing_the_last_notes_keeps_unrelated_vml_primitives() {
    const PRIMITIVES: &str = concat!(
        "<v:rect id=\"_x0000_s4097\" style=\"position:absolute;left:1pt;top:2pt;width:12pt;height:8pt\"><v:fill color=\"#112233\"/></v:rect>",
        "<v:oval id=\"_x0000_s4098\" style=\"position:absolute;left:20pt;top:30pt;width:9pt;height:7pt\" strokecolor=\"#445566\"/>",
    );
    let original = note_cut_fixture(false);
    let mut parts = table_member_map(&original);
    let sheet = "xl/worksheets/sheet1.xml";
    let original_owner = note_cut_related(&parts, sheet, "vmlDrawing").unwrap();
    let xml = String::from_utf8(parts[&original_owner.1].clone()).unwrap();
    assert_eq!(xml.matches("</xml>").count(), 1);
    parts.insert(
        original_owner.1.clone(),
        xml.replace("</xml>", &format!("{PRIMITIVES}</xml>"))
            .into_bytes(),
    );
    for source in ["Data", "Other"] {
        let mut workbook = cross_table_reopen(&parts);
        let before = table_member_map(&workbook);
        // A10:C12 is blank on both worksheets. Its 3x3 landing clears
        // both B2 and D4 notes, without moving any VML primitive.
        let applied = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: (source.into(), "A10:C12".parse().unwrap()),
                to: ("Data".into(), at("B2")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        assert!(note_cut_related(&after, sheet, "comments").is_none());
        assert_eq!(
            note_cut_related(&after, sheet, "vmlDrawing"),
            Some(original_owner.clone()),
            "{source}: unrelated primitive shapes retain their worksheet owner and part",
        );
        assert!(
            std::str::from_utf8(&after[&original_owner.1])
                .unwrap()
                .contains(PRIMITIVES),
            "{source}: unrelated VML primitive payloads remain byte-exact",
        );
        note_cut_assert_vml(&after, sheet, &[]);
        note_cut_assert_graph(
            &after,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn note_cut_blank_source_erases_destination_notes_and_prunes_empty_parts() {
    for same in [true, false] {
        let mut workbook = note_cut_fixture(false);
        let before = table_member_map(&workbook);
        let (source, block) = if same {
            ("Data", "A1")
        } else {
            ("Other", "A1:C3")
        };
        let applied = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: (source.into(), block.parse().unwrap()),
                to: ("Data".into(), at("B2")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let parts = table_member_map(&workbook);
        if same {
            note_cut_assert_comments(
                &parts,
                "xl/worksheets/sheet1.xml",
                &["Author", "Guest"],
                &[("D4", 1, "stationary")],
            );
            note_cut_assert_vml(
                &parts,
                "xl/worksheets/sheet1.xml",
                &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
            );
            note_cut_assert_kept_shape(&before, &parts, "xl/worksheets/sheet1.xml", "_x0000_s1026");
        } else {
            assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "comments").is_none());
            assert!(note_cut_related(&parts, "xl/worksheets/sheet1.xml", "vmlDrawing").is_none());
        }
        assert!(workbook.sheet("Data").unwrap().cell(at("B2")).is_none());
        note_cut_assert_graph(
            &parts,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

// Valid OPC layouts need not use conventional filenames.

fn note_cut_relocated_source(comments: &str, drawing: Option<&str>) -> Workbook {
    let workbook = note_cut_fixture(false);
    let mut parts = table_member_map(&workbook);
    let bytes = parts.remove("xl/comments1.xml").unwrap();
    assert!(parts.insert(comments.into(), bytes).is_none());
    let mut types = String::from_utf8(parts.remove("[Content_Types].xml").unwrap())
        .unwrap()
        .replace(
            "PartName=\"/xl/comments1.xml\"",
            &format!("PartName=\"/{comments}\""),
        );
    let mut rels = String::from_utf8(parts.remove("xl/worksheets/_rels/sheet1.xml.rels").unwrap())
        .unwrap()
        .replace(
            "Target=\"../comments1.xml\"",
            &format!("Target=\"/{comments}\""),
        );
    if let Some(drawing) = drawing {
        let bytes = parts.remove("xl/drawings/vmlDrawing1.vml").unwrap();
        assert!(parts.insert(drawing.into(), bytes).is_none());
        rels = rels.replace(
            "Target=\"../drawings/vmlDrawing1.vml\"",
            &format!("Target=\"/{drawing}\""),
        );
        types = types.replace("</Types>", &format!(
            "<Override PartName=\"/{drawing}\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/></Types>"
        ));
    }
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    parts.insert(
        "xl/worksheets/_rels/sheet1.xml.rels".into(),
        rels.into_bytes(),
    );
    cross_table_reopen(&parts)
}

#[test]
fn note_cut_split_parts_reserve_live_fresh_worksheet_identities() {
    let mut workbook = note_cut_relocated_source("xl/worksheets/sheet0.xml", None);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    assert!(before.contains_key("xl/worksheets/sheet3.xml"));
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    let (_, member) = note_cut_related(&parts, "xl/worksheets/sheet3.xml", "comments").unwrap();
    assert_ne!(
        member, "xl/worksheets/sheet3.xml",
        "a note part cannot occupy its fresh worksheet identity"
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet3.xml",
        &["Author", "Guest"],
        &[("F6", 0, "Author: list price")],
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D4", 1, "stationary")],
    );
    note_cut_assert_graph(
        &parts,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_split_parts_reserve_each_other_before_publication() {
    let mut workbook = note_cut_relocated_source("xl/notes1.xml", Some("xl/notes2.xml"));
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    let (_, comments) = note_cut_related(&parts, "xl/worksheets/sheet3.xml", "comments").unwrap();
    let (_, drawing) = note_cut_related(&parts, "xl/worksheets/sheet3.xml", "vmlDrawing").unwrap();
    assert_ne!(
        comments, drawing,
        "distinct media cannot share one newly allocated member"
    );
    assert!(!before.contains_key(&comments));
    assert!(!before.contains_key(&drawing));
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet3.xml",
        &["Author", "Guest"],
        &[("F6", 0, "Author: list price")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet3.xml",
        // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
        &[((5, 5), [6, 12, 4, 11, 8, 6, 9, 8])],
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D4", 1, "stationary")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet1.xml",
        &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
    );
    let document = yggdryl::xml::from_bytes(&parts["[Content_Types].xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    for (member, mime) in [
        (
            comments,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
        ),
        (
            drawing,
            "application/vnd.openxmlformats-officedocument.vmlDrawing",
        ),
    ] {
        let name = format!("/{member}");
        let registrations: Vec<_> = root
            .children()
            .filter(|child| child.attribute_in(None, "PartName") == Some(name.as_str()))
            .collect();
        assert_eq!(registrations.len(), 1);
        assert_eq!(
            registrations[0].attribute_in(None, "ContentType"),
            Some(mime)
        );
    }
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_split_parts_reserve_owners_of_orphan_relationship_documents() {
    let workbook = note_cut_relocated_source("xl/notes99.xml", None);
    let mut parts = table_member_map(&workbook);
    parts.insert("xl/_rels/notes1.xml.rels".into(),
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdOpaque\" Type=\"urn:opaque\" Target=\"../opaque.bin\"/></Relationships>".to_vec());
    parts.insert("opaque.bin".into(), vec![1, 2, 3, 4]);
    let types = String::from_utf8(parts.remove("[Content_Types].xml").unwrap())
        .unwrap()
        .replace(
            "</Types>",
            "<Default Extension=\"bin\" ContentType=\"application/octet-stream\"/></Types>",
        );
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    let (_, comments) = note_cut_related(&parts, "xl/worksheets/sheet3.xml", "comments").unwrap();
    assert_ne!(
        comments, "xl/notes1.xml",
        "a new note part cannot acquire an orphan .rels document"
    );
    assert_eq!(
        parts["xl/_rels/notes1.xml.rels"],
        before["xl/_rels/notes1.xml.rels"]
    );
    assert_eq!(parts["opaque.bin"], [1, 2, 3, 4]);
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet3.xml",
        &["Author", "Guest"],
        &[("F6", 0, "Author: list price")],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_blank_source_with_other_comment_namespace_only_clears_the_destination() {
    let workbook = note_cut_fixture(true);
    let mut parts = table_member_map(&workbook);
    let (_, source_comments) =
        note_cut_related(&parts, "xl/worksheets/sheet2.xml", "comments").unwrap();
    let text = std::str::from_utf8(&parts[&source_comments])
        .unwrap()
        .replace(NS, "http://purl.oclc.org/ooxml/spreadsheetml/main");
    parts.insert(source_comments.clone(), text.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Other".into(), "A1".parse().unwrap()),
            to: ("Data".into(), at("B2")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    assert_eq!(
        parts[&source_comments], before[&source_comments],
        "no source comment traveled"
    );
    note_cut_assert_comments(
        &parts,
        "xl/worksheets/sheet1.xml",
        &["Author", "Guest"],
        &[("D4", 1, "stationary")],
    );
    note_cut_assert_vml(
        &parts,
        "xl/worksheets/sheet1.xml",
        &[((3, 3), [4, 15, 2, 2, 6, 15, 6, 16])],
    );
    note_cut_assert_graph(
        &parts,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_remaps_standard_numeric_shape_and_type_id_collisions() {
    const VML: &str = "urn:schemas-microsoft-com:vml";
    const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
    for different_type in [false, true] {
        let workbook = note_cut_fixture(true);
        let mut parts = table_member_map(&workbook);
        let (_, target) =
            note_cut_related(&parts, "xl/worksheets/sheet2.xml", "vmlDrawing").unwrap();
        let mut xml = std::str::from_utf8(&parts[&target])
            .unwrap()
            .replace("_x0000_s3073", "_x0000_s1025")
            .replace("data=\"3\"", "data=\"1\"");
        if different_type {
            assert!(xml.contains("coordsize=\"21600,21600\""));
            xml = xml.replace("coordsize=\"21600,21600\"", "coordsize=\"21601,21601\"");
        }
        parts.insert(target.clone(), xml.into_bytes());
        let mut workbook = cross_table_reopen(&parts);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: ("Data".into(), "B2".parse().unwrap()),
                to: ("Other".into(), at("F6")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        note_cut_assert_comments(
            &after,
            "xl/worksheets/sheet1.xml",
            &["Author", "Guest"],
            &[("D4", 1, "stationary")],
        );
        note_cut_assert_comments(
            &after,
            "xl/worksheets/sheet2.xml",
            &["Bob", "Author"],
            &[("J10", 0, "destination"), ("F6", 1, "Author: list price")],
        );
        note_cut_assert_vml(
            &after,
            "xl/worksheets/sheet2.xml",
            &[
                ((9, 9), [10, 15, 8, 2, 12, 15, 12, 16]),
                // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
                ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
            ],
        );
        note_cut_assert_kept_shape(&before, &after, "xl/worksheets/sheet2.xml", "_x0000_s1025");
        let document = yggdryl::xml::from_bytes(&after[&target]).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let mut moved_type = None;
        let mut kept_type = None;
        for shape in root.children_in(Some(VML), "shape") {
            let data = shape.child(Some(EXCEL), "ClientData").unwrap();
            let row = data
                .child(Some(EXCEL), "Row")
                .unwrap()
                .text()
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap();
            let kind = shape
                .attribute_in(None, "type")
                .unwrap()
                .trim_start_matches('#')
                .to_owned();
            match row {
                5 => {
                    assert_ne!(shape.attribute_in(None, "id"), Some("_x0000_s1025"));
                    assert!(
                        shape
                            .attribute_in(None, "id")
                            .unwrap()
                            .strip_prefix("_x0000_s")
                            .unwrap()
                            .parse::<u32>()
                            .is_ok(),
                        "allocated ID remains a standard numeric shape ID"
                    );
                    moved_type = Some(kind);
                }
                9 => {
                    assert_eq!(shape.attribute_in(None, "id"), Some("_x0000_s1025"));
                    kept_type = Some(kind);
                }
                _ => panic!("unexpected note owner"),
            }
        }
        let moved_type = moved_type.unwrap();
        let kept_type = kept_type.unwrap();
        assert_eq!(kept_type, "_x0000_t202");
        assert_eq!(moved_type == kept_type, !different_type);
        for kind in root.children_in(Some(VML), "shapetype") {
            let id = kind.attribute_in(None, "id").unwrap();
            if id == moved_type {
                assert_eq!(kind.attribute_in(None, "coordsize"), Some("21600,21600"));
            }
            if id == kept_type && different_type {
                assert_eq!(kind.attribute_in(None, "coordsize"), Some("21601,21601"));
            }
        }
        note_cut_assert_graph(
            &after,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn note_cut_remaps_a_standard_type_id_without_a_shape_id_collision() {
    const VML: &str = "urn:schemas-microsoft-com:vml";
    const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
    let workbook = note_cut_fixture(true);
    let mut parts = table_member_map(&workbook);
    let (_, member) = note_cut_related(&parts, "xl/worksheets/sheet2.xml", "vmlDrawing").unwrap();
    let xml = std::str::from_utf8(&parts[&member]).unwrap();
    assert!(xml.contains("coordsize=\"21600,21600\""));
    parts.insert(
        member.clone(),
        xml.replace("coordsize=\"21600,21600\"", "coordsize=\"21601,21601\"")
            .into_bytes(),
    );
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    note_cut_assert_kept_shape(&before, &after, "xl/worksheets/sheet2.xml", "_x0000_s3073");
    note_cut_assert_vml(
        &after,
        "xl/worksheets/sheet2.xml",
        &[
            ((9, 9), [10, 15, 8, 2, 12, 15, 12, 16]),
            // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
            ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
        ],
    );
    let document = yggdryl::xml::from_bytes(&after[&member]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let moved = root
        .children_in(Some(VML), "shape")
        .into_iter()
        .find(|shape| {
            shape
                .child(Some(EXCEL), "ClientData")
                .unwrap()
                .child(Some(EXCEL), "Row")
                .unwrap()
                .text()
                .unwrap()
                .trim()
                == "5"
        })
        .unwrap();
    assert_eq!(moved.attribute_in(None, "id"), Some("_x0000_s1025"));
    let kind = moved
        .attribute_in(None, "type")
        .unwrap()
        .trim_start_matches('#');
    assert_ne!(kind, "_x0000_t202");
    assert!(
        kind.strip_prefix("_x0000_t")
            .unwrap()
            .parse::<u32>()
            .is_ok()
    );
    let types = root.children_in(Some(VML), "shapetype");
    assert!(
        types
            .iter()
            .any(|entry| entry.attribute_in(None, "id") == Some(kind)
                && entry.attribute_in(None, "coordsize") == Some("21600,21600"))
    );
    assert!(types.iter().any(
        |entry| entry.attribute_in(None, "id") == Some("_x0000_t202")
            && entry.attribute_in(None, "coordsize") == Some("21601,21601")
    ));
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn note_cut_remaps_matching_office_shape_identity_with_the_numeric_id() {
    const VML: &str = "urn:schemas-microsoft-com:vml";
    const OFFICE: &str = "urn:schemas-microsoft-com:office:office";
    const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
    let workbook = note_cut_fixture(true);
    let mut parts = table_member_map(&workbook);
    let (_, source) = note_cut_related(&parts, "xl/worksheets/sheet1.xml", "vmlDrawing").unwrap();
    let source_xml = std::str::from_utf8(&parts[&source]).unwrap();
    let needle = "id=\"_x0000_s1025\"";
    assert_eq!(source_xml.matches(needle).count(), 1);
    parts.insert(
        source.clone(),
        source_xml
            .replace(needle, "id=\"_x0000_s1025\" o:spid=\"_x0000_s1025\"")
            .into_bytes(),
    );
    let (_, target) = note_cut_related(&parts, "xl/worksheets/sheet2.xml", "vmlDrawing").unwrap();
    let target_xml = std::str::from_utf8(&parts[&target]).unwrap();
    parts.insert(
        target.clone(),
        target_xml
            .replace("_x0000_s3073", "_x0000_s1025")
            .into_bytes(),
    );
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    note_cut_assert_kept_shape(&before, &after, "xl/worksheets/sheet2.xml", "_x0000_s1025");
    let document = yggdryl::xml::from_bytes(&after[&target]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let moved = root
        .children_in(Some(VML), "shape")
        .into_iter()
        .find(|shape| {
            shape
                .child(Some(EXCEL), "ClientData")
                .unwrap()
                .child(Some(EXCEL), "Row")
                .unwrap()
                .text()
                .unwrap()
                .trim()
                == "5"
        })
        .unwrap();
    let id = moved.attribute_in(None, "id").unwrap();
    assert_ne!(id, "_x0000_s1025");
    assert_eq!(moved.attribute_in(Some(OFFICE), "spid"), Some(id));
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[cfg(feature = "internals")]
mod internal {

    #[test]
    fn workbook_changes_failed_attempts_restore_pending_facts_and_generation() {
        use super::{at, table_member_map};
        use yggdryl::excel::Edit;
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, changes_active, pending_changes, track_changes,
        };
        for tracked in [false, true] {
            for scenario in [
                "guard",
                "flat",
                "inner success",
                "inner failure",
                "rename",
                "remove",
                "band",
            ] {
                let mut book = Workbook::new();
                book.add_sheet("Data")
                    .unwrap()
                    .set_cell(at("A1"), 1.0)
                    .unwrap();
                book.add_sheet("Keep")
                    .unwrap()
                    .set_cell(at("A1"), 10.0)
                    .unwrap();
                if tracked {
                    for name in ["Data", "Keep"] {
                        let sheet = book.sheet_mut(name).unwrap();
                        track_changes(sheet);
                        acknowledge_changes(sheet);
                    }
                }
                // Already pending input must survive the failed attempt.
                book.sheet_mut("Data")
                    .unwrap()
                    .set_cell(at("A1"), 2.0)
                    .unwrap();
                let prior = [
                    pending_changes(book.sheet("Data").unwrap()),
                    pending_changes(book.sheet("Keep").unwrap()),
                ];
                let revisions = [
                    book.sheet("Data").unwrap().revision(),
                    book.sheet("Keep").unwrap().revision(),
                ];
                let dirty = book.is_dirty();
                let before = table_member_map(&book);
                let set = || Edit::SetEntries {
                    sheet: "Data".into(),
                    entries: vec![(at("A1"), "=3+4".into())],
                };
                let missing = || Edit::SetEntries {
                    sheet: "Missing".into(),
                    entries: vec![(at("A1"), "1".into())],
                };
                let bad = || Edit::SetEntries {
                    sheet: "Data".into(),
                    entries: vec![(at("A1"), "=5+6".into()), (at("B1"), "=(".into())],
                };
                let edit = match scenario {
                    "guard" => bad(),
                    "flat" => Edit::Batch(vec![set(), missing()]),
                    "inner success" => Edit::Batch(vec![Edit::Batch(vec![set()]), missing()]),
                    "inner failure" => Edit::Batch(vec![set(), Edit::Batch(vec![bad()])]),
                    "rename" => Edit::Batch(vec![
                        Edit::RenameSheet {
                            name: "Data".into(),
                            to: "Renamed".into(),
                        },
                        missing(),
                    ]),
                    "remove" => Edit::Batch(vec![
                        Edit::RemoveSheet {
                            name: "Data".into(),
                        },
                        missing(),
                    ]),
                    "band" => Edit::Batch(vec![
                        Edit::InsertRows {
                            sheet: "Data".into(),
                            at: 0,
                            count: 1,
                        },
                        missing(),
                    ]),
                    _ => unreachable!(),
                };
                assert!(book.apply(edit).is_err(), "{scenario}, tracked={tracked}");
                assert_eq!(
                    [
                        pending_changes(book.sheet("Data").unwrap()),
                        pending_changes(book.sheet("Keep").unwrap())
                    ],
                    prior,
                    "{scenario}, tracked={tracked}"
                );
                assert_eq!(
                    [
                        book.sheet("Data").unwrap().revision(),
                        book.sheet("Keep").unwrap().revision()
                    ],
                    revisions,
                    "{scenario}"
                );
                assert_eq!(book.is_dirty(), dirty, "{scenario}");
                assert_eq!(table_member_map(&book), before, "{scenario}");
                assert_eq!(changes_active(book.sheet("Data").unwrap()), tracked);
            }
        }
    }

    // Private resolver boundary: parsed references are resolved once for both
    // graph areas and borrowed evaluator reads, before either consumer exists.
    fn reference_resolver_book(names: &str) -> Workbook {
        use crate::excel_package as package;
        let workbook = package::workbook(&["Jan", "Feb", "Mar"], false).replace(
            "</workbook>",
            &format!("<definedNames>{names}</definedNames></workbook>"),
        );
        let sheets = [
            package::worksheet(
                r#"<row r="1"><c r="A1"><v>11</v></c></row><row r="9"><c r="C9"><v>19</v></c></row>"#,
            ),
            package::worksheet(r#"<row r="1"><c r="A1"><v>21</v></c></row>"#),
            package::worksheet(r#"<row r="1"><c r="A1"><v>31</v></c></row>"#),
        ];
        Workbook::from_bytes(package::package(&[
            (
                "[Content_Types].xml",
                package::content_types(3, false, false),
            ),
            ("_rels/.rels", package::root_relationships()),
            ("xl/workbook.xml", workbook),
            (
                "xl/_rels/workbook.xml.rels",
                package::workbook_relationships(3, false, false),
            ),
            ("xl/worksheets/sheet1.xml", sheets[0].clone()),
            ("xl/worksheets/sheet2.xml", sheets[1].clone()),
            ("xl/worksheets/sheet3.xml", sheets[2].clone()),
        ]))
        .unwrap()
    }

    #[test]
    fn reference_resolver_descriptors_reborrow_independent_ranges_after_lookup_ends() {
        use yggdryl::excel::{CellRange, CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book("");
        let at = CellRef::new(0, 0);
        let ranges = {
            let resolver = References::new(&book).unwrap();
            ["Jan:Mar!$A$1", "Jan!$C$9", "Feb!$B$8"].map(|text| {
                let formula = Formula::from_file(text, at);
                let Resolved::Range(view) = resolver.resolve("Feb", &formula, at).unwrap() else {
                    panic!("{text}: expected a resolved range");
                };
                view
            })
        };
        assert_eq!(
            std::array::from_fn::<_, 3, _>(|index| ranges[index].logical_len()),
            [3, 1, 1]
        );
        assert_eq!(
            ranges[0].areas().collect::<Vec<_>>(),
            ["Jan", "Feb", "Mar"].map(|name| {
                (
                    book.sheet_key(name).unwrap(),
                    "A1".parse::<CellRange>().unwrap(),
                )
            })
        );
        assert_eq!(
            ranges[1].areas().collect::<Vec<_>>(),
            [(
                book.sheet_key("Jan").unwrap(),
                "C9".parse::<CellRange>().unwrap()
            ),]
        );
        assert_eq!(ranges[2].cells().count(), 0);
        assert_eq!(
            ranges[2].areas().next().unwrap().1,
            "B8".parse::<CellRange>().unwrap()
        );
        for index in [1, 0, 2, 0] {
            for (key, cell) in ranges[index].cells() {
                let sheet = book.sheet(book.sheet_by_key(key).unwrap()).unwrap();
                assert!(std::ptr::eq(cell, sheet.cell(cell.reference()).unwrap()));
            }
        }
        assert_eq!(ranges[0].cells().count(), 3);
        assert_eq!(
            ranges[1].cells().next().unwrap().1.reference(),
            "C9".parse().unwrap()
        );
    }

    #[test]
    fn reference_resolver_uses_relative_mixed_and_whole_axis_geometry() {
        use yggdryl::excel::{CellRange, CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book("");
        let resolver = References::new(&book).unwrap();
        let authored: CellRef = "B2".parse().unwrap();
        let evaluated: CellRef = "D4".parse().unwrap();
        for (text, expected) in [
            ("A1", "C3"),
            ("$A1", "A3"),
            ("A$1", "C1"),
            ("$A$1", "A1"),
            ("C3:A1", "C3:E5"),
            ("$1:$3", "1:3"),
            ("$A:$C", "A:C"),
            ("1:3", "3:5"),
            ("A:C", "C:E"),
        ] {
            let formula = Formula::from_file(text, authored);
            let Resolved::Range(view) = resolver.resolve("Jan", &formula, evaluated).unwrap()
            else {
                panic!("{text} did not bind to a range");
            };
            assert_eq!(
                view.areas().collect::<Vec<_>>(),
                [(
                    book.sheet_key("Jan").unwrap(),
                    expected.parse::<CellRange>().unwrap(),
                )],
                "{text}"
            );
        }
    }

    #[test]
    fn reference_resolver_keeps_full_geometry_while_borrowing_only_stored_cells() {
        use yggdryl::excel::{CellRange, CellRef, Formula, MAX_ROWS};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book("");
        let resolver = References::new(&book).unwrap();
        let at = CellRef::new(0, 0);
        let formula = Formula::from_file("$A:$C", at);
        let Resolved::Range(view) = resolver.resolve("Jan", &formula, at).unwrap() else {
            panic!("whole columns must stay a borrowed rectangle");
        };
        assert_eq!(view.areas().count(), 1);
        assert_eq!(view.logical_len(), u128::from(MAX_ROWS) * 3);
        let cells = view.cells().collect::<Vec<_>>();
        assert_eq!(
            cells
                .iter()
                .map(|(_, cell)| cell.reference())
                .collect::<Vec<_>>(),
            [
                "A1".parse::<CellRef>().unwrap(),
                "C9".parse::<CellRef>().unwrap()
            ]
        );
        for (key, cell) in cells {
            assert_eq!(Some(key), book.sheet_key("Jan"));
            assert!(std::ptr::eq(
                cell,
                book.sheet("Jan").unwrap().cell(cell.reference()).unwrap()
            ));
        }
        let blank = Formula::from_file("$B$8", at);
        let Resolved::Range(blank) = resolver.resolve("Jan", &blank, at).unwrap() else {
            panic!("a missing physical cell still has a reference");
        };
        assert_eq!(blank.logical_len(), 1);
        assert_eq!(blank.cells().count(), 0);
        assert_eq!(
            blank.areas().next().unwrap().1,
            "B8".parse::<CellRange>().unwrap()
        );
    }

    #[test]
    fn reference_resolver_uses_quoted_names_and_current_three_dimensional_tab_order() {
        use yggdryl::excel::{CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let mut book = reference_resolver_book("");
        let at = CellRef::new(0, 0);
        let formula = Formula::from_file("Mar:Jan!$A$1", at);
        for expected in [vec!["Jan", "Feb", "Mar"], vec!["Jan", "Mar"]] {
            {
                let resolver = References::new(&book).unwrap();
                let Resolved::Range(view) = resolver.resolve("Feb", &formula, at).unwrap() else {
                    panic!("the 3D endpoints are resolved in current tab order");
                };
                assert_eq!(
                    view.areas()
                        .map(|(key, _)| book.sheet_by_key(key).unwrap())
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(view.logical_len(), expected.len() as u128);
            }
            book.move_sheet("Feb", 2).unwrap();
        }
        let key = book.sheet_key("Jan").unwrap();
        book.rename_sheet("Jan", "Q1 '24").unwrap();
        let resolver = References::new(&book).unwrap();
        let quoted = Formula::from_file("'q1 ''24'!$A$1", at);
        let Resolved::Range(view) = resolver.resolve("Mar", &quoted, at).unwrap() else {
            panic!("quoted apostrophes must reuse the existing typed sheet name");
        };
        assert_eq!(view.areas().next().unwrap().0, key);
    }

    #[test]
    fn reference_resolver_selects_local_before_global_and_borrows_name_arenas() {
        use yggdryl::excel::{CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book(concat!(
            r#"<definedName name="Rate">7</definedName>"#,
            r#"<definedName name="Rate" localSheetId="0">Jan!$A$1</definedName>"#,
            r#"<definedName name="Rate" localSheetId="1">Feb!$A$1</definedName>"#,
            r#"<definedName name="Alias" localSheetId="0">Rate</definedName>"#,
        ));
        let resolver = References::new(&book).unwrap();
        let at = CellRef::new(0, 0);
        let rate = Formula::from_file("rAtE", at);
        for (sheet, expected_scope, expected_text) in [
            ("Jan", book.sheet_key("Jan"), "Jan!$A$1"),
            ("Feb", book.sheet_key("Feb"), "Feb!$A$1"),
            ("Mar", None, "7"),
        ] {
            let Resolved::Name(binding) = resolver.resolve(sheet, &rate, at).unwrap() else {
                panic!("{sheet}: a name is an existing borrowed expression binding");
            };
            let original = book
                .defined_names()
                .find(|name| name.name() == "Rate" && name.scope() == expected_scope)
                .unwrap();
            assert!(std::ptr::eq(binding.definition(), original));
            assert_eq!(binding.definition().text(), expected_text);
        }
        let qualified = Formula::from_file("Feb!Rate", at);
        let Resolved::Name(binding) = resolver.resolve("Jan", &qualified, at).unwrap() else {
            panic!()
        };
        assert_eq!(binding.definition().scope(), book.sheet_key("Feb"));
        let alias = Formula::from_file("Alias", at);
        let Resolved::Name(binding) = resolver.resolve("Jan", &alias, at).unwrap() else {
            panic!()
        };
        let Resolved::Name(rate) = resolver.resolve_name_reference(&binding).unwrap() else {
            panic!()
        };
        // Native-defined local Alias=Rate binds the global Rate, not a local
        // shadow. Direct cell lookup above still selects local-before-global.
        assert_eq!(rate.definition().scope(), None);
        assert_eq!(rate.definition().text(), "7");
        let Resolved::Name(direct) = resolver
            .resolve("Jan", &Formula::from_file("Rate", at), at)
            .unwrap()
        else {
            panic!()
        };
        let Resolved::Range(range) = resolver.resolve_name_reference(&direct).unwrap() else {
            panic!()
        };
        assert_eq!(
            range.areas().next().unwrap(),
            (book.sheet_key("Jan").unwrap(), "A1".parse().unwrap())
        );
    }

    #[test]
    fn reference_resolver_distinguishes_missing_names_sheets_and_unsafe_name_anchors() {
        use yggdryl::excel::{CellRef, ExcelError, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book(concat!(
            r#"<definedName name="Relative">Jan!A1</definedName>"#,
            r#"<definedName name="Unqualified">$A$1</definedName>"#,
            r#"<definedName name="GlobalOnly">7</definedName>"#,
        ));
        let resolver = References::new(&book).unwrap();
        let at = CellRef::new(0, 0);
        for (text, authored, error) in [
            ("Missing!A1", at, ExcelError::Ref),
            ("NoSuchName", at, ExcelError::Name),
            ("A1", "B2".parse().unwrap(), ExcelError::Ref),
        ] {
            let formula = Formula::from_file(text, authored);
            assert!(
                matches!(resolver.resolve("Jan", &formula, at).unwrap(), Resolved::Error(actual) if actual == error),
                "{text}"
            );
        }
        let consumer: CellRef = "C7".parse().unwrap();
        let formula = Formula::from_file("Relative", consumer);
        let Resolved::Name(binding) = resolver.resolve("Jan", &formula, consumer).unwrap() else {
            panic!()
        };
        let Resolved::Range(range) = resolver.resolve_name_reference(&binding).unwrap() else {
            panic!()
        };
        assert_eq!(
            range.areas().next().unwrap(),
            (book.sheet_key("Jan").unwrap(), "C7".parse().unwrap())
        );
        let formula = Formula::from_file("Unqualified", at);
        let Resolved::Name(binding) = resolver.resolve("Jan", &formula, at).unwrap() else {
            panic!()
        };
        assert!(matches!(
            resolver.resolve_name_reference(&binding).unwrap(),
            Resolved::Held("defined-name anchor")
        ));
        let formula = Formula::from_file("Feb!GlobalOnly", at);
        assert!(matches!(
            resolver.resolve("Jan", &formula, at).unwrap(),
            Resolved::Held("qualified workbook name scope")
        ));
    }

    #[test]
    fn reference_resolver_ambiguous_referenced_name_refuses_without_mutation() {
        use yggdryl::Error;
        use yggdryl::excel::{CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book(concat!(
            r#"<definedName name="Rate" localSheetId="0">1</definedName>"#,
            r#"<definedName name="rAtE" localSheetId="0">2</definedName>"#,
            r#"<definedName name="Rate">7</definedName>"#,
        ));
        let dirty = book.is_dirty();
        let resolver = References::new(&book).unwrap();
        let at = CellRef::new(0, 0);
        let formula = Formula::from_file("Rate", at);
        assert!(matches!(
            resolver.resolve("Feb", &formula, at).unwrap(),
            Resolved::Name(_)
        ));
        match resolver.resolve("Jan", &formula, at) {
            Err(Error::InvalidRecord { path, reason }) => {
                assert!(
                    path.contains("xl/workbook.xml") && path.contains("Rate"),
                    "{path}"
                );
                assert!(reason.contains("one") && reason.contains("2"), "{reason}");
            }
            _ => panic!("two names in one scope cannot silently select the first"),
        }
        assert_eq!(book.is_dirty(), dirty);
        assert_eq!(
            book.defined_names()
                .map(|name| name.text())
                .collect::<Vec<_>>(),
            ["1", "2", "7"]
        );
    }

    #[test]
    fn reference_resolver_name_identity_distinguishes_a_cycle_from_repeated_siblings() {
        use yggdryl::excel::{CellRef, Formula};
        use yggdryl::internals::excel_workbook::{References, Resolved};
        let book = reference_resolver_book(concat!(
            r#"<definedName name="First">Second</definedName>"#,
            r#"<definedName name="Second">First</definedName>"#,
        ));
        let resolver = References::new(&book).unwrap();
        let at = CellRef::new(0, 0);
        let formula = Formula::from_file("First", at);
        let Resolved::Name(first) = resolver.resolve("Jan", &formula, at).unwrap() else {
            panic!()
        };
        let Resolved::Name(second) = resolver.resolve_name_reference(&first).unwrap() else {
            panic!()
        };
        let Resolved::Name(again) = resolver.resolve_name_reference(&second).unwrap() else {
            panic!()
        };
        assert!(!std::ptr::eq(first.definition(), second.definition()));
        assert!(std::ptr::eq(first.definition(), again.definition()));
        // Expansion ancestry belongs to the one expression stack. Resolving a
        // completed sibling again must not poison a global visited-name set.
        let Resolved::Name(sibling) = resolver.resolve("Jan", &formula, at).unwrap() else {
            panic!()
        };
        assert!(std::ptr::eq(first.definition(), sibling.definition()));
    }

    use super::{Workbook, archive, removal_member_map, save_removed_sheet_book};
    use yggdryl::holder::Holder;
    use yggdryl::internals::excel_edit::parts_edit;

    const MEMBER: &str = "customXml/item1.xml";
    const FIRST: &[u8] = b"<annotation>first</annotation>";
    const SECOND: &[u8] = b"<annotation>second</annotation>";

    #[test]
    fn historical_content_types_reconcile_live_tabs_and_shared_parts() {
        const TYPES: &str = "[Content_Types].xml";
        const TYPE_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
        const MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";
        for kind in ["chartsheet", "dialogsheet"] {
            let tab = format!("xl/{kind}s/sheet1.xml");
            let historic = super::content_types(1, false, false).replace(
                "PartName=\"/xl/worksheets/sheet1.xml\"",
                "PartName='&#47;xl/worksheets/sheet1.xml'",
            );
            let current = historic.replace(
                "</Types>",
                &format!(
                    "<Override PartName=\"/{tab}\" ContentType=\"{MIME}.{kind}+xml\"/></Types>"
                ),
            );
            let relation = format!("{}/{kind}", super::R_NS);
            let tab_xml = format!("<{kind} xmlns=\"{}\"/>", super::NS);
            let source = super::package(&[
                (TYPES, &current),
                ("_rels/.rels", &super::root_relationships()),
                ("xl/workbook.xml", &super::workbook(&["Data", "Tab"], false)),
                (
                    "xl/_rels/workbook.xml.rels",
                    &super::relationships(&[
                        ("rId1", super::WORKSHEET, "worksheets/sheet1.xml"),
                        ("rId2", &relation, &format!("{kind}s/sheet1.xml")),
                    ]),
                ),
                ("xl/worksheets/sheet1.xml", &super::number_sheet(7)),
                (&tab, &tab_xml),
            ]);
            let mut book = Workbook::from_bytes(source).unwrap();
            book.sheet_mut("Data")
                .unwrap()
                .set_cell(super::at("B1"), "fresh text")
                .unwrap();
            book.set_style(
                "Data",
                &["A1".parse().unwrap()],
                &yggdryl::excel::StylePatch {
                    bold: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
            save_removed_sheet_book(&mut book);
            // A retained pre-save overlay has none of the later shared/table
            // declarations; all live payload identities remain backed.
            book.apply(parts_edit(&book, &[(TYPES, Some(historic.as_bytes()))]))
                .unwrap();
            let written = save_removed_sheet_book(&mut book);
            let types = super::member_text(&written, TYPES);
            let document = yggdryl::xml::from_bytes(types.as_bytes()).unwrap();
            let root = yggdryl::xml::Element::root(&document).unwrap();
            let declarations = root.children_in(Some(TYPE_NS), "Override");
            let listed: std::collections::BTreeMap<_, _> = declarations
                .iter()
                .map(|entry| {
                    (
                        entry.attribute_in(None, "PartName").unwrap().to_owned(),
                        entry.attribute_in(None, "ContentType").unwrap().to_owned(),
                    )
                })
                .collect();
            assert_eq!(
                listed.len(),
                declarations.len(),
                "no duplicate normalized PartName: {types}"
            );
            for (part, suffix) in [
                ("/xl/worksheets/sheet1.xml".to_owned(), "worksheet"),
                (format!("/{tab}"), kind),
                ("/xl/sharedStrings.xml".to_owned(), "sharedStrings"),
                ("/xl/styles.xml".to_owned(), "styles"),
            ] {
                assert_eq!(
                    listed.get(&part),
                    Some(&format!("{MIME}.{suffix}+xml")),
                    "{kind}: {part}"
                );
            }
            assert!(types.contains("PartName='&#47;xl/worksheets/sheet1.xml'"));
            assert_eq!(super::member_text(&written, &tab), tab_xml);
            assert_eq!(
                super::member_text(&save_removed_sheet_book(&mut book), TYPES),
                types,
                "a second save neither duplicates nor normalizes declarations",
            );
        }
    }

    fn sheet_seven() -> Workbook {
        let types = super::content_types(1, false, false).replace("sheet1.xml", "sheet7.xml");
        Workbook::from_bytes(super::package(&[
            ("[Content_Types].xml", &types),
            ("_rels/.rels", &super::root_relationships()),
            ("xl/workbook.xml", &super::workbook(&["Existing"], false)),
            (
                "xl/_rels/workbook.xml.rels",
                &super::relationships(&[("rId1", super::WORKSHEET, "worksheets/sheet7.xml")]),
            ),
            ("xl/worksheets/sheet7.xml", &super::number_sheet(7)),
        ]))
        .unwrap()
    }

    fn attach_comment(book: &mut Workbook, number: usize, reference: &str) {
        let relationship = format!("xl/worksheets/_rels/sheet{number}.xml.rels");
        let member = format!("xl/comments/comment{number}.xml");
        let kind = format!("{}/comments", super::R_NS);
        let target = format!("../comments/comment{number}.xml");
        let relationships = super::relationships(&[("rIdNote", &kind, &target)]);
        let comments = format!(
            "<comments xmlns=\"{}\"><authors><author>Author {number}</author></authors><commentList><comment ref=\"{reference}\" authorId=\"0\"><text><t>Note {number}</t></text></comment></commentList></comments>",
            super::NS,
        );
        let types = super::member_text(&book.into_bytes().unwrap(), "[Content_Types].xml");
        let types = types.replace("</Types>", &format!(
            "<Override PartName=\"/{member}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml\"/></Types>"
        ));
        book.apply(parts_edit(
            book,
            &[
                (&relationship, Some(relationships.as_bytes())),
                (&member, Some(comments.as_bytes())),
                ("[Content_Types].xml", Some(types.as_bytes())),
            ],
        ))
        .unwrap();
    }

    fn saved_sheet_part(bytes: &[u8], name: &str) -> String {
        let workbook = super::member_text(bytes, "xl/workbook.xml");
        let document = yggdryl::xml::from_bytes(workbook.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let sheets = root.child(Some(super::NS), "sheets").unwrap();
        let sheet = sheets
            .children()
            .find(|sheet| sheet.attribute_in(None, "name") == Some(name))
            .unwrap();
        let id = sheet.attribute_in(Some(super::R_NS), "id").unwrap();
        let relationships = super::member_text(bytes, "xl/_rels/workbook.xml.rels");
        let document = yggdryl::xml::from_bytes(relationships.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let relation = root
            .children()
            .find(|relation| relation.attribute_in(None, "Id") == Some(id))
            .unwrap();
        format!("xl/{}", relation.attribute_in(None, "Target").unwrap())
    }

    #[test]
    fn fresh_identity_reads_attachments_before_the_first_save() {
        let mut book = sheet_seven();
        book.add_sheet("Fresh")
            .unwrap()
            .set_cell(super::at("A2"), 8)
            .unwrap();
        attach_comment(&mut book, 8, "A2");
        book.insert_rows("Fresh", 0, 1).unwrap();
        let bytes = save_removed_sheet_book(&mut book);
        assert_eq!(
            saved_sheet_part(&bytes, "Fresh"),
            "xl/worksheets/sheet8.xml"
        );
        assert!(super::member_text(&bytes, "xl/comments/comment8.xml").contains("ref=\"A3\""));
        assert_eq!(
            book.sheet("Fresh").unwrap().scalar(super::at("A3")),
            8.into()
        );
        assert_eq!(
            super::member_text(&bytes, "xl/worksheets/sheet7.xml"),
            super::number_sheet(7)
        );
        assert!(!book.is_dirty());
    }

    #[test]
    fn fresh_identity_stays_with_each_sheet_when_unsaved_tabs_reorder() {
        let mut book = sheet_seven();
        book.add_sheet("A")
            .unwrap()
            .set_cell(super::at("A2"), 8)
            .unwrap();
        book.add_sheet("B")
            .unwrap()
            .set_cell(super::at("B2"), 9)
            .unwrap();
        attach_comment(&mut book, 8, "A2");
        attach_comment(&mut book, 9, "B2");
        book.move_sheet("B", 0).unwrap();
        book.insert_rows("A", 0, 1).unwrap();
        let bytes = save_removed_sheet_book(&mut book);
        assert_eq!(saved_sheet_part(&bytes, "A"), "xl/worksheets/sheet8.xml");
        assert_eq!(saved_sheet_part(&bytes, "B"), "xl/worksheets/sheet9.xml");
        assert!(super::member_text(&bytes, "xl/comments/comment8.xml").contains("ref=\"A3\""));
        assert!(super::member_text(&bytes, "xl/comments/comment9.xml").contains("ref=\"B2\""));
        let reopened = Workbook::from_bytes(bytes).unwrap();
        assert_eq!(
            reopened.sheet("A").unwrap().scalar(super::at("A3")),
            8.0.into()
        );
        assert_eq!(
            reopened.sheet("B").unwrap().scalar(super::at("B2")),
            9.0.into()
        );
    }

    #[test]
    fn fresh_identity_keeps_newer_attachments_when_an_older_snapshot_is_adopted() {
        let mut book = sheet_seven();
        book.add_sheet("A").unwrap();
        let older = book.into_package().unwrap();
        book.add_sheet("B")
            .unwrap()
            .set_cell(super::at("B2"), 9)
            .unwrap();
        attach_comment(&mut book, 9, "B2");
        book.move_sheet("B", 0).unwrap();
        book.rebase(older).unwrap();
        assert!(book.is_dirty());
        book.insert_rows("B", 0, 1).unwrap();
        let bytes = save_removed_sheet_book(&mut book);
        assert_eq!(saved_sheet_part(&bytes, "A"), "xl/worksheets/sheet8.xml");
        assert_eq!(saved_sheet_part(&bytes, "B"), "xl/worksheets/sheet9.xml");
        assert!(super::member_text(&bytes, "xl/comments/comment9.xml").contains("ref=\"B3\""));
        assert_eq!(
            Workbook::from_bytes(bytes)
                .unwrap()
                .sheet("B")
                .unwrap()
                .scalar(super::at("B3")),
            9.0.into()
        );
        assert!(!book.is_dirty());
    }

    #[test]
    fn fresh_identity_does_not_adopt_an_older_sheets_image_at_a_reused_part() {
        let mut book = sheet_seven();
        book.add_sheet("A")
            .unwrap()
            .set_cell(super::at("A1"), 81)
            .unwrap();
        let older = book.into_package().unwrap();
        book.remove_sheet("A").unwrap();
        book.add_sheet("B").unwrap();
        book.rebase(older).unwrap();
        assert!(book.is_dirty());
        let bytes = save_removed_sheet_book(&mut book);
        assert_eq!(saved_sheet_part(&bytes, "B"), "xl/worksheets/sheet8.xml");
        let reopened = Workbook::from_bytes(bytes).unwrap();
        assert_eq!(reopened.sheet_names(), ["Existing", "B"]);
        assert!(reopened.sheet("B").unwrap().cell(super::at("A1")).is_none());
        assert!(!book.is_dirty());
    }

    #[test]
    fn fresh_identity_restore_refuses_a_reused_live_part_atomically() {
        use yggdryl::excel::Edit;
        let mut book = sheet_seven();
        book.add_sheet("A").unwrap();
        let removed = book.apply(Edit::RemoveSheet { name: "A".into() }).unwrap();
        book.add_sheet("B").unwrap();
        let before = removal_member_map(&book.into_bytes().unwrap());
        let state = yggdryl::internals::excel_edit::state(&book);
        let dirty = book.is_dirty();
        let Err(error) = book.apply(removed.inverse.unwrap()) else {
            panic!("a removed sheet's identity cannot be reassigned around a live owner");
        };
        assert!(matches!(error, yggdryl::Error::Conflict { .. }), "{error}");
        assert!(error.to_string().contains("sheet8.xml"), "{error}");
        assert_eq!(yggdryl::internals::excel_edit::state(&book), state);
        assert_eq!(book.is_dirty(), dirty);
        assert_eq!(removal_member_map(&book.into_bytes().unwrap()), before);
    }

    #[test]
    fn fresh_identity_reserves_relationship_owners_without_a_worksheet_member() {
        for scenario in ["overlay", "source", "older snapshot"] {
            let mut book = sheet_seven();
            book.add_sheet("A").unwrap();
            attach_comment(&mut book, 8, "A2");
            let older = (scenario == "older snapshot").then(|| book.into_package().unwrap());
            book.remove_sheet("A").unwrap();
            if scenario == "source" {
                // Saving before this sheet ever existed in the source leaves
                // carried orphan attachments; their path is still occupied.
                book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            }
            book.add_sheet("B").unwrap();
            if let Some(older) = older {
                book.rebase(older).unwrap();
            }
            book.insert_rows("B", 0, 1).unwrap();
            let bytes = save_removed_sheet_book(&mut book);
            assert_eq!(
                saved_sheet_part(&bytes, "B"),
                "xl/worksheets/sheet9.xml",
                "{scenario}"
            );
            let members = super::member_names(&bytes);
            assert!(
                !members
                    .iter()
                    .any(|name| name == "xl/worksheets/_rels/sheet9.xml.rels"),
                "{scenario}"
            );
            if members
                .iter()
                .any(|name| name == "xl/comments/comment8.xml")
            {
                assert!(
                    super::member_text(&bytes, "xl/comments/comment8.xml").contains("ref=\"A2\""),
                    "{scenario}"
                );
            }
            assert!(
                Workbook::from_bytes(bytes)
                    .unwrap()
                    .sheet("B")
                    .unwrap()
                    .is_empty(),
                "{scenario}"
            );
        }
    }

    #[test]
    fn fresh_identity_reused_path_keeps_old_snapshot_attachments_absent() {
        let mut book = sheet_seven();
        book.add_sheet("A")
            .unwrap()
            .set_cell(super::at("A1"), 81)
            .unwrap();
        attach_comment(&mut book, 8, "A2");
        let older = book.into_package().unwrap();
        book.remove_sheet("A").unwrap();
        book.apply(parts_edit(
            &book,
            &[
                ("xl/worksheets/_rels/sheet8.xml.rels", None),
                ("xl/comments/comment8.xml", None),
            ],
        ))
        .unwrap();
        book.add_sheet("B").unwrap();
        book.rebase(older).unwrap();
        book.insert_rows("B", 0, 1).unwrap();
        let bytes = save_removed_sheet_book(&mut book);
        assert_eq!(saved_sheet_part(&bytes, "B"), "xl/worksheets/sheet8.xml");
        let members = super::member_names(&bytes);
        assert!(
            !members
                .iter()
                .any(|name| name == "xl/worksheets/_rels/sheet8.xml.rels")
        );
        assert!(
            !members
                .iter()
                .any(|name| name == "xl/comments/comment8.xml")
        );
        assert!(
            Workbook::from_bytes(bytes)
                .unwrap()
                .sheet("B")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fresh_identity_restored_image_survives_a_snapshot_that_dropped_the_same_bytes() {
        use yggdryl::excel::Edit;
        for newer_undo in [false, true] {
            let mut book = sheet_seven();
            book.add_sheet("A")
                .unwrap()
                .set_cell(super::at("A2"), 8)
                .unwrap();
            attach_comment(&mut book, 8, "A2");
            let original = save_removed_sheet_book(&mut book);
            let removed = book.apply(Edit::RemoveSheet { name: "A".into() }).unwrap();
            save_removed_sheet_book(&mut book);
            let restored = book.apply(removed.inverse.unwrap()).unwrap();
            let removed_again = book.apply(restored.inverse.unwrap()).unwrap();
            let older = book.into_package().unwrap();
            if newer_undo {
                book.apply(removed_again.inverse.unwrap()).unwrap();
            }
            book.rebase(older).unwrap();
            assert_eq!(book.is_dirty(), newer_undo);
            let bytes = save_removed_sheet_book(&mut book);
            let members = super::member_names(&bytes);
            for member in [
                "xl/worksheets/sheet8.xml",
                "xl/worksheets/_rels/sheet8.xml.rels",
                "xl/comments/comment8.xml",
            ] {
                assert_eq!(
                    members.iter().any(|name| name == member),
                    newer_undo,
                    "{member}: newer undo = {newer_undo}"
                );
                if newer_undo {
                    assert_eq!(
                        super::member_text(&bytes, member),
                        super::member_text(&original, member)
                    );
                }
            }
            assert!(!book.is_dirty());
            let reopened = Workbook::from_bytes(bytes).unwrap();
            if newer_undo {
                assert_eq!(
                    reopened.sheet("A").unwrap().scalar(super::at("A2")),
                    8.0.into()
                );
            } else {
                assert_eq!(reopened.sheet_names(), ["Existing"]);
            }
        }
    }

    fn blank() -> Workbook {
        let mut book = Workbook::new();
        book.add_sheet("Sheet1").unwrap();
        save_removed_sheet_book(&mut book);
        assert!(!book.is_dirty());
        book
    }

    fn part(book: &Workbook) -> Option<Vec<u8>> {
        let package = book.into_bytes().unwrap();
        let archive = archive(&package);
        archive
            .entries()
            .unwrap()
            .iter()
            .any(|entry| entry.name() == MEMBER)
            .then(|| archive.read_member(MEMBER).unwrap())
    }

    #[test]
    fn package_overlay_new_part_undo_restores_absence_across_saves() {
        let mut book = blank();
        let original = removal_member_map(&book.into_bytes().unwrap());
        let applied = book
            .apply(parts_edit(&book, &[(MEMBER, Some(FIRST))]))
            .unwrap();
        assert!(book.is_dirty());
        assert_eq!(part(&book).as_deref(), Some(FIRST));
        let mut undo = applied.inverse.unwrap();
        assert!(undo.byte_size() >= MEMBER.len());
        for _ in 0..2 {
            save_removed_sheet_book(&mut book);
            assert!(!book.is_dirty());
            let redo = book.apply(undo).unwrap().inverse.unwrap();
            assert!(book.is_dirty());
            assert_eq!(
                part(&book),
                None,
                "absence overrides the still-present source member"
            );
            assert!(redo.byte_size() >= MEMBER.len() + FIRST.len());
            save_removed_sheet_book(&mut book);
            assert!(!book.is_dirty());
            assert_eq!(removal_member_map(&book.into_bytes().unwrap()), original);
            undo = book.apply(redo).unwrap().inverse.unwrap();
            assert_eq!(part(&book).as_deref(), Some(FIRST));
        }
    }

    #[test]
    fn package_overlay_newer_deletion_survives_adopting_a_present_snapshot() {
        let mut book = blank();
        let undo = book
            .apply(parts_edit(&book, &[(MEMBER, Some(FIRST))]))
            .unwrap()
            .inverse
            .unwrap();
        let older = book.into_package().unwrap();
        let redo = book.apply(undo).unwrap().inverse.unwrap();
        assert_eq!(part(&book), None);
        book.rebase(older).unwrap();
        assert!(book.is_dirty(), "the adopted package predates deletion");
        assert_eq!(
            part(&book),
            None,
            "adoption cannot expose the old source member"
        );
        save_removed_sheet_book(&mut book);
        assert!(!book.is_dirty());
        book.apply(redo).unwrap();
        assert_eq!(part(&book).as_deref(), Some(FIRST));
    }

    #[test]
    fn package_overlay_newer_presence_survives_adopting_an_absent_snapshot() {
        let mut book = blank();
        let undo = book
            .apply(parts_edit(&book, &[(MEMBER, Some(FIRST))]))
            .unwrap()
            .inverse
            .unwrap();
        save_removed_sheet_book(&mut book);
        let redo = book.apply(undo).unwrap().inverse.unwrap();
        let older = book.into_package().unwrap();
        book.apply(redo).unwrap();
        book.apply(parts_edit(&book, &[(MEMBER, Some(SECOND))]))
            .unwrap();
        book.rebase(older).unwrap();
        assert!(book.is_dirty(), "the adopted package predates restoration");
        assert_eq!(part(&book).as_deref(), Some(SECOND));
        save_removed_sheet_book(&mut book);
        assert!(!book.is_dirty());
        assert_eq!(part(&book).as_deref(), Some(SECOND));
    }

    #[test]
    fn package_overlay_failed_write_keeps_a_part_only_edit_dirty() {
        let mut book = Workbook::from_bytes(super::rich_package()).unwrap();
        let member = "docProps/app.xml";
        let original = super::member_text(&book.into_bytes().unwrap(), member);
        let changed = original.replace("Microsoft Excel", "Edited application");
        assert_ne!(original, changed);
        book.apply(parts_edit(&book, &[(member, Some(changed.as_bytes()))]))
            .unwrap();
        assert!(book.is_dirty());
        let folder = super::scratch("overlay-failed-write");
        let blocker = folder.join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let mut target = Holder::file(blocker.join("book.xlsx")).unwrap();
        assert!(book.write_into(&mut target).is_err());
        assert!(
            book.is_dirty(),
            "adopting the built package is not a successful write"
        );
        assert_eq!(
            super::member_text(&book.into_bytes().unwrap(), member),
            changed
        );
        save_removed_sheet_book(&mut book);
        assert!(!book.is_dirty());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn package_overlay_deleted_optional_relationships_do_not_read_the_source() {
        let mut book = blank();
        let member = "xl/worksheets/_rels/sheet1.xml.rels";
        // A carried unrelated part is not parsed by a plain save. Once
        // explicitly absent, even a structural edit must never read it.
        book.apply(parts_edit(&book, &[(member, Some(b"not XML"))]))
            .unwrap();
        save_removed_sheet_book(&mut book);
        book.apply(parts_edit(&book, &[(member, None)])).unwrap();
        book.insert_rows("Sheet1", 0, 1).unwrap();
        let bytes = save_removed_sheet_book(&mut book);
        assert!(
            !super::member_names(&bytes)
                .iter()
                .any(|name| name == member)
        );
        assert!(!book.is_dirty());
    }

    #[test]
    fn package_overlay_required_metadata_removal_is_an_atomic_refusal() {
        for member in [
            "[Content_Types].xml",
            "_rels/.rels",
            "xl/workbook.xml",
            "xl/_rels/workbook.xml.rels",
        ] {
            let mut book = blank();
            let before = removal_member_map(&book.into_bytes().unwrap());
            let Err(error) =
                book.apply(parts_edit(&book, &[(MEMBER, Some(FIRST)), (member, None)]))
            else {
                panic!("the whole restore must be validated before its first part is added");
            };
            assert!(
                matches!(error, yggdryl::Error::InvalidRecord { .. }),
                "{member}: {error}"
            );
            assert!(error.to_string().contains(member), "{error}");
            assert!(!book.is_dirty(), "{member}");
            assert_eq!(
                removal_member_map(&book.into_bytes().unwrap()),
                before,
                "{member}"
            );
        }
    }
}

/// The workbook's namespace family is resolved at its root, including an
/// alternate QName prefix and XML character references in its declaration.
#[test]
fn workbook_namespace_family_follows_the_actual_root_binding() {
    use yggdryl::excel::{STRICT_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE};

    for (prefix, namespace) in [
        ("", STRICT_NAMESPACE.to_owned()),
        ("s:", STRICT_NAMESPACE.to_owned()),
        ("s:", STRICT_NAMESPACE.replace("purl", "p&#x75;rl")),
        ("s:", NS.to_owned()),
    ] {
        let strict = namespace != NS;
        let family = if strict { STRICT_NAMESPACE } else { NS };
        let relations = if strict {
            STRICT_RELATIONSHIPS_NAMESPACE
        } else {
            R_NS
        };
        let declaration = if prefix.is_empty() {
            "xmlns"
        } else {
            "xmlns:s"
        };
        let document = format!(
            "<{prefix}workbook {declaration}=\"{namespace}\" xmlns:r=\"{relations}\">\
             <{prefix}sheets><{prefix}sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
             </{prefix}sheets></{prefix}workbook>"
        );
        let original = number_sheet(1).replace(NS, family);
        let types = content_types(1, false, false);
        let root = root_relationships().replace(R_NS, relations);
        let relationships = workbook_relationships(1, false, false).replace(R_NS, relations);
        let mut book = Workbook::from_bytes(package(&[
            ("[Content_Types].xml", &types),
            ("_rels/.rels", &root),
            ("xl/workbook.xml", &document),
            ("xl/_rels/workbook.xml.rels", &relationships),
            ("xl/worksheets/sheet1.xml", &original),
        ]))
        .unwrap();
        book.add_sheet("Fresh")
            .unwrap()
            .set_cell(at("A1"), 7.0)
            .unwrap();
        let bytes = book.into_bytes().unwrap();
        assert_eq!(member_text(&bytes, "xl/worksheets/sheet1.xml"), original);
        for (part, name) in [
            ("xl/workbook.xml", "workbook"),
            ("xl/worksheets/sheet2.xml", "worksheet"),
        ] {
            let text = member_text(&bytes, part);
            let xml = yggdryl::xml::from_bytes(text.as_bytes()).unwrap();
            assert!(
                yggdryl::xml::Element::root(&xml)
                    .unwrap()
                    .is(Some(family), name),
                "{text}"
            );
        }
        let relationships = member_text(&bytes, "xl/_rels/workbook.xml.rels");
        assert!(
            relationships.contains(&format!(
                "Type=\"{relations}/worksheet\" Target=\"worksheets/sheet2.xml\""
            )),
            "{relationships}"
        );
        let document = member_text(&bytes, "xl/workbook.xml");
        let xml = yggdryl::xml::from_bytes(document.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&xml).unwrap();
        let sheets = root.child(Some(family), "sheets").unwrap();
        let fresh = sheets
            .children()
            .find(|sheet| sheet.attribute_in(None, "name") == Some("Fresh"))
            .unwrap();
        assert!(
            fresh.attribute_in(Some(relations), "id").is_some(),
            "{document}"
        );
        let mut reopened = Workbook::from_bytes(bytes).unwrap();
        reopened.add_sheet("AfterReopen").unwrap();
        let text = member_text(&reopened.into_bytes().unwrap(), "xl/worksheets/sheet3.xml");
        let xml = yggdryl::xml::from_bytes(text.as_bytes()).unwrap();
        assert!(
            yggdryl::xml::Element::root(&xml)
                .unwrap()
                .is(Some(family), "worksheet"),
            "{text}"
        );
    }
}

#[test]
fn workbook_namespace_family_refuses_an_unresolved_root_before_reading_sheets() {
    for document in [
        "<workbook/>",
        "<workbook xmlns=\"urn:unknown\"/>",
        "<x:workbook xmlns:y=\"http://purl.oclc.org/ooxml/spreadsheetml/main\"/>",
        "<worksheet xmlns=\"http://purl.oclc.org/ooxml/spreadsheetml/main\"/>",
        "<?xml version=\"1.0\"?>",
    ] {
        let error = Workbook::from_bytes(one_sheet_under(document)).unwrap_err();
        let Error::InvalidRecord { path, reason } = error else {
            panic!("{error}")
        };
        assert_eq!(path, "xl/workbook.xml#workbook");
        assert!(reason.contains("namespace family"), "{reason}");
    }
}

#[test]
fn note_cut_retained_same_sheet_inverse_refuses_parts_transferred_to_another_sheet() {
    let mut workbook = note_cut_fixture(false);
    let first = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Data".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let undo = first.inverse.unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    workbook.add_sheet("Third").unwrap();
    workbook
        .paste(
            ("Data", "D4:F6".parse().unwrap()),
            ("Third", at("J10")),
            Paste::All,
            true,
        )
        .unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let before = table_member_map(&workbook);
    assert!(note_cut_related(&before, "xl/worksheets/sheet1.xml", "comments").is_none());
    let (_, owner) = note_cut_related(&before, "xl/worksheets/sheet3.xml", "comments").unwrap();
    assert_eq!(owner, "xl/comments1.xml");
    let revisions: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    let dirty = workbook.is_dirty();
    let result = workbook.apply(undo).map(|_| ());
    assert!(
        matches!(result, Err(Error::Conflict { .. })),
        "an unchanged original owner is still an inverse precondition: {result:?}"
    );
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(workbook.is_dirty(), dirty);
    assert_eq!(
        workbook
            .sheet_names()
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
}

const THREAD_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
const THREAD_REL: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const PERSON_REL: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
const THREAD_A: &str = "{00000000-0000-4000-8000-000000000001}";
const REPLY_A: &str = "{00000000-0000-4000-8000-000000000002}";
const REPLY_B: &str = "{00000000-0000-4000-8000-000000000003}";
const THREAD_B: &str = "{00000000-0000-4000-8000-000000000004}";
const THREAD_C: &str = "{00000000-0000-4000-8000-000000000005}";
const REPLY_C: &str = "{00000000-0000-4000-8000-000000000006}";
const PERSON_A: &str = "{00000000-0000-4000-8000-000000000101}";
const PERSON_B: &str = "{00000000-0000-4000-8000-000000000102}";
const PERSON_C: &str = "{00000000-0000-4000-8000-000000000103}";
const MENTION_A: &str = "{00000000-0000-4000-8000-000000000201}";

fn threaded_cut_fixture(root_ref: bool) -> Workbook {
    let mut parts = table_member_map(&note_cut_fixture(true));
    for (member, changes) in [
        (
            "xl/comments1.xml",
            vec![
                (
                    "<author>Author</author>",
                    format!("<author>tc={THREAD_A}</author>"),
                ),
                (
                    "<author>Guest</author>",
                    format!("<author>tc={THREAD_B}</author>"),
                ),
            ],
        ),
        (
            "xl/comments2.xml",
            vec![(
                "<author>Bob</author>",
                format!("<author>tc={THREAD_C}</author>"),
            )],
        ),
    ] {
        let mut xml = String::from_utf8(parts[member].clone()).unwrap();
        for (before, after) in changes {
            assert_eq!(xml.matches(before).count(), 1);
            xml = xml.replace(before, &after);
        }
        // MS-XLSX2.3.7.3 requires tc={id} author linkage; comment@uid is a SHOULD.
        // Omitting uid pins that the required author link is sufficient.
        parts.insert(member.into(), xml.into_bytes());
    }
    let location = if root_ref { " ref=\"B2\"" } else { "" };
    let source = format!(
        r#"<ThreadedComments xmlns="{THREAD_NS}"><threadedComment{location} dT="2024-01-01T12:00:00Z" personId="{PERSON_A}" id="{THREAD_A}" done="1"><text>Hi Bob</text><mentions><mention mentionpersonId="{PERSON_C}" mentionId="{MENTION_A}" startIndex="3" length="3"/></mentions></threadedComment><threadedComment personId="{PERSON_B}" id="{REPLY_A}" parentId="{THREAD_A}" dT="2024-01-01T12:01:00Z"><text>first reply</text></threadedComment><threadedComment ref="B2" personId="{PERSON_A}" id="{REPLY_B}" parentId="{THREAD_A}"><text>second reply</text></threadedComment><threadedComment ref="D4" personId="{PERSON_B}" id="{THREAD_B}"><text>stationary thread</text></threadedComment></ThreadedComments>"#
    );
    let target = format!(
        r#"<ThreadedComments xmlns="{THREAD_NS}"><threadedComment ref="J10" personId="{PERSON_C}" id="{THREAD_C}"><text>destination thread</text></threadedComment><threadedComment personId="{PERSON_A}" id="{REPLY_C}" parentId="{THREAD_C}"><text>destination reply</text></threadedComment></ThreadedComments>"#
    );
    for (number, xml) in [(1, source), (2, target)] {
        let member = format!("xl/threadedComments/threadedComment{number}.xml");
        parts.insert(member, xml.into_bytes());
        let member = format!("xl/worksheets/_rels/sheet{number}.xml.rels");
        let xml = std::str::from_utf8(&parts[&member]).unwrap().replace("</Relationships>", &format!("<Relationship Id=\"rIdThreads\" Type=\"{THREAD_REL}\" Target=\"../threadedComments/threadedComment{number}.xml\"/></Relationships>"));
        parts.insert(member, xml.into_bytes());
    }
    let persons = format!(
        r#"<personList xmlns="{THREAD_NS}"><person id="{PERSON_A}" displayName="Author" userId="Author" providerId="None"/><person id="{PERSON_B}" displayName="Guest" userId="Guest" providerId="None"/><person id="{PERSON_C}" displayName="Bob" userId="bob@example.invalid" providerId="PeoplePicker"/></personList>"#
    );
    parts.insert("xl/persons/person.xml".into(), persons.into_bytes());
    let rels = std::str::from_utf8(&parts["xl/_rels/workbook.xml.rels"]).unwrap().replace("</Relationships>", &format!("<Relationship Id=\"rIdPersons\" Type=\"{PERSON_REL}\" Target=\"persons/person.xml\"/></Relationships>"));
    parts.insert("xl/_rels/workbook.xml.rels".into(), rels.into_bytes());
    let declarations = format!("<Override PartName=\"/xl/persons/person.xml\" ContentType=\"application/vnd.ms-excel.person+xml\"/>{}", (1..=2).map(|number| format!("<Override PartName=\"/xl/threadedComments/threadedComment{number}.xml\" ContentType=\"application/vnd.ms-excel.threadedcomments+xml\"/>")).collect::<String>());
    let types = std::str::from_utf8(&parts["[Content_Types].xml"])
        .unwrap()
        .replace("</Types>", &format!("{declarations}</Types>"));
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    cross_table_reopen(&parts)
}

/// Resolve this fixture's threaded part through its actual worksheet relationship.
fn threaded_cut_member(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
) -> Option<String> {
    let (folder, file) = sheet.rsplit_once('/').unwrap();
    let relationships = parts.get(&format!("{folder}/_rels/{file}.rels"))?;
    let rows = cross_table_relationships(relationships);
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|row| &row["Id"]).collect();
    assert_eq!(ids.len(), rows.len(), "unique relationship IDs");
    let related: Vec<_> = rows
        .iter()
        .filter(|row| row["Type"] == THREAD_REL)
        .collect();
    assert!(related.len() <= 1, "one threaded part per worksheet");
    let row = related.first()?;
    assert!(!row.contains_key("TargetMode"));
    let mut path: Vec<_> = if row["Target"].starts_with('/') {
        Vec::new()
    } else {
        folder.split('/').collect()
    };
    for segment in row["Target"].split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                assert!(path.pop().is_some());
            }
            value => path.push(value),
        }
    }
    let member = path.join("/");
    assert!(
        parts.contains_key(&member),
        "missing threaded target: {member}"
    );
    Some(member)
}

/// Each row retains all identity/status/time fields and all child payloads.
type ThreadedCutRow = (
    std::collections::BTreeMap<String, String>,
    Vec<(String, String, Scalar)>,
);

fn threaded_cut_rows(bytes: &[u8]) -> std::collections::BTreeMap<String, ThreadedCutRow> {
    let doc = yggdryl::xml::from_bytes(bytes).unwrap();
    let root = yggdryl::xml::Element::root(&doc).unwrap();
    assert!(root.is(Some(THREAD_NS), "ThreadedComments"));
    let mut rows = std::collections::BTreeMap::new();
    for entry in root.children_in(Some(THREAD_NS), "threadedComment") {
        let id = entry.attribute_in(None, "id").unwrap().to_owned();
        let attributes = ["id", "personId", "parentId", "ref", "dT", "done"]
            .into_iter()
            .filter_map(|name| {
                entry
                    .attribute_in(None, name)
                    .map(|value| (name.to_owned(), value.to_owned()))
            })
            .collect();
        let children = entry
            .children()
            .map(|child| {
                (
                    child.namespace().unwrap().to_owned(),
                    child.local_name().to_owned(),
                    child.value().clone(),
                )
            })
            .collect();
        assert!(
            rows.insert(id, (attributes, children)).is_none(),
            "unique thread message IDs"
        );
    }
    rows
}

fn threaded_cut_assert_entries(
    before: &std::collections::BTreeMap<String, Vec<u8>>,
    after: &std::collections::BTreeMap<String, Vec<u8>>,
    sheet: &str,
    expected: &[(&str, Option<&str>)],
) {
    let mut original = std::collections::BTreeMap::new();
    for sheet in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        if let Some(member) = threaded_cut_member(before, sheet) {
            for (id, entry) in threaded_cut_rows(&before[&member]) {
                assert!(original.insert(id, entry).is_none());
            }
        }
    }
    let actual = match threaded_cut_member(after, sheet) {
        Some(member) => threaded_cut_rows(&after[&member]),
        None => {
            assert!(expected.is_empty());
            return;
        }
    };
    let expected: std::collections::BTreeMap<_, _> = expected
        .iter()
        .map(|(id, reference)| {
            let mut entry = original[*id].clone();
            if let Some(reference) = reference {
                entry.0.insert("ref".into(), (*reference).into());
            } else {
                entry.0.remove("ref");
            }
            ((*id).to_owned(), entry)
        })
        .collect();
    assert_eq!(
        actual, expected,
        "{sheet}: refs change, messages/person/parent/mentions remain exact"
    );
}

fn threaded_cut_assert_graph(
    before: &std::collections::BTreeMap<String, Vec<u8>>,
    after: &std::collections::BTreeMap<String, Vec<u8>>,
    sheets: &[&str],
) {
    assert_eq!(
        after["xl/persons/person.xml"], before["xl/persons/person.xml"],
        "same-workbook moves never rewrite shared persons"
    );
    let person_row = |parts: &std::collections::BTreeMap<String, Vec<u8>>| {
        let rows = cross_table_relationships(&parts["xl/_rels/workbook.xml.rels"]);
        let persons: Vec<_> = rows
            .into_iter()
            .filter(|row| row["Type"] == PERSON_REL)
            .collect();
        assert_eq!(persons.len(), 1);
        persons.into_iter().next().unwrap()
    };
    assert_eq!(person_row(after), person_row(before));
    let owned: std::collections::BTreeSet<_> = sheets
        .iter()
        .filter_map(|sheet| threaded_cut_member(after, sheet))
        .collect();
    let present: std::collections::BTreeSet<_> = after
        .iter()
        .filter_map(|(name, bytes)| {
            if !name.ends_with(".xml") {
                return None;
            }
            let doc = yggdryl::xml::from_bytes(bytes).unwrap();
            let root = yggdryl::xml::Element::root(&doc).unwrap();
            root.is(Some(THREAD_NS), "ThreadedComments")
                .then(|| name.clone())
        })
        .collect();
    assert_eq!(
        present, owned,
        "empty thread parts disappear, live threads keep one owner"
    );
    let doc = yggdryl::xml::from_bytes(&after["[Content_Types].xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&doc).unwrap();
    let declared: std::collections::BTreeSet<_> = root
        .children()
        .filter(|entry| {
            entry.attribute_in(None, "ContentType")
                == Some("application/vnd.ms-excel.threadedcomments+xml")
        })
        .map(|entry| {
            entry
                .attribute_in(None, "PartName")
                .unwrap()
                .trim_start_matches('/')
                .to_owned()
        })
        .collect();
    assert_eq!(declared, owned);
    note_cut_assert_graph(after, sheets);
}

#[test]
fn threaded_cut_same_sheet_moves_the_whole_chain_and_placeholder_without_cells() {
    for root_ref in [true, false] {
        let mut workbook = threaded_cut_fixture(root_ref);
        assert!(workbook.sheet("Data").unwrap().cell(at("B2")).is_none());
        let before = table_member_map(&workbook);
        let edit = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: ("Data".into(), "B2".parse().unwrap()),
                to: ("Data".into(), at("F6")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        // A missing ref stays omitted; association follows the required placeholder link.
        threaded_cut_assert_entries(
            &before,
            &after,
            "xl/worksheets/sheet1.xml",
            &[
                (THREAD_A, root_ref.then_some("F6")),
                (REPLY_A, None),
                (REPLY_B, Some("F6")),
                (THREAD_B, Some("D4")),
            ],
        );
        threaded_cut_assert_entries(
            &before,
            &after,
            "xl/worksheets/sheet2.xml",
            &[(THREAD_C, Some("J10")), (REPLY_C, None)],
        );
        note_cut_assert_comments(
            &after,
            "xl/worksheets/sheet1.xml",
            &[&format!("tc={THREAD_A}"), &format!("tc={THREAD_B}")],
            &[("F6", 0, "Author: list price"), ("D4", 1, "stationary")],
        );
        note_cut_assert_vml(
            &after,
            "xl/worksheets/sheet1.xml",
            &[
                // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
                ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
                ((3, 3), [4, 15, 2, 2, 6, 15, 6, 16]),
            ],
        );
        threaded_cut_assert_graph(
            &before,
            &after,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
    }
}

#[test]
fn threaded_cut_cross_sheet_partitions_replies_and_overwrites_the_destination_chain() {
    let mut workbook = threaded_cut_fixture(true);
    let before = table_member_map(&workbook);
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet1.xml",
        &[(THREAD_B, Some("D4"))],
    );
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet2.xml",
        &[
            (THREAD_A, Some("J10")),
            (REPLY_A, None),
            (REPLY_B, Some("J10")),
        ],
    );
    note_cut_assert_comments(
        &after,
        "xl/worksheets/sheet2.xml",
        &[&format!("tc={THREAD_C}"), &format!("tc={THREAD_A}")],
        &[("J10", 1, "Author: list price")],
    );
    // The placeholder's B2 VML comes from native rich_vml.xml; the J10
    // move adds eight to [2,12,0,11,4,6,5,8]'s indices, retaining its offsets.
    note_cut_assert_vml(
        &after,
        "xl/worksheets/sheet2.xml",
        &[((9, 9), [10, 12, 8, 11, 12, 6, 13, 8])],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

#[test]
fn threaded_cut_all_to_fresh_moves_ownership_and_saved_undo_restores_absence() {
    let mut workbook = threaded_cut_fixture(true);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    let original = threaded_cut_member(&before, "xl/worksheets/sheet1.xml").unwrap();
    assert!(threaded_cut_member(&before, "xl/worksheets/sheet3.xml").is_none());
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2:D4".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    assert!(threaded_cut_member(&after, "xl/worksheets/sheet1.xml").is_none());
    assert_eq!(
        threaded_cut_member(&after, "xl/worksheets/sheet3.xml").as_deref(),
        Some(original.as_str())
    );
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet3.xml",
        &[
            (THREAD_A, Some("F6")),
            (REPLY_A, None),
            (REPLY_B, Some("F6")),
            (THREAD_B, Some("H8")),
        ],
    );
    note_cut_assert_comments(
        &after,
        "xl/worksheets/sheet3.xml",
        &[&format!("tc={THREAD_A}"), &format!("tc={THREAD_B}")],
        &[("F6", 0, "Author: list price"), ("H8", 1, "stationary")],
    );
    note_cut_assert_vml(
        &after,
        "xl/worksheets/sheet3.xml",
        &[
            // Native B2 anchor +4 rows/+4 columns to F6; offsets are unchanged.
            ((5, 5), [6, 12, 4, 11, 8, 6, 9, 8]),
            ((7, 7), [8, 15, 6, 2, 10, 15, 10, 16]),
        ],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

#[test]
fn retained_owner_note_band_inverse_refuses_parts_transferred_to_another_sheet() {
    use yggdryl::excel::Edit;

    let mut workbook = note_cut_fixture(false);
    let undo = workbook
        .apply(Edit::InsertRows {
            sheet: "Data".into(),
            at: 0,
            count: 1,
        })
        .unwrap()
        .inverse
        .unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    workbook.add_sheet("Third").unwrap();
    workbook
        .paste(
            ("Data", "B3:D5".parse().unwrap()),
            ("Third", at("J10")),
            Paste::All,
            true,
        )
        .unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let before = table_member_map(&workbook);
    assert!(note_cut_related(&before, "xl/worksheets/sheet1.xml", "comments").is_none());
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet3.xml", "comments")
            .unwrap()
            .1,
        "xl/comments1.xml"
    );
    let revisions: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    assert!(!workbook.is_dirty());
    assert!(
        matches!(workbook.apply(undo), Err(Error::Conflict { .. })),
        "a band's retained note payload still requires its original owner"
    );
    assert_eq!(table_member_map(&workbook), before);
    assert!(!workbook.is_dirty());
    assert_eq!(
        workbook
            .sheet_names()
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
}

#[test]
fn retained_owner_table_move_inverse_refuses_parts_transferred_to_another_sheet() {
    retained_table_owner(false);
}

#[test]
fn retained_owner_table_band_inverse_refuses_parts_transferred_to_another_sheet() {
    retained_table_owner(true);
}

fn retained_table_owner(band: bool) {
    use yggdryl::excel::Edit;

    let mut workbook = with_cut_tables(&[("Data", costs_cut_part())]);
    let (first, range) = if band {
        (
            Edit::InsertRows {
                sheet: "Data".into(),
                at: 0,
                count: 1,
            },
            "E3:F7",
        )
    } else {
        (
            Edit::Paste {
                from: ("Data".into(), "E2:F6".parse().unwrap()),
                to: ("Data".into(), at("E10")),
                what: Paste::All,
                cut: true,
            },
            "E10:F14",
        )
    };
    let undo = workbook.apply(first).unwrap().inverse.unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    workbook
        .paste(
            ("Data", range.parse().unwrap()),
            ("Other", at("J10")),
            Paste::All,
            true,
        )
        .unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let before = table_member_map(&workbook);
    assert!(cross_table_ids(&before["xl/worksheets/sheet1.xml"]).is_empty());
    assert_eq!(
        note_cut_related(&before, "xl/worksheets/sheet2.xml", "table")
            .unwrap()
            .1,
        "xl/tables/table1.xml"
    );
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    assert!(!workbook.is_dirty());
    assert!(
        matches!(workbook.apply(undo), Err(Error::Conflict { .. })),
        "a retained table payload still requires its original owner; band={band}"
    );
    assert_eq!(table_member_map(&workbook), before, "band={band}");
    assert!(!workbook.is_dirty());
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
}

fn threaded_cut_assert_invalid_thread_part(edit: impl FnOnce(String) -> String) {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let member = threaded_cut_member(&parts, "xl/worksheets/sheet1.xml").unwrap();
    let changed = edit(String::from_utf8(parts[&member].clone()).unwrap());
    parts.insert(member.clone(), changed.into_bytes());
    threaded_cut_assert_invalid_package(&parts, &member);
}

#[test]
fn threaded_cut_duplicate_message_id_refuses_atomically() {
    threaded_cut_assert_invalid_thread_part(|xml| {
        xml.replace(&format!("id=\"{REPLY_A}\""), &format!("id=\"{THREAD_A}\""))
    });
}

#[test]
fn threaded_cut_parent_cycle_refuses_atomically() {
    threaded_cut_assert_invalid_thread_part(|xml| {
        xml.replace(
            &format!("id=\"{THREAD_A}\" done="),
            &format!("id=\"{THREAD_A}\" parentId=\"{REPLY_A}\" done="),
        )
    });
}

#[test]
fn threaded_cut_dangling_parent_refuses_atomically() {
    threaded_cut_assert_invalid_thread_part(|xml| {
        xml.replace(
            &format!("parentId=\"{THREAD_A}\""),
            &format!("parentId=\"{THREAD_C}\""),
        )
    });
}

#[test]
fn threaded_cut_malformed_guid_refuses_atomically() {
    threaded_cut_assert_invalid_thread_part(|xml| {
        xml.replace(&format!("id=\"{REPLY_A}\""), "id=\"not-a-guid\"")
    });
}

#[test]
fn threaded_cut_missing_person_refuses_atomically() {
    threaded_cut_assert_invalid_thread_part(|xml| {
        xml.replace(
            &format!("personId=\"{PERSON_B}\""),
            "personId=\"{00000000-0000-4000-8000-000000000999}\"",
        )
    });
}

#[test]
fn adding_sheet_after_removed_table_owner_can_undo_after_save() {
    use yggdryl::excel::Edit;

    let mut workbook = with_cut_tables(&[("Data", costs_cut_part())]);
    workbook.add_sheet("Keep").unwrap();
    workbook.remove_sheet("Data").unwrap();
    workbook.remove_sheet("Other").unwrap();

    // remove_sheet leaves the old table in the Referring cache, even though
    // its worksheet no longer owns the table. The old table still says
    // Other!B3, so naming("Other") captures that orphan in AddSheet's undo.
    let added = workbook
        .apply(Edit::AddSheet {
            name: Some("Other".into()),
            at: None,
        })
        .unwrap();
    let undo = added.inverse.unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let before = table_member_map(&workbook);
    assert!(!before.contains_key("xl/tables/table1.xml"));

    // Undo should remove the newly added Other sheet. The stale snapshot
    // instead expects the orphan table part to be present and conflicts.
    workbook.apply(undo).unwrap();
    assert_eq!(workbook.sheet_names(), ["Keep"]);
}

#[test]
fn adding_sheet_after_replaced_table_owner_can_undo_after_save() {
    use yggdryl::excel::{Edit, Sheet};

    let table = costs_cut_part().replace("Other!B3", "Future!B3");
    let mut workbook = with_cut_tables(&[("Data", table)]);
    workbook.rename_sheet("Data", "DataTemp").unwrap();
    workbook
        .insert_sheet(Sheet::new("DataTemp").unwrap())
        .unwrap();

    // The replacement has a new SheetKey and part, but the cached Referring
    // still lists table1 under the replaced sheet's key. Its Future!B3
    // formula makes AddSheet's inverse capture that now-orphaned part.
    let added = workbook
        .apply(Edit::AddSheet {
            name: Some("Future".into()),
            at: None,
        })
        .unwrap();
    let undo = added.inverse.unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let before = table_member_map(&workbook);
    assert!(!before.contains_key("xl/tables/table1.xml"));

    workbook.apply(undo).unwrap();
    assert_eq!(workbook.sheet_names(), ["DataTemp", "Other"]);
}

#[test]
fn threaded_cut_messages_stay_before_trailing_extensions_and_follow_placeholder_location() {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    const EXT: &str = "<extLst><ext uri='vendor-thread-state'><x:state xmlns:x='urn:thread-vendor' x:flag=' exact '/></ext></extLst>";
    for number in 1..=2 {
        let member = format!("xl/threadedComments/threadedComment{number}.xml");
        let mut xml = String::from_utf8(parts[&member].clone()).unwrap();
        if number == 1 {
            // A valid but stale explicit ref is not the display location:
            // MS-XLSX2.3.7.3 makes the unique legacy placeholder authoritative.
            xml = xml.replace("ref=\"B2\"", "ref=\"C3\"");
        }
        xml = xml.replace("</ThreadedComments>", &format!("{EXT}</ThreadedComments>"));
        parts.insert(member, xml.into_bytes());
    }
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    for (sheet, count) in [
        ("xl/worksheets/sheet1.xml", 1),
        ("xl/worksheets/sheet2.xml", 3),
    ] {
        let member = threaded_cut_member(&after, sheet).unwrap();
        let xml = std::str::from_utf8(&after[&member]).unwrap();
        assert!(xml.contains(EXT), "extension bytes remain untouched");
        // The natural XML value groups unlike children by name; schema order
        // must be checked on the authored event stream instead.
        let mut reader = quick_xml::Reader::from_str(xml);
        let mut depth = 0;
        let mut names = Vec::new();
        loop {
            use quick_xml::events::Event;
            match reader.read_event().unwrap() {
                Event::Start(start) => {
                    if depth == 1 {
                        names
                            .push(String::from_utf8(start.local_name().as_ref().to_vec()).unwrap());
                    }
                    depth += 1;
                }
                Event::Empty(start) if depth == 1 => {
                    names.push(String::from_utf8(start.local_name().as_ref().to_vec()).unwrap());
                }
                Event::End(_) => depth -= 1,
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(names.len(), count + 1);
        assert!(names[..count].iter().all(|name| name == "threadedComment"));
        assert_eq!(names[count], "extLst", "CT_ThreadedComments schema order");
    }
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet1.xml",
        &[(THREAD_B, Some("D4"))],
    );
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet2.xml",
        &[
            (THREAD_A, Some("J10")),
            (REPLY_A, None),
            (REPLY_B, Some("J10")),
        ],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

#[test]
fn threaded_cut_partial_to_fresh_splits_the_message_part_and_restores_absence() {
    let mut workbook = threaded_cut_fixture(true);
    workbook.add_sheet("Fresh").unwrap();
    let before = table_member_map(&workbook);
    let original = threaded_cut_member(&before, "xl/worksheets/sheet1.xml").unwrap();
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Fresh".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let split = threaded_cut_member(&after, "xl/worksheets/sheet3.xml").unwrap();
    assert_ne!(split, original);
    assert_eq!(
        threaded_cut_member(&after, "xl/worksheets/sheet1.xml").unwrap(),
        original
    );
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet1.xml",
        &[(THREAD_B, Some("D4"))],
    );
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet3.xml",
        &[
            (THREAD_A, Some("F6")),
            (REPLY_A, None),
            (REPLY_B, Some("F6")),
        ],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "xl/worksheets/sheet3.xml",
        ],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

#[test]
fn threaded_cut_blank_source_clears_whole_destination_chains() {
    for (sheet, target) in [("Data", "B2"), ("Other", "J10")] {
        let mut workbook = threaded_cut_fixture(true);
        let before = table_member_map(&workbook);
        let edit = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: ("Data".into(), "A1".parse().unwrap()),
                to: (sheet.into(), at(target)),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        if sheet == "Data" {
            threaded_cut_assert_entries(
                &before,
                &after,
                "xl/worksheets/sheet1.xml",
                &[(THREAD_B, Some("D4"))],
            );
            threaded_cut_assert_entries(
                &before,
                &after,
                "xl/worksheets/sheet2.xml",
                &[(THREAD_C, Some("J10")), (REPLY_C, None)],
            );
        } else {
            threaded_cut_assert_entries(
                &before,
                &after,
                "xl/worksheets/sheet1.xml",
                &[
                    (THREAD_A, Some("B2")),
                    (REPLY_A, None),
                    (REPLY_B, Some("B2")),
                    (THREAD_B, Some("D4")),
                ],
            );
            threaded_cut_assert_entries(&before, &after, "xl/worksheets/sheet2.xml", &[]);
        }
        threaded_cut_assert_graph(
            &before,
            &after,
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
        );
        note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
    }
}

#[test]
fn threaded_cut_resolves_out_of_order_nested_replies_once() {
    let mut parts = table_member_map(&threaded_cut_fixture(false));
    let member = threaded_cut_member(&parts, "xl/worksheets/sheet1.xml").unwrap();
    let mut xml = String::from_utf8(parts[&member].clone()).unwrap();
    xml = xml.replace(
        &format!("id=\"{REPLY_B}\" parentId=\"{THREAD_A}\""),
        &format!("id=\"{REPLY_B}\" parentId=\"{REPLY_A}\""),
    );
    let start = xml
        .find(&format!(
            "<threadedComment personId=\"{PERSON_B}\" id=\"{REPLY_A}\""
        ))
        .unwrap();
    let end = start + xml[start..].find("</threadedComment>").unwrap() + "</threadedComment>".len();
    let reply = xml[start..end].to_owned();
    xml.replace_range(start..end, "");
    let start = xml.find('>').unwrap() + 1;
    xml.insert_str(start, &reply);
    parts.insert(member, xml.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Data".into(), at("F6")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet1.xml",
        &[
            (THREAD_A, None),
            (REPLY_A, None),
            (REPLY_B, Some("F6")),
            (THREAD_B, Some("D4")),
        ],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

#[test]
fn threaded_cut_accepts_resolved_alternate_prefixes_for_messages_and_people() {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let encoded = THREAD_NS.replace("2018/", "2018&#x2f;");
    for (member, prefix, names) in [
        (
            "xl/threadedComments/threadedComment1.xml",
            "t",
            &[
                "ThreadedComments",
                "threadedComment",
                "text",
                "mentions",
                "mention",
            ][..],
        ),
        (
            "xl/threadedComments/threadedComment2.xml",
            "q",
            &[
                "ThreadedComments",
                "threadedComment",
                "text",
                "mentions",
                "mention",
            ][..],
        ),
        ("xl/persons/person.xml", "p", &["personList", "person"][..]),
    ] {
        let mut xml = String::from_utf8(parts[member].clone()).unwrap();
        xml = xml.replace(
            &format!("xmlns=\"{THREAD_NS}\""),
            &format!("xmlns:{prefix}=\"{encoded}\""),
        );
        for name in names {
            xml = xml
                .replace(&format!("<{name}"), &format!("<{prefix}:{name}"))
                .replace(&format!("</{name}"), &format!("</{prefix}:{name}"));
        }
        parts.insert(member.into(), xml.into_bytes());
    }
    let mut workbook = cross_table_reopen(&parts);
    let before = table_member_map(&workbook);
    let edit = workbook
        .apply(yggdryl::excel::Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    threaded_cut_assert_entries(
        &before,
        &after,
        "xl/worksheets/sheet2.xml",
        &[
            (THREAD_A, Some("J10")),
            (REPLY_A, None),
            (REPLY_B, Some("J10")),
        ],
    );
    threaded_cut_assert_graph(
        &before,
        &after,
        &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"],
    );
    note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
}

fn threaded_cut_assert_invalid_package(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    member: &str,
) {
    let mut workbook = cross_table_reopen(parts);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    let dirty = workbook.is_dirty();
    match workbook
        .paste(
            ("Data", "B2".parse().unwrap()),
            ("Other", at("F6")),
            Paste::All,
            true,
        )
        .unwrap_err()
    {
        Error::InvalidRecord { path, reason } => {
            assert!(path.contains(member), "located protocol error: {path}");
            assert!(!reason.is_empty());
        }
        error => panic!("expected a typed located invalid threaded package, got {error}"),
    }
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(workbook.is_dirty(), dirty);
    assert_eq!(
        workbook
            .sheet_names()
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
}

#[test]
fn threaded_cut_ambiguous_placeholder_author_refuses_atomically() {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let member = "xl/comments1.xml";
    let xml = std::str::from_utf8(&parts[member]).unwrap().replace(
        &format!("<author>tc={THREAD_A}</author>"),
        &format!("<author>tc={THREAD_A} tc={THREAD_B}</author>"),
    );
    parts.insert(member.into(), xml.into_bytes());
    threaded_cut_assert_invalid_package(&parts, "xl/threadedComments/threadedComment1.xml");
}

#[test]
fn threaded_cut_person_and_mention_protocol_links_refuse_atomically() {
    for case in 0..3 {
        let mut parts = table_member_map(&threaded_cut_fixture(true));
        let (member, before, after, located) = match case {
            0 => (
                "xl/persons/person.xml",
                format!("id=\"{PERSON_B}\""),
                format!("id=\"{PERSON_A}\""),
                "xl/persons/person.xml",
            ),
            1 => (
                "xl/threadedComments/threadedComment1.xml",
                format!("mentionpersonId=\"{PERSON_C}\""),
                "mentionpersonId=\"{00000000-0000-4000-8000-000000000999}\"".to_owned(),
                "xl/threadedComments/threadedComment1.xml",
            ),
            _ => (
                "xl/persons/person.xml",
                "providerId=\"PeoplePicker\"".to_owned(),
                "providerId=\"None\"".to_owned(),
                "xl/threadedComments/threadedComment1.xml",
            ),
        };
        let xml = std::str::from_utf8(&parts[member])
            .unwrap()
            .replace(&before, &after);
        parts.insert(member.into(), xml.into_bytes());
        threaded_cut_assert_invalid_package(&parts, located);
    }
}

#[test]
fn threaded_cut_outbound_protocol_part_refuses_atomically() {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let member = "xl/threadedComments/_rels/threadedComment1.xml.rels";
    // MS-XLSX2.1.17 prohibits outgoing relationships to protocol-defined
    // parts. The persons part is explicitly defined by MS-XLSX2.1.18;
    // this does not assume the same prohibition covers arbitrary custom data.
    parts.insert(member.into(), format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdInvalidPerson\" Type=\"{PERSON_REL}\" Target=\"../persons/person.xml\"/></Relationships>"
    ).into_bytes());
    threaded_cut_assert_invalid_package(&parts, member);
}

#[test]
fn threaded_cut_unknown_thread_relationship_family_never_moves_only_the_placeholder() {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let member = "xl/worksheets/_rels/sheet1.xml.rels";
    let xml = std::str::from_utf8(&parts[member]).unwrap().replace(
        THREAD_REL,
        "https://vendor.invalid/relationships/threadedComment",
    );
    parts.insert(member.into(), xml.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    let dirty = workbook.is_dirty();
    match workbook
        .paste(
            ("Data", "B2".parse().unwrap()),
            ("Other", at("F6")),
            Paste::All,
            true,
        )
        .unwrap_err()
    {
        Error::Unsupported {
            operation,
            filesystem,
        } => {
            assert!(
                operation.contains("thread"),
                "named protocol boundary: {operation}"
            );
            assert!(
                filesystem.contains(member),
                "located relationship: {filesystem}"
            );
        }
        error => panic!("expected a located unsupported threaded family, got {error}"),
    }
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(workbook.is_dirty(), dirty);
    assert_eq!(
        workbook
            .sheet_names()
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
}

fn threaded_cut_custom_edge_fixture() -> Workbook {
    let mut parts = table_member_map(&threaded_cut_fixture(true));
    let rels = "xl/threadedComments/_rels/threadedComment1.xml.rels";
    parts.insert(rels.into(), br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdCustom" Type="urn:vendor:thread-resource" Target="../custom/thread-info.xml"/></Relationships>"#.to_vec());
    parts.insert(
        "xl/custom/thread-info.xml".into(),
        b"<resource xmlns='urn:vendor:thread-resource'>opaque payload</resource>".to_vec(),
    );
    let member = "xl/threadedComments/threadedComment1.xml";
    let xml = std::str::from_utf8(&parts[member]).unwrap().replace(
        "</mentions></threadedComment>",
        "</mentions><extLst><ext uri='vendor-message-resource'><x:data xmlns:x='urn:vendor:thread-resource' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' r:id='rIdCustom'/></ext></extLst></threadedComment>",
    );
    parts.insert(member.into(), xml.into_bytes());
    let types = std::str::from_utf8(&parts["[Content_Types].xml"]).unwrap().replace("</Types>", "<Override PartName='/xl/custom/thread-info.xml' ContentType='application/vnd.vendor.thread-resource+xml'/></Types>");
    parts.insert("[Content_Types].xml".into(), types.into_bytes());
    cross_table_reopen(&parts)
}

#[test]
fn threaded_cut_custom_outbound_edges_stay_exact_for_same_sheet_and_whole_member_moves() {
    for whole in [false, true] {
        let mut workbook = threaded_cut_custom_edge_fixture();
        if whole {
            workbook.add_sheet("Fresh").unwrap();
        }
        let before = table_member_map(&workbook);
        let edit = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: (
                    "Data".into(),
                    if whole { "B2:D4" } else { "B2" }.parse().unwrap(),
                ),
                to: (if whole { "Fresh" } else { "Data" }.into(), at("F6")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        let sheet = if whole {
            "xl/worksheets/sheet3.xml"
        } else {
            "xl/worksheets/sheet1.xml"
        };
        assert_eq!(
            threaded_cut_member(&after, sheet).as_deref(),
            Some("xl/threadedComments/threadedComment1.xml")
        );
        for member in [
            "xl/threadedComments/_rels/threadedComment1.xml.rels",
            "xl/custom/thread-info.xml",
        ] {
            assert_eq!(
                after[member], before[member],
                "unchanged member-relative dependency: {member}"
            );
        }
        threaded_cut_assert_entries(
            &before,
            &after,
            sheet,
            &[
                (THREAD_A, Some("F6")),
                (REPLY_A, None),
                (REPLY_B, Some("F6")),
                (THREAD_B, Some(if whole { "H8" } else { "D4" })),
            ],
        );
        let sheets = if whole {
            &[
                "xl/worksheets/sheet1.xml",
                "xl/worksheets/sheet2.xml",
                "xl/worksheets/sheet3.xml",
            ][..]
        } else {
            &["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"][..]
        };
        threaded_cut_assert_graph(&before, &after, sheets);
        note_cut_saved_inverse(&mut workbook, &before, edit.inverse.unwrap());
    }
}

#[test]
fn threaded_cut_custom_outbound_edges_refuse_partial_transfer_to_existing_atomically() {
    threaded_cut_refuse_custom_transfer(false, false);
}

#[test]
fn threaded_cut_custom_outbound_edges_refuse_partial_transfer_to_fresh_atomically() {
    threaded_cut_refuse_custom_transfer(true, false);
}

#[test]
fn threaded_cut_custom_outbound_edges_refuse_whole_transfer_into_another_part_atomically() {
    threaded_cut_refuse_custom_transfer(false, true);
}

fn threaded_cut_refuse_custom_transfer(fresh: bool, whole: bool) {
    let mut workbook = threaded_cut_custom_edge_fixture();
    if fresh {
        workbook.add_sheet("Fresh").unwrap();
    }
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    let dirty = workbook.is_dirty();
    match workbook
        .paste(
            ("Data", if whole { "B2:D4" } else { "B2" }.parse().unwrap()),
            (if fresh { "Fresh" } else { "Other" }, at("F6")),
            Paste::All,
            true,
        )
        .unwrap_err()
    {
        Error::Unsupported {
            operation,
            filesystem,
        } => {
            assert!(operation.contains("thread"));
            assert!(
                filesystem.contains("xl/threadedComments/_rels/threadedComment1.xml.rels"),
                "located custom dependency: {filesystem}"
            );
        }
        error => {
            panic!("expected a located unsupported custom threaded dependency, got {error}")
        }
    }
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(workbook.is_dirty(), dirty);
    assert_eq!(
        workbook
            .sheet_names()
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
}

// These assertions pin preserved binding identity under the cut contract;
// they are not observations of Excel's exact Cut-generated formula spelling.

fn scoped_name_cut_book(
    source_local: bool,
    destination_local: bool,
    global: bool,
    table: bool,
) -> Workbook {
    let tables = if table {
        vec![(
            "Data",
            cut_table_part(
                "Costs",
                11,
                ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
                [
                    "E3+Rate+Costs[[#This Row],[Cost]]+N(\"Rate\")",
                    "SUM(E3:E5)+rate+Costs[Cost]+N(\"Rate\")",
                ],
            ),
        )]
    } else {
        Vec::new()
    };
    let original = with_cut_tables(&tables);
    let mut parts = table_member_map(&original);
    let mut names = String::from("<definedNames>");
    if source_local {
        names.push_str("<definedName name=\"Rate\" localSheetId=\"0\">11</definedName>");
    }
    if destination_local {
        names.push_str("<definedName name=\"Rate\" localSheetId=\"1\">22</definedName>");
    }
    if global {
        names.push_str("<definedName name=\"Rate\">33</definedName>");
    }
    names.push_str("</definedNames>");
    let xml = std::str::from_utf8(&parts["xl/workbook.xml"]).unwrap();
    assert_eq!(xml.matches("</workbook>").count(), 1);
    assert!(!xml.contains("<definedNames"));
    let xml = xml.replace("</workbook>", &format!("{names}</workbook>"));
    parts.insert("xl/workbook.xml".into(), xml.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    workbook
        .sheet_mut("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("C3"), 0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(yggdryl::excel::Formula::from_file(
                    "rate+SUM(Rate)+N(\"Rate\")",
                    at("C3"),
                )),
        )
        .unwrap();
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    workbook
}

#[test]
fn scoped_name_cut_preserves_source_local_cell_binding_and_saved_inverse() {
    use yggdryl::excel::Edit;

    for (destination_local, global, source) in [
        (false, false, "Data"),
        (false, true, "Data"),
        (true, false, "Data"),
        (true, true, "Source Data"),
    ] {
        let mut workbook = scoped_name_cut_book(true, destination_local, global, false);
        if source != "Data" {
            workbook.rename_sheet("Data", source).unwrap();
            let saved = workbook.into_package().unwrap();
            workbook.rebase(saved).unwrap();
        }
        let names: Vec<_> = workbook.defined_names().cloned().collect();
        let before = table_member_map(&workbook);
        let moved = workbook
            .apply(Edit::Paste {
                from: (source.into(), "C3".parse().unwrap()),
                to: ("Other".into(), at("H8")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let qualifier = if source == "Data" {
            "Data"
        } else {
            "'Source Data'"
        };
        assert_eq!(
            workbook.entry_text("Other", at("H8")).unwrap(),
            Some(format!(
                "={qualifier}!rate+SUM({qualifier}!Rate)+N(\"Rate\")"
            )),
            "source={source}, destination_local={destination_local}, global={global}"
        );
        assert_eq!(workbook.defined_names().cloned().collect::<Vec<_>>(), names);
        let after = table_member_map(&workbook);
        let saved = workbook.into_package().unwrap();
        workbook.rebase(saved).unwrap();
        let undone = workbook.apply(moved.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), before);
        let saved = workbook.into_package().unwrap();
        workbook.rebase(saved).unwrap();
        workbook.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), after);
    }
}

#[test]
fn scoped_name_cut_preserves_source_local_table_formulas_and_saved_inverse() {
    use yggdryl::excel::Edit;

    let mut workbook = scoped_name_cut_book(true, true, true, true);
    let before = table_member_map(&workbook);
    let moved = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "E2:F6".parse().unwrap()),
            to: ("Other".into(), at("J8")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        cut_table_part(
            "Costs",
            11,
            ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
            [
                "J9+Data!Rate+Costs[[#This Row],[Cost]]+N(\"Rate\")",
                "SUM(J9:J11)+Data!rate+Costs[Cost]+N(\"Rate\")",
            ],
        )
    );
    let after = table_member_map(&workbook);
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    let undone = workbook.apply(moved.inverse.unwrap()).unwrap();
    assert_eq!(table_member_map(&workbook), before);
    let saved = workbook.into_package().unwrap();
    workbook.rebase(saved).unwrap();
    workbook.apply(undone.inverse.unwrap()).unwrap();
    assert_eq!(table_member_map(&workbook), after);
}

#[test]
fn scoped_name_cut_keeps_unshadowed_global_cell_and_table_names_unqualified() {
    for table in [false, true] {
        let mut workbook = scoped_name_cut_book(false, false, true, table);
        let (range, target) = if table { ("E2:F6", "J8") } else { ("C3", "H8") };
        workbook
            .paste(
                ("Data", range.parse().unwrap()),
                ("Other", at(target)),
                Paste::All,
                true,
            )
            .unwrap();
        if table {
            assert_eq!(
                member(&workbook, "xl/tables/table1.xml"),
                cut_table_part(
                    "Costs",
                    11,
                    ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
                    [
                        "J9+Rate+Costs[[#This Row],[Cost]]+N(\"Rate\")",
                        "SUM(J9:J11)+rate+Costs[Cost]+N(\"Rate\")",
                    ],
                )
            );
        } else {
            assert_eq!(
                workbook.entry_text("Other", at("H8")).unwrap().as_deref(),
                Some("=rate+SUM(Rate)+N(\"Rate\")")
            );
        }
    }
}

#[test]
fn scoped_name_cut_refuses_a_global_name_shadowed_at_the_destination_atomically() {
    for (table, global) in [(false, true), (true, true), (false, false), (true, false)] {
        // A formerly undefined bare name must not silently become the
        // destination local name either.
        let mut workbook = scoped_name_cut_book(false, true, global, table);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let (range, target) = if table { ("E2:F6", "J8") } else { ("C3", "H8") };
        match workbook
            .paste(
                ("Data", range.parse().unwrap()),
                ("Other", at(target)),
                Paste::All,
                true,
            )
            .unwrap_err()
        {
            Error::Unsupported {
                operation,
                filesystem,
            } => {
                assert!(
                    operation.contains("name") && operation.contains("shadow"),
                    "{operation}"
                );
                assert!(
                    filesystem.to_ascii_lowercase().contains("rate")
                        && filesystem.contains("Other"),
                    "{filesystem}"
                );
                let location = if table {
                    "xl/tables/table1.xml"
                } else {
                    "Data!C3"
                };
                assert!(filesystem.contains(location), "{filesystem}");
            }
            error => panic!("expected a located name-scope refusal, got {error}"),
        }
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert!(!workbook.is_dirty());
    }
}

#[test]
fn scoped_name_cut_qualified_names_already_follow_sheet_rename_in_cells_and_tables() {
    let mut workbook = scoped_name_cut_book(true, true, true, true);
    let mut parts = table_member_map(&workbook);
    parts.insert(
        "xl/tables/table1.xml".into(),
        cut_table_part(
            "Costs",
            11,
            ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
            ["Data!Rate+E3", "SUM(Data!Rate)+N(\"Data!Rate\")"],
        )
        .into_bytes(),
    );
    workbook = cross_table_reopen(&parts);
    workbook
        .sheet_mut("Other")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("H8"), 0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(yggdryl::excel::Formula::from_file(
                    "Data!Rate+N(\"Data!Rate\")",
                    at("H8"),
                )),
        )
        .unwrap();
    workbook.rename_sheet("Data", "Source Data").unwrap();
    assert_eq!(
        workbook.entry_text("Other", at("H8")).unwrap().as_deref(),
        Some("='Source Data'!Rate+N(\"Data!Rate\")")
    );
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        cut_table_part(
            "Costs",
            11,
            ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
            [
                "'Source Data'!Rate+E3",
                "SUM('Source Data'!Rate)+N(\"Data!Rate\")"
            ],
        )
    );
}

#[test]
fn scoped_name_cut_refuses_lexical_binding_collisions_before_rewriting_free_names() {
    for (function, formula) in [
        ("LET", "_xlfn.LET(Rate,1,Rate)+rate"),
        ("LAMBDA", "_xlfn.LAMBDA(Rate,Rate)(1)+rate"),
    ] {
        for table in [false, true] {
            let mut workbook = scoped_name_cut_book(true, true, true, table);
            if table {
                let mut parts = table_member_map(&workbook);
                parts.insert(
                    "xl/tables/table1.xml".into(),
                    cut_table_part(
                        "Costs",
                        11,
                        ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
                        [formula, "SUM(E3:E5)"],
                    )
                    .into_bytes(),
                );
                workbook = cross_table_reopen(&parts);
            } else {
                workbook
                    .sheet_mut("Data")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at("C3"), 0.into(), DateSystem::Year1900)
                            .unwrap()
                            .with_formula(yggdryl::excel::Formula::from_file(formula, at("C3"))),
                    )
                    .unwrap();
            }
            let saved = workbook.into_package().unwrap();
            workbook.rebase(saved).unwrap();
            workbook.parse_all().unwrap();
            let before = table_member_map(&workbook);
            let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
            let (range, target) = if table { ("E2:F6", "J8") } else { ("C3", "H8") };
            match workbook
                .paste(
                    ("Data", range.parse().unwrap()),
                    ("Other", at(target)),
                    Paste::All,
                    true,
                )
                .unwrap_err()
            {
                Error::Unsupported {
                    operation,
                    filesystem,
                } => {
                    assert!(
                        operation.contains("name") && operation.contains("lexical"),
                        "{operation}"
                    );
                    assert!(
                        filesystem.contains(function)
                            && filesystem.to_ascii_lowercase().contains("rate"),
                        "{filesystem}"
                    );
                    let location = if table {
                        "xl/tables/table1.xml"
                    } else {
                        "Data!C3"
                    };
                    assert!(filesystem.contains(location), "{filesystem}");
                }
                error => panic!("expected a located lexical-name refusal, got {error}"),
            }
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert!(!workbook.is_dirty());
        }
    }
}

fn carried_cross_cut_ready(mut workbook: Workbook) -> Workbook {
    // Normalize dimensions, calculation settings and default styles before
    // byte-exact saved-inverse checks; the test sheets still contain no cells.
    for name in ["Data", "Other"] {
        let sheet = workbook.sheet_mut(name).unwrap();
        sheet.set_cell(at("A1"), 1.0).unwrap();
        sheet.remove_cell(at("A1"));
    }
    Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap()
}

fn carried_cross_cut_fixture(fragment: &str, external_link: bool) -> Workbook {
    let base = sheets_book(
        &[
            ("Data", sheet("", fragment), &[]),
            ("Other", sheet("", ""), &[]),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    if external_link {
        let source = "xl/worksheets/sheet1.xml";
        let xml = String::from_utf8(parts[source].clone()).unwrap();
        assert_eq!(xml.matches("<hyperlink ref=\"D4\"").count(), 1);
        parts.insert(
            source.into(),
            xml.replace(
                "<hyperlink ref=\"D4\"",
                "<hyperlink ref=\"D4\" r:id=\"rIdLink\"",
            )
            .into_bytes(),
        );
        parts.insert(
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            format!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdLink\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/cut\" TargetMode=\"External\"/></Relationships>"
            ).into_bytes(),
        );
    }
    carried_cross_cut_ready(cross_table_reopen(&parts))
}

fn carried_cross_cut(
    mut workbook: Workbook,
    source_range: &str,
    destination_fragment: &str,
    source_marker: &str,
) -> (
    Workbook,
    std::collections::BTreeMap<String, Vec<u8>>,
    yggdryl::excel::Edit,
) {
    use yggdryl::excel::Edit;

    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), source_range.parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        !source.contains(source_marker),
        "moved source registration remains: {source}"
    );
    assert!(
        target.contains(destination_fragment),
        "destination lost carried registration: {target}"
    );
    (workbook, before, applied.inverse.unwrap())
}

#[test]
fn cross_sheet_cut_moves_conditional_format_and_saved_inverse() {
    let fragment = "<conditionalFormatting sqref=\"B2:B3\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;0</formula></cfRule></conditionalFormatting>";
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "B2:B3",
        "<conditionalFormatting sqref=\"J10:J11\"",
        "<conditionalFormatting sqref=\"B2:B3\"",
    );
    let after = table_member_map(&workbook);
    assert!(
        std::str::from_utf8(&after["xl/worksheets/sheet2.xml"])
            .unwrap()
            .contains("<formula>J10&gt;0</formula>")
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_moves_x14_conditional_format_and_keeps_unselected_sibling() {
    let fragment = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting><x14:conditionalFormatting><x14:cfRule type="expression" priority="2"><xm:f>F6&gt;0</xm:f></x14:cfRule><xm:sqref>F6</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "B2:B3",
        "<xm:sqref>J10:J11</xm:sqref>",
        "<xm:sqref>B2:B3</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(source.contains("<xm:sqref>F6</xm:sqref>"), "{source}");
    assert!(!target.contains("<xm:sqref>F6</xm:sqref>"), "{target}");
    assert!(target.contains("<xm:f>J10&gt;0</xm:f>"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_moves_x14_validation_and_saved_inverse() {
    let fragment = r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidations count="1"><x14:dataValidation type="whole"><x14:formula1><xm:f>C2</xm:f></x14:formula1><xm:sqref>C2:C3</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "C2:C3",
        "<xm:sqref>J10:J11</xm:sqref>",
        "<xm:sqref>C2:C3</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(target.contains("<xm:f>J10</xm:f>"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_moves_x14_sparkline_without_changing_its_data_source() {
    let fragment = r#"<extLst><ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroups><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>Data!B2:C2</xm:f><xm:sqref>D2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "D2",
        "<xm:sqref>J10</xm:sqref>",
        "<xm:sqref>D2</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(target.contains("<xm:f>Data!B2:C2</xm:f>"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_moves_x14_owner_even_when_coordinates_are_unchanged() {
    use yggdryl::excel::{Edit, Paste};
    let fragment = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("B2")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(!source.contains("<xm:sqref>B2:B3</xm:sqref>"), "{source}");
    assert!(target.contains("<xm:sqref>B2:B3</xm:sqref>"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_merges_x14_validation_into_existing_uri() {
    let source = r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidations count="1"><x14:dataValidation type="whole"><x14:formula1><xm:f>C2</xm:f></x14:formula1><xm:sqref>C2:C3</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#;
    let destination = r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidations count="1"><x14:dataValidation type="whole"><x14:formula1><xm:f>A1</xm:f></x14:formula1><xm:sqref>A1</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(source, false);
    let mut parts = table_member_map(&workbook);
    let target = String::from_utf8(parts["xl/worksheets/sheet2.xml"].clone()).unwrap();
    assert_eq!(target.matches("</worksheet>").count(), 1);
    parts.insert(
        "xl/worksheets/sheet2.xml".into(),
        target
            .replace("</worksheet>", &format!("{destination}</worksheet>"))
            .into_bytes(),
    );
    let workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "C2:C3",
        "<xm:sqref>J10:J11</xm:sqref>",
        "<xm:sqref>C2:C3</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert_eq!(
        target
            .matches("{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}")
            .count(),
        1,
        "{target}"
    );
    assert!(target.contains("<xm:sqref>A1</xm:sqref>"), "{target}");
    assert!(target.contains("count=\"2\""), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_allocates_x14_conditional_format_priority_at_destination() {
    let source = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let destination = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>A1&lt;0</xm:f></x14:cfRule><xm:sqref>A1</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(source, false);
    let mut parts = table_member_map(&workbook);
    let target = String::from_utf8(parts["xl/worksheets/sheet2.xml"].clone()).unwrap();
    assert_eq!(target.matches("</worksheet>").count(), 1);
    parts.insert(
        "xl/worksheets/sheet2.xml".into(),
        target
            .replace("</worksheet>", &format!("{destination}</worksheet>"))
            .into_bytes(),
    );
    let workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "B2:B3",
        "<xm:sqref>J10:J11</xm:sqref>",
        "<xm:sqref>B2:B3</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert_eq!(target.matches("priority=\"1\"").count(), 1, "{target}");
    assert_eq!(target.matches("priority=\"2\"").count(), 1, "{target}");
    assert!(target.contains("priority=\"1\"><xm:f>J10"), "{target}");
    assert!(target.contains("priority=\"2\"><xm:f>A1"), "{target}");
    assert!(target.contains("<xm:f>J10&gt;0</xm:f>"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_refuses_foreign_x14_range_lookalikes_atomically() {
    use yggdryl::excel::Edit;
    let fragments = [
        r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" xmlns:v="urn:vendor"><x14:conditionalFormattings><v:conditionalFormatting><xm:sqref>B2:B3</xm:sqref></v:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#,
        r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" xmlns:v="urn:vendor"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><v:sqref>B2:B3</v:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#,
    ];
    for (case, fragment) in fragments.into_iter().enumerate() {
        let mut workbook = carried_cross_cut_fixture(fragment, false);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let result = workbook.apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        });
        if case == 0 {
            assert!(
                matches!(result, Err(Error::Unsupported { .. })),
                "{result:?}"
            );
        } else {
            // Retained planning resolves the genuine host before transfer:
            // a foreign lookalike cannot supply its mandatory xm:sqref.
            assert!(
                matches!(result, Err(Error::InvalidRecord { ref path, ref reason })
                if path == "xl/worksheets/sheet1.xml#sqref" && reason == "expected one sqref, got 0"),
                "{result:?}"
            );
        }
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn cross_sheet_cut_refuses_unknown_x14_extension_uri_atomically() {
    use yggdryl::excel::Edit;
    let fragment = r#"<extLst><ext uri="urn:vendor:range" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><xm:sqref>B2:B3</xm:sqref></ext></extLst>"#;
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    let dirty = workbook.is_dirty();
    let result = workbook.apply(Edit::Paste {
        from: ("Data".into(), "B2:B3".parse().unwrap()),
        to: ("Other".into(), at("J10")),
        what: Paste::All,
        cut: true,
    });
    assert!(
        matches!(result, Err(Error::Unsupported { .. })),
        "{result:?}"
    );
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert_eq!(workbook.is_dirty(), dirty);
}

#[test]
fn cross_sheet_cut_merges_x14_two_hosts_and_preserves_source_mce() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
    let source = r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" xmlns:v="urn:source"><x14:dataValidations count="2"><x14:dataValidation v:flag="source" type="whole"><x14:formula1><xm:f>B2</xm:f></x14:formula1><xm:sqref>B2:B3</xm:sqref></x14:dataValidation><x14:dataValidation type="whole"><x14:formula1><xm:f>F6</xm:f></x14:formula1><xm:sqref>F6</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#;
    let destination = r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidations count="1"><x14:dataValidation type="whole"><x14:formula1><xm:f>A1</xm:f></x14:formula1><xm:sqref>A1</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#;
    let base = sheets_book(
        &[
            ("Data", sheet("", source), &[]),
            ("Other", sheet("", destination), &[]),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    let original = std::str::from_utf8(&parts["xl/worksheets/sheet1.xml"]).unwrap();
    parts.insert("xl/worksheets/sheet1.xml".into(), original.replace(
        "<worksheet xmlns=", "<worksheet xmlns:v=\"urn:source\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"v\" xmlns=",
    ).into_bytes());
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(source.contains("<xm:sqref>F6</xm:sqref>"), "{source}");
    assert!(!source.contains("<xm:sqref>B2:B3</xm:sqref>"), "{source}");
    assert_eq!(
        target
            .matches("{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}")
            .count(),
        1,
        "{target}"
    );
    let doc = from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
    let root = Element::root(&doc).unwrap();
    let extensions = root.child(Some(NS), "extLst").unwrap();
    let extension = extensions.child(Some(NS), "ext").unwrap();
    let list = extension.child(Some(X14), "dataValidations").unwrap();
    assert_eq!(list.attribute_in(None, "count"), Some("2"));
    let mut seen = Vec::new();
    for host in list.children_in(Some(X14), "dataValidation") {
        let sqref = host.child(Some(XM), "sqref").unwrap();
        let reference = sqref.text().unwrap();
        if reference == "J10:J11" {
            assert_eq!(
                host.attribute_in(Some("urn:source"), "flag"),
                Some("source")
            );
        }
        seen.push(reference.to_owned());
    }
    assert!(
        seen.contains(&"A1".to_owned()) && seen.contains(&"J10:J11".to_owned()),
        "{seen:?}"
    );
    assert!(target.contains("Ignorable"), "{target}");
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_moves_linked_legacy_and_x14_databar_guid_together() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const GUID: &str = "{00000000-000E-0000-0000-000001000000}";
    let source = format!(
        r#"<conditionalFormatting sqref="B2:B3"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{GUID}</x14:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{GUID}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/><x14:negativeFillColor rgb="FFFF0000"/><x14:axisColor rgb="FF000000"/></x14:dataBar></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
    );
    let mut workbook = carried_cross_cut_fixture(&source, false);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(!source.contains(GUID), "{source}");
    assert!(target.contains("sqref=\"J10:J11\""), "{target}");
    assert!(target.contains("<xm:sqref>J10:J11</xm:sqref>"), "{target}");
    assert!(
        target.contains("type=\"dataBar\" priority=\"1\""),
        "{target}"
    );
    let document = from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
    let root = Element::root(&document).unwrap();
    let legacy = root
        .children_in(Some(NS), "conditionalFormatting")
        .into_iter()
        .find(|entry| entry.attribute_in(None, "sqref") == Some("J10:J11"))
        .unwrap();
    let legacy_rule = legacy.child(Some(NS), "cfRule").unwrap();
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    let legacy_extensions = legacy_rule.child(Some(NS), "extLst").unwrap();
    let legacy_extension = legacy_extensions.child(Some(NS), "ext").unwrap();
    let linked_id = legacy_extension.child(Some(X14), "id").unwrap();
    let link = linked_id.text().unwrap();
    let x14_extensions = root.child(Some(NS), "extLst").unwrap();
    let x14_extension = x14_extensions.child(Some(NS), "ext").unwrap();
    let x14_hosts = x14_extension
        .child(Some(X14), "conditionalFormattings")
        .unwrap();
    let x14_host = x14_hosts.child(Some(X14), "conditionalFormatting").unwrap();
    let x14_rule = x14_host.child(Some(X14), "cfRule").unwrap();
    assert!(!link.is_empty());
    assert_eq!(x14_rule.attribute_in(None, "id"), Some(link));
    assert_eq!(target.matches(link).count(), 2, "{target}");
    assert_eq!(x14_rule.attribute_in(None, "priority"), None);
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_refuses_destination_x14_guid_collision_atomically() {
    use yggdryl::excel::Edit;
    let guid = "{00000000-000E-0000-0000-000001000000}";
    let source = format!(
        r#"<conditionalFormatting sqref="B2:B3"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{guid}</x14:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{guid}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
    );
    let destination = format!(
        r#"<extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{guid}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>A1</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
    );
    let base = sheets_book(
        &[
            ("Data", sheet("", &source), &[]),
            ("Other", sheet("", &destination), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    let dirty = workbook.is_dirty();
    let error = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .expect_err("two x14 rules cannot share a destination GUID");
    let diagnostic = error.to_string();
    assert!(
        diagnostic.contains("sheet2.xml") && diagnostic.to_ascii_lowercase().contains("guid"),
        "{diagnostic}"
    );
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert_eq!(workbook.is_dirty(), dirty);
}

#[test]
fn cross_sheet_cut_linked_guid_uri_is_decoded_but_exactly_unqualified() {
    use yggdryl::excel::Edit;
    let guid = "{00000000-000E-0000-0000-000001000000}";
    for (attribute, allowed) in [
        (r#"uri="{&#x42;025F937-C7B1-47D3-B67F-A62EFF666E3E}""#, true),
        (
            r#"v:uri="{B025F937-C7B1-47D3-B67F-A62EFF666E3E}" xmlns:v="urn:vendor""#,
            false,
        ),
    ] {
        let source = format!(
            r#"<conditionalFormatting sqref="B2:B3"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext {attribute} xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{guid}</x14:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{guid}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
        );
        let mut workbook = carried_cross_cut_fixture(&source, false);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let result = workbook.apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        });
        if allowed {
            let applied = result.expect("encoded unqualified URI must bind the linked rule");
            let package = table_member_map(&workbook);
            let target = std::str::from_utf8(&package["xl/worksheets/sheet2.xml"]).unwrap();
            assert!(target.contains("<xm:sqref>J10:J11</xm:sqref>"), "{target}");
            note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
        } else {
            let error = result.expect_err("vendor:uri cannot supply the unqualified URI");
            let diagnostic = error.to_string();
            assert!(
                diagnostic.contains("sheet1.xml") && diagnostic.contains("foreign"),
                "{diagnostic}"
            );
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert_eq!(workbook.is_dirty(), dirty);
        }
    }
}

#[test]
fn cross_sheet_cut_refuses_unproved_linked_databar_guid_atomically() {
    use yggdryl::excel::Edit;
    let guid = "{00000000-000E-0000-0000-000001000000}";
    let other = "{00000000-000E-0000-0000-000002000000}";
    for (x14_guid, x14_range) in [(other, "B2:B3"), (guid, "F6")] {
        let fragment = format!(
            r#"<conditionalFormatting sqref="B2:B3"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{guid}</x14:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{x14_guid}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>{x14_range}</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
        );
        let mut workbook = carried_cross_cut_fixture(&fragment, false);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let result = workbook.apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        });
        let error = result.expect_err("an unpaired linked dataBar must refuse");
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("sheet1.xml") && diagnostic.to_ascii_lowercase().contains("guid"),
            "{diagnostic}"
        );
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn cross_sheet_cut_moves_data_validation_and_saved_inverse() {
    let fragment = "<dataValidations count=\"1\"><dataValidation type=\"whole\" sqref=\"C2:C3\"><formula1>C2</formula1></dataValidation></dataValidations>";
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "C2:C3",
        "<dataValidation type=\"whole\" sqref=\"J10:J11\"",
        "<dataValidation type=\"whole\" sqref=\"C2:C3\"",
    );
    let after = table_member_map(&workbook);
    assert!(
        std::str::from_utf8(&after["xl/worksheets/sheet2.xml"])
            .unwrap()
            .contains("<formula1>J10</formula1>")
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_cut_moves_hyperlink_relationship_and_saved_inverse() {
    let fragment =
        "<hyperlinks><hyperlink ref=\"D4\" location=\"Data!B2\" tooltip=\"keep\"/></hyperlinks>";
    let workbook = carried_cross_cut_fixture(fragment, true);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "D4",
        "<hyperlink ref=\"J10\"",
        "<hyperlink ref=\"D4\"",
    );
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(target.contains("location=\"Data!B2\""));
    let rels = std::str::from_utf8(&after["xl/worksheets/_rels/sheet2.xml.rels"]).unwrap();
    assert!(rels.contains("Target=\"https://example.invalid/cut\""));
    assert!(rels.contains("TargetMode=\"External\""));
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn scoped_name_cut_refuses_callable_local_names_before_their_binding_changes() {
    for (source_local, table) in [(true, false), (true, true), (false, false), (false, true)] {
        let mut workbook = scoped_name_cut_book(source_local, true, false, table);
        let mut parts = table_member_map(&workbook);
        let mut xml = std::str::from_utf8(&parts["xl/workbook.xml"])
            .unwrap()
            .to_owned();
        for (scope, value) in [(0, 11), (1, 22)] {
            let before = format!(
                "<definedName name=\"Rate\" localSheetId=\"{scope}\">{value}</definedName>"
            );
            let after = format!(
                "<definedName name=\"LocalAdder\" localSheetId=\"{scope}\">_xlfn.LAMBDA(x,x+{value})</definedName>"
            );
            assert_eq!(
                xml.matches(&before).count(),
                usize::from(scope == 1 || source_local)
            );
            xml = xml.replace(&before, &after);
        }
        parts.insert("xl/workbook.xml".into(), xml.into_bytes());
        if table {
            parts.insert(
                "xl/tables/table1.xml".into(),
                cut_table_part(
                    "Costs",
                    11,
                    ["E2:F6", "E2:F5", "E3:F5", "F3:F5"],
                    ["LocalAdder(1)", "SUM(lOcAlAdDeR(2))"],
                )
                .into_bytes(),
            );
        }
        workbook = cross_table_reopen(&parts);
        if !table {
            workbook
                .sheet_mut("Data")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(at("C3"), 0.into(), DateSystem::Year1900)
                        .unwrap()
                        .with_formula(yggdryl::excel::Formula::from_file(
                            "LocalAdder(1)+lOcAlAdDeR(2)",
                            at("C3"),
                        )),
                )
                .unwrap();
        }
        let saved = workbook.into_package().unwrap();
        workbook.rebase(saved).unwrap();
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let (range, target) = if table { ("E2:F6", "J8") } else { ("C3", "H8") };
        match workbook
            .paste(
                ("Data", range.parse().unwrap()),
                ("Other", at(target)),
                Paste::All,
                true,
            )
            .unwrap_err()
        {
            Error::Unsupported {
                operation,
                filesystem,
            } => {
                let diagnostic = format!("{operation} {filesystem}");
                assert!(
                    diagnostic.contains("callable")
                        && diagnostic.to_ascii_lowercase().contains("localadder"),
                    "{diagnostic}"
                );
                let location = if table {
                    "xl/tables/table1.xml"
                } else {
                    "Data!C3"
                };
                assert!(filesystem.contains(location), "{filesystem}");
            }
            error => panic!("expected a located callable-name refusal, got {error}"),
        }
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert!(!workbook.is_dirty());
    }
}

#[test]
fn cross_sheet_cut_partitions_partial_source_conditional_format_and_saved_inverse() {
    use yggdryl::excel::Edit;
    let fragment = "<conditionalFormatting sqref=\"B2:B4\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;0</formula></cfRule></conditionalFormatting>";
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let destination = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        source.contains("<conditionalFormatting sqref=\"B4\">"),
        "{source}"
    );
    assert!(source.contains("<formula>B4&gt;0</formula>"), "{source}");
    assert!(
        destination.contains("<conditionalFormatting sqref=\"J10:J11\">"),
        "{destination}"
    );
    assert!(
        destination.contains("<formula>J10&gt;0</formula>"),
        "{destination}"
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_partitions_partial_destination_validation_and_saved_inverse() {
    use yggdryl::excel::Edit;
    let base = sheets_book(
        &[
            ("Data", sheet("", ""), &[]),
            (
                "Other",
                sheet(
                    "",
                    "<dataValidations count=\"1\"><dataValidation sqref=\"J10:J12\" type=\"whole\"><formula1>1</formula1></dataValidation></dataValidations>",
                ),
                &[],
            ),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let destination = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        destination.contains("<dataValidations count=\"1\">"),
        "{destination}"
    );
    assert!(destination.contains("sqref=\"J12\""), "{destination}");
    assert!(
        destination.contains("<formula1>1</formula1>"),
        "{destination}"
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_allocates_noncolliding_conditional_format_priorities() {
    use yggdryl::excel::Edit;
    let source_cf = "<conditionalFormatting sqref=\"B2:B3\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;0</formula></cfRule></conditionalFormatting>";
    let target_cf = "<conditionalFormatting sqref=\"A1\"><cfRule type=\"expression\" priority=\"1\"><formula>A1&lt;0</formula></cfRule></conditionalFormatting>";
    let base = sheets_book(
        &[
            ("Data", sheet("", source_cf), &[]),
            ("Other", sheet("", target_cf), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        target.contains("sqref=\"A1\"><cfRule type=\"expression\" priority=\"2\""),
        "{target}"
    );
    assert!(
        target.contains("sqref=\"J10:J11\"><cfRule type=\"expression\" priority=\"1\""),
        "{target}"
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_keeps_moved_cf_order_and_destination_priority_gaps() {
    use yggdryl::excel::Edit;
    let source_cf = concat!(
        r#"<conditionalFormatting sqref="B2"><cfRule type="expression" priority="8"><formula>B2&gt;8</formula></cfRule></conditionalFormatting>"#,
        r#"<conditionalFormatting sqref="B3"><cfRule type="expression" priority="2"><formula>B3&gt;2</formula></cfRule></conditionalFormatting>"#,
    );
    let target_cf = concat!(
        r#"<conditionalFormatting sqref="A1"><cfRule type="expression" priority="7"><formula>A1&lt;0</formula></cfRule></conditionalFormatting>"#,
        r#"<conditionalFormatting sqref="F6"><cfRule type="expression" priority="3"><formula>F6&lt;0</formula></cfRule></conditionalFormatting>"#,
    );
    let base = sheets_book(
        &[
            ("Data", sheet("", source_cf), &[]),
            ("Other", sheet("", target_cf), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let package = table_member_map(&workbook);
    let target = std::str::from_utf8(&package["xl/worksheets/sheet2.xml"]).unwrap();
    for (range, priority) in [("J10", 2), ("J11", 1), ("A1", 9), ("F6", 5)] {
        assert!(
            target.contains(&format!(
                "sqref=\"{range}\"><cfRule type=\"expression\" priority=\"{priority}\""
            )),
            "{target}"
        );
    }
}

#[test]
fn cross_sheet_cut_keeps_global_conditional_format_precedence() {
    use yggdryl::excel::Edit;
    let source_cf = concat!(
        "<conditionalFormatting sqref=\"B2\"><cfRule type=\"expression\" priority=\"8\"><formula>B2&gt;8</formula></cfRule></conditionalFormatting>",
        "<conditionalFormatting sqref=\"B3\"><cfRule type=\"expression\" priority=\"2\"><formula>B3&gt;2</formula></cfRule></conditionalFormatting>",
    );
    let target_cf = "<conditionalFormatting sqref=\"A1\"><cfRule type=\"expression\" priority=\"1\"><formula>A1&lt;0</formula></cfRule></conditionalFormatting>";
    let base = sheets_book(
        &[
            ("Data", sheet("", source_cf), &[]),
            ("Other", sheet("", target_cf), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        target.contains("sqref=\"J10\"><cfRule type=\"expression\" priority=\"2\""),
        "{target}"
    );
    assert!(
        target.contains("sqref=\"J11\"><cfRule type=\"expression\" priority=\"1\""),
        "{target}"
    );
    assert!(
        target.contains("sqref=\"A1\"><cfRule type=\"expression\" priority=\"3\""),
        "{target}"
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cut_refuses_foreign_formula_lookalike() {
    use yggdryl::excel::Edit;
    let fragment = "<conditionalFormatting xmlns:v=\"urn:vendor\" sqref=\"B2:B3\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;0</formula><v:formula>B2 vendor literal</v:formula></cfRule></conditionalFormatting>";
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    let before = table_member_map(&workbook);
    let result = workbook.apply(Edit::Paste {
        from: ("Data".into(), "B2:B3".parse().unwrap()),
        to: ("Other".into(), at("J10")),
        what: Paste::All,
        cut: true,
    });
    assert!(
        matches!(result, Err(Error::Unsupported { .. })),
        "foreign formula lookalike must be refused"
    );
    assert_eq!(table_member_map(&workbook), before);
}

#[test]
fn hyperlink_only_cross_sheet_cut_ignores_unrelated_bad_cf_priority() {
    use yggdryl::excel::Edit;
    let source = "<hyperlinks><hyperlink ref=\"D4\" location=\"Data!B2\"/></hyperlinks>";
    let destination = "<conditionalFormatting sqref=\"A1\"><cfRule type=\"expression\" priority=\"vendor\"><formula>A1&gt;0</formula></cfRule></conditionalFormatting>";
    let base = sheets_book(
        &[
            ("Data", sheet("", source), &[]),
            ("Other", sheet("", destination), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = cross_table_reopen(&table_member_map(&base));
    workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "D4".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let target = member(&workbook, "xl/worksheets/sheet2.xml");
    assert!(target.contains("<hyperlink ref=\"J10\""));
    assert!(target.contains("priority=\"vendor\""));
}

#[test]
fn cross_sheet_hyperlink_transfer_resolves_destination_id_collision() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

    let source = "<hyperlinks><hyperlink ref=\"D4\" r:id=\"rIdLink\"/></hyperlinks>";
    let destination = "<hyperlinks><hyperlink ref=\"A1\" r:id=\"rIdLink\"/></hyperlinks>";
    let base = sheets_book(
        &[
            ("Data", sheet("", source), &[]),
            ("Other", sheet("", destination), &[]),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    parts.insert("xl/worksheets/_rels/sheet1.xml.rels".into(), format!(
        "<Relationships xmlns=\"{PKG_RELS}\"><Relationship Id=\"rIdLink\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/moved\" TargetMode=\"External\"/></Relationships>"
    ).into_bytes());
    parts.insert("xl/worksheets/_rels/sheet2.xml.rels".into(), format!(
        "<Relationships xmlns=\"{PKG_RELS}\"><Relationship Id=\"rIdLink\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/kept\" TargetMode=\"External\"/></Relationships>"
    ).into_bytes());
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "D4".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let target_doc = from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
    let target = Element::root(&target_doc).unwrap();
    let hyperlinks = target.child(Some(NS), "hyperlinks").unwrap();
    let links: std::collections::BTreeMap<_, _> = hyperlinks
        .children_in(Some(NS), "hyperlink")
        .into_iter()
        .map(|link| {
            (
                link.attribute_in(None, "ref").unwrap().to_owned(),
                link.attribute_in(Some(R_NS), "id").unwrap().to_owned(),
            )
        })
        .collect();
    let kept = links.get("A1").expect("the destination hyperlink remains");
    let moved = links.get("J10").expect("the moved hyperlink arrives");
    assert_ne!(
        kept, moved,
        "one destination relationship cannot name both targets"
    );

    let rels_doc = from_bytes(&after["xl/worksheets/_rels/sheet2.xml.rels"]).unwrap();
    let rels = Element::root(&rels_doc).unwrap();
    let targets: std::collections::BTreeMap<_, _> = rels
        .children_in(Some(PKG_RELS), "Relationship")
        .into_iter()
        .map(|relation| {
            (
                relation.attribute_in(None, "Id").unwrap().to_owned(),
                relation.attribute_in(None, "Target").unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        targets.get(kept).map(String::as_str),
        Some("https://example.invalid/kept")
    );
    assert_eq!(
        targets.get(moved).map(String::as_str),
        Some("https://example.invalid/moved")
    );

    let source_doc = from_bytes(&after["xl/worksheets/_rels/sheet1.xml.rels"]).unwrap();
    let source_rels = Element::root(&source_doc).unwrap();
    assert!(
        source_rels
            .children_in(Some(PKG_RELS), "Relationship")
            .is_empty(),
        "the moved source relationship is removed"
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn cross_sheet_cf_transfer_carries_inherited_namespace_and_mce() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let base = sheets_book(
        &[
            (
                "Data",
                sheet(
                    "",
                    "<conditionalFormatting sqref=\"B2\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;0</formula></cfRule></conditionalFormatting>",
                ),
                &[],
            ),
            ("Other", sheet("", ""), &[]),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    let source = String::from_utf8(parts["xl/worksheets/sheet1.xml"].clone()).unwrap();
    parts.insert("xl/worksheets/sheet1.xml".into(), source.replace(
        "<worksheet xmlns=", "<worksheet xmlns:v=\"urn:vendor\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"v\" xmlns=",
    ).replace("<cfRule type=", "<cfRule v:flag=\"kept\" type=").into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let doc = from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
    let root = Element::root(&doc).unwrap();
    let format = root
        .children_in(Some(NS), "conditionalFormatting")
        .into_iter()
        .find(|format| format.attribute_in(None, "sqref") == Some("J10"))
        .expect("the carried format has its moved range");
    let rule = format.child(Some(NS), "cfRule").unwrap();
    assert_eq!(rule.attribute_in(Some("urn:vendor"), "flag"), Some("kept"));
    assert_eq!(format.attribute_in(Some(MCE), "Ignorable"), Some("v"));
}

fn insertion_style_book(
    source_border: &str,
    following_border: &str,
    cells: &str,
    count: usize,
) -> Workbook {
    let mut xfs = String::new();
    for index in 0..count {
        let (font, border) = match index {
            1 => (1, 1),
            2 => (0, 2),
            _ => (0, 0),
        };
        xfs.push_str(&format!(
            "<xf numFmtId=\"0\" fontId=\"{font}\" fillId=\"0\" borderId=\"{border}\" xfId=\"0\"/>"
        ));
    }
    let style_xml = format!(
        "<styleSheet xmlns=\"{NS}\"><fonts count=\"2\"><font><name val=\"Calibri\"/><sz val=\"11\"/></font><font><name val=\"Calibri\"/><sz val=\"11\"/><b/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"3\"><border/>{source_border}{following_border}</borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"{count}\">{xfs}</cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>"
    );
    Workbook::from_bytes(package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Data"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/styles.xml", &style_xml),
        ("xl/worksheets/sheet1.xml", &worksheet(cells)),
    ]))
    .unwrap()
}

#[test]
fn inserted_style_bands_intersect_each_exterior_edge_and_the_whole_diagonal() {
    use yggdryl::excel::Edge;
    let blue = "<color rgb=\"FF204080\"/>";
    let red = "<color rgb=\"FFC00000\"/>";
    // Excel16 build20430: left differs while top agrees; a diagonal with
    // equal line/color but both->up flags clears as one indivisible fact.
    for columns in [false, true] {
        for count in [1, 2] {
            for same_diagonal in [false, true] {
                let source = format!(
                    "<border diagonalUp=\"1\" diagonalDown=\"1\"><left style=\"thin\">{blue}</left><top style=\"thin\">{blue}</top><diagonal style=\"thin\">{blue}</diagonal></border>"
                );
                let flags = if same_diagonal {
                    "diagonalUp=\"1\" diagonalDown=\"1\""
                } else {
                    "diagonalUp=\"1\""
                };
                let following = format!(
                    "<border {flags}><left style=\"dashed\">{red}</left><top style=\"thin\">{blue}</top><diagonal style=\"thin\">{blue}</diagonal></border>"
                );
                let cells = if columns {
                    "<row r=\"2\"><c r=\"B2\" s=\"1\"><v>1</v></c><c r=\"C2\" s=\"2\"><v>2</v></c></row>"
                } else {
                    "<row r=\"2\"><c r=\"B2\" s=\"1\"><v>1</v></c></row><row r=\"3\"><c r=\"B3\" s=\"2\"><v>2</v></c></row>"
                };
                let mut book = insertion_style_book(&source, &following, cells, 3);
                let mut expected = book.cell_style("Data", at("B2")).unwrap();
                expected.border.left = Edge::default();
                if !same_diagonal {
                    expected.border.diagonal = Edge::default();
                    expected.border.diagonal_up = false;
                    expected.border.diagonal_down = false;
                }
                if columns {
                    book.insert_columns("Data", 2, count).unwrap();
                } else {
                    book.insert_rows("Data", 2, count).unwrap();
                }
                for index in 2..2 + count {
                    let cell = if columns {
                        CellRef::new(1, index)
                    } else {
                        CellRef::new(index, 1)
                    };
                    assert_eq!(book.sheet("Data").unwrap().scalar(cell), Scalar::Null);
                    assert_eq!(
                        book.cell_style("Data", cell).unwrap(),
                        expected,
                        "columns={columns}, count={count}, same_diagonal={same_diagonal}"
                    );
                }
                assert_eq!(book.sheet("Data").unwrap().cell_count(), 2 + count as usize);
            }
        }
    }
}

#[test]
fn inserted_style_bands_preserve_temporal_formats_and_leave_default_blanks_absent() {
    for columns in [false, true] {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Data").unwrap();
        sheet.set_cell(at("A1"), 1).unwrap();
        sheet.set_cell(at("B2"), Scalar::date32(19_723)).unwrap();
        let expected = book.cell_style("Data", at("B2")).unwrap();
        if columns {
            book.insert_columns("Data", 2, 2).unwrap();
        } else {
            book.insert_rows("Data", 2, 2).unwrap();
        }
        for index in 2..4 {
            let cell = if columns {
                CellRef::new(1, index)
            } else {
                CellRef::new(index, 1)
            };
            assert_eq!(book.cell_style("Data", cell).unwrap(), expected);
            assert_eq!(book.sheet("Data").unwrap().scalar(cell), Scalar::Null);
        }
        assert_eq!(book.sheet("Data").unwrap().cell_count(), 4);
        assert!(book.sheet("Data").unwrap().cell(at("H8")).is_none());
        if columns {
            book.insert_columns("Data", 0, 2).unwrap();
        } else {
            book.insert_rows("Data", 0, 2).unwrap();
        }
        assert_eq!(book.sheet("Data").unwrap().cell_count(), 4);
        assert!(book.sheet("Data").unwrap().cell(at("A1")).is_none());
    }
}

#[test]
fn inserted_style_bands_copy_parent_formats_without_materializing_empty_cells() {
    use yggdryl::excel::StylePatch;
    for columns in [false, true] {
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        let range = if columns { "B:B" } else { "2:2" };
        book.set_style(
            "Data",
            &[range.parse().unwrap()],
            &StylePatch {
                bold: Some(true),
                number_format: Some("0.00%".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
        let expected = book.cell_style("Data", at("B2")).unwrap();
        if columns {
            book.insert_columns("Data", 2, 2).unwrap();
        } else {
            book.insert_rows("Data", 2, 2).unwrap();
        }
        assert_eq!(book.sheet("Data").unwrap().cell_count(), 0);
        for index in 2..4 {
            let cell = if columns {
                CellRef::new(500_000, index)
            } else {
                CellRef::new(index, 10_000)
            };
            assert_eq!(book.cell_style("Data", cell).unwrap(), expected);
        }
        let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        assert_eq!(reopened.sheet("Data").unwrap().cell_count(), 0);
    }
}

#[test]
fn inserted_style_bands_refuse_style_capacity_without_publishing_any_mutation() {
    let border = "<border><left style=\"thin\"><color rgb=\"FF204080\"/></left></border>";
    let mut book = insertion_style_book(
        border,
        "<border/>",
        "<row r=\"2\"><c r=\"B2\" s=\"1\"><v>1</v></c></row>",
        yggdryl::excel::MAX_CELL_FORMATS,
    );
    let before = table_member_map(&book);
    let revision = book.sheet("Data").unwrap().revision();
    let dirty = book.is_dirty();
    let error = book.insert_rows("Data", 2, 1).unwrap_err();
    assert!(
        matches!(error, Error::InvalidRecord { ref path, .. } if path.as_str() == "$.styles"),
        "{error}"
    );
    assert_eq!(
        book.style_sheet().unwrap().len(),
        yggdryl::excel::MAX_CELL_FORMATS
    );
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
    assert_eq!(book.is_dirty(), dirty);
    assert_eq!(table_member_map(&book), before);
}

#[test]
fn inserted_style_bands_refuse_unobserved_inner_borders_atomically() {
    for border in [
        "<border><vertical style=\"thin\"/></border>",
        "<border><horizontal style=\"thin\"/></border>",
        "<border outline=\"0\"/>",
    ] {
        let mut book = insertion_style_book(
            border,
            "<border/>",
            "<row r=\"2\"><c r=\"B2\" s=\"1\"><v>1</v></c></row>",
            3,
        );
        let before = table_member_map(&book);
        let revision = book.sheet("Data").unwrap().revision();
        let error = book.insert_rows("Data", 2, 1).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Unsupported {
                    operation: "inserting through inner or non-outline cell borders",
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(book.sheet("Data").unwrap().revision(), revision);
        assert_eq!(table_member_map(&book), before);
    }
}

// Formula-free worksheet registrations still follow the cells a cut moves.
// Exercise complete-child partitions before introducing partial sqref splitting.
fn carried_scope_entries(
    parts: &std::collections::BTreeMap<String, Vec<u8>>,
    part: &str,
    container: &str,
    child: &str,
    attributes: &[&str],
) -> Vec<Vec<String>> {
    let document = yggdryl::xml::from_bytes(&parts[part]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    assert!(root.is(Some(NS), "worksheet"));
    let Some(held) = root.child(Some(NS), container) else {
        return Vec::new();
    };
    let mut values: Vec<_> = held
        .children()
        .map(|entry| {
            assert!(entry.is(Some(NS), child), "{part}: expanded child name");
            attributes
                .iter()
                .map(|name| entry.attribute_in(None, name).unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    values.sort();
    values
}

fn carried_scope_cut(
    source: &str,
    destination: &str,
) -> (
    Workbook,
    std::collections::BTreeMap<String, Vec<u8>>,
    yggdryl::excel::Edit,
) {
    use yggdryl::excel::Edit;
    let base = sheets_book(
        &[
            ("Data", sheet("", source), &[]),
            ("Other", sheet("", destination), &[]),
        ],
        &[],
        &[],
    );
    let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    (workbook, before, applied.inverse.unwrap())
}

#[test]
fn carried_scope_cut_moves_protected_ranges_and_saved_inverse() {
    let source = r#"<protectedRanges><protectedRange name="moved" sqref="B2:B3" password="ABCD"/><protectedRange name="source" sqref="F6" password="1234"/></protectedRanges>"#;
    let destination = r#"<protectedRanges><protectedRange name="covered" sqref="J10:J11" password="FEDC"/><protectedRange name="target" sqref="M20" password="5678"/></protectedRanges>"#;
    let (mut workbook, before, undo) = carried_scope_cut(source, destination);
    let after = table_member_map(&workbook);
    let entries = |part| {
        carried_scope_entries(
            &after,
            part,
            "protectedRanges",
            "protectedRange",
            &["name", "sqref", "password"],
        )
    };
    assert_eq!(
        entries("xl/worksheets/sheet1.xml"),
        vec![vec!["source", "F6", "1234"]]
    );
    assert_eq!(
        entries("xl/worksheets/sheet2.xml"),
        vec![
            vec!["moved", "J10:J11", "ABCD"],
            vec!["target", "M20", "5678"],
        ]
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn carried_scope_cut_moves_ignored_errors_and_saved_inverse() {
    let source = r#"<ignoredErrors><ignoredError sqref="B2:B3" numberStoredAsText="1" evalError="0"/><ignoredError sqref="F6" numberStoredAsText="0" evalError="1"/></ignoredErrors>"#;
    let destination = r#"<ignoredErrors><ignoredError sqref="J10:J11" numberStoredAsText="0" evalError="0"/><ignoredError sqref="M20" numberStoredAsText="1" evalError="1"/></ignoredErrors>"#;
    let (mut workbook, before, undo) = carried_scope_cut(source, destination);
    let after = table_member_map(&workbook);
    let entries = |part| {
        carried_scope_entries(
            &after,
            part,
            "ignoredErrors",
            "ignoredError",
            &["sqref", "numberStoredAsText", "evalError"],
        )
    };
    assert_eq!(
        entries("xl/worksheets/sheet1.xml"),
        vec![vec!["F6", "0", "1"]]
    );
    assert_eq!(
        entries("xl/worksheets/sheet2.xml"),
        vec![vec!["J10:J11", "1", "0"], vec!["M20", "1", "1"],]
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn carried_scope_cut_moves_cell_watches_and_saved_inverse() {
    let source =
        r#"<cellWatches><cellWatch r="B2"/><cellWatch r="B3"/><cellWatch r="F6"/></cellWatches>"#;
    let destination = r#"<cellWatches><cellWatch r="J10"/><cellWatch r="J11"/><cellWatch r="M20"/></cellWatches>"#;
    let (mut workbook, before, undo) = carried_scope_cut(source, destination);
    let after = table_member_map(&workbook);
    let entries = |part| carried_scope_entries(&after, part, "cellWatches", "cellWatch", &["r"]);
    assert_eq!(entries("xl/worksheets/sheet1.xml"), vec![vec!["F6"]]);
    assert_eq!(
        entries("xl/worksheets/sheet2.xml"),
        vec![vec!["J10"], vec!["J11"], vec!["M20"],]
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn carried_scope_cut_keeps_same_coordinates_on_the_new_owner() {
    use yggdryl::excel::Edit;
    for (container, child, key, expected, fragment) in [
        (
            "conditionalFormatting",
            "",
            "sqref",
            "B2:B3",
            r#"<conditionalFormatting sqref="B2:B3"><cfRule type="expression" priority="1"><formula>B2&gt;0</formula></cfRule></conditionalFormatting>"#,
        ),
        (
            "dataValidations",
            "dataValidation",
            "sqref",
            "B2:B3",
            r#"<dataValidations count="1"><dataValidation type="whole" sqref="B2:B3"><formula1>1</formula1></dataValidation></dataValidations>"#,
        ),
        (
            "hyperlinks",
            "hyperlink",
            "ref",
            "B2",
            r#"<hyperlinks><hyperlink ref="B2" location="Data!A1"/></hyperlinks>"#,
        ),
        (
            "protectedRanges",
            "protectedRange",
            "sqref",
            "B2:B3",
            r#"<protectedRanges><protectedRange name="moved" sqref="B2:B3"/></protectedRanges>"#,
        ),
        (
            "ignoredErrors",
            "ignoredError",
            "sqref",
            "B2:B3",
            r#"<ignoredErrors><ignoredError sqref="B2:B3" numberStoredAsText="1"/></ignoredErrors>"#,
        ),
        (
            "cellWatches",
            "cellWatch",
            "r",
            "B2",
            r#"<cellWatches><cellWatch r="B2"/></cellWatches>"#,
        ),
    ] {
        let mut workbook = carried_cross_cut_fixture(fragment, false);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "B2:B3".parse().unwrap()),
                to: ("Other".into(), at("B2")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        let source = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet1.xml"]).unwrap();
        let source = yggdryl::xml::Element::root(&source).unwrap();
        assert!(
            source.child(Some(NS), container).is_none(),
            "{container}: old owner"
        );
        let destination = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
        let destination = yggdryl::xml::Element::root(&destination).unwrap();
        let moved = destination
            .child(Some(NS), container)
            .expect("unchanged coordinates still change ownership");
        let entry = if child.is_empty() {
            moved
        } else {
            moved.child(Some(NS), child).unwrap()
        };
        assert_eq!(entry.attribute_in(None, key), Some(expected), "{container}");
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn carried_scope_cut_keeps_destination_ignored_extension_last() {
    let source =
        r#"<ignoredErrors><ignoredError sqref="B2:B3" numberStoredAsText="1"/></ignoredErrors>"#;
    let destination = r#"<ignoredErrors><ignoredError sqref="M20" evalError="1"/><extLst><ext uri="urn:existing"><v:payload xmlns:v="urn:vendor" v:keep="yes"/></ext></extLst></ignoredErrors>"#;
    let (mut workbook, before, undo) = carried_scope_cut(source, destination);
    let after = table_member_map(&workbook);
    let document = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let container = root.child(Some(NS), "ignoredErrors").unwrap();
    let children = container.children_in(Some(NS), "ignoredError");
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].attribute_in(None, "sqref"), Some("M20"));
    assert_eq!(children[1].attribute_in(None, "sqref"), Some("J10:J11"));
    // The XML value groups children by expanded name; wire order is a fact
    // of the original XML, not of that grouped view's children iterator.
    let xml = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(
        xml.find("<ignoredError sqref=\"M20\"").unwrap()
            < xml.find("<ignoredError sqref=\"J10:J11\"").unwrap()
    );
    assert!(xml.find("<ignoredError sqref=\"J10:J11\"").unwrap() < xml.find("<extLst>").unwrap());
    let extensions = container.child(Some(NS), "extLst").unwrap();
    let extension = extensions.child(Some(NS), "ext").unwrap();
    assert_eq!(extension.attribute_in(None, "uri"), Some("urn:existing"));
    assert_eq!(
        extension
            .child(Some("urn:vendor"), "payload")
            .unwrap()
            .attribute_in(Some("urn:vendor"), "keep"),
        Some("yes")
    );
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn carried_scope_cut_preserves_inherited_vendor_attribute_meaning() {
    use yggdryl::excel::Edit;
    for (container, child, fragment) in [
        (
            "protectedRanges",
            "protectedRange",
            r#"<protectedRanges><protectedRange name="moved" sqref="B2:B3"/></protectedRanges>"#,
        ),
        (
            "ignoredErrors",
            "ignoredError",
            r#"<ignoredErrors><ignoredError sqref="B2:B3" numberStoredAsText="1"/></ignoredErrors>"#,
        ),
        (
            "cellWatches",
            "cellWatch",
            r#"<cellWatches><cellWatch r="B2"/></cellWatches>"#,
        ),
    ] {
        let base = carried_cross_cut_fixture(fragment, false);
        let mut parts = table_member_map(&base);
        let source = std::str::from_utf8(&parts["xl/worksheets/sheet1.xml"])
            .unwrap()
            .to_owned();
        let marker = format!("<{child} ");
        assert_eq!(source.matches(&marker).count(), 1);
        parts.insert("xl/worksheets/sheet1.xml".into(), source.replace(
            "<worksheet xmlns=", "<worksheet xmlns:v=\"urn:source\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"v\" xmlns=",
        ).replace(&marker, &format!("<{child} v:flag=\"authored\" ")).into_bytes());
        let destination = std::str::from_utf8(&parts["xl/worksheets/sheet2.xml"])
            .unwrap()
            .to_owned();
        parts.insert(
            "xl/worksheets/sheet2.xml".into(),
            destination
                .replace(
                    "<worksheet xmlns=",
                    "<worksheet xmlns:v=\"urn:target\" xmlns=",
                )
                .into_bytes(),
        );
        let mut workbook = cross_table_reopen(&parts);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "B2:B3".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        let document = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let container = root.child(Some(NS), container).unwrap();
        let moved = container.child(Some(NS), child).unwrap();
        assert_eq!(
            moved.attribute_in(Some("urn:source"), "flag"),
            Some("authored")
        );
        assert_eq!(moved.attribute_in(Some("urn:target"), "flag"), None);
        assert_eq!(
            container.attribute_in(
                Some("http://schemas.openxmlformats.org/markup-compatibility/2006"),
                "Ignorable"
            ),
            Some("v")
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn carried_scope_cut_refuses_unowned_source_extension_atomically() {
    use yggdryl::excel::Edit;
    // The first registration can move; the second has an extension whose
    // ownership cannot be inferred from its ignored-error range.
    let fragment = r#"<protectedRanges><protectedRange name="moved" sqref="B2:B3"/></protectedRanges><ignoredErrors><ignoredError sqref="B2:B3" numberStoredAsText="1"/><extLst><ext uri="urn:opaque"/></extLst></ignoredErrors>"#;
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    let dirty = workbook.is_dirty();
    let error = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap_err();
    assert!(matches!(error, Error::Unsupported { operation, filesystem }
        if operation == "moving ignored-error container extensions"
            && filesystem == "xl/worksheets/sheet1.xml#ignoredErrors/extLst"));
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert_eq!(workbook.is_dirty(), dirty);
}

#[test]
fn carried_scope_cut_leaves_unselected_container_extensions_with_their_owner() {
    use yggdryl::excel::Edit;
    for fragment in [
        r#"<dataValidations xmlns:v="urn:vendor" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="v" count="1"><dataValidation type="whole" sqref="F6"><formula1>1</formula1></dataValidation><v:payload keep="authored"/></dataValidations>"#,
        r#"<ignoredErrors><ignoredError sqref="F6" numberStoredAsText="1"/><extLst><ext uri="urn:existing"><v:payload xmlns:v="urn:vendor" keep="authored"/></ext></extLst></ignoredErrors>"#,
    ] {
        for target in ["B2", "J10"] {
            let mut workbook = carried_cross_cut_fixture(fragment, false);
            let before = table_member_map(&workbook);
            let applied = workbook
                .apply(Edit::Paste {
                    from: ("Data".into(), "B2:B3".parse().unwrap()),
                    to: ("Other".into(), at(target)),
                    what: Paste::All,
                    cut: true,
                })
                .unwrap();
            assert_eq!(
                table_member_map(&workbook),
                before,
                "no source registration selected at target {target}"
            );
            note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
        }
    }
}

#[test]
fn carried_scope_cut_refuses_conflicting_expanded_container_attributes_atomically() {
    use yggdryl::excel::Edit;
    for (container, moved, kept) in [
        (
            "dataValidations",
            r#"<dataValidation type="whole" sqref="B2:B3"><formula1>1</formula1></dataValidation>"#,
            r#"<dataValidation type="whole" sqref="M20"><formula1>1</formula1></dataValidation>"#,
        ),
        (
            "protectedRanges",
            r#"<protectedRange name="moved" sqref="B2:B3"/>"#,
            r#"<protectedRange name="kept" sqref="M20"/>"#,
        ),
    ] {
        let source = format!(
            r#"<{container} xmlns:v="urn:source" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="v" v:flag="authored">{moved}</{container}>"#
        );
        let destination = format!(
            r#"<{container} xmlns:v="urn:target" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="v" v:flag="authored">{kept}</{container}>"#
        );
        let base = sheets_book(
            &[
                ("Data", sheet("", &source), &[]),
                ("Other", sheet("", &destination), &[]),
            ],
            &[],
            &[],
        );
        let mut workbook = carried_cross_cut_ready(cross_table_reopen(&table_member_map(&base)));
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let error = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "B2:B3".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported { operation, filesystem }
            if operation == "merging carried container attributes"
                && filesystem == format!("xl/worksheets/sheet2.xml#{container}")));
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn carried_scope_cut_merges_equivalent_expanded_container_attributes() {
    for inherited in [false, true] {
        let source = r#"<protectedRanges xmlns:v="urn:sha&#114;ed" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="v" v:flag="authored"><protectedRange name="moved" sqref="B2:B3"/></protectedRanges>"#;
        let destination = r#"<protectedRanges xmlns:x="urn:shared" xmlns:compat="http://schemas.openxmlformats.org/markup-compatibility/2006" compat:Ignorable="x" x:flag="authored"><protectedRange name="kept" sqref="M20"/></protectedRanges>"#;
        let base = sheets_book(
            &[
                ("Data", sheet("", source), &[]),
                ("Other", sheet("", destination), &[]),
            ],
            &[],
            &[],
        );
        let mut parts = table_member_map(&base);
        if inherited {
            for (part, declaration) in [
                (
                    "xl/worksheets/sheet1.xml",
                    r#" xmlns:v="urn:sha&#114;ed" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#,
                ),
                (
                    "xl/worksheets/sheet2.xml",
                    r#" xmlns:x="urn:shared" xmlns:compat="http://schemas.openxmlformats.org/markup-compatibility/2006""#,
                ),
            ] {
                let xml = std::str::from_utf8(&parts[part]).unwrap();
                assert_eq!(xml.matches(declaration).count(), 1);
                let xml = xml
                    .replace(declaration, "")
                    .replace("<worksheet ", &format!("<worksheet{declaration} "));
                parts.insert(part.into(), xml.into_bytes());
            }
        }
        let mut workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(yggdryl::excel::Edit::Paste {
                from: ("Data".into(), "B2:B3".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let after = table_member_map(&workbook);
        let document = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet2.xml"]).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let container = root.child(Some(NS), "protectedRanges").unwrap();
        assert_eq!(
            container.attribute_in(Some("urn:shared"), "flag"),
            Some("authored")
        );
        let mut references: Vec<_> = container
            .children_in(Some(NS), "protectedRange")
            .into_iter()
            .map(|child| child.attribute_in(None, "sqref").unwrap().to_owned())
            .collect();
        references.sort();
        assert_eq!(references, ["J10:J11", "M20"]);
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

fn worksheet_filter_native_fixture(
    source: &str,
    destination: &str,
    destination_name: Option<&str>,
) -> Workbook {
    let base = sheets_book(
        &[
            (
                "Data",
                sheet(
                    r#"<row r="2"><c r="B2" t="inlineStr"><is><t>Name</t></is></c></row><row r="3"><c r="B3"><v>11</v></c></row><row r="5"><c r="D5"><v>33</v></c></row>"#,
                    source,
                ),
                &[],
            ),
            (
                "Other",
                sheet(r#"<row r="30"><c r="Z30"><v>99</v></c></row>"#, destination),
                &[],
            ),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    if source.starts_with("<autoFilter") {
        let mut names = String::from(
            r#"<definedNames><definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$B$2:$D$5</definedName>"#,
        );
        if let Some(formula) = destination_name {
            names.push_str(&format!(r#"<definedName name="_xlnm._FilterDatabase" localSheetId="1" hidden="1">{formula}</definedName>"#));
        }
        names.push_str("</definedNames>");
        let xml = std::str::from_utf8(&parts["xl/workbook.xml"]).unwrap();
        parts.insert(
            "xl/workbook.xml".into(),
            xml.replace("</sheets>", &format!("</sheets>{names}"))
                .into_bytes(),
        );
    }
    carried_cross_cut_ready(cross_table_reopen(&parts))
}

fn worksheet_filter_native_source() -> &'static str {
    r#"<autoFilter ref="B2:D5"><filterColumn colId="0"><filters><filter val="alpha"/><filter val="gamma"/></filters></filterColumn><sortState ref="B2:D5"><sortCondition ref="C3:C5"/></sortState></autoFilter>"#
}

fn worksheet_filter_native_names(workbook: &Workbook) -> Vec<(String, String, bool)> {
    workbook
        .defined_names()
        .filter(|name| name.name() == "_xlnm._FilterDatabase")
        .map(|name| {
            (
                workbook
                    .sheet_by_key(name.scope().unwrap())
                    .unwrap()
                    .to_owned(),
                name.text(),
                name.is_hidden(),
            )
        })
        .collect()
}

#[test]
fn worksheet_filter_cut_matches_native_full_cross_sheet_removal_and_name_scope() {
    use yggdryl::excel::Edit;
    // Excel 16.0 build 20430.0: worksheet-filter-cut filter-cross and filter-same-coordinates.
    for (target, formula) in [("J10", "Other!$J$10:$L$13"), ("B2", "Other!$B$2:$D$5")] {
        let mut workbook =
            worksheet_filter_native_fixture(worksheet_filter_native_source(), "", None);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "B2:D5".parse().unwrap()),
                to: ("Other".into(), at(target)),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        for sheet in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            let document = yggdryl::xml::from_bytes(&table_member_map(&workbook)[sheet]).unwrap();
            assert!(
                yggdryl::xml::Element::root(&document)
                    .unwrap()
                    .child(Some(NS), "autoFilter")
                    .is_none()
            );
        }
        assert_eq!(
            worksheet_filter_native_names(&workbook),
            [("Data".into(), formula.into(), true)]
        );
        assert_eq!(
            workbook.sheet("Other").unwrap().scalar(at("Z30")),
            99.0.into()
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn worksheet_filter_cut_matches_native_same_sheet_criteria_clear_and_nested_sort() {
    use yggdryl::excel::Edit;
    let mut workbook = worksheet_filter_native_fixture(worksheet_filter_native_source(), "", None);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:D5".parse().unwrap()),
            to: ("Data".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let parts = table_member_map(&workbook);
    let document = yggdryl::xml::from_bytes(&parts["xl/worksheets/sheet1.xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let filter = root.child(Some(NS), "autoFilter").unwrap();
    assert_eq!(filter.attribute_in(None, "ref"), Some("J10:L13"));
    assert!(filter.child(Some(NS), "filterColumn").is_none());
    let sort = filter.child(Some(NS), "sortState").unwrap();
    assert_eq!(sort.attribute_in(None, "ref"), Some("J10:L13"));
    assert_eq!(
        sort.child(Some(NS), "sortCondition")
            .unwrap()
            .attribute_in(None, "ref"),
        Some("K11:K13")
    );
    assert_eq!(
        worksheet_filter_native_names(&workbook),
        [("Data".into(), "Data!$J$10:$L$13".into(), true)]
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn worksheet_filter_cut_matches_native_unaffected_and_covered_destination_filters() {
    use yggdryl::excel::Edit;
    for (range, name, remains) in [
        ("M20:O23", "Other!$M$20:$O$23", true),
        ("J10:L13", "Other!$J$10:$L$13", false),
    ] {
        let destination = format!(
            r#"<autoFilter ref="{range}"><filterColumn colId="0"><filters><filter val="delta"/></filters></filterColumn></autoFilter>"#
        );
        let mut workbook = worksheet_filter_native_fixture(
            worksheet_filter_native_source(),
            &destination,
            Some(name),
        );
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "B2:D5".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let source = member(&workbook, "xl/worksheets/sheet1.xml");
        assert!(!source.contains("<autoFilter"));
        let target = member(&workbook, "xl/worksheets/sheet2.xml");
        if remains {
            assert!(target.contains(&destination), "{target}");
        } else {
            assert!(!target.contains("<autoFilter"), "{target}");
        }
        assert_eq!(
            worksheet_filter_native_names(&workbook),
            [
                ("Data".into(), "Other!$J$10:$L$13".into(), true),
                (
                    "Other".into(),
                    if remains {
                        name.into()
                    } else {
                        "Other!#REF!".into()
                    },
                    true
                ),
            ]
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn worksheet_filter_cut_matches_native_body_only_name_contraction() {
    use yggdryl::excel::Edit;
    let mut workbook = worksheet_filter_native_fixture(worksheet_filter_native_source(), "", None);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B3:D5".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    assert!(
        member(&workbook, "xl/worksheets/sheet1.xml").contains(worksheet_filter_native_source())
    );
    assert!(!member(&workbook, "xl/worksheets/sheet2.xml").contains("<autoFilter"));
    assert_eq!(
        worksheet_filter_native_names(&workbook),
        [("Data".into(), "Data!$B$2:$D$2".into(), true)]
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn worksheet_filter_cut_matches_native_standalone_sort_retention() {
    use yggdryl::excel::Edit;
    let sort = r#"<sortState ref="B3:D5"><sortCondition ref="C3:C5"/></sortState>"#;
    let mut workbook = worksheet_filter_native_fixture(sort, "", None);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:D5".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    assert!(member(&workbook, "xl/worksheets/sheet1.xml").contains(sort));
    assert!(!member(&workbook, "xl/worksheets/sheet2.xml").contains("<sortState"));
    assert!(worksheet_filter_native_names(&workbook).is_empty());
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

fn worksheet_filter_partial_native_fixture() -> Workbook {
    let rows = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Amount</t></is></c><c r="C1" t="inlineStr"><is><t>Count</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>alpha</t></is></c><c r="B2"><v>11</v></c><c r="C2"><v>3</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>beta</t></is></c><c r="B3"><v>22</v></c><c r="C3"><v>2</v></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>gamma</t></is></c><c r="B4"><v>33</v></c><c r="C4"><v>1</v></c></row>"#;
    let filter = r#"<autoFilter ref="A1:C4"><filterColumn colId="1"><customFilters><customFilter operator="greaterThan" val="1"/></customFilters></filterColumn><sortState ref="A2:C4"><sortCondition ref="B2:B4"/></sortState></autoFilter>"#;
    let base = sheets_book(
        &[
            ("Data", sheet(rows, filter), &[]),
            ("Other", sheet("", ""), &[]),
        ],
        &[],
        &[],
    );
    let mut parts = table_member_map(&base);
    let xml = std::str::from_utf8(&parts["xl/workbook.xml"]).unwrap();
    let names = r#"<definedNames><definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$A$1:$C$4</definedName></definedNames>"#;
    parts.insert(
        "xl/workbook.xml".into(),
        xml.replace("</sheets>", &format!("</sheets>{names}"))
            .into_bytes(),
    );
    let mut workbook = cross_table_reopen(&parts);
    for name in ["Data", "Other"] {
        let sheet = workbook.sheet_mut(name).unwrap();
        sheet.set_cell(at("Z100"), 1.0).unwrap();
        sheet.remove_cell(at("Z100"));
    }
    Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap()
}

#[test]
fn worksheet_filter_partial_cut_preserves_native_filter_sort_and_name() {
    use yggdryl::excel::Edit;
    // Excel 16.0 build 20430.0: all four cuts kept the filter, criterion,
    // nested sort ranges and FilterDatabase name in the source worksheet.
    for (block, destination) in [
        ("B2:C3", "Data"),
        ("B2:C3", "Other"),
        ("A1:B2", "Data"),
        ("A1:B2", "Other"),
    ] {
        let mut workbook = worksheet_filter_partial_native_fixture();
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), block.parse().unwrap()),
                to: (destination.into(), at("H8")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let source = member(&workbook, "xl/worksheets/sheet1.xml");
        let document = yggdryl::xml::from_bytes(source.as_bytes()).unwrap();
        let root = yggdryl::xml::Element::root(&document).unwrap();
        let filter = root.child(Some(NS), "autoFilter").unwrap();
        assert_eq!(filter.attribute_in(None, "ref"), Some("A1:C4"));
        let criterion = filter.child(Some(NS), "filterColumn").unwrap();
        assert_eq!(criterion.attribute_in(None, "colId"), Some("1"));
        let criteria = criterion.child(Some(NS), "customFilters").unwrap();
        let criterion = criteria.child(Some(NS), "customFilter").unwrap();
        assert_eq!(
            criterion.attribute_in(None, "operator"),
            Some("greaterThan")
        );
        assert_eq!(criterion.attribute_in(None, "val"), Some("1"));
        let sort = filter.child(Some(NS), "sortState").unwrap();
        assert_eq!(sort.attribute_in(None, "ref"), Some("A2:C4"));
        assert_eq!(
            sort.child(Some(NS), "sortCondition")
                .unwrap()
                .attribute_in(None, "ref"),
            Some("B2:B4")
        );
        assert_eq!(
            worksheet_filter_native_names(&workbook),
            [("Data".into(), "Data!$A$1:$C$4".into(), true)]
        );
        if destination == "Other" {
            assert!(!member(&workbook, "xl/worksheets/sheet2.xml").contains("<autoFilter"));
        }
        let expected: Scalar = if block == "B2:C3" {
            11.0.into()
        } else {
            "Item".into()
        };
        assert_eq!(
            workbook.sheet(destination).unwrap().scalar(at("H8")),
            expected
        );
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn worksheet_filter_cut_refuses_unobserved_partial_geometry_atomically() {
    use yggdryl::excel::Edit;
    for (block, destination, location) in [
        ("B2:C5", "", "xl/worksheets/sheet1.xml#autoFilter"),
        (
            "B2:D5",
            r#"<autoFilter ref="K11:M14"/>"#,
            "xl/worksheets/sheet2.xml#autoFilter",
        ),
    ] {
        let mut workbook =
            worksheet_filter_native_fixture(worksheet_filter_native_source(), destination, None);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let dirty = workbook.is_dirty();
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let error = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), block.parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported { operation, filesystem }
            if operation == "moving an unmodeled worksheet filter overlap" && filesystem == location));
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn worksheet_filter_cut_resolves_inherited_namespaces_before_clearing_criteria() {
    use yggdryl::excel::Edit;
    let base = worksheet_filter_native_fixture(worksheet_filter_native_source(), "", None);
    let mut parts = table_member_map(&base);
    let xml = std::str::from_utf8(&parts["xl/worksheets/sheet1.xml"]).unwrap();
    let xml = xml
        .replace(
            "<worksheet ",
            &format!(r#"<worksheet xmlns:m="{NS}" xmlns:v="urn:filter-vendor" "#),
        )
        .replace("<autoFilter ", "<m:autoFilter ")
        .replace("</autoFilter>", "</m:autoFilter>")
        .replace("<filterColumn ", "<m:filterColumn ")
        .replace("</filterColumn>", "</m:filterColumn>")
        .replace(
            "</m:filterColumn>",
            r#"</m:filterColumn><v:filterColumn colId="77"/><filterColumn xmlns="" colId="88"/>"#,
        )
        .replace(
            "</sortState>",
            r#"<v:sortCondition ref="C3:C5"/></sortState>"#,
        );
    parts.insert("xl/worksheets/sheet1.xml".into(), xml.into_bytes());
    let mut workbook = cross_table_reopen(&parts);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:D5".parse().unwrap()),
            to: ("Data".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let document = yggdryl::xml::from_bytes(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let filter = root.child(Some(NS), "autoFilter").unwrap();
    assert_eq!(filter.attribute_in(None, "ref"), Some("J10:L13"));
    assert!(filter.children_in(Some(NS), "filterColumn").is_empty());
    assert_eq!(
        filter
            .child(Some("urn:filter-vendor"), "filterColumn")
            .unwrap()
            .attribute_in(None, "colId"),
        Some("77")
    );
    assert_eq!(
        filter
            .child(None, "filterColumn")
            .unwrap()
            .attribute_in(None, "colId"),
        Some("88")
    );
    let sort = filter.child(Some(NS), "sortState").unwrap();
    assert_eq!(
        sort.child(Some(NS), "sortCondition")
            .unwrap()
            .attribute_in(None, "ref"),
        Some("K11:K13")
    );
    assert_eq!(
        sort.child(Some("urn:filter-vendor"), "sortCondition")
            .unwrap()
            .attribute_in(None, "ref"),
        Some("C3:C5")
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn worksheet_filter_cut_keeps_an_identity_move_and_unaffected_complex_name() {
    use yggdryl::excel::Edit;
    for identity in [true, false] {
        let mut workbook =
            worksheet_filter_native_fixture(worksheet_filter_native_source(), "", None);
        if !identity {
            let mut parts = table_member_map(&workbook);
            let xml = std::str::from_utf8(&parts["xl/workbook.xml"]).unwrap();
            parts.insert(
                "xl/workbook.xml".into(),
                xml.replace(
                    ">Data!$B$2:$D$5</definedName>",
                    ">SUM(Data!$B$2:$D$5)</definedName>",
                )
                .into_bytes(),
            );
            workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
        }
        let before = table_member_map(&workbook);
        let names = worksheet_filter_native_names(&workbook);
        let (block, destination, target) = if identity {
            ("B2:D5", "Data", "B2")
        } else {
            ("X20:X21", "Other", "X25")
        };
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), block.parse().unwrap()),
                to: (destination.into(), at(target)),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        assert!(
            member(&workbook, "xl/worksheets/sheet1.xml")
                .contains(worksheet_filter_native_source())
        );
        assert_eq!(worksheet_filter_native_names(&workbook), names);
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn cross_sheet_cut_x14_extension_uri_is_normalized_and_unqualified() {
    use yggdryl::excel::Edit;
    let ordinary = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let encoded = ordinary.replacen("uri=\"{78", "uri=\"{&#x37;8", 1);
    let foreign = ordinary.replacen("uri=\"{78", "v:uri=\"{78", 1).replacen(
        "<extLst>",
        "<extLst xmlns:v=\"urn:vendor\">",
        1,
    );
    for (fragment, allowed) in [(encoded, true), (foreign, false)] {
        let mut workbook = carried_cross_cut_fixture(&fragment, false);
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let result = workbook.apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        });
        if allowed {
            let applied = result.expect("encoded unqualified x14 URI must move");
            let package = table_member_map(&workbook);
            let target = std::str::from_utf8(&package["xl/worksheets/sheet2.xml"]).unwrap();
            assert!(target.contains("<xm:sqref>J10:J11</xm:sqref>"), "{target}");
            note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
        } else {
            let error = result.expect_err("vendor:uri cannot classify the x14 extension");
            assert!(error.to_string().contains("sheet1.xml"), "{error}");
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert_eq!(workbook.is_dirty(), dirty);
        }
    }
}

#[test]
fn cross_sheet_cut_refuses_x14_rule_rebound_to_main_namespace() {
    use yggdryl::excel::Edit;
    let fragment = r#"<extLst><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule xmlns:x14="http://schemas.openxmlformats.org/spreadsheetml/2006/main" type="expression" priority="1"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let mut workbook = carried_cross_cut_fixture(fragment, false);
    workbook.parse_all().unwrap();
    let before = table_member_map(&workbook);
    let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
    let dirty = workbook.is_dirty();
    let error = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .expect_err("a rebound cfRule has no proved x14 owner");
    let diagnostic = error.to_string();
    assert!(
        diagnostic.contains("sheet1.xml") && diagnostic.contains("cfRule"),
        "{diagnostic}"
    );
    assert_eq!(table_member_map(&workbook), before);
    assert_eq!(
        ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
        revisions
    );
    assert_eq!(workbook.is_dirty(), dirty);
}

#[test]
fn cross_sheet_cut_ignores_ids_on_prioritized_x14_rules() {
    // MS-XLSX CT_CfRule: an x14 id is ignored when priority exists.
    const GUID: &str = "{00000000-000E-0000-0000-000001000000}";
    let fragment = format!(
        r#"<extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1" id="{GUID}"><xm:f>B2&gt;0</xm:f></x14:cfRule><xm:sqref>B2:B3</xm:sqref></x14:conditionalFormatting><x14:conditionalFormatting><x14:cfRule type="expression" priority="2" id="{GUID}"><xm:f>F6&gt;0</xm:f></x14:cfRule><xm:sqref>F6</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
    );
    let workbook = carried_cross_cut_fixture(&fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "B2:B3",
        "<xm:sqref>J10:J11</xm:sqref>",
        "<xm:sqref>B2:B3</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    let target = std::str::from_utf8(&after["xl/worksheets/sheet2.xml"]).unwrap();
    assert!(source.contains("<xm:sqref>F6</xm:sqref>"), "{source}");
    assert!(target.contains("<xm:sqref>J10:J11</xm:sqref>"), "{target}");
    assert!(source.contains(GUID) && target.contains(GUID));
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn cross_sheet_partial_databar_forks_one_linked_guid_pair() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
    const OLD: &str = "{00000000-000E-0000-0000-000001000000}";
    let source = format!(
        r#"<conditionalFormatting sqref="B2:B4"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}" xmlns:x14="{X14}"><x14:id>{OLD}</x14:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="{X14}" xmlns:xm="{XM}"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" id="{OLD}"><x14:dataBar minLength="0" maxLength="100" gradient="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>B2:B4</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
    );
    let mut workbook = carried_cross_cut_fixture(&source, false);
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let after = table_member_map(&workbook);
    let linked = |bytes: &[u8], range: &str| -> (String, String) {
        let document = from_bytes(bytes).unwrap();
        let root = Element::root(&document).unwrap();
        let legacy = root
            .children_in(Some(NS), "conditionalFormatting")
            .into_iter()
            .find(|host| host.attribute_in(None, "sqref") == Some(range))
            .unwrap();
        let legacy_rule = legacy.child(Some(NS), "cfRule").unwrap();
        let legacy_list = legacy_rule.child(Some(NS), "extLst").unwrap();
        let legacy_ext = legacy_list.child(Some(NS), "ext").unwrap();
        let legacy_id = legacy_ext
            .child(Some(X14), "id")
            .unwrap()
            .text()
            .unwrap()
            .to_owned();
        let ext_list = root.child(Some(NS), "extLst").unwrap();
        let extension = ext_list.child(Some(NS), "ext").unwrap();
        let list = extension
            .child(Some(X14), "conditionalFormattings")
            .unwrap();
        let host = list
            .children_in(Some(X14), "conditionalFormatting")
            .into_iter()
            .find(|host| host.child(Some(XM), "sqref").unwrap().text() == Some(range))
            .unwrap();
        let x14_id = host
            .child(Some(X14), "cfRule")
            .unwrap()
            .attribute_in(None, "id")
            .unwrap()
            .to_owned();
        (legacy_id, x14_id)
    };
    let retained = linked(&after["xl/worksheets/sheet1.xml"], "B4");
    let incoming = linked(&after["xl/worksheets/sheet2.xml"], "J10:J11");
    assert_eq!(retained.0, OLD);
    assert_eq!(retained.0, retained.1);
    assert_eq!(incoming.0, incoming.1);
    assert_ne!(incoming.0, OLD);
    yggdryl::Uuid::from_bytes(
        incoming
            .0
            .trim_start_matches('{')
            .trim_end_matches('}')
            .as_bytes(),
    )
    .unwrap();
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

#[test]
fn retained_carried_formula_fields_require_the_complete_standard_ancestor_path() {
    let legacy_opaque = r#"<v:opaque><cfvo type="formula" val="A1"/></v:opaque>"#;
    let x14_opaque = r#"<v:opaque><x14:formula1><xm:f>A1</xm:f></x14:formula1></v:opaque>"#;
    for (fragment, opaque, genuine) in [
        (
            format!(
                r#"<conditionalFormatting xmlns:v="urn:vendor" sqref="H8:J10"><cfRule type="colorScale" priority="1"><colorScale><cfvo type="formula" val="A1"/><cfvo type="max"/><color rgb="FFFF0000"/><color rgb="FF00FF00"/></colorScale><extLst><ext uri="urn:vendor:opaque">{legacy_opaque}</ext></extLst></cfRule></conditionalFormatting>"#
            ),
            legacy_opaque,
            "val=\"Other!J10\"",
        ),
        (
            format!(
                r#"<extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main" xmlns:v="urn:vendor"><x14:dataValidations count="1"><x14:dataValidation type="custom"><x14:formula1><xm:f>A1</xm:f></x14:formula1><xm:sqref>H8:J10</xm:sqref><x14:extLst><x14:ext uri="urn:vendor:opaque">{x14_opaque}</x14:ext></x14:extLst></x14:dataValidation></x14:dataValidations></ext></extLst>"#
            ),
            x14_opaque,
            "<xm:f>Other!J10</xm:f>",
        ),
    ] {
        let mut workbook = carried_cross_cut_fixture(&fragment, false);
        workbook
            .paste(
                ("Data", "A1".parse().unwrap()),
                ("Other", at("J10")),
                Paste::All,
                true,
            )
            .unwrap();
        let after = table_member_map(&workbook);
        let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
        assert!(
            source.contains(genuine),
            "the genuine field must follow the cut: {source}"
        );
        let opaque_count = source.matches("<v:opaque>").count();
        assert!(opaque_count > 0, "the extension must survive: {source}");
        assert_eq!(
            source.matches(opaque).count(),
            opaque_count,
            "a standard-looking leaf inside an opaque ancestor must stay exact: {source}"
        );
    }
}

#[test]
fn sparkline_cut_ignores_unchanged_bad_x14_cf_priority_in_same_ext_list() {
    let fragment = r#"<extLst><ext uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroups><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>Data!B2:C2</xm:f><xm:sqref>D2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext><ext uri="{78C0D931-6437-407d-A8EE-F0AAD7539E65}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="vendor"><xm:f>A1&gt;0</xm:f></x14:cfRule><xm:sqref>A1</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#;
    let workbook = carried_cross_cut_fixture(fragment, false);
    let (mut workbook, before, undo) = carried_cross_cut(
        workbook,
        "D2",
        "<xm:sqref>J10</xm:sqref>",
        "<xm:sqref>D2</xm:sqref>",
    );
    let after = table_member_map(&workbook);
    let source = std::str::from_utf8(&after["xl/worksheets/sheet1.xml"]).unwrap();
    assert!(source.contains("priority=\"vendor\""), "{source}");
    note_cut_saved_inverse(&mut workbook, &before, undo);
}

#[test]
fn unrelated_cell_cut_keeps_unchanged_validation_container_bytes() {
    for fragment in [
        r#"<dataValidations count="01" disablePrompts="0"><dataValidation sqref="H8:J10" type="whole"><formula1>7</formula1></dataValidation></dataValidations>"#,
        r#"<extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:dataValidations count="01" disablePrompts="0"><x14:dataValidation type="whole"><x14:formula1><xm:f>7</xm:f></x14:formula1><xm:sqref>H8:J10</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#,
    ] {
        let mut workbook = carried_cross_cut_fixture(fragment, false);
        let before = member(&workbook, "xl/worksheets/sheet1.xml");
        assert!(
            before.contains(fragment),
            "fixture preserves exact registration: {before}"
        );
        workbook
            .paste(
                ("Data", "A1".parse().unwrap()),
                ("Other", at("J10")),
                Paste::All,
                true,
            )
            .unwrap();
        let after = member(&workbook, "xl/worksheets/sheet1.xml");
        assert!(
            after.contains(fragment),
            "an unrelated cut must not normalize count or rewrite an unchanged registration: {after}"
        );
    }
}

#[test]
fn partial_range_hyperlinks_match_four_native_cut_cases_and_saved_inverse() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/partial_hyperlinks_excel.json")).unwrap();
    assert_eq!(fixture["provenance"]["excel"]["version"], "16.0");
    assert_eq!(fixture["provenance"]["cleanup_completed"], true);
    assert_eq!(fixture["provenance"]["reopened_without_repair"], true);
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let external = case["kind"] == "external";
        let source = if external {
            "<hyperlinks><hyperlink ref=\"B2:D5\" r:id=\"rIdLink\"/></hyperlinks>"
        } else {
            "<hyperlinks><hyperlink ref=\"B2:D5\" location=\"Data!A1\"/></hyperlinks>"
        };
        let base = sheets_book(
            &[
                ("Data", sheet("", source), &[]),
                ("Other", sheet("", ""), &[]),
            ],
            &[],
            &[],
        );
        let mut parts = table_member_map(&base);
        if external {
            parts.insert("xl/worksheets/_rels/sheet1.xml.rels".into(), format!(
                "<Relationships xmlns=\"{PKG_RELS}\"><Relationship Id=\"rIdLink\" Type=\"{R_NS}/hyperlink\" Target=\"https://example.invalid/range-hyperlink\" TargetMode=\"External\"/></Relationships>"
            ).into_bytes());
        }
        let mut workbook = carried_cross_cut_ready(cross_table_reopen(&parts));
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: (
                    "Data".into(),
                    case["block"].as_str().unwrap().parse().unwrap(),
                ),
                to: (
                    case["destination"].as_str().unwrap().into(),
                    case["target"].as_str().unwrap().parse().unwrap(),
                ),
                what: Paste::All,
                cut: true,
            })
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        let after = table_member_map(&workbook);
        for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            let document = from_bytes(&after[part]).unwrap();
            let root = Element::root(&document).unwrap();
            let list = root.child(Some(NS), "hyperlinks");
            let links = list
                .as_ref()
                .map(|list| list.children_in(Some(NS), "hyperlink"))
                .unwrap_or_default();
            let expected = case["after"][part].as_array().unwrap();
            assert_eq!(links.len(), expected.len(), "{id} {part}");
            for (link, fact) in links.into_iter().zip(expected) {
                assert_eq!(
                    link.attribute_in(None, "ref"),
                    fact["ref"].as_str(),
                    "{id} {part}"
                );
                assert_eq!(
                    link.attribute_in(None, "location"),
                    fact["location"].as_str(),
                    "{id} {part}"
                );
                let actual_id = link.attribute_in(Some(R_NS), "id");
                if let Some(native_id) = fact["relationship_id"].as_str() {
                    assert!(!native_id.is_empty() && actual_id.is_some(), "{id} {part}");
                    if part.ends_with("sheet1.xml") {
                        assert_eq!(
                            actual_id,
                            Some("rIdLink"),
                            "{id} source relationship identity"
                        );
                    }
                    let rel_part = if part.ends_with("sheet1.xml") {
                        "xl/worksheets/_rels/sheet1.xml.rels"
                    } else {
                        "xl/worksheets/_rels/sheet2.xml.rels"
                    };
                    let rel_document = from_bytes(&after[rel_part]).unwrap();
                    let rel_root = Element::root(&rel_document).unwrap();
                    let relation = rel_root
                        .children_in(Some(PKG_RELS), "Relationship")
                        .into_iter()
                        .find(|entry| entry.attribute_in(None, "Id") == actual_id)
                        .unwrap();
                    assert_eq!(
                        relation.attribute_in(None, "Target"),
                        fact["target"].as_str(),
                        "{id} {part}"
                    );
                } else {
                    assert_eq!(actual_id, None, "{id} {part}");
                }
            }
        }
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn retained_cf_rule_templates_preserve_interleaved_opaque_siblings() {
    use yggdryl::excel::Edit;
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
    const GAP: &str = "<!--between-rules--><v:opaque keep=\"exact\">\u{e9}</v:opaque>";
    for extended in [false, true] {
        let (fragment, prefix, formula_tag) = if extended {
            (
                format!(
                    r#"<extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="{X14}" xmlns:xm="{XM}" xmlns:v="urn:vendor"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression" priority="1"><xm:f>A1</xm:f></x14:cfRule>{GAP}<x14:cfRule type="expression" priority="2"><xm:f>A1</xm:f></x14:cfRule><xm:sqref>H8:J10</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
                ),
                "x14:",
                "xm:f",
            )
        } else {
            (
                format!(
                    r#"<conditionalFormatting xmlns:v="urn:vendor" sqref="H8:J10"><cfRule type="expression" priority="1"><formula>A1</formula></cfRule>{GAP}<cfRule type="expression" priority="2"><formula>A1</formula></cfRule></conditionalFormatting>"#
                ),
                "",
                "formula",
            )
        };
        let mut workbook = carried_cross_cut_fixture(&fragment, false);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "A1".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let source = member(&workbook, "xl/worksheets/sheet1.xml");
        let close = format!("</{prefix}conditionalFormatting>");
        let open = format!("<{prefix}conditionalFormatting");
        let rule = format!("<{prefix}cfRule ");
        let mut count = 0;
        for (end, _) in source.match_indices(&close) {
            let start = source[..end].rfind(&open).unwrap();
            let host = &source[start..end];
            assert_eq!(host.matches(&rule).count(), 1, "{host}");
            assert!(
                host.contains(GAP),
                "opaque XML and its comment must stay exact: {host}"
            );
            assert!(
                host.contains(&format!("<{formula_tag}>Other!J10</{formula_tag}>")),
                "{host}"
            );
            let rule_at = host.find(&rule).unwrap();
            let gap_at = host.find(GAP).unwrap();
            if host.contains("priority=\"1\"") {
                assert!(
                    rule_at < gap_at,
                    "first rule must remain before opaque siblings: {host}"
                );
            } else {
                assert!(host.contains("priority=\"2\""), "{host}");
                assert!(
                    gap_at < rule_at,
                    "second rule must remain after opaque siblings: {host}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 2, "{source}");
        note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
    }
}

#[test]
fn cross_sheet_validation_merge_retains_the_destination_extension_tail() {
    use yggdryl::excel::Edit;
    use yggdryl::xml::{Element, from_bytes};
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
    const TAIL: &str = r#"<x14:extLst><x14:ext uri="urn:vendor:tail"><v:opaque keep="exact"/></x14:ext></x14:extLst>"#;
    let extension = |validation: &str, tail: &str| {
        format!(
            r#"<extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="{X14}" xmlns:xm="{XM}" xmlns:v="urn:vendor"><x14:dataValidations count="1">{validation}{tail}</x14:dataValidations></ext></extLst>"#
        )
    };
    let source = extension(
        r#"<x14:dataValidation type="whole"><x14:formula1><xm:f>B2</xm:f></x14:formula1><xm:sqref>B2:B3</xm:sqref></x14:dataValidation>"#,
        "",
    );
    let destination = extension(
        r#"<x14:dataValidation type="whole"><x14:formula1><xm:f>7</xm:f></x14:formula1><xm:sqref>A1</xm:sqref></x14:dataValidation>"#,
        TAIL,
    );
    let mut workbook = carried_cross_cut_ready(sheets_book(
        &[
            ("Data", sheet("", &source), &[]),
            ("Other", sheet("", &destination), &[]),
        ],
        &[],
        &[],
    ));
    let before = table_member_map(&workbook);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:B3".parse().unwrap()),
            to: ("Other".into(), at("J10")),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    let target = member(&workbook, "xl/worksheets/sheet2.xml");
    assert!(
        target.contains(TAIL),
        "unrelated extension bytes must remain exact: {target}"
    );
    // Element's map groups children by name; inspect the carried XML for
    // schema order so a misplaced extLst cannot pass through that grouping.
    assert!(
        target.rfind("</x14:dataValidation>").unwrap() < target.find(TAIL).unwrap(),
        "validations must precede the extension tail: {target}"
    );
    let document = from_bytes(target.as_bytes()).unwrap();
    let root = Element::root(&document).unwrap();
    let ext_list = root.child(Some(NS), "extLst").unwrap();
    let extension = ext_list.child(Some(NS), "ext").unwrap();
    let validations = extension.child(Some(X14), "dataValidations").unwrap();
    assert_eq!(validations.attribute_in(None, "count"), Some("2"));
    assert_eq!(
        validations
            .children()
            .map(|child| child.local_name().to_owned())
            .collect::<Vec<_>>(),
        ["dataValidation", "dataValidation", "extLst"]
    );
    let children = validations.children_in(Some(X14), "dataValidation");
    assert_eq!(
        children[0].child(Some(XM), "sqref").unwrap().text(),
        Some("A1")
    );
    assert_eq!(
        children[1].child(Some(XM), "sqref").unwrap().text(),
        Some("J10:J11")
    );
    note_cut_saved_inverse(&mut workbook, &before, applied.inverse.unwrap());
}

fn calculation_cached(reference: &str, text: &str, cached: f64) -> Cell {
    use yggdryl::excel::Formula;
    let reference = at(reference);
    Cell::from_scalar(reference, Scalar::from(cached), DateSystem::Year1900)
        .unwrap()
        .with_formula(Formula::from_file(text, reference))
}

fn calculation_diamond() -> Workbook {
    let mut book = Workbook::new();
    book.add_sheet("Other")
        .unwrap()
        .insert_cell(calculation_cached("A1", "Data!A2*2", -1.0))
        .unwrap();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    for (reference, text) in [
        ("A2", "B1+C1"),
        ("B1", "A1*2"),
        ("C1", "A1+1"),
        ("D1", "7+8"),
    ] {
        data.insert_cell(calculation_cached(reference, text, -1.0))
            .unwrap();
    }
    book
}

#[test]
fn workbook_calculation_orders_cross_sheet_diamond_once_and_preserves_clean_cache() {
    let mut book = calculation_diamond();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (5, 0, 0)
    );
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 14.0.into());
    for (reference, value) in [("B1", 4.0), ("C1", 3.0), ("A2", 7.0), ("D1", 15.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(reference)),
            Scalar::from(value)
        );
    }
    let saved = book.into_package().unwrap();
    book.rebase(saved).unwrap();
    let revisions = [
        book.sheet("Other").unwrap().revision(),
        book.sheet("Data").unwrap().revision(),
    ];
    let again = book.calculate_all().unwrap();
    assert_eq!(
        again.evaluated, 5,
        "force-all remains distinct from incremental scheduling"
    );
    assert!(!book.is_dirty());
    assert_eq!(
        [
            book.sheet("Other").unwrap().revision(),
            book.sheet("Data").unwrap().revision()
        ],
        revisions
    );
}

#[test]
fn workbook_calculation_reports_only_true_cycle_members_and_holds_downstream() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    for (reference, text, cached) in [
        ("A1", "B1+1", 101.0),
        ("B1", "A1+1", 102.0),
        ("C1", "A1+10", 103.0),
        ("D1", "D1+1", 104.0),
        ("E1", "3+4", 105.0),
    ] {
        data.insert_cell(calculation_cached(reference, text, cached))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (1, 4, 3)
    );
    assert_eq!(
        report.circular,
        ["A1", "B1", "D1"]
            .map(|reference| ("Data".into(), at(reference)))
            .to_vec()
    );
    for (reference, value) in [
        ("A1", 101.0),
        ("B1", 102.0),
        ("C1", 103.0),
        ("D1", 104.0),
        ("E1", 7.0),
    ] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(reference)),
            Scalar::from(value)
        );
    }
}

#[test]
fn workbook_calculation_bounds_circular_list_without_truncating_status_count() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    for row in 1..=300 {
        let reference = format!("A{row}");
        data.insert_cell(calculation_cached(
            &reference,
            &format!("{reference}+1"),
            9.0,
        ))
        .unwrap();
    }
    data.insert_cell(calculation_cached("B1", "A1+1", 11.0))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 301, 300)
    );
    assert_eq!(report.circular.len(), 256);
    assert_eq!(report.circular.first(), Some(&("Data".into(), at("A1"))));
    assert_eq!(report.circular.last(), Some(&("Data".into(), at("A256"))));
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 11.0.into());
}

#[test]
fn workbook_calculation_late_ambiguous_name_preserves_all_caches_and_revisions() {
    let document = workbook(&["Data"], false).replace(
        "</workbook>",
        concat!(
            "<definedNames><definedName name=\"Rate\">1</definedName>",
            "<definedName name=\"Rate\">2</definedName></definedNames></workbook>",
        ),
    );
    let bytes = package(&[
        ("[Content_Types].xml", content_types(1, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", document),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet("")),
    ]);
    let mut book = Workbook::from_bytes(bytes).unwrap();
    for (reference, text) in [("A1", "1+2"), ("B1", "Rate+1")] {
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(calculation_cached(reference, text, 99.0))
            .unwrap();
    }
    let before = table_member_map(&book);
    let revision = book.sheet("Data").unwrap().revision();
    let dirty = book.is_dirty();
    let error = book.calculate_all().unwrap_err();
    assert!(
        matches!(error, Error::InvalidRecord { ref path, .. } if path.as_str() == "xl/workbook.xml#definedName[Rate]"),
        "{error}"
    );
    assert_eq!(book.sheet("Data").unwrap().scalar(at("A1")), 99.0.into());
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 99.0.into());
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
    assert_eq!(book.is_dirty(), dirty);
    assert_eq!(table_member_map(&book), before);
}

/// Compare the 38 owned Excel 16.0 reference observations with both the
/// authored OOXML formula and Excel's saved unmarked formula text.
#[test]
fn reference_array_legacy_native_cache_matches_authored_and_saved_formulas() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reference_array_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 38);
    for year in ["1900", "1904"] {
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        for replay_saved in [false, true] {
            let mut workbook = Workbook::new();
            workbook.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                workbook.add_sheet(name).unwrap();
            }
            for name in ["Values", "CycleShape"] {
                for (address, value) in fixture["input_source_cells"][name].as_object().unwrap() {
                    workbook
                        .sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value.as_f64().unwrap())
                        .unwrap();
                }
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let sheet = case["sheet"].as_str().unwrap();
                let address = at(case["cell"].as_str().unwrap());
                let formula = if replay_saved {
                    case["saved_cache"]["formula_text"].as_str().unwrap()
                } else {
                    case["wire_formula"].as_str().unwrap()
                };
                let cell = Cell::from_scalar(address, Scalar::Null, system)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, address));
                workbook
                    .sheet_mut(sheet)
                    .unwrap()
                    .insert_cell(cell)
                    .unwrap();
            }
            let report = workbook.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed),
                (19, 0),
                "{year} replay_saved={replay_saved}"
            );
            assert_eq!(
                report.circular_count, 0,
                "{year} replay_saved={replay_saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let sheet = case["sheet"].as_str().unwrap();
                let address = at(case["cell"].as_str().unwrap());
                let cell = workbook.sheet(sheet).unwrap().cell(address).unwrap();
                let cache = &case["saved_cache"];
                let label = format!("{} replay_saved={replay_saved}", case["id"]);
                match cache["type"].as_str().unwrap() {
                    "n" => {
                        let expected: f64 = cache["value_text"].as_str().unwrap().parse().unwrap();
                        assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(expected.to_bits()),
                            "{label}"
                        );
                        assert_eq!(cell.error(), None, "{label}");
                    }
                    "e" => {
                        assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            cache["value_text"].as_str(),
                            "{label}"
                        );
                    }
                    other => panic!("unhandled native cache type {other} in {label}"),
                }
            }
        }
    }
}

#[test]
fn workbook_recalculate_changes_only_the_dirty_closure_and_replaces_formula_edges() {
    let mut book = calculation_diamond();
    assert_eq!(book.recalculate().unwrap().evaluated, 5);
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 3.0)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 20.0.into());
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(calculation_cached("B1", "Z1+10", -1.0))
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 28.0.into());
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 4.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        3,
        "old B1->A1 dependency was removed"
    );
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 30.0.into());
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("Z1"), 2.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        3,
        "an absent scalar still owns an indexed dependency"
    );
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 34.0.into());
    book.sheet_mut("Data").unwrap().remove_cell(at("Z1"));
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    assert_eq!(book.sheet("Other").unwrap().scalar(at("A1")), 30.0.into());
}

#[test]
fn workbook_recalculate_keeps_workbook_status_when_no_formula_is_dirty() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    for (reference, text, cached) in [
        ("A1", "NO_SUCH_FUNCTION(1)", 100.0),
        ("B1", "A1+1", 200.0),
        ("C1", "C1+1", 300.0),
    ] {
        data.insert_cell(calculation_cached(reference, text, cached))
            .unwrap();
    }
    let first = book.recalculate().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (0, 3, 1)
    );
    let second = book.recalculate().unwrap();
    assert_eq!(
        second, first,
        "an unchanged pass cannot erase held/circular status"
    );
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(calculation_cached("A1", "1+1", 100.0))
        .unwrap();
    let repaired = book.recalculate().unwrap();
    assert_eq!(
        (
            repaired.evaluated,
            repaired.uncomputed,
            repaired.circular_count
        ),
        (2, 1, 1)
    );
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 3.0.into());
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("C1"), 10.0)
        .unwrap();
    let removed = book.recalculate().unwrap();
    assert_eq!(
        (
            removed.evaluated,
            removed.uncomputed,
            removed.circular_count
        ),
        (0, 0, 0)
    );
    assert!(removed.circular.is_empty());
}

#[test]
fn workbook_recalculate_observes_forgotten_cell_guards_but_not_a_noop_sheet_borrow() {
    use yggdryl::excel::Formula;
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.insert_cell(calculation_cached("B1", "A1+1", -1.0))
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    let _ = book.sheet_mut("Data").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    {
        let mut cell = book.sheet_mut("Data").unwrap().cell_mut(at("B1")).unwrap();
        *cell = cell
            .clone()
            .with_formula(Formula::from_file("A1*3", at("B1")));
        std::mem::forget(cell);
    }
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 6.0.into());
}

#[test]
fn workbook_recalculate_detects_raw_sheet_replacement_even_with_equal_revision() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.insert_cell(calculation_cached("B1", "A1+1", -1.0))
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    let cloned = book.sheet("Data").unwrap().clone();
    *book.sheet_mut("Data").unwrap() = cloned;
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        1,
        "active clone has a distinct calculation generation"
    );
    let mut replacement = Sheet::new("Data").unwrap();
    replacement.set_cell(at("A1"), 8.0).unwrap();
    replacement
        .insert_cell(calculation_cached("B1", "A1*2", -1.0))
        .unwrap();
    *book.sheet_mut("Data").unwrap() = replacement;
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        1,
        "inactive replacement needs initial graph intake"
    );
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 16.0.into());
}

#[test]
fn workbook_recalculate_failed_nested_batch_keeps_the_preexisting_pending_change() {
    use yggdryl::excel::Edit;
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.insert_cell(calculation_cached("B1", "A1+1", -1.0))
        .unwrap();
    book.recalculate().unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 4.0)
        .unwrap();
    let before = table_member_map(&book);
    let revision = book.sheet("Data").unwrap().revision();
    let dirty = book.is_dirty();
    let edit = |sheet: &str, value: &str| Edit::SetEntries {
        sheet: sheet.into(),
        entries: vec![(at("A1"), value.into())],
    };
    let error = book
        .apply(Edit::Batch(vec![
            edit("Data", "9"),
            Edit::Batch(vec![edit("Data", "12"), edit("Missing", "1")]),
        ]))
        .unwrap_err();
    assert!(matches!(error, Error::Absent { .. }), "{error}");
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
    assert_eq!(book.is_dirty(), dirty);
    assert_eq!(table_member_map(&book), before);
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 3.0.into());
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 5.0.into());
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn workbook_calculation_streams_sparse_ranges_and_preserves_argument_origin() {
    use yggdryl::excel::ExcelError;
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.set_cell(at("A2"), "3").unwrap();
    data.set_cell(at("A3"), true).unwrap();
    data.set_cell(at("A5"), 5.0).unwrap();
    book.add_sheet("Other")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    let output = book.add_sheet("Output").unwrap();
    for (cell, expression) in [
        ("B1", "SUM(Data!A:A)"),
        ("B2", "SUM(Data:Other!A1:A5)"),
        ("B3", r#"SUM(Data!A1:A5,TRUE,"3")"#),
        ("C2", "@Data!A1:A5+1"),
        ("D9", "@Data!A1:A5"),
        ("E1", "Missing!A1"),
        ("F1", "UnknownName"),
        ("G1", "Data:Other!A1"),
    ] {
        output
            .insert_cell(calculation_cached(cell, expression, -1.0))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (7, 1, 0)
    );
    for (cell, value) in [
        ("B1", 7.0),
        ("B2", 14.0),
        ("B3", 11.0),
        ("C2", 4.0),
        ("G1", -1.0),
    ] {
        assert_eq!(
            book.sheet("Output").unwrap().scalar(at(cell)),
            Scalar::from(value),
            "{cell}"
        );
    }
    for (cell, error) in [
        ("D9", ExcelError::Value),
        ("E1", ExcelError::Ref),
        ("F1", ExcelError::Name),
    ] {
        assert_eq!(
            book.sheet("Output")
                .unwrap()
                .cell(at(cell))
                .unwrap()
                .error(),
            Some(error)
        );
    }
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1048576"), 11.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        1,
        "whole-column dependency includes a formerly absent last row"
    );
    assert_eq!(book.sheet("Output").unwrap().scalar(at("B1")), 18.0.into());
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn workbook_calculation_never_reads_clean_held_predecessor_caches_as_values() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.insert_cell(calculation_cached("A1", "MYSTERY(1)", 100.0))
        .unwrap();
    data.insert_cell(calculation_cached("B1", "A1+1", 200.0))
        .unwrap();
    data.insert_cell(calculation_cached("C1", "SUM(A1:A3)", 300.0))
        .unwrap();
    data.insert_cell(calculation_cached("D1", "IF(FALSE,D1,1)", 400.0))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (1, 3, 0)
    );
    // The selected IF branch computes; its inactive self-reference is no
    // longer a held formula. The three reached unknown dependencies remain held.
    assert_eq!(book.sheet("Data").unwrap().scalar(at("D1")), 1.0.into());
    // Only the consumers are dirty: A1's retained status must block its cache.
    let data = book.sheet_mut("Data").unwrap();
    data.insert_cell(calculation_cached("B1", "A1+2", 201.0))
        .unwrap();
    data.insert_cell(calculation_cached("C1", "SUM(A1:A3,1)", 301.0))
        .unwrap();
    let report = book.recalculate().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 3, 0)
    );
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B1")), 201.0.into());
    assert_eq!(book.sheet("Data").unwrap().scalar(at("C1")), 301.0.into());
}

#[test]
fn workbook_calculation_reads_temporal_serials_and_reuses_typed_decimal_conversion() {
    use yggdryl::DataType;
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), Scalar::date32(19_723)).unwrap();
    let decimal = Scalar::decimal128(99_999_999_999_999_995, 16);
    let expected = DataType::Float64.cast_scalar(&decimal).unwrap();
    data.set_cell(at("A2"), decimal).unwrap();
    data.insert_cell(calculation_cached("B1", "A1+1", 99.0))
        .unwrap();
    data.insert_cell(calculation_cached("B2", "A2", -1.0))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (2, 0));
    let expected_date = DateSystem::Year1900
        .serial_of(&Scalar::date32(19_723))
        .unwrap()
        .unwrap()
        .0
        + 1.0;
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")),
        expected_date.into()
    );
    assert_eq!(book.sheet("Data").unwrap().scalar(at("B2")), expected);
}

#[test]
fn workbook_recalculate_failed_metadata_batch_does_not_invalidate_a_clean_graph() {
    use yggdryl::excel::Edit;
    let mut book = calculation_diamond();
    assert_eq!(book.recalculate().unwrap().evaluated, 5);
    let before = table_member_map(&book);
    let revisions: Vec<_> = book
        .sheet_names()
        .iter()
        .map(|name| book.sheet(name).unwrap().revision())
        .collect();
    let edit = Edit::Batch(vec![
        Edit::RenameSheet {
            name: "Data".into(),
            to: "Renamed".into(),
        },
        Edit::SetEntries {
            sheet: "Missing".into(),
            entries: vec![(at("A1"), "1".into())],
        },
    ]);
    assert!(book.apply(edit).is_err());
    assert_eq!(table_member_map(&book), before);
    assert_eq!(
        book.sheet_names()
            .iter()
            .map(|name| book.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn workbook_calculation_references_use_the_cells_existing_text_kind() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    let value = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    data.set_cell(at("A1"), value).unwrap();
    let expected = data.cell(at("A1")).unwrap().text().into_owned();
    assert!(data.cell(at("A1")).unwrap().kind().is_text());
    data.insert_cell(calculation_cached("B1", "A1", -1.0))
        .unwrap();
    data.insert_cell(calculation_cached("C1", "SUM(A1:A2,2)", -1.0))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (2, 0));
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")),
        Scalar::from(expected)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C1")),
        Scalar::from(2.0)
    );
}

#[test]
fn workbook_calculation_sum_keeps_held_dependencies_with_nonnumeric_old_caches() {
    use yggdryl::excel::Formula;
    for cached in [Scalar::from("old text"), Scalar::from(true), Scalar::Null] {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        data.insert_cell(
            Cell::from_scalar(at("A1"), cached, DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("MYSTERY(1)", at("A1"))),
        )
        .unwrap();
        data.insert_cell(calculation_cached("B1", "SUM(A1:A2,2)", 99.0))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (0, 2));
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")),
            Scalar::from(99.0)
        );
        // A clean held predecessor must also survive Numbers filtering when
        // just its consumer becomes dirty on the next pass.
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(calculation_cached("B1", "SUM(A1:A2,3)", 100.0))
            .unwrap();
        let report = book.recalculate().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (0, 2));
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")),
            Scalar::from(100.0)
        );
    }
}

#[test]
fn workbook_calculation_legacy_intersection_does_not_create_false_range_cycles() {
    use yggdryl::excel::Formula;
    for (intersection, text, dependent, expression, unrelated) in [
        ("B1", "@A:A", "A2", "B1+1", "A1048576"),
        ("B1", "A:A", "A2", "B1+1", "A1048576"),
        ("A2", "@1:1", "B1", "A2+1", "XFD1"),
        ("A2", "1:1", "B1", "A2+1", "XFD1"),
    ] {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        data.set_cell(at("A1"), 1.0).unwrap();
        data.insert_cell(
            Cell::from_scalar(at(intersection), Scalar::from(-1.0), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_entry(text, at(intersection)).unwrap()),
        )
        .unwrap();
        data.insert_cell(calculation_cached(dependent, expression, -1.0))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (2, 0, 0),
            "{text}"
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(intersection)),
            Scalar::from(1.0)
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(dependent)),
            Scalar::from(2.0)
        );
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at(unrelated), 100.0)
            .unwrap();
        assert_eq!(
            book.recalculate().unwrap().evaluated,
            0,
            "the intersection depends only on A1"
        );
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at("A1"), 3.0)
            .unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(dependent)),
            Scalar::from(4.0)
        );
    }
}

#[test]
fn workbook_calculation_intersects_legacy_scalar_operands_but_retains_aggregate_ranges() {
    use yggdryl::excel::ExcelError;
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    for (cell, value) in [
        ("A1", 3.0),
        ("A2", -4.0),
        ("A3", 5.25),
        ("A4", 6.0),
        ("A5", 7.0),
    ] {
        data.set_cell(at(cell), value).unwrap();
    }
    for (cell, expression) in [
        ("C1", "A1:A5+1"),
        ("C2", "SUM(A1:A5+1)"),
        ("C3", "ABS(A1:A5)"),
        ("C4", "ROUND(A1:A5,0)"),
        ("C5", "A1:A5"),
        ("D1", "SUM(A1+1)"),
        ("E1", "@A1:A5+1"),
        ("J1", "SUM(A1:A5)"),
        ("F9", "A1:A5"),
        ("G9", "ABS(A1:A5)"),
        ("H9", "SUM(A1:A5+1)"),
    ] {
        data.insert_cell(calculation_cached(cell, expression, 99.0))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (11, 0, 0)
    );
    for (cell, expected) in [
        ("C1", 4.0),
        ("C2", -3.0),
        ("C3", 5.25),
        ("C4", 6.0),
        ("C5", 7.0),
        ("D1", 4.0),
        ("E1", 4.0),
        ("J1", 17.25),
    ] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)),
            Scalar::from(expected),
            "{cell}"
        );
    }
    for cell in ["F9", "G9", "H9"] {
        assert_eq!(
            book.sheet("Data").unwrap().cell(at(cell)).unwrap().error(),
            Some(ExcelError::Value),
            "{cell}"
        );
    }
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A3"), 9.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        2,
        "only C3 and full-range J1 depend on A3"
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C3")),
        Scalar::from(9.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("J1")),
        Scalar::from(21.0)
    );
}

#[test]
fn workbook_calculation_temporal_reference_coverage_is_independent_of_pass_history() {
    use yggdryl::excel::{CellRange, StylePatch};
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", at("A1"), "=45292").unwrap();
    book.set_style(
        "Data",
        &[CellRange::new(at("A1"), at("A1"))],
        &StylePatch {
            number_format: Some("yyyy-mm-dd".into()),
            ..StylePatch::default()
        },
    )
    .unwrap();
    book.set_entry("Data", at("B1"), "=A1+1").unwrap();
    book.set_entry("Data", at("C1"), "=SUM(A1:A2)").unwrap();
    let full = book.calculate_all().unwrap();
    assert_eq!(
        (full.evaluated, full.uncomputed, full.circular_count),
        (3, 0, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::date32(19_723)
    );
    for (cell, serial) in [("B1", 45293.0), ("C1", 45292.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)),
            Scalar::from(serial)
        );
    }
    book.set_entry("Data", at("B1"), "=A1+2").unwrap();
    let later = book.recalculate().unwrap();
    assert_eq!(
        (later.evaluated, later.uncomputed, later.circular_count),
        (1, 0, 0)
    );
    for (cell, serial) in [("B1", 45294.0), ("C1", 45292.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)),
            Scalar::from(serial)
        );
    }
    let repeated = book.calculate_all().unwrap();
    assert_eq!(
        (
            repeated.evaluated,
            repeated.uncomputed,
            repeated.circular_count
        ),
        (3, 0, 0)
    );
    for (cell, serial) in [("B1", 45294.0), ("C1", 45292.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)),
            Scalar::from(serial)
        );
    }
}

#[test]
fn workbook_calculation_blank_reference_coverage_is_independent_of_pass_history() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet.set_cell(at("A1"), 1.0).unwrap();
    sheet.set_cell(at("A2"), -(1.0 - f64::EPSILON)).unwrap();
    book.set_entry("Data", at("A3"), "=Z99").unwrap();
    book.set_entry("Data", at("B1"), "=SUM(A1:A3)").unwrap();

    // A formula referring to an absent cell publishes numeric zero. That
    // trailing zero participates in SUM's final pair; an omitted blank does not.
    let first = book.calculate_all().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (2, 0, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A3")),
        Scalar::from(0.0)
    );
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(at("B1"))
            .as_f64()
            .unwrap()
            .to_bits(),
        f64::EPSILON.to_bits()
    );

    // Re-entering the consumer clears only its cache; A3 now comes from the
    // published cell instead of the current pass's result overlay.
    book.set_entry("Data", at("B1"), "=SUM(A1:A3)").unwrap();
    let later = book.recalculate().unwrap();
    assert_eq!(
        (later.evaluated, later.uncomputed, later.circular_count),
        (1, 0, 0)
    );
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(at("B1"))
            .as_f64()
            .unwrap()
            .to_bits(),
        f64::EPSILON.to_bits()
    );

    // A3's cache is already correct, so the full pass also covers publication
    // returning no replacement cell while its evaluator still returns Blank.
    let repeated = book.calculate_all().unwrap();
    assert_eq!(
        (
            repeated.evaluated,
            repeated.uncomputed,
            repeated.circular_count
        ),
        (2, 0, 0)
    );
    assert_eq!(
        book.sheet("Data")
            .unwrap()
            .scalar(at("B1"))
            .as_f64()
            .unwrap()
            .to_bits(),
        f64::EPSILON.to_bits()
    );
}

#[test]
fn workbook_calculation_nonblank_reference_coverage_is_independent_of_pass_history() {
    use yggdryl::excel::ExcelError;
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, formula) in [
        ("A1", "=1/3"),
        ("A2", "=\"\""),
        ("A3", "=TRUE"),
        ("A4", "=1/0"),
    ] {
        book.set_entry("Data", at(cell), formula).unwrap();
    }
    for (cell, formula) in [("B1", "=A1"), ("B2", "=A2"), ("B3", "=A3"), ("B4", "=A4")] {
        book.set_entry("Data", at(cell), formula).unwrap();
    }
    for pass in 0..3 {
        let report = if pass == 1 {
            for (cell, formula) in [("B1", "=A1"), ("B2", "=A2"), ("B3", "=A3"), ("B4", "=A4")] {
                book.set_entry("Data", at(cell), formula).unwrap();
            }
            book.recalculate().unwrap()
        } else {
            book.calculate_all().unwrap()
        };
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (if pass == 1 { 4 } else { 8 }, 0, 0)
        );
        let sheet = book.sheet("Data").unwrap();
        for cell in ["A1", "B1"] {
            assert_eq!(
                sheet.scalar(at(cell)).as_f64().unwrap().to_bits(),
                (1.0f64 / 3.0).to_bits()
            );
        }
        for cell in ["A2", "B2"] {
            assert_eq!(sheet.scalar(at(cell)).as_str(), Some(""));
        }
        for cell in ["A3", "B3"] {
            assert_eq!(sheet.scalar(at(cell)), Scalar::from(true));
        }
        for cell in ["A4", "B4"] {
            assert_eq!(
                sheet.cell(at(cell)).unwrap().error(),
                Some(ExcelError::Div0)
            );
        }
    }
}

/// Compare 94 implemented reference paths from the owned Excel 16.0
/// coercion run with their saved OOXML cache type and exact numeric bits.
#[test]
fn reference_coercion_native_cache_matches_94_numeric_paths() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/coercion_full_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 230, "native fixture scope changed");
    let mut origin_counts = [0usize; 4]; // reference, control, direct, array
    let mut selected_counts = [0usize; 2]; // number, error
    for case in cases {
        match case["parameters"]["origin"].as_str().unwrap() {
            "reference" => origin_counts[0] += 1,
            "control" => origin_counts[1] += 1,
            "direct" => origin_counts[2] += 1,
            "array" => origin_counts[3] += 1,
            other => panic!("new native origin {other}: {}", case["id"]),
        }
    }
    assert_eq!(origin_counts, [88, 46, 72, 24]);

    for year in ["1900", "1904"] {
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        let mut workbook = Workbook::new();
        workbook.set_date_system(system);
        workbook.add_sheet("Inputs").unwrap();
        workbook.add_sheet("Cases").unwrap();
        {
            let inputs = workbook.sheet_mut("Inputs").unwrap();
            // These are the authored source cells from p5-coercion/native-inputs.json.
            inputs.set_cell(at("A1"), true).unwrap();
            inputs.set_cell(at("A2"), false).unwrap();
            inputs.set_cell(at("A3"), "3").unwrap();
            inputs.set_cell(at("A4"), "abc").unwrap();
            // A5 is physically absent, unlike the empty-text formula in A7.
            inputs.set_cell(at("A6"), 0.0).unwrap();
            let address = at("A7");
            inputs
                .insert_cell(
                    Cell::from_scalar(address, Scalar::Null, system)
                        .unwrap()
                        .with_formula(Formula::from_file("\"\"", address)),
                )
                .unwrap();
            inputs.set_cell(at("A8"), "2024-01-31").unwrap();
            inputs.set_cell(at("A9"), "23:45:30").unwrap();
            inputs.set_cell(at("A10"), "12.5%").unwrap();
            inputs.set_cell(at("A11"), "1,234.50").unwrap();
        }
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| {
                if case["date_system"] != year {
                    return false;
                }
                match case["parameters"]["origin"].as_str().unwrap() {
                    "reference" => true,
                    "control" => matches!(
                        case["parameters"]["source_kind"].as_str().unwrap(),
                        "range" | "blank_arithmetic" | "empty_arithmetic"
                    ),
                    "direct" | "array" => false,
                    other => panic!("new native origin {other}: {}", case["id"]),
                }
            })
            .collect();
        assert_eq!(
            selected.len(),
            47,
            "{year}: reference/control selection changed"
        );
        assert_eq!(
            selected
                .iter()
                .filter(|case| case["parameters"]["origin"] == "reference")
                .count(),
            44
        );
        for case in &selected {
            let address = at(case["cell"].as_str().unwrap());
            assert_eq!(case["sheet"], "Cases");
            assert_eq!(
                case["cache_formula_text"], case["formula"],
                "{}",
                case["id"]
            );
            if case["parameters"]["origin"] == "control" {
                let formula = case["formula"].as_str().unwrap();
                match case["parameters"]["source_kind"].as_str().unwrap() {
                    "range" => assert_eq!(formula, "SUM(Inputs!A1:A11)"),
                    "blank_arithmetic" => assert_eq!(formula, "Inputs!A5+1"),
                    "empty_arithmetic" => assert_eq!(formula, "Inputs!A7+1"),
                    other => panic!("new reference control {other}"),
                }
            }
            workbook
                .sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::Null, system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }

        let report = workbook.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (48, 0, 0),
            "{year}"
        );
        for case in selected {
            let address = at(case["cell"].as_str().unwrap());
            let cell = workbook.sheet("Cases").unwrap().cell(address).unwrap();
            let label = case["id"].as_str().unwrap();
            let cache = case["cache_value_text"].as_str().unwrap();
            assert_eq!(case["cache_comparison_actual"], "equal", "{label}");
            match case["cache_type"].as_str().unwrap() {
                "n" => {
                    selected_counts[0] += 1;
                    let cached: f64 = cache.parse().unwrap();
                    let actual_bits = u64::from_str_radix(
                        case["actual_value2"]["ieee754_hex"].as_str().unwrap(),
                        16,
                    )
                    .unwrap();
                    assert_eq!(
                        cached.to_bits(),
                        actual_bits,
                        "native COM/cache disagreement: {label}"
                    );
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(actual_bits),
                        "{label}"
                    );
                    assert_eq!(cell.error(), None, "{label}");
                }
                "e" => {
                    selected_counts[1] += 1;
                    assert_eq!(
                        cell.error().map(|error| error.as_str()),
                        Some(cache),
                        "{label}"
                    );
                }
                other => panic!("unsupported selected native cache type {other}: {label}"),
            }
        }
    }
    assert_eq!(selected_counts, [76, 18]);
}

/// Excel's two blank-reference equality observations require a separate
/// comparison contract; the current evaluator intentionally holds them.
#[test]
fn blank_reference_equality_matches_two_native_boolean_caches() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/coercion_full_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    let selected: Vec<_> = cases
        .iter()
        .filter(|case| {
            case["parameters"]["origin"] == "control"
                && case["parameters"]["source_kind"] == "blank_comparison"
        })
        .collect();
    assert_eq!(
        selected.len(),
        2,
        "blank-reference equality native scope changed"
    );
    for case in selected {
        let year = case["date_system"].as_str().unwrap();
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        assert_eq!(case["formula"], "Inputs!A5=\"\"");
        assert_eq!(case["cache_formula_text"], case["formula"]);
        assert_eq!(case["cache_type"], "b");
        assert_eq!(case["cache_value_text"], "1");
        let mut workbook = Workbook::new();
        workbook.set_date_system(system);
        workbook.add_sheet("Inputs").unwrap(); // A5 physically absent.
        workbook.add_sheet("Cases").unwrap();
        let address = at(case["cell"].as_str().unwrap());
        workbook
            .sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(address, Scalar::Null, system)
                    .unwrap()
                    .with_formula(Formula::from_file(
                        case["formula"].as_str().unwrap(),
                        address,
                    )),
            )
            .unwrap();
        let report = workbook.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (1, 0, 0),
            "{year}"
        );
        let cell = workbook.sheet("Cases").unwrap().cell(address).unwrap();
        assert_eq!(cell.value().as_bool(), Some(true), "{}", case["id"]);
        assert_eq!(cell.error(), None, "{}", case["id"]);
    }
}

/// Blank-to-text equality is symmetric and distinguishes empty from nonempty
/// text; unlike generic Scalar null comparison, this is Excel formula policy.
#[test]
fn blank_reference_text_equality_is_symmetric_and_empty_only() {
    use yggdryl::excel::Formula;

    for system in [DateSystem::Year1900, DateSystem::Year1904] {
        let mut workbook = Workbook::new();
        workbook.set_date_system(system);
        workbook.add_sheet("Inputs").unwrap(); // A5 is physically absent.
        workbook.add_sheet("Cases").unwrap();
        for (address, formula) in [
            ("B2", "Inputs!A5=\"\""),
            ("B3", "\"\"=Inputs!A5"),
            ("B4", "Inputs!A5=\"abc\""),
            ("B5", "\"abc\"=Inputs!A5"),
        ] {
            let address = at(address);
            workbook
                .sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::Null, system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, address)),
                )
                .unwrap();
        }
        let report = workbook.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (4, 0, 0)
        );
        for (address, expected) in [("B2", true), ("B3", true), ("B4", false), ("B5", false)] {
            let cell = workbook.sheet("Cases").unwrap().cell(at(address)).unwrap();
            assert_eq!(
                cell.value().as_bool(),
                Some(expected),
                "{system:?} {address}"
            );
            assert_eq!(cell.error(), None, "{system:?} {address}");
        }
    }
}

#[test]
fn workbook_recalculate_structural_bands_replace_point_and_range_dependencies() {
    use yggdryl::excel::ExcelError;
    for rows in [true, false] {
        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        let inputs = if rows {
            ["A1", "A2", "A3"]
        } else {
            ["A1", "B1", "C1"]
        };
        for (cell, value) in inputs.into_iter().zip([2.0, 3.0, 5.0]) {
            source.set_cell(at(cell), value).unwrap();
        }
        let (local, area, point) = if rows {
            ("D1", "A1:A3", "A2")
        } else {
            ("D4", "A1:C1", "B1")
        };
        source
            .insert_cell(calculation_cached(local, &format!("SUM({area})"), -1.0))
            .unwrap();
        let report = book.add_sheet("Report").unwrap();
        report
            .insert_cell(calculation_cached("A1", &format!("Data!{point}*2"), -1.0))
            .unwrap();
        report
            .insert_cell(calculation_cached("A2", &format!("SUM(Data!{area})"), -1.0))
            .unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 3);
        assert_eq!(book.recalculate().unwrap().evaluated, 0);

        if rows {
            book.insert_rows("Data", 1, 1).unwrap();
        } else {
            book.insert_columns("Data", 1, 1).unwrap();
        }
        let inserted_local = if rows { "D1" } else { "E4" };
        assert_eq!(book.recalculate().unwrap().evaluated, 3);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(inserted_local)),
            Scalar::from(10.0)
        );
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A1")),
            Scalar::from(6.0)
        );
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A2")),
            Scalar::from(10.0)
        );
        assert_eq!(book.recalculate().unwrap().evaluated, 0);

        let inserted = if rows { "A2" } else { "B1" };
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at(inserted), 7.0)
            .unwrap();
        assert_eq!(
            book.recalculate().unwrap().evaluated,
            2,
            "the expanded ranges include the new blank band"
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(inserted_local)),
            Scalar::from(17.0)
        );
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A2")),
            Scalar::from(17.0)
        );

        if rows {
            book.remove_rows("Data", 1..3).unwrap();
        } else {
            book.remove_columns("Data", 1..3).unwrap();
        }
        let removed_local = if rows { "D1" } else { "C4" };
        let changed = book.recalculate().unwrap();
        assert_eq!(
            (
                changed.evaluated,
                changed.uncomputed,
                changed.circular_count
            ),
            (3, 0, 0)
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(removed_local)),
            Scalar::from(7.0)
        );
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A2")),
            Scalar::from(7.0)
        );
        assert_eq!(
            book.sheet("Report")
                .unwrap()
                .cell(at("A1"))
                .unwrap()
                .error(),
            Some(ExcelError::Ref)
        );

        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at(inserted), 11.0)
            .unwrap();
        assert_eq!(
            book.recalculate().unwrap().evaluated,
            2,
            "the deleted point edge must not survive"
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(removed_local)),
            Scalar::from(13.0)
        );
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A2")),
            Scalar::from(13.0)
        );
        let outside = if rows { "A3" } else { "C1" };
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at(outside), 19.0)
            .unwrap();
        assert_eq!(
            book.recalculate().unwrap().evaluated,
            0,
            "the contracted range must not retain its old tail"
        );
        assert_eq!(book.calculate_all().unwrap().evaluated, 3);
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("A2")),
            Scalar::from(13.0)
        );
    }
}

#[test]
fn workbook_recalculate_structural_rename_remove_and_undo_follow_sheet_identity() {
    use yggdryl::excel::{Edit, ExcelError};
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.set_cell(at("A2"), 3.0).unwrap();
    let report = book.add_sheet("Report").unwrap();
    report
        .insert_cell(calculation_cached("A1", "Data!A1+1", -1.0))
        .unwrap();
    report
        .insert_cell(calculation_cached("A2", "SUM(Data!A1:A2)", -1.0))
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(book.recalculate().unwrap().evaluated, 0);

    book.rename_sheet("Data", "Renamed Data").unwrap();
    book.sheet_mut("Renamed Data")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(8.0)
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A2")),
        Scalar::from(10.0)
    );

    book.add_sheet("Data")
        .unwrap()
        .set_cell(at("A1"), 100.0)
        .unwrap();
    book.recalculate().unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 101.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        0,
        "reusing the old spelling cannot steal dependencies"
    );

    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Renamed Data".into(),
        })
        .unwrap();
    // An Edit commits its recalculation with the authored mutation.
    let report = &removed.calc;
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (2, 0, 0)
    );
    for cell in ["A1", "A2"] {
        assert_eq!(
            book.sheet("Report")
                .unwrap()
                .cell(at(cell))
                .unwrap()
                .error(),
            Some(ExcelError::Ref)
        );
    }
    let restored_edit = book
        .apply(removed.inverse.expect("removal has an inverse"))
        .unwrap();
    assert_eq!(
        restored_edit.calc.evaluated, 0,
        "undo restores retained caches without evaluating"
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(8.0)
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A2")),
        Scalar::from(10.0)
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn workbook_recalculate_structural_tab_move_rebinds_three_dimensional_ranges() {
    let mut book = Workbook::new();
    for (name, value) in [("Jan", 2.0), ("Feb", 3.0), ("Mar", 5.0)] {
        book.add_sheet(name)
            .unwrap()
            .set_cell(at("A1"), value)
            .unwrap();
    }
    let report = book.add_sheet("Report").unwrap();
    report
        .insert_cell(calculation_cached("A1", "SUM(Jan:Mar!A1)", -1.0))
        .unwrap();
    report
        .insert_cell(calculation_cached("A2", "Feb!A1+1", -1.0))
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(10.0)
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 0);

    book.move_sheet("Feb", 3).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A2")),
        Scalar::from(4.0)
    );
    book.sheet_mut("Feb")
        .unwrap()
        .set_cell(at("A1"), 11.0)
        .unwrap();
    assert_eq!(
        book.recalculate().unwrap().evaluated,
        1,
        "the moved tab left the 3-D span"
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A2")),
        Scalar::from(12.0)
    );

    book.move_sheet("Feb", 1).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(18.0)
    );
    book.sheet_mut("Jan")
        .unwrap()
        .set_cell(at("A1"), 7.0)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(23.0)
    );
    assert_eq!(book.calculate_all().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(23.0)
    );
}

#[test]
fn workbook_recalculate_structural_name_rewrites_remove_and_restore_computed_status() {
    use yggdryl::excel::{Edit, ExcelError};
    let document = workbook(&["Data", "Report"], false).replace(
        "</workbook>",
        concat!(
            "<definedNames><definedName name=\"Rate\" localSheetId=\"0\">Data!$A$1</definedName>",
            "</definedNames></workbook>",
        ),
    );
    let mut book = Workbook::from_bytes(package(&[
        ("[Content_Types].xml", content_types(2, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", document),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(2, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet("")),
        ("xl/worksheets/sheet2.xml", worksheet("")),
    ]))
    .unwrap();
    let data = book.sheet_mut("Data").unwrap();
    data.set_cell(at("A1"), 2.0).unwrap();
    data.insert_cell(calculation_cached("B1", "Rate", 99.0))
        .unwrap();
    book.sheet_mut("Report")
        .unwrap()
        .insert_cell(calculation_cached("A1", "SUM(Data!A1:A2)", -1.0))
        .unwrap();
    let first = book.recalculate().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (2, 0, 0)
    );

    book.insert_rows("Data", 0, 1).unwrap();
    assert_eq!(book.defined_names().next().unwrap().text(), "Data!$A$2");
    let moved = book.recalculate().unwrap();
    assert_eq!(
        (moved.evaluated, moved.uncomputed, moved.circular_count),
        (2, 0, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B2")),
        Scalar::from(2.0)
    );
    book.rename_sheet("Data", "Source").unwrap();
    assert_eq!(book.defined_names().next().unwrap().text(), "Source!$A$2");
    let renamed = book.recalculate().unwrap();
    assert_eq!(
        (
            renamed.evaluated,
            renamed.uncomputed,
            renamed.circular_count
        ),
        (2, 0, 0)
    );

    let removed = book
        .apply(Edit::RemoveSheet {
            name: "Source".into(),
        })
        .unwrap();
    assert_eq!(book.defined_names().count(), 0);
    // An Edit commits its recalculation with the authored mutation.
    let after = &removed.calc;
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    assert_eq!(
        (after.evaluated, after.uncomputed, after.circular_count),
        (1, 0, 0)
    );
    assert_eq!(
        book.sheet("Report")
            .unwrap()
            .cell(at("A1"))
            .unwrap()
            .error(),
        Some(ExcelError::Ref)
    );
    let restored_edit = book
        .apply(removed.inverse.expect("removal has an inverse"))
        .unwrap();
    assert_eq!(
        restored_edit.calc.evaluated, 0,
        "undo restores retained caches without evaluating"
    );
    assert_eq!(book.defined_names().next().unwrap().text(), "Source!$A$2");
    let restored = book.recalculate().unwrap();
    assert_eq!(
        (
            restored.evaluated,
            restored.uncomputed,
            restored.circular_count
        ),
        (2, 0, 0)
    );
    assert_eq!(
        book.sheet("Report").unwrap().scalar(at("A1")),
        Scalar::from(2.0)
    );
    assert_eq!(
        book.sheet("Source").unwrap().scalar(at("B2")),
        Scalar::from(2.0)
    );
    let idle = book.recalculate().unwrap();
    assert_eq!(
        (idle.evaluated, idle.uncomputed, idle.circular_count),
        (0, 0, 0)
    );
}

#[test]
fn general_numeric_serial_becomes_exact_when_a_date_style_is_applied() {
    use yggdryl::excel::StylePatch;
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\"><v>60</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    workbook
        .set_style(
            "Sheet1",
            &["A1".parse().unwrap()],
            &StylePatch {
                number_format: Some("m/d/yyyy".into()),
                ..StylePatch::default()
            },
        )
        .unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    let a1 = part
        .split_once("<c r=\"A1\"")
        .unwrap()
        .1
        .split_once("</c>")
        .unwrap()
        .0;
    assert!(a1.contains("<v>60</v>"), "{part}");
}

#[test]
fn entering_and_pasting_general_numbers_into_date_styles_keep_the_original_serial() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\"><v>45292.000000001</v></c>\
         <c r=\"B1\" s=\"1\"><v>1</v></c>\
         <c r=\"C1\" s=\"2\"><v>1</v></c></row>",
        &[],
        &[],
        &[0, 14, 22],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    workbook.set_entry("Sheet1", at("B1"), "60").unwrap();
    workbook
        .paste(
            ("Sheet1", "A1".parse().unwrap()),
            ("Sheet1", at("C1")),
            Paste::Values,
            false,
        )
        .unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    let cell = |reference: &str| {
        part.split_once(&format!("<c r=\"{reference}\""))
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0
            .to_owned()
    };
    assert!(cell("B1").contains("<v>60</v>"), "{part}");
    let c1 = cell("C1");
    let actual: f64 = c1
        .split_once("<v>")
        .unwrap()
        .1
        .split_once("</v>")
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert_eq!(
        actual.to_bits(),
        "45292.000000001".parse::<f64>().unwrap().to_bits(),
        "{part}"
    );
}

#[test]
fn paste_all_and_values_carry_the_source_temporal_serial() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>60</v></c>\
         <c r=\"B1\" s=\"1\"><v>1</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    workbook
        .paste(
            ("Sheet1", "A1".parse().unwrap()),
            ("Sheet1", at("C1")),
            Paste::All,
            false,
        )
        .unwrap();
    workbook
        .paste(
            ("Sheet1", "A1".parse().unwrap()),
            ("Sheet1", at("B1")),
            Paste::Values,
            false,
        )
        .unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    for reference in ["A1", "B1", "C1"] {
        let tail = part.split_once(&format!("<c r=\"{reference}\"")).unwrap().1;
        let cell = tail.split_once("</c>").unwrap().0;
        assert!(cell.contains("<v>60</v>"), "{reference}: {part}");
    }
}

#[test]
fn calculation_preserves_exact_serials_in_same_pass_and_publishes_raw_only_changes() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\">\
         <c r=\"A1\" s=\"1\"><v>59</v></c>\
         <c r=\"B1\" s=\"1\"><v>60</v></c>\
         <c r=\"C1\" s=\"1\"><f>A1+1</f><v>59</v></c>\
         <c r=\"D1\"><f>C1-A1</f><v>0</v></c>\
         <c r=\"E1\" s=\"1\"><f>A1</f><v>60</v></c>\
         <c r=\"F1\"><f>B1-A1</f><v>0</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("A1")),
        workbook.sheet("Sheet1").unwrap().scalar(at("B1"))
    );
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (4, 0));
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("D1")),
        Scalar::from(1.0)
    );
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("F1")),
        Scalar::from(1.0)
    );
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    for (reference, expected) in [("C1", 60.0_f64), ("D1", 1.0), ("E1", 59.0), ("F1", 1.0)] {
        let cell = part
            .split_once(&format!("<c r=\"{reference}\""))
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0;
        let actual: f64 = cell
            .split_once("<v>")
            .unwrap()
            .1
            .split_once("</v>")
            .unwrap()
            .0
            .parse()
            .unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits(), "{reference}: {part}");
    }
    assert_eq!(workbook.recalculate().unwrap().evaluated, 0);
}

#[test]
fn calculation_reference_keeps_submillisecond_serial_bits() {
    let raw = "45292.000000001";
    let source = crate::excel_package::one_sheet(
        &format!(
            "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>{raw}</v></c>\
                  <c r=\"B1\" s=\"2\"><f>A1</f><v>45292</v></c></row>"
        ),
        &[],
        &[],
        &[0, 14, 22],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    let report = workbook.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (1, 0));
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    let b1 = part
        .split_once("<c r=\"B1\"")
        .unwrap()
        .1
        .split_once("</c>")
        .unwrap()
        .0;
    let written: f64 = b1
        .split_once("<v>")
        .unwrap()
        .1
        .split_once("</v>")
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert_eq!(
        written.to_bits(),
        raw.parse::<f64>().unwrap().to_bits(),
        "{part}"
    );
}

#[test]
fn calculation_reads_iso_date_cells_as_canonical_serials() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" t=\"d\"><v>2024-01-01</v></c>\
         <c r=\"B1\"><f>A1+1</f><v>0</v></c></row>",
        &[],
        &[],
        &[0],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    let expected = DateSystem::Year1900
        .serial_of(&workbook.sheet("Sheet1").unwrap().scalar(at("A1")))
        .unwrap()
        .unwrap()
        .0
        + 1.0;
    assert_eq!(workbook.calculate_all().unwrap().evaluated, 1);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("B1")),
        Scalar::from(expected)
    );
}

/// All 58 TRUE/FALSE/NOT observations are read; the nine text cases per epoch
/// remain named held caches until a formula-locale contract is available.
#[test]
fn logical_native_numeric_boolean_blank_and_error_cache_match_40_of_58_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/logical_native.json")).unwrap();
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 72);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    let mut matched = 0usize;
    let mut held_text = 0usize;
    let mut lazy_controls = 0usize;
    for year in ["1900", "1904"] {
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        assert_eq!(fixture["source_cells"].as_array().unwrap().len(), 11);
        for source in fixture["source_cells"].as_array().unwrap() {
            let address = at(source["cell"].as_str().unwrap());
            let kind = source["kind"].as_str().unwrap();
            let inputs = book.sheet_mut("Inputs").unwrap();
            match kind {
                "number_zero" | "number_positive" | "number_negative" => {
                    inputs
                        .set_cell(address, source["value"].as_f64().unwrap())
                        .unwrap();
                }
                "boolean_true" | "boolean_false" => {
                    inputs
                        .set_cell(address, source["value"].as_bool().unwrap())
                        .unwrap();
                }
                "physically_absent_blank" => {
                    assert_eq!(source["cell"], "A6");
                }
                "text_zero" | "text_one" | "text_invalid" => {
                    inputs
                        .set_cell(address, source["value"].as_str().unwrap())
                        .unwrap();
                }
                "formula_empty_text" | "formula_div_zero" => {
                    let formula = source["value"].as_str().unwrap().strip_prefix('=').unwrap();
                    inputs
                        .insert_cell(
                            Cell::from_scalar(address, Scalar::Null, system)
                                .unwrap()
                                .with_formula(Formula::from_file(formula, address)),
                        )
                        .unwrap();
                }
                other => panic!("unexpected native source kind {other}"),
            }
        }
        let selected: Vec<_> = all
            .iter()
            .filter(|case| {
                if case["date_system"] != year {
                    return false;
                }
                case["parameters"]["origin"] != "lazy_control"
            })
            .collect();
        assert_eq!(selected.len(), 29, "{year}: native NOT selection changed");
        lazy_controls += all
            .iter()
            .filter(|case| {
                case["date_system"] == year && case["parameters"]["origin"] == "lazy_control"
            })
            .count();
        for case in &selected {
            assert_eq!(case["sheet"], "Cases");
            assert_eq!(
                case["actual_value2"], case["after_save_value2"],
                "{}",
                case["id"]
            );
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (22, 9, 0),
            "{year}"
        );
        for case in selected {
            let label = case["id"].as_str().unwrap();
            let address = at(case["cell"].as_str().unwrap());
            let cell = book.sheet("Cases").unwrap().cell(address).unwrap();
            let kind = case["parameters"]["source_kind"].as_str().unwrap();
            let text = matches!(
                kind,
                "text_zero" | "text_one" | "text_invalid" | "text_empty" | "formula_empty_text"
            );
            let cache = case["cache_value_text"].as_str().unwrap();
            if text {
                held_text += 1;
                assert_eq!(
                    (case["cache_type"].as_str(), cache),
                    (Some("e"), "#VALUE!"),
                    "{label}"
                );
                assert_eq!(
                    cell.value().as_f64().map(f64::to_bits),
                    Some(77.0f64.to_bits()),
                    "held text cache changed: {label}"
                );
                assert_eq!(cell.error(), None, "{label}");
            } else {
                matched += 1;
                match case["cache_type"].as_str().unwrap() {
                    "b" => {
                        let expected = cache == "1";
                        assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
                        assert_eq!(case["actual_value2"]["value"], expected, "{label}");
                        assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
                        assert_eq!(cell.error(), None, "{label}");
                    }
                    "e" => {
                        assert_eq!(cache, "#DIV/0!", "{label}");
                        assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            Some(cache),
                            "{label}"
                        );
                    }
                    other => panic!("unsupported native NOT cache type {other}: {label}"),
                }
            }
        }
    }
    assert_eq!((matched, held_text, lazy_controls), (40, 18, 14));
}

/// The host was French (1036): VRAI/FAUX were accepted, but without a typed
/// formula locale every text argument is held with its old cache intact.
#[test]
fn logical_text_native_locale_boundary_is_explicitly_held_for_all_68_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/logical_text_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 68);
    assert_eq!(fixture["excel"]["ui_language"], 1036);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    let cases = fixture["cases"].as_array().unwrap();
    let mut native_boolean = 0usize;
    let mut native_error = 0usize;
    let mut held = 0usize;
    for year in ["1900", "1904"] {
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        assert_eq!(fixture["source_cells"].as_array().unwrap().len(), 17);
        for source in fixture["source_cells"].as_array().unwrap() {
            let address = at(source["cell"].as_str().unwrap());
            let inputs = book.sheet_mut("Inputs").unwrap();
            if let Some(text) = source["text"].as_str() {
                inputs.set_cell(address, text).unwrap();
            } else {
                let expression = source["formula"].as_str().unwrap();
                inputs
                    .insert_cell(
                        Cell::from_scalar(address, Scalar::Null, system)
                            .unwrap()
                            .with_formula(Formula::from_file(
                                expression.strip_prefix('=').unwrap(),
                                address,
                            )),
                    )
                    .unwrap();
            }
        }
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(selected.len(), 34, "{year}");
        for case in &selected {
            let address = at(case["cell"].as_str().unwrap());
            assert_eq!(case["sheet"], "Cases");
            assert_eq!(case["actual_value2"], case["after_save_value2"]);
            assert_eq!(case["actual_formula"], case["actual_formula2"]);
            match case["cache_type"].as_str().unwrap() {
                "b" => {
                    native_boolean += 1;
                    assert!(matches!(
                        case["parameters"]["kind"].as_str(),
                        Some("french_true" | "french_false")
                    ));
                    assert!(matches!(case["cache_value_text"].as_str(), Some("0" | "1")));
                }
                "e" => {
                    native_error += 1;
                    assert_eq!(case["cache_value_text"], "#VALUE!");
                }
                other => panic!("unexpected native cache {other}: {}", case["id"]),
            }
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (1, 34, 0),
            "{year}"
        );
        for case in selected {
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            assert_eq!(
                cell.value().as_f64().map(f64::to_bits),
                Some(77.0f64.to_bits()),
                "{}",
                case["id"]
            );
            assert_eq!(cell.error(), None, "{}", case["id"]);
            held += 1;
        }
    }
    assert_eq!((native_boolean, native_error, held), (8, 60, 68));
}

/// Direct numeric/Boolean properties and scalar references share one NOT path.
#[test]
fn logical_not_numeric_boolean_blank_reference_and_error_boundaries() {
    use yggdryl::excel::Formula;

    for system in [DateSystem::Year1900, DateSystem::Year1904] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        let inputs = book.add_sheet("Inputs").unwrap();
        inputs.set_cell(at("A1"), false).unwrap();
        inputs.set_cell(at("A2"), 3.0).unwrap();
        // A3 is physically absent. A4 is text even though its spelling is numeric.
        inputs.set_cell(at("A4"), "3").unwrap();
        for (address, formula, cache) in [
            ("A5", "1/0", Scalar::from(9.0)),
            ("A6", "UNSUPPORTED_LOGIC(1)", Scalar::from(false)),
        ] {
            let address = at(address);
            inputs
                .insert_cell(
                    Cell::from_scalar(address, cache, system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, address)),
                )
                .unwrap();
        }
        let cases = book.add_sheet("Cases").unwrap();
        for (address, formula, cache) in [
            ("B2", "NOT(Inputs!A1)", Scalar::from(false)),
            ("B3", "NOT(Inputs!A2)", Scalar::from(true)),
            ("B4", "NOT(Inputs!A3)", Scalar::from(false)),
            ("B5", "NOT(Inputs!A4)", Scalar::from(true)),
            ("B6", "NOT(Inputs!A5)", Scalar::from(true)),
            ("B7", "NOT(Inputs!A6)", Scalar::from(true)),
            ("B8", "NOT(-0)", Scalar::from(false)),
            ("B9", "NOT(2)", Scalar::from(true)),
            ("B10", "NOT(TRUE)", Scalar::from(true)),
            ("B11", "NOT(FALSE)", Scalar::from(false)),
        ] {
            let address = at(address);
            cases
                .insert_cell(
                    Cell::from_scalar(address, cache, system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, address)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        // A5 and eight supported NOT calls compute; A6, text B5, and B7 hold.
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (9, 3, 0)
        );
        let cases = book.sheet("Cases").unwrap();
        for (address, value) in [
            ("B2", true),
            ("B3", false),
            ("B4", true),
            ("B8", true),
            ("B9", false),
            ("B10", false),
            ("B11", true),
        ] {
            assert_eq!(
                cases.cell(at(address)).unwrap().value().as_bool(),
                Some(value),
                "{system:?} {address}"
            );
        }
        assert_eq!(
            book.sheet("Inputs")
                .unwrap()
                .cell(at("A5"))
                .unwrap()
                .error()
                .map(|e| e.as_str()),
            Some("#DIV/0!")
        );
        assert_eq!(
            cases.cell(at("B6")).unwrap().error().map(|e| e.as_str()),
            Some("#DIV/0!")
        );
        for address in ["B5", "B7"] {
            assert_eq!(
                cases.cell(at(address)).unwrap().value().as_bool(),
                Some(true),
                "held cache changed: {system:?} {address}"
            );
            assert_eq!(cases.cell(at(address)).unwrap().error(), None);
        }
        assert_eq!(
            book.sheet("Inputs")
                .unwrap()
                .cell(at("A6"))
                .unwrap()
                .value()
                .as_bool(),
            Some(false)
        );
    }
}

/// The existing registry owns arity; wrong shapes remain held with prior cache.
#[test]
fn logical_constant_and_not_wrong_arity_preserves_cached_values() {
    use yggdryl::excel::Formula;

    let mut book = Workbook::new();
    let sheet = book.add_sheet("Cases").unwrap();
    for (address, formula) in [
        ("B2", "TRUE(1)"),
        ("B3", "FALSE(1)"),
        ("B4", "NOT()"),
        ("B5", "NOT(1,2)"),
    ] {
        let address = at(address);
        sheet
            .insert_cell(
                Cell::from_scalar(address, Scalar::from(9.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, address)),
            )
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 4, 0)
    );
    for address in ["B2", "B3", "B4", "B5"] {
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at(address)),
            Scalar::from(9.0)
        );
    }
}

#[test]
fn temporal_serial_native_twelve_cache_observations() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/temporal_serial_native.json")).unwrap();
    assert_eq!(fixture["native_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    let input = std::env::var_os("YGGDRYL_EXCEL_TEMPORAL_IN");
    let output = std::env::var_os("YGGDRYL_EXCEL_TEMPORAL_OUT");
    assert!(
        output.is_none() || input.is_some(),
        "native export needs its native-oracle input directory"
    );
    for year in ["1900", "1904"] {
        let name = format!("functions-input-{year}.xlsx");
        let source = if let Some(root) = &input {
            std::fs::read(std::path::Path::new(root).join(&name)).unwrap()
        } else {
            let data = "<row r=\"2\"><c r=\"A2\" s=\"1\"><v>59</v></c><c r=\"B2\" s=\"1\"><f>A2</f><v>0</v></c></row>\
                <row r=\"3\"><c r=\"A3\" s=\"1\"><v>60</v></c><c r=\"B3\" s=\"1\"><f>A3</f><v>0</v></c></row>\
                <row r=\"4\"><c r=\"A4\" s=\"2\"><v>60.5</v></c><c r=\"B4\" s=\"2\"><f>A4</f><v>0</v></c></row>\
                <row r=\"5\"><c r=\"A5\" s=\"2\"><v>45292.000000001</v></c><c r=\"B5\" s=\"2\"><f>A5</f><v>0</v></c></row>\
                <row r=\"6\"><c r=\"B6\"><f>A3-A2</f><v>0</v></c></row>\
                <row r=\"7\"><c r=\"B7\"><f>A5-45292</f><v>0</v></c></row>";
            let types = crate::excel_package::content_types(1, false, true);
            let root = crate::excel_package::root_relationships();
            let book = crate::excel_package::workbook(&["Sheet1"], year == "1904");
            let rels = crate::excel_package::workbook_relationships(1, false, true);
            let sheet = crate::excel_package::worksheet(data);
            let styles = crate::excel_package::styles(&[], &[0, 14, 22]);
            crate::excel_package::package(&[
                ("[Content_Types].xml", types.as_str()),
                ("_rels/.rels", root.as_str()),
                ("xl/workbook.xml", book.as_str()),
                ("xl/_rels/workbook.xml.rels", rels.as_str()),
                ("xl/worksheets/sheet1.xml", sheet.as_str()),
                ("xl/styles.xml", styles.as_str()),
            ])
        };
        let mut workbook = Workbook::from_bytes(source).unwrap();
        let report = workbook.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (6, 0), "{year}");
        let sheet_name = workbook.sheet_names()[0];
        let worksheet = member(&workbook, "xl/worksheets/sheet1.xml");
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let address = case["cell"].as_str().unwrap();
            let expected =
                u64::from_str_radix(case["cached_ieee754_hex"].as_str().unwrap(), 16).unwrap();
            let cell = worksheet
                .split_once(&format!("<c r=\"{address}\""))
                .unwrap()
                .1
                .split_once("</c>")
                .unwrap()
                .0;
            let actual: f64 = cell
                .split_once("<v>")
                .unwrap()
                .1
                .split_once("</v>")
                .unwrap()
                .0
                .parse()
                .unwrap();
            assert_eq!(actual.to_bits(), expected, "{}: {}", case["id"], worksheet);
            assert!(
                workbook
                    .sheet(sheet_name)
                    .unwrap()
                    .cell(at(address))
                    .unwrap()
                    .formula()
                    .is_some()
            );
        }
        if let Some(root) = &output {
            let path = std::path::Path::new(root).join(name);
            assert!(
                !path.exists(),
                "native export must use fresh filenames: {}",
                path.display()
            );
            std::fs::write(path, workbook.into_bytes().unwrap()).unwrap();
        }
    }
}

/// Replay all 266 typed native information caches, including 32 ISREF
/// reference-identity results.
#[test]
fn information_native_typed_cases_match_266_including_isref_geometry() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/information_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 266);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["b"], 218);
    assert_eq!(fixture["cache_type_counts"]["n"], 28);
    assert_eq!(fixture["cache_type_counts"]["e"], 20);
    let mut matched = 0usize;
    for year in ["1900", "1904"] {
        let system = if year == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        assert_eq!(fixture["source_cells"].as_array().unwrap().len(), 12);
        for source in fixture["source_cells"].as_array().unwrap() {
            let address = at(source["cell"].as_str().unwrap());
            let inputs = book.sheet_mut("Inputs").unwrap();
            match source["kind"].as_str().unwrap() {
                "number_zero" | "number_even" | "number_negative_fraction" => {
                    inputs
                        .set_cell(address, source["value"].as_f64().unwrap())
                        .unwrap();
                }
                "boolean_true" | "boolean_false" => {
                    inputs
                        .set_cell(address, source["value"].as_bool().unwrap())
                        .unwrap();
                }
                "numeric_text" | "other_text" => {
                    inputs
                        .set_cell(address, source["value"].as_str().unwrap())
                        .unwrap();
                }
                "formula_empty_text" | "formula_div_zero" | "formula_na" => {
                    let expression = source["value"].as_str().unwrap().strip_prefix('=').unwrap();
                    inputs
                        .insert_cell(
                            Cell::from_scalar(address, Scalar::Null, system)
                                .unwrap()
                                .with_formula(Formula::from_file(expression, address)),
                        )
                        .unwrap();
                }
                "physically_absent_blank" => assert_eq!(source["cell"], "A9"),
                "date_2024_01_01" => {
                    assert_eq!(source["value"], "2024-01-01T00:00:00");
                    inputs.set_cell(address, Scalar::date32(19_723)).unwrap();
                }
                other => panic!("unexpected native input kind {other}"),
            }
        }
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(selected.len(), 133, "{year}");
        for case in &selected {
            assert_eq!(case["actual_cache_equal"], true, "{}", case["id"]);
            assert_eq!(case["after_save_cache_equal"], true, "{}", case["id"]);
            assert_eq!(
                case["actual_value2"], case["after_save_value2"],
                "{}",
                case["id"]
            );
            assert_eq!(
                case["actual_formula"], case["actual_formula2"],
                "{}",
                case["id"]
            );
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (136, 0, 0),
            "{year}"
        );
        for case in selected {
            let label = case["id"].as_str().unwrap();
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            matched += 1;
            match case["cache_type"].as_str().unwrap() {
                "b" => {
                    let expected = case["cache_value_text"] == "1";
                    assert!(
                        matches!(case["cache_value_text"].as_str(), Some("0" | "1")),
                        "{label}"
                    );
                    assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
                    assert_eq!(case["actual_value2"]["value"], expected, "{label}");
                    assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
                    assert_eq!(cell.error(), None, "{label}");
                }
                "n" => {
                    let expected: f64 = case["cache_value_text"].as_str().unwrap().parse().unwrap();
                    let native_bits = u64::from_str_radix(
                        case["actual_value2"]["ieee754_hex"].as_str().unwrap(),
                        16,
                    )
                    .unwrap();
                    assert_eq!(
                        expected.to_bits(),
                        native_bits,
                        "native transport/cache: {label}"
                    );
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(expected.to_bits()),
                        "{label}"
                    );
                    assert_eq!(cell.error(), None, "{label}");
                }
                "e" => {
                    let expected = case["cache_value_text"].as_str().unwrap();
                    assert_eq!(
                        cell.error().map(|error| error.as_str()),
                        Some(expected),
                        "{label}"
                    );
                }
                other => panic!("unsupported native cache {other}: {label}"),
            }
        }
    }
    assert_eq!(matched, 266);
}

fn calculation_named_book(names: &str, system: DateSystem) -> Workbook {
    let document = workbook(
        &["Data", "LocalA", "LocalB", "Cases"],
        system == DateSystem::Year1904,
    )
    .replace(
        "</workbook>",
        &format!("<definedNames>{names}</definedNames></workbook>"),
    );
    let mut parts = vec![
        (
            "[Content_Types].xml".to_owned(),
            content_types(4, false, false),
        ),
        ("_rels/.rels".to_owned(), root_relationships()),
        ("xl/workbook.xml".to_owned(), document),
        (
            "xl/_rels/workbook.xml.rels".to_owned(),
            workbook_relationships(4, false, false),
        ),
    ];
    for index in 1..=4 {
        parts.push((format!("xl/worksheets/sheet{index}.xml"), worksheet("")));
    }
    Workbook::from_bytes(package(
        &parts
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
    ))
    .unwrap()
}

#[test]
fn workbook_calculation_defined_names_match_all_220_native_contexts() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/defined_names_native.json")).unwrap();
    let sheets = ["Data", "LocalA", "LocalB", "Cases"];
    let mut names = String::new();
    for name in fixture["defined_names"].as_array().unwrap() {
        let scope = name["scope"]
            .as_str()
            .map(|scope| {
                format!(
                    " localSheetId=\"{}\"",
                    sheets.iter().position(|sheet| *sheet == scope).unwrap()
                )
            })
            .unwrap_or_default();
        names.push_str(&format!(
            "<definedName name=\"{}\"{scope}>{}</definedName>",
            name["name"].as_str().unwrap(),
            quick_xml::escape::escape(name["wire_formula"].as_str().unwrap())
        ));
    }
    let mut checked = 0;
    for context in ["a1", "d5"] {
        for year in ["1900", "1904"] {
            let system = if year == "1900" {
                DateSystem::Year1900
            } else {
                DateSystem::Year1904
            };
            let mut book = calculation_named_book(&names, system);
            for (sheet, cells) in fixture["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let scalar = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(sheet)
                        .unwrap()
                        .set_cell(at(address), scalar)
                        .unwrap();
                }
            }
            let cases: Vec<_> = fixture["cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|case| case["context"] == context && case["date_system"] == year)
                .collect();
            assert_eq!(cases.len(), 55);
            for case in &cases {
                let address = at(case["cell"].as_str().unwrap());
                let cell = Cell::from_scalar(address, Scalar::from(-777.0), system)
                    .unwrap()
                    .with_formula(Formula::from_file(
                        case["wire_formula"].as_str().unwrap(),
                        address,
                    ));
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(cell)
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (55, 0, 0),
                "{context}/{year}"
            );
            for case in cases {
                let id = case["id"].as_str().unwrap();
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                // Shared Power now computes the forty cancellation controls as well.
                let cache = &case["saved_cache"];
                let value = cache["value_text"].as_str().unwrap();
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(value.parse::<f64>().unwrap().to_bits()),
                        "{context}/{id}"
                    ),
                    "e" => assert_eq!(
                        cell.error().map(|error| error.as_str()),
                        Some(value),
                        "{context}/{id}"
                    ),
                    "b" => assert_eq!(cell.value(), &Scalar::from(value == "1"), "{context}/{id}"),
                    "str" => assert_eq!(cell.value(), &Scalar::from(value), "{context}/{id}"),
                    other => panic!("unhandled native type {other}"),
                }
                checked += 1;
            }
            let unchanged = book.recalculate().unwrap();
            assert_eq!(
                (
                    unchanged.evaluated,
                    unchanged.uncomputed,
                    unchanged.circular_count
                ),
                (0, 0, 0)
            );
        }
    }
    assert_eq!(checked, 220);
}

#[test]
fn workbook_calculation_defined_names_keep_range_use_and_incremental_edges() {
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"Column\">Data!$A:$A</definedName>",
            "<definedName name=\"Point\">Data!$A$1</definedName>",
            "<definedName name=\"Alias\">Point</definedName>",
        ),
        DateSystem::Year1900,
    );
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    for (sheet, address, formula) in [
        ("Data", "B1", "Column"),
        ("Data", "A2", "B1+1"),
        ("Data", "C1", "Alias+Alias"),
        ("Cases", "A1", "SUM(Column)"),
    ] {
        book.sheet_mut(sheet)
            .unwrap()
            .insert_cell(calculation_cached(address, formula, -1.0))
            .unwrap();
    }
    let first = book.calculate_all().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (4, 0, 0)
    );
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(3.0)
    );
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 3.0)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C1")),
        Scalar::from(6.0)
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn workbook_calculation_defined_names_distinguish_alias_loops_from_cell_cycles() {
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"First\">Second</definedName>",
            "<definedName name=\"Second\">First</definedName>",
            "<definedName name=\"LoopCell\">Data!$A$1</definedName>",
            "<definedName name=\"Constant\">7</definedName>",
        ),
        DateSystem::Year1900,
    );
    for (address, formula) in [
        ("A1", "LoopCell+1"),
        ("B1", "A1+1"),
        ("C1", "First"),
        ("D1", "Constant+Constant"),
    ] {
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(calculation_cached(address, formula, 99.0))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (1, 3, 1)
    );
    assert_eq!(report.circular, [("Data".into(), at("A1"))]);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C1")),
        Scalar::from(99.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("D1")),
        Scalar::from(14.0)
    );
}

#[test]
fn workbook_calculation_defined_names_refuse_nested_ambiguity_atomically() {
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"Alias\">Rate</definedName>",
            "<definedName name=\"Rate\">1</definedName>",
            "<definedName name=\"rAtE\">2</definedName>",
        ),
        DateSystem::Year1900,
    );
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(calculation_cached("A1", "1+1", 99.0))
        .unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(calculation_cached("B1", "Alias", 88.0))
        .unwrap();
    let before = table_member_map(&book);
    let revision = book.sheet("Data").unwrap().revision();
    let dirty = book.is_dirty();
    let error = book.calculate_all().unwrap_err();
    assert!(
        matches!(error, Error::InvalidRecord { ref path, ref reason } if path.contains("definedName[Rate]") && reason.contains("got 2")),
        "{error}"
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(99.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")),
        Scalar::from(88.0)
    );
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
    assert_eq!(book.is_dirty(), dirty);
    assert_eq!(table_member_map(&book), before);
}

/// Native classic error literals, including referenced source error caches,
/// produce ERROR.TYPE codes 1..7 in both Excel date systems.
#[test]
fn error_type_classic7_native_cache_matches_28_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/error_type_classic7_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 28);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["n"], 28);
    let cases = fixture["cases"].as_array().unwrap();
    let mut matched = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        for source in fixture["source_cells"].as_array().unwrap() {
            let address = at(source["cell"].as_str().unwrap());
            let expression = source["formula"]
                .as_str()
                .unwrap()
                .strip_prefix('=')
                .unwrap();
            book.sheet_mut("Inputs")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::Null, system)
                        .unwrap()
                        .with_formula(Formula::from_file(expression, address)),
                )
                .unwrap();
        }
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(selected.len(), 14);
        for case in &selected {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (21, 0, 0)
        );
        for case in selected {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["cache_type"], "n", "{label}");
            assert_eq!(case["actual_formula"], case["actual_formula2"], "{label}");
            let expected: f64 = case["cache_value_text"].as_str().unwrap().parse().unwrap();
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            assert_eq!(
                cell.value().as_f64().map(f64::to_bits),
                Some(expected.to_bits()),
                "{label}"
            );
            assert_eq!(cell.error(), None, "{label}");
            matched += 1;
        }
    }
    assert_eq!(matched, 28);
}

/// Excel evaluates typed #GETTING_DATA as 8, while its SaveAs rewrites the
/// source error cache to #N/A. This checks the calculation boundary only.
#[test]
fn error_type_getting_data_typed_source_matches_native_code_8() {
    use yggdryl::excel::{ExcelError, Formula};

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/error_type_getting_data_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 2);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let saved = &fixture["saved_workbooks"][year]["cells"][0];
        assert_eq!(saved["type"], "e");
        assert_eq!(saved["value_text"], "#N/A");
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        let source = at("A1");
        book.sheet_mut("Inputs")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(source, Scalar::Null, system)
                    .unwrap()
                    .with_error(ExcelError::GettingData),
            )
            .unwrap();
        let case = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["date_system"] == year)
            .unwrap();
        let address = at(case["cell"].as_str().unwrap());
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(address, Scalar::from(77.0), system)
                    .unwrap()
                    .with_formula(Formula::from_file(
                        case["wire_formula"].as_str().unwrap(),
                        address,
                    )),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (1, 0, 0)
        );
        assert_eq!(case["cache_type"], "n");
        assert_eq!(case["cache_value_text"], "8");
        assert_eq!(case["actual_value2"]["ieee754_hex"], "4020000000000000");
        let cell = book.sheet("Cases").unwrap().cell(address).unwrap();
        assert_eq!(
            cell.value().as_f64().map(f64::to_bits),
            Some(8.0f64.to_bits())
        );
        assert_eq!(cell.error(), None);
    }
}

/// Parity text intake differs from entry grammar: dates/times and percent
/// strings coerce, while currency text is a value error.
#[test]
fn parity_text_native_cache_matches_72_direct_and_reference_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/parity_text_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 72);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["b"], 56);
    assert_eq!(fixture["cache_type_counts"]["e"], 16);
    let cases = fixture["cases"].as_array().unwrap();
    let mut matched = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        for source in fixture["source_cells"].as_array().unwrap() {
            book.sheet_mut("Inputs")
                .unwrap()
                .set_cell(
                    at(source["cell"].as_str().unwrap()),
                    source["text"].as_str().unwrap(),
                )
                .unwrap();
        }
        let selected: Vec<_> = cases
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(selected.len(), 36);
        for case in &selected {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (36, 0, 0)
        );
        for case in selected {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["actual_formula"], case["actual_formula2"], "{label}");
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            match case["cache_type"].as_str().unwrap() {
                "b" => {
                    let expected = case["cache_value_text"] == "1";
                    assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
                    assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
                    assert_eq!(cell.error(), None, "{label}");
                }
                "e" => {
                    let expected = case["cache_value_text"].as_str().unwrap();
                    assert_eq!(case["actual_value2"]["variant"], "int", "{label}");
                    assert_eq!(
                        cell.error().map(|error| error.as_str()),
                        Some(expected),
                        "{label}"
                    );
                }
                other => panic!("unexpected native cache {other}: {label}"),
            }
            matched += 1;
        }
    }
    assert_eq!(matched, 72);
}

/// One error-valued information result cannot prevent neighboring typed
/// predicates and parity calls from updating their own cached cells.
#[test]
fn information_parity_and_error_predicates_update_independently() {
    use yggdryl::excel::Formula;

    for system in [DateSystem::Year1900, DateSystem::Year1904] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        let sheet = book.add_sheet("Cases").unwrap();
        let cases = [
            ("B2", "ISEVEN(0.1)", Some(true), None),
            ("B3", "ISODD(-0.1)", Some(false), None),
            ("B4", "ISEVEN(TRUE)", None, Some("#VALUE!")),
            ("B5", "ISODD(FALSE)", None, Some("#VALUE!")),
            ("B6", "ISEVEN(1/0)", None, Some("#DIV/0!")),
            ("B7", "ISODD(NA())", None, Some("#N/A")),
            ("B8", "ISERR(NA())", Some(false), None),
            ("B9", "ISERROR(1/0)", Some(true), None),
            ("B10", "ISNA(NA())", Some(true), None),
            ("B11", "ERROR.TYPE(1/0)", None, None),
        ];
        for (address, formula, _, _) in cases {
            let address = at(address);
            sheet
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, address)),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (10, 0, 0)
        );
        let sheet = book.sheet("Cases").unwrap();
        for (address, _, boolean, error) in cases {
            let cell = sheet.cell(at(address)).unwrap();
            if let Some(expected) = boolean {
                assert_eq!(
                    cell.value().as_bool(),
                    Some(expected),
                    "{system:?} {address}"
                );
                assert_eq!(cell.error(), None, "{system:?} {address}");
            } else if let Some(expected) = error {
                assert_eq!(
                    cell.error().map(|error| error.as_str()),
                    Some(expected),
                    "{system:?} {address}"
                );
            } else {
                assert_eq!(address, "B11");
                assert_eq!(
                    cell.value().as_f64().map(f64::to_bits),
                    Some(2.0f64.to_bits())
                );
            }
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn workbook_calculation_defined_name_dag_work_is_bounded_by_reached_syntax() {
    use yggdryl::internals::excel_workbook::calculation_work;
    for levels in [10, 16] {
        let mut names = String::from("<definedName name=\"AliasLevel0\">Data!$A$1</definedName>");
        for index in 1..=levels {
            names.push_str(&format!(
                "<definedName name=\"AliasLevel{index}\">AliasLevel{}+AliasLevel{}</definedName>",
                index - 1,
                index - 1
            ));
        }
        let mut book = calculation_named_book(&names, DateSystem::Year1900);
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at("A1"), 1.0)
            .unwrap();
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(calculation_cached(
                "A1",
                &format!("AliasLevel{levels}"),
                -1.0,
            ))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (1, 0));
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at("A1")),
            Scalar::from((1_u64 << levels) as f64)
        );
        let (dependency_nodes, evaluation_nodes, registrations, scheduled) =
            calculation_work(&book);
        let bound = 4 * (levels + 1);
        assert!(
            dependency_nodes <= bound && evaluation_nodes <= bound,
            "{levels} aliases: dependency nodes={dependency_nodes}, evaluator nodes={evaluation_nodes}, bound={bound}"
        );
        assert_eq!(
            registrations, 1,
            "the reached base reference is registered once"
        );
        assert_eq!(scheduled, 1);
    }
}

#[test]
fn workbook_calculation_defined_name_chains_use_an_explicit_stack() {
    // Cold package parsing and even one named expression exceed 64 KiB on
    // Windows debug builds. Both one and 4096 names fit the same 128 KiB;
    // this pins alias-depth independence without conflating that fixed floor.
    for depth in [1, 4096] {
        let mut names = String::from("<definedName name=\"AliasLevel0\">7</definedName>");
        for index in 1..depth {
            names.push_str(&format!(
                "<definedName name=\"AliasLevel{index}\">AliasLevel{}</definedName>",
                index - 1
            ));
        }
        let mut book = calculation_named_book(&names, DateSystem::Year1900);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(calculation_cached(
                "A1",
                &format!("AliasLevel{}", depth - 1),
                -1.0,
            ))
            .unwrap();
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                let report = book.calculate_all().unwrap();
                assert_eq!(
                    (report.evaluated, report.uncomputed, report.circular_count),
                    (1, 0, 0)
                );
                assert_eq!(
                    book.sheet("Cases").unwrap().scalar(at("A1")),
                    Scalar::from(7.0)
                );
            })
            .unwrap()
            .join()
            .expect("deep names do not consume the machine stack");
    }
}

#[cfg(feature = "internals")]
#[test]
fn workbook_calculation_defined_name_memo_keeps_reference_use_and_volatile_paths() {
    use yggdryl::internals::excel_workbook::calculation_work;
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"Column\">Data!$A:$A</definedName>",
            "<definedName name=\"RandomName\">INDIRECT(&quot;A1&quot;,FALSE)</definedName>",
            "<definedName name=\"RandomAlias\">RandomName</definedName>",
        ),
        DateSystem::Year1900,
    );
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A2"), 2.0)
        .unwrap();
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("B1", "Column+SUM(Column)", -1.0))
        .unwrap();
    let first = book.calculate_all().unwrap();
    assert_eq!((first.evaluated, first.uncomputed), (1, 0));
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("B1")),
        Scalar::from(4.0)
    );
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A2"), 3.0)
        .unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("B1")),
        Scalar::from(5.0)
    );
    book.sheet_mut("Cases").unwrap().remove_cell(at("B1"));
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A1", "RandomAlias+RandomAlias", 99.0))
        .unwrap();
    assert_eq!(book.calculate_all().unwrap().uncomputed, 1);
    // R1C1 INDIRECT stays held and volatile. Its now-implemented strict
    // call visits the text and FALSE arguments before the named refusal:
    // the prior five-node name/memo path therefore gains exactly two visits.
    // The held name is still memoized and is rescheduled on the next pass.
    assert_eq!(calculation_work(&book).1, 7);
    assert_eq!(book.recalculate().unwrap().uncomputed, 1);
    assert_eq!(calculation_work(&book).1, 7);
    assert_eq!(calculation_work(&book).3, 1);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(99.0),
        "an unsupported R1C1 INDIRECT preserves its prior cache"
    );
}

#[cfg(feature = "internals")]
#[test]
fn workbook_calculation_defined_name_held_volatile_dag_work_is_bounded() {
    use yggdryl::internals::excel_workbook::calculation_work;
    for levels in [10, 16] {
        let mut names = String::from(
            "<definedName name=\"AliasLevel0\">INDIRECT(&quot;A1&quot;,FALSE)</definedName>",
        );
        for index in 1..=levels {
            names.push_str(&format!(
                "<definedName name=\"AliasLevel{index}\">AliasLevel{}+AliasLevel{}</definedName>",
                index - 1,
                index - 1
            ));
        }
        let mut book = calculation_named_book(&names, DateSystem::Year1900);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(calculation_cached(
                "A1",
                &format!("AliasLevel{levels}"),
                99.0,
            ))
            .unwrap();
        let first = book.calculate_all().unwrap();
        assert_eq!((first.evaluated, first.uncomputed), (0, 1));
        let bound = 4 * (levels + 1);
        let (dependency_nodes, evaluation_nodes, registrations, scheduled) =
            calculation_work(&book);
        assert!(
            dependency_nodes <= bound && evaluation_nodes <= bound,
            "{levels} held aliases: dependency nodes={dependency_nodes}, evaluator nodes={evaluation_nodes}, bound={bound}"
        );
        assert_eq!((registrations, scheduled), (0, 1));
        let second = book.recalculate().unwrap();
        assert_eq!((second.evaluated, second.uncomputed), (0, 1));
        assert!(calculation_work(&book).1 <= bound);
        assert_eq!(calculation_work(&book).3, 1);
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at("A1")),
            Scalar::from(99.0)
        );
    }
}

#[test]
fn workbook_power_bounded_native_cases_keep_unproved_fractional_roots_held() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/power_native.json")).unwrap();
    assert_eq!(fixture["cleanup_completed"], true);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 15);
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Cases").unwrap();
    for case in cases {
        sheet
            .insert_cell(calculation_cached(
                case["cell"].as_str().unwrap(),
                case["formula"].as_str().unwrap(),
                -777.0,
            ))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (12, 3));
    let xml = member(&book, "xl/worksheets/sheet1.xml");
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let address = case["cell"].as_str().unwrap();
        let held = matches!(
            id,
            "power-negative-fraction" | "power-negative-third" | "power-function-negative-third"
        );
        let cell = xml
            .split_once(&format!("<c r=\"{address}\""))
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0;
        if held {
            assert_eq!(
                book.sheet("Cases").unwrap().scalar(at(address)),
                Scalar::from(-777.0),
                "{id}"
            );
            assert!(cell.contains("<v>-777</v>"), "{id}: {cell}");
        } else if case["cache_type"] == "e" {
            assert!(cell.contains("t=\"e\""), "{id}: {cell}");
            assert!(
                cell.contains(&format!("<v>{}</v>", case["cache_text"].as_str().unwrap())),
                "{id}: {cell}"
            );
        } else {
            let expected =
                u64::from_str_radix(case["value2_ieee754_hex"].as_str().unwrap(), 16).unwrap();
            let actual: f64 = cell
                .split_once("<v>")
                .unwrap()
                .1
                .split_once("</v>")
                .unwrap()
                .0
                .parse()
                .unwrap();
            assert_eq!(actual.to_bits(), expected, "{id}: {cell}");
        }
    }
}

/// A reference remains a reference across grouping, names later, and a
/// blank/error target; arithmetic consumes it into a scalar first.
#[test]
fn information_isref_native_reference_identity_matches_32_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/information_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    let selected: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["function"] == "ISREF")
        .collect();
    assert_eq!(selected.len(), 32);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        book.add_sheet("Inputs").unwrap();
        book.add_sheet("Cases").unwrap();
        let inputs = book.sheet_mut("Inputs").unwrap();
        inputs.set_cell(at("A1"), 0.0).unwrap();
        inputs.set_cell(at("A4"), true).unwrap();
        inputs.set_cell(at("A6"), "2").unwrap();
        for (address, formula) in [("A8", "\"\""), ("A10", "1/0")] {
            let at = at(address);
            inputs
                .insert_cell(
                    Cell::from_scalar(at, Scalar::Null, system)
                        .unwrap()
                        .with_formula(Formula::from_file(formula, at)),
                )
                .unwrap();
        }
        let cases: Vec<_> = selected
            .iter()
            .copied()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(cases.len(), 16);
        for case in &cases {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (18, 0, 0),
            "{year}"
        );
        for case in cases {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["cache_type"], "b", "{label}");
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["actual_formula"], case["actual_formula2"], "{label}");
            let expected = case["cache_value_text"] == "1";
            assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
            assert_eq!(case["actual_value2"]["value"], expected, "{label}");
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
            assert_eq!(cell.error(), None, "{label}");
        }
    }
}

fn isref_native_book(names: &serde_json::Value, system: DateSystem) -> Workbook {
    let mut definitions = String::new();
    for (name, formula) in names.as_object().unwrap() {
        definitions.push_str(&format!(
            "<definedName name=\"{name}\">{}</definedName>",
            quick_xml::escape::escape(formula.as_str().unwrap())
        ));
    }
    let document = workbook(&["Inputs", "Cases"], system == DateSystem::Year1904).replace(
        "</workbook>",
        &format!("<definedNames>{definitions}</definedNames></workbook>"),
    );
    let parts = [
        ("[Content_Types].xml", content_types(2, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", document),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(2, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet("")),
        ("xl/worksheets/sheet2.xml", worksheet("")),
    ];
    Workbook::from_bytes(package(
        &parts
            .iter()
            .map(|(name, content)| (*name, content.as_str()))
            .collect::<Vec<_>>(),
    ))
    .unwrap()
}

#[test]
fn information_isref_safe_geometry_native_28_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/isref_safe_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 28);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["b"], 28);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = isref_native_book(&fixture["defined_names"], system);
        book.sheet_mut("Inputs")
            .unwrap()
            .set_cell(at("A1"), 2.0)
            .unwrap();
        book.sheet_mut("Inputs")
            .unwrap()
            .set_cell(at("A2"), "text")
            .unwrap();
        book.sheet_mut("Inputs")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at("A4"), Scalar::Null, system)
                    .unwrap()
                    .with_formula(Formula::from_file("1/0", at("A4"))),
            )
            .unwrap();
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(cases.len(), 14);
        for case in &cases {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (15, 0, 0),
            "{year}"
        );
        for case in cases {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["actual_formula"], case["actual_formula2"], "{label}");
            assert_eq!(case["cache_type"], "b", "{label}");
            let expected = case["cache_value_text"] == "1";
            assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
            assert_eq!(case["actual_value2"]["value"], expected, "{label}");
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
            assert_eq!(cell.error(), None, "{label}");
            checked += 1;
        }
    }
    assert_eq!(checked, 28);
}

#[test]
fn information_isref_self_geometry_avoids_false_cycles_but_computed_cycle_holds() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/isref_self_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 16);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["b"], 12);
    assert_eq!(fixture["cache_type_counts"]["str"], 4);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = isref_native_book(&fixture["defined_names"], system);
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(cases.len(), 8);
        for case in &cases {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(""), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (6, 2, 2),
            "{year}"
        );
        for case in cases {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["actual_formula"], case["actual_formula2"], "{label}");
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            match case["cache_type"].as_str().unwrap() {
                "b" => {
                    let expected = case["cache_value_text"] == "1";
                    assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
                    assert_eq!(cell.error(), None, "{label}");
                }
                "str" => {
                    assert_eq!(case["cache_value_text"], serde_json::Value::Null, "{label}");
                    assert_eq!(case["actual_value2"]["variant"], "str", "{label}");
                    assert_eq!(case["actual_value2"]["value"], "", "{label}");
                    assert_eq!(
                        cell.value(),
                        &Scalar::from(""),
                        "held circular cache: {label}"
                    );
                }
                other => panic!("unexpected cache type {other}: {label}"),
            }
            checked += 1;
        }
        assert_eq!(book.recalculate().unwrap().evaluated, 0);
    }
    assert_eq!(checked, 16);
}

/// ISREF sees address identity; only a computed child has a value precedent.
#[test]
fn information_isref_geometry_has_no_value_edges_but_arithmetic_child_does() {
    let mut book = Workbook::new();
    book.add_sheet("Inputs")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    book.add_sheet("Cases").unwrap();
    for (address, formula) in [
        ("B2", "ISREF(Inputs!A1)"),
        ("B3", "ISREF(Inputs!A:A)"),
        ("B4", "ISREF(Inputs!A1+0)"),
        ("B5", "ISREF(B5)"),
    ] {
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(calculation_cached(address, formula, 77.0))
            .unwrap();
    }
    let first = book.calculate_all().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (4, 0, 0)
    );
    let cases = book.sheet("Cases").unwrap();
    for address in ["B2", "B3", "B5"] {
        assert_eq!(
            cases.cell(at(address)).unwrap().value().as_bool(),
            Some(true),
            "{address}"
        );
    }
    assert_eq!(cases.cell(at("B4")).unwrap().value().as_bool(), Some(false));
    book.sheet_mut("Inputs")
        .unwrap()
        .set_cell(at("A1"), 3.0)
        .unwrap();
    let changed = book.recalculate().unwrap();
    assert_eq!(
        (
            changed.evaluated,
            changed.uncomputed,
            changed.circular_count
        ),
        (1, 0, 0)
    );
    let cases = book.sheet("Cases").unwrap();
    assert_eq!(cases.cell(at("B2")).unwrap().value().as_bool(), Some(true));
    assert_eq!(cases.cell(at("B3")).unwrap().value().as_bool(), Some(true));
    assert_eq!(cases.cell(at("B4")).unwrap().value().as_bool(), Some(false));
    assert_eq!(cases.cell(at("B5")).unwrap().value().as_bool(), Some(true));
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

/// Native @/SINGLE cases remain held at this slice; proven numeric unary
/// results are false. The original native Boolean answer remains in fixture.
#[test]
fn information_isref_unary_boundary_computes_14_and_holds_22_cases() {
    use yggdryl::excel::Formula;

    let groups = [
        (
            include_str!("fixtures/isref_at_native.json"),
            28usize,
            14usize,
            14usize,
        ),
        (
            include_str!("fixtures/isref_single_native.json"),
            8usize,
            0usize,
            8usize,
        ),
    ];
    let (mut computed, mut held) = (0, 0);
    for (source, total, group_computed, group_held) in groups {
        let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(fixture["case_count"], total);
        assert_eq!(fixture["native_run_passed"], true);
        assert_eq!(fixture["cleanup_completed"], true);
        assert_eq!(fixture["native_comparisons_equal"], true);
        assert_eq!(fixture["cache_type_counts"]["b"], total);
        let (mut group_actual, mut group_pending) = (0, 0);
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            let mut book = isref_native_book(&fixture["defined_names"], system);
            for (row, value) in [
                ("A1", Scalar::from(2.0)),
                ("A2", Scalar::from("text")),
                ("A3", Scalar::from(0.0)),
                ("A4", Scalar::from(4.0)),
            ] {
                book.sheet_mut("Inputs")
                    .unwrap()
                    .set_cell(at(row), value)
                    .unwrap();
            }
            let cases: Vec<_> = fixture["cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|case| case["date_system"] == year)
                .collect();
            assert_eq!(cases.len(), total / 2);
            for case in &cases {
                let address = at(case["cell"].as_str().unwrap());
                book.sheet_mut("Cases")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(address, Scalar::from(77.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(
                                case["wire_formula"].as_str().unwrap(),
                                address,
                            )),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                ((group_computed / 2) as u64, (group_held / 2) as u64, 0),
                "{year}"
            );
            for case in cases {
                let label = case["id"].as_str().unwrap();
                assert_eq!(case["actual_cache_equal"], true, "{label}");
                assert_eq!(case["after_save_cache_equal"], true, "{label}");
                assert_eq!(case["cache_type"], "b", "{label}");
                assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                let kind = case["parameters"]["kind"].as_str().unwrap();
                if kind.starts_with("positive_")
                    || kind.starts_with("negative_")
                    || kind.starts_with("percent_")
                {
                    assert_eq!(case["cache_value_text"], "0", "{label}");
                    assert_eq!(cell.value().as_bool(), Some(false), "{label}");
                    group_actual += 1;
                } else {
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(77.0f64.to_bits()),
                        "held @/SINGLE spelling: {label}"
                    );
                    group_pending += 1;
                }
            }
        }
        assert_eq!((group_actual, group_pending), (group_computed, group_held));
        computed += group_actual;
        held += group_pending;
    }
    assert_eq!((computed, held), (14, 22));
}

/// A named @ result must carry its origin through grouping and alias memo
/// until ISREF declines it; ordinary numeric consumers still see its scalar.
#[test]
fn information_isref_named_intersection_alias_is_held_without_leaking_to_values() {
    use yggdryl::excel::Formula;

    let names = serde_json::json!({
        "WithAt": "@Inputs!$A$1",
        "Alias": "WithAt",
    });
    let mut book = isref_native_book(&names, DateSystem::Year1900);
    book.sheet_mut("Inputs")
        .unwrap()
        .set_cell(at("A1"), 2.0)
        .unwrap();
    let cases: [(&str, &str, Option<f64>); 6] = [
        ("B2", "ISREF(WithAt)", None),
        ("B3", "ISREF((WithAt))", None),
        ("B4", "ISREF(Alias)", None),
        ("B5", "WithAt", Some(2.0)),
        ("B6", "WithAt+1", Some(3.0)),
        ("B7", "SUM(WithAt,1)", Some(3.0)),
    ];
    for (address, formula, _) in cases {
        let address = at(address);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(address, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, address)),
            )
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (3, 3, 0)
    );
    for (address, _, expected) in cases {
        let cell = book.sheet("Cases").unwrap().cell(at(address)).unwrap();
        assert_eq!(cell.error(), None, "{address}");
        assert_eq!(
            cell.value().as_f64().map(f64::to_bits),
            Some(expected.unwrap_or(77.0).to_bits()),
            "{address}"
        );
    }
}

fn isref_three_sheet_book(system: DateSystem) -> Workbook {
    let parts = [
        ("[Content_Types].xml", content_types(4, false, false)),
        ("_rels/.rels", root_relationships()),
        (
            "xl/workbook.xml",
            workbook(
                &["Jan", "Feb", "Mar", "Cases"],
                system == DateSystem::Year1904,
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(4, false, false),
        ),
        ("xl/worksheets/sheet1.xml", worksheet("")),
        ("xl/worksheets/sheet2.xml", worksheet("")),
        ("xl/worksheets/sheet3.xml", worksheet("")),
        ("xl/worksheets/sheet4.xml", worksheet("")),
    ];
    Workbook::from_bytes(package(
        &parts
            .iter()
            .map(|(name, content)| (*name, content.as_str()))
            .collect::<Vec<_>>(),
    ))
    .unwrap()
}

/// Native Excel returns FALSE for a genuine 3-D span and TRUE when Jan:Jan
/// is rewritten to a single-sheet reference. Compare both dates and caches.
#[test]
fn information_isref_native_3d_sheet_span_8_cases() {
    use yggdryl::excel::Formula;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/isref_3d_native.json")).unwrap();
    assert_eq!(fixture["case_count"], 8);
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_comparisons_equal"], true);
    assert_eq!(fixture["cache_type_counts"]["b"], 8);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        let mut book = isref_three_sheet_book(system);
        for sheet in fixture["source_cells"].as_array().unwrap() {
            for (address, value) in sheet["cells"].as_object().unwrap() {
                book.sheet_mut(sheet["sheet"].as_str().unwrap())
                    .unwrap()
                    .set_cell(at(address), value.as_f64().unwrap())
                    .unwrap();
            }
        }
        let cases: Vec<_> = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["date_system"] == year)
            .collect();
        assert_eq!(cases.len(), 4);
        for case in &cases {
            let address = at(case["cell"].as_str().unwrap());
            book.sheet_mut("Cases")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(address, Scalar::from(77.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file(
                            case["wire_formula"].as_str().unwrap(),
                            address,
                        )),
                )
                .unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (4, 0, 0),
            "{year}"
        );
        for case in cases {
            let label = case["id"].as_str().unwrap();
            assert_eq!(case["actual_cache_equal"], true, "{label}");
            assert_eq!(case["after_save_cache_equal"], true, "{label}");
            assert_eq!(case["cache_type"], "b", "{label}");
            let expected = case["cache_value_text"] == "1";
            assert_eq!(case["actual_value2"]["variant"], "bool", "{label}");
            assert_eq!(case["actual_value2"]["value"], expected, "{label}");
            let cell = book
                .sheet("Cases")
                .unwrap()
                .cell(at(case["cell"].as_str().unwrap()))
                .unwrap();
            assert_eq!(cell.value().as_bool(), Some(expected), "{label}");
            assert_eq!(cell.error(), None, "{label}");
            checked += 1;
        }
    }
    assert_eq!(checked, 8);
}

/// ISREF declines direct and named @ results without inventing self-value
/// dependencies. This is a Rust scheduling assertion, not a native @self claim.
#[test]
fn information_isref_intersected_self_is_held_without_false_cycle() {
    use yggdryl::excel::Formula;

    let names = serde_json::json!({
        "WithAt": "@Cases!$B$3",
        "Alias": "WithAt",
    });
    let mut book = isref_native_book(&names, DateSystem::Year1900);
    for (cell, formula) in [("B2", "ISREF(@B2)"), ("B3", "ISREF(Alias)")] {
        let address = at(cell);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(address, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, address)),
            )
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 2, 0)
    );
    for address in ["B2", "B3"] {
        let cell = book.sheet("Cases").unwrap().cell(at(address)).unwrap();
        assert_eq!(
            cell.value().as_f64().map(f64::to_bits),
            Some(77.0f64.to_bits())
        );
        assert_eq!(cell.error(), None);
    }
}

#[test]
fn fixed_clock_recalculation_uses_one_serial_and_bounded_random() {
    use yggdryl::excel::Clock;

    let mut book = Workbook::new().with_clock(Clock::fixed(
        -2_203_977_600_000_000_000,
        Timezone::UTC,
        0x1234_5678,
    ));
    book.add_sheet("Cases").unwrap();
    for (address, text) in [
        ("A1", "=NOW()"),
        ("A2", "=TODAY()"),
        ("A3", "=RAND()"),
        ("A4", "=RANDBETWEEN(7,7)"),
        ("A5", "=RANDBETWEEN(8,7)"),
    ] {
        book.set_entry("Cases", at(address), text).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (5, 0));
    let sheet = book.sheet("Cases").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(59.0));
    assert_eq!(sheet.scalar(at("A2")), Scalar::from(59.0));
    let random = sheet.scalar(at("A3")).as_f64().unwrap();
    assert!((0.0..1.0).contains(&random));
    assert_eq!(sheet.scalar(at("A4")), Scalar::from(7.0));
    assert_eq!(
        sheet.cell(at("A5")).unwrap().error(),
        Some(yggdryl::excel::ExcelError::Num)
    );

    let mut other = Workbook::new().with_clock(Clock::fixed(
        -2_203_977_600_000_000_000,
        Timezone::UTC,
        0x1234_5678,
    ));
    other.add_sheet("Cases").unwrap();
    other.set_entry("Cases", at("A3"), "=RAND()").unwrap();
    other.calculate_all().unwrap();
    assert_eq!(
        other
            .sheet("Cases")
            .unwrap()
            .scalar(at("A3"))
            .as_f64()
            .unwrap()
            .to_bits(),
        random.to_bits()
    );
}

#[test]
fn fixed_clock_date_system_and_naive_wall_reading() {
    use yggdryl::excel::Clock;

    let mut book =
        Workbook::new().with_clock(Clock::fixed(-2_082_844_800_000_000_000, Timezone::NAIVE, 1));
    book.set_date_system(DateSystem::Year1904);
    book.add_sheet("Cases").unwrap();
    book.set_entry("Cases", at("A1"), "=TODAY()").unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(0.0)
    );
}

#[test]
fn workbook_calculation_volatile_name_consumes_successive_host_draws() {
    use yggdryl::excel::Clock;
    let names = "<definedName name=\"RandomName\">RAND()</definedName>";
    let clock = Clock::fixed(-2_203_977_600_000_000_000, Timezone::UTC, 0x1234_5678);
    let mut named = calculation_named_book(names, DateSystem::Year1900).with_clock(clock);
    named
        .sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A1", "RandomName+RandomName", -1.0))
        .unwrap();
    let mut direct = calculation_named_book(names, DateSystem::Year1900).with_clock(clock);
    direct
        .sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A1", "RAND()+RAND()", -1.0))
        .unwrap();
    let mut single = calculation_named_book(names, DateSystem::Year1900).with_clock(clock);
    single
        .sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A1", "RAND()", -1.0))
        .unwrap();
    for book in [&mut named, &mut direct, &mut single] {
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (1, 0));
    }
    let named = named
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    let direct = direct
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    let first = single
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    assert_eq!(named.to_bits(), direct.to_bits());
    assert_ne!(named.to_bits(), (first + first).to_bits());
}

#[test]
fn workbook_clock_failure_preserves_earlier_random_cache_and_pass_seed() {
    use yggdryl::excel::Clock;
    let clock = Clock::fixed(-2_300_000_000_000_000_000, Timezone::UTC, 73);
    let mut book = Workbook::new().with_clock(clock);
    book.add_sheet("Cases").unwrap();
    book.set_entry("Cases", at("A1"), "=RAND()").unwrap();
    book.set_entry("Cases", at("A2"), "=NOW()").unwrap();
    let error = book.calculate_all().unwrap_err();
    assert!(error.to_string().contains("$.clock"), "{error}");
    assert!(book.sheet("Cases").unwrap().scalar(at("A1")).is_null());
    book.sheet_mut("Cases").unwrap().remove_cell(at("A2"));
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    let retried = book
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();

    let mut fresh = Workbook::new().with_clock(clock);
    fresh.add_sheet("Cases").unwrap();
    fresh.set_entry("Cases", at("A1"), "=RAND()").unwrap();
    fresh.calculate_all().unwrap();
    let first = fresh
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    assert_eq!(retried.to_bits(), first.to_bits());
}

#[test]
fn workbook_randbetween_fractional_interval_remains_uncomputed() {
    use yggdryl::excel::Clock;
    let mut book = Workbook::new().with_clock(Clock::fixed(0, Timezone::UTC, 73));
    book.add_sheet("Cases").unwrap();
    book.set_entry("Cases", at("A1"), "=RANDBETWEEN(1.2,2.9)")
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (0, 1));
    assert!(book.sheet("Cases").unwrap().scalar(at("A1")).is_null());
}

#[test]
fn fixed_clock_reset_replays_random_and_incremental_volatile_dependents() {
    use yggdryl::excel::Clock;
    let initial = Clock::fixed(-2_203_977_600_000_000_000, Timezone::UTC, 83);
    let mut book = Workbook::new().with_clock(initial);
    book.add_sheet("Cases").unwrap();
    for (at_text, formula) in [("A1", "=RAND()"), ("B1", "=A1+1"), ("C1", "=TODAY()")] {
        book.set_entry("Cases", at(at_text), formula).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 3);
    let first = book
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("C1")),
        Scalar::from(59.0)
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    let second = book
        .sheet("Cases")
        .unwrap()
        .scalar(at("A1"))
        .as_f64()
        .unwrap();
    assert_ne!(first.to_bits(), second.to_bits());
    let dependent = book
        .sheet("Cases")
        .unwrap()
        .scalar(at("B1"))
        .as_f64()
        .unwrap();
    assert_eq!(dependent.to_bits(), (second + 1.0).to_bits());
    book.set_clock(initial);
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    assert_eq!(
        book.sheet("Cases")
            .unwrap()
            .scalar(at("A1"))
            .as_f64()
            .unwrap()
            .to_bits(),
        first.to_bits()
    );
    book.set_clock(Clock::fixed(-2_203_891_200_000_000_000, Timezone::UTC, 83));
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("C1")),
        Scalar::from(61.0)
    );
}

#[test]
fn fixed_clock_signed_nanos_midnight_and_dst_use_one_timezone_owner() {
    use yggdryl::excel::Clock;
    fn observe(nanos: i64, zone: Timezone) -> (f64, f64) {
        let mut book = Workbook::new().with_clock(Clock::fixed(nanos, zone, 1));
        book.add_sheet("Cases").unwrap();
        book.set_entry("Cases", at("A1"), "=NOW()").unwrap();
        book.set_entry("Cases", at("A2"), "=TODAY()").unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 2);
        let sheet = book.sheet("Cases").unwrap();
        (
            sheet.scalar(at("A1")).as_f64().unwrap(),
            sheet.scalar(at("A2")).as_f64().unwrap(),
        )
    }
    let (before_epoch, old_day) = observe(-1_000_000_000, Timezone::UTC);
    assert_eq!(old_day, 25_568.0);
    assert!(before_epoch > old_day && before_epoch < 25_569.0);
    let paris = Timezone::from_str("Europe/Paris").unwrap();
    let (paris_midnight, paris_day) = observe(1_711_841_400_000_000_000, paris);
    let (_, utc_day) = observe(1_711_841_400_000_000_000, Timezone::UTC);
    assert_eq!((paris_day, utc_day), (45_382.0, 45_381.0));
    assert!((paris_midnight - (paris_day + 0.5 / 24.0)).abs() < 1e-10);
    let (before_jump, before_day) = observe(1_711_845_000_000_000_000, paris);
    let (after_jump, after_day) = observe(1_711_848_600_000_000_000, paris);
    assert_eq!((before_day, after_day), (45_382.0, 45_382.0));
    assert!(((after_jump - before_jump) - 2.0 / 24.0).abs() < 1e-10);
}

#[test]
fn unknown_clock_zone_is_lazy_and_a_late_refusal_is_atomic() {
    use yggdryl::excel::Clock;
    let unknown = Timezone::from_str("Mars/Olympus_Mons").unwrap();
    assert!(!unknown.is_known());
    let mut book = Workbook::new().with_clock(Clock::fixed(0, unknown, 1));
    book.add_sheet("Cases").unwrap();
    book.set_entry("Cases", at("A1"), "=2+3").unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );
    book.set_entry("Cases", at("A2"), "=RAND()").unwrap();
    book.set_entry("Cases", at("A3"), "=NOW()").unwrap();
    let error = book.recalculate().unwrap_err();
    assert!(error.to_string().contains("$.clock.timezone"), "{error}");
    assert!(book.sheet("Cases").unwrap().scalar(at("A2")).is_null());
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(5.0)
    );
    book.set_clock(Clock::fixed(0, Timezone::UTC, 1));
    // The failed pass invalidates its prepared graph; retry rebuilds all
    // three formulas while preserving the previously published values.
    assert_eq!(book.recalculate().unwrap().evaluated, 3);
}

#[test]
fn lazy_selectors_match_all_native_omissions_reference_identities_and_inactive_cycles() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/lazy_selectors_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 240);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 120);
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
            (58, 2, 0)
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
                    "{}",
                    case["id"]
                ),
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
        let repeat = book.calculate_all().unwrap();
        assert_eq!(
            (repeat.evaluated, repeat.uncomputed, repeat.circular_count),
            (58, 2, 0)
        );
        assert_eq!(
            before,
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision())
        );
        let idle = book.recalculate().unwrap();
        assert_eq!((idle.evaluated, idle.uncomputed), (0, 2));
    }
    assert_eq!(covered, (116, 4));
}

#[test]
fn lazy_selectors_replace_active_dependencies_after_predicate_changes() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, text) in [
        ("A1", "TRUE"),
        ("B1", "=IF(A1,C1,D1)"),
        ("C1", "10"),
        ("D1", "20"),
    ] {
        book.set_entry("Data", at(cell), text).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(10.0)
    );
    book.set_entry("Data", at("D1"), "30").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    book.set_entry("Data", at("C1"), "11").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    book.set_entry("Data", at("A1"), "FALSE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(30.0)
    );
    book.set_entry("Data", at("C1"), "12").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    book.set_entry("Data", at("D1"), "31").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(31.0)
    );
}

#[test]
fn lazy_selectors_use_this_pass_predicate_and_report_only_active_cycles() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, text) in [
        ("A1", "TRUE"),
        ("A2", "=NOT(A1)"),
        ("B1", "=IF(A2,C1,7)"),
        ("C1", "=B1+1"),
    ] {
        book.set_entry("Data", at(cell), text).unwrap();
    }
    let first = book.calculate_all().unwrap();
    assert_eq!(
        (first.evaluated, first.uncomputed, first.circular_count),
        (3, 0, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(7.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C1")).as_f64(),
        Some(8.0)
    );
    book.set_entry("Data", at("A1"), "FALSE").unwrap();
    let cycle = book.recalculate().unwrap();
    assert_eq!(
        (cycle.evaluated, cycle.uncomputed, cycle.circular_count),
        (1, 2, 2)
    );
    assert_eq!(
        cycle.circular,
        vec![("Data".into(), at("B1")), ("Data".into(), at("C1"))]
    );
    // The cycle's old values are retained by the core contract, not inferred
    // from Excel's default zero cache for a newly introduced circular formula.
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(7.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("C1")).as_f64(),
        Some(8.0)
    );
    book.set_entry("Data", at("A1"), "TRUE").unwrap();
    let broken = book.recalculate().unwrap();
    assert_eq!(
        (broken.evaluated, broken.uncomputed, broken.circular_count),
        (3, 0, 0)
    );
}

#[test]
fn lazy_selectors_skip_inactive_held_formulas_and_preserve_active_held_status() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("A1"), Scalar::from(false), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("NO_SUCH_LAZY_FUNCTION(1)", at("A1"))),
        )
        .unwrap();
    book.set_entry("Data", at("A2"), "TRUE").unwrap();
    book.set_entry("Data", at("B1"), "=IF(A2,7,A1)").unwrap();
    let first = book.calculate_all().unwrap();
    assert_eq!((first.evaluated, first.uncomputed), (1, 1));
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(7.0)
    );
    book.set_entry("Data", at("A2"), "FALSE").unwrap();
    let held = book.recalculate().unwrap();
    assert_eq!(
        (held.evaluated, held.uncomputed, held.circular_count),
        (0, 2, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(7.0)
    );
}

#[test]
fn lazy_selectors_name_memos_keep_scalar_range_and_geometry_uses_after_suspension() {
    for formula in ["IF(Span,SUM(Span),0)", "SUM(Span)+IF(Span,0,0)"] {
        let mut book = calculation_named_book(
            "<definedName name=\"Span\">Cases!$Z$1:$Z$2</definedName>",
            DateSystem::Year1900,
        );
        book.set_entry("Cases", at("Z1"), "1").unwrap();
        book.set_entry("Cases", at("Y1"), "10").unwrap();
        book.set_entry("Cases", at("Z2"), "=Y1+1").unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(calculation_cached("A1", formula, -1.0))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (2, 0, 0),
            "{formula}"
        );
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("A1")),
            Scalar::from(12.0),
            "{formula}"
        );
        book.set_entry("Cases", at("Y1"), "20").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("A1")),
            Scalar::from(22.0),
            "{formula}"
        );
    }
    let mut book = calculation_named_book(
        "<definedName name=\"Span\">Cases!$Z$1:$Z$2</definedName>",
        DateSystem::Year1900,
    );
    for (sheet, cell, value) in [
        ("Cases", "Z1", "1"),
        ("Cases", "Y1", "10"),
        ("Cases", "Z2", "=Y1+1"),
        ("Data", "B1", "TRUE"),
        (
            "Data",
            "A1",
            "=IF(B1,IF(Span,SUM(Span),0),IF(Span,ISREF(Span),FALSE))",
        ),
    ] {
        book.set_entry(sheet, at(cell), value).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 2);
    book.set_entry("Data", at("B1"), "FALSE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(true)
    );
    book.set_entry("Cases", at("Y1"), "20").unwrap();
    // The selected result inspects reference identity: it must not retain
    // the previous SUM branch's dependency on the second source row.
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(true)
    );
    book.set_entry("Data", at("B1"), "TRUE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(22.0)
    );
}

#[test]
fn lazy_selectors_suspension_does_not_replay_random_draws_or_inactive_calls() {
    use yggdryl::Timezone;
    use yggdryl::excel::Clock;
    let make = |formula: &str, dependency: bool| {
        let mut book = Workbook::new().with_clock(Clock::fixed(0, Timezone::UTC, 73));
        book.add_sheet("Data").unwrap();
        book.set_entry("Data", at("A1"), formula).unwrap();
        if dependency {
            book.set_entry("Data", at("Z1"), "=1+1").unwrap();
        }
        book
    };
    for (formula, control, dependency) in [
        (
            "=IF(RAND()>=0,Z1+RAND(),1/0)",
            "=IF(RAND()>=0,2+RAND(),1/0)",
            true,
        ),
        ("=IF(TRUE,RAND(),RAND()+RAND())", "=RAND()", false),
    ] {
        let mut book = make(formula, dependency);
        let mut expected = make(control, false);
        for _ in 0..2 {
            assert_eq!(book.calculate_all().unwrap().uncomputed, 0);
            assert_eq!(expected.calculate_all().unwrap().uncomputed, 0);
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("A1")),
                expected.sheet("Data").unwrap().scalar(at("A1")),
                "{formula}"
            );
        }
    }
}

#[test]
fn lazy_selectors_late_selected_name_refusal_keeps_all_caches_atomic() {
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"Rate\">1</definedName>",
            "<definedName name=\"rAtE\">2</definedName>",
        ),
        DateSystem::Year1900,
    );
    for (cell, formula) in [("A1", "=1+1"), ("A2", "TRUE"), ("B1", "=IF(A2,7,Rate)")] {
        book.set_entry("Data", at(cell), formula).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 2);
    book.set_entry("Data", at("A1"), "=3+4").unwrap();
    book.set_entry("Data", at("A2"), "FALSE").unwrap();
    let before = table_member_map(&book);
    let revision = book.sheet("Data").unwrap().revision();
    let dirty = book.is_dirty();
    let error = book.recalculate().unwrap_err();
    assert!(
        matches!(error, Error::InvalidRecord { ref path, ref reason }
        if path.contains("definedName[Rate]") && reason.contains("got 2")),
        "{error}"
    );
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
    assert_eq!(book.is_dirty(), dirty);
    assert_eq!(table_member_map(&book), before);
    book.set_entry("Data", at("A2"), "TRUE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 2);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
}

#[test]
fn lazy_selectors_deep_cell_dependencies_use_suspended_heap_frames() {
    for depth in [1, 2048] {
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        for row in 1..=depth {
            book.set_entry(
                "Data",
                at(&format!("A{row}")),
                &format!("=IF(TRUE,A{},0)", row + 1),
            )
            .unwrap();
        }
        book.set_entry("Data", at(&format!("A{}", depth + 1)), "7")
            .unwrap();
        // The fixed workbook/name path already needs 128 KiB on this build.
        // Increasing dependency depth must not increase machine-stack use.
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                let report = book.calculate_all().unwrap();
                assert_eq!(
                    (report.evaluated, report.uncomputed, report.circular_count),
                    (depth, 0, 0)
                );
                assert_eq!(
                    book.sheet("Data").unwrap().scalar(at("A1")),
                    Scalar::from(7.0)
                );
            })
            .unwrap()
            .join()
            .expect("lazy dependencies use explicit suspension frames");
    }
}

#[test]
fn lazy_selectors_track_volatility_only_while_its_branch_is_reached() {
    use yggdryl::Timezone;
    use yggdryl::excel::Clock;
    let mut book = Workbook::new().with_clock(Clock::fixed(0, Timezone::UTC, 73));
    book.add_sheet("Data").unwrap();
    book.set_entry("Data", at("B1"), "FALSE").unwrap();
    book.set_entry("Data", at("A1"), "=IF(B1,RAND(),7)")
        .unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated, 1);
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    book.set_entry("Data", at("B1"), "TRUE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    book.set_entry("Data", at("B1"), "FALSE").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(7.0)
    );
    let revision = book.sheet("Data").unwrap().revision();
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
    assert_eq!(book.sheet("Data").unwrap().revision(), revision);
}

#[test]
fn multi_selectors_match_native_typed_selection_and_reference_identity() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/selector_order_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 260);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 130);
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
            (60, 5, 0)
        );
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let cell = book
                .sheet(case["sheet"].as_str().unwrap())
                .unwrap()
                .cell(case["cell"].as_str().unwrap().parse().unwrap())
                .unwrap();
            if case["rust_policy"] == "held_locale" {
                covered.1 += 1;
                assert_eq!(cell.value(), &Scalar::from(-777.0), "{}", case["id"]);
                continue;
            }
            covered.0 += 1;
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
                    "{}",
                    case["id"]
                ),
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
        let repeat = book.calculate_all().unwrap();
        assert_eq!(
            (repeat.evaluated, repeat.uncomputed, repeat.circular_count),
            (60, 5, 0)
        );
        assert_eq!(
            before,
            ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision())
        );
        let idle = book.recalculate().unwrap();
        assert_eq!((idle.evaluated, idle.uncomputed), (0, 5));
    }
    assert_eq!(covered, (120, 10));
}

#[test]
fn multi_selectors_register_all_tests_but_only_the_selected_result() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    for (formula, first, next) in [
        ("IFS(A1,C1,A2,D1,TRUE,E1)", "TRUE", "FALSE"),
        ("SWITCH(A1,1,C1,A2,D1,E1)", "1", "2"),
    ] {
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        for (cell, text) in [
            ("A1", first),
            ("A2", "2"),
            ("C1", "10"),
            ("D1", "20"),
            ("E1", "30"),
        ] {
            book.set_entry("Data", at(cell), text).unwrap();
        }
        book.set_entry("Data", at("B1"), &format!("={formula}"))
            .unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 1, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(10.0)
        );
        // Later tests/keys remain dependencies even after the first match.
        book.set_entry("Data", at("A2"), "3").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 1, "{formula}");
        book.set_entry("Data", at("A2"), "2").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 1, "{formula}");
        book.set_entry("Data", at("D1"), "21").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 0, "{formula}");
        book.set_entry("Data", at("A1"), next).unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 1, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(21.0)
        );
        book.set_entry("Data", at("C1"), "11").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 0, "{formula}");
    }
}

#[test]
fn multi_selectors_match_native_cycle_locations_and_retain_prior_caches() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/selector_order_native.json")).unwrap();
    let evidence = &fixture["scheduling_evidence"];
    assert_eq!(evidence["cleanup_completed"], true);
    assert_eq!(evidence["application_iteration"], false);
    assert_eq!(evidence["cached_values_are_answers"], false);
    let mut checked = (0, 0);
    for case in evidence["cases"].as_array().unwrap() {
        let at = CellRef::new(0, 0);
        let mut book = Workbook::new();
        book.add_sheet("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(77.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(
                        case["requested_formula"].as_str().unwrap(),
                        at,
                    )),
            )
            .unwrap();
        let result = book.calculate_all().unwrap();
        if case["circular_address"].is_null() {
            checked.0 += 1;
            assert_eq!(
                (result.evaluated, result.uncomputed, result.circular_count),
                (1, 0, 0),
                "{}",
                case["id"]
            );
            assert_eq!(
                book.sheet("Cases").unwrap().scalar(at).as_f64(),
                case["value2"]["value"].as_f64(),
                "{}",
                case["id"]
            );
        } else {
            checked.1 += 1;
            assert_eq!(case["circular_address"], "$A$1");
            assert_eq!(
                (result.evaluated, result.uncomputed, result.circular_count),
                (0, 1, 1),
                "{}",
                case["id"]
            );
            assert_eq!(
                result.circular,
                vec![("Cases".into(), at)],
                "{}",
                case["id"]
            );
            // The core contract retains 77; native's default circular zero is not an answer.
            assert_eq!(book.sheet("Cases").unwrap().scalar(at), Scalar::from(77.0));
        }
    }
    assert_eq!(checked, (8, 3));
}

#[test]
fn multi_selectors_use_this_pass_formula_keys_and_retire_only_result_edges() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    for formula in ["=IFS(Z1=1,C1,Z2=2,D1,TRUE,99)", "=SWITCH(Z1,1,C1,Z2,D1,99)"] {
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        for (cell, value) in [
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "10"),
            ("A4", "20"),
            ("Z1", "=A1+0"),
            ("Z2", "=A2+0"),
            ("C1", "=A3+1"),
            ("D1", "=A4+1"),
            ("B1", formula),
        ] {
            book.set_entry("Data", at(cell), value).unwrap();
        }
        assert_eq!(book.calculate_all().unwrap().evaluated, 5, "{formula}");
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(11.0)
        );
        book.set_entry("Data", at("A3"), "11").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(12.0)
        );
        book.set_entry("Data", at("A1"), "2").unwrap();
        book.set_entry("Data", at("A4"), "30").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 3);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(31.0)
        );
        book.set_entry("Data", at("A2"), "3").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(99.0)
        );
        book.set_entry("Data", at("A4"), "40").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 1);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(99.0)
        );
        book.set_entry("Data", at("A1"), "1").unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated, 2);
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
            Some(12.0)
        );
    }
}

#[test]
fn multi_selectors_resume_selected_formula_chains_without_replaying_inputs() {
    use yggdryl::{
        Timezone,
        excel::{CellRef, Clock, Workbook},
    };
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let make = |formula: &str, dependency: bool| {
        let mut book = Workbook::new().with_clock(Clock::fixed(0, Timezone::UTC, 73));
        book.add_sheet("Data").unwrap();
        book.set_entry("Data", at("A1"), formula).unwrap();
        if dependency {
            book.set_entry("Data", at("B1"), "=IF(TRUE,Z1,1/0)")
                .unwrap();
            book.set_entry("Data", at("Z1"), "=7").unwrap();
        }
        book
    };
    for (formula, control) in [
        (
            "=IFS(RAND()>=0,B1+RAND(),FALSE,1/0)",
            "=IFS(RAND()>=0,7+RAND(),FALSE,1/0)",
        ),
        (
            "=SWITCH(RAND()>=0,TRUE,B1+RAND(),FALSE,1/0,1/0)",
            "=SWITCH(RAND()>=0,TRUE,7+RAND(),FALSE,1/0,1/0)",
        ),
    ] {
        let mut book = make(formula, true);
        let mut expected = make(control, false);
        for _ in 0..2 {
            let result = book.calculate_all().unwrap();
            assert_eq!(
                (result.evaluated, result.uncomputed, result.circular_count),
                (3, 0, 0),
                "{formula}"
            );
            assert_eq!(expected.calculate_all().unwrap().evaluated, 1);
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("A1")),
                expected.sheet("Data").unwrap().scalar(at("A1")),
                "{formula}"
            );
        }
    }
}

#[test]
fn order_statistics_hold_omitted_parameters_without_panic() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    for formula in ["LARGE(Data!A1:A2,)", "RANK(2,Data!A1:A2,)"] {
        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        book.add_sheet("Cases").unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .set_cell("A1".parse().unwrap(), 2.0)
            .unwrap();
        let at: CellRef = "B2".parse().unwrap();
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed, report.circular_count),
            (0, 1, 0),
            "{formula}"
        );
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at),
            Scalar::from(-777.0),
            "{formula}"
        );
    }
}

#[test]
fn blank_and_conditional_extrema_match_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/blank_extrema_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 102);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(204));
    assert_eq!(evidence["cleanup_completed"], true);
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let text = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (51, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    "str" => assert_eq!(
                        cell.value().as_str(),
                        Some(""),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected blank/extrema cache type {other}"),
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 204);
}

#[test]
fn sumproduct_native_scalar_range_shape_and_precision() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sumproduct_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 76);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(152));
    assert_eq!(evidence["cleanup_completed"], true);
    let cases = evidence["cases"].as_array().unwrap();
    let mut computed = 0;
    let mut held = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let text = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (33, 5, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                if case["shape"]["operand_shape"] == "array_expression" {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                    held += 1;
                    continue;
                }
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    "str" => assert_eq!(
                        cell.value().as_str(),
                        Some(""),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected SUMPRODUCT native cache type {other}"),
                }
                computed += 1;
            }
        }
    }
    assert_eq!((computed, held), (132, 20));
}

#[test]
fn remaining_order_statistics_match_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/order_stat_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 198);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(396));
    assert_eq!(evidence["cleanup_completed"], true);
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            let source: CellRef = "E1".parse().unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(source, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file("1/0", source)),
                )
                .unwrap();
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["sheet"] == "Cases")
            {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (99, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["sheet"] == "Cases")
            {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native order-stat cache type {other}"),
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 392);
}

#[test]
fn remaining_order_statistics_fractional_rank_and_interpolation_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/order_stat_refinement_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 84);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(168));
    assert_eq!(evidence["cleanup_completed"], true);
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (42, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native refinement cache type {other}"),
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 168);
}

#[test]
fn statistics_rank_edges_match_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rank_edge_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 44);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(88));
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            let source: CellRef = "E1".parse().unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(source, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file("1/0", source)),
                )
                .unwrap();
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["sheet"] == "Cases")
            {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (22, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["sheet"] == "Cases")
            {
                checked += 1;
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native rank edge cache type {other}"),
                }
            }
        }
    }
    assert_eq!(checked, 84);
}

#[test]
fn statistics_median_and_mode_match_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/statistical_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 438);
    assert_eq!(evidence["provenance"].as_array().unwrap().len(), 8);
    let cases = evidence["cases"].as_array().unwrap();
    let ranked = |case: &&serde_json::Value| {
        matches!(
            case["shape"]["function"].as_str(),
            Some("MEDIAN" | "MODE" | "_xlfn.MODE.SNGL")
        )
    };
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            // The rank corpus references E1's authored #DIV/0! source.
            let source: CellRef = "E1".parse().unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(source, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file("1/0", source)),
                )
                .unwrap();
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .filter(&ranked)
            {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let text = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (37, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .filter(&ranked)
            {
                checked += 1;
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native rank cache type {other}"),
                }
            }
        }
    }
    assert_eq!(checked, 144);
}

#[test]
fn statistics_rank_refinement_matches_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rank_refinement_stable_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 28);
    assert_eq!(evidence["native_cache_equal"].as_u64(), Some(56));
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            book.sheet_mut("Values")
                .unwrap()
                .set_cell("A1".parse().unwrap(), Scalar::from("3"))
                .unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (14, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                checked += 1;
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native rank refinement cache type {other}"),
                }
            }
        }
    }
    assert_eq!(checked, 56);
}

#[test]
fn statistical_criteria_match_native_cache_bits() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/statistical_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 438);
    let cases = evidence["cases"].as_array().unwrap();
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            // E2/E3 are native-observed source caches: empty text and 6.
            // Their producing formulas are outside the criteria contract.
            book.sheet_mut("Values")
                .unwrap()
                .set_cell("E2".parse().unwrap(), Scalar::from(""))
                .unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .set_cell("E3".parse().unwrap(), Scalar::from(6.0))
                .unwrap();
            let source: CellRef = "E1".parse().unwrap();
            book.sheet_mut("Values")
                .unwrap()
                .insert_cell(
                    Cell::from_scalar(source, Scalar::from(-777.0), system)
                        .unwrap()
                        .with_formula(Formula::from_file("1/0", source)),
                )
                .unwrap();
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["shape"]["kind"] == "criteria")
            {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (58, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["shape"]["kind"] == "criteria")
            {
                checked += 1;
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native criteria cache type {other}"),
                }
            }
        }
    }
    assert_eq!(checked, 228);
}

#[test]
fn subtotal_native_codes_hidden_rows_and_nested_sources() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/statistical_native.json")).unwrap();
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 438);
    let cases = evidence["cases"].as_array().unwrap();
    let mut exact = 0;
    let mut held = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            book.sheet_mut("Values")
                .unwrap()
                .set_rows_hidden(2..3, true)
                .unwrap();
            for (address, formula) in [("E1", "1/0"), ("E3", "SUBTOTAL(9,B1:B3)")] {
                let at: CellRef = address.parse().unwrap();
                book.sheet_mut("Values")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula, at)),
                    )
                    .unwrap();
            }
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["shape"]["kind"] == "subtotal")
            {
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
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (25, 4, 0),
                "{year} saved={saved}"
            );
            for case in cases
                .iter()
                .filter(|case| case["date_system"] == year && case["shape"]["kind"] == "subtotal")
            {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
                let code = case["wire_formula"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("SUBTOTAL(")
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
                    .parse::<u16>()
                    .unwrap();
                // The four hidden-row variance means are33/7, outside the
                // exact dyadic-moment domain. Ordinary1..8 has exact moments.
                if matches!(code, 107 | 108 | 110 | 111) {
                    held += 1;
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                    continue;
                }
                exact += 1;
                let cache = &case["saved_cache"];
                match cache["type"].as_str().unwrap() {
                    "n" => assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
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
                        cell.error().map(|error| error.as_str()),
                        cache["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected native subtotal cache type {other}"),
                }
            }
        }
    }
    assert_eq!((exact, held), (92, 16));
}

#[test]
fn criteria_boundaries_match_native_source_types_and_saved_cache() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let boundary: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/criteria_boundary_native.json")).unwrap();
    let isolated: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/criteria_isolation_native.json")).unwrap();
    assert_eq!(boundary["cases"].as_array().unwrap().len(), 56);
    assert_eq!(isolated["cases"].as_array().unwrap().len(), 46);
    assert_eq!(boundary["cleanup_completed"], true);
    assert_eq!(isolated["cleanup_completed"], true);
    assert_eq!(boundary["native_cache_equal"], 112);
    assert_eq!(isolated["native_cache_equal"], 92);
    let mut exact = 0;
    let mut held = 0;
    for evidence in [&boundary, &isolated] {
        let cases = evidence["cases"].as_array().unwrap();
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            for saved in [false, true] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                for name in ["Values", "Cases"] {
                    book.add_sheet(name).unwrap();
                }
                for (name, cells) in evidence["source_cells"].as_object().unwrap() {
                    for (address, value) in cells.as_object().unwrap() {
                        let value = if let Some(value) = value.as_bool() {
                            Scalar::from(value)
                        } else if let Some(value) = value.as_str() {
                            Scalar::from(value)
                        } else {
                            Scalar::from(value.as_f64().unwrap())
                        };
                        book.sheet_mut(name)
                            .unwrap()
                            .set_cell(address.parse().unwrap(), value)
                            .unwrap();
                    }
                }
                for case in cases.iter().filter(|case| case["date_system"] == year) {
                    let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                    let cache = case
                        .get("saved_cache")
                        .or_else(|| case["after_save"].get("saved_cache"))
                        .unwrap();
                    let text = if saved {
                        &cache["formula_text"]
                    } else {
                        &case["wire_formula"]
                    };
                    book.sheet_mut(case["sheet"].as_str().unwrap())
                        .unwrap()
                        .insert_cell(
                            Cell::from_scalar(at, Scalar::from(-777.0), system)
                                .unwrap()
                                .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                        )
                        .unwrap();
                }
                book.calculate_all().unwrap();
                for case in cases.iter().filter(|case| case["date_system"] == year) {
                    let cell = book
                        .sheet(case["sheet"].as_str().unwrap())
                        .unwrap()
                        .cell(case["cell"].as_str().unwrap().parse().unwrap())
                        .unwrap();
                    // Native establishes the answer but not the text collation
                    // rule for relational >abc under another Excel locale.
                    if case["id"]
                        .as_str()
                        .unwrap()
                        .starts_with("countif-text-greater-")
                    {
                        held += 1;
                        assert_eq!(
                            cell.value().as_f64(),
                            Some(-777.0),
                            "{} saved={saved}",
                            case["id"]
                        );
                        continue;
                    }
                    let cache = case
                        .get("saved_cache")
                        .or_else(|| case["after_save"].get("saved_cache"))
                        .unwrap();
                    match cache["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(
                                cache["value_text"]
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
                            cell.error().map(|error| error.as_str()),
                            cache["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            Some(""),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native criteria cache type {other}"),
                    }
                    exact += 1;
                }
            }
        }
    }
    assert_eq!((exact, held), (200, 4));
}

#[test]
fn geometry_functions_match_all_successful_native_partitions_and_saved_formulas() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/geometry_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], 300);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 150);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for sheet in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(sheet).unwrap();
            }
            for (name, cells) in fixture["source_cells"].as_object().unwrap() {
                for (address, value) in cells.as_object().unwrap() {
                    let value = if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(address.parse().unwrap(), value)
                        .unwrap();
                }
            }
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let text = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(text.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (75, 0, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                checked += 1;
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(case["cell"].as_str().unwrap().parse().unwrap())
                    .unwrap();
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
                        cell.error().map(|error| error.as_str()),
                        expected["value_text"].as_str(),
                        "{} saved={saved}",
                        case["id"]
                    ),
                    other => panic!("unexpected geometry cache type {other}"),
                }
            }
            let before =
                ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision());
            assert_eq!(book.calculate_all().unwrap().evaluated, 75);
            assert_eq!(
                ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision()),
                before
            );
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 300);
}

#[test]
fn geometry_functions_have_no_cell_value_dependencies_or_false_self_cycles() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, formula) in [
        ("A1", "=ROW(A1)"),
        ("A2", "=COLUMN(A2)"),
        ("B1", "=ROWS(C:C)"),
        ("B2", "=COLUMNS(3:3)"),
        ("C1", "=1/0"),
    ] {
        book.set_entry("Data", at(cell), formula).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (5, 0, 0)
    );
    for (cell, value) in [
        ("A1", 1.0),
        ("A2", 1.0),
        ("B1", 1_048_576.0),
        ("B2", 16_384.0),
    ] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)).as_f64(),
            Some(value)
        );
    }
    book.set_entry("Data", at("C1"), "=2").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    book.set_entry("Data", at("C100"), "=3").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
}

#[test]
fn geometry_functions_rebuild_coordinates_and_dimensions_after_structural_edits() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.add_sheet("Cases").unwrap();
    for (cell, formula) in [
        ("A1", "=ROW(Data!B3:D5)"),
        ("A2", "=COLUMN(Data!B3:D5)"),
        ("A3", "=ROWS(Data!B3:D5)"),
        ("A4", "=COLUMNS(Data!B3:D5)"),
    ] {
        book.set_entry("Cases", at(cell), formula).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated, 4);
    book.insert_rows("Data", 3, 1).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    book.insert_columns("Data", 0, 1).unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 4);
    for (cell, expected) in [("A1", 3.0), ("A2", 3.0), ("A3", 4.0), ("A4", 3.0)] {
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at(cell)).as_f64(),
            Some(expected)
        );
    }
}

#[test]
fn geometry_functions_keep_named_intersections_and_arrays_independent_of_consumer_order() {
    let mut book = calculation_named_book(
        concat!(
            "<definedName name=\"Point\">_xlfn.SINGLE(Cases!$B$1:$B$3)</definedName>",
            "<definedName name=\"Matrix\">{1,2;3,4}</definedName>",
        ),
        DateSystem::Year1900,
    );
    for cell in ["B1", "B2", "B3"] {
        book.set_entry("Cases", at(cell), "TRUE").unwrap();
    }
    for (cell, text) in [
        ("A1", "=SUM(Point)+COLUMN(Point)"),
        ("A2", "=COLUMN(Point)+SUM(Point)"),
        ("C1", "=COLUMN(Point)"),
        ("D1", "=ROWS(Matrix)+COLUMNS(Matrix)"),
        ("D2", "=ROWS(IF(TRUE,Matrix,0))"),
    ] {
        book.set_entry("Data", at(cell), text).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (5, 0, 0)
    );
    for (cell, expected) in [
        ("A1", 2.0),
        ("A2", 2.0),
        ("C1", 2.0),
        ("D1", 4.0),
        ("D2", 2.0),
    ] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(cell)).as_f64(),
            Some(expected)
        );
    }
    book.set_entry("Cases", at("B1"), "7").unwrap();
    // Only the value consumer follows B1. The geometry-only C1 does not.
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")).as_f64(),
        Some(9.0)
    );
    book.set_entry("Cases", at("B2"), "11").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated, 1);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A2")).as_f64(),
        Some(13.0)
    );
}

const TEXT_COMPATIBILITY_NS: &str =
    "http://schemas.microsoft.com/office/spreadsheetml/2024/workbookCompatibilityVersion";
const TEXT_COMPATIBILITY_URI: &str = "{D14903EA-33C4-47F7-8F05-3474C54BE107}";

fn text_compatibility_package(root_attributes: &str, extension: &str) -> Vec<u8> {
    let document = workbook(&["Sheet1"], false)
        .replace("<workbook ", &format!("<workbook {root_attributes} "))
        .replace("</workbook>", &format!("{extension}</workbook>"));
    package(&[
        ("[Content_Types].xml", content_types(1, false, false)),
        ("_rels/.rels", root_relationships()),
        ("xl/workbook.xml", document),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, false),
        ),
        (
            "xl/worksheets/sheet1.xml",
            worksheet(
                "<row r=\"1\"><c r=\"A1\"><f>LEN(&quot;A\u{1f600}Z&quot;)</f><v>777</v></c><c r=\"B1\"><f>1+2</f><v>333</v></c></row>",
            ),
        ),
    ])
}

#[test]
fn text_compatibility_resolves_the_authored_version_and_keeps_wire_metadata() {
    for (attributes, extension, expected) in [
        (String::new(), String::new(), Some(4.0)),
        (
            String::new(),
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"1\"/></ext></extLst>"
            ),
            Some(4.0),
        ),
        (
            format!(
                "xmlns:alt=\"{}\"",
                TEXT_COMPATIBILITY_NS.replace("2024", "20&#x32;4")
            ),
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><alt:version warnBelowVersion=\"2\" setVersion=\"&#x32;\"/></ext></extLst>"
            ),
            Some(3.0),
        ),
        (
            String::new(),
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><version xmlns=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"2\"/></ext></extLst>"
            ),
            Some(3.0),
        ),
        (
            String::new(),
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"9\"/></ext></extLst>"
            ),
            None,
        ),
        (
            String::new(),
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\"/></ext></extLst>"
            ),
            None,
        ),
    ] {
        let mut book =
            Workbook::from_bytes(text_compatibility_package(&attributes, &extension)).unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed),
            if expected.is_some() { (2, 0) } else { (1, 1) }
        );
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(at("A1")).as_f64(),
            Some(expected.unwrap_or(777.0))
        );
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(at("B1")).as_f64(),
            Some(3.0)
        );
        let bytes = book.into_bytes().unwrap();
        assert!(member_text(&bytes, "xl/workbook.xml").contains(&extension));
        let mut reopened = Workbook::from_bytes(bytes).unwrap();
        assert_eq!(
            reopened.calculate_all().unwrap().uncomputed,
            u64::from(expected.is_none())
        );
        assert_eq!(
            reopened.sheet("Sheet1").unwrap().scalar(at("A1")).as_f64(),
            Some(expected.unwrap_or(777.0))
        );
    }
}

#[test]
fn text_compatibility_requires_the_exact_extension_ancestor_path() {
    for extension in [
        format!(
            "<x:extLst xmlns:x=\"urn:foreign\"><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"2\"/></ext></x:extLst>"
        ),
        format!(
            "<extLst><x:ext xmlns:x=\"urn:foreign\" uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"2\"/></x:ext></extLst>"
        ),
        format!(
            "<extLst><ext uri=\"urn:other\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"2\"/></ext></extLst>"
        ),
        format!(
            "<vendor xmlns=\"urn:vendor\"><extLst xmlns=\"{NS}\"><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"2\"/></ext></extLst></vendor>"
        ),
    ] {
        let mut book = Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 2);
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(at("A1")).as_f64(),
            Some(4.0)
        );
        assert!(member_text(&book.into_bytes().unwrap(), "xl/workbook.xml").contains(&extension));
    }
}

#[test]
fn text_compatibility_refuses_ambiguous_or_invalid_genuine_metadata() {
    for children in [
        "<v:version setVersion=\"1\"/><v:version setVersion=\"2\"/>",
        "<v:version setVersion=\"2\" setVersion=\"1\"/>",
        "<v:version setVersion=\"-1\"/>",
        "<v:version setVersion=\"4294967296\"/>",
        "<v:version setVersion=\"1\" warnBelowVersion=\"2\"/>",
        "<v:version setVersion=\"2\" warnBelowVersion=\"0\"/>",
        "<fake:version xmlns:fake=\"urn:foreign\" setVersion=\"2\"/>",
        "<vendor xmlns=\"urn:vendor\"><v:version setVersion=\"2\"/></vendor>",
    ] {
        let extension = format!(
            "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\" xmlns:v=\"{TEXT_COMPATIBILITY_NS}\">{children}</ext></extLst>"
        );
        let error = Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap_err();
        let Error::InvalidRecord { path, reason } = error else {
            panic!("{error}")
        };
        assert_eq!(path, "xl/workbook.xml#extLst/ext/version", "{children}");
        assert!(
            reason.contains("expected") && reason.contains("got"),
            "{reason}"
        );
    }
}

#[test]
fn text_compatibility_len_uses_all_native_version_vectors() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/unicode_native.json")).unwrap();
    let mut checked = 0;
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["wire_formula"].as_str().unwrap().starts_with("LEN("))
    {
        let version = case["version_marker"].as_str().unwrap();
        let extension = if version == "absent" {
            String::new()
        } else {
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"{version}\"/></ext></extLst>"
            )
        };
        let mut book = Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap();
        book.set_entry(
            "Sheet1",
            at("A1"),
            &format!("={}", case["wire_formula"].as_str().unwrap()),
        )
        .unwrap();
        assert_eq!(book.calculate_all().unwrap().evaluated, 2);
        let expected = case["cache_value_xml"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(at("A1")).as_f64(),
            Some(expected),
            "{} {}",
            version,
            case["id"]
        );
        checked += 1;
    }
    assert_eq!(checked, 12);
}

#[test]
fn text_compatibility_resolves_strict_prefixes_and_locates_custom_parts() {
    let strict = yggdryl::excel::STRICT_NAMESPACE;
    let relationships = yggdryl::excel::STRICT_RELATIONSHIPS_NAMESPACE;
    let custom = "custom/actual.xml";
    for (attributes, accepted) in [("setVersion=\"2\"", true), ("setVersion=\"NaN\"", false)] {
        let source = format!(
            "<s:workbook xmlns:s=\"{strict}\" xmlns:r=\"{relationships}\" xmlns:v=\"{TEXT_COMPATIBILITY_NS}\"><s:sheets><s:sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></s:sheets><s:extLst><s:ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version {attributes}/></s:ext></s:extLst></s:workbook>"
        );
        let data = worksheet(
            "<row r=\"1\"><c r=\"A1\"><f>LEN(&quot;A\u{1f600}Z&quot;)</f><v>777</v></c></row>",
        )
        .replace(NS, strict)
        .replace(R_NS, relationships);
        let bytes = package(&[
            (
                "[Content_Types].xml",
                content_types(1, false, false).replace("xl/workbook.xml", custom),
            ),
            (
                "_rels/.rels",
                root_relationships()
                    .replace("xl/workbook.xml", custom)
                    .replace(R_NS, relationships),
            ),
            (custom, source),
            (
                "custom/_rels/actual.xml.rels",
                workbook_relationships(1, false, false)
                    .replace("Target=\"worksheets/", "Target=\"../xl/worksheets/")
                    .replace(R_NS, relationships),
            ),
            ("xl/worksheets/sheet1.xml", data),
        ]);
        if accepted {
            let mut book = Workbook::from_bytes(bytes).unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 1);
            assert_eq!(
                book.sheet("Sheet1").unwrap().scalar(at("A1")).as_f64(),
                Some(3.0)
            );
        } else {
            let Error::InvalidRecord { path, reason } = Workbook::from_bytes(bytes).unwrap_err()
            else {
                panic!("expected located refusal")
            };
            assert_eq!(path, "custom/actual.xml#extLst/ext/version");
            assert!(reason.contains("expected") && reason.contains("NaN"));
        }
    }
}

#[test]
fn text_functions_portable_initial_cases_match_native_and_hold_locale_or_utf16_boundaries() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = [
        "LEN",
        "T",
        "CLEAN",
        "TRIM",
        "LEFT",
        "RIGHT",
        "MID",
        "EXACT",
        "REPT",
        "SUBSTITUTE",
    ];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 284);
    let held = |case: &serde_json::Value| {
        case["id"]
            .as_str()
            .unwrap()
            .starts_with("mid-surrogate-window-")
    };
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 2);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (141, 1, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    let expected = case
                        .get("en_us")
                        .map_or(&case["actual"]["value2"], |value| &value["raw"]);
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            expected["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            expected["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            expected["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 568); // 564 exact computations, 4 explicit UTF-16 held-cache controls.
}

#[test]
fn text_functions_native_compatibility_windows_never_publish_lone_surrogates() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/unicode_native.json")).unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
        ["MID(", "LEFT(", "RIGHT("]
            .iter()
            .any(|function| case["wire_formula"].as_str().unwrap().starts_with(function))
    }) {
        let version = case["version_marker"].as_str().unwrap();
        let extension = if version == "absent" {
            String::new()
        } else {
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"{version}\"/></ext></extLst>"
            )
        };
        let mut book = Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap();
        let host = at("A1");
        book.sheet_mut("Sheet1")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(host, Scalar::from("held original"), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(yggdryl::excel::Formula::from_file(
                        case["wire_formula"].as_str().unwrap(),
                        host,
                    )),
            )
            .unwrap();
        let unrepresentable = case["value2_before"]["text_transport"].is_object();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed),
            if unrepresentable { (1, 1) } else { (2, 0) }
        );
        let expected = if unrepresentable {
            "held original"
        } else {
            case["value2_before"]["value"].as_str().unwrap()
        };
        assert_eq!(
            book.sheet("Sheet1").unwrap().scalar(host).as_str(),
            Some(expected),
            "{} {}",
            version,
            case["id"]
        );
        checked += 1;
    }
    assert_eq!(checked, 48);
}

#[test]
fn text_functions_numeric_coercion_and_utf16_limits_match_all_native_observations() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_number_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 676);
    assert_eq!(fixture["native_cache_comparisons_equal"], 1352);
    let mut checked = 0;
    for saved in [false, true] {
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Cases").unwrap();
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .collect();
            // Source runs reuse B1 onward: separate fixture addresses preserve
            // every formula while avoiding collisions between those runs.
            for (row, case) in selected.iter().enumerate() {
                let at = CellRef::new(row as u32, 0);
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
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (337, 1, 0),
                "{year} saved={saved}"
            );
            for (row, case) in selected.iter().enumerate() {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(CellRef::new(row as u32, 0))
                    .unwrap();
                let id = case["id"].as_str().unwrap();
                if id.contains("trim-controls") {
                    assert_eq!(cell.value().as_f64(), Some(-777.0), "{id}");
                } else {
                    let expected = &case["saved_cache"];
                    match expected["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().unwrap().to_bits(),
                            expected["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap()
                                .to_bits(),
                            "{id} saved={saved}"
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            expected["value_text"].as_str(),
                            "{id} saved={saved}"
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            Some(expected["value_text"] == "1"),
                            "{id} saved={saved}"
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            expected["value_text"].as_str(),
                            "{id} saved={saved}"
                        ),
                        kind => panic!("unexpected native cache {kind}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 1352); //1348 computed and4 explicit CHAR(160) code-page held comparisons.
}

#[test]
fn text_join_preserves_sparse_blank_positions_and_explicit_empty_strings() {
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap();
    book.add_sheet("Cases").unwrap();
    for (address, value) in [("B2", "a"), ("D2", ""), ("C3", "b")] {
        book.sheet_mut("Values")
            .unwrap()
            .set_cell(at(address), Scalar::from(value))
            .unwrap();
    }
    for (address, formula, expected) in [
        ("A1", "TEXTJOIN(\"|\",FALSE,Values!B2:D3)", "a||||b|"),
        ("A2", "TEXTJOIN(\"|\",TRUE,Values!B2:D3)", "a|b"),
        ("A3", "CONCAT(Values!B2:D3)", "ab"),
        ("A4", "TEXTJOIN(\"|\",FALSE,Values!A1:A3)", "||"),
        ("A5", "TEXTJOIN(\"\",FALSE,Values!A:A)", ""),
    ] {
        book.set_entry("Cases", at(address), &format!("={formula}"))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(report.uncomputed, 0, "{formula}");
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at(address)).as_str(),
            Some(expected),
            "{formula}"
        );
    }
}

#[test]
fn text_join_checks_output_bound_before_expanding_absent_positions() {
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap();
    book.add_sheet("Cases").unwrap();
    for (address, formula) in [
        ("A1", "TEXTJOIN(\"|\",FALSE,Values!A:A)"),
        ("A2", "TEXTJOIN(\"|\",FALSE,Values!1:1048576)"),
        ("A3", "TEXTJOIN(\"\",FALSE,Values!1:1048576)"),
    ] {
        book.set_entry("Cases", at(address), &format!("={formula}"))
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (3, 0));
    for address in ["A1", "A2"] {
        assert_eq!(
            book.sheet("Cases")
                .unwrap()
                .cell(at(address))
                .unwrap()
                .error(),
            Some(yggdryl::excel::ExcelError::Value)
        );
    }
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A3")).as_str(),
        Some("")
    );
}

#[test]
fn text_join_never_skips_a_held_or_error_dependency_behind_its_old_empty_cache() {
    use yggdryl::excel::Formula;
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap();
    book.add_sheet("Cases").unwrap();
    book.sheet_mut("Values")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("A3"), Scalar::from(""), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("UNKNOWN(1)", at("A3"))),
        )
        .unwrap();
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("A1"), Scalar::from("prior"), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file(
                    "_xlfn.TEXTJOIN(\"|\",TRUE,Values!A1:A5)",
                    at("A1"),
                )),
        )
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (0, 2));
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")).as_str(),
        Some("prior")
    );
    book.set_entry("Values", at("A3"), "=1/0").unwrap();
    let report = book.recalculate().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (2, 0));
    assert_eq!(
        book.sheet("Cases").unwrap().cell(at("A1")).unwrap().error(),
        Some(yggdryl::excel::ExcelError::Div0)
    );
}

#[test]
fn text_join_functions_match_native_range_and_scalar_inputs() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = ["CONCAT", "CONCATENATE", "TEXTJOIN"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 38);
    let held = |_: &serde_json::Value| false;
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 0);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (19, 0, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    let expected = case
                        .get("en_us")
                        .map_or(&case["actual"]["value2"], |value| &value["raw"]);
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            expected["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            expected["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            expected["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 76); //76 exact computations; Boolean text uses the separate en-US observations.
}

#[test]
fn text_find_replace_match_native_positions_and_preserve_unrepresentable_cache() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = ["FIND", "REPLACE"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 48);
    let held =
        |case: &serde_json::Value| case["id"].as_str().unwrap().starts_with("replace-unicode-");
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 2);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (23, 1, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            case["actual"]["value2"]["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            case["actual"]["value2"]["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            case["actual"]["value2"]["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 96); //92 exact computations,4 unpaired-UTF16 held comparisons.
}

#[test]
fn text_find_empty_and_inside_pair_starts_match_native_search_boundaries() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_find_boundary_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    assert_eq!(fixture["native_cache_comparisons_equal"], 72);
    let mut checked = 0;
    for saved in [false, true] {
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Cases").unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
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
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (18, 0, 0)
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                {
                    let expected = &case["saved_cache"];
                    if expected["type"] == "e" {
                        assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            expected["value_text"].as_str(),
                            "{}",
                            case["id"]
                        );
                    } else {
                        assert_eq!(
                            cell.value().as_f64().unwrap().to_bits(),
                            expected["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap()
                                .to_bits(),
                            "{}",
                            case["id"]
                        );
                    }
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 72); //All72 authored/saved FIND and SEARCH comparisons are exact.
}

#[test]
fn text_find_replace_use_authored_compatibility_and_never_emit_half_surrogates() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/unicode_native.json")).unwrap();
    let mut checked = 0;
    for case in fixture["cases"].as_array().unwrap().iter().filter(|case| {
        case["wire_formula"].as_str().unwrap().starts_with("FIND(")
            || case["wire_formula"]
                .as_str()
                .unwrap()
                .starts_with("REPLACE(")
    }) {
        let version = case["version_marker"].as_str().unwrap();
        let extension = if version == "absent" {
            String::new()
        } else {
            format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"{version}\"/></ext></extLst>"
            )
        };
        let mut book = Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap();
        let host = at("A1");
        book.sheet_mut("Sheet1")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(host, Scalar::from("held original"), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(yggdryl::excel::Formula::from_file(
                        case["wire_formula"].as_str().unwrap(),
                        host,
                    )),
            )
            .unwrap();
        let held = case["value2_before"]["text_transport"].is_object();
        let report = book.calculate_all().unwrap();
        assert_eq!(
            (report.evaluated, report.uncomputed),
            if held { (1, 1) } else { (2, 0) }
        );
        let cell = book.sheet("Sheet1").unwrap().cell(host).unwrap();
        if held {
            assert_eq!(cell.value().as_str(), Some("held original"));
        } else if let Some(number) = case["value2_before"]["value"].as_f64() {
            assert_eq!(cell.value().as_f64(), Some(number));
        } else {
            assert_eq!(
                cell.value().as_str(),
                case["value2_before"]["value"].as_str()
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 18);
}

#[test]
fn text_casing_matches_portable_native_cases_and_preserves_locale_sensitive_caches() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = ["LOWER", "UPPER"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 62);
    let held = |case: &serde_json::Value| {
        ["accented"].iter().any(|label| {
            case["id"]
                .as_str()
                .unwrap()
                .starts_with(&format!("lower-{label}-"))
                || case["id"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("upper-{label}-"))
        })
    };
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 4);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (29, 2, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    let expected = case
                        .get("en_us")
                        .map_or(&case["actual"]["value2"], |value| &value["raw"]);
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            expected["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            expected["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            expected["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 124); //116 exact computations,8 explicit locale-sensitive held comparisons.
}

#[test]
fn text_search_matches_native_wildcards_coercion_and_error_precedence() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = ["SEARCH"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 32);
    let held = |_: &serde_json::Value| false;
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 0);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (16, 0, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            case["actual"]["value2"]["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            case["actual"]["value2"]["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            case["actual"]["value2"]["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 64); //All32 original native observations, authored and saved formulas.
}

#[test]
fn text_search_keeps_native_utf16_wildcards_and_versioned_positions_distinct() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_search_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 176);
    assert_eq!(fixture["native_cache_comparisons_equal"], 352);
    let held = |case: &serde_json::Value| {
        if case["corpus"] != "primary" {
            return false;
        }
        let ordinal = case["id"]
            .as_str()
            .unwrap()
            .split('-')
            .nth(1)
            .unwrap()
            .parse::<usize>()
            .unwrap();
        (23..=30).contains(&ordinal)
    };
    assert_eq!(cases.iter().filter(|case| held(case)).count(), 32);
    let mut checked = 0;
    for corpus in ["primary", "followup"] {
        for marker in ["1", "2"] {
            let extension = format!(
                "<extLst><ext uri=\"{TEXT_COMPATIBILITY_URI}\"><v:version xmlns:v=\"{TEXT_COMPATIBILITY_NS}\" setVersion=\"{marker}\"/></ext></extLst>"
            );
            for (year, system) in [
                ("1900", DateSystem::Year1900),
                ("1904", DateSystem::Year1904),
            ] {
                for saved in [false, true] {
                    let mut book =
                        Workbook::from_bytes(text_compatibility_package("", &extension)).unwrap();
                    book.set_date_system(system);
                    book.add_sheet("Cases").unwrap();
                    let selected: Vec<_> = cases
                        .iter()
                        .filter(|case| {
                            case["corpus"] == corpus
                                && case["version_marker"] == marker
                                && case["date_system"] == year
                        })
                        .collect();
                    assert_eq!(selected.len(), if corpus == "primary" { 32 } else { 12 });
                    for case in &selected {
                        let host = at(case["cell"].as_str().unwrap());
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
                                    .with_formula(Formula::from_file(
                                        formula.as_str().unwrap(),
                                        host,
                                    )),
                            )
                            .unwrap();
                    }
                    let report = book.calculate_all().unwrap();
                    assert_eq!(
                        (report.evaluated, report.uncomputed, report.circular_count),
                        if corpus == "primary" {
                            (26, 8, 0)
                        } else {
                            (14, 0, 0)
                        },
                        "corpus={corpus} marker={marker} year={year} saved={saved}"
                    );
                    for case in selected {
                        let cell = book
                            .sheet("Cases")
                            .unwrap()
                            .cell(at(case["cell"].as_str().unwrap()))
                            .unwrap();
                        if held(case) {
                            assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                        } else if case["saved_cache"]["type"] == "e" {
                            assert_eq!(
                                cell.error().map(|error| error.as_str()),
                                case["saved_cache"]["value_text"].as_str(),
                                "{} v{marker}",
                                case["id"]
                            );
                        } else {
                            assert_eq!(
                                cell.value().as_f64().map(f64::to_bits),
                                case["actual"]["value2"]["value"].as_f64().map(f64::to_bits),
                                "{} v{marker}",
                                case["id"]
                            );
                        }
                        checked += 1;
                    }
                    assert_eq!(book.recalculate().unwrap().evaluated, 0);
                }
            }
        }
    }
    assert_eq!(checked, 352); //288 computations,64 explicit non-ASCII case-policy holds.
}

#[test]
fn text_character_functions_match_native_ascii_and_word_boundaries() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 1128);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 564);
    let functions = ["CHAR", "CODE", "PROPER"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 100);
    let held = |case: &serde_json::Value| {
        let id = case["id"].as_str().unwrap();
        id.starts_with("char-code-128-")
            || id.starts_with("char-code-255-")
            || ["code", "proper"].iter().any(|function| {
                ["accented"]
                    .iter()
                    .any(|label| id.starts_with(&format!("{function}-{label}-")))
            })
    };
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 8);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (46, 4, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    let expected = case
                        .get("en_us")
                        .map_or(&case["actual"]["value2"], |value| &value["raw"]);
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            expected["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            expected["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            expected["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 200); //184 exact computations,16 explicit code-page/locale holds.
}

#[test]
fn text_character_followup_preserves_all_native_code_page_observations() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_remaining_native.json")).unwrap();
    assert_eq!(fixture["native_cache_comparisons_equal"], 484);
    let all = fixture["cases"].as_array().unwrap();
    assert_eq!(all.len(), 242);
    let functions = ["CHAR", "CODE", "PROPER"];
    let selected: Vec<_> = all
        .iter()
        .filter(|case| {
            case["shape"]["kind"] == "source"
                || functions.contains(&case["shape"]["function"].as_str().unwrap_or(""))
        })
        .collect();
    assert_eq!(selected.len(), 100);
    let held = |case: &serde_json::Value| {
        let id = case["id"].as_str().unwrap();
        if id.starts_with("char-") {
            let value = case["wire_formula"]
                .as_str()
                .unwrap()
                .strip_prefix("CHAR(")
                .unwrap()
                .strip_suffix(')')
                .unwrap()
                .parse::<f64>()
                .unwrap();
            return value.trunc() > 127.0 && value.trunc() <= 255.0;
        }
        if id.starts_with("code-") {
            return !id.starts_with("code-10-");
        }
        [8, 16, 17, 18]
            .iter()
            .any(|ordinal| id.starts_with(&format!("proper-{ordinal}-")))
    };
    assert_eq!(selected.iter().filter(|case| held(case)).count(), 52);
    let mut checked = 0;
    for saved in [false, true] {
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
                    let value = if let Some(value) = value.as_str() {
                        Scalar::from(value)
                    } else if let Some(value) = value.as_bool() {
                        Scalar::from(value)
                    } else {
                        Scalar::from(value.as_f64().unwrap())
                    };
                    book.sheet_mut(name)
                        .unwrap()
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let at = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(at, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), at)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed, report.circular_count),
                (24, 26, 0),
                "{year} saved={saved}"
            );
            for case in selected.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(
                        cell.value().as_f64(),
                        Some(-777.0),
                        "{} saved={saved}",
                        case["id"]
                    );
                } else {
                    match case["saved_cache"]["type"].as_str().unwrap() {
                        "n" => assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            case["actual"]["value2"]["value"].as_f64().map(f64::to_bits),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "b" => assert_eq!(
                            cell.value().as_bool(),
                            case["actual"]["value2"]["value"].as_bool(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "str" => assert_eq!(
                            cell.value().as_str(),
                            case["actual"]["value2"]["value"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        "e" => assert_eq!(
                            cell.error().map(|error| error.as_str()),
                            case["saved_cache"]["value_text"].as_str(),
                            "{} saved={saved}",
                            case["id"]
                        ),
                        other => panic!("unexpected native text cache {other}"),
                    }
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 200); //96 exact computations,104 explicit code-page/locale holds.
}

#[test]
fn text_value_matches_explicit_en_us_observations_without_rewriting_native_caches() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_en_us_native.json")).unwrap();
    assert_eq!(fixture["native_observations"], 162);
    assert_eq!(fixture["native_cache_equivalence_claim"], false);
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["wire_formula"].as_str().unwrap().starts_with("VALUE("))
        .collect();
    assert_eq!(cases.len(), 76);
    let held = |case: &serde_json::Value| {
        [14, 19, 20].iter().any(|number| {
            case["id"]
                .as_str()
                .unwrap()
                .starts_with(&format!("value-{number}-"))
        })
    };
    assert_eq!(cases.iter().filter(|case| held(case)).count(), 6);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Values").unwrap();
            book.add_sheet("Cases").unwrap();
            book.set_entry("Values", at("D2"), "=1/0").unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let host = at(case["cell"].as_str().unwrap());
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
                (report.evaluated, report.uncomputed),
                (36, 3),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                } else if case["en_us"]["is_error"] == true {
                    assert_eq!(
                        cell.error().map(|v| v.as_str()),
                        case["en_us"]["error_type"]["literal"].as_str(),
                        "{}",
                        case["id"]
                    );
                } else {
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        case["en_us"]["raw"]["value"].as_f64().map(f64::to_bits),
                        "{}",
                        case["id"]
                    );
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 152); //140 exact LCID comparisons;12 unsupported temporal spelling holds.
}

#[test]
fn text_format_matches_explicit_en_us_observations_without_rewriting_native_caches() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text_en_us_native.json")).unwrap();
    assert_eq!(fixture["native_observations"], 162);
    assert_eq!(fixture["native_cache_equivalence_claim"], false);
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["wire_formula"].as_str().unwrap().starts_with("TEXT("))
        .collect();
    assert_eq!(cases.len(), 86);
    let held = |_: &serde_json::Value| false;
    assert_eq!(cases.iter().filter(|case| held(case)).count(), 0);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Values").unwrap();
            book.add_sheet("Cases").unwrap();
            book.set_entry("Values", at("D2"), "=1/0").unwrap();
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let host = at(case["cell"].as_str().unwrap());
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut("Cases")
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(host, Scalar::from("prior"), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), host)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(
                (report.evaluated, report.uncomputed),
                (44, 0),
                "{year} saved={saved}"
            );
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                if held(case) {
                    assert_eq!(cell.value().as_str(), Some("prior"), "{}", case["id"]);
                } else if case["en_us"]["is_error"] == true {
                    assert_eq!(
                        cell.error().map(|v| v.as_str()),
                        case["en_us"]["error_type"]["literal"].as_str(),
                        "{}",
                        case["id"]
                    );
                } else {
                    assert_eq!(
                        cell.value().as_str(),
                        case["en_us"]["raw"]["value"].as_str(),
                        "{}",
                        case["id"]
                    );
                }
                checked += 1;
            }
            assert_eq!(book.recalculate().unwrap().evaluated, 0);
        }
    }
    assert_eq!(checked, 172); //172 exact LCID comparisons, including Boolean format coercion to #VALUE!.
}

#[test]
fn financial_annuities_match_both_native_corpora_and_saved_spellings() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/financial_native.json")).unwrap();
    assert_eq!(fixture["native_observations"], 382);
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| matches!(case["shape"]["function"].as_str(), Some("PV" | "FV")))
        .collect();
    assert_eq!(cases.len(), 176);
    let mut checked = 0;
    for (corpus, count) in [("primary", 52), ("orders", 36)] {
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            for saved in [false, true] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                book.add_sheet("Values").unwrap();
                book.add_sheet("Cases").unwrap();
                book.set_entry("Values", at("D2"), "=1/0").unwrap();
                let selected: Vec<_> = cases
                    .iter()
                    .filter(|case| case["corpus"] == corpus && case["date_system"] == year)
                    .collect();
                assert_eq!(selected.len(), count);
                for case in &selected {
                    let host = at(case["cell"].as_str().unwrap());
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
                    (report.evaluated, report.uncomputed),
                    ((count + 1) as u64, 0),
                    "{corpus} {year} saved={saved}"
                );
                for case in selected {
                    let cell = book
                        .sheet("Cases")
                        .unwrap()
                        .cell(at(case["cell"].as_str().unwrap()))
                        .unwrap();
                    let cache = &case["saved_cache"];
                    if cache["type"] == "e" {
                        assert_eq!(
                            cell.error().map(|v| v.as_str()),
                            cache["value_text"].as_str(),
                            "{}",
                            case["id"]
                        );
                    } else {
                        assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(
                                cache["value_text"]
                                    .as_str()
                                    .unwrap()
                                    .parse::<f64>()
                                    .unwrap()
                                    .to_bits()
                            ),
                            "{}",
                            case["id"]
                        );
                    }
                    checked += 1;
                }
                assert_eq!(book.recalculate().unwrap().evaluated, 0);
            }
        }
    }
    assert_eq!(checked, 352);
}

// SUBTOTAL source exclusion is also the graph dependency boundary.

#[test]
fn subtotal_nested_formula_is_excluded_from_value_but_watched_for_changes() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, entry) in [
        ("A1", "1"),
        ("A2", "=SUBTOTAL(9,A1:A1)"),
        ("A3", "2"),
        ("B1", "=SUBTOTAL(9,A1:A3)"),
    ] {
        book.set_entry("Data", at(cell), entry).unwrap();
    }
    let result = book.calculate_all().unwrap();
    assert_eq!(result.circular_count, 0);
    assert_eq!(result.uncomputed, 0);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(3.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A2")).as_f64(),
        Some(1.0)
    );
    // A skipped formula can become an included literal. The full-range
    // dirty watch must notice this change without a prior value edge.
    book.set_entry("Data", at("A2"), "4").unwrap();
    let changed = book.recalculate().unwrap();
    assert_eq!(changed.circular_count, 0);
    assert_eq!(changed.uncomputed, 0);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B1")).as_f64(),
        Some(7.0)
    );
}

#[test]
fn subtotal_109_hidden_formula_does_not_create_a_circular_dependency() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, entry) in [
        ("A1", "1"),
        ("A2", "=B2+100"),
        ("A3", "2"),
        ("B2", "=SUBTOTAL(109,A1:A3)"),
    ] {
        book.set_entry("Data", at(cell), entry).unwrap();
    }
    book.sheet_mut("Data")
        .unwrap()
        .set_rows_hidden(1..2, true)
        .unwrap();
    let result = book.calculate_all().unwrap();
    assert_eq!(result.circular_count, 0);
    assert_eq!(result.uncomputed, 0);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("B2")).as_f64(),
        Some(3.0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A2")).as_f64(),
        Some(103.0)
    );
}

#[test]
fn subtotal_nested_value_exclusion_keeps_native_circular_dependencies() {
    use yggdryl::excel::{CellRef, Workbook};
    let at = |text: &str| text.parse::<CellRef>().unwrap();
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (cell, entry) in [
        ("A1", "1"),
        ("A2", "=SUBTOTAL(9,B1:B1)"),
        ("A3", "2"),
        ("B1", "=SUBTOTAL(9,A1:A3)"),
    ] {
        book.set_entry("Data", at(cell), entry).unwrap();
    }
    let result = book.calculate_all().unwrap();
    // Excel 16.0 build 20430 reports Worksheet.CircularReference=$A$2.
    // Its default zero cache is not a calculated numeric answer.
    assert_eq!(
        (result.evaluated, result.uncomputed, result.circular_count),
        (0, 2, 2)
    );
    assert_eq!(
        result.circular,
        vec![("Data".into(), at("B1")), ("Data".into(), at("A2"))]
    );
}

#[test]
fn financial_npv_matches_native_discount_order_and_argument_origins() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/financial_native.json")).unwrap();
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["shape"]["function"] == "NPV")
        .collect();
    assert_eq!(cases.len(), 88);
    let mut checked = 0;
    for (corpus, count) in [("primary", 14), ("orders", 30)] {
        let run = fixture["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["corpus"] == corpus)
            .unwrap();
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            for saved in [false, true] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                book.add_sheet("Values").unwrap();
                book.add_sheet("Cases").unwrap();
                for (sheet, cells) in run["source_cells"].as_object().unwrap() {
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
                            .set_cell(at(address), value)
                            .unwrap();
                    }
                }
                book.set_entry("Values", at("D2"), "=1/0").unwrap();
                let selected: Vec<_> = cases
                    .iter()
                    .filter(|case| case["corpus"] == corpus && case["date_system"] == year)
                    .collect();
                assert_eq!(selected.len(), count);
                for case in &selected {
                    let host = at(case["cell"].as_str().unwrap());
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
                    (report.evaluated, report.uncomputed),
                    ((count + 1) as u64, 0),
                    "{corpus} {year} saved={saved}"
                );
                for case in selected {
                    let cell = book
                        .sheet("Cases")
                        .unwrap()
                        .cell(at(case["cell"].as_str().unwrap()))
                        .unwrap();
                    let cache = &case["saved_cache"];
                    if cache["type"] == "e" {
                        assert_eq!(
                            cell.error().map(|v| v.as_str()),
                            cache["value_text"].as_str(),
                            "{}",
                            case["id"]
                        );
                    } else {
                        assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(
                                cache["value_text"]
                                    .as_str()
                                    .unwrap()
                                    .parse::<f64>()
                                    .unwrap()
                                    .to_bits()
                            ),
                            "{}",
                            case["id"]
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 176);
}

#[test]
fn financial_pmt_keeps_nonzero_rate_policy_explicit_while_computing_proven_domains() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/financial_native.json")).unwrap();
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["shape"]["function"] == "PMT")
        .collect();
    assert_eq!(cases.len(), 114);
    let (mut checked, mut held) = (0, 0);
    for (corpus, count, supported) in [("primary", 26, 8), ("orders", 31, 6)] {
        for (year, system) in [
            ("1900", DateSystem::Year1900),
            ("1904", DateSystem::Year1904),
        ] {
            for saved in [false, true] {
                let mut book = Workbook::new();
                book.set_date_system(system);
                book.add_sheet("Values").unwrap();
                book.add_sheet("Cases").unwrap();
                book.set_entry("Values", at("D2"), "=1/0").unwrap();
                let selected: Vec<_> = cases
                    .iter()
                    .filter(|case| case["corpus"] == corpus && case["date_system"] == year)
                    .collect();
                assert_eq!(selected.len(), count);
                for case in &selected {
                    let host = at(case["cell"].as_str().unwrap());
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
                    (report.evaluated, report.uncomputed),
                    ((supported + 1) as u64, (count - supported) as u64),
                    "{corpus} {year} saved={saved}"
                );
                for case in selected {
                    let cell = book
                        .sheet("Cases")
                        .unwrap()
                        .cell(at(case["cell"].as_str().unwrap()))
                        .unwrap();
                    let cache = &case["saved_cache"];
                    // The native corpus is retained in full. This slice only
                    // promises zero-rate arithmetic and observed error domains.
                    if cache["type"] == "e" {
                        assert_eq!(
                            cell.error().map(|v| v.as_str()),
                            cache["value_text"].as_str(),
                            "{}",
                            case["id"]
                        );
                        checked += 1;
                    } else if case["shape"]["args"].as_str().unwrap().starts_with("0,") {
                        assert_eq!(
                            cell.value().as_f64().map(f64::to_bits),
                            Some(
                                cache["value_text"]
                                    .as_str()
                                    .unwrap()
                                    .parse::<f64>()
                                    .unwrap()
                                    .to_bits()
                            ),
                            "{}",
                            case["id"]
                        );
                        checked += 1;
                    } else {
                        assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                        held += 1;
                    }
                }
            }
        }
    }
    assert_eq!((checked, held), (56, 172));
}

#[test]
fn statistical_variance_exact_domain_keeps_all_native_cases_accounted_for() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/statistical_native.json")).unwrap();
    let cases: Vec<_> = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| {
            matches!(
                case["shape"]["function"].as_str(),
                Some(
                    "VAR"
                        | "VARP"
                        | "STDEV"
                        | "STDEVP"
                        | "_xlfn.VAR.S"
                        | "_xlfn.VAR.P"
                        | "_xlfn.STDEV.S"
                        | "_xlfn.STDEV.P"
                )
            )
        })
        .collect();
    assert_eq!(cases.len(), 192);
    let (mut checked, mut held) = (0, 0);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "CycleShape", "Cases"] {
                book.add_sheet(name).unwrap();
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
                        .set_cell(at(address), value)
                        .unwrap();
                }
            }
            book.set_entry("Values", at("E1"), "=1/0").unwrap();
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .collect();
            for case in &selected {
                let host = at(case["cell"].as_str().unwrap());
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
                (report.evaluated, report.uncomputed),
                (73, 24),
                "{year} saved={saved}"
            );
            for case in selected {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                let cache = &case["saved_cache"];
                let supported = matches!(
                    case["shape"]["operand_shape"].as_str(),
                    Some(
                        "error-reference"
                            | "single"
                            | "three"
                            | "zeros"
                            | "mixed-range"
                            | "numeric-plus-zero"
                            | "numeric-range"
                            | "numeric-text-direct"
                            | "true-direct"
                    )
                );
                if !supported {
                    assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                    held += 1;
                    continue;
                }
                if cache["type"] == "e" {
                    assert_eq!(
                        cell.error().map(|v| v.as_str()),
                        cache["value_text"].as_str(),
                        "{}",
                        case["id"]
                    );
                } else {
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap()
                                .to_bits()
                        ),
                        "{}",
                        case["id"]
                    );
                }
                checked += 1;
            }
        }
    }
    assert_eq!((checked, held), (288, 96));
}

#[test]
fn statistical_variance_exact_boundary_observations_preserve_outside_domain_caches() {
    use yggdryl::excel::Formula;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/variance_exact_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 192);
    let (mut checked, mut held) = (0, 0);
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            book.add_sheet("Values").unwrap();
            book.add_sheet("Cases").unwrap();
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .collect();
            for case in &selected {
                let host = at(case["cell"].as_str().unwrap());
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
                (report.evaluated, report.uncomputed),
                (72, 24),
                "{year} saved={saved}"
            );
            for case in selected {
                let cell = book
                    .sheet("Cases")
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                let cache = &case["saved_cache"];
                if matches!(
                    case["shape"]["label"].as_str(),
                    Some("past-bound" | "decimal" | "square-sum-bound")
                ) {
                    assert_eq!(cell.value().as_f64(), Some(-777.0), "{}", case["id"]);
                    held += 1;
                    continue;
                }
                if cache["type"] == "e" {
                    assert_eq!(
                        cell.error().map(|v| v.as_str()),
                        cache["value_text"].as_str(),
                        "{}",
                        case["id"]
                    );
                } else {
                    assert_eq!(
                        cell.value().as_f64().map(f64::to_bits),
                        Some(
                            cache["value_text"]
                                .as_str()
                                .unwrap()
                                .parse::<f64>()
                                .unwrap()
                                .to_bits()
                        ),
                        "{}",
                        case["id"]
                    );
                }
                checked += 1;
            }
        }
    }
    assert_eq!((checked, held), (288, 96));
}

#[test]
fn criteria_typed_text_cache_follows_normal_and_forgotten_cell_mutation() {
    use yggdryl::excel::{Cell, DateSystem};
    for forgotten in [false, true] {
        for number in [false, true] {
            let mut book = Workbook::new();
            let sheet = book.add_sheet("Data").unwrap();
            sheet
                .set_cell(
                    at("A1"),
                    Scalar::from_sequence([Scalar::from("before"), Scalar::from(17_i64)]),
                )
                .unwrap();
            book.set_entry("Data", at("B1"), "=COUNTIF(A1,\"*before*\")")
                .unwrap();
            book.set_entry("Data", at("C1"), "=COUNTIF(A1,\"*after*\")")
                .unwrap();
            assert_eq!(book.calculate_all().unwrap().evaluated, 2);
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("B1")),
                Scalar::from(1.0)
            );
            let value = if number {
                Scalar::from(42.0)
            } else {
                Scalar::from_sequence([Scalar::from("after"), Scalar::from(23_i64)])
            };
            let mut guard = book.sheet_mut("Data").unwrap().cell_mut(at("A1")).unwrap();
            *guard = Cell::from_scalar(at("A1"), value, DateSystem::Year1900).unwrap();
            if forgotten {
                std::mem::forget(guard)
            } else {
                drop(guard)
            };
            assert_eq!(book.recalculate().unwrap().evaluated, 2);
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("B1")),
                Scalar::from(0.0),
                "forgotten={forgotten} number={number}"
            );
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("C1")),
                Scalar::from(if number { 0.0 } else { 1.0 })
            );
        }
    }
}

#[test]
fn full_function_native_oracle_replays_exact_scope_and_explicit_policy_holds() {
    use yggdryl::Timezone;
    use yggdryl::excel::{Clock, Formula};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/functions_oracle_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 324);
    assert_eq!(
        (
            fixture["native_passed"].as_bool(),
            fixture["cleanup_completed"].as_bool()
        ),
        (Some(true), Some(true))
    );
    let (mut exact, mut held, mut behavior) = (0, 0, 0);
    let mut failures = Vec::new();
    for (year, system, authored, native) in [
        (
            "1900",
            DateSystem::Year1900,
            include_bytes!("fixtures/functions_oracle_input_1900.xlsx").as_slice(),
            include_bytes!("fixtures/functions_excel.xlsx").as_slice(),
        ),
        (
            "1904",
            DateSystem::Year1904,
            include_bytes!("fixtures/functions_oracle_input_1904.xlsx").as_slice(),
            include_bytes!("fixtures/functions_excel_1904.xlsx").as_slice(),
        ),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::from_bytes(if saved { native } else { authored }.to_vec())
                .unwrap()
                .with_clock(Clock::fixed(0, Timezone::UTC, 711));
            book.parse_all().unwrap();
            if saved && year == "1900" {
                assert_eq!(
                    book.sheet("Errors")
                        .unwrap()
                        .cell(at("A8"))
                        .unwrap()
                        .error()
                        .map(|error| error.as_str()),
                    Some("#N/A")
                );
                assert_eq!(
                    book.sheet("Cases").unwrap().scalar(at("B293")).as_f64(),
                    Some(8.0)
                );
            }
            let selected: Vec<_> = cases
                .iter()
                .filter(|case| case["date_system"] == year)
                .collect();
            for case in &selected {
                let host = at(case["cell"].as_str().unwrap());
                let sheet = case["sheet"].as_str().unwrap();
                // English source TEXT is evaluated by Rust's en-US owner.
                // Its independently proved companion supplies the expected
                // value only; a French localized formula is never substituted.
                let formula = if saved {
                    &case["saved_cache"]["formula_text"]
                } else {
                    &case["wire_formula"]
                };
                book.sheet_mut(sheet)
                    .unwrap()
                    .insert_cell(
                        Cell::from_scalar(host, Scalar::from(-777.0), system)
                            .unwrap()
                            .with_formula(Formula::from_file(formula.as_str().unwrap(), host)),
                    )
                    .unwrap();
            }
            let report = book.calculate_all().unwrap();
            assert_eq!(report.circular_count, 0, "{year} saved={saved}");
            for case in selected {
                let id = case["id"].as_str().unwrap();
                let cell = book
                    .sheet(case["sheet"].as_str().unwrap())
                    .unwrap()
                    .cell(at(case["cell"].as_str().unwrap()))
                    .unwrap();
                // Excel SaveAs may normalize a source error without recomputing
                // its same-session consumer cache. Reopened native evidence is
                // distinct from the original retained cache and input proof.
                let cache = if saved && case.get("rust_saved_recalculated_cache").is_some() {
                    &case["rust_saved_recalculated_cache"]
                } else {
                    &case["rust_expected_cache"]
                };
                let okay = match case["rust_policy"]["kind"].as_str().unwrap() {
                    "held" => {
                        held += 1;
                        cell.error().is_none() && cell.value().as_f64() == Some(-777.0)
                    }
                    "behavior" => {
                        behavior += 1;
                        cell.value().as_f64().is_some_and(|number| match id {
                            "function-rand" => (0.0..1.0).contains(&number),
                            "function-randbetween" => {
                                number.fract() == 0.0 && (-5.0..=7.0).contains(&number)
                            }
                            "function-now" | "function-today" => {
                                number == if year == "1900" { 25569.0 } else { 24107.0 }
                            }
                            "clock-day-fraction" => number == 0.0,
                            _ => panic!("unclassified volatile case {id}"),
                        })
                    }
                    "exact" => {
                        exact += 1;
                        match cache["type"].as_str().unwrap() {
                            "n" => {
                                cell.error().is_none()
                                    && cell.value().as_f64().map(f64::to_bits)
                                        == Some(
                                            cache["value_text"]
                                                .as_str()
                                                .unwrap()
                                                .parse::<f64>()
                                                .unwrap()
                                                .to_bits(),
                                        )
                            }
                            "b" => cell.value().as_bool() == Some(cache["value_text"] == "1"),
                            "str" => cell.value().as_str() == cache["value_text"].as_str(),
                            "e" => {
                                cell.error().map(|error| error.as_str())
                                    == cache["value_text"].as_str()
                            }
                            other => panic!("unexpected cache type {other} for {id}"),
                        }
                    }
                    other => panic!("unclassified Rust policy {other}"),
                };
                if !okay {
                    failures.push(format!("{id} ({year}, saved={saved}): policy={} expected={cache}, actual={:?}, error={:?}",case["rust_policy"]["kind"],cell.value(),cell.error()));
                }
            }
        }
    }
    // Union and intersection now compute the four authored/saved native cases.
    assert_eq!((exact, held, behavior), (624, 14, 10));
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn cold_sheet_state_survives_late_parse_and_native_package_save() {
    use yggdryl::excel::SheetState;
    for parse_before_save in [false, true] {
        let mut authored = Workbook::new();
        authored
            .add_sheet("Data")
            .unwrap()
            .set_cell(at("A1"), 7.0)
            .unwrap();
        authored.add_sheet("Other").unwrap();
        let mut book = Workbook::from_bytes(authored.into_bytes().unwrap()).unwrap();
        book.set_sheet_state("Data", SheetState::VeryHidden)
            .unwrap();
        assert_eq!(book.sheet_state("Data"), Some(SheetState::VeryHidden));
        if parse_before_save {
            assert_eq!(
                book.sheet("Data").unwrap().scalar(at("A1")),
                Scalar::from(7.0)
            );
        }
        let saved = book.into_bytes().unwrap();
        let reopened = Workbook::from_bytes(saved).unwrap();
        assert_eq!(reopened.sheet_state("Data"), Some(SheetState::VeryHidden));
        assert_eq!(
            reopened.sheet("Data").unwrap().scalar(at("A1")),
            Scalar::from(7.0)
        );
        assert_eq!(reopened.sheet_state("Data"), Some(SheetState::VeryHidden));
    }
}

#[test]
fn criteria_tilde_literal_and_wildcard_native_292_cache_cases() {
    use yggdryl::{
        Scalar,
        excel::{Cell, CellRef, DateSystem, Formula, Workbook},
    };
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/criteria_tilde_native.json")).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_equal"], 584);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 292);
    let mut checked = 0;
    for (year, system) in [
        ("1900", DateSystem::Year1900),
        ("1904", DateSystem::Year1904),
    ] {
        for saved in [false, true] {
            let mut book = Workbook::new();
            book.set_date_system(system);
            for name in ["Values", "Cases"] {
                book.add_sheet(name).unwrap();
            }
            for (address, value) in fixture["source_cells"]["Values"].as_object().unwrap() {
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
            for case in cases.iter().filter(|case| case["date_system"] == year) {
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
            for case in cases.iter().filter(|case| case["date_system"] == year) {
                let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
                let cell = book.sheet("Cases").unwrap().cell(at).unwrap();
                let cache = &case["saved_cache"];
                assert_eq!(cache["type"], "n", "{}", case["id"]);
                let expected = cache["value_text"]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                assert_eq!(
                    cell.value().as_f64().map(f64::to_bits),
                    Some(expected.to_bits()),
                    "{} saved={saved}",
                    case["id"]
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 584);
}

#[test]
fn literal_arrays_named_aliases_preserve_shape_and_origin_through_selected_results() {
    use yggdryl::excel::Formula;
    let mut book = calculation_named_book(
        r#"<definedName name="Constants">{1,TRUE,"3"}</definedName><definedName name="Alias">Constants</definedName>"#,
        DateSystem::Year1900,
    );
    for (address, formula, number, text) in [
        ("A1", "SUM(Alias)", Some(1.0), None),
        ("A2", "COUNTA(Alias)", Some(3.0), None),
        ("A3", "COLUMNS(Alias)", Some(3.0), None),
        ("A4", "SUM(IF(TRUE,Alias,{9,8,7}))", Some(1.0), None),
        ("A5", "_xlfn.CONCAT(Alias)", None, Some("1TRUE3")),
        ("A6", "SUM(Alias)+SUM(Constants)", Some(2.0), None),
    ] {
        book.sheet_mut("Data")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at(address), Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at(address))),
            )
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!(report.uncomputed, 0, "{formula}");
        let value = book.sheet("Data").unwrap().scalar(at(address));
        if let Some(number) = number {
            assert_eq!(value.as_f64(), Some(number), "{formula}");
        }
        if let Some(text) = text {
            assert_eq!(value.as_str(), Some(text), "{formula}");
        }
    }
}

#[test]
fn reference_intersection_generated_components_have_a_named_bound() {
    use yggdryl::excel::Formula;
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap();
    book.add_sheet("Cases").unwrap();
    for row in 0..65 {
        book.sheet_mut("Values")
            .unwrap()
            .set_cell(CellRef::new(row, 0), f64::from(row + 1))
            .unwrap();
    }
    let union = |count: u32| {
        (1..=count)
            .map(|row| format!("Values!$A${row}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let limited = union(32);
    let excessive = union(65);
    let formulas = [
        format!("SUM(({limited}) ({limited}))"),
        format!("SUM(({excessive}) ({excessive}))"),
        "SUM(Values!$A:$A Values!$A$1)".to_owned(),
    ];
    for (row, formula) in formulas.iter().enumerate() {
        let at = CellRef::new(row as u32, 0);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(formula, at)),
            )
            .unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (2, 1, 0)
    );
    let cases = book.sheet("Cases").unwrap();
    assert_eq!(cases.scalar(CellRef::new(0, 0)).as_f64(), Some(528.0));
    assert_eq!(cases.scalar(CellRef::new(1, 0)).as_f64(), Some(-777.0));
    // Cell count is not the reference-component budget: a full sparse column
    // plus one point has only two area descriptors.
    assert_eq!(cases.scalar(CellRef::new(2, 0)).as_f64(), Some(1.0));
}

#[cfg(feature = "internals")]
#[test]
fn named_union_dag_respects_reference_component_budget() {
    let mut names = String::from("<definedName name=\"AreaLevel0\">Data!$A$1</definedName>");
    for level in 1..=14 {
        names.push_str(&format!(
            "<definedName name=\"AreaLevel{level}\">(AreaLevel{},AreaLevel{})</definedName>",
            level - 1,
            level - 1
        ));
    }
    let mut book = calculation_named_book(&names, DateSystem::Year1900);
    book.sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 1.0)
        .unwrap();
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A1", "SUM(AreaLevel12)", -777.0))
        .unwrap();
    book.sheet_mut("Cases")
        .unwrap()
        .insert_cell(calculation_cached("A2", "SUM(AreaLevel14)", -777.0))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (1, 1, 0)
    );
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A1")),
        Scalar::from(4096.0)
    );
    // The prior cache remains because 16,384 repeated leaves exceed the
    // formula-derived 8,192 component budget before any value is published.
    assert_eq!(
        book.sheet("Cases").unwrap().scalar(at("A2")),
        Scalar::from(-777.0)
    );
}

#[test]
fn literal_arrays_selected_branches_admit_only_reached_cycles_and_dirty_sources() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (address, formula) in [
        ("A1", "=SUM(IF({TRUE,TRUE},{1,2},A1))"),
        ("B1", "=SUM(IF({TRUE,FALSE},{1,2},B1))"),
        ("C1", "=SUM(CHOOSE({1,1},{1,2},C1))"),
        ("D1", "=SUM(IF({TRUE,FALSE},E1,F1))"),
        ("E1", "=10"),
        ("F1", "=20"),
    ] {
        book.set_entry("Data", at(address), formula).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (5, 1, 1)
    );
    assert_eq!(report.circular, [("Data".into(), at("B1"))]);
    for (address, value) in [("A1", 3.0), ("C1", 3.0), ("D1", 30.0)] {
        assert_eq!(
            book.sheet("Data").unwrap().scalar(at(address)).as_f64(),
            Some(value)
        );
    }
    book.set_entry("Data", at("E1"), "=11").unwrap();
    let report = book.recalculate().unwrap();
    assert_eq!(report.evaluated, 2);
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("D1")).as_f64(),
        Some(31.0)
    );
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn literal_arrays_selected_unrepresented_range_does_not_admit_false_cycles() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    book.sheet_mut("Data")
        .unwrap()
        .insert_cell(calculation_cached(
            "A1",
            "SUM(IF({TRUE,FALSE},A1:A3,{0,0}))",
            77.0,
        ))
        .unwrap();
    let report = book.calculate_all().unwrap();
    assert_eq!(
        (report.evaluated, report.uncomputed, report.circular_count),
        (0, 1, 0)
    );
    assert_eq!(
        book.sheet("Data").unwrap().scalar(at("A1")),
        Scalar::from(77.0)
    );
}

#[cfg(feature = "internals")]
#[test]
fn literal_arrays_named_mapped_dag_executes_each_plan_once_per_position() {
    for levels in [10, 20] {
        let mut names = String::from("<definedName name=\"ArrayLevel0\">{1,2}</definedName>");
        for index in 1..=levels {
            names.push_str(&format!(
                "<definedName name=\"ArrayLevel{index}\">ArrayLevel{}+ArrayLevel{}</definedName>",
                index - 1,
                index - 1
            ));
        }
        let mut book = calculation_named_book(&names, DateSystem::Year1900);
        book.sheet_mut("Cases")
            .unwrap()
            .insert_cell(calculation_cached(
                "A1",
                &format!("SUM(ArrayLevel{levels})"),
                -1.0,
            ))
            .unwrap();
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated, report.uncomputed), (1, 0));
        assert_eq!(
            book.sheet("Cases").unwrap().scalar(at("A1")),
            Scalar::from(3.0 * (1_u64 << levels) as f64)
        );
        assert_eq!(
            yggdryl::internals::excel_workbook::array_work(&book),
            2 * levels,
            "repeated alias operands must reuse each position"
        );
    }
}
