//! `rust/src/excel/regions.rs`: named-table ownership and bounded sparse worksheet region discovery.

use std::collections::BTreeMap;
use std::fmt::Write;

use yggdryl::MimeType;
use yggdryl::excel::{CellRange, CellRef, ExcelRegion, ExcelRegionKind, regions};
use yggdryl::holder::Buffer;

use crate::excel_package::{named_table_package, named_table_parts, one_sheet, package, worksheet};

fn xlsx(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into())
}

fn expected(sheet: &str, range: &str, table: Option<&str>) -> ExcelRegion {
    ExcelRegion {
        sheet: sheet.into(),
        range: range.parse().unwrap(),
        kind: table.map_or(ExcelRegionKind::Suggested, |name| ExcelRegionKind::Table {
            name: name.into(),
        }),
    }
}

/// Cells sorted exactly as the worksheet grammar requires; fixture-only geometry.
fn rectangles(ranges: &[&str]) -> String {
    let mut rows: BTreeMap<u32, BTreeMap<u32, ()>> = BTreeMap::new();
    for range in ranges {
        let range: CellRange = range.parse().unwrap();
        for row in range.start().row()..=range.end().row() {
            for column in range.start().column()..=range.end().column() {
                rows.entry(row).or_default().insert(column, ());
            }
        }
    }
    let mut xml = String::new();
    for (row, columns) in rows {
        write!(xml, "<row r=\"{}\">", row + 1).unwrap();
        for column in columns.keys() {
            write!(xml, "<c r=\"{}\"><v>1</v></c>", CellRef::new(row, *column)).unwrap();
        }
        xml.push_str("</row>");
    }
    xml
}

#[test]
fn regions_three_separate_blocks_use_cells_not_stated_dimensions() {
    let xml = worksheet(&rectangles(&["A1:C5", "D8:E10", "C13:F16"]))
        .replace("<sheetData>", "<dimension ref=\"A1\"/><sheetData>");
    let parts = [
        (
            "[Content_Types].xml",
            crate::excel_package::content_types(1, false, false),
        ),
        ("_rels/.rels", crate::excel_package::root_relationships()),
        (
            "xl/workbook.xml",
            crate::excel_package::workbook(&["Sheet1"], false),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            crate::excel_package::workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", xml),
    ];
    let handle = xlsx(package(
        &parts
            .iter()
            .map(|(name, text)| (*name, text.as_str()))
            .collect::<Vec<_>>(),
    ));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Sheet1", "A1:C5", None),
            expected("Sheet1", "D8:E10", None),
            expected("Sheet1", "C13:F16", None),
        ]
    );
    assert_eq!(
        regions(&handle, Some("sheet1")).unwrap(),
        regions(&handle, None).unwrap()
    );
}

#[test]
fn regions_ignore_foreign_ignorable_rows_inside_sheet_data() {
    let data = "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>\
                <q:row r=\"2\"><q:c r=\"Z2\"><q:v>999</q:v></q:c></q:row>";
    let xml = worksheet(data).replace(
        "<worksheet ",
        "<worksheet xmlns:q=\"urn:custom\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"q\" ",
    );
    let parts = [
        (
            "[Content_Types].xml",
            crate::excel_package::content_types(1, false, false),
        ),
        ("_rels/.rels", crate::excel_package::root_relationships()),
        (
            "xl/workbook.xml",
            crate::excel_package::workbook(&["Sheet1"], false),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            crate::excel_package::workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", xml),
    ];
    let handle = xlsx(package(
        &parts
            .iter()
            .map(|(name, text)| (*name, text.as_str()))
            .collect::<Vec<_>>(),
    ));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [expected("Sheet1", "A1", None)]
    );
}

#[test]
fn regions_resolve_encoded_prefixed_scope_and_ignore_shadowed_or_other_family_rows() {
    let namespace = yggdryl::excel::NAMESPACE.replace("http", "h&#116;tp");
    let xml = format!(
        "<s:worksheet xmlns:s=\"{namespace}\" xmlns:q=\"urn:foreign\" xmlns:t=\"{}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"q t\">\
         <s:sheetData><s:row r=\"1\"><s:c r=\"A1\"><s:v>1</s:v></s:c></s:row>\
         <q:row xmlns:q=\"{namespace}\" r=\"2\"><q:c r=\"C2\"><q:v>2</q:v></q:c></q:row>\
         <t:row r=\"3\"><t:c r=\"Z3\"><t:v>888</t:v></t:c></t:row>\
         </s:sheetData></s:worksheet>",
        yggdryl::excel::STRICT_NAMESPACE,
    );
    let parts = [
        (
            "[Content_Types].xml",
            crate::excel_package::content_types(1, false, false),
        ),
        ("_rels/.rels", crate::excel_package::root_relationships()),
        (
            "xl/workbook.xml",
            crate::excel_package::workbook(&["Sheet1"], false),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            crate::excel_package::workbook_relationships(1, false, false),
        ),
        ("xl/worksheets/sheet1.xml", xml),
    ];
    let handle = xlsx(package(
        &parts
            .iter()
            .map(|(name, text)| (*name, text.as_str()))
            .collect::<Vec<_>>(),
    ));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Sheet1", "A1", None),
            expected("Sheet1", "C2", None)
        ]
    );
}

