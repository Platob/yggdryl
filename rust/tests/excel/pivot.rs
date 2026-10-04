//! Mirrored tests for rust/src/excel/pivot.rs.
//! Source-sheet validation follows the typed request intake.

use yggdryl::excel::{Aggregate, PivotSpec};
use yggdryl::{Error, Scalar};

fn source() -> Scalar {
    Scalar::from_struct([
        ("sheet", Scalar::from("Data")),
        ("range", Scalar::from("A1:D6")),
    ])
    .unwrap()
}

fn axis(name: &str, order: &str) -> Scalar {
    Scalar::from_struct([
        ("field", Scalar::from(name)),
        ("order", Scalar::from(order)),
    ])
    .unwrap()
}

fn value(name: &str, aggregate: &str, caption: Scalar, format: Scalar) -> Scalar {
    Scalar::from_struct([
        ("field", Scalar::from(name)),
        ("aggregate", Scalar::from(aggregate)),
        ("caption", caption),
        ("numberFormat", format),
    ])
    .unwrap()
}

fn document(name: &str, rows: Vec<Scalar>, values: Vec<Scalar>) -> Scalar {
    Scalar::from_struct([
        ("name", Scalar::from(name)),
        ("source", source()),
        ("rows", Scalar::from_sequence(rows)),
        (
            "columns",
            Scalar::from_sequence([axis("Year", "descending")]),
        ),
        ("values", Scalar::from_sequence(values)),
        ("subtotals", Scalar::from(true)),
        ("rowGrandTotals", Scalar::from(false)),
        ("columnGrandTotals", Scalar::from(true)),
    ])
    .unwrap()
}

fn invalid_at(result: yggdryl::Result<PivotSpec>, expected: &str) {
    match result.unwrap_err() {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(path.as_str(), expected);
            assert!(
                reason.contains("expected") && reason.contains("got"),
                "{reason}"
            );
        }
        error => panic!("expected located InvalidRecord, got {error:?}"),
    }
}

#[test]
fn pivot_spec_named_document_roundtrips_all_aggregate_names() {
    let aggregates = [
        Aggregate::Sum,
        Aggregate::Count,
        Aggregate::Average,
        Aggregate::Max,
        Aggregate::Min,
        Aggregate::Product,
        Aggregate::CountNumbers,
        Aggregate::StdDev,
        Aggregate::StdDevP,
        Aggregate::Var,
        Aggregate::VarP,
    ];
    let values = aggregates
        .iter()
        .enumerate()
        .map(|(index, aggregate)| {
            value(
                "Sales",
                aggregate.as_str(),
                Scalar::from(format!("Metric{index}")),
                Scalar::Null,
            )
        })
        .collect();
    let document = document("PivotTable1", vec![axis("Region", "ascending")], values);
    let spec = PivotSpec::from_scalar(&document).unwrap();
    assert_eq!(spec.rows.len(), 1);
    assert_eq!(spec.columns.len(), 1);
    assert_eq!(
        spec.values
            .iter()
            .map(|field| field.aggregate)
            .collect::<Vec<_>>(),
        aggregates.to_vec()
    );
    assert_eq!(spec.source.range.to_string(), "A1:D6");
    assert_eq!(spec.into_scalar(), document);
    assert_eq!(PivotSpec::from_scalar(&spec.into_scalar()).unwrap(), spec);
}

#[test]
fn pivot_spec_optional_value_properties_canonicalize_to_null() {
    let value = Scalar::from_struct([
        ("field", Scalar::from("Sales")),
        ("aggregate", Scalar::from("sum")),
    ])
    .unwrap();
    let document = document(
        "PivotTable1",
        vec![axis("Region", "ascending")],
        vec![value],
    );
    let spec = PivotSpec::from_scalar(&document).unwrap();
    assert_eq!(spec.values[0].caption, None);
    assert_eq!(spec.values[0].number_format, None);
    let canonical = spec.into_scalar();
    let values = canonical.as_struct().unwrap()["values"]
        .sequence_rows()
        .unwrap();
    let item = values[0].as_struct().unwrap();
    assert!(item["caption"].is_null());
    assert!(item["numberFormat"].is_null());
}

#[test]
fn pivot_spec_intake_refuses_malformed_and_ambiguous_documents_at_field() {
    invalid_at(PivotSpec::from_scalar(&Scalar::from(7)), "$");
    invalid_at(
        PivotSpec::from_scalar(&document(
            "",
            vec![axis("Region", "ascending")],
            vec![value("Sales", "sum", Scalar::Null, Scalar::Null)],
        )),
        "$.name",
    );
    invalid_at(
        PivotSpec::from_scalar(&document(
            "P",
            vec![],
            vec![value("Sales", "sum", Scalar::Null, Scalar::Null)],
        )),
        "$.rows",
    );
    invalid_at(
        PivotSpec::from_scalar(&document("P", vec![axis("Region", "ascending")], vec![])),
        "$.values",
    );
    invalid_at(
        PivotSpec::from_scalar(&document(
            "P",
            vec![axis("Region", "sideways")],
            vec![value("Sales", "sum", Scalar::Null, Scalar::Null)],
        )),
        "$.rows[0].order",
    );
    invalid_at(
        PivotSpec::from_scalar(&document(
            "P",
            vec![axis("Region", "ascending")],
            vec![value("Sales", "median", Scalar::Null, Scalar::Null)],
        )),
        "$.values[0].aggregate",
    );
    let mut fields = document(
        "P",
        vec![axis("Region", "ascending")],
        vec![value("Sales", "sum", Scalar::Null, Scalar::Null)],
    )
    .as_struct()
    .unwrap()
    .clone();
    fields.remove("source");
    invalid_at(
        PivotSpec::from_scalar(&Scalar::from_struct(fields).unwrap()),
        "$.source",
    );
    let mut fields = document(
        "P",
        vec![axis("Region", "ascending")],
        vec![value("Sales", "sum", Scalar::Null, Scalar::Null)],
    )
    .as_struct()
    .unwrap()
    .clone();
    fields.insert("unexpected".into(), Scalar::Null);
    invalid_at(
        PivotSpec::from_scalar(&Scalar::from_struct(fields).unwrap()),
        "$.unexpected",
    );
}

#[test]
fn pivot_spec_refuses_a_field_on_two_axes() {
    let values = || vec![value("Sales", "sum", Scalar::Null, Scalar::Null)];
    invalid_at(
        PivotSpec::from_scalar(&document(
            "P",
            vec![axis("Region", "ascending"), axis("Region", "descending")],
            values(),
        )),
        "$.rows[1].field",
    );
    invalid_at(
        PivotSpec::from_scalar(&document("P", vec![axis("Year", "ascending")], values())),
        "$.columns[0].field",
    );
}

#[cfg(feature = "internals")]
#[path = "pivot/layout.rs"]
mod layout;

#[cfg(feature = "internals")]
#[path = "pivot/compute.rs"]
mod compute;

#[test]
fn pivot_inventory_reads_five_native_tables_through_package_relationships() {
    use yggdryl::excel::Workbook;

    let workbook =
        Workbook::from_bytes(include_bytes!("fixtures/pivot_excel.xlsx").to_vec()).unwrap();
    let mut observed: Vec<_> = workbook
        .pivots()
        .unwrap()
        .iter()
        .map(|pivot| {
            (
                pivot.name().to_owned(),
                pivot.host_sheet().to_owned(),
                pivot.location().to_string(),
                pivot.cache_id(),
            )
        })
        .collect();
    observed.sort();
    assert_eq!(
        observed,
        [
            (
                "P6_matrix_one_value".into(),
                "CaseMatrix".into(),
                "A3:E8".into(),
                61
            ),
            (
                "P6_matrix_two_values".into(),
                "CaseTwoValues".into(),
                "A3:I9".into(),
                71
            ),
            (
                "P6_nested_subtotals".into(),
                "CaseNested".into(),
                "A3:E14".into(),
                91
            ),
            (
                "P6_source_order_grand".into(),
                "CaseOrder".into(),
                "A3:B6".into(),
                99
            ),
            (
                "P6_values_without_column".into(),
                "CaseNoColumn".into(),
                "A3:C7".into(),
                79
            ),
        ]
    );
}

mod vertical {
    //! Public create, save, reopen, refresh and remove controls for
    //! `rust/src/excel/pivot.rs` and its workbook publisher.

    use smol_str::SmolStr;
    use yggdryl::Scalar;
    use yggdryl::excel::{
        Aggregate, AxisField, CellRef, Edit, ItemOrder, PivotSource, PivotSpec, ValueField,
        Workbook,
    };

    fn at(text: &str) -> CellRef {
        text.parse().unwrap()
    }

    fn fixture() -> (Workbook, PivotSpec) {
        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        for (at, value) in [
            ("A1", Scalar::from("Group")),
            ("B1", Scalar::from("Value")),
            ("A2", Scalar::from("East")),
            ("B2", Scalar::from(1.0e16)),
            ("A3", Scalar::from("West")),
            ("B3", Scalar::from(1.0)),
            ("A4", Scalar::from("East")),
            ("B4", Scalar::from(-1.0e16)),
            ("A5", Scalar::from("West")),
            ("B5", Scalar::from(2.0)),
        ] {
            source.set_cell(at.parse().unwrap(), value).unwrap();
        }
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: SmolStr::new_static("P6_Order"),
            source: PivotSource {
                sheet: SmolStr::new_static("Data"),
                range: "A1:B5".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: SmolStr::new_static("Group"),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: SmolStr::new_static("Value"),
                aggregate: Aggregate::Sum,
                caption: Some(SmolStr::new_static("Value Sum")),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: true,
        };
        (book, spec)
    }

    #[test]
    fn pivot_parent_average_consumes_source_values_at_both_axis_prefixes() {
        use yggdryl::Scalar;
        use yggdryl::excel::{
            Aggregate, AxisField, CellRef, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
        };

        let mut book = Workbook::new();
        book.add_sheet("Data").unwrap();
        book.add_sheet("Report").unwrap();
        for (cell, text) in [
            ("A1", "Region"),
            ("B1", "Product"),
            ("C1", "Year"),
            ("D1", "Quarter"),
            ("E1", "Sales"),
        ] {
            book.set_entry("Data", cell.parse().unwrap(), text).unwrap();
        }
        for (index, (product, quarter, sales)) in [
            ("A", "Q1", "1"),
            ("A", "Q1", "3"),
            ("B", "Q1", "8"),
            ("B", "Q1", "8"),
            ("B", "Q1", "8"),
            ("B", "Q1", "8"),
            ("A", "Q2", "6"),
            ("A", "Q2", "6"),
        ]
        .into_iter()
        .enumerate()
        {
            let row = index + 2;
            for (column, value) in [
                ("A", "East"),
                ("B", product),
                ("C", "2024"),
                ("D", quarter),
                ("E", sales),
            ] {
                book.set_entry("Data", format!("{column}{row}").parse().unwrap(), value)
                    .unwrap();
            }
        }
        let axis = |field: &str| AxisField {
            field: field.into(),
            order: ItemOrder::Ascending,
        };
        let spec = PivotSpec {
            name: "AverageParents".into(),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:E9".parse().unwrap(),
            },
            rows: vec![axis("Region"), axis("Product")],
            columns: vec![axis("Year"), axis("Quarter")],
            values: vec![ValueField {
                field: "Sales".into(),
                aggregate: Aggregate::Average,
                caption: Some("Sales Average".into()),
                number_format: None,
            }],
            subtotals: true,
            row_grand_totals: true,
            column_grand_totals: true,
        };
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        let report = book.sheet("Report").unwrap();
        let subtotal = (0..20)
            .find(|&row| report.scalar(CellRef::new(row, 0)) == Scalar::from("Total East"))
            .expect("East subtotal row");
        // Native Excel's Q1 subtotal weights six source values, unlike the
        // unweighted mean of its displayed leaf averages 2 and 8 (=5).
        let native: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_parent_subtotals_native.json"))
                .unwrap();
        let unequal = native["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == "unequal_weighted_parent_average")
            .unwrap();
        for (address, row, column) in [("A3", 0, 0), ("E4", 1, 4)] {
            let label = unequal["value2"][row][column]["value"].as_str().unwrap();
            assert_eq!(
                report.scalar(address.parse().unwrap()),
                Scalar::from(label),
                "native parent header {address}"
            );
        }
        let expected = unequal["value2"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row[0]["value"] == "Total East")
            .unwrap();
        for column in 2..=5 {
            let number = expected[column]["value"].as_f64().unwrap();
            assert_eq!(number, 6.0, "native subtotal column {column}");
            assert_eq!(
                report.scalar(CellRef::new(subtotal, column as u32)),
                Scalar::from(number)
            );
        }
        let bytes = book.into_bytes().unwrap();
        if let Ok(path) = std::env::var("YGGDRYL_EXCEL_PIVOT_PARENT_OUT") {
            let path = std::path::PathBuf::from(path);
            assert!(!path.exists(), "select a fresh pivot export path");
            std::fs::write(path, &bytes).unwrap();
        }
        let mut reopened = Workbook::from_bytes(bytes).unwrap();
        reopened.refresh_pivot("Report", "AverageParents").unwrap();
        assert_eq!(
            reopened
                .sheet("Report")
                .unwrap()
                .scalar(CellRef::new(subtotal, 4)),
            Scalar::from(6.0)
        );
    }

