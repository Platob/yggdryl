//! Mirrored tests for rust/src/excel/pivot/compute.rs.

use yggdryl::excel::{
    Aggregate, AxisField, Cell, CellKind, CellRef, DateSystem, ExcelError, ItemOrder, NumberFormat,
    PivotSource, PivotSpec, ValueField, Workbook,
};
use yggdryl::internals::excel_pivot_compute::{
    BoundSource, PivotComputed, PivotItem, PivotItems, PivotMeasure,
};
use yggdryl::{Error, Scalar};

fn spec() -> PivotSpec {
    PivotSpec {
        name: "P".into(),
        source: PivotSource {
            sheet: "Data".into(),
            range: "A1:C4".parse().unwrap(),
        },
        rows: vec![AxisField {
            field: "Region".into(),
            order: ItemOrder::Ascending,
        }],
        columns: vec![AxisField {
            field: "Product".into(),
            order: ItemOrder::Ascending,
        }],
        values: vec![ValueField {
            field: "Sales".into(),
            aggregate: Aggregate::Sum,
            caption: None,
            number_format: None,
        }],
        subtotals: false,
        row_grand_totals: true,
        column_grand_totals: true,
    }
}

#[test]
fn pivot_grouping_replays_native_completed_item_grand_order() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/pivot_grand_native.json")).unwrap();
    assert_eq!(fixture["provenance"]["passed"], true);
    assert_eq!(fixture["provenance"]["cleanup_completed"], true);
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet.set_cell("A1".parse().unwrap(), "Region").unwrap();
    sheet.set_cell("B1".parse().unwrap(), "Sales").unwrap();
    for (row, region, amount) in [
        (2, "East", 1e16),
        (3, "West", 1.0),
        (4, "East", -1e16),
        (5, "West", 2.0),
    ] {
        sheet.set_cell(CellRef::new(row - 1, 0), region).unwrap();
        sheet.set_cell(CellRef::new(row - 1, 1), amount).unwrap();
    }
    let mut request = spec();
    request.source.range = "A1:B5".parse().unwrap();
    request.columns.clear();
    request.values[0].field = "Sales".into();
    let bound = BoundSource::bind(&request, sheet).unwrap();
    let computed = PivotComputed::build(&request, &bound, sheet).unwrap();
    assert_eq!(computed.row_tuples, [vec![0], vec![1]]);
    assert_eq!(computed.column_tuples, [Vec::<usize>::new()]);
    assert_eq!(
        computed.values.get(&(0, 0, 0)),
        Some(&PivotMeasure::Number(0.0))
    );
    assert_eq!(
        computed.values.get(&(1, 0, 0)),
        Some(&PivotMeasure::Number(3.0))
    );
    assert_eq!(computed.sum_grand_from_rows(0).unwrap(), Some(3.0));
}

#[test]
fn pivot_grouping_unifies_authored_blank_axis_and_supports_sample_variance() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (column, label) in ["Region", "Product", "Sales"].into_iter().enumerate() {
        sheet
            .set_cell(CellRef::new(0, column as u32), label)
            .unwrap();
    }
    sheet.set_cell("A2".parse().unwrap(), "East").unwrap();
    sheet.set_cell("B2".parse().unwrap(), "Apples").unwrap();
    sheet.set_cell("C2".parse().unwrap(), 2.0).unwrap();
    sheet.set_cell("B3".parse().unwrap(), "Apples").unwrap();
    sheet.set_cell("C3".parse().unwrap(), 3.0).unwrap();
    sheet.set_cell("B4".parse().unwrap(), "Apples").unwrap();
    sheet.set_cell("C4".parse().unwrap(), 4.0).unwrap();
    let request = spec();
    let bound = BoundSource::bind(&request, sheet).unwrap();
    let computed = PivotComputed::build(&request, &bound, sheet).unwrap();
    assert_eq!(computed.row_tuples, [vec![0], vec![1]]);
    assert_eq!(computed.column_tuples, [vec![0]]);
    assert_eq!(
        computed.values.get(&(0, 0, 0)),
        Some(&PivotMeasure::Number(2.0))
    );
    assert_eq!(
        computed.values.get(&(1, 0, 0)),
        Some(&PivotMeasure::Number(7.0))
    );

    let mut variance = request;
    variance.values[0].aggregate = Aggregate::Var;
    let computed = PivotComputed::build(&variance, &bound, sheet).unwrap();
    assert_eq!(
        computed.values.get(&(0, 0, 0)),
        Some(&PivotMeasure::Error(ExcelError::Div0))
    );
    assert_eq!(
        computed.values.get(&(1, 0, 0)),
        Some(&PivotMeasure::Number(0.5))
    );
    assert_eq!(computed.grand_rollups, [Some(PivotMeasure::Number(1.0))]);
}

