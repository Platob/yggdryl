//! `rust/src/excel/pivot/part.rs`: one rendered vertical pivot's complete
//! OOXML membership and empty refresh-on-load cache contract.

use smol_str::SmolStr;
use yggdryl::excel::{
    Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
};

use crate::excel_package::member;

#[test]
fn pivot_part_vertical_writer_registers_empty_cache_and_relationships() {
    let mut book = Workbook::new();
    let data = book.add_sheet("Data").unwrap();
    for (at, text) in [
        ("A1", "Group"),
        ("B1", "Value"),
        ("A2", "East"),
        ("A3", "West"),
    ] {
        data.set_cell(at.parse().unwrap(), text).unwrap();
    }
    for (at, value) in [("B2", 1.0), ("B3", 2.0)] {
        data.set_cell(at.parse().unwrap(), value).unwrap();
    }
    book.add_sheet("Report").unwrap();
    let spec = PivotSpec {
        name: SmolStr::new_static("P6_Small"),
        source: PivotSource {
            sheet: "Data".into(),
            range: "A1:B3".parse().unwrap(),
        },
        rows: vec![AxisField {
            field: "Group".into(),
            order: ItemOrder::Ascending,
        }],
        columns: Vec::new(),
        values: vec![ValueField {
            field: "Value".into(),
            aggregate: Aggregate::Sum,
            caption: Some("Value Sum".into()),
            number_format: None,
        }],
        subtotals: false,
        row_grand_totals: false,
        column_grand_totals: true,
    };
    book.add_pivot(spec, "Report", "A3".parse().unwrap())
        .unwrap();
    let saved = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    let cache = member(&saved, "xl/pivotCache/pivotCacheDefinition1.xml");
    let records = member(&saved, "xl/pivotCache/pivotCacheRecords1.xml");
    let table = member(&saved, "xl/pivotTables/pivotTable1.xml");
    let host_rels = member(&saved, "xl/worksheets/_rels/sheet2.xml.rels");
    let cache_rels = member(&saved, "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels");
    let workbook = member(&saved, "xl/workbook.xml");
    let types = member(&saved, "[Content_Types].xml");
    assert!(cache.contains("recordCount=\"0\""));
    assert!(cache.contains("saveData=\"0\""));
    assert!(cache.contains("refreshOnLoad=\"1\""));
    assert!(records.contains("count=\"0\""));
    assert!(table.contains("name=\"P6_Small\""));
    assert!(table.contains("ref=\"A3:B6\""));
    assert!(host_rels.contains("/pivotTable\""));
    assert!(cache_rels.contains("/pivotCacheRecords\""));
    assert!(workbook.contains("<pivotCaches>"));
    assert!(types.contains("pivotCacheDefinition+xml"));
}

#[test]
fn pivot_value_number_format_shares_one_custom_id_across_values_and_result_cells() {
    let mut book = Workbook::new();
    let source = book.add_sheet("Data").unwrap();
    for (at, text) in [("A1", "Group"), ("B1", "Value"), ("A2", "East")] {
        source.set_cell(at.parse().unwrap(), text).unwrap();
    }
    source.set_cell("B2".parse().unwrap(), 12.5).unwrap();
    book.add_sheet("Report").unwrap();
    let value = ValueField {
        field: "Value".into(),
        aggregate: Aggregate::Sum,
        caption: Some("Total".into()),
        number_format: Some("0.00000".into()),
    };
    let mut second = value.clone();
    second.caption = Some("Count".into());
    second.aggregate = Aggregate::Count;
    let spec = PivotSpec {
        name: "Formats".into(),
        source: PivotSource {
            sheet: "Data".into(),
            range: "A1:B2".parse().unwrap(),
        },
        rows: vec![AxisField {
            field: "Group".into(),
            order: ItemOrder::Ascending,
        }],
        columns: Vec::new(),
        values: vec![value, second],
        subtotals: false,
        row_grand_totals: false,
        column_grand_totals: false,
    };
    let before = book.style_sheet().unwrap().len();
    book.add_pivot(spec, "Report", "A1".parse().unwrap())
        .unwrap();
    assert_eq!(book.style_sheet().unwrap().len(), before + 1);
    let sheet = book.sheet("Report").unwrap();
    assert_eq!(
        sheet.cell("B2".parse().unwrap()).unwrap().style(),
        sheet.cell("C2".parse().unwrap()).unwrap().style()
    );
    let saved = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    let table = member(&saved, "xl/pivotTables/pivotTable1.xml");
    let styles = member(&saved, "xl/styles.xml");
    assert_eq!(table.matches("numFmtId=\"164\"").count(), 2, "{table}");
    assert_eq!(
        styles.matches("formatCode=\"0.00000\"").count(),
        1,
        "{styles}"
    );
}