    #[test]
    fn pivot_vertical_create_save_reopen_refresh_remove() {
        let (mut book, spec) = fixture();
        let location = book.add_pivot(spec, "Report", at("A3")).unwrap();
        assert_eq!(location.to_string(), "A3:B6");
        let report = book.sheet("Report").unwrap();
        assert_eq!(report.scalar(at("A4")), Scalar::from("East"));
        assert_eq!(report.scalar(at("B4")), Scalar::from(0.0));
        assert_eq!(report.scalar(at("A5")), Scalar::from("West"));
        assert_eq!(report.scalar(at("B5")), Scalar::from(3.0));
        assert_eq!(report.scalar(at("B6")), Scalar::from(3.0));

        let bytes = book.into_bytes().unwrap();
        let mut reopened = Workbook::from_bytes(bytes).unwrap();
        let entries = reopened.pivots().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), "P6_Order");
        assert_eq!(entries[0].host_sheet(), "Report");
        assert_eq!(entries[0].location(), location);
        assert!(entries[0].editable());
        reopened
            .sheet_mut("Data")
            .unwrap()
            .set_cell(at("B5"), 4.0)
            .unwrap();
        assert_eq!(
            reopened.refresh_pivot("Report", "P6_Order").unwrap(),
            location
        );
        assert_eq!(
            reopened.sheet("Report").unwrap().scalar(at("B5")),
            Scalar::from(5.0)
        );
        assert_eq!(
            reopened.sheet("Report").unwrap().scalar(at("B6")),
            Scalar::from(5.0)
        );
        reopened.remove_pivot("Report", "P6_Order").unwrap();
        assert!(reopened.pivots().unwrap().is_empty());
        assert!(reopened.sheet("Report").unwrap().cell(at("A3")).is_none());
        assert!(
            Workbook::from_bytes(reopened.into_bytes().unwrap())
                .unwrap()
                .pivots()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pivot_no_column_writer_retains_native_one_header_row_wire_contract() {
        for values in [1, 2] {
            let (mut book, mut spec) = fixture();
            if values == 2 {
                let mut second = spec.values[0].clone();
                second.caption = Some("Second Value".into());
                spec.values.push(second);
            }
            book.add_pivot(spec, "Report", at("A3")).unwrap();
            let bytes = book.into_bytes().unwrap();
            let parts = crate::excel_package::exact_parts(&bytes);
            let table = String::from_utf8(parts["xl/pivotTables/pivotTable1.xml"].clone()).unwrap();
            assert!(table.contains("gridDropZones=\"0\""), "{table}");
            assert!(table.contains("hideValuesRow=\"1\""), "{table}");
            assert_eq!(
                table
                    .matches("962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2")
                    .count(),
                1
            );
            let reopened = yggdryl::excel::Workbook::from_bytes(bytes).unwrap();
            let pivots = reopened.pivots().unwrap();
            let selected = &pivots[0];
            assert!(selected.editable(), "{}: {:?}", values, selected.origin());
        }
    }

    #[test]
    fn pivot_vertical_late_collision_refuses_without_a_change() {
        let (mut book, spec) = fixture();
        book.sheet_mut("Report")
            .unwrap()
            .set_cell(at("B5"), "keep")
            .unwrap();
        let before = book.into_bytes().unwrap();
        let revision = book.sheet("Report").unwrap().revision();
        let error = book.add_pivot(spec, "Report", at("A3")).unwrap_err();
        assert!(error.to_string().contains("B5"), "{error}");
        assert_eq!(book.sheet("Report").unwrap().revision(), revision);
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("B5")),
            Scalar::from("keep")
        );
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        assert!(book.pivots().unwrap().is_empty());
    }

    #[test]
    fn pivot_vertical_inverse_restores_cells_and_part_membership_exactly() {
        let (mut book, spec) = fixture();
        let before = book.into_bytes().unwrap();
        let applied = book
            .apply(Edit::PivotCreate {
                spec,
                sheet: SmolStr::new_static("Report"),
                anchor: at("A3"),
            })
            .unwrap();
        let created = book.into_bytes().unwrap();
        assert_ne!(created, before);
        let undone = book.apply(applied.inverse.unwrap()).unwrap();
        assert!(book.pivots().unwrap().is_empty());
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        book.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&created)
        );
    }

    #[test]
    fn pivot_update_edits_spec_and_restores_exact_package_and_cells() {
        let (mut book, spec) = fixture();
        book.add_pivot(spec.clone(), "Report", at("A3")).unwrap();
        let before = book.into_bytes().unwrap();
        let mut revised = spec;
        revised.values[0].caption = Some(SmolStr::new_static("Revised Total"));
        let document = Scalar::from_struct([
            ("op", Scalar::from("pivotUpdate")),
            (
                "sheet",
                Scalar::from(book.sheet_key("Report").unwrap().as_u32()),
            ),
            ("name", Scalar::from("P6_Order")),
            ("spec", revised.into_scalar()),
        ])
        .unwrap();
        let edit = Edit::from_scalar(&document, &book).unwrap();
        let applied = book.apply(edit).unwrap();
        assert_eq!(
            book.pivots().unwrap()[0].spec().unwrap().values[0]
                .caption
                .as_deref(),
            Some("Revised Total")
        );
        let updated = book.into_bytes().unwrap();
        assert_ne!(updated, before);
        let undone = book.apply(applied.inverse.unwrap()).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        book.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&updated)
        );
    }

    #[test]
    fn pivot_refresh_all_is_atomic_and_one_inverse() {
        let (mut book, first) = fixture();
        let second_source = book.add_sheet("Other").unwrap();
        for (at, value) in [
            ("A1", Scalar::from("Group")),
            ("B1", Scalar::from("Value")),
            ("A2", Scalar::from("East")),
            ("B2", Scalar::from(7.0)),
        ] {
            second_source.set_cell(at.parse().unwrap(), value).unwrap();
        }
        let mut second = first.clone();
        second.name = SmolStr::new_static("P6_Other");
        second.source.sheet = SmolStr::new_static("Other");
        second.source.range = "A1:B2".parse().unwrap();
        book.add_pivot(first, "Report", at("A3")).unwrap();
        book.add_pivot(second, "Report", at("A10")).unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .set_cell(at("B5"), 4.0)
            .unwrap();
        book.sheet_mut("Other")
            .unwrap()
            .set_cell(at("B1"), "Changed")
            .unwrap();

        let request = Scalar::from_struct([("op", Scalar::from("pivotRefreshAll"))]).unwrap();
        let before_failure = book.into_bytes().unwrap();
        let edit = Edit::from_scalar(&request, &book).unwrap();
        let error = book.apply(edit).unwrap_err();
        assert!(error.to_string().contains("Value"), "{error}");
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before_failure)
        );

        book.sheet_mut("Other")
            .unwrap()
            .set_cell(at("B1"), "Value")
            .unwrap();
        let before = book.into_bytes().unwrap();
        let edit = Edit::from_scalar(&request, &book).unwrap();
        let applied = book.apply(edit).unwrap();
        assert_eq!(
            book.sheet("Report").unwrap().scalar(at("B5")),
            Scalar::from(5.0)
        );
        assert_eq!(book.pivots().unwrap().len(), 2);
        let after = book.into_bytes().unwrap();
        assert_ne!(after, before);
        let inverse = applied.inverse.expect("one inverse for both pivots");
        let undone = book.apply(inverse).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        book.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&after)
        );
    }

    #[test]
    fn pivot_value_number_format_survives_create_reopen_refresh_and_inverse() {
        let (mut book, mut spec) = fixture();
        let code = "#,##0.0000";
        spec.values[0].number_format = Some(SmolStr::new(code));
        let applied = book
            .apply(Edit::PivotCreate {
                spec,
                sheet: SmolStr::new_static("Report"),
                anchor: at("A3"),
            })
            .unwrap();
        assert_eq!(
            book.cell_style("Report", at("B4")).unwrap().number_format,
            code
        );
        assert_eq!(
            book.cell_style("Report", at("B6")).unwrap().number_format,
            code
        );
        assert_eq!(
            book.cell_style("Report", at("A4")).unwrap().number_format,
            "General"
        );
        let styles_created = book.style_sheet().unwrap().len();
        let created = book.into_bytes().unwrap();
        if let Ok(path) = std::env::var("YGGDRYL_EXCEL_PIVOT_FORMAT_OUT") {
            let path = std::path::PathBuf::from(path);
            assert!(!path.exists(), "select a fresh pivot export path");
            std::fs::write(path, &created).unwrap();
        }
        let mut reopened = Workbook::from_bytes(created.clone()).unwrap();
        assert_eq!(
            reopened.pivots().unwrap()[0].spec().unwrap().values[0]
                .number_format
                .as_deref(),
            Some(code)
        );
        assert_eq!(
            reopened
                .cell_style("Report", at("B4"))
                .unwrap()
                .number_format,
            code
        );
        let reopened_styles = reopened.style_sheet().unwrap().len();
        reopened.refresh_pivot("Report", "P6_Order").unwrap();
        assert_eq!(
            reopened
                .cell_style("Report", at("B4"))
                .unwrap()
                .number_format,
            code
        );
        assert_eq!(reopened.style_sheet().unwrap().len(), reopened_styles);

        let undone = book.apply(applied.inverse.unwrap()).unwrap();
        assert!(book.pivots().unwrap().is_empty());
        assert!(book.sheet("Report").unwrap().cell(at("B4")).is_none());
        assert_eq!(
            book.cell_style("Report", at("A4")).unwrap().number_format,
            "General"
        );
        // StyleSheet is append-only. Undo restores cell/package meaning;
        // retained unused XFs are reused by redo rather than truncated.
        assert_eq!(book.style_sheet().unwrap().len(), styles_created);
        book.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&created)
        );
        assert_eq!(book.style_sheet().unwrap().len(), styles_created);

        let removed = book
            .apply(Edit::PivotRemove {
                sheet: "Report".into(),
                name: "P6_Order".into(),
            })
            .unwrap();
        book.apply(removed.inverse.unwrap()).unwrap();
        assert_eq!(
            book.cell_style("Report", at("B4")).unwrap().number_format,
            code
        );
        assert_eq!(book.style_sheet().unwrap().len(), styles_created);
    }

    #[test]
    fn pivot_value_number_format_late_refusals_leave_styles_and_parts_unchanged() {
        for collision in [false, true] {
            let (mut book, mut spec) = fixture();
            spec.values[0].number_format = Some("#,##0.0000".into());
            if collision {
                book.sheet_mut("Report")
                    .unwrap()
                    .set_cell(at("B5"), "keep")
                    .unwrap();
            } else {
                let mut second = spec.values[0].clone();
                second.caption = Some("Second".into());
                second.number_format = Some("0.00\"x".into());
                spec.values.push(second);
            }
            let before = book.into_bytes().unwrap();
            let styles = book.style_sheet().unwrap().len();
            let revision = book.sheet("Report").unwrap().revision();
            let error = book.add_pivot(spec, "Report", at("A3")).unwrap_err();
            let location = if collision {
                "B5"
            } else {
                "$.values[1].numberFormat"
            };
            assert!(error.to_string().contains(location), "{error}");
            assert_eq!(book.style_sheet().unwrap().len(), styles);
            assert_eq!(book.sheet("Report").unwrap().revision(), revision);
            assert!(book.pivots().unwrap().is_empty());
            assert_eq!(
                crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
                crate::excel_package::exact_parts(&before)
            );
        }
    }

    #[test]
    fn pivot_value_number_format_temporal_serials_survive_publication_and_reopen() {
        use yggdryl::excel::DateSystem;
        for system in [DateSystem::Year1900, DateSystem::Year1904] {
            for raw in [60.0_f64, 0.000_000_001, 2_958_465.999_999_999_5] {
                let (mut book, mut spec) = fixture();
                book.set_date_system(system);
                for (cell, value) in [("B2", raw), ("B4", 0.0)] {
                    book.sheet_mut("Data")
                        .unwrap()
                        .set_cell(at(cell), value)
                        .unwrap();
                }
                spec.values[0].number_format = Some("m/d/yyyy h:mm:ss.000".into());
                spec.column_grand_totals = false;
                book.add_pivot(spec, "Report", at("A3")).unwrap();
                for saved in [false, true] {
                    if saved {
                        book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
                    }
                    book.set_entry("Report", at("D1"), "=B4").unwrap();
                    book.calculate_all().unwrap();
                    let actual = book
                        .sheet("Report")
                        .unwrap()
                        .scalar(at("D1"))
                        .as_f64()
                        .unwrap();
                    assert_eq!(actual.to_bits(), raw.to_bits(), "{system:?}, saved={saved}");
                }
            }
        }
    }
}

#[path = "pivot/part.rs"]
mod part;

mod matrix {
    //! Native five-case PivotTable fixture controls for the shared display plan.