#[test]
fn pivot_compute_binds_headers_once_to_physical_columns() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (column, label) in ["Region", "Product", "Sales"].into_iter().enumerate() {
        sheet
            .set_cell(yggdryl::excel::CellRef::new(0, column as u32), label)
            .unwrap();
    }
    sheet.set_cell("A2".parse().unwrap(), "East").unwrap();
    sheet.set_cell("B2".parse().unwrap(), "Apples").unwrap();
    sheet.set_cell("C2".parse().unwrap(), 13.0).unwrap();
    let bound = BoundSource::bind(&spec(), sheet).unwrap();
    assert_eq!(bound.rows, [0]);
    assert_eq!(bound.columns, [1]);
    assert_eq!(bound.values, [2]);
}

#[test]
fn pivot_compute_refuses_duplicate_and_missing_source_headers_at_intake() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (column, label) in ["Region", "region", "Sales"].into_iter().enumerate() {
        sheet
            .set_cell(yggdryl::excel::CellRef::new(0, column as u32), label)
            .unwrap();
    }
    assert!(matches!(
        BoundSource::bind(&spec(), sheet),
        Err(Error::InvalidRecord { .. })
    ));
    sheet.set_cell("B1".parse().unwrap(), "Product").unwrap();
    let mut missing = spec();
    missing.values[0].field = "Unknown".into();
    let error = BoundSource::bind(&missing, sheet).unwrap_err();
    match error {
        Error::InvalidRecord { path, .. } => assert_eq!(path, "$.values[0].field"),
        other => panic!("expected located missing-field refusal, got {other}"),
    }
}

#[test]
fn pivot_items_match_native_ascii_blank_number_boolean_and_error_identity() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/pivot_items_native.json")).unwrap();
    assert_eq!(fixture["all_passed"], true);
    let mut items = PivotItems::new(DateSystem::Year1900);
    let mut intern = |column: u32, value: Scalar| {
        let at = CellRef::new(1, column);
        let cell = Cell::from_scalar(at, value, DateSystem::Year1900).unwrap();
        items.intern(Some(&cell), None, at).unwrap()
    };
    assert_eq!(intern(0, Scalar::from("East")), 0);
    assert_eq!(intern(1, Scalar::from("east")), 0);
    assert_eq!(intern(2, Scalar::from("EAST")), 0);
    assert_eq!(intern(3, Scalar::from("West")), 1);
    assert_eq!(intern(4, Scalar::from(" East")), 2);
    assert_eq!(intern(5, Scalar::from("East ")), 3);
    assert_eq!(intern(6, Scalar::from(1)), 4);
    assert_eq!(intern(7, Scalar::from(1.0)), 4);
    assert_eq!(intern(8, Scalar::from("1")), 5);
    assert_eq!(intern(9, Scalar::from(true)), 6);
    assert_eq!(intern(10, Scalar::from("TRUE")), 7);
    drop(intern);
    assert_eq!(items.intern(None, None, CellRef::new(1, 11)).unwrap(), 8);
    let blank =
        Cell::from_scalar(CellRef::new(1, 12), Scalar::from(""), DateSystem::Year1900).unwrap();
    assert_eq!(
        items
            .intern(Some(&blank), None, CellRef::new(1, 12))
            .unwrap(),
        8
    );
    let error = Cell::from_scalar(CellRef::new(1, 13), Scalar::Null, DateSystem::Year1900)
        .unwrap()
        .with_error(ExcelError::NA);
    assert_eq!(
        items
            .intern(Some(&error), None, CellRef::new(1, 13))
            .unwrap(),
        9
    );
    assert!(matches!(items.items()[0], PivotItem::Text(ref value) if value == "East"));
    assert_eq!(items.items().len(), 10);
    let unsupported = Cell::from_scalar(
        CellRef::new(1, 14),
        Scalar::from("ÉTÉ"),
        DateSystem::Year1900,
    )
    .unwrap();
    assert!(
        matches!(items.intern(Some(&unsupported), None, CellRef::new(1, 14)), Err(Error::InvalidRecord { path, .. }) if path == "O2")
    );
}

