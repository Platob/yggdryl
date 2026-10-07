//! `rust/src/excel/workbook.rs`: `Workbook` - the sheets of one package found without case, parsed on demand at a pinned read cost, and written back with every untouched part carried over.

use std::sync::Arc;

use yggdryl::excel::{CellRef, DateSystem, NumberFormat, Sheet, SheetKind, SheetState, Workbook};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;
use yggdryl::{Error, Scalar, TimeUnit, Timezone};

use crate::excel_package::{
    NS, R_NS, content_types, package, root_relationships, shared_strings, styles, workbook,
    workbook_relationships, worksheet,
};

const WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const CHARTSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";

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
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", book),
        ("xl/_rels/workbook.xml.rels", &rels),
        ("xl/worksheets/sheet1.xml", &sheet),
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
    assert!(
        part.contains("<sheet name=\"Tucked\" sheetId=\"2\" r:id=\"rId5\" state=\"hidden\"/>"),
        "{part}"
    );
    assert!(
        part.contains("<sheet name=\"Locked\" sheetId=\"3\" r:id=\"rId6\" state=\"veryHidden\"/>"),
        "{part}"
    );
    assert!(
        part.contains("<sheet name=\"Added\" sheetId=\"4\" r:id=\"rId7\"/>"),
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
    for name in built.sheet_names() {
        assert_eq!(
            reopened.sheet(name).unwrap(),
            built.sheet(name).unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        reopened
            .sheet("Prices")
            .unwrap()
            .scalar(at("E2"))
            .into_json()
            .unwrap(),
        "\"2024-01-01T12:00:00\""
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
    // Sheet relationships take ids past every id the part already held.
    let rels = member_text(&written, "xl/_rels/workbook.xml.rels");
    assert!(
        rels.contains(&format!(
            "<Relationship Id=\"rId5\" Type=\"{WORKSHEET}\" Target=\"worksheets/sheet6.xml\"/>"
        )),
        "{rels}"
    );
    assert!(
        rels.contains(&format!(
            "<Relationship Id=\"rId4\" Type=\"{WORKSHEET}\" Target=\"worksheets/sheet5.xml\"/>"
        )),
        "{rels}"
    );
    let part = member_text(&written, "xl/workbook.xml");
    assert!(
        part.contains("<sheet name=\"Third\" sheetId=\"3\" r:id=\"rId5\"/>"),
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
    let part = member_text(&written, "xl/workbook.xml");
    assert!(
        part.contains(
            "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId4\"/>\
             <sheet name=\"Sheet3\" sheetId=\"2\" r:id=\"rId5\"/></sheets>"
        ),
        "{part}"
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