    use smol_str::SmolStr;
    use yggdryl::Scalar;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };

    fn source() -> Workbook {
        let mut book =
            Workbook::from_bytes(include_bytes!("fixtures/pivot_excel.xlsx").to_vec()).unwrap();
        book.add_sheet("P6Proof").unwrap();
        book
    }

    fn spec(columns: bool, two_values: bool) -> PivotSpec {
        let mut values = vec![ValueField {
            field: "Sales".into(),
            aggregate: Aggregate::Sum,
            caption: Some("Sales Sum".into()),
            number_format: None,
        }];
        if two_values {
            values.push(ValueField {
                field: "Units".into(),
                aggregate: Aggregate::Average,
                caption: Some("Units Average".into()),
                number_format: None,
            });
        }
        PivotSpec {
            name: SmolStr::new_static("P6_Proof"),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:F9".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: "Region".into(),
                order: ItemOrder::Ascending,
            }],
            columns: if columns {
                vec![AxisField {
                    field: "Product".into(),
                    order: ItemOrder::Ascending,
                }]
            } else {
                Vec::new()
            },
            values,
            subtotals: false,
            row_grand_totals: columns,
            column_grand_totals: true,
        }
    }

    fn number(book: &Workbook, at: &str) -> Scalar {
        book.sheet("P6Proof").unwrap().scalar(at.parse().unwrap())
    }

    #[test]
    fn authored_blank_caption_matches_its_own_excel_refreshed_grid() {
        let evidence: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_explicit_blank_native.json"))
                .unwrap();
        assert_eq!(evidence["rust_matrix"]["overall_run_passed"], false);
        assert_eq!(evidence["reports"].as_array().unwrap().len(), 12);
        let mut book = source();
        book.add_pivot(spec(true, false), "P6Proof", "A3".parse().unwrap())
            .unwrap();
        let grid = evidence["rust_matrix"]["candidate_grid"]
            .as_array()
            .unwrap();
        assert_eq!((grid.len(), grid[0].as_array().unwrap().len()), (6, 5));
        let sheet = book.sheet("P6Proof").unwrap();
        for (row, values) in grid.iter().enumerate() {
            for (column, expected) in values.as_array().unwrap().iter().enumerate() {
                let at = yggdryl::excel::CellRef::new(row as u32 + 2, column as u32);
                let actual = sheet.cell(at);
                match expected["variant"].as_str().unwrap() {
                    "empty" => assert!(actual.is_none(), "{at}"),
                    "str" => assert_eq!(
                        actual.unwrap().value().as_str(),
                        expected["value"].as_str(),
                        "{at}"
                    ),
                    "float" => assert_eq!(
                        actual.unwrap().value().as_f64().unwrap().to_bits(),
                        u64::from_str_radix(expected["ieee754_hex"].as_str().unwrap(), 16).unwrap(),
                        "{at}"
                    ),
                    other => panic!("unhandled native transport {other} at {at}"),
                }
            }
        }
    }

    #[test]
    fn pivot_matrix_one_value_sorts_visible_items_and_matches_native_grid() {
        let mut book = source();
        assert_eq!(
            book.add_pivot(spec(true, false), "P6Proof", "A3".parse().unwrap())
                .unwrap()
                .to_string(),
            "A3:E8"
        );
        for (at, expected) in [
            ("B5", 13.0),
            ("C5", 5.0),
            ("D5", 4.0),
            ("E5", 22.0),
            ("B6", 20.0),
            ("C6", 18.0),
            ("E6", 38.0),
            ("B8", 35.0),
            ("C8", 23.0),
            ("D8", 4.0),
            ("E8", 62.0),
        ] {
            assert_eq!(number(&book, at), Scalar::from(expected), "{at}");
        }
        assert!(
            book.sheet("P6Proof")
                .unwrap()
                .cell("D6".parse().unwrap())
                .is_none()
        );
    }

    #[test]
    fn pivot_values_without_column_uses_one_header_and_weighted_average() {
        let mut book = source();
        assert_eq!(
            book.add_pivot(spec(false, true), "P6Proof", "A3".parse().unwrap())
                .unwrap()
                .to_string(),
            "A3:C7"
        );
        for (at, expected) in [
            ("B4", 22.0),
            ("C4", 1.75),
            ("B5", 38.0),
            ("C5", 3.3333333333333335),
            ("B6", 2.0),
            ("C6", 1.0),
            ("B7", 62.0),
            ("C7", 2.25),
        ] {
            assert_eq!(number(&book, at), Scalar::from(expected), "{at}");
        }
    }

    #[test]
    fn pivot_matrix_two_values_places_values_axis_and_weighted_grands() {
        let mut book = source();
        assert_eq!(
            book.add_pivot(spec(true, true), "P6Proof", "A3".parse().unwrap())
                .unwrap()
                .to_string(),
            "A3:I9"
        );
        for (at, expected) in [
            ("B6", 13.0),
            ("C6", 2.5),
            ("D6", 5.0),
            ("E6", 1.0),
            ("H6", 22.0),
            ("I6", 1.75),
            ("B7", 20.0),
            ("C7", 2.0),
            ("H7", 38.0),
            ("I7", 3.3333333333333335),
            ("H9", 62.0),
            ("I9", 2.25),
        ] {
            assert_eq!(number(&book, at), Scalar::from(expected), "{at}");
        }
    }
}

mod numeric_policy {
    //! A regrouping fold must not turn an unresolved numeric result into a blank.

    use smol_str::SmolStr;
    use yggdryl::Scalar;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };

    #[test]
    fn pivot_row_total_uncertain_numeric_fold_refuses_atomically() {
        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        for (at, value) in [
            ("A1", Scalar::from("Region")),
            ("B1", Scalar::from("Product")),
            ("C1", Scalar::from("Amount")),
            ("A2", Scalar::from("East")),
            ("B2", Scalar::from("One")),
            ("C2", Scalar::from(3.0e-308)),
            ("A3", Scalar::from("East")),
            ("B3", Scalar::from("Two")),
            ("C3", Scalar::from(-2.5e-308)),
        ] {
            source.set_cell(at.parse().unwrap(), value).unwrap();
        }
        book.add_sheet("Report").unwrap();
        let before = book.into_bytes().unwrap();
        let revision = book.sheet("Report").unwrap().revision();
        let spec = PivotSpec {
            name: SmolStr::new_static("P6_Uncertain"),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:C3".parse().unwrap(),
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
                field: "Amount".into(),
                aggregate: Aggregate::Sum,
                caption: None,
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: true,
            column_grand_totals: false,
        };
        let error = book
            .add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("$.pivot.rowTotal"), "{error}");
        assert_eq!(book.sheet("Report").unwrap().revision(), revision);
        assert!(book.pivots().unwrap().is_empty());
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
    }

    #[test]
    fn pivot_grand_total_uncertain_numeric_fold_refuses_atomically() {
        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        for (at, value) in [
            ("A1", Scalar::from("Region")),
            ("B1", Scalar::from("Amount")),
            ("A2", Scalar::from("East")),
            ("B2", Scalar::from(3.0e-308)),
            ("A3", Scalar::from("West")),
            ("B3", Scalar::from(-2.5e-308)),
        ] {
            source.set_cell(at.parse().unwrap(), value).unwrap();
        }
        book.add_sheet("Report").unwrap();
        let before = book.into_bytes().unwrap();
        let spec = PivotSpec {
            name: SmolStr::new_static("P6_Grand_Uncertain"),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:B3".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: "Region".into(),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: "Amount".into(),
                aggregate: Aggregate::Sum,
                caption: None,
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: true,
        };
        let error = book
            .add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("$.pivot.grandTotal"), "{error}");
        assert!(book.pivots().unwrap().is_empty());
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
    }
}

mod subtotal {
    //! The fourth native P6 shape: two row axes, one column axis and subtotals.

    use smol_str::SmolStr;
    use yggdryl::Scalar;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };

    #[test]
    fn pivot_nested_subtotals_match_native_numeric_grid() {
        let mut book =
            Workbook::from_bytes(include_bytes!("fixtures/pivot_excel.xlsx").to_vec()).unwrap();
        book.add_sheet("P6Proof").unwrap();
        let spec = PivotSpec {
            name: SmolStr::new_static("P6_NestedProof"),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:F9".parse().unwrap(),
            },
            rows: vec![
                AxisField {
                    field: "Region".into(),
                    order: ItemOrder::Ascending,
                },
                AxisField {
                    field: "Product".into(),
                    order: ItemOrder::Ascending,
                },
            ],
            columns: vec![AxisField {
                field: "Year".into(),
                order: ItemOrder::Ascending,
            }],
            values: vec![ValueField {
                field: "Sales".into(),
                aggregate: Aggregate::Sum,
                caption: Some("Sales Sum".into()),
                number_format: None,
            }],
            subtotals: true,
            row_grand_totals: true,
            column_grand_totals: true,
        };
        assert_eq!(
            book.add_pivot(spec, "P6Proof", "A3".parse().unwrap())
                .unwrap()
                .to_string(),
            "A3:E14"
        );
        let sheet = book.sheet("P6Proof").unwrap();
        for (at, expected) in [
            ("C5", 10.0),
            ("D5", 3.0),
            ("E5", 13.0),
            ("D6", 5.0),
            ("C7", 4.0),
            ("C8", 14.0),
            ("D8", 8.0),
            ("E8", 22.0),
            ("C11", 31.0),
            ("D11", 7.0),
            ("E11", 38.0),
            ("D13", 2.0),
            ("C14", 45.0),
            ("D14", 17.0),
            ("E14", 62.0),
        ] {
            assert_eq!(
                sheet.scalar(at.parse().unwrap()),
                Scalar::from(expected),
                "{at}"
            );
        }
        assert!(sheet.cell("C6".parse().unwrap()).is_none());
    }
}

mod reopened {
    //! Created multi-shape parts must stay typed after a package round trip.

    use smol_str::SmolStr;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };

    fn request(subtotals: bool) -> PivotSpec {
        PivotSpec {
            name: SmolStr::new_static(if subtotals {
                "P6_ReopenSubtotal"
            } else {
                "P6_ReopenMatrix"
            }),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:F9".parse().unwrap(),
            },
            rows: if subtotals {
                vec![
                    AxisField {
                        field: "Region".into(),
                        order: ItemOrder::Ascending,
                    },
                    AxisField {
                        field: "Product".into(),
                        order: ItemOrder::Ascending,
                    },
                ]
            } else {
                vec![AxisField {
                    field: "Region".into(),
                    order: ItemOrder::Ascending,
                }]
            },
            columns: vec![AxisField {
                field: if subtotals { "Year" } else { "Product" }.into(),
                order: ItemOrder::Ascending,
            }],
            values: if subtotals {
                vec![ValueField {
                    field: "Sales".into(),
                    aggregate: Aggregate::Sum,
                    caption: Some("Sales Sum".into()),
                    number_format: None,
                }]
            } else {
                vec![
                    ValueField {
                        field: "Sales".into(),
                        aggregate: Aggregate::Sum,
                        caption: Some("Sales Sum".into()),
                        number_format: None,
                    },
                    ValueField {
                        field: "Units".into(),
                        aggregate: Aggregate::Average,
                        caption: Some("Units Average".into()),
                        number_format: None,
                    },
                ]
            },
            subtotals,
            row_grand_totals: true,
            column_grand_totals: true,
        }
    }

    #[test]
    fn pivot_created_matrix_and_subtotal_reopen_refresh_and_remove() {
        for subtotals in [false, true] {
            let mut book =
                Workbook::from_bytes(include_bytes!("fixtures/pivot_excel.xlsx").to_vec()).unwrap();
            book.add_sheet("P6Proof").unwrap();
            let spec = request(subtotals);
            let name = spec.name.clone();
            let location = book
                .add_pivot(spec, "P6Proof", "A3".parse().unwrap())
                .unwrap();
            let mut reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            let pivot = reopened
                .pivots()
                .unwrap()
                .iter()
                .find(|pivot| pivot.name() == name.as_str())
                .unwrap();
            assert!(pivot.editable(), "{}: {:?}", name, pivot.origin());
            assert_eq!(pivot.location(), location);
            reopened
                .sheet_mut("Data")
                .unwrap()
                .set_cell("D2".parse().unwrap(), 11.0)
                .unwrap();
            assert_eq!(reopened.refresh_pivot("P6Proof", &name).unwrap(), location);
            reopened.remove_pivot("P6Proof", &name).unwrap();
            let removed = Workbook::from_bytes(reopened.into_bytes().unwrap()).unwrap();
            assert!(
                !removed
                    .pivots()
                    .unwrap()
                    .iter()
                    .any(|pivot| pivot.name() == name.as_str())
            );
        }
    }
}

#[test]
fn pivot_native_simple_is_editable_without_losing_excel_extensions() {
    use yggdryl::excel::Workbook;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    let mut workbook =
        Workbook::from_bytes(include_bytes!("fixtures/pivot_ascending_native.xlsx").to_vec())
            .unwrap();
    let selected = workbook
        .pivots()
        .unwrap()
        .iter()
        .find(|pivot| pivot.name() == "P6_source_order_grand")
        .unwrap();
    assert_eq!(selected.host_sheet(), "CaseOrder");
    assert_eq!(selected.location().to_string(), "A3:B6");
    assert!(selected.editable(), "{:?}", selected.origin());
    let spec = selected.spec().unwrap().clone();
    assert_eq!(spec.source.sheet, "Order");
    assert_eq!(spec.source.range.to_string(), "A1:B5");
    assert_eq!(spec.rows.len(), 1);
    assert_eq!(spec.rows[0].field, "Group");
    assert!(spec.columns.is_empty());
    assert_eq!(spec.values.len(), 1);
    assert_eq!(spec.values[0].field, "Value");
    assert_eq!(spec.values[0].aggregate, Aggregate::Sum);
    assert_eq!(spec.values[0].caption.as_deref(), Some("Value Sum"));
    let header_style = workbook
        .sheet("CaseOrder")
        .unwrap()
        .cell("A3".parse().unwrap())
        .unwrap()
        .style();
    let number_style = workbook
        .sheet("CaseOrder")
        .unwrap()
        .cell("B4".parse().unwrap())
        .unwrap()
        .style();

    workbook
        .refresh_pivot("CaseOrder", "P6_source_order_grand")
        .unwrap();
    assert_eq!(
        workbook
            .sheet("CaseOrder")
            .unwrap()
            .cell("A3".parse().unwrap())
            .unwrap()
            .style(),
        header_style
    );
    assert_eq!(
        workbook
            .sheet("CaseOrder")
            .unwrap()
            .cell("B4".parse().unwrap())
            .unwrap()
            .style(),
        number_style
    );
    let bytes = workbook.into_bytes().unwrap();
    let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.clone(),
    ))));
    let table = String::from_utf8(
        archive
            .read_member("xl/pivotTables/pivotTable5.xml")
            .unwrap(),
    )
    .unwrap();
    let cache = String::from_utf8(
        archive
            .read_member("xl/pivotCache/pivotCacheDefinition5.xml")
            .unwrap(),
    )
    .unwrap();
    assert!(table.contains("hideValuesRow=\"1\""), "{table}");
    assert!(table.contains("pivotTableDefinition16"), "{table}");
    assert!(cache.contains("pivotCacheDefinition"), "{cache}");
    let reopened = Workbook::from_bytes(bytes).unwrap();
    let same = reopened
        .pivots()
        .unwrap()
        .iter()
        .find(|pivot| pivot.name() == "P6_source_order_grand")
        .unwrap();
    assert!(same.editable(), "{:?}", same.origin());
    assert_eq!(same.spec(), Some(&spec));
}

// Native11 source rows and typed answers are loaded from the committed oracle,
// including the openpyxl empty inline-string source fact at Data!B7.
mod native_aggregates {
    use super::*;
    use yggdryl::excel::{
        AxisField, Cell, CellRef, DateSystem, ExcelError, Formula, ItemOrder, PivotSource,
        ValueField, Workbook,
    };