#[test]
fn pivot_date_items_preserve_native_raw_serial_and_epoch_cache_encoding() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/pivot_date_native.json")).unwrap();
    assert_eq!(fixture["refresh_save_reopen_equal"], true);
    for observed in fixture["epochs"].as_array().unwrap() {
        let system = if observed["epoch"] == "1900" {
            DateSystem::Year1900
        } else {
            DateSystem::Year1904
        };
        let mut items = PivotItems::new(system);
        let cells = observed["source_group_cells"].as_array().unwrap();
        let expected = observed["cache_group_items"].as_array().unwrap();
        assert_eq!(cells.len(), 12);
        for (index, source) in cells.iter().enumerate() {
            let serial = source["serial"].as_str().unwrap().parse::<f64>().unwrap();
            let at: CellRef = source["ref"].as_str().unwrap().parse().unwrap();
            let styled = source["style"] == "1";
            let value = if styled {
                system
                    .scalar_from_serial(serial, NumberFormat::Date)
                    .unwrap()
            } else {
                Scalar::from(serial)
            };
            let cell = Cell::new(
                at,
                CellKind::Number,
                if styled {
                    NumberFormat::Date
                } else {
                    NumberFormat::General
                },
                value,
            );
            // The whole phantom day, including its clock, needs the original
            // worksheet serial, as retained by the sheet's cell extras.
            let retained =
                (system == DateSystem::Year1900 && styled && (60.0..61.0).contains(&serial))
                    .then_some(serial);
            assert_eq!(items.intern(Some(&cell), retained, at).unwrap(), index);
            assert_eq!(expected[index]["kind"], if styled { "d" } else { "n" });
            if styled {
                assert_eq!(
                    items.cache_date(index).unwrap(),
                    expected[index]["value"].as_str().unwrap()
                );
            }
        }
        assert_eq!(items.items().len(), 12);
    }
}

#[test]
fn pivot_compute_refuses_case_aliased_axes_after_physical_binding() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet.set_cell("A1".parse().unwrap(), "Region").unwrap();
    sheet.set_cell("B1".parse().unwrap(), "Value").unwrap();
    sheet.set_cell("A2".parse().unwrap(), "East").unwrap();
    sheet.set_cell("B2".parse().unwrap(), 1.0).unwrap();
    let mut direct = spec();
    direct.source.range = "A1:B2".parse().unwrap();
    direct.rows[0].field = "Region".into();
    direct.columns[0].field = "REGION".into();
    direct.values[0].field = "Value".into();
    let error = BoundSource::bind(&direct, sheet).unwrap_err();
    match error {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(path, "$.columns[0].field");
            assert!(reason.contains("$.rows[0].field"), "{reason}");
        }
        other => panic!("expected located axis collision, got {other}"),
    }
}

#[test]
fn pivot_compute_refuses_out_of_grid_source_before_dimensions() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (column, label) in ["Region", "Product", "Sales"].into_iter().enumerate() {
        sheet
            .set_cell(CellRef::new(0, column as u32), label)
            .unwrap();
    }
    for last in [CellRef::new(u32::MAX, 2), CellRef::new(1, u32::MAX)] {
        let mut request = spec();
        request.source.range = yggdryl::excel::CellRange::new(CellRef::new(0, 0), last);
        let error = BoundSource::bind(&request, sheet).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidRecord { path, .. } if path.as_str() == "$.source.range"
        ));
    }
}