#[test]
fn regions_refuse_an_unbound_prefix_and_do_not_enter_nested_sheet_data() {
    let unbound = worksheet("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><q:row r=\"2\"/>");
    let nested = worksheet("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>")
        .replace("<worksheet ", "<worksheet xmlns:q=\"urn:foreign\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"q\" ")
        .replace("<sheetData>",
                 "<q:wrapper><sheetData><row r=\"2\"><c r=\"Z2\"><v>999</v></c></row></sheetData></q:wrapper><sheetData>");
    for (xml, refusal) in [(unbound, true), (nested, false)] {
        let parts = [
            (
                "[Content_Types].xml",
                crate::excel_package::content_types(1, false, false),
            ),
            ("_rels/.rels", crate::excel_package::root_relationships()),
            (
                "xl/workbook.xml",
                crate::excel_package::workbook(&["Sheet1"], false),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                crate::excel_package::workbook_relationships(1, false, false),
            ),
            ("xl/worksheets/sheet1.xml", xml),
        ];
        let handle = xlsx(package(
            &parts
                .iter()
                .map(|(name, text)| (*name, text.as_str()))
                .collect::<Vec<_>>(),
        ));
        if refusal {
            let message = regions(&handle, None).unwrap_err().to_string();
            assert!(
                message.contains("Sheet1") && message.contains("q"),
                "{message}"
            );
        } else {
            assert_eq!(
                regions(&handle, None).unwrap(),
                [expected("Sheet1", "A1", None)]
            );
        }
    }
}

#[test]
fn regions_ignore_foreign_ignorable_attributes_that_shadow_row_cell_and_formula_facts() {
    for data in [
        "<row r=\"1\"><c q:r=\"Z1\" r=\"A1\"><v>7</v></c></row>",
        "<row q:r=\"999\" r=\"1\"><c r=\"A1\"><v>7</v></c></row>",
        "<row r=\"1\"><c r=\"A1\"><f q:t=\"nonsense\">1+1</f></c></row>",
    ] {
        let xml = worksheet(data).replace("<worksheet ",
            "<worksheet xmlns:q=\"urn:foreign\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"q\" ");
        let parts = [
            (
                "[Content_Types].xml",
                crate::excel_package::content_types(1, false, false),
            ),
            ("_rels/.rels", crate::excel_package::root_relationships()),
            (
                "xl/workbook.xml",
                crate::excel_package::workbook(&["Sheet1"], false),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                crate::excel_package::workbook_relationships(1, false, false),
            ),
            ("xl/worksheets/sheet1.xml", xml),
        ];
        let handle = xlsx(package(
            &parts
                .iter()
                .map(|(name, text)| (*name, text.as_str()))
                .collect::<Vec<_>>(),
        ));
        assert_eq!(
            regions(&handle, None).unwrap(),
            [expected("Sheet1", "A1", None)],
            "{data}"
        );
    }
}

#[test]
fn regions_join_diagonal_runs_and_late_bridges_then_retire_across_blank_rows() {
    let data = rectangles(&["A1", "C1", "B2", "E2:F4", "A5:B6", "C7"]);
    let handle = xlsx(one_sheet(&data, &[], &[], &[]));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Sheet1", "A1:C2", None),
            expected("Sheet1", "E2:F4", None),
            expected("Sheet1", "A5:C7", None),
        ]
    );
}

#[test]
fn regions_occupancy_distinguishes_formulas_empty_text_whitespace_and_styles() {
    let data = "<row r=\"1\"><c r=\"A1\" s=\"0\"/><c r=\"C1\" t=\"inlineStr\"><is><t></t></is></c><c r=\"E1\" t=\"inlineStr\"><is><t xml:space=\"preserve\"> </t></is></c><c r=\"G1\"><f>1+1</f></c><c r=\"I1\" t=\"s\"><v>0</v></c><c r=\"K1\" t=\"s\"><v>1</v></c></row><row r=\"3\"><c r=\"M3\"><v> </v></c><c r=\"O3\" t=\"e\"><v>#VALUE!</v></c></row>";
    let handle = xlsx(one_sheet(data, &["", " "], &[], &[]));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Sheet1", "E1", None),
            expected("Sheet1", "G1", None),
            expected("Sheet1", "K1", None),
            expected("Sheet1", "O3", None),
        ]
    );
}

#[test]
fn regions_named_extents_exclude_their_cells_and_do_not_absorb_touching_notes() {
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace("<c r=\"D2\">", "<c r=\"C2\"><v>9</v></c><c r=\"D2\">");
    let other = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet2.xml")
        .unwrap()
        .1;
    *other = worksheet(&rectangles(&["A1:B2"]));
    let handle = xlsx(named_table_package(&parts));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Data", "A1:B3", Some("Names")),
            expected("Data", "D1:E4", Some("Quantities")),
            expected("Data", "C2", None),
            expected("Other", "A1:B2", None),
        ]
    );
    assert_eq!(
        regions(&handle, Some("other")).unwrap(),
        [expected("Other", "A1:B2", None)]
    );
}