    fn native_aggregate_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("fixtures/pivot_all_aggregates_native.json")).unwrap()
    }

    fn native_aggregate_kind(name: &str) -> Aggregate {
        match name {
            "sum" => Aggregate::Sum,
            "count" => Aggregate::Count,
            "average" => Aggregate::Average,
            "max" => Aggregate::Max,
            "min" => Aggregate::Min,
            "product" => Aggregate::Product,
            "countNumbers" => Aggregate::CountNumbers,
            "stdDev" => Aggregate::StdDev,
            "stdDevP" => Aggregate::StdDevP,
            "var" => Aggregate::Var,
            "varP" => Aggregate::VarP,
            other => panic!("unrecognized native aggregate {other}"),
        }
    }

    fn native_aggregate_book(fixture: &serde_json::Value) -> Workbook {
        let mut book = Workbook::new();
        let sheet = book.add_sheet("Data").unwrap();
        let rows = fixture["authored_source_tables"][0]["rows"]
            .as_array()
            .unwrap();
        for (row, values) in rows.iter().enumerate() {
            for (column, value) in values.as_array().unwrap().iter().enumerate() {
                let at = CellRef::new(row as u32, column as u32);
                if let Some(text) = value.as_str() {
                    if text == "#N/A" || text == "#DIV/0!" {
                        let cell = Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                            .unwrap()
                            .with_error(ExcelError::from_text(text));
                        sheet.insert_cell(cell).unwrap();
                        continue;
                    }
                    if text == "=\"\"" {
                        let cell = Cell::from_scalar(at, Scalar::from(""), DateSystem::Year1900)
                            .unwrap()
                            .with_formula(Formula::from_file("\"\"", at));
                        sheet.insert_cell(cell).unwrap();
                        continue;
                    }
                }
                let scalar = match value {
                    serde_json::Value::Null => continue,
                    serde_json::Value::String(text) => Scalar::from(text.as_str()),
                    serde_json::Value::Bool(value) => Scalar::from(*value),
                    serde_json::Value::Number(value) => Scalar::from(value.as_f64().unwrap()),
                    other => panic!("unsupported authored source scalar: {other}"),
                };
                sheet.set_cell(at, scalar).unwrap();
            }
        }
        book.add_sheet("Result").unwrap();
        book
    }

    fn native_aggregate_spec(case: &serde_json::Value, range: &str, grand: bool) -> PivotSpec {
        PivotSpec {
            name: format!("P6_{}", case["id"].as_str().unwrap()).into(),
            source: PivotSource {
                sheet: "Data".into(),
                range: range.parse().unwrap(),
            },
            rows: vec![AxisField {
                field: "Group".into(),
                order: ItemOrder::Ascending,
            }],
            columns: vec![],
            values: vec![ValueField {
                field: "Metric".into(),
                aggregate: native_aggregate_kind(case["aggregate"].as_str().unwrap()),
                caption: Some(case["data_field"]["caption"].as_str().unwrap().into()),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: grand,
        }
    }

    #[test]
    fn pivot_native_aggregate_eleven_group_results_preserve_empty_error_and_numeric_cells() {
        let fixture = native_aggregate_fixture();
        assert!(fixture["cleanup_completed"].as_bool().unwrap());
        let source = fixture["source_xml"]["xl/worksheets/sheet1.xml"]
            .as_str()
            .unwrap();
        assert!(source.contains("<c r=\"B7\" t=\"inlineStr\" />"));
        for case in fixture["cases"].as_array().unwrap() {
            let mut book = native_aggregate_book(&fixture);
            book.add_pivot(
                native_aggregate_spec(case, "A1:B17", false),
                "Result",
                "A3".parse().unwrap(),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            let sheet = book.sheet("Result").unwrap();
            for (index, row) in case["value2"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .take(5)
                .enumerate()
            {
                let at = yggdryl::excel::CellRef::new(3 + index as u32, 1);
                let expected = &row[1];
                match expected["variant"].as_str().unwrap() {
                    "empty" => assert!(sheet.cell(at).is_none(), "{} {at}", case["id"]),
                    "float" => {
                        let actual = sheet.scalar(at).as_f64().unwrap();
                        assert_eq!(
                            actual.to_bits(),
                            expected["value"].as_f64().unwrap().to_bits(),
                            "{} {at}",
                            case["id"]
                        );
                    }
                    "int" => {
                        assert_eq!(expected["value"].as_i64(), Some(-2146826281));
                        assert_eq!(
                            sheet.cell(at).and_then(|cell| cell.error()),
                            Some(yggdryl::excel::ExcelError::Div0),
                            "{} {at}",
                            case["id"]
                        );
                    }
                    other => panic!("{} {at}: unsupported observed kind {other}", case["id"]),
                }
            }
            let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            let reopened = reopened.sheet("Result").unwrap();
            for (index, row) in case["value2"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .take(5)
                .enumerate()
            {
                if row[1]["variant"] == "int" {
                    let at = yggdryl::excel::CellRef::new(3 + index as u32, 1);
                    assert_eq!(
                        reopened.cell(at).and_then(|cell| cell.error()),
                        Some(yggdryl::excel::ExcelError::Div0)
                    );
                }
            }
        }
    }

    #[test]
    fn pivot_native_aggregate_nondyadic_grand_variance_refuses_before_publication() {
        let fixture = native_aggregate_fixture();
        for case in fixture["cases"].as_array().unwrap() {
            if !matches!(
                case["aggregate"].as_str().unwrap(),
                "var" | "varP" | "stdDev" | "stdDevP"
            ) {
                continue;
            }
            let mut book = native_aggregate_book(&fixture);
            let before = book.into_bytes().unwrap();
            let error = book
                .add_pivot(
                    native_aggregate_spec(case, "A1:B17", true),
                    "Result",
                    "A3".parse().unwrap(),
                )
                .unwrap_err();
            match error {
                yggdryl::Error::InvalidRecord { path, .. } => {
                    assert_eq!(path.as_str(), "$.pivot.grandTotal[0]", "{}", case["id"])
                }
                other => panic!("{}: expected located numeric hold, got {other}", case["id"]),
            }
            assert!(
                book.sheet("Result")
                    .unwrap()
                    .cell("A3".parse().unwrap())
                    .is_none()
            );
            assert_eq!(
                crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
                crate::excel_package::exact_parts(&before)
            );
        }
    }

    #[test]
    fn pivot_native_aggregate_error_blank_and_formula_empty_groups() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_aggregate_edges_native.json"))
                .unwrap();
        assert!(fixture["cleanup_completed"].as_bool().unwrap());
        let source = fixture["source_xml"]["xl/worksheets/sheet1.xml"]
            .as_str()
            .unwrap();
        assert!(source.contains("r=\"B14\" t=\"str\"><f>\"\"</f>"));
        assert!(source.contains("r=\"B2\" t=\"e\"><v>#N/A</v>"));
        for case in fixture["cases"].as_array().unwrap() {
            let mut book = native_aggregate_book(&fixture);
            book.add_pivot(
                native_aggregate_spec(case, "A1:B31", false),
                "Result",
                "A3".parse().unwrap(),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            let sheet = book.sheet("Result").unwrap();
            for (index, row) in case["value2"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .take(15)
                .enumerate()
            {
                let at = CellRef::new(3 + index as u32, 1);
                let expected = &row[1];
                match expected["variant"].as_str().unwrap() {
                    "empty" => assert!(sheet.cell(at).is_none(), "{} {at}", case["id"]),
                    "float" => {
                        let actual = sheet.scalar(at).as_f64().unwrap();
                        assert_eq!(
                            actual.to_bits(),
                            expected["value"].as_f64().unwrap().to_bits(),
                            "{} {at}",
                            case["id"]
                        );
                    }
                    "int" => {
                        let error = match expected["value"].as_i64().unwrap() {
                            -2146826281 => ExcelError::Div0,
                            -2146826246 => ExcelError::NA,
                            other => panic!("{} {at}: unproved COM error {other}", case["id"]),
                        };
                        assert_eq!(
                            sheet.cell(at).and_then(|cell| cell.error()),
                            Some(error),
                            "{} {at}",
                            case["id"]
                        );
                    }
                    other => panic!("{} {at}: unsupported observed kind {other}", case["id"]),
                }
            }
        }
    }

    #[test]
    fn pivot_native_aggregate_mixed_error_grand_total_follows_visible_group_order() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_aggregate_edges_native.json"))
                .unwrap();
        let case = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["aggregate"] == "sum")
            .unwrap();
        assert_eq!(
            case["value2"].as_array().unwrap().last().unwrap()[1]["value"].as_i64(),
            Some(-2146826281)
        );
        let mut book = native_aggregate_book(&fixture);
        book.add_pivot(
            native_aggregate_spec(case, "A1:B31", true),
            "Result",
            "A3".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(
            book.sheet("Result")
                .unwrap()
                .cell("B19".parse().unwrap())
                .and_then(|cell| cell.error()),
            Some(ExcelError::Div0)
        );
    }

    #[test]
    fn pivot_native_aggregate_error_precedence_follows_displayed_groups() {
        for source in [
            include_str!("fixtures/pivot_error_order_forward_native.json"),
            include_str!("fixtures/pivot_error_order_reversed_native.json"),
        ] {
            let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
            assert_eq!(fixture["cases"].as_array().unwrap().len(), 11);
            assert_eq!(fixture["cleanup_completed"], true);
            let authored = &fixture["authored_source_tables"][0]["rows"];
            let reversed = authored[1][0] == "Z_DIV";
            for case in fixture["cases"].as_array().unwrap() {
                let mut book = native_aggregate_book(&fixture);
                book.add_pivot(
                    native_aggregate_spec(case, "A1:B5", true),
                    "Result",
                    "A3".parse().unwrap(),
                )
                .unwrap_or_else(|error| panic!("{} reversed={reversed}: {error}", case["id"]));
                for pass in 0..2 {
                    let sheet = book.sheet("Result").unwrap();
                    for (index, row) in case["value2"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .skip(1)
                        .enumerate()
                    {
                        let at = CellRef::new(3 + index as u32, 1);
                        let expected = &row[1];
                        match expected["variant"].as_str().unwrap() {
                            "int" => {
                                let error = match expected["value"].as_i64().unwrap() {
                                    -2146826246 => ExcelError::NA,
                                    -2146826281 => ExcelError::Div0,
                                    other => {
                                        panic!("{}: unsupported observed error {other}", case["id"])
                                    }
                                };
                                assert_eq!(
                                    sheet.cell(at).and_then(|cell| cell.error()),
                                    Some(error),
                                    "{} reversed={reversed} {at}",
                                    case["id"]
                                );
                            }
                            "float" => assert_eq!(
                                sheet.scalar(at).as_f64().unwrap().to_bits(),
                                expected["value"].as_f64().unwrap().to_bits(),
                                "{} reversed={reversed} {at}",
                                case["id"]
                            ),
                            other => panic!("{}: unsupported observed kind {other}", case["id"]),
                        }
                    }
                    if pass == 0 {
                        book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn pivot_fields_uses_bound_headers_and_typed_distinct_items() {
    use yggdryl::excel::{Aggregate, PivotSource, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    for (at, value) in [
        ("A1", "Amount"),
        ("B1", "Category"),
        ("A2", "2"),
        ("B2", "East"),
        ("A3", "3"),
        ("B3", "East"),
        ("A4", ""),
        ("B4", "West"),
    ] {
        if at.starts_with('A') && at != "A1" && !value.is_empty() {
            sheet
                .set_cell(at.parse().unwrap(), value.parse::<i64>().unwrap())
                .unwrap();
        } else if !value.is_empty() {
            sheet.set_cell(at.parse().unwrap(), value).unwrap();
        }
    }
    let source = PivotSource {
        sheet: "Data".into(),
        range: "A1:B4".parse().unwrap(),
    };
    let fields = book.pivot_fields(&source).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name.as_str(), "Amount");
    assert!(fields[0].numeric);
    assert_eq!(fields[0].aggregate, Aggregate::Sum);
    assert_eq!(fields[0].items, 3); // 2, 3, blank
    assert_eq!(fields[1].name.as_str(), "Category");
    assert!(!fields[1].numeric);
    assert_eq!(fields[1].aggregate, Aggregate::Count);
    assert_eq!(fields[1].items, 2);
}

#[test]
fn pivot_fields_counts_cached_formula_empty_as_nonnumeric_but_skips_physical_blank() {
    use yggdryl::Scalar;
    use yggdryl::excel::{Aggregate, Cell, DateSystem, Formula, PivotSource, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    sheet
        .set_cell("A1".parse().unwrap(), "OnlyNumbers")
        .unwrap();
    sheet
        .set_cell("B1".parse().unwrap(), "HasFormulaEmpty")
        .unwrap();
    sheet.set_cell("A2".parse().unwrap(), 2.0).unwrap();
    sheet.set_cell("B2".parse().unwrap(), 2.0).unwrap();
    let at = "B3".parse().unwrap();
    sheet
        .insert_cell(
            Cell::from_scalar(at, Scalar::from(""), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("\"\"", at)),
        )
        .unwrap();
    let source = PivotSource {
        sheet: "Data".into(),
        range: "A1:B3".parse().unwrap(),
    };
    let fields = book.pivot_fields(&source).unwrap();
    assert_eq!(fields[0].aggregate, Aggregate::Sum);
    assert_eq!(fields[1].aggregate, Aggregate::Count);
}

mod pivot_label_bounds {
    use smol_str::SmolStr;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };

    fn fixture(header: &str) -> (Workbook, PivotSpec) {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        data.set_cell("A1".parse().unwrap(), "Group").unwrap();
        data.set_cell("B1".parse().unwrap(), header).unwrap();
        data.set_cell("A2".parse().unwrap(), "East").unwrap();
        data.set_cell("B2".parse().unwrap(), 3.0).unwrap();
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: SmolStr::new("Good"),
            source: PivotSource {
                sheet: SmolStr::new("Data"),
                range: "A1:B2".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: SmolStr::new("Group"),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: SmolStr::new(header),
                aggregate: Aggregate::Sum,
                caption: Some(SmolStr::new("Sum Value")),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: true,
        };
        (book, spec)
    }

    #[test]
    fn typed_pivot_name_over_255_refuses_atomically() {
        let (mut book, mut spec) = fixture("Value");
        spec.name = SmolStr::new("N".repeat(256));
        let before = book.into_bytes().unwrap();
        let error = book
            .add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("$.name"), "{error}");
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        assert!(book.pivots().unwrap().is_empty());
    }

    #[test]
    fn typed_and_generated_pivot_captions_over_255_refuse_atomically() {
        for generated in [false, true] {
            let header = if generated {
                "V".repeat(253)
            } else {
                "Value".to_owned()
            };
            let (mut book, mut spec) = fixture(&header);
            spec.values[0].caption = if generated {
                None
            } else {
                Some(SmolStr::new("C".repeat(256)))
            };
            let before = book.into_bytes().unwrap();
            let error = book
                .add_pivot(spec, "Report", "A3".parse().unwrap())
                .unwrap_err();
            assert!(error.to_string().contains("$.values[0].caption"), "{error}");
            assert_eq!(
                crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
                crate::excel_package::exact_parts(&before)
            );
            assert!(book.pivots().unwrap().is_empty());
        }
    }
}

mod native_general {
    use super::*;
    use yggdryl::excel::{AxisField, CellRef, ItemOrder, PivotSource, ValueField, Workbook};

    #[test]
    fn pivot_general_two_column_three_values_matches_native_geometry_and_numbers() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_layout_c2v3_native.json")).unwrap();
        assert_eq!(fixture["refresh_save_reopen_equal"], true);
        assert_eq!(fixture["rust_equivalence_checked"], false);
        assert_eq!(fixture["column_item_classes"]["item"], 15);
        assert_eq!(fixture["column_item_classes"]["default"], 9);
        assert_eq!(fixture["column_item_classes"]["grand"], 3);
        let mut book = Workbook::new();
        let source = book.add_sheet("Source").unwrap();
        for (row_index, row) in fixture["source_rows"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            for (column_index, value) in row.as_array().unwrap().iter().enumerate() {
                let at = CellRef::new(row_index as u32, column_index as u32);
                match value {
                    serde_json::Value::Null => {}
                    serde_json::Value::String(text) => {
                        source.set_cell(at, text.as_str()).unwrap();
                    }
                    serde_json::Value::Number(number) => {
                        source.set_cell(at, number.as_f64().unwrap()).unwrap();
                    }
                    other => panic!("unsupported native source scalar {other}"),
                }
            }
        }
        book.add_sheet("Case").unwrap();
        let spec = PivotSpec {
            name: "P6_two_column_three_values".into(),
            source: PivotSource {
                sheet: "Source".into(),
                range: "A1:F9".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: "Region".into(),
                order: ItemOrder::Ascending,
            }],
            columns: ["Product", "Year"]
                .into_iter()
                .map(|field| AxisField {
                    field: field.into(),
                    order: ItemOrder::Ascending,
                })
                .collect(),
            values: [
                ("Sales", "Sales Sum"),
                ("Units", "Units Sum"),
                ("Qty", "Qty Sum"),
            ]
            .into_iter()
            .map(|(field, caption)| ValueField {
                field: field.into(),
                aggregate: Aggregate::Sum,
                caption: Some(caption.into()),
                number_format: None,
            })
            .collect(),
            // A singleton row field has no row-parent subtotal; column-parent
            // subtotal events still occur in the native 2-column layout.
            subtotals: true,
            row_grand_totals: true,
            column_grand_totals: true,
        };
        let location = book.add_pivot(spec, "Case", "A3".parse().unwrap()).unwrap();
        assert_eq!(location.to_string(), "A3:AB10");
        if let Some(path) = std::env::var_os("YGGDRYL_EXCEL_PIVOT_C2V3_OUT") {
            use std::io::Write;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
            output.write_all(&book.into_bytes().unwrap()).unwrap();
        }
        let grid = fixture["value2"].as_array().unwrap();
        assert_eq!(grid.len(), 8);
        assert!(grid.iter().all(|row| row.as_array().unwrap().len() == 28));
        for (row, column, label) in [
            (0, 1, "Product"),
            (0, 2, "Year"),
            (1, 1, "Apples"),
            (1, 7, "Sales Sum Apples"),
            (1, 10, "Pears"),
            (1, 16, "Sales Sum Pears"),
            (1, 19, "(vide)"),
            (1, 22, "Sales Sum (vide)"),
            (1, 25, "Total Sales Sum"),
            (4, 0, "East"),
            (5, 0, "West"),
            (6, 0, "(vide)"),
        ] {
            assert_eq!(grid[row][column]["value"].as_str(), Some(label));
        }
        // Default-native blanks sort last; authored `(blank)` sorts first.
        // authored_blank_caption_matches_its_own_excel_refreshed_grid owns that
        // order. Align native axis items here, retaining all bits/empty checks.
        let native_rows = [0, 1, 2, 3, 6, 4, 5, 7];
        // Blank leaf/subtotal, Apples leaves/subtotal, Pears leaves/subtotal, grand.
        let native_column_groups = [6, 7, 0, 1, 2, 3, 4, 5, 8];
        for pass in 0..2 {
            let sheet = book.sheet("Case").unwrap();
            for (at, label) in [
                ("B4", "(blank)"),
                ("H4", "Apples"),
                ("Q4", "Pears"),
                ("A7", "(blank)"),
                ("A8", "East"),
                ("A9", "West"),
            ] {
                assert_eq!(
                    sheet.scalar(at.parse().unwrap()).as_str(),
                    Some(label),
                    "pass={pass} {at}"
                );
            }
            let mut compared = [[false; 28]; 8];
            for (row_index, &native_row) in native_rows.iter().enumerate() {
                for column_index in 0..28 {
                    // Field-selector headers and row labels retain their columns.
                    let native_column = if row_index == 0 || column_index == 0 {
                        column_index
                    } else {
                        let data_column = column_index - 1;
                        1 + native_column_groups[data_column / 3] * 3 + data_column % 3
                    };
                    assert!(!compared[native_row][native_column]);
                    compared[native_row][native_column] = true;
                    let expected = &grid[native_row][native_column];
                    let at = CellRef::new(2 + row_index as u32, column_index as u32);
                    match expected["variant"].as_str().unwrap() {
                        "float" => assert_eq!(
                            sheet.scalar(at).as_f64().unwrap().to_bits(),
                            expected["value"].as_f64().unwrap().to_bits(),
                            "pass={pass} {at}"
                        ),
                        "empty" => assert_eq!(sheet.scalar(at), Scalar::Null, "pass={pass} {at}"),
                        // Locale-selected native captions/blank labels are checked
                        // separately; this fixture pins aligned numeric values and bits.
                        "str" => {}
                        other => panic!("unsupported native grid cell {other}"),
                    }
                }
            }
            assert!(compared.iter().flatten().all(|compared| *compared));
            if pass == 0 {
                book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            }
        }
    }
}

mod native_matrix_errors {
    use super::*;
    use yggdryl::excel::{
        AxisField, Cell, CellRef, DateSystem, ExcelError, ItemOrder, PivotSource, ValueField,
        Workbook,
    };

    fn aggregate(name: &str) -> Aggregate {
        match name {
            "sum" => Aggregate::Sum,
            "count" => Aggregate::Count,
            "average" => Aggregate::Average,
            "max" => Aggregate::Max,
            "min" => Aggregate::Min,
            "product" => Aggregate::Product,
            "countNumbers" => Aggregate::CountNumbers,
            "stdDev" => Aggregate::StdDev,
            "stdDevP" => Aggregate::StdDevP,
            "var" => Aggregate::Var,
            "varP" => Aggregate::VarP,
            other => panic!("unrecognized native aggregate {other}"),
        }
    }

    #[test]
    fn pivot_general_matrix_error_order_matches_native() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/pivot_matrix_error_order_native.json"
        ))
        .unwrap();
        assert_eq!(fixture["cleanup_completed"], true);
        assert_eq!(fixture["cases"].as_array().unwrap().len(), 11);
        for case in fixture["cases"].as_array().unwrap() {
            let mut book = Workbook::new();
            let source = book.add_sheet("Data").unwrap();
            for (row_index, row) in fixture["authored_source_tables"][0]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                for (column_index, value) in row.as_array().unwrap().iter().enumerate() {
                    let at = CellRef::new(row_index as u32, column_index as u32);
                    match value {
                        serde_json::Value::String(text) if text.starts_with('#') => {
                            source
                                .insert_cell(
                                    Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                                        .unwrap()
                                        .with_error(ExcelError::from_text(text)),
                                )
                                .unwrap();
                        }
                        serde_json::Value::String(text) => {
                            source.set_cell(at, text.as_str()).unwrap();
                        }
                        serde_json::Value::Number(number) => {
                            source.set_cell(at, number.as_f64().unwrap()).unwrap();
                        }
                        other => panic!("unsupported source scalar {other}"),
                    }
                }
            }
            book.add_sheet("Result").unwrap();
            let spec = PivotSpec {
                name: format!("P6_{}", case["id"].as_str().unwrap()).into(),
                source: PivotSource {
                    sheet: "Data".into(),
                    range: "A1:C5".parse().unwrap(),
                },
                rows: vec![AxisField {
                    field: "Group".into(),
                    order: ItemOrder::Ascending,
                }],
                columns: vec![AxisField {
                    field: "Column".into(),
                    order: ItemOrder::Ascending,
                }],
                values: vec![ValueField {
                    field: "Metric".into(),
                    aggregate: aggregate(case["aggregate"].as_str().unwrap()),
                    caption: Some(case["data_field"]["caption"].as_str().unwrap().into()),
                    number_format: None,
                }],
                subtotals: false,
                row_grand_totals: true,
                column_grand_totals: true,
            };
            let range = book
                .add_pivot(spec, "Result", "A3".parse().unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            assert_eq!(range.to_string(), "A3:D7");
            for pass in 0..2 {
                let sheet = book.sheet("Result").unwrap();
                for (row_index, row) in case["value2"].as_array().unwrap().iter().enumerate() {
                    for (column_index, expected) in row.as_array().unwrap().iter().enumerate() {
                        let at = CellRef::new(2 + row_index as u32, column_index as u32);
                        match expected["variant"].as_str().unwrap() {
                            "int" => {
                                let error = match expected["value"].as_i64().unwrap() {
                                    -2146826246 => ExcelError::NA,
                                    -2146826281 => ExcelError::Div0,
                                    other => panic!("unproved native error {other}"),
                                };
                                assert_eq!(
                                    sheet.cell(at).and_then(|cell| cell.error()),
                                    Some(error),
                                    "{} pass={pass} {at}",
                                    case["id"]
                                );
                            }
                            "float" => assert_eq!(
                                sheet.scalar(at).as_f64().unwrap().to_bits(),
                                expected["value"].as_f64().unwrap().to_bits(),
                                "{} pass={pass} {at}",
                                case["id"]
                            ),
                            "empty" => assert_eq!(
                                sheet.scalar(at),
                                Scalar::Null,
                                "{} pass={pass} {at}",
                                case["id"]
                            ),
                            "str" => {} // locale-selected captions have a separate wire contract
                            other => panic!("unproved native cell kind {other}"),
                        }
                    }
                }
                if pass == 0 {
                    book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
                }
            }
        }
    }
}

mod pivot_date_axis_publication {
    use smol_str::SmolStr;
    use yggdryl::excel::{
        Aggregate, AxisField, Cell, CellRef, ItemOrder, PivotSource, PivotSpec, ValueField,
        Workbook,
    };
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    fn source(date1904: bool) -> Workbook {
        // The source cells use the same six raw serials observed in
        // pivot_date_native.json, but all are styled dates so one axis can
        // exercise the retained-serial publication path without mixed types.
        let mut data = String::from(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Group</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>Value</t></is></c></row>",
        );
        for (row, serial, style, value) in [
            (2, "0", 1, 1),
            (3, "1", 1, 2),
            (4, "59", 1, 4),
            (5, "60", 1, 8),
            (6, "60.5", 1, 16),
            (7, "61", 1, 32),
        ] {
            data.push_str(&format!("<row r=\"{row}\"><c r=\"A{row}\" s=\"{style}\"><v>{serial}</v></c><c r=\"B{row}\"><v>{value}</v></c></row>"));
        }
        let parts = [
            (
                "[Content_Types].xml",
                crate::excel_package::content_types(1, false, true),
            ),
            ("_rels/.rels", crate::excel_package::root_relationships()),
            (
                "xl/workbook.xml",
                crate::excel_package::workbook(&["Data"], date1904),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                crate::excel_package::workbook_relationships(1, false, true),
            ),
            (
                "xl/worksheets/sheet1.xml",
                crate::excel_package::worksheet(&data),
            ),
            ("xl/styles.xml", crate::excel_package::styles(&[], &[0, 14])),
        ];
        let mut workbook = Workbook::from_bytes(crate::excel_package::package(&parts)).unwrap();
        workbook.add_sheet("Report").unwrap();
        workbook
    }

    fn raw_cells(bytes: &[u8]) -> String {
        let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            bytes.to_vec(),
        ))));
        String::from_utf8(archive.read_member("xl/worksheets/sheet2.xml").unwrap()).unwrap()
    }

    fn assert_axis(workbook: &Workbook, bytes: &[u8]) {
        let sheet = workbook.sheet("Report").unwrap();
        let xml = raw_cells(bytes);
        for (row, expected) in [
            (4, 0.0_f64),
            (5, 1.0),
            (6, 59.0),
            (7, 60.0),
            (8, 60.5),
            (9, 61.0),
        ] {
            let at: CellRef = format!("A{row}").parse().unwrap();
            let cell: &Cell = sheet.cell(at).unwrap();
            assert!(cell.value().temporal_unit().is_some(), "{at}: {cell:?}");
            let fragment = xml
                .split_once(&format!("<c r=\"A{row}\""))
                .unwrap()
                .1
                .split_once("</c>")
                .unwrap()
                .0;
            let written: f64 = fragment
                .split_once("<v>")
                .unwrap()
                .1
                .split_once("</v>")
                .unwrap()
                .0
                .parse()
                .unwrap();
            assert_eq!(written.to_bits(), expected.to_bits(), "{at}: {fragment}");
        }
    }

    #[test]
    fn pivot_date_axis_keeps_native_raw_serials_before_and_after_reopen() {
        for date1904 in [false, true] {
            let mut workbook = source(date1904);
            let spec = PivotSpec {
                name: SmolStr::new("Dates"),
                source: PivotSource {
                    sheet: SmolStr::new("Data"),
                    range: "A1:B7".parse().unwrap(),
                },
                rows: vec![AxisField {
                    field: SmolStr::new("Group"),
                    order: ItemOrder::Ascending,
                }],
                columns: Vec::new(),
                values: vec![ValueField {
                    field: SmolStr::new("Value"),
                    aggregate: Aggregate::Sum,
                    caption: Some(SmolStr::new("Value Sum")),
                    number_format: None,
                }],
                subtotals: false,
                row_grand_totals: false,
                column_grand_totals: false,
            };
            assert_eq!(
                workbook
                    .add_pivot(spec, "Report", "A3".parse().unwrap())
                    .unwrap()
                    .to_string(),
                "A3:B9"
            );
            let saved = workbook.into_bytes().unwrap();
            assert_axis(&workbook, &saved);
            let reopened = Workbook::from_bytes(saved).unwrap();
            assert_axis(&reopened, &reopened.into_bytes().unwrap());
        }
    }
}

mod pivot_publication_audit {
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;
    use yggdryl::{Error, Scalar};

    fn fixture() -> (Workbook, PivotSpec) {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        for (at, value) in [
            ("A1", "Region"),
            ("B1", "Value"),
            ("C1", "Period"),
            ("A2", "East"),
            ("C2", "Now"),
        ] {
            data.set_cell(at.parse().unwrap(), value).unwrap();
        }
        data.set_cell("B2".parse().unwrap(), 3.0).unwrap();
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: "Audit".into(),
            source: PivotSource {
                sheet: "Data".into(),
                range: "A1:C2".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: "Region".into(),
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
            column_grand_totals: false,
        };
        (book, spec)
    }

    fn member(book: &Workbook, path: &str) -> String {
        let bytes = book.into_bytes().unwrap();
        let zip = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(bytes))));
        String::from_utf8(zip.read_member(path).unwrap()).unwrap()
    }

    #[test]
    fn pivot_publication_source_and_host_aliases_resolve_to_canonical_sheets() {
        let (mut book, mut spec) = fixture();
        spec.source.sheet = "data".into();
        spec.rows[0].field = "region".into();
        spec.values[0].field = "value".into();
        assert_eq!(
            book.pivot_fields(&spec.source).unwrap()[0].name.as_str(),
            "Region"
        );
        book.add_pivot(spec, "report", "A3".parse().unwrap())
            .unwrap();
        assert_eq!(
            book.sheet("Report").unwrap().scalar("A3".parse().unwrap()),
            Scalar::from("Region")
        );
        let pivot = &book.pivots().unwrap()[0];
        assert_eq!(pivot.host_sheet(), "Report");
        assert_eq!(pivot.spec().unwrap().source.sheet.as_str(), "Data");
        assert_eq!(pivot.spec().unwrap().rows[0].field.as_str(), "Region");
        let mut reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened.pivots().unwrap()[0]
                .spec()
                .unwrap()
                .source
                .sheet
                .as_str(),
            "Data"
        );
        reopened.refresh_pivot("report", "Audit").unwrap();
        let mut changed = reopened.pivots().unwrap()[0].spec().unwrap().clone();
        changed.values[0].caption = Some("Updated".into());
        reopened.update_pivot("REPORT", "Audit", changed).unwrap();
        reopened.remove_pivot("rEpOrT", "Audit").unwrap();
        assert!(reopened.pivots().unwrap().is_empty());
    }

    #[test]
    fn pivot_publication_host_alias_cannot_bypass_empty_source_overlap() {
        let (mut book, mut spec) = fixture();
        // The output is physically empty, but still belongs to the source.
        spec.source.range = "A1:C100".parse().unwrap();
        let before = book.into_bytes().unwrap();
        let error = book
            .add_pivot(spec, "data", "A50".parse().unwrap())
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::Conflict {
                    expected: "a pivot output outside its source rectangle",
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(
            crate::excel_package::exact_parts(&book.into_bytes().unwrap()),
            crate::excel_package::exact_parts(&before)
        );
        assert!(book.pivots().unwrap().is_empty());
    }

    #[test]
    fn pivot_publication_imported_update_replaces_optional_column_fields_and_name() {
        for columns_before in [false, true] {
            let (mut book, mut spec) = fixture();
            if columns_before {
                spec.columns = vec![AxisField {
                    field: "Period".into(),
                    order: ItemOrder::Ascending,
                }];
            }
            book.add_pivot(spec.clone(), "Report", "A3".parse().unwrap())
                .unwrap();
            let mut book = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            assert!(book.pivots().unwrap()[0].editable());
            spec.name = "Renamed".into();
            spec.columns = if columns_before {
                Vec::new()
            } else {
                vec![AxisField {
                    field: "Period".into(),
                    order: ItemOrder::Ascending,
                }]
            };
            book.update_pivot("Report", "Audit", spec.clone()).unwrap();
            let xml = member(&book, "xl/pivotTables/pivotTable1.xml");
            assert_eq!(xml.contains("<colFields"), !columns_before, "{xml}");
            assert!(xml.contains("name=\"Renamed\""), "{xml}");
            let mut reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
            let pivot = &reopened.pivots().unwrap()[0];
            assert_eq!(pivot.name(), "Renamed");
            assert_eq!(pivot.spec().unwrap().columns, spec.columns);
            reopened.refresh_pivot("Report", "Renamed").unwrap();
        }
    }

    fn pivot_with_shadow_relationship(scope: u8) -> (Workbook, String) {
        const OPC: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
        let (mut book, spec) = fixture();
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        let saved = book.into_bytes().unwrap();
        let zip = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(saved))));
        let mut parts: Vec<(String, Vec<u8>)> = zip
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    zip.read_member(entry.name()).unwrap(),
                )
            })
            .collect();
        let rels = parts
            .iter_mut()
            .find(|(name, _)| name == "xl/worksheets/_rels/sheet2.xml.rels")
            .unwrap();
        let mut xml = String::from_utf8(rels.1.clone()).unwrap();
        let fake = if scope == 1 {
            format!(
                "<v:wrapper xmlns:v=\"urn:foreign\"><Relationship xmlns=\"{OPC}\" Id=\"shadow\" Type=\"urn:vendor\" Target=\"../pivotTables/pivotTable1.xml\"/></v:wrapper>"
            )
        } else if scope == 0 {
            "<Relationship xmlns=\"urn:foreign\" Id=\"shadow\" Type=\"urn:vendor\" Target=\"../pivotTables/pivotTable1.xml\"/>".to_owned()
        } else {
            format!(
                "<Relationship xmlns=\"{OPC}\" Id=\"shadow\" Type=\"urn:vendor\" Target=\"../pivotTables/pivotTable1.xml\"/>"
            )
        };
        let before = xml.find("<Relationship ").unwrap();
        xml.insert_str(before, &fake);
        rels.1 = xml.into_bytes();
        let borrowed: Vec<_> = parts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        (
            Workbook::from_bytes(crate::excel_package::package(&borrowed)).unwrap(),
            fake,
        )
    }

    #[test]
    fn pivot_publication_remove_ignores_foreign_relationship_lookalikes() {
        for scope in [0, 1] {
            let (mut book, fake) = pivot_with_shadow_relationship(scope);
            assert_eq!(book.pivots().unwrap().len(), 1);
            book.remove_pivot("Report", "Audit").unwrap();
            let after = member(&book, "xl/worksheets/_rels/sheet2.xml.rels");
            assert!(
                after.contains(&fake),
                "opaque registration changed: {after}"
            );
            assert!(book.pivots().unwrap().is_empty());
            assert!(
                Workbook::from_bytes(book.into_bytes().unwrap())
                    .unwrap()
                    .pivots()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn pivot_publication_remove_refuses_a_shared_opc_target_atomically() {
        let (mut book, _) = pivot_with_shadow_relationship(2);
        let before = book.into_bytes().unwrap();
        let error = book.remove_pivot("Report", "Audit").unwrap_err();
        assert!(matches!(error, Error::Unsupported { .. }), "{error}");
        assert_eq!(book.into_bytes().unwrap(), before);
        assert_eq!(book.pivots().unwrap().len(), 1);
        assert_eq!(
            book.sheet("Report").unwrap().scalar("B4".parse().unwrap()),
            Scalar::from(3.0)
        );
    }
    #[test]
    fn pivot_publication_xstrings_preserve_controls_and_literal_escape_spellings() {
        let (mut book, mut spec) = fixture();
        let name = "Pivot_x0041_";
        let header = "Region\u{0}\"<&_x0041_";
        let caption = "Sum\u{b}\"<&_x0042_";
        let item = "East\u{1}\"<&_x0043_";
        book.sheet_mut("Data")
            .unwrap()
            .set_cell("A1".parse().unwrap(), header)
            .unwrap();
        book.sheet_mut("Data")
            .unwrap()
            .set_cell("A2".parse().unwrap(), item)
            .unwrap();
        spec.name = name.into();
        spec.rows[0].field = header.into();
        spec.values[0].caption = Some(caption.into());
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        let table = member(&book, "xl/pivotTables/pivotTable1.xml");
        let cache = member(&book, "xl/pivotCache/pivotCacheDefinition1.xml");
        assert!(!table.contains('\u{b}') && !cache.contains('\u{0}') && !cache.contains('\u{1}'));
        for spelling in [
            "_x0000_",
            "_x0001_",
            "_x005F_x0041_",
            "_x005F_x0043_",
            "&quot;",
            "&lt;",
            "&amp;",
        ] {
            assert!(cache.contains(spelling), "missing {spelling}: {cache}");
        }
        assert!(
            table.contains("_x000B_")
                && table.contains("_x005F_x0042_")
                && table.contains("_x005F_x0041_"),
            "{table}"
        );
        let mut reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        let pivot = &reopened.pivots().unwrap()[0];
        assert_eq!(pivot.name(), name);
        let spec = pivot.spec().unwrap();
        assert_eq!(spec.rows[0].field.as_str(), header);
        assert_eq!(spec.values[0].caption.as_deref(), Some(caption));
        assert_eq!(
            reopened
                .sheet("Report")
                .unwrap()
                .scalar("A4".parse().unwrap()),
            Scalar::from(item)
        );
        reopened.refresh_pivot("Report", name).unwrap();
    }
}

#[test]
fn pivot_publication_reopens_all_eleven_aggregate_spellings() {
    use yggdryl::excel::{AxisField, ItemOrder, PivotSource, ValueField, Workbook};
    for aggregate in [
        Aggregate::Sum,
        Aggregate::Count,
        Aggregate::Average,
        Aggregate::Max,
        Aggregate::Min,
        Aggregate::Product,
        Aggregate::CountNumbers,
        Aggregate::StdDev,
        Aggregate::StdDevP,
        Aggregate::Var,
        Aggregate::VarP,
    ] {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        for (at, text) in [
            ("A1", "Group"),
            ("B1", "Value"),
            ("A2", "One"),
            ("A3", "One"),
        ] {
            data.set_cell(at.parse().unwrap(), text).unwrap();
        }
        data.set_cell("B2".parse().unwrap(), 1.0).unwrap();
        data.set_cell("B3".parse().unwrap(), 3.0).unwrap();
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: "Kinds".into(),
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
                aggregate,
                caption: Some("Metric".into()),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: false,
        };
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        let before = book.sheet("Report").unwrap().scalar("B4".parse().unwrap());
        let mut reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        let pivot = &reopened.pivots().unwrap()[0];
        assert!(pivot.editable(), "{aggregate:?}: {:?}", pivot.origin());
        assert_eq!(pivot.spec().unwrap().values[0].aggregate, aggregate);
        reopened.refresh_pivot("Report", "Kinds").unwrap();
        assert_eq!(
            reopened
                .sheet("Report")
                .unwrap()
                .scalar("B4".parse().unwrap()),
            before,
            "{aggregate:?}"
        );
    }
}

mod pivot_orphan_cache_id {
    use smol_str::SmolStr;
    use std::collections::BTreeMap;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    fn with_unreferenced_cache(mut book: Workbook) -> Workbook {
        let bytes = book.into_bytes().unwrap();
        let archive =
            std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(bytes))));
        let mut members: BTreeMap<String, Vec<u8>> = archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    archive.read_member(entry.name()).unwrap(),
                )
            })
            .collect();
        let edit =
            |member: &str, end: &str, insertion: &str, members: &mut BTreeMap<String, Vec<u8>>| {
                let bytes = members.get_mut(member).unwrap();
                let text = String::from_utf8(std::mem::take(bytes)).unwrap();
                assert_eq!(text.matches(end).count(), 1);
                *bytes = text.replace(end, &format!("{insertion}{end}")).into_bytes();
            };
        edit(
            "xl/workbook.xml",
            "</workbook>",
            "<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId99\"/></pivotCaches>",
            &mut members,
        );
        edit(
            "xl/_rels/workbook.xml.rels",
            "</Relationships>",
            "<Relationship Id=\"rId99\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition\" Target=\"pivotCache/pivotCacheDefinition99.xml\"/>",
            &mut members,
        );
        edit(
            "[Content_Types].xml",
            "</Types>",
            "<Override PartName=\"/xl/pivotCache/pivotCacheDefinition99.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml\"/>",
            &mut members,
        );
        members.insert("xl/pivotCache/pivotCacheDefinition99.xml".to_owned(),
            format!("<pivotCacheDefinition xmlns=\"{}\" recordCount=\"0\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"A1:B2\" sheet=\"Data\"/></cacheSource><cacheFields count=\"2\"><cacheField name=\"Group\"/><cacheField name=\"Value\"/></cacheFields></pivotCacheDefinition>", crate::excel_package::NS).into_bytes());
        let parts: Vec<(&str, Vec<u8>)> = members
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        book = Workbook::from_bytes(crate::excel_package::package(&parts)).unwrap();
        book
    }

    #[test]
    fn new_pivot_cache_id_accounts_for_unreferenced_workbook_cache() {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        for (at, value) in [("A1", "Group"), ("B1", "Value"), ("A2", "East")] {
            data.set_cell(at.parse().unwrap(), value).unwrap();
        }
        data.set_cell("B2".parse().unwrap(), 3.0).unwrap();
        book.add_sheet("Report").unwrap();
        let mut book = with_unreferenced_cache(book);
        assert!(book.pivots().unwrap().is_empty());
        let spec = PivotSpec {
            name: SmolStr::new("New"),
            source: PivotSource {
                sheet: SmolStr::new("Data"),
                range: "A1:B2".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: SmolStr::new("Group"),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: SmolStr::new("Value"),
                aggregate: Aggregate::Sum,
                caption: Some(SmolStr::new("Sum Value")),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: false,
        };
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        let bytes = book.into_bytes().unwrap();
        let archive =
            std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(bytes))));
        let workbook = String::from_utf8(archive.read_member("xl/workbook.xml").unwrap()).unwrap();
        assert!(workbook.contains("cacheId=\"1\""), "{workbook}");
        assert!(workbook.contains("cacheId=\"2\""), "{workbook}");
        let pivot = String::from_utf8(
            archive
                .read_member("xl/pivotTables/pivotTable1.xml")
                .unwrap(),
        )
        .unwrap();
        assert!(pivot.contains("cacheId=\"2\""), "{pivot}");
        assert_eq!(
            Workbook::from_bytes(book.into_bytes().unwrap())
                .unwrap()
                .pivots()
                .unwrap()
                .len(),
            1
        );
    }
}