#[test]
fn pivot_compute_refuses_rounded_extreme_integer_items() {
    let at = CellRef::new(1, 0);
    for value in [Scalar::from(i64::MAX), Scalar::from(u64::MAX)] {
        let cell = Cell::from_scalar(at, value, DateSystem::Year1900).unwrap();
        assert_eq!(cell.kind(), CellKind::Number);
        let mut items = PivotItems::new(DateSystem::Year1900);
        assert!(matches!(
            items.intern(Some(&cell), None, at),
            Err(Error::InvalidRecord { path, .. }) if path.as_str() == at.to_string()
        ));
    }
    for value in [
        Scalar::from(i64::MIN),
        Scalar::from(1u64 << 53),
        Scalar::from(1u64 << 60),
    ] {
        let cell = Cell::from_scalar(at, value, DateSystem::Year1900).unwrap();
        let mut items = PivotItems::new(DateSystem::Year1900);
        assert_eq!(items.intern(Some(&cell), None, at).unwrap(), 0);
    }
}

#[test]
fn pivot_parent_rollups_share_all_eleven_aggregate_modes() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (column, header) in ["Region", "Product", "Year", "Quarter", "Sales"]
        .into_iter()
        .enumerate()
    {
        sheet
            .set_cell(CellRef::new(0, column as u32), header)
            .unwrap();
    }
    for row in 1..=4 {
        for (column, value) in [
            (0, "East"),
            (1, if row <= 2 { "A" } else { "B" }),
            (2, "2024"),
            (3, "Q1"),
        ] {
            sheet.set_cell(CellRef::new(row, column), value).unwrap();
        }
        sheet.set_cell(CellRef::new(row, 4), 4.0).unwrap();
    }
    let axis = |field: &str| AxisField {
        field: field.into(),
        order: ItemOrder::Ascending,
    };
    let mut request = spec();
    request.source.range = "A1:E5".parse().unwrap();
    request.rows = vec![axis("Region"), axis("Product")];
    request.columns = vec![axis("Year"), axis("Quarter")];
    request.subtotals = true;
    let modes = [
        Aggregate::Sum,
        Aggregate::Average,
        Aggregate::Count,
        Aggregate::CountNumbers,
        Aggregate::Min,
        Aggregate::Max,
        Aggregate::Product,
        Aggregate::StdDev,
        Aggregate::StdDevP,
        Aggregate::Var,
        Aggregate::VarP,
    ];
    request.values = modes
        .into_iter()
        .map(|aggregate| ValueField {
            field: "Sales".into(),
            aggregate,
            caption: None,
            number_format: None,
        })
        .collect();
    let bound = BoundSource::bind(&request, sheet).unwrap();
    let computed = PivotComputed::build(&request, &bound, sheet).unwrap();
    let expected = [16.0, 4.0, 4.0, 4.0, 4.0, 4.0, 256.0, 0.0, 0.0, 0.0, 0.0];
    for (value, &all_rows) in expected.iter().enumerate().skip(1) {
        for (row_depth, column_depth) in [(1, 2), (2, 1), (1, 1)] {
            // Leaf A has two source records; row parent East has all four.
            let number = if row_depth == 2 {
                match modes[value] {
                    Aggregate::Count | Aggregate::CountNumbers => 2.0,
                    Aggregate::Product => 16.0,
                    _ => all_rows,
                }
            } else {
                all_rows
            };
            assert_eq!(
                computed
                    .parent_rollups
                    .get(&(row_depth, 0, column_depth, 0, value)),
                Some(&PivotMeasure::Number(number)),
                "mode {:?}, parent ({row_depth},{column_depth})",
                modes[value]
            );
        }
    }
    assert!(
        computed.parent_rollups.keys().all(|key| key.4 != 0),
        "SUM reuses its existing visible-order fold"
    );
}
