//! `rust/src/excel/names.rs`: `DefinedName` - the workbook's defined names read with their scope, formula, hidden flag and comment, written back as they were until a sheet they name is renamed or removed.

use yggdryl::excel::{CellRef, Workbook};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;

use crate::excel_package::rich_package;

fn member(bytes: &[u8], name: &str) -> String {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// The `<definedNames>` element of the workbook part in `bytes`.
fn defined_names(bytes: &[u8]) -> String {
    let book = member(bytes, "xl/workbook.xml");
    let start = book.find("<definedNames>").unwrap_or(0);
    let end = book
        .find("</definedNames>")
        .map_or(start, |at| at + "</definedNames>".len());
    book[start..end].to_owned()
}

#[test]
fn a_workbook_reads_each_defined_name_with_its_scope_formula_and_flags() {
    let workbook = Workbook::from_bytes(rich_package()).unwrap();
    let names: Vec<_> = workbook.defined_names().collect();
    assert_eq!(
        names.iter().map(|name| name.name()).collect::<Vec<_>>(),
        [
            "_xlnm._FilterDatabase",
            "_xlnm.Print_Area",
            "Prices",
            "Rate"
        ]
    );
    let data = workbook.sheet_key("Data");
    let report = workbook.sheet_key("Report");
    assert_eq!(
        names.iter().map(|name| name.scope()).collect::<Vec<_>>(),
        [data, report, None, None]
    );
    assert_eq!(names[0].text(), "Data!$A$1:$C$4");
    assert!(names[0].is_hidden());
    assert!(!names[2].is_hidden());
    assert_eq!(names[2].comment(), Some("the price column"));
    assert_eq!(names[3].text(), "0.07");
    // A name's formula is held at `A1`, its references relative to it.
    assert_eq!(
        names[2].formula().at(CellRef::new(0, 0)).to_string(),
        "Data!$B$2:$B$4"
    );
    assert_eq!(workbook.active_tab(), 1);
}

#[test]
fn names_are_written_as_they_were_until_a_sheet_they_name_changes() {
    let original = rich_package();
    let mut workbook = Workbook::from_bytes(original.clone()).unwrap();
    workbook
        .sheet_mut("Report")
        .unwrap()
        .set_cell("Z1".parse().unwrap(), 1.0)
        .unwrap();
    let saved = workbook.into_bytes().unwrap();
    assert_eq!(defined_names(&saved), defined_names(&original));

    // A rename restates every name naming the sheet, and only those.
    workbook.rename_sheet("Data", "Q1 Data").unwrap();
    let renamed = workbook.into_bytes().unwrap();
    assert_eq!(
        defined_names(&renamed),
        "<definedNames><definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">'Q1 Data'!$A$1:$C$4</definedName>\
         <definedName name=\"_xlnm.Print_Area\" localSheetId=\"1\">Report!$A$1:$E$4</definedName>\
         <definedName name=\"Prices\" comment=\"the price column\">'Q1 Data'!$B$2:$B$4</definedName>\
         <definedName name=\"Rate\">0.07</definedName></definedNames>"
    );
}

#[test]
fn a_removed_sheet_takes_the_names_defined_on_it_and_leaves_the_rest_counting_the_tabs_left() {
    let mut workbook = Workbook::from_bytes(rich_package()).unwrap();
    workbook.remove_sheet("Data").unwrap();
    assert_eq!(
        workbook
            .defined_names()
            .map(|name| (name.name().to_owned(), name.text()))
            .collect::<Vec<_>>(),
        [
            ("_xlnm.Print_Area".to_owned(), "Report!$A$1:$E$4".to_owned()),
            ("Prices".to_owned(), "#REF!$B$2:$B$4".to_owned()),
            ("Rate".to_owned(), "0.07".to_owned()),
        ]
    );
    let saved = workbook.into_bytes().unwrap();
    assert_eq!(
        defined_names(&saved),
        "<definedNames><definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">Report!$A$1:$E$4</definedName>\
         <definedName name=\"Prices\" comment=\"the price column\">#REF!$B$2:$B$4</definedName>\
         <definedName name=\"Rate\">0.07</definedName></definedNames>"
    );
    // The names read back from what was written.
    let reopened = Workbook::from_bytes(saved).unwrap();
    assert_eq!(
        reopened.defined_names().next().map(|name| name.scope()),
        Some(reopened.sheet_key("Report"))
    );
}

#[test]
fn a_workbook_whose_names_all_go_writes_no_defined_names() {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Only").unwrap();
    let bytes = workbook.into_bytes().unwrap();
    assert!(!member(&bytes, "xl/workbook.xml").contains("definedName"));
}

#[test]
fn a_name_past_a_sheet_prefix_in_any_script_opens_and_writes_back() {
    use crate::excel_package::{
        NS, R_NS, content_types, package, root_relationships, workbook_relationships, worksheet,
    };
    let book = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <workbook xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><sheets>\
         <sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets><definedNames>\
         <definedName name=\"Итого\" localSheetId=\"0\">Sheet1!Größe*2</definedName>\
         <definedName name=\"名前\">Лист1!Итого+Sheet1!名前</definedName>\
         </definedNames></workbook>"
    );
    let sheet = worksheet(
        "<row r=\"1\"><c r=\"A1\"><f>Sheet1!Größe*2</f><v>0</v></c>\
         <c r=\"B1\"><f>Лист1!Итого</f><v>0</v></c><c r=\"C1\"><f>Sheet1!名前</f><v>0</v></c></row>",
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(1, false, false)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &book),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let mut workbook = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(
        workbook
            .defined_names()
            .map(|name| name.text())
            .collect::<Vec<_>>(),
        ["Sheet1!Größe*2", "Лист1!Итого+Sheet1!名前"]
    );
    let first = workbook.sheet("Sheet1").unwrap();
    let formulas: Vec<_> = ["A1", "B1", "C1"]
        .into_iter()
        .map(|at| {
            let at: CellRef = at.parse().unwrap();
            first
                .cell(at)
                .unwrap()
                .formula()
                .unwrap()
                .at(at)
                .to_string()
        })
        .collect();
    assert_eq!(formulas, ["Sheet1!Größe*2", "Лист1!Итого", "Sheet1!名前"]);
    // Written again from its cells, each formula keeps its text.
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("D1".parse().unwrap(), 1.0)
        .unwrap();
    workbook.rename_sheet("Sheet1", "Лист1").unwrap();
    let saved = workbook.into_bytes().unwrap();
    let part = member(&saved, "xl/worksheets/sheet1.xml");
    assert!(part.contains("<f>Лист1!Größe*2</f>"), "{part}");
    assert!(part.contains("<f>Лист1!Итого</f>"), "{part}");
    assert!(
        defined_names(&saved).contains(">Лист1!Итого+Лист1!名前</definedName>"),
        "{}",
        defined_names(&saved)
    );
}

#[test]
fn a_sheet_put_in_place_of_a_tab_of_its_name_is_what_the_tab_s_names_are_defined_on() {
    let mut workbook = Workbook::from_bytes(rich_package()).unwrap();
    // A rename first, so the names are written again from the model.
    workbook.rename_sheet("Report", "Rep").unwrap();
    let data = workbook.sheet("Data").unwrap().clone();
    workbook.insert_sheet(data).unwrap();
    assert_eq!(
        workbook.defined_names().next().map(|name| name.scope()),
        Some(workbook.sheet_key("Data"))
    );
    let saved = workbook.into_bytes().unwrap();
    assert!(
        defined_names(&saved).starts_with(
            "<definedNames><definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">"
        ),
        "{}",
        defined_names(&saved)
    );
    // Without a rename, the names are copied as they were: the same scope.
    let mut workbook = Workbook::from_bytes(rich_package()).unwrap();
    let data = workbook.sheet("Data").unwrap().clone();
    workbook.insert_sheet(data).unwrap();
    assert_eq!(
        workbook.defined_names().next().map(|name| name.scope()),
        Some(workbook.sheet_key("Data"))
    );
}