mod pivot_explicit_captions {
    use smol_str::SmolStr;
    use yggdryl::excel::{
        Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    fn member(bytes: &[u8], name: &str) -> String {
        let archive = std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            bytes.to_vec(),
        ))));
        String::from_utf8(archive.read_member(name).unwrap()).unwrap()
    }

    #[test]
    fn authored_boolean_and_grand_captions_are_typed_text_and_persist_in_xml() {
        let mut book = Workbook::new();
        let data = book.add_sheet("Data").unwrap();
        data.set_cell("A1".parse().unwrap(), "Group").unwrap();
        data.set_cell("B1".parse().unwrap(), "Value").unwrap();
        data.set_cell("A2".parse().unwrap(), false).unwrap();
        data.set_cell("B2".parse().unwrap(), 1.0).unwrap();
        data.set_cell("A3".parse().unwrap(), true).unwrap();
        data.set_cell("B3".parse().unwrap(), 2.0).unwrap();
        data.set_cell("B4".parse().unwrap(), 4.0).unwrap();
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: SmolStr::new("BoolLabels"),
            source: PivotSource {
                sheet: SmolStr::new("Data"),
                range: "A1:B4".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: SmolStr::new("Group"),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: SmolStr::new("Value"),
                aggregate: Aggregate::Sum,
                caption: Some(SmolStr::new("Value Sum")),
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: false,
            column_grand_totals: true,
        };
        book.add_pivot(spec, "Report", "A3".parse().unwrap())
            .unwrap();
        // Native explicit blank sorts before Boolean; authored item@n fixes English Boolean labels.
        for (at, text) in [
            ("A4", "(blank)"),
            ("A5", "FALSE"),
            ("A6", "TRUE"),
            ("A7", "Grand Total"),
        ] {
            assert_eq!(
                book.sheet("Report")
                    .unwrap()
                    .cell(at.parse().unwrap())
                    .unwrap()
                    .value()
                    .as_str(),
                Some(text)
            );
        }
        let saved = book.into_bytes().unwrap();
        let table = member(&saved, "xl/pivotTables/pivotTable1.xml");
        assert!(table.contains("dataCaption=\"Values\""), "{table}");
        assert!(
            table.contains("grandTotalCaption=\"Grand Total\""),
            "{table}"
        );
        assert!(table.contains("n=\"FALSE\""), "{table}");
        assert!(table.contains("n=\"TRUE\""), "{table}");
        assert!(table.contains("n=\"(blank)\""), "{table}");
        let reopened = Workbook::from_bytes(saved).unwrap();
        let imported = reopened
            .pivots()
            .unwrap()
            .iter()
            .find(|pivot| pivot.name() == "BoolLabels")
            .unwrap();
        assert!(imported.editable(), "{:?}", imported.origin());
        for (at, text) in [
            ("A4", "(blank)"),
            ("A5", "FALSE"),
            ("A6", "TRUE"),
            ("A7", "Grand Total"),
        ] {
            assert_eq!(
                reopened
                    .sheet("Report")
                    .unwrap()
                    .cell(at.parse().unwrap())
                    .unwrap()
                    .value()
                    .as_str(),
                Some(text)
            );
        }
    }
}

