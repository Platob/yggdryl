//! `rust/src/excel/layout.rs`: what a worksheet states about its grid beside its cells - column spans, row formats (a row with a format and no cell a row of its own), merges, the frozen pane of the first view and the defaults - read once, answered by the sheet and written back from the model.

use yggdryl::Error;
use yggdryl::excel::{
    CellRef, DEFAULT_COLUMN_WIDTH, DEFAULT_ROW_HEIGHT, Frozen, Sheet, StyleId, Workbook,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::ZipArchive;

use crate::excel_package::{
    NS, R_NS, content_types, package, rich_shared_strings, rich_styles, rich_worksheet,
    root_relationships, workbook, workbook_relationships,
};

/// A one-sheet package whose worksheet part is `sheet` exactly as spelled,
/// beside the shared strings and the styles of the rich package, which its
/// cells index.
fn book_with(sheet: &str) -> Vec<u8> {
    let types = content_types(1, true, true);
    let root = root_relationships();
    let relationships = workbook_relationships(1, true, true);
    let book = workbook(&["Sheet1"], false);
    let strings = rich_shared_strings();
    let styles = rich_styles();
    package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/sharedStrings.xml", strings.as_str()),
        ("xl/styles.xml", styles.as_str()),
    ])
}

/// A worksheet part stating `before` ahead of its cells, `rows` as its
/// rows and `after` behind them.
fn part(before: &str, rows: &str, after: &str) -> String {
    format!(
        "<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\">{before}<sheetData>{rows}</sheetData>{after}</worksheet>"
    )
}

