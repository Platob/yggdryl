//! Mirrored tests for rust/src/excel/pivot/layout.rs.

use serde_json::Value;
use yggdryl::excel::{
    Aggregate, AxisField, CellRef, ItemOrder, PivotSource, PivotSpec, ValueField,
};
use yggdryl::internals::excel_pivot_layout::geometry;

fn spec(row_fields: usize, column_fields: usize, value_fields: usize) -> PivotSpec {
    let axis = |index: usize| AxisField {
        field: format!("Field{index}").into(),
        order: ItemOrder::Ascending,
    };
    PivotSpec {
        name: "P".into(),
        source: PivotSource {
            sheet: "Data".into(),
            range: "A1:F9".parse().unwrap(),
        },
        rows: (0..row_fields).map(axis).collect(),
        columns: (row_fields..row_fields + column_fields).map(axis).collect(),
        values: (0..value_fields)
            .map(|index| ValueField {
                field: format!("Value{index}").into(),
                aggregate: Aggregate::Sum,
                caption: None,
                number_format: None,
            })
            .collect(),
        subtotals: true,
        row_grand_totals: true,
        column_grand_totals: true,
    }
}

#[test]
fn pivot_layout_geometry_matches_five_native_tables() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/pivot_native.json")).unwrap();
    assert_eq!(fixture["native"]["refresh_save_reopen_equal"], true);
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let r = case["row_fields_com"].as_array().unwrap().len();
        // Excel's ColumnFields includes the implicit Values field (-2).
        // Geometry adds that level from the value count, exactly once.
        let c = case["column_fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|field| field.as_i64().unwrap() != -2)
            .count();
        let v = case["data_fields_com"].as_array().unwrap().len();
        let grid = case["value2"].as_array().unwrap();
        let native_width = grid[0].as_array().unwrap().len();
        let native_header = case["location"]["firstDataRow"]
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let rendered_rows = u32::try_from(grid.len() - native_header).unwrap();
        let column_tuples = if c == 0 {
            0
        } else {
            u32::try_from((native_width - r) / v - 1).unwrap()
        };
        let output = geometry(
            &spec(r, c, v),
            "A3".parse().unwrap(),
            rendered_rows,
            column_tuples,
        )
        .unwrap();
        assert_eq!(
            output.range.to_string(),
            case["location"]["ref"].as_str().unwrap(),
            "{id}"
        );
        assert_eq!(output.width as usize, native_width, "{id}");
        assert_eq!(output.height as usize, grid.len(), "{id}");
        assert_eq!(
            output.first_header_row.to_string(),
            case["location"]["firstHeaderRow"].as_str().unwrap(),
            "{id}"
        );
        assert_eq!(
            output.first_data_row.to_string(),
            case["location"]["firstDataRow"].as_str().unwrap(),
            "{id}"
        );
        assert_eq!(
            output.first_data_col.to_string(),
            case["location"]["firstDataCol"].as_str().unwrap(),
            "{id}"
        );
    }
}

#[test]
fn pivot_layout_refuses_overflow_before_allocating_output() {
    let one = spec(1, 1, 1);
    assert!(geometry(&one, CellRef::new(1_048_575, 0), 2, 1).is_err());
    assert!(geometry(&one, CellRef::new(0, 16_383), 1, 1).is_err());
    assert!(geometry(&one, CellRef::new(0, 0), 1_048_575, 2).is_err());
}

#[test]
fn pivot_layout_counts_native_two_axis_subtotals_as_column_groups() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/pivot_layout_c2v3_native.json")).unwrap();
    assert_eq!(fixture["refresh_save_reopen_equal"], true);
    let values = fixture["data_fields"].as_array().unwrap().len();
    assert_eq!(values, 3);
    let classes = &fixture["column_item_classes"];
    let leaves = classes["item"].as_u64().unwrap();
    let subtotals = classes["default"].as_u64().unwrap();
    let grand = classes["grand"].as_u64().unwrap();
    assert_eq!((leaves, subtotals, grand), (15, 9, 3));
    let rendered_groups = u32::try_from((leaves + subtotals) / values as u64).unwrap();
    assert_eq!(rendered_groups, 8);
    let output = geometry(
        &spec(1, 2, values),
        "A3".parse().unwrap(),
        4,
        rendered_groups,
    )
    .unwrap();
    assert_eq!(
        output.range.to_string(),
        fixture["location"]["ref"].as_str().unwrap()
    );
    assert_eq!(output.width, 28);
    assert_eq!(output.height, 8);
    assert_eq!(
        output.first_header_row.to_string(),
        fixture["location"]["firstHeaderRow"].as_str().unwrap()
    );
    assert_eq!(
        output.first_data_row.to_string(),
        fixture["location"]["firstDataRow"].as_str().unwrap()
    );
    assert_eq!(
        output.first_data_col.to_string(),
        fixture["location"]["firstDataCol"].as_str().unwrap()
    );
}