mod pivot_cache_relationship_identity {
    use std::collections::BTreeMap;
    use yggdryl::Error;
    use yggdryl::excel::Workbook;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    #[test]
    fn pivot_table_cache_relationship_must_name_its_workbook_cache_id_target() {
        let bytes = include_bytes!("fixtures/pivot_excel.xlsx").to_vec();
        let archive =
            std::sync::Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(bytes))));
        let mut members: BTreeMap<String, Vec<u8>> = archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    archive.read_member(entry.name()).unwrap(),
                )
            })
            .collect();
        let rels = "xl/pivotTables/_rels/pivotTable1.xml.rels";
        let xml = String::from_utf8(members.get(rels).unwrap().clone()).unwrap();
        assert_eq!(xml.matches("pivotCacheDefinition1.xml").count(), 1);
        members.insert(
            rels.to_owned(),
            xml.replace("pivotCacheDefinition1.xml", "pivotCacheDefinition2.xml")
                .into_bytes(),
        );
        let parts: Vec<(&str, Vec<u8>)> = members
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        let book = Workbook::from_bytes(crate::excel_package::package(&parts)).unwrap();
        match book.pivots().unwrap_err() {
            Error::InvalidRecord { path, reason } => {
                assert!(path.contains(rels), "{path}");
                assert!(reason.contains("cache"), "{reason}");
            }
            error => panic!("expected located cache-identity refusal, got {error:?}"),
        }
    }
}