fn member(bytes: &[u8], name: &str) -> String {
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// The refusal opening `sheet` answers, as text.
fn refusal(sheet: &str) -> String {
    let workbook = Workbook::from_bytes(book_with(sheet)).unwrap();
    match workbook.sheet("Sheet1") {
        Err(error @ Error::InvalidRecord { .. }) => error.to_string(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn qualified_layout_attributes_do_not_override_unqualified_facts() {
    for namespace in [NS, "urn:foreign"] {
        let xml = part(
            &format!(
                "<sheetFormatPr xmlns:q=\"{namespace}\" defaultColWidth=\"13\" defaultRowHeight=\"18\" q:defaultColWidth=\"60\" q:defaultRowHeight=\"45\"/>\
                 <cols><col xmlns:q=\"{namespace}\" min=\"1\" max=\"1\" width=\"12\" style=\"1\" hidden=\"0\" q:min=\"3\" q:max=\"3\" q:width=\"98\" q:style=\"2\" q:hidden=\"1\"/></cols>"
            ),
            "",
            &format!(
                "<mergeCells count=\"1\"><mergeCell xmlns:q=\"{namespace}\" q:ref=\"C3:D3\" ref=\"A1:B1\"/></mergeCells>"
            ),
        );
        let book = Workbook::from_bytes(book_with(&xml)).unwrap();
        let sheet = book.sheet("Sheet1").unwrap();
        assert_eq!(sheet.default_column_width(), 13.0);
        assert_eq!(sheet.default_row_height(), 18.0);
        assert_eq!(sheet.column_width(0), 12.0);
        assert_eq!(sheet.column_style(0), Some(StyleId::new(1)));
        assert!(!sheet.is_column_hidden(0));
        assert_eq!(sheet.column_width(2), 13.0);
        assert_eq!(
            sheet
                .merges()
                .map(|range| range.to_string())
                .collect::<Vec<_>>(),
            ["A1:B1"]
        );
    }
}

#[test]
fn qualified_layout_attributes_do_not_supply_required_unqualified_facts() {
    for namespace in [NS, "urn:foreign"] {
        let columns = part(
            &format!(
                "<cols><col xmlns:q=\"{namespace}\" q:min=\"1\" q:max=\"1\" width=\"12\"/></cols>"
            ),
            "",
            "",
        );
        assert!(refusal(&columns).contains("without"));
        let merge = part(
            "",
            "",
            &format!(
                "<mergeCells count=\"1\"><mergeCell xmlns:q=\"{namespace}\" q:ref=\"A1:B1\"/></mergeCells>"
            ),
        );
        assert!(refusal(&merge).contains("without"));
    }
}

#[test]
fn a_sheet_reads_its_columns_rows_merges_pane_and_defaults() {
    let workbook = Workbook::from_bytes(book_with(&rich_worksheet())).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    // `<col min="1" max="1" width="14.7265625">`, then B and C at one width
    // and style; D states nothing, and the sheet states no default width.
    assert_eq!(sheet.column_width(0), 14.726_562_5);
    assert_eq!(sheet.column_width(1), 10.542_968_75);
    assert_eq!(sheet.column_width(2), 10.542_968_75);
    assert_eq!(sheet.column_width(3), DEFAULT_COLUMN_WIDTH);
    assert_eq!(sheet.default_column_width(), DEFAULT_COLUMN_WIDTH);
    assert_eq!(sheet.column_style(0), None);
    assert_eq!(sheet.column_style(2), Some(StyleId::new(2)));
    assert!(!sheet.is_column_hidden(1));
    // Row 2 states its height; the rest take `defaultRowHeight`.
    assert_eq!(sheet.default_row_height(), 14.5);
    assert_eq!(sheet.row_height(1), 20.0);
    assert_eq!(sheet.row_height(0), 14.5);
    // Row 5 is hidden and styled; row 7 holds no cell and states both a
    // height and a hidden flag.
    assert!(sheet.is_row_hidden(4));
    assert_eq!(sheet.row_style(4), Some(StyleId::new(1)));
    assert_eq!(sheet.row_style(1), None);
    assert!(sheet.is_row_hidden(6));
    assert_eq!(sheet.row_height(6), 30.0);
    assert!(sheet.row(6).is_none());
    assert_eq!(
        sheet
            .merges()
            .map(|range| range.to_string())
            .collect::<Vec<_>>(),
        ["A6:C6"]
    );
    assert_eq!(
        sheet.frozen(),
        Some(Frozen {
            rows: 1,
            columns: 1
        })
    );
}

#[test]
fn a_sheet_stating_nothing_answers_excel_s_defaults() {
    let sheet = Sheet::new("Empty").unwrap();
    assert_eq!(sheet.column_width(5), DEFAULT_COLUMN_WIDTH);
    assert_eq!(sheet.row_height(5), DEFAULT_ROW_HEIGHT);
    assert!(!sheet.is_row_hidden(0));
    assert_eq!(sheet.row_style(0), None);
    assert_eq!(sheet.merges().count(), 0);
    assert_eq!(sheet.frozen(), None);
    // A row stating a style without `customFormat` shows no style of its own.
    let workbook = Workbook::from_bytes(book_with(&part(
        "",
        "<row r=\"1\" s=\"3\"><c r=\"A1\"><v>1</v></c></row>",
        "",
    )))
    .unwrap();
    assert_eq!(workbook.sheet("Sheet1").unwrap().row_style(0), None);
}

#[test]
fn only_a_frozen_pane_of_the_first_view_is_a_frozen_pane() {
    for (views, frozen) in [
        (
            "<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"2\" topLeftCell=\"A3\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>",
            Some(Frozen {
                rows: 2,
                columns: 0,
            }),
        ),
        (
            "<sheetViews><sheetView workbookViewId=\"0\"><pane xSplit=\"3\" topLeftCell=\"D1\" activePane=\"topRight\" state=\"frozenSplit\"/></sheetView></sheetViews>",
            Some(Frozen {
                rows: 0,
                columns: 3,
            }),
        ),
        // A split pane scrolls: it is carried, and nothing is frozen.
        (
            "<sheetViews><sheetView workbookViewId=\"0\"><pane xSplit=\"2000\" ySplit=\"1500\" topLeftCell=\"C4\" activePane=\"bottomRight\"/></sheetView></sheetViews>",
            None,
        ),
        // The second view's pane is that view's.
        (
            "<sheetViews><sheetView workbookViewId=\"0\"/><sheetView workbookViewId=\"1\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>",
            None,
        ),
    ] {
        let workbook = Workbook::from_bytes(book_with(&part(views, "", ""))).unwrap();
        assert_eq!(
            workbook.sheet("Sheet1").unwrap().frozen(),
            frozen,
            "{views}"
        );
    }
}

#[test]
fn a_row_with_a_format_and_no_cell_survives_a_save() {
    let original = part(
        "<cols><col min=\"2\" max=\"4\" width=\"20\" hidden=\"1\" customWidth=\"1\"/></cols>",
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>\
         <row r=\"3\" ht=\"30\" hidden=\"1\" customHeight=\"1\"/>\
         <row r=\"9\" s=\"1\" customFormat=\"1\" outlineLevel=\"2\" collapsed=\"1\" x14ac:dyDescent=\"0.25\"/>",
        "<mergeCells count=\"1\"><mergeCell ref=\"B5:C6\"/></mergeCells>",
    );
    let mut workbook = Workbook::from_bytes(book_with(&original)).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("B5".parse().unwrap(), 2.0)
        .unwrap();
    let written = workbook.into_bytes().unwrap();
    let text = member(&written, "xl/worksheets/sheet1.xml");
    for kept in [
        "<cols><col min=\"2\" max=\"4\" width=\"20\" hidden=\"1\" customWidth=\"1\"/></cols>",
        "<row r=\"3\" ht=\"30\" hidden=\"1\" customHeight=\"1\"/>",
        "<row r=\"5\"><c r=\"B5\"><v>2</v></c></row>",
        "<row r=\"9\" s=\"1\" customFormat=\"1\" outlineLevel=\"2\" collapsed=\"1\" x14ac:dyDescent=\"0.25\"/>",
        "<mergeCells count=\"1\"><mergeCell ref=\"B5:C6\"/></mergeCells>",
    ] {
        assert!(text.contains(kept), "{kept} in {text}");
    }
    let reopened = Workbook::from_bytes(written).unwrap();
    let sheet = reopened.sheet("Sheet1").unwrap();
    assert_eq!(sheet.row_height(2), 30.0);
    assert!(sheet.is_row_hidden(2));
    assert!(sheet.is_column_hidden(3));
    assert_eq!(sheet.column_width(1), 20.0);
    assert_eq!(sheet.row_style(8), Some(StyleId::new(1)));
    assert_eq!(sheet.merges().count(), 1);
}

#[test]
fn inserted_bands_inherit_dimensions_and_outline_without_hidden_or_collapsed_state() {
    // Excel 16.0 build 20430, twelve saved insertion cases: the preceding
    // band supplies size and outline; the first band uses sheet defaults.
    // Hidden/collapsed state does not propagate to the inserted band.
    let original = part(
        "<sheetFormatPr defaultColWidth=\"9.140625\" defaultRowHeight=\"15\"/>\
         <cols><col min=\"1\" max=\"1\" width=\"8.77734375\" hidden=\"1\" customWidth=\"1\" outlineLevel=\"1\"/>\
         <col min=\"2\" max=\"2\" width=\"20.77734375\" customWidth=\"1\" collapsed=\"1\"/></cols>",
        "<row r=\"1\" ht=\"18\" hidden=\"1\" customHeight=\"1\" outlineLevel=\"1\"/>\
         <row r=\"2\" ht=\"24\" customHeight=\"1\" collapsed=\"1\"/>",
        "",
    );
    for columns in [false, true] {
        for at in 0..3 {
            for count in 1..3 {
                let mut book = Workbook::from_bytes(book_with(&original)).unwrap();
                if columns {
                    book.insert_columns("Sheet1", at, count).unwrap();
                } else {
                    book.insert_rows("Sheet1", at, count).unwrap();
                }
                let bytes = book.into_bytes().unwrap();
                let reopened = Workbook::from_bytes(bytes.clone()).unwrap();
                let sheet = reopened.sheet("Sheet1").unwrap();
                for index in at..at + count {
                    if columns {
                        assert_eq!(
                            sheet.column_width(index),
                            [9.140625, 8.77734375, 20.77734375][at as usize]
                        );
                        assert!(!sheet.is_column_hidden(index));
                    } else {
                        assert_eq!(sheet.row_height(index), [15.0, 18.0, 24.0][at as usize]);
                        assert!(!sheet.is_row_hidden(index));
                    }
                }
                if at > 0 {
                    let xml = member(&bytes, "xl/worksheets/sheet1.xml");
                    let mut reader = quick_xml::Reader::from_str(&xml);
                    loop {
                        use quick_xml::events::Event;
                        match reader.read_event().unwrap() {
                            Event::Start(tag) | Event::Empty(tag)
                                if tag.name().as_ref() == if columns { b"col" } else { b"row" } =>
                            {
                                let key: &[u8] = if columns { b"min" } else { b"r" };
                                let index = tag.try_get_attribute(key).unwrap().unwrap();
                                if index.value.as_ref() == (at + 1).to_string().as_bytes() {
                                    let outline = tag.try_get_attribute(b"outlineLevel").unwrap();
                                    assert_eq!(
                                        outline.as_ref().map(|value| value.value.as_ref()),
                                        (at == 1).then_some(b"1".as_slice())
                                    );
                                    assert!(tag.try_get_attribute(b"collapsed").unwrap().is_none());
                                    break;
                                }
                            }
                            Event::Eof => panic!("inserted band missing: {xml}"),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_row_format_moves_with_the_rows_below_an_insertion_and_goes_with_a_removed_row() {
    let mut workbook = Workbook::from_bytes(book_with(&part(
        "",
        "<row r=\"2\" ht=\"25\" customHeight=\"1\"/><row r=\"4\" hidden=\"1\"><c r=\"A4\"><v>1</v></c></row>",
        "",
    )))
    .unwrap();
    workbook.insert_rows("Sheet1", 0, 2).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.row_height(3), 25.0);
    assert!(sheet.is_row_hidden(5));
    assert!(!sheet.is_row_hidden(3));
    workbook.remove_rows("Sheet1", 3..4).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.row_height(3), sheet.default_row_height());
    assert!(sheet.is_row_hidden(4));
}

#[test]
fn merges_and_the_frozen_split_move_with_the_rows_as_excel_moves_them() {
    let book = || {
        Workbook::from_bytes(book_with(&part(
            "<sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"2\" topLeftCell=\"A3\" \
             activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews>",
            "",
            "<mergeCells count=\"4\"><mergeCell ref=\"A1:B1\"/><mergeCell ref=\"A3:A6\"/>\
             <mergeCell ref=\"C8:D9\"/><mergeCell ref=\"E4:E5\"/></mergeCells>",
        )))
        .unwrap()
    };
    let merges = |workbook: &Workbook| {
        workbook
            .sheet("Sheet1")
            .unwrap()
            .merges()
            .map(|range| range.to_string())
            .collect::<Vec<_>>()
    };
    let frozen = |workbook: &Workbook| workbook.sheet("Sheet1").unwrap().frozen();
    // A sheet holding no cell still moves what stands on its rows.
    let mut inserted = book();
    inserted.insert_rows("Sheet1", 3, 2).unwrap();
    assert_eq!(merges(&inserted), ["A1:B1", "A3:A8", "C10:D11", "E6:E7"]);
    assert_eq!(
        frozen(&inserted),
        Some(Frozen {
            rows: 2,
            columns: 0
        })
    );
    // Inside the frozen rows, the split moves down with them.
    inserted.insert_rows("Sheet1", 0, 1).unwrap();
    assert_eq!(merges(&inserted), ["A2:B2", "A4:A9", "C11:D12", "E7:E8"]);
    assert_eq!(frozen(&inserted).map(|frozen| frozen.rows), Some(3));

    let mut removed = book();
    // Rows 4 and 5: a merge across them loses them, one inside them goes,
    // one below moves up.
    removed.remove_rows("Sheet1", 3..5).unwrap();
    assert_eq!(merges(&removed), ["A1:B1", "A3:A4", "C6:D7"]);
    assert_eq!(frozen(&removed).map(|frozen| frozen.rows), Some(2));
    // Row 1: the merge on it goes, and the split loses a row.
    removed.remove_rows("Sheet1", 0..1).unwrap();
    assert_eq!(merges(&removed), ["A2:A3", "C5:D6"]);
    assert_eq!(frozen(&removed).map(|frozen| frozen.rows), Some(1));
    // A merge left one cell is none, and a split with nothing frozen goes.
    removed.remove_rows("Sheet1", 0..2).unwrap();
    assert_eq!(merges(&removed), ["C3:D4"]);
    assert_eq!(frozen(&removed), None);

    // Columns move the same way.
    let mut columns = book();
    columns.insert_columns("Sheet1", 1, 2).unwrap();
    assert_eq!(merges(&columns), ["A1:D1", "A3:A6", "E8:F9", "G4:G5"]);
    columns.remove_columns("Sheet1", 0..1).unwrap();
    assert_eq!(merges(&columns), ["A1:C1", "D8:E9", "F4:F5"]);

    let written = member(&inserted.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(
        written.contains(
            "<mergeCells count=\"4\"><mergeCell ref=\"A2:B2\"/><mergeCell ref=\"A4:A9\"/>\
             <mergeCell ref=\"C11:D12\"/><mergeCell ref=\"E7:E8\"/></mergeCells>"
        ),
        "{written}"
    );
    assert!(written.contains("ySplit=\"3\""), "{written}");
}

#[test]
fn a_layout_fact_the_schema_does_not_spell_is_refused_naming_the_sheet_and_the_element() {
    for (before, rows, after, expected) in [
        (
            "<cols><col min=\"3\" max=\"2\" width=\"9\"/></cols>",
            "",
            "",
            "Sheet1!col",
        ),
        (
            "<cols><col min=\"1\" max=\"3\"/><col min=\"2\" max=\"4\"/></cols>",
            "",
            "",
            "expected column spans in ascending order, none overlapping, got 2 to 4",
        ),
        (
            "<cols><col min=\"1\" max=\"16385\"/></cols>",
            "",
            "",
            "expected columns from 1 to 16384",
        ),
        (
            "<cols><col min=\"1\" max=\"1\" width=\"wide\"/></cols>",
            "",
            "",
            "expected a number of at least 0 for width, got \"wide\"",
        ),
        (
            "",
            "",
            "<mergeCells count=\"1\"><mergeCell ref=\"nowhere\"/></mergeCells>",
            "Sheet1!mergeCell",
        ),
        (
            "<sheetFormatPr defaultRowHeight=\"tall\"/>",
            "",
            "",
            "Sheet1!sheetFormatPr",
        ),
        ("", "<row r=\"2\" ht=\"-1\"/>", "", "Sheet1!A2"),
        (
            "",
            "<row r=\"2\" outlineLevel=\"8\"/>",
            "",
            "expected an outline level from 0 to 7, got \"8\"",
        ),
        (
            "",
            "<row r=\"2\" hidden=\"maybe\"/>",
            "",
            "expected 1, true, 0 or false for hidden, got \"maybe\"",
        ),
    ] {
        let error = refusal(&part(before, rows, after));
        assert!(error.contains(expected), "{expected} in {error}");
    }
}

#[test]
fn frozen_names_its_first_scrolling_cell_and_its_pane() {
    for (frozen, top_left) in [
        (
            Frozen {
                rows: 1,
                columns: 0,
            },
            "A2",
        ),
        (
            Frozen {
                rows: 0,
                columns: 2,
            },
            "C1",
        ),
        (
            Frozen {
                rows: 3,
                columns: 1,
            },
            "B4",
        ),
    ] {
        assert_eq!(frozen.top_left(), top_left.parse::<CellRef>().unwrap());
    }
}

#[test]
fn a_frozen_pane_set_counts_as_a_change_and_an_empty_one_as_none() {
    let mut sheet = Sheet::new("Pane").unwrap();
    let before = sheet.revision();
    sheet
        .set_frozen(Some(Frozen {
            rows: 0,
            columns: 0,
        }))
        .unwrap();
    assert_eq!(sheet.frozen(), None);
    assert_eq!(sheet.revision(), before);
    let pane = Some(Frozen {
        rows: 2,
        columns: 1,
    });
    sheet.set_frozen(pane).unwrap();
    assert_eq!(sheet.frozen(), pane);
    assert!(sheet.revision() > before);
    // Setting what is already set changes nothing.
    let after = sheet.revision();
    sheet.set_frozen(pane).unwrap();
    assert_eq!(sheet.revision(), after);
    // A pane freezing every row leaves nothing to scroll.
    assert_eq!(
        sheet
            .set_frozen(Some(Frozen {
                rows: 1_048_576,
                columns: 0,
            }))
            .unwrap_err()
            .to_string(),
        "invalid record value at Pane!pane: expected fewer than 1048576 rows and 16384 columns \
         frozen, got 1048576 rows and 0 columns"
    );
    assert_eq!(sheet.frozen(), pane);
    // A frozen pane written reads back.
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.sheet("Pane").unwrap().frozen(), pane);
}