#[test]
fn regions_empty_sheet_returns_no_suggestions_and_missing_sheet_is_located() {
    assert!(regions(&Buffer::new(), None).unwrap().is_empty());
    assert!(matches!(regions(&Buffer::new(), Some("Missing")),
        Err(yggdryl::Error::InvalidRecord { path, .. }) if path == "$.sheet"));
    let handle = xlsx(one_sheet(
        "<row r=\"9\"><c r=\"XFD9\" s=\"0\"/></row>",
        &[],
        &[],
        &[],
    ));
    assert!(regions(&handle, None).unwrap().is_empty());
    let error = regions(&handle, Some("Missing")).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("{error}")
    };
    assert_eq!(path.as_str(), "$.sheet");
    assert!(
        reason.contains("Missing") && reason.contains("worksheet"),
        "{reason}"
    );
}

#[test]
fn regions_candidate_cap_refuses_instead_of_truncating() {
    for count in [1024_u32, 1025] {
        let mut data = String::new();
        for index in 0..count {
            let row = index * 2 + 1;
            write!(data, "<row r=\"{row}\"><c r=\"A{row}\"><v>1</v></c></row>").unwrap();
        }
        let handle = xlsx(one_sheet(&data, &[], &[], &[]));
        let result = regions(&handle, None);
        if count == 1024 {
            assert_eq!(result.unwrap().len(), 1024);
        } else {
            let error = result.unwrap_err();
            let yggdryl::Error::InvalidRecord { path, reason } = error else {
                panic!("{error}")
            };
            assert_eq!(path.as_str(), "Sheet1!A2049");
            assert!(
                reason.contains("1024") && reason.contains("regions"),
                "{reason}"
            );
        }
    }
}

#[test]
fn regions_candidate_cap_counts_finished_regions_not_live_fragments() {
    let mut cells = vec!["A2:DCC2".to_owned()];
    for column in (0..2049).step_by(2) {
        cells.push(CellRef::new(0, column).to_string());
    }
    let ranges: Vec<_> = cells.iter().map(String::as_str).collect();
    let handle = xlsx(one_sheet(&rectangles(&ranges), &[], &[], &[]));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [expected("Sheet1", "A1:DCC2", None)]
    );
}

#[test]
fn regions_do_not_hide_invalid_registered_table_ownership() {
    let mut parts = named_table_parts();
    let relationships = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/_rels/sheet1.xml.rels")
        .unwrap()
        .1;
    *relationships = relationships.replace("../tables/table1.xml", "../tables/missing.xml");
    let handle = xlsx(named_table_package(&parts));
    let error = regions(&handle, None).unwrap_err();
    assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
    let text = error.to_string();
    assert!(
        text.contains("sheet1.xml") && text.contains("rIdT1"),
        "{text}"
    );
}

#[test]
fn regions_keep_a_split_live_component_when_its_other_branch_retires() {
    // B1 reaches both A2 and C2 diagonally. Only C2 continues to C3;
    // A2 still belongs in the final bounds. H2 retires independently.
    let handle = xlsx(one_sheet(
        &rectangles(&["B1", "A2", "C2", "C3", "H2"]),
        &[],
        &[],
        &[],
    ));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Sheet1", "A1:C3", None),
            expected("Sheet1", "H2", None),
        ]
    );
}

#[test]
fn regions_exclude_two_table_intervals_without_joining_their_flanking_cells() {
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet
        .replace("<c r=\"D2\">", "<c r=\"C2\"><v>9</v></c><c r=\"D2\">")
        .replace(
            "<c r=\"E2\"><v>3</v></c>",
            "<c r=\"E2\"><v>3</v></c><c r=\"F2\"><v>8</v></c>",
        );
    let handle = xlsx(named_table_package(&parts));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Data", "A1:B3", Some("Names")),
            expected("Data", "D1:E4", Some("Quantities")),
            expected("Data", "C2", None),
            expected("Data", "F2", None),
        ]
    );
}

#[test]
fn regions_report_component_bounds_even_when_they_enclose_an_excluded_table() {
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    let ring = rectangles(&["C1:F1", "C2:C5", "F2:F5", "C5:F5"]);
    *sheet = crate::excel_package::sheet(
        &ring,
        "<tableParts count=\"2\"><tablePart r:id=\"rIdT1\"/><tablePart r:id=\"rIdT2\"/></tableParts>",
    );
    let handle = xlsx(named_table_package(&parts));
    assert_eq!(
        regions(&handle, None).unwrap(),
        [
            expected("Data", "A1:B3", Some("Names")),
            expected("Data", "C1:F5", None),
            expected("Data", "D1:E4", Some("Quantities")),
        ]
    );
}