mod pivot_shared_items_metadata {
    use smol_str::SmolStr;
    use yggdryl::excel::{
        Aggregate, AxisField, CellRef, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
    };
    fn at(value: &str) -> CellRef {
        value.parse().unwrap()
    }

    #[test]
    fn pivot_shared_items_numeric_metadata_replays_typed_cache_contract() {
        use std::sync::Arc;
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::zip::ZipArchive;

        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        source.set_cell(at("A1"), "Group").unwrap();
        source.set_cell(at("B1"), "Value").unwrap();
        for (index, number) in [-1.0, 0.0, 2.0].into_iter().enumerate() {
            let row = index as u32 + 1;
            source.set_cell(CellRef::new(row, 0), number).unwrap();
            source.set_cell(CellRef::new(row, 1), 1.0).unwrap();
        }
        book.add_sheet("Report").unwrap();
        let spec = PivotSpec {
            name: SmolStr::new_static("Metadata"),
            source: PivotSource {
                sheet: SmolStr::new_static("Data"),
                range: "A1:B4".parse().unwrap(),
            },
            rows: vec![AxisField {
                field: SmolStr::new_static("Group"),
                order: ItemOrder::Ascending,
            }],
            columns: Vec::new(),
            values: vec![ValueField {
                field: SmolStr::new_static("Value"),
                aggregate: Aggregate::Sum,
                caption: None,
                number_format: None,
            }],
            subtotals: false,
            row_grand_totals: true,
            column_grand_totals: true,
        };
        book.add_pivot(spec, "Report", at("A3")).unwrap();
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            book.into_bytes().unwrap(),
        ))));
        let cache = String::from_utf8(
            archive
                .read_member("xl/pivotCache/pivotCacheDefinition1.xml")
                .unwrap(),
        )
        .unwrap();
        let shared = cache
            .split("<sharedItems")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap();
        for attribute in [
            "containsNumber=\"1\"",
            "containsInteger=\"1\"",
            "minValue=\"-1\"",
            "maxValue=\"2\"",
            "count=\"3\"",
        ] {
            assert!(
                shared.contains(attribute),
                "missing {attribute} in {shared}"
            );
        }
        assert!(!shared.contains("containsDate"), "{shared}");
        assert!(!shared.contains("containsMixedTypes"), "{shared}");
    }

    #[test]
    fn pivot_shared_items_replay_fourteen_native_typed_cache_schemas() {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use quick_xml::Reader;
        use quick_xml::events::{BytesStart, Event};
        use serde_json::Value;
        use yggdryl::holder::{Buffer, Holder};
        use yggdryl::zip::ZipArchive;

        fn attributes(start: &BytesStart<'_>) -> BTreeMap<String, String> {
            start
                .attributes()
                .map(|attribute| {
                    let attribute = attribute.unwrap();
                    (
                        String::from_utf8(attribute.key.as_ref().to_vec()).unwrap(),
                        String::from_utf8(attribute.value.as_ref().to_vec()).unwrap(),
                    )
                })
                .collect()
        }

        fn expected_map(value: &Value) -> BTreeMap<String, String> {
            value
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
                .collect()
        }

        fn read_cache(
            xml: &[u8],
        ) -> (
            BTreeMap<String, String>,
            BTreeMap<String, String>,
            Vec<(String, BTreeMap<String, String>)>,
        ) {
            let source = std::str::from_utf8(xml).unwrap();
            let mut reader = Reader::from_str(source);
            let mut field = None;
            let mut shared = None;
            let mut items = Vec::new();
            let mut inside_shared = false;
            loop {
                match reader.read_event().unwrap() {
                    Event::Start(start)
                        if start.name().as_ref() == b"cacheField" && field.is_none() =>
                    {
                        field = Some(attributes(&start));
                    }
                    Event::Start(start)
                        if start.name().as_ref() == b"sharedItems"
                            && field.is_some()
                            && shared.is_none() =>
                    {
                        shared = Some(attributes(&start));
                        inside_shared = true;
                    }
                    Event::Empty(start) if inside_shared => {
                        items.push((
                            String::from_utf8(start.name().as_ref().to_vec()).unwrap(),
                            attributes(&start),
                        ));
                    }
                    Event::End(end) if end.name().as_ref() == b"sharedItems" => {
                        inside_shared = false
                    }
                    Event::Eof => break,
                    _ => {}
                }
            }
            (field.unwrap(), shared.unwrap(), items)
        }

        let observed: Value = serde_json::from_str(include_str!(
            "fixtures/pivot_shared_items_metadata_native.json"
        ))
        .unwrap();
        assert_eq!(
            observed["kind"],
            "p6_shared_items_metadata_native_observations"
        );
        assert_eq!(observed["rust_equivalence_checked"], false);
        let mut compared = 0;
        for run in observed["runs"].as_array().unwrap() {
            assert_eq!(run["native_passed"], true);
            assert_eq!(run["cleanup_completed"], true);
            let input = if run["epoch"] == "1900" {
                include_bytes!("fixtures/pivot_shared_items_1900_input.xlsx").as_slice()
            } else {
                include_bytes!("fixtures/pivot_shared_items_1904_input.xlsx").as_slice()
            };
            let mut book = Workbook::from_bytes(input.to_vec()).unwrap();
            for (index, case) in run["cases"].as_array().unwrap().iter().enumerate() {
                let source = format!("Source{:02}", index + 1);
                let target = format!("Case{:02}", index + 1);
                let last = case["items"].as_array().unwrap().len() + 1;
                let spec = PivotSpec {
                    name: SmolStr::new(format!("P6_{}", case["id"].as_str().unwrap())),
                    source: PivotSource {
                        sheet: SmolStr::new(&source),
                        range: format!("A1:B{last}").parse().unwrap(),
                    },
                    rows: vec![AxisField {
                        field: SmolStr::new_static("Group"),
                        order: ItemOrder::Ascending,
                    }],
                    columns: Vec::new(),
                    values: vec![ValueField {
                        field: SmolStr::new_static("Value"),
                        aggregate: Aggregate::Sum,
                        caption: Some(SmolStr::new_static("Value Sum")),
                        number_format: None,
                    }],
                    subtotals: false,
                    row_grand_totals: true,
                    column_grand_totals: true,
                };
                book.add_pivot(spec, &target, at("A3")).unwrap();
            }
            let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
                book.into_bytes().unwrap(),
            ))));
            for (index, case) in run["cases"].as_array().unwrap().iter().enumerate() {
                let xml = archive
                    .read_member(&format!(
                        "xl/pivotCache/pivotCacheDefinition{}.xml",
                        index + 1
                    ))
                    .unwrap();
                let (field, shared, items) = read_cache(&xml);
                let mut native_shared = expected_map(&case["shared_attributes"]);
                // OOXML permits date bounds to be omitted. Excel's observed
                // bounds do not equal extrema of its own <d> cache items.
                native_shared.remove("minDate");
                native_shared.remove("maxDate");
                assert_eq!(
                    field,
                    expected_map(&case["cache_field_attributes"]),
                    "{} {} field",
                    run["epoch"],
                    case["id"]
                );
                assert_eq!(
                    shared, native_shared,
                    "{} {} sharedItems",
                    run["epoch"], case["id"]
                );
                let native_items: Vec<_> = case["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| {
                        (
                            item["kind"].as_str().unwrap().to_owned(),
                            expected_map(&item["attributes"]),
                        )
                    })
                    .collect();
                assert_eq!(
                    items, native_items,
                    "{} {} child items",
                    run["epoch"], case["id"]
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 14);
    }
}

mod pivot_error_axis_native {
    use smol_str::SmolStr;
    use yggdryl::Scalar;
    use yggdryl::excel::{
        Aggregate, AxisField, Cell, CellRef, DateSystem, ExcelError, ItemOrder, PivotSource,
        PivotSpec, ValueField, Workbook,
    };

    #[test]
    fn pivot_error_axis_keeps_typed_cache_error_and_displays_text_label() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/pivot_items_native.json")).unwrap();
        assert_eq!(fixture["all_passed"], true);
        let native = fixture["batches"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|batch| batch["cases"].as_array().unwrap())
            .find(|case| case["id"] == "error_versus_text")
            .unwrap();
        assert_eq!(native["cache_group_items"]["items"][0]["tag"], "e");
        assert_eq!(native["value2"][2][0]["value"], "#DIV/0!");
        assert_eq!(native["value2"][3][0]["value"], "#N/A");

        let mut book = Workbook::new();
        let source = book.add_sheet("Data").unwrap();
        source.set_cell("A1".parse().unwrap(), "Group").unwrap();
        source.set_cell("B1".parse().unwrap(), "Value").unwrap();
        for (row, error, amount) in [(2, ExcelError::NA, 1.0), (3, ExcelError::Div0, 2.0)] {
            let at = CellRef::new(row - 1, 0);
            source
                .insert_cell(
                    Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900)
                        .unwrap()
                        .with_error(error),
                )
                .unwrap();
            source.set_cell(CellRef::new(row - 1, 1), amount).unwrap();
        }
        source.set_cell("A4".parse().unwrap(), "ordinary").unwrap();
        source.set_cell("B4".parse().unwrap(), 4.0).unwrap();
        source.set_cell("B5".parse().unwrap(), 8.0).unwrap();
        book.add_sheet("Report").unwrap();
        book.add_pivot(
            PivotSpec {
                name: SmolStr::new_static("ErrorAxis"),
                source: PivotSource {
                    sheet: SmolStr::new_static("Data"),
                    range: "A1:B5".parse().unwrap(),
                },
                rows: vec![AxisField {
                    field: SmolStr::new_static("Group"),
                    order: ItemOrder::Ascending,
                }],
                columns: Vec::new(),
                values: vec![ValueField {
                    field: SmolStr::new_static("Value"),
                    aggregate: Aggregate::Sum,
                    caption: Some(SmolStr::new_static("Value Sum")),
                    number_format: None,
                }],
                subtotals: false,
                row_grand_totals: true,
                column_grand_totals: true,
            },
            "Report",
            "A3".parse().unwrap(),
        )
        .unwrap();

        // p6-blank-mixed-text_error-ascending: explicit blank precedes text/errors.
        for (row, label, amount) in [
            (4, "(blank)", 8.0),
            (5, "ordinary", 4.0),
            (6, "#DIV/0!", 2.0),
            (7, "#N/A", 1.0),
        ] {
            let report = book.sheet("Report").unwrap();
            let cell = report.cell(CellRef::new(row - 1, 0)).unwrap();
            assert_eq!(cell.value(), &Scalar::from(label));
            assert_eq!(cell.error(), None, "visible error item heading is text");
            assert_eq!(
                report.scalar(CellRef::new(row - 1, 1)),
                Scalar::from(amount)
            );
        }
        let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        let imported = reopened
            .pivots()
            .unwrap()
            .iter()
            .find(|pivot| pivot.name() == "ErrorAxis")
            .unwrap();
        assert!(imported.editable(), "{:?}", imported.origin());
        for (row, label) in [(4, "(blank)"), (5, "ordinary"), (6, "#DIV/0!"), (7, "#N/A")] {
            let cell = reopened
                .sheet("Report")
                .unwrap()
                .cell(CellRef::new(row - 1, 0))
                .unwrap();
            assert_eq!(cell.value(), &Scalar::from(label));
            assert_eq!(cell.error(), None);
        }
    }

    // Local-only exchange export: the separately authored source workbook and
    // five-case manifest are checked by the desktop oracle before Excel opens them.
    #[test]
    #[ignore = "requires YGGDRYL_EXCEL_PIVOT_CASES and YGGDRYL_EXCEL_PIVOT_OUT"]
    fn pivot_export_five_native_shapes_for_excel() {
        use smol_str::SmolStr;
        use yggdryl::excel::{
            Aggregate, AxisField, ItemOrder, PivotSource, PivotSpec, ValueField, Workbook,
        };

        let cases = std::path::PathBuf::from(
            std::env::var_os("YGGDRYL_EXCEL_PIVOT_CASES")
                .expect("set the five-case native manifest"),
        );
        let output = std::path::PathBuf::from(
            std::env::var_os("YGGDRYL_EXCEL_PIVOT_OUT").expect("set a fresh Rust XLSX output path"),
        );
        assert!(!output.exists(), "select a fresh Rust pivot output path");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&cases).unwrap()).unwrap();
        let input = cases
            .parent()
            .unwrap()
            .join(manifest["workbook"]["file"].as_str().unwrap());
        let mut book = Workbook::from_bytes(std::fs::read(input).unwrap()).unwrap();
        let authored = manifest["cases"].as_array().unwrap();
        assert_eq!(authored.len(), 5);
        for case in authored {
            let id = case["id"].as_str().unwrap();
            let axes = |name: &str| {
                case[name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|field| AxisField {
                        field: SmolStr::new(field.as_str().unwrap()),
                        order: ItemOrder::Ascending,
                    })
                    .collect::<Vec<_>>()
            };
            let values = case["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| ValueField {
                    field: SmolStr::new(value["field"].as_str().unwrap()),
                    aggregate: match value["aggregate"].as_str().unwrap() {
                        "sum" => Aggregate::Sum,
                        "average" => Aggregate::Average,
                        other => panic!("unproved five-case aggregate {other}"),
                    },
                    caption: Some(SmolStr::new(value["caption"].as_str().unwrap())),
                    number_format: None,
                })
                .collect();
            let spec = PivotSpec {
                name: SmolStr::new(format!("P6_{id}")),
                source: PivotSource {
                    sheet: SmolStr::new(case["source"]["sheet"].as_str().unwrap()),
                    range: case["source"]["range"].as_str().unwrap().parse().unwrap(),
                },
                rows: axes("rows"),
                columns: axes("columns"),
                values,
                subtotals: case["subtotals"].as_bool().unwrap(),
                row_grand_totals: case["row_grand_totals"].as_bool().unwrap(),
                column_grand_totals: case["column_grand_totals"].as_bool().unwrap(),
            };
            book.add_pivot(
                spec,
                case["sheet"].as_str().unwrap(),
                case["anchor"].as_str().unwrap().parse().unwrap(),
            )
            .unwrap();
        }
        let bytes = book.into_bytes().unwrap();
        assert_eq!(
            Workbook::from_bytes(bytes.clone())
                .unwrap()
                .pivots()
                .unwrap()
                .len(),
            5
        );
        std::fs::write(output, bytes).unwrap();
    }
}

#[test]
fn imported_unrepresentable_item_metadata_stays_read_only_and_byte_exact() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use yggdryl::excel::{PivotOrigin, Workbook};
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    const PART: &str = "xl/pivotTables/pivotTable1.xml";
    let input = include_bytes!("fixtures/pivot_ascending_native.xlsx").to_vec();
    let baseline = Workbook::from_bytes(input.clone()).unwrap();
    let native = baseline
        .pivots()
        .unwrap()
        .iter()
        .find(|pivot| pivot.name() == "P6_matrix_one_value")
        .unwrap();
    assert!(native.editable(), "{:?}", native.origin());

    for (old, new, location) in [
        (
            "<item x=\"2\"/>",
            "<item n=\"CUSTOM_BLANK\" x=\"2\"/>",
            "authored item caption",
        ),
        (
            "<item x=\"1\"/>",
            "<item n=\"CUSTOM_EAST\" x=\"1\"/>",
            "authored item caption",
        ),
        ("<item x=\"1\"/>", "<item h=\"1\" x=\"1\"/>", "hidden item"),
        ("<item x=\"1\"/>", "<item x=\"bogus\"/>", "item index"),
    ] {
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            input.clone(),
        ))));
        let mut members: BTreeMap<String, Vec<u8>> = archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    archive.read_member(entry.name()).unwrap(),
                )
            })
            .collect();
        let xml = String::from_utf8(members.get(PART).unwrap().clone()).unwrap();
        assert!(xml.contains(old));
        let changed = xml.replacen(old, new, 1);
        members.insert(PART.into(), changed.as_bytes().to_vec());
        let parts: Vec<(&str, Vec<u8>)> = members
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        let book = Workbook::from_bytes(crate::excel_package::package(&parts)).unwrap();
        let pivot = book
            .pivots()
            .unwrap()
            .iter()
            .find(|pivot| pivot.name() == "P6_matrix_one_value")
            .unwrap();
        assert!(!pivot.editable(), "{new}");
        assert!(pivot.spec().is_none(), "{new}");
        let PivotOrigin::Read {
            reason: Some(reason),
            ..
        } = pivot.origin()
        else {
            panic!("expected located imported read-only reason for {new}")
        };
        assert!(
            reason.contains("pivotTable1.xml#pivotFields[0]/items"),
            "{reason}"
        );
        assert!(reason.contains(location), "{reason}");
        let saved = book.into_bytes().unwrap();
        let saved = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(saved))));
        assert_eq!(saved.read_member(PART).unwrap(), changed.as_bytes());
    }
}

mod pivot_imported_captions {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use yggdryl::excel::Workbook;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::zip::ZipArchive;

    fn imported_with_grand_caption_for(part: &str) -> Vec<u8> {
        let source = include_bytes!("fixtures/pivot_ascending_native.xlsx").to_vec();
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(source))));
        let mut members: BTreeMap<String, Vec<u8>> = archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    archive.read_member(entry.name()).unwrap(),
                )
            })
            .collect();
        let xml = String::from_utf8(members.get(part).unwrap().clone()).unwrap();
        assert_eq!(xml.matches("dataCaption=\"Values\"").count(), 1);
        assert_eq!(xml.matches("grandTotalCaption=\"Grand Total\"").count(), 1);
        let xml = xml
            .replacen("dataCaption=\"Values\"", "dataCaption=\"Valeurs\"", 1)
            .replacen(
                "grandTotalCaption=\"Grand Total\"",
                "grandTotalCaption=\"CUSTOM_GRAND\"",
                1,
            );
        members.insert(part.into(), xml.into_bytes());
        let parts: Vec<(&str, Vec<u8>)> = members
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        crate::excel_package::package(&parts)
    }

    fn imported_with_grand_caption() -> Vec<u8> {
        imported_with_grand_caption_for("xl/pivotTables/pivotTable2.xml")
    }

    fn assert_captions(book: &Workbook, name: &str) {
        let sheet = book.sheet("CaseTwoValues").unwrap();
        assert_eq!(
            sheet.cell("C3".parse().unwrap()).unwrap().value().as_str(),
            Some("Valeurs")
        );
        assert_eq!(
            sheet.cell("A9".parse().unwrap()).unwrap().value().as_str(),
            Some("CUSTOM_GRAND")
        );
        // Native multi-value column Grand labels remain per-measure totals.
        assert_eq!(
            sheet.cell("H4".parse().unwrap()).unwrap().value().as_str(),
            Some("Total Sales Sum")
        );
        assert_eq!(
            sheet.cell("I4".parse().unwrap()).unwrap().value().as_str(),
            Some("Total Units Average")
        );
        assert!(
            book.pivots()
                .unwrap()
                .iter()
                .any(|pivot| pivot.name() == name)
        );
    }

    fn assert_xml(bytes: &[u8], name: &str) {
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            bytes.to_vec(),
        ))));
        let xml = String::from_utf8(
            archive
                .read_member("xl/pivotTables/pivotTable2.xml")
                .unwrap(),
        )
        .unwrap();
        assert!(xml.contains("dataCaption=\"Valeurs\""), "{xml}");
        assert!(xml.contains("grandTotalCaption=\"CUSTOM_GRAND\""), "{xml}");
        assert!(xml.contains(&format!("name=\"{name}\"")), "{xml}");
    }

    #[test]
    fn imported_pivot_refresh_uses_one_caption_owner_for_cells_and_xml() {
        let mut book = Workbook::from_bytes(imported_with_grand_caption()).unwrap();
        book.refresh_pivot("CaseTwoValues", "P6_matrix_two_values")
            .unwrap();
        assert_captions(&book, "P6_matrix_two_values");
        let saved = book.into_bytes().unwrap();
        assert_xml(&saved, "P6_matrix_two_values");
        let reopened = Workbook::from_bytes(saved).unwrap();
        assert_captions(&reopened, "P6_matrix_two_values");
    }

    #[test]
    fn imported_pivot_rename_retains_its_authored_captions() {
        let mut book = Workbook::from_bytes(imported_with_grand_caption()).unwrap();
        let spec = book
            .pivots()
            .unwrap()
            .iter()
            .find(|pivot| pivot.name() == "P6_matrix_two_values")
            .unwrap()
            .spec()
            .unwrap()
            .clone();
        let mut spec = spec;
        spec.name = "RenamedTwoValues".into();
        book.update_pivot("CaseTwoValues", "P6_matrix_two_values", spec)
            .unwrap();
        assert_captions(&book, "RenamedTwoValues");
        let saved = book.into_bytes().unwrap();
        assert_xml(&saved, "RenamedTwoValues");
        let reopened = Workbook::from_bytes(saved).unwrap();
        assert_captions(&reopened, "RenamedTwoValues");
    }

    #[test]
    fn imported_single_value_column_grand_uses_explicit_caption() {
        let mut book = Workbook::from_bytes(imported_with_grand_caption_for(
            "xl/pivotTables/pivotTable1.xml",
        ))
        .unwrap();
        book.refresh_pivot("CaseMatrix", "P6_matrix_one_value")
            .unwrap();
        let sheet = book.sheet("CaseMatrix").unwrap();
        assert_eq!(
            sheet.cell("E4".parse().unwrap()).unwrap().value().as_str(),
            Some("CUSTOM_GRAND")
        );
        let saved = book.into_bytes().unwrap();
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            saved.clone(),
        ))));
        let xml = String::from_utf8(
            archive
                .read_member("xl/pivotTables/pivotTable1.xml")
                .unwrap(),
        )
        .unwrap();
        assert!(xml.contains("grandTotalCaption=\"CUSTOM_GRAND\""), "{xml}");
        let reopened = Workbook::from_bytes(saved).unwrap();
        assert_eq!(
            reopened
                .sheet("CaseMatrix")
                .unwrap()
                .cell("E4".parse().unwrap())
                .unwrap()
                .value()
                .as_str(),
            Some("CUSTOM_GRAND")
        );
    }
}

#[test]
fn native_pivot_with_implicit_manual_sort_is_read_only_without_losing_inventory() {
    use yggdryl::excel::Workbook;

    let workbook =
        Workbook::from_bytes(include_bytes!("fixtures/pivot_excel.xlsx").to_vec()).unwrap();
    let pivots = workbook.pivots().unwrap();
    assert_eq!(pivots.len(), 5);
    for pivot in pivots {
        assert!(!pivot.editable(), "{}", pivot.name());
        assert!(pivot.spec().is_none(), "{}", pivot.name());
        assert!(format!("{:?}", pivot.origin()).contains("representable"));
    }
}
