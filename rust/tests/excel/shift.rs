//! `rust/src/excel/shift.rs`: the reference adjuster - every reference a workbook states keeps naming the cells it named when rows or columns open or close, a block moves or a sheet is renamed or removed; ranges grow, shrink or go `#REF!`, and what no edit here can carry through is refused by name before anything changes.

use yggdryl::Error;
use yggdryl::excel::{Cell, CellRange, CellRef, DateSystem, Formula, Paste, Sheet, Workbook};

use crate::excel_package::{
    NS, book, costs_cut_part, cut_table_part, member, sheet, sheets_book, stationary_cut_table,
    table_formula_part, table_member_map, with_cut_tables,
};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

/// A workbook of the sheets `Data` and `Other`, each cell of `formulas` on
/// `Data` holding its formula.
fn with_formulas(formulas: &[(&str, &str)]) -> Workbook {
    let mut workbook = Workbook::new();
    let data = workbook.add_sheet("Data").unwrap();
    for (reference, text) in formulas {
        let host = at(reference);
        data.insert_cell(
            Cell::from_scalar(host, 0.0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file(text, host)),
        )
        .unwrap();
    }
    workbook.add_sheet("Other").unwrap();
    workbook
}

/// The file spelling of the formula each cell of `Data` holds, by cell.
fn spelled(workbook: &Workbook) -> Vec<(String, String)> {
    workbook
        .sheet("Data")
        .unwrap()
        .cells()
        .filter_map(|cell| {
            Some((
                cell.reference().to_string(),
                cell.formula()?.at(cell.reference()).to_string(),
            ))
        })
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(at, text)| ((*at).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn an_insertion_moves_what_lies_past_it_and_grows_what_it_falls_inside() {
    let mut workbook = with_formulas(&[
        ("A1", "B1+B5+$B$5+B$9"),
        ("A2", "SUM(B1:B4)+SUM(B3:B9)+SUM(B5:B9)"),
        ("A3", "SUM(3:4)+SUM(B:C)+Other!B5+Data!B5"),
        ("A9", "B1+B9+A1048576"),
    ]);
    // Two rows open before row 3.
    workbook.insert_rows("Data", 2, 2).unwrap();
    assert_eq!(
        spelled(&workbook),
        pairs(&[
            ("A1", "B1+B7+$B$7+B$11"),
            ("A2", "SUM(B1:B6)+SUM(B5:B11)+SUM(B7:B11)"),
            // The host moved: its relative references follow it, every
            // cell they named still named.
            ("A5", "SUM(5:6)+SUM(B:C)+Other!B5+Data!B7"),
            ("A11", "B1+B11+#REF!"),
        ])
    );
    // A formula on another sheet naming this one follows too.
    let mut workbook = with_formulas(&[]);
    let other = workbook.sheet_mut("Other").unwrap();
    other
        .insert_cell(
            Cell::from_scalar(at("C3"), 0.0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("Data!B2:C9*'Data'!$A3+C3", at("C3"))),
        )
        .unwrap();
    workbook.insert_columns("Data", 2, 1).unwrap();
    let cell = workbook
        .sheet("Other")
        .unwrap()
        .cell(at("C3"))
        .unwrap()
        .clone();
    assert_eq!(
        cell.formula().unwrap().at(at("C3")).to_string(),
        "Data!B2:D9*'Data'!$A3+C3"
    );
}

#[test]
fn a_removal_shrinks_what_it_cuts_and_makes_what_it_takes_whole_ref() {
    let mut workbook = with_formulas(&[
        ("A1", "B2+B3+B5+SUM(B2:B6)+SUM(B3:B4)+SUM(B1:B3)"),
        ("A8", "SUM(3:4)+SUM(2:5)+Other!B3"),
        ("E1", "SUM(A:A)+SUM(C:D)+SUM(B:E)"),
    ]);
    // Rows 3 and 4 go.
    workbook.remove_rows("Data", 2..4).unwrap();
    assert_eq!(
        spelled(&workbook),
        pairs(&[
            ("A1", "B2+#REF!+B3+SUM(B2:B4)+SUM(#REF!)+SUM(B1:B2)"),
            ("E1", "SUM(A:A)+SUM(C:D)+SUM(B:E)"),
            ("A6", "SUM(#REF!)+SUM(2:3)+Other!B3"),
        ])
    );
    // Columns C and D go.
    workbook.remove_columns("Data", 2..4).unwrap();
    assert_eq!(
        spelled(&workbook)[1..],
        pairs(&[
            ("C1", "SUM(A:A)+SUM(#REF!)+SUM(B:C)"),
            ("A6", "SUM(#REF!)+SUM(2:3)+Other!B3"),
        ])
    );
}

#[test]
fn a_three_dimensional_span_and_a_reference_into_another_workbook_name_no_one_sheet() {
    let mut workbook = with_formulas(&[("A5", "SUM(Data:Other!B5)+[1]Data!B5+B5")]);
    workbook.insert_rows("Data", 0, 1).unwrap();
    // The span and the external reference keep naming the cells they
    // named from the host's new place.
    assert_eq!(
        spelled(&workbook),
        pairs(&[("A6", "SUM(Data:Other!B5)+[1]Data!B5+B6")])
    );
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::excel_formula::shares_shape;

    use super::{at, with_formulas};

    #[test]
    fn affine_carried_formulas_reproduce_every_native_scalar_reference_class() {
        use yggdryl::excel::{CellRange, CellRef, Formula};
        use yggdryl::internals::excel_shift::carried_formula_regions;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/carried_formulas_excel.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let id = case["id"].as_str().unwrap();
            let destination = case["destination"].as_str().unwrap();
            let source = ("Data", case["block"].as_str().unwrap().parse().unwrap());
            let target = (
                destination,
                case["target"].as_str().unwrap().parse().unwrap(),
            );
            for output_sheet in ["Data", "Other"] {
                let mut produced = Vec::new();
                for input_sheet in ["Data", "Other"] {
                    let xml = format!(
                        "<worksheet xmlns=\"{}\">{}</worksheet>",
                        super::NS,
                        case["before"][input_sheet].as_str().unwrap()
                    );
                    for (ranges, texts) in super::partial_formula_rules(&xml) {
                        let at = CellRef::new(
                            ranges.iter().map(|area| area.start().row()).min().unwrap(),
                            ranges
                                .iter()
                                .map(|area| area.start().column())
                                .min()
                                .unwrap(),
                        );
                        let formulas: Vec<_> = texts
                            .iter()
                            .map(|text| Formula::from_file(text, at))
                            .collect();
                        let owner = if input_sheet == output_sheet {
                            (input_sheet, None)
                        } else if input_sheet == "Data" && output_sheet == destination {
                            (input_sheet, Some(destination))
                        } else {
                            continue;
                        };
                        let regions = carried_formula_regions(
                            id.starts_with("cf-"),
                            source,
                            target,
                            owner,
                            &ranges,
                            &formulas,
                        )
                        .unwrap();
                        produced.extend(regions.into_iter().map(|(ranges, formulas)| {
                            let host = CellRef::new(
                                ranges.iter().map(|area| area.start().row()).min().unwrap(),
                                ranges
                                    .iter()
                                    .map(|area| area.start().column())
                                    .min()
                                    .unwrap(),
                            );
                            (
                                ranges,
                                formulas
                                    .iter()
                                    .map(|formula| formula.at(host).to_string())
                                    .collect(),
                            )
                        }));
                    }
                }
                let expected = case["after"][output_sheet]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|rule| {
                        (
                            rule["sqref"]
                                .as_str()
                                .unwrap()
                                .split_whitespace()
                                .map(|area| area.parse::<CellRange>().unwrap())
                                .collect(),
                            rule["formulas"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|value| value.as_str().unwrap().to_owned())
                                .collect(),
                        )
                    })
                    .collect();
                assert_eq!(
                    super::partial_formula_meaning(produced, output_sheet),
                    super::partial_formula_meaning(expected, output_sheet),
                    "{id} {output_sheet}"
                );
            }
        }
    }

    #[test]
    fn affine_carried_formulas_bound_generated_fragments_without_bounding_input_areas() {
        use yggdryl::excel::{CellRange, CellRef, Formula};
        use yggdryl::internals::excel_shift::carried_formula_regions;
        // Forty independent row boundaries cross forty column boundaries.
        // This small formula would otherwise create thousands of host classes.
        let mut terms: Vec<String> = (1..=40).map(|row| format!("$A{row}")).collect();
        terms.extend((0..40).map(|column| format!("{}$100", CellRef::column_name(column))));
        let formula = Formula::from_file(&format!("SUM({})", terms.join(",")), at("A1"));
        let ranges: Vec<CellRange> = vec!["A1:CV1000".parse().unwrap()];
        let error = carried_formula_regions(
            false,
            ("Data", "A100:AN139".parse().unwrap()),
            ("Other", at("Z2000")),
            ("Data", None),
            &ranges,
            &[formula],
        )
        .unwrap_err();
        assert!(
            matches!(error, yggdryl::Error::Unsupported { operation, filesystem }
            if operation == "producing more than 1024 carried-formula host fragments" && filesystem == "worksheet#rule")
        );
        let areas: Vec<_> = (0..1_100)
            .map(|row| CellRange::new(CellRef::new(row, 10), CellRef::new(row, 10)))
            .collect();
        let original = Formula::from_file("1", at("K1"));
        let held = carried_formula_regions(
            false,
            ("Data", "A1".parse().unwrap()),
            ("Other", at("A1")),
            ("Data", None),
            &areas,
            &[original],
        )
        .unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(
            held[0].0, areas,
            "unchanged input areas do not consume a generated-fragment allowance"
        );
    }

    #[test]
    fn carried_rectangle_partition_conserves_geometry_without_cell_expansion() {
        use yggdryl::excel::{CellRange, CellRef};
        use yggdryl::internals::excel_shift::range_partition;
        let overlaps = |a: CellRange, b: CellRange| {
            a.start().row() <= b.end().row()
                && b.start().row() <= a.end().row()
                && a.start().column() <= b.end().column()
                && b.start().column() <= a.end().column()
        };
        let area = |range: CellRange| u64::from(range.row_size()) * u64::from(range.column_size());
        let check = |range: CellRange, cut: CellRange| {
            let (selected, remainder) = range_partition(range, cut);
            let mut covered = 0;
            if let Some(selected) = selected {
                assert!(range.contains(selected.start()) && range.contains(selected.end()));
                assert!(cut.contains(selected.start()) && cut.contains(selected.end()));
                covered += area(selected);
            }
            for (index, piece) in remainder.iter().enumerate() {
                let Some(piece) = *piece else {
                    continue;
                };
                assert!(range.contains(piece.start()) && range.contains(piece.end()));
                assert!(!overlaps(piece, cut));
                for other in remainder[..index].iter().flatten() {
                    assert!(!overlaps(piece, *other));
                }
                covered += area(piece);
            }
            assert_eq!(covered, area(range), "{range} minus {cut}");
        };
        let mut rectangles = Vec::new();
        for first_row in 0..4 {
            for last_row in first_row..4 {
                for first_column in 0..4 {
                    for last_column in first_column..4 {
                        rectangles.push(CellRange::new(
                            CellRef::new(first_row, first_column),
                            CellRef::new(last_row, last_column),
                        ));
                    }
                }
            }
        }
        for range in &rectangles {
            for cut in &rectangles {
                check(*range, *cut);
            }
        }
        // Full-grid cardinality exceeds u32; the algorithm still emits at
        // most four rectangles and never visits any of their cells.
        let all = CellRange::all();
        for cut in [
            "A1",
            "XFD1",
            "A1048576",
            "XFD1048576",
            "A1:XFD1048576",
            "B2:XFC1048575",
            "A2:XFD1048575",
            "B1:XFC1048576",
        ] {
            check(all, cut.parse().unwrap());
            check(cut.parse().unwrap(), all);
        }
    }

    #[test]
    fn carried_rectangle_partition_large_hole_has_four_exact_strips() {
        use yggdryl::excel::CellRange;
        use yggdryl::internals::excel_shift::range_partition;
        let hole = "B2:XFC1048575".parse().unwrap();
        let (selected, remainder) = range_partition(CellRange::all(), hole);
        assert_eq!(selected, Some(hole));
        assert_eq!(
            remainder,
            [
                Some("A1:XFD1".parse().unwrap()),
                Some("A1048576:XFD1048576".parse().unwrap()),
                Some("A2:A1048575".parse().unwrap()),
                Some("XFD2:XFD1048575".parse().unwrap()),
            ]
        );
    }

    #[test]
    fn a_shape_shared_by_many_cells_is_rewritten_once_per_host_class() {
        // Forty cells holding one shape, as a filled column read from a part
        // does.
        let mut workbook = with_formulas(&[]);
        let shape = yggdryl::excel::Formula::from_file("A1*B1+$D$1", at("C1"));
        let sheet = workbook.sheet_mut("Data").unwrap();
        for row in 0..40 {
            let host = yggdryl::excel::CellRef::new(row, 2);
            sheet
                .insert_cell(
                    yggdryl::excel::Cell::from_scalar(
                        host,
                        0.0.into(),
                        yggdryl::excel::DateSystem::Year1900,
                    )
                    .unwrap()
                    .with_formula(shape.clone()),
                )
                .unwrap();
        }
        let shared = |workbook: &yggdryl::excel::Workbook, first: &str, second: &str| {
            let sheet = workbook.sheet("Data").unwrap();
            shares_shape(
                sheet.cell(at(first)).unwrap().formula().unwrap(),
                sheet.cell(at(second)).unwrap().formula().unwrap(),
            )
        };
        assert!(shared(&workbook, "C2", "C3"));
        workbook.insert_rows("Data", 10, 5).unwrap();
        // Above the rows, below them: the hosts the insertion treats alike
        // share the one rewritten shape.
        assert!(shared(&workbook, "C17", "C30"));
        assert!(shared(&workbook, "C2", "C3"));
    }
}

#[test]
fn an_element_no_edit_carries_through_refuses_every_structural_edit_by_name() {
    for (element, expected) in [
        (
            "<oleObjects><oleObject progId=\"Package\" shapeId=\"1025\" r:id=\"rId9\"/></oleObjects>",
            "filesystem \"xl/worksheets/sheet1.xml\" does not support inserting rows through a sheet carrying oleObjects",
        ),
        (
            "<scenarios><scenario name=\"s\" count=\"1\"><inputCells r=\"A1\" val=\"1\"/></scenario></scenarios>",
            "filesystem \"xl/worksheets/sheet1.xml\" does not support inserting rows through a sheet carrying scenarios",
        ),
        (
            "<extLst><ext uri=\"{00000000-0000-0000-0000-000000000000}\"/></extLst>",
            "filesystem \"xl/worksheets/sheet1.xml\" does not support inserting rows through a sheet carrying an extension this crate does not model",
        ),
        (
            "<mystery/>",
            "filesystem \"xl/worksheets/sheet1.xml#mystery\" does not support inserting rows through a sheet carrying an element this crate does not model",
        ),
    ] {
        let mut workbook = book(
            &sheet("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", element),
            &[],
            &[],
        );
        let before = workbook.sheet("Sheet1").unwrap().clone();
        let error = workbook.insert_rows("Sheet1", 0, 1).unwrap_err();
        assert!(matches!(error, Error::Unsupported { .. }), "{error}");
        assert_eq!(error.to_string(), expected);
        assert_eq!(workbook.sheet("Sheet1").unwrap(), &before);
        // Other edits pass: nothing names cells the element holds.
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .set_cell(at("B1"), 2.0)
            .unwrap();
    }
    let mut workbook = book(&sheet("", "<controls/>"), &[], &[]);
    assert_eq!(
        workbook
            .remove_columns("Sheet1", 0..1)
            .unwrap_err()
            .to_string(),
        "filesystem \"xl/worksheets/sheet1.xml\" does not support removing columns through a sheet carrying controls"
    );
}

#[test]
fn an_edit_splitting_an_array_formula_or_a_data_table_is_refused() {
    let rows = "<row r=\"2\"><c r=\"B2\"><f t=\"array\" ref=\"B2:B4\">A2:A4*2</f><v>2</v></c><c r=\"D2\"><f t=\"dataTable\" ref=\"D2:E3\" dt2D=\"1\" r1=\"A1\" r2=\"A2\"/><v>1</v></c></row>";
    let mut workbook = book(&sheet(rows, ""), &[], &[]);
    for (outcome, expected) in [
        (
            workbook.insert_rows("Sheet1", 2, 1),
            "invalid record value at Sheet1!B2:B4: expected rows leaving the array formula or data table anchored at B2 whole, got some of it",
        ),
        (
            workbook.remove_rows("Sheet1", 3..5),
            "invalid record value at Sheet1!B2:B4: expected rows leaving the array formula or data table anchored at B2 whole, got some of it",
        ),
        (
            workbook.insert_columns("Sheet1", 4, 1),
            "invalid record value at Sheet1!D2:E3: expected columns leaving the array formula or data table anchored at D2 whole, got some of it",
        ),
    ] {
        assert_eq!(outcome.unwrap_err().to_string(), expected);
    }
    // At the array's first row it moves whole, its range with it; a row
    // taking all of it takes it.
    workbook.insert_rows("Sheet1", 1, 1).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<f t=\"array\" ref=\"B3:B5\">A3:A5*2</f>"),
        "{part}"
    );
    assert!(part.contains("ref=\"D3:E4\""), "{part}");
    assert!(part.contains("r1=\"A1\" r2=\"A3\""), "{part}");
}

/// The table on Data refers both to its own sheet and to Other; its XML
/// formulas, array attributes and structured-reference spelling are carried.
fn with_table_formulas(calculated: &str, totals: &str) -> Workbook {
    sheets_book(
        &[
            (
                "Data",
                sheet(
                    "<row r=\"3\"><c r=\"B3\"><v>3</v></c></row><row r=\"4\"><c r=\"B4\"><v>4</v></c></row><row r=\"5\"><c r=\"B5\"><v>5</v></c></row>",
                    "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                ),
                &[("rIdTable", "table", "../tables/table1.xml")],
            ),
            (
                "Other",
                sheet(
                    "<row r=\"3\"><c r=\"B3\"><v>30</v></c></row><row r=\"4\"><c r=\"B4\"><v>40</v></c></row><row r=\"5\"><c r=\"B5\"><v>50</v></c></row>",
                    "",
                ),
                &[],
            ),
        ],
        &[],
        &[(
            "xl/tables/table1.xml",
            table_formula_part("E2:F6", "E2:F5", calculated, totals),
        )],
    )
}

#[test]
fn table_formulas_refuse_malformed_row_counts_atomically() {
    for attribute in ["headerRowCount", "totalsRowCount"] {
        for value in ["bad", "4294967296", "-1"] {
            for local in [false, true] {
                let table = table_formula_part("E2:F6", "E2:F5", "Other!B3+B3", "SUM(Other!B3:B5)");
                let attribute_text = format!("{attribute}=\"{value}\"");
                let table = if attribute == "headerRowCount" {
                    table.replace(
                        "totalsRowCount=\"1\"",
                        &format!("{attribute_text} totalsRowCount=\"1\""),
                    )
                } else {
                    table.replace("totalsRowCount=\"1\"", &attribute_text)
                };
                let mut workbook = sheets_book(
                    &[
                        (
                            "Data",
                            sheet(
                                "<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>",
                                "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                            ),
                            &[("rIdTable", "table", "../tables/table1.xml")],
                        ),
                        ("Other", sheet("", ""), &[]),
                    ],
                    &[],
                    &[("xl/tables/table1.xml", table)],
                );
                let before = table_member_map(&workbook);
                let result = if local {
                    workbook.insert_rows("Data", 0, 1)
                } else {
                    workbook.rename_sheet("Other", "Renamed")
                };
                match result.unwrap_err() {
                    Error::InvalidRecord { path, reason } => {
                        assert_eq!(path.as_str(), format!("xl/tables/table1.xml#{attribute}"));
                        assert_eq!(
                            reason.as_str(),
                            format!("expected an unsigned 32-bit row count, got {value:?}")
                        );
                    }
                    error => panic!("expected a located invalid row count, got {error}"),
                }
                assert_eq!(workbook.sheet_names(), ["Data", "Other"]);
                assert!(!workbook.is_dirty());
                assert_eq!(
                    table_member_map(&workbook),
                    before,
                    "{attribute}={value}, local={local}"
                );
            }
        }
    }
}

#[test]
fn table_formulas_refuse_clipping_blank_tables_at_grid_edges_atomically() {
    for (range, rows, limit, axis) in [
        ("XFC2:XFD6", false, 16384, "columns"),
        ("B1048572:C1048576", true, 1048576, "rows"),
    ] {
        // No stored cell or formula reaches the edge: the table's complete
        // extent is the fact that must prevent silent truncation.
        let table = format!(
            "<table xmlns=\"{NS}\" id=\"1\" name=\"Edge\" displayName=\"Edge\" ref=\"{range}\"><autoFilter ref=\"{range}\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"A\"/><tableColumn id=\"2\" name=\"B\"/></tableColumns></table>"
        );
        let mut workbook = sheets_book(
            &[(
                "Data",
                sheet(
                    "<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>",
                    "<tableParts count=\"1\"><tablePart r:id=\"rIdTable\"/></tableParts>",
                ),
                &[("rIdTable", "table", "../tables/table1.xml")],
            )],
            &[],
            &[("xl/tables/table1.xml", table)],
        );
        let before = table_member_map(&workbook);
        let result = if rows {
            workbook.insert_rows("Data", 0, 1)
        } else {
            workbook.insert_columns("Data", 0, 1)
        };
        match result.unwrap_err() {
            Error::InvalidRecord { path, reason } => {
                assert_eq!(
                    path.as_str(),
                    format!(
                        "Data!{}",
                        range.parse::<yggdryl::excel::CellRange>().unwrap()
                    )
                );
                assert_eq!(
                    reason.as_str(),
                    format!(
                        "expected the table Edge to stay within {limit} {axis}, got its last index moving by 1"
                    )
                );
            }
            error => panic!("expected a located table extent refusal, got {error}"),
        }
        assert!(!workbook.is_dirty());
        assert_eq!(table_member_map(&workbook), before, "{range}");
    }
}

#[test]
fn table_formulas_follow_local_rows_and_columns() {
    let calculated = "B3+$B$4+SUM(B3:B5)+Costs[[#This Row],[Cost]]";
    let totals = "SUM(B3:B5)+$B$4+Costs[Cost]";
    let mut workbook = with_table_formulas(calculated, totals);
    workbook.insert_rows("Data", 3, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F7",
            "E2:F6",
            "B3+$B$5+SUM(B3:B6)+Costs[[#This Row],[Cost]]",
            "SUM(B3:B6)+$B$5+Costs[Cost]",
        )
    );
    workbook.insert_columns("Data", 0, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "F2:G7",
            "F2:G6",
            "C3+$C$5+SUM(C3:C6)+Costs[[#This Row],[Cost]]",
            "SUM(C3:C6)+$C$5+Costs[Cost]",
        )
    );
    workbook.remove_rows("Data", 3..4).unwrap();
    workbook.remove_columns("Data", 0..1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part("E2:F6", "E2:F5", calculated, totals)
    );
}

#[test]
fn table_formulas_follow_other_sheet_rows_and_columns() {
    let mut workbook = with_table_formulas(
        "Other!B3+$B$4+Costs[[#This Row],[Cost]]",
        "SUM(Other!B3:B5)+B3+Costs[Cost]",
    );
    workbook.insert_rows("Other", 2, 1).unwrap();
    workbook.insert_columns("Other", 1, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "Other!C4+$B$4+Costs[[#This Row],[Cost]]",
            "SUM(Other!C4:C6)+B3+Costs[Cost]",
        )
    );
    workbook.remove_rows("Other", 4..5).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "Other!C4+$B$4+Costs[[#This Row],[Cost]]",
            "SUM(Other!C4:C5)+B3+Costs[Cost]",
        )
    );
}

#[test]
fn table_formulas_follow_sheet_rename_and_removal() {
    let mut workbook = with_table_formulas(
        "Other!B3+B3+Costs[[#This Row],[Cost]]",
        "SUM(Other!B3:B5)+Costs[Cost]",
    );
    workbook.rename_sheet("Other", "Other Data").unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "'Other Data'!B3+B3+Costs[[#This Row],[Cost]]",
            "SUM('Other Data'!B3:B5)+Costs[Cost]",
        )
    );
    workbook.remove_sheet("Other Data").unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "#REF!B3+B3+Costs[[#This Row],[Cost]]",
            "SUM(#REF!B3:B5)+Costs[Cost]",
        )
    );
}

#[test]
fn table_formulas_follow_cut_references_from_either_sheet() {
    let mut workbook = with_table_formulas(
        "Other!B3+B3+Costs[[#This Row],[Cost]]",
        "SUM(Other!B3:B5)+SUM(B3:B5)+Costs[Cost]",
    );
    workbook
        .paste(
            ("Other", "B3:B5".parse().unwrap()),
            ("Other", at("D8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "Other!D8+B3+Costs[[#This Row],[Cost]]",
            "SUM(Other!D8:D10)+SUM(B3:B5)+Costs[Cost]",
        )
    );
    workbook
        .paste(
            ("Data", "B3:B5".parse().unwrap()),
            ("Other", at("F8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "E2:F6",
            "E2:F5",
            "Other!D8+Other!F8+Costs[[#This Row],[Cost]]",
            "SUM(Other!D8:D10)+SUM(Other!F8:F10)+Costs[Cost]",
        )
    );
}

#[test]
fn table_formulas_keep_their_owner_after_its_rename() {
    let mut workbook = with_table_formulas(
        "B3+Other!B3+Costs[[#This Row],[Cost]]",
        "SUM(B3:B5)+Costs[Cost]",
    );
    workbook.rename_sheet("Data", "Renamed").unwrap();
    workbook.insert_columns("Renamed", 0, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "F2:G6",
            "F2:G5",
            "C3+Other!B3+Costs[[#This Row],[Cost]]",
            "SUM(C3:C5)+Costs[Cost]",
        ),
        "the owner column insertion shifts both formula templates"
    );
    workbook.insert_rows("Other", 2, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part(
            "F2:G6",
            "F2:G5",
            "C3+Other!B4+Costs[[#This Row],[Cost]]",
            "SUM(C3:C5)+Costs[Cost]",
        )
    );
}

#[test]
fn table_formulas_in_an_empty_table_keep_following_external_sheets() {
    let original = table_formula_part(
        "E2:F2",
        "E2:F2",
        "Other!B3+Costs[[#This Row],[Cost]]",
        "SUM(Other!B3:B5)+Costs[Cost]",
    )
    .replace(
        "totalsRowCount=\"1\"",
        "totalsRowCount=\"0\" totalsRowShown=\"0\"",
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
            ("Other", sheet("", ""), &[]),
        ],
        &[],
        &[("xl/tables/table1.xml", original.clone())],
    );
    // Formula templates survive when there is no data row and the totals
    // row is hidden; renaming their external sheet still has one reading.
    workbook.rename_sheet("Other", "Renamed").unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        original.replace("Other!", "Renamed!")
    );
    workbook.insert_rows("Renamed", 2, 1).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        original
            .replace("Other!B3:B5", "Renamed!B4:B6")
            .replace("Other!B3", "Renamed!B4")
    );
}

#[test]
fn table_cut_empty_templates_keep_their_original_reference_hosts() {
    let calculated = "E3+$A$1+Other!B3+Costs[[#This Row],[Cost]]";
    let totals = "SUM(E3:E5)+Costs[Cost]+$A$1";
    let original = table_formula_part("E2:F2", "E2:F2", calculated, totals).replace(
        "totalsRowCount=\"1\"",
        "totalsRowCount=\"0\" totalsRowShown=\"0\"",
    );
    for destination in ["Data", "Other"] {
        let mut workbook = with_cut_tables(&[("Data", original.clone())]);
        workbook
            .paste(
                ("Data", "E2:F2".parse().unwrap()),
                (destination, at("J8")),
                Paste::All,
                true,
            )
            .unwrap();
        // Neither dormant host F3 nor its data references belongs to this
        // header-only cut. They still name Data's original cells, even when
        // the template is now stored at K9 on another sheet.
        let expected = if destination == "Data" {
            table_formula_part("J8:K8", "J8:K8", calculated, totals)
        } else {
            table_formula_part(
                "J8:K8",
                "J8:K8",
                "Data!E3+Data!$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(Data!E3:E5)+Costs[Cost]+Data!$A$1",
            )
        }
        .replace(
            "totalsRowCount=\"1\"",
            "totalsRowCount=\"0\" totalsRowShown=\"0\"",
        );
        assert_eq!(member(&workbook, "xl/tables/table1.xml"), expected);
        let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
        assert_eq!(member(&reopened, "xl/tables/table1.xml"), expected);
        let data = workbook.sheet("Data").unwrap();
        assert_eq!(data.scalar(at("E3")), 3.0.into());
        assert_eq!(data.scalar(at("A1")), 9.0.into());
    }
}

#[test]
fn table_cut_empty_templates_refuse_hosts_outside_the_grid_atomically() {
    for destination in ["Data", "Other"] {
        for formula in ["calculatedColumnFormula", "totalsRowFormula"] {
            // Test each dormant template independently: a valid table range
            // at the final row must not conceal its off-grid formula host.
            let mut table = table_formula_part("E2:F2", "E2:F2", "E3", "SUM(E3:E5)").replace(
                "totalsRowCount=\"1\"",
                "totalsRowCount=\"0\" totalsRowShown=\"0\"",
            );
            let absent = if formula == "calculatedColumnFormula" {
                "<totalsRowFormula array=\"0\">SUM(E3:E5)</totalsRowFormula>"
            } else {
                "<calculatedColumnFormula array=\"1\">E3</calculatedColumnFormula>"
            };
            assert!(table.contains(absent));
            table = table.replace(absent, "");
            let mut workbook = with_cut_tables(&[("Data", table)]);
            let before = table_member_map(&workbook);
            let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
            let error = workbook
                .paste(
                    ("Data", "E2:F2".parse().unwrap()),
                    (destination, at("J1048576")),
                    Paste::All,
                    true,
                )
                .unwrap_err();
            match error {
                Error::InvalidRecord { path, reason } => {
                    assert_eq!(path, "xl/tables/table1.xml");
                    assert_eq!(
                        reason,
                        "expected a formula host inside the worksheet grid for table Costs, got K1048577"
                    );
                }
                other => panic!("expected a located off-grid template refusal, got {other}"),
            }
            assert_eq!(
                table_member_map(&workbook),
                before,
                "{destination}, {formula}"
            );
            assert_eq!(
                ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
                revisions
            );
            assert!(!workbook.is_dirty());
        }
    }
}

#[test]
fn table_formulas_undo_and_redo_across_saved_packages() {
    use yggdryl::excel::Edit;

    let calculated = "Other!B3+B3+Costs[[#This Row],[Cost]]";
    let totals = "SUM(Other!B3:B5)+Costs[Cost]";
    let mut workbook = with_table_formulas(calculated, totals);
    let applied = workbook
        .apply(Edit::InsertRows {
            sheet: "Other".into(),
            at: 2,
            count: 1,
        })
        .unwrap();
    let expected = table_formula_part(
        "E2:F6",
        "E2:F5",
        "Other!B4+B3+Costs[[#This Row],[Cost]]",
        "SUM(Other!B4:B6)+Costs[Cost]",
    );
    assert_eq!(member(&workbook, "xl/tables/table1.xml"), expected);
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        table_formula_part("E2:F6", "E2:F5", calculated, totals)
    );
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    workbook.apply(undone.inverse.unwrap()).unwrap();
    assert_eq!(member(&workbook, "xl/tables/table1.xml"), expected);
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(member(&reopened, "xl/tables/table1.xml"), expected);
}

/// The same full-package inverse must survive each intervening save/rebase.
fn assert_table_cut_saves_and_restores(
    workbook: &mut Workbook,
    source: &str,
    target: &str,
    expected: &[String],
) {
    use yggdryl::excel::Edit;

    let before = table_member_map(workbook);
    let relations = member(workbook, "xl/worksheets/_rels/sheet1.xml.rels");
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), source.parse().unwrap()),
            to: ("Data".into(), at(target)),
            what: Paste::All,
            cut: true,
        })
        .unwrap();
    for (index, xml) in expected.iter().enumerate() {
        assert_eq!(
            member(workbook, &format!("xl/tables/table{}.xml", index + 1)),
            *xml
        );
    }
    assert_eq!(
        member(workbook, "xl/worksheets/_rels/sheet1.xml.rels"),
        relations
    );
    let moved = table_member_map(workbook);
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    let mut undo = applied.inverse.unwrap();
    for _ in 0..2 {
        let undone = workbook.apply(undo).unwrap();
        assert_eq!(table_member_map(workbook), before);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let redone = workbook.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(workbook), moved);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        undo = redone.inverse.unwrap();
    }
}

#[test]
fn table_cut_moves_a_whole_table_without_overlap() {
    let mut workbook = with_cut_tables(&[("Data", costs_cut_part())]);
    let expected = cut_table_part(
        "Costs",
        11,
        ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
        [
            "J9+$J$9+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
            "SUM(J9:J11)+SUM($J$9:$J$11)+Costs[Cost]+$A$1",
        ],
    );
    assert_table_cut_saves_and_restores(&mut workbook, "E2:F6", "J8", &[expected]);
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("J9")), 3.0.into());
    assert!(data.cell(at("E3")).is_none());
    assert_eq!(data.scalar(at("G4")), 44.0.into());
    assert_eq!(
        data.cell(at("K9"))
            .unwrap()
            .formula()
            .unwrap()
            .at(at("K9"))
            .to_string(),
        "J9+$A$1"
    );
}

#[test]
fn table_cut_moves_a_whole_table_over_its_previous_extent_in_both_directions() {
    for (target, ranges, formulas) in [
        (
            "F3",
            ["F3:G7", "F3:G6", "F4:G6", "G4:G6"],
            [
                "F4+$F$4+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(F4:F6)+SUM($F$4:$F$6)+Costs[Cost]+$A$1",
            ],
        ),
        (
            "D1",
            ["D1:E5", "D1:E4", "D2:E4", "E2:E4"],
            [
                "D2+$D$2+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(D2:D4)+SUM($D$2:$D$4)+Costs[Cost]+$A$1",
            ],
        ),
    ] {
        let mut workbook = with_cut_tables(&[("Data", costs_cut_part())]);
        let expected = cut_table_part("Costs", 11, ranges, formulas);
        assert_table_cut_saves_and_restores(&mut workbook, "E2:F6", target, &[expected]);
    }
}

fn assert_table_cut_collision(
    mut workbook: Workbook,
    source: &str,
    target: (&str, &str),
    moving: &str,
    moved_range: &str,
    stationary_range: &str,
) {
    let before = table_member_map(&workbook);
    let dirty = workbook.is_dirty();
    let names: Vec<_> = workbook
        .sheet_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let revisions: Vec<_> = names
        .iter()
        .map(|name| workbook.sheet(name).unwrap().revision())
        .collect();
    let error = workbook
        .paste(
            (&names[0], source.parse().unwrap()),
            (target.0, at(target.1)),
            Paste::All,
            true,
        )
        .unwrap_err();
    match error {
        Error::InvalidRecord { path, reason } => {
            assert_eq!(path.as_str(), format!("{}!{moved_range}", target.0));
            assert_eq!(
                reason.as_str(),
                format!(
                    "expected the moved table {moving} to avoid other tables, got Taken at {stationary_range}"
                )
            );
        }
        error => panic!("expected a located table collision refusal, got {error}"),
    }
    assert_eq!(workbook.is_dirty(), dirty);
    assert_eq!(
        names
            .iter()
            .map(|name| workbook.sheet(name).unwrap().revision())
            .collect::<Vec<_>>(),
        revisions
    );
    assert_eq!(workbook.sheet_names(), names);
    assert_eq!(table_member_map(&workbook), before);
}

#[test]
fn table_cut_refuses_partial_destination_table_overlap_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "K10:L14")),
    ]);
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Data", "J8"),
        "Costs",
        "J8:K12",
        "K10:L14",
    );
}

#[test]
fn table_cut_refuses_an_enclosing_destination_table_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "J7:K13")),
    ]);
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Data", "J8"),
        "Costs",
        "J8:K12",
        "J7:K13",
    );
}

#[test]
fn table_cut_refuses_an_enclosed_destination_table_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "J9:K11")),
    ]);
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Data", "J8"),
        "Costs",
        "J8:K12",
        "J9:K11",
    );
}

#[test]
fn table_cut_refuses_an_equal_destination_table_extent_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "J8:K12")),
    ]);
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Data", "J8"),
        "Costs",
        "J8:K12",
        "J8:K12",
    );
}

#[test]
fn table_cut_refuses_cross_sheet_destination_table_overlap_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Other", stationary_cut_table("Taken", 12, "K10:L14")),
    ]);
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Other", "J8"),
        "Costs",
        "J8:K12",
        "K10:L14",
    );
}

fn tax_cut_part() -> String {
    cut_table_part(
        "Tax",
        29,
        ["H3:I7", "H3:I6", "H4:I6", "I4:I6"],
        [
            "H4+$H$4+$A$1+Tax[[#This Row],[Cost]]",
            "SUM(H4:H6)+Tax[Cost]",
        ],
    )
}

#[test]
fn table_cut_moves_two_tables_preserving_their_offsets() {
    let mut workbook = with_cut_tables(&[("Data", costs_cut_part()), ("Data", tax_cut_part())]);
    let expected = [
        cut_table_part(
            "Costs",
            11,
            ["N11:O15", "N11:O14", "N12:O14", "O12:O14"],
            [
                "N12+$N$12+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(N12:N14)+SUM($N$12:$N$14)+Costs[Cost]+$A$1",
            ],
        ),
        cut_table_part(
            "Tax",
            29,
            ["Q12:R16", "Q12:R15", "Q13:R15", "R13:R15"],
            [
                "Q13+$Q$13+$A$1+Tax[[#This Row],[Cost]]",
                "SUM(Q13:Q15)+Tax[Cost]",
            ],
        ),
    ];
    assert_table_cut_saves_and_restores(&mut workbook, "D1:J8", "M10", &expected);
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("N12")), 3.0.into());
    assert_eq!(data.scalar(at("Q13")), 8.0.into());
    assert_eq!(data.scalar(at("P13")), 44.0.into());
    assert_eq!(data.scalar(at("A1")), 9.0.into());
}

#[test]
fn table_cut_refuses_a_later_table_collision_atomically() {
    let workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", tax_cut_part()),
        ("Data", stationary_cut_table("Taken", 31, "R14:S18")),
    ]);
    assert_table_cut_collision(
        workbook,
        "D1:J8",
        ("Data", "M10"),
        "Tax",
        "Q12:R16",
        "R14:S18",
    );
}

#[test]
fn table_cut_checks_the_unsaved_source_extent_with_a_cached_index() {
    let mut workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "P10:Q14")),
    ]);
    workbook
        .paste(
            ("Data", "E2:F6".parse().unwrap()),
            ("Data", at("J8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(
        member(&workbook, "xl/tables/table1.xml"),
        cut_table_part(
            "Costs",
            11,
            ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
            [
                "J9+$J$9+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(J9:J11)+SUM($J$9:$J$11)+Costs[Cost]+$A$1",
            ],
        )
    );
    assert_table_cut_collision(
        workbook,
        "J8:K12",
        ("Data", "O9"),
        "Costs",
        "O9:P13",
        "P10:Q14",
    );
}

#[test]
fn table_cut_checks_the_unsaved_destination_extent_with_a_cached_index() {
    let mut workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "P10:Q14")),
    ]);
    workbook
        .paste(
            ("Data", "P10:Q14".parse().unwrap()),
            ("Data", at("K10")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(member(&workbook, "xl/tables/table1.xml"), costs_cut_part());
    assert_eq!(
        member(&workbook, "xl/tables/table2.xml"),
        stationary_cut_table("Taken", 12, "K10:L14")
    );
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Data", "J8"),
        "Costs",
        "J8:K12",
        "K10:L14",
    );
}

#[test]
fn table_cut_uses_the_cached_owner_after_a_sheet_rename() {
    let mut workbook = with_cut_tables(&[
        ("Data", costs_cut_part()),
        ("Data", stationary_cut_table("Taken", 12, "K10:L14")),
    ]);
    workbook.rename_sheet("Data", "Renamed Data").unwrap();
    assert_table_cut_collision(
        workbook,
        "E2:F6",
        ("Renamed Data", "J8"),
        "Costs",
        "J8:K12",
        "K10:L14",
    );
}

#[test]
fn table_cut_moves_tables_over_each_others_previous_extents() {
    let mut workbook = with_cut_tables(&[("Data", costs_cut_part()), ("Data", tax_cut_part())]);
    let expected = [
        cut_table_part(
            "Costs",
            11,
            ["H2:I6", "H2:I5", "H3:I5", "I3:I5"],
            [
                "H3+$H$3+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(H3:H5)+SUM($H$3:$H$5)+Costs[Cost]+$A$1",
            ],
        ),
        cut_table_part(
            "Tax",
            29,
            ["K3:L7", "K3:L6", "K4:L6", "L4:L6"],
            [
                "K4+$K$4+$A$1+Tax[[#This Row],[Cost]]",
                "SUM(K4:K6)+Tax[Cost]",
            ],
        ),
    ];
    assert_table_cut_saves_and_restores(&mut workbook, "D1:J8", "G1", &expected);
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar(at("H3")), 3.0.into());
    assert_eq!(data.scalar(at("K4")), 8.0.into());
    assert_eq!(data.scalar(at("J4")), 44.0.into());
}

#[test]
fn table_cut_allows_adjacent_table_boundaries_without_shared_cells() {
    for range in ["H8:I12", "L8:M12", "J3:K7", "J13:K17"] {
        let stationary = stationary_cut_table("Taken", 12, range);
        let mut workbook =
            with_cut_tables(&[("Data", costs_cut_part()), ("Data", stationary.clone())]);
        let moved = cut_table_part(
            "Costs",
            11,
            ["J8:K12", "J8:K11", "J9:K11", "K9:K11"],
            [
                "J9+$J$9+$A$1+Other!B3+Costs[[#This Row],[Cost]]",
                "SUM(J9:J11)+SUM($J$9:$J$11)+Costs[Cost]+$A$1",
            ],
        );
        assert_table_cut_saves_and_restores(&mut workbook, "E2:F6", "J8", &[moved, stationary]);
    }
}

#[test]
fn a_table_is_moved_grown_or_shrunk_and_never_cut() {
    let table = format!(
        "<table xmlns=\"{NS}\" id=\"1\" name=\"Costs\" displayName=\"Costs\" ref=\"B2:C6\" totalsRowShown=\"0\"><autoFilter ref=\"B2:C6\"><filterColumn colId=\"1\"><customFilters><customFilter operator=\"greaterThan\" val=\"1\"/></customFilters></filterColumn></autoFilter><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Cost\"/></tableColumns></table>"
    );
    let open = || {
        book(
            &sheet(
                "",
                "<tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts>",
            ),
            &[("rId1", "table", "../tables/table1.xml")],
            &[("xl/tables/table1.xml", &table)],
        )
    };
    let mut workbook = open();
    for (outcome, expected) in [
        (
            workbook.insert_columns("Sheet1", 2, 1),
            "invalid record value at Sheet1!B2:C6: expected columns opening outside the table Costs, got some inside it",
        ),
        (
            workbook.remove_columns("Sheet1", 2..3),
            "invalid record value at Sheet1!B2:C6: expected columns outside the table Costs, got some of its columns",
        ),
        (
            workbook.remove_rows("Sheet1", 1..2),
            "invalid record value at Sheet1!B2:C6: expected rows leaving the header, the totals and a data row of the table Costs, got its header row",
        ),
        (
            workbook.remove_rows("Sheet1", 2..6),
            "invalid record value at Sheet1!B2:C6: expected rows leaving the header, the totals and a data row of the table Costs, got every data row",
        ),
    ] {
        assert_eq!(outcome.unwrap_err().to_string(), expected);
    }
    // Rows inside it grow it; a column before it moves it, its filter's
    // column counted from its own first column still.
    workbook.insert_rows("Sheet1", 3, 2).unwrap();
    workbook.insert_columns("Sheet1", 0, 1).unwrap();
    let part = member(&workbook, "xl/tables/table1.xml");
    assert!(part.contains("ref=\"C2:D8\" totalsRowShown"), "{part}");
    assert!(
        part.contains("<autoFilter ref=\"C2:D8\"><filterColumn colId=\"1\">"),
        "{part}"
    );
    // Some data rows go: it shrinks.
    let mut workbook = open();
    workbook.remove_rows("Sheet1", 3..5).unwrap();
    let part = member(&workbook, "xl/tables/table1.xml");
    assert!(part.contains("ref=\"B2:C4\""), "{part}");
    // A table a query fills is never moved.
    let mut workbook = book(
        &sheet(
            "",
            "<tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts>",
        ),
        &[("rId1", "table", "../tables/table1.xml")],
        &[
            ("xl/tables/table1.xml", &table),
            (
                "xl/tables/_rels/table1.xml.rels",
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/queryTable\" Target=\"../queryTables/queryTable1.xml\"/></Relationships>",
            ),
            ("xl/queryTables/queryTable1.xml", "<queryTable/>"),
        ],
    );
    assert_eq!(
        workbook
            .insert_rows("Sheet1", 20, 1)
            .unwrap_err()
            .to_string(),
        "filesystem \"xl/tables/table1.xml\" does not support moving rows or columns through a table a query fills"
    );
}

#[test]
fn a_pivot_table_is_moved_whole_or_the_edit_refused() {
    let pivot = format!(
        "<pivotTableDefinition xmlns=\"{NS}\" name=\"P\" cacheId=\"1\"><location ref=\"B3:C6\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/></pivotTableDefinition>"
    );
    let mut workbook = book(
        &sheet("", ""),
        &[("rId1", "pivotTable", "../pivotTables/pivotTable1.xml")],
        &[("xl/pivotTables/pivotTable1.xml", &pivot)],
    );
    assert!(
        workbook
            .insert_rows("Sheet1", 3, 1)
            .unwrap_err()
            .to_string()
            .contains("expected rows leaving the pivot table of xl/pivotTables/pivotTable1.xml where it is")
    );
    assert!(workbook.remove_rows("Sheet1", 5..9).is_err());
    workbook.insert_rows("Sheet1", 0, 2).unwrap();
    let part = member(&workbook, "xl/pivotTables/pivotTable1.xml");
    assert!(part.contains("<location ref=\"B5:C8\""), "{part}");
}

#[test]
fn what_a_sheet_carries_follows_its_cells() {
    let carried = "<autoFilter ref=\"A1:D9\"><filterColumn colId=\"1\"/><filterColumn colId=\"3\"/></autoFilter>\
        <conditionalFormatting sqref=\"B2:B9 D2\"><cfRule type=\"expression\" priority=\"1\"><formula>B2&gt;A2</formula></cfRule></conditionalFormatting>\
        <conditionalFormatting sqref=\"C5\"><cfRule type=\"expression\" priority=\"2\"><formula>C5=1</formula></cfRule></conditionalFormatting>\
        <dataValidations count=\"2\"><dataValidation type=\"list\" sqref=\"C2:C3\"><formula1>$E$1:$E$4</formula1></dataValidation><dataValidation type=\"whole\" sqref=\"B3\"><formula1>A3</formula1></dataValidation></dataValidations>\
        <hyperlinks><hyperlink ref=\"B4\" location=\"'Sheet1'!C3\"/></hyperlinks>\
        <rowBreaks count=\"2\" manualBreakCount=\"2\"><brk id=\"2\" max=\"16383\" man=\"1\"/><brk id=\"8\" max=\"16383\" man=\"1\"/></rowBreaks>\
        <extLst><ext uri=\"{78C0D931-6437-407d-A8EE-F0AAD7539E65}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:conditionalFormattings><x14:conditionalFormatting xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:cfRule type=\"expression\" priority=\"3\"><xm:f>B5&gt;A5</xm:f></x14:cfRule><xm:sqref>B5:B6</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>";
    let mut workbook = book(&sheet("", carried), &[], &[]);
    // A column opens before B: the ranges move right, each formula keeps
    // naming the cells it named from its range's first cell.
    workbook.insert_columns("Sheet1", 1, 1).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<autoFilter ref=\"A1:E9\"><filterColumn colId=\"2\"/><filterColumn colId=\"4\"/></autoFilter>",
        "<conditionalFormatting sqref=\"C2:C9 E2\"><cfRule type=\"expression\" priority=\"1\"><formula>C2&gt;A2</formula>",
        "<formula1>$F$1:$F$4</formula1>",
        "<dataValidation type=\"whole\" sqref=\"C3\"><formula1>A3</formula1>",
        "<hyperlink ref=\"C4\" location=\"'Sheet1'!D3\"/>",
        "<xm:f>C5&gt;A5</xm:f></x14:cfRule><xm:sqref>C5:C6</xm:sqref>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
    // Rows 3 to 5 go: a rule on them goes, a validation on them goes and
    // the count says so, a break among them goes.
    workbook.remove_rows("Sheet1", 2..5).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<autoFilter ref=\"A1:E6\">",
        "<conditionalFormatting sqref=\"C2:C6 E2\">",
        "<dataValidations count=\"1\"><dataValidation type=\"list\" sqref=\"D2\"><formula1>$F$1:$F$2</formula1>",
        "<rowBreaks count=\"1\" manualBreakCount=\"1\"><brk id=\"5\" max=\"16383\" man=\"1\"/></rowBreaks>",
        // The rule's first cell went: its formula is read from the cell
        // that is its first now.
        "<xm:f>C3&gt;A3</xm:f></x14:cfRule><xm:sqref>C3</xm:sqref>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
    // The hyperlink on a removed row goes, and the list with its last one.
    for gone in ["sqref=\"C5\"", "hyperlink", "sqref=\"C3\""] {
        assert!(!part.contains(gone), "{gone} in {part}");
    }
}

#[test]
fn a_drawing_s_anchors_and_a_note_move_with_their_cells() {
    let drawing = "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\">\
        <xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>5</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>7</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>9</xdr:row><xdr:rowOff>9</xdr:rowOff></xdr:to><xdr:clientData/></xdr:twoCellAnchor>\
        <xdr:twoCellAnchor editAs=\"absolute\"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>2</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:clientData/></xdr:twoCellAnchor>\
        <xdr:absoluteAnchor><xdr:pos x=\"0\" y=\"0\"/><xdr:ext cx=\"1\" cy=\"1\"/><xdr:clientData/></xdr:absoluteAnchor></xdr:wsDr>";
    let comments = format!(
        "<comments xmlns=\"{NS}\"><authors><author>A</author></authors><commentList><comment ref=\"B3\" authorId=\"0\"><text><t>three</t></text></comment><comment ref=\"B9\" authorId=\"0\"><text><t>nine</t></text></comment></commentList></comments>"
    );
    let vml = "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">\
        <v:shape id=\"_x0000_s1025\"><x:ClientData ObjectType=\"Note\"><x:Anchor>2, 15, 2, 2, 4, 15, 6, 16</x:Anchor><x:Row>2</x:Row><x:Column>1</x:Column></x:ClientData></v:shape>\
        <v:shape id=\"_x0000_s1026\"><x:ClientData ObjectType=\"Note\"><x:Anchor>2, 15, 8, 2, 4, 15, 12, 16</x:Anchor><x:Row>8</x:Row><x:Column>1</x:Column></x:ClientData></v:shape></xml>";
    let mut workbook = book(
        &sheet("", "<drawing r:id=\"rId1\"/><legacyDrawing r:id=\"rId2\"/>"),
        &[
            ("rId1", "drawing", "../drawings/drawing1.xml"),
            ("rId2", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
            ("rId3", "comments", "../comments1.xml"),
        ],
        &[
            ("xl/drawings/drawing1.xml", drawing),
            ("xl/drawings/vmlDrawing1.vml", vml),
            ("xl/comments1.xml", &comments),
        ],
    );
    // Rows 2 to 4 go: the picture anchored in them moves to row 2, its
    // offset zero; the one anchored absolutely stays.
    workbook.remove_rows("Sheet1", 1..4).unwrap();
    let part = member(&workbook, "xl/drawings/drawing1.xml");
    assert!(
        part.contains("<xdr:from><xdr:col>1</xdr:col><xdr:colOff>5</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>9</xdr:rowOff></xdr:to>"),
        "{part}"
    );
    assert!(part.contains("<xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>2</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row>"), "{part}");
    // The note on row 3 goes with it, the one on row 9 moves up.
    let part = member(&workbook, "xl/comments1.xml");
    assert!(!part.contains("three"), "{part}");
    assert!(
        part.contains("<comment ref=\"B6\" authorId=\"0\"><text><t>nine</t>"),
        "{part}"
    );
    let part = member(&workbook, "xl/drawings/vmlDrawing1.vml");
    assert!(!part.contains("_x0000_s1025"), "{part}");
    assert!(
        part.contains("<x:Anchor>2, 15, 5, 2, 4, 15, 9, 16</x:Anchor><x:Row>5</x:Row>"),
        "{part}"
    );

    // A VML drawing that is no XML is refused by name.
    let mut workbook = book(
        &sheet("", "<legacyDrawing r:id=\"rId2\"/>"),
        &[("rId2", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
        &[(
            "xl/drawings/vmlDrawing1.vml",
            "<xml><v:shape><br></v:shape></xml>",
        )],
    );
    assert_eq!(
        workbook
            .insert_rows("Sheet1", 0, 1)
            .unwrap_err()
            .to_string(),
        "filesystem \"xl/drawings/vmlDrawing1.vml\" does not support moving the notes of a VML drawing that is not well-formed XML"
    );
}

#[test]
fn a_cut_moves_every_reference_into_the_block_and_one_into_the_cells_pasted_over_is_ref() {
    let mut workbook =
        with_formulas(&[("A1", "B2+C3+B9"), ("B2", "C2*2"), ("E1", "SUM(B2:C3)+D8")]);
    workbook
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("C2"), 5.0)
        .unwrap();
    workbook
        .paste(
            ("Data", "B2:C3".parse().unwrap()),
            ("Data", at("D8")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(
        spelled(&workbook),
        pairs(&[
            ("A1", "D8+E9+B9"),
            ("E1", "SUM(D8:E9)+#REF!"),
            // The moved formula keeps naming the cell it named, which moved
            // with it.
            ("D8", "E8*2"),
        ])
    );
    assert!(workbook.sheet("Data").unwrap().cell(at("C2")).is_none());
    assert_eq!(workbook.sheet("Data").unwrap().scalar(at("E8")), 5.0.into());
}

#[test]
fn a_sheet_built_in_memory_moves_its_merges_and_its_pane_with_its_cells() {
    let mut sheet = Sheet::new("Mem").unwrap();
    sheet.set_cell(at("A1"), 1.0).unwrap();
    sheet.merge("B2:C3".parse().unwrap()).unwrap();
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    workbook.insert_columns("Mem", 0, 2).unwrap();
    let sheet = workbook.sheet("Mem").unwrap();
    assert_eq!(sheet.scalar(at("C1")), 1.0.into());
    assert_eq!(
        sheet
            .merges()
            .map(|merge| merge.to_string())
            .collect::<Vec<_>>(),
        ["D2:E3"]
    );
}

#[test]
fn an_element_no_edit_carries_through_on_another_sheet_refuses_an_edit_it_names() {
    let consolidating = |sheet: &str| {
        sheet_part(&format!(
            "<dataConsolidate><dataRefs count=\"1\"><dataRef ref=\"A1:B2\" sheet=\"{sheet}\"/></dataRefs></dataConsolidate>"
        ))
    };
    let open = |named: &str| {
        sheets_book(
            &[
                (
                    "Data",
                    sheet("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", ""),
                    &[],
                ),
                ("Other", consolidating(named), &[]),
            ],
            &[],
            &[],
        )
    };
    let mut workbook = open("Data");
    let before = workbook.sheet("Data").unwrap().clone();
    for (outcome, expected) in [
        (
            workbook.rename_sheet("Data", "Renamed"),
            "renaming a sheet an element this crate does not model names",
        ),
        (
            workbook.remove_sheet("Data").map(drop),
            "removing a sheet an element this crate does not model names",
        ),
        (
            workbook.insert_rows("Data", 0, 1),
            "inserting rows in a sheet an element this crate does not model names",
        ),
        (
            workbook.remove_columns("Data", 0..1),
            "removing columns in a sheet an element this crate does not model names",
        ),
        (
            workbook
                .paste(
                    ("Data", "A1".parse().unwrap()),
                    ("Data", at("C3")),
                    Paste::All,
                    true,
                )
                .map(drop),
            "moving cells an element this crate does not model names",
        ),
        (
            workbook
                .paste(
                    ("Other", "A1".parse().unwrap()),
                    ("Other", at("C3")),
                    Paste::All,
                    true,
                )
                .map(drop),
            "moving cells through a sheet carrying an element this crate does not model",
        ),
    ] {
        let error = outcome.unwrap_err();
        assert!(matches!(error, Error::Unsupported { .. }), "{error}");
        assert_eq!(
            error.to_string(),
            format!(
                "filesystem \"xl/worksheets/sheet2.xml#dataConsolidate\" does not support {expected}"
            )
        );
    }
    assert_eq!(workbook.sheet_names(), ["Data", "Other"]);
    assert_eq!(workbook.sheet("Data").unwrap(), &before);
    // An element naming another sheet - a name whose letters `Data`
    // happens to hold included - names nothing the edit moves.
    let mut workbook = open("Database");
    workbook.insert_rows("Data", 0, 1).unwrap();
    workbook.rename_sheet("Data", "Renamed").unwrap();
}

/// A worksheet part with no cells and `after` past them.
fn sheet_part(after: &str) -> String {
    sheet("", after)
}

#[test]
fn a_sheet_whose_name_xml_escapes_is_found_wherever_a_part_names_it() {
    let chart = "<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:f>'P&amp;L'!$A$1:$A$5</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>";
    let cache = format!(
        "<pivotCacheDefinition xmlns=\"{NS}\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"A1:B5\" sheet=\"P&amp;L\"/></cacheSource></pivotCacheDefinition>"
    );
    let drawing = "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\"><xdr:oneCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx=\"1\" cy=\"1\"/><xdr:sp macro=\"\" textlink=\"'P&amp;L'!$A$2\"><xdr:nvSpPr><xdr:cNvPr id=\"2\" name=\"Box\"/><xdr:cNvSpPr/></xdr:nvSpPr></xdr:sp><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>";
    let other = sheet_part(
        "<conditionalFormatting sqref=\"A1\"><cfRule type=\"expression\" priority=\"1\"><formula>'P&amp;L'!$A$1&gt;0</formula></cfRule></conditionalFormatting><drawing r:id=\"rId1\"/>",
    );
    let mut workbook = sheets_book(
        &[
            ("P&amp;L", sheet_part(""), &[]),
            (
                "Other",
                other,
                &[("rId1", "drawing", "../drawings/drawing1.xml")],
            ),
        ],
        &[],
        &[
            ("xl/charts/chart1.xml", chart.to_owned()),
            ("xl/pivotCache/pivotCacheDefinition1.xml", cache),
            ("xl/drawings/drawing1.xml", drawing.to_owned()),
        ],
    );
    let parts = |workbook: &Workbook| {
        [
            "xl/charts/chart1.xml",
            "xl/pivotCache/pivotCacheDefinition1.xml",
            "xl/drawings/drawing1.xml",
            "xl/worksheets/sheet2.xml",
        ]
        .map(|name| member(workbook, name))
    };
    workbook.insert_rows("P&L", 0, 2).unwrap();
    let [chart, cache, drawing, other] = parts(&workbook);
    assert!(chart.contains("<c:f>'P&amp;L'!$A$3:$A$7</c:f>"), "{chart}");
    assert!(cache.contains("ref=\"A3:B7\" sheet=\"P&amp;L\""), "{cache}");
    assert!(drawing.contains("textlink=\"'P&amp;L'!$A$4\""), "{drawing}");
    assert!(
        other.contains("<formula>'P&amp;L'!$A$3&gt;0</formula>"),
        "{other}"
    );
    workbook.rename_sheet("P&L", "Q&A").unwrap();
    let [chart, cache, drawing, other] = parts(&workbook);
    assert!(chart.contains("<c:f>'Q&amp;A'!$A$3:$A$7</c:f>"), "{chart}");
    assert!(cache.contains("sheet=\"Q&amp;A\""), "{cache}");
    assert!(drawing.contains("textlink=\"'Q&amp;A'!$A$4\""), "{drawing}");
    assert!(
        other.contains("<formula>'Q&amp;A'!$A$3&gt;0</formula>"),
        "{other}"
    );
    workbook.remove_sheet("Q&A").unwrap();
    let [chart, _, drawing, other] = parts(&workbook);
    for part in [&chart, &drawing, &other] {
        assert!(!part.contains("Q&amp;A"), "{part}");
        assert!(part.contains("#REF!"), "{part}");
    }
}

#[test]
fn a_threshold_a_formula_states_follows_its_cells() {
    let carried = "<conditionalFormatting sqref=\"D2:D9\"><cfRule type=\"colorScale\" priority=\"1\"><colorScale><cfvo type=\"min\"/><cfvo type=\"formula\" val=\"$D$1\"/><cfvo type=\"num\" val=\"Sheet1!$A$5\"/><color rgb=\"FFFF0000\"/><color rgb=\"FFFFFF00\"/><color rgb=\"FF00FF00\"/></colorScale></cfRule></conditionalFormatting>\
        <conditionalFormatting sqref=\"E2:E9\"><cfRule type=\"dataBar\" priority=\"2\"><dataBar><cfvo type=\"num\" val=\"0\"/><cfvo type=\"formula\" val=\"MAX($E$2:$E$9)\"/><color rgb=\"FF638EC6\"/></dataBar></cfRule></conditionalFormatting>";
    let mut workbook = book(&sheet("", carried), &[], &[]);
    workbook.insert_rows("Sheet1", 0, 2).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<conditionalFormatting sqref=\"D4:D11\">",
        "<cfvo type=\"formula\" val=\"$D$3\"/><cfvo type=\"num\" val=\"Sheet1!$A$7\"/>",
        "<cfvo type=\"num\" val=\"0\"/><cfvo type=\"formula\" val=\"MAX($E$4:$E$11)\"/>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
    workbook.rename_sheet("Sheet1", "Renamed").unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    assert!(part.contains("val=\"Renamed!$A$7\""), "{part}");
}

#[test]
fn a_filter_keeps_its_columns_when_rows_move_and_a_table_s_filter_leaves_its_totals_out() {
    let carried = "<autoFilter ref=\"A1:D9\"><filterColumn colId=\"1\"><filters><filter val=\"x\"/></filters></filterColumn><filterColumn colId=\"3\"><filters><filter val=\"y\"/></filters></filterColumn></autoFilter>";
    let mut workbook = book(&sheet("", carried), &[], &[]);
    workbook.insert_rows("Sheet1", 0, 1).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<autoFilter ref=\"A2:D10\"><filterColumn colId=\"1\"><filters><filter val=\"x\"/></filters></filterColumn><filterColumn colId=\"3\">"),
        "{part}"
    );
    workbook.remove_rows("Sheet1", 0..1).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    assert!(part.contains(carried), "{part}");
    // A column going before a filtered one moves it; one going with it
    // takes its criteria.
    workbook.remove_columns("Sheet1", 1..2).unwrap();
    let part = member(&workbook, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains(
            "<autoFilter ref=\"A1:C9\"><filterColumn colId=\"2\"><filters><filter val=\"y\"/>"
        ),
        "{part}"
    );
    assert!(!part.contains("val=\"x\""), "{part}");

    let table = format!(
        "<table xmlns=\"{NS}\" id=\"1\" name=\"Costs\" displayName=\"Costs\" ref=\"B2:C6\" totalsRowCount=\"1\"><autoFilter ref=\"B2:C5\"><filterColumn colId=\"1\"><filters><filter val=\"z\"/></filters></filterColumn></autoFilter><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\" totalsRowLabel=\"Total\"/><tableColumn id=\"2\" name=\"Cost\" totalsRowFunction=\"sum\"/></tableColumns></table>"
    );
    let mut workbook = book(
        &sheet(
            "",
            "<tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts>",
        ),
        &[("rId1", "table", "../tables/table1.xml")],
        &[("xl/tables/table1.xml", &table)],
    );
    // A row above the table moves it and its filter whole, criteria kept.
    workbook.insert_rows("Sheet1", 0, 1).unwrap();
    let part = member(&workbook, "xl/tables/table1.xml");
    assert!(
        part.contains("ref=\"B3:C7\" totalsRowCount=\"1\"><autoFilter ref=\"B3:C6\"><filterColumn colId=\"1\">"),
        "{part}"
    );
    // A row opening at the totals row is a data row: the filter grows to
    // it and leaves the totals out still.
    workbook.insert_rows("Sheet1", 6, 1).unwrap();
    let part = member(&workbook, "xl/tables/table1.xml");
    assert!(
        part.contains("ref=\"B3:C8\" totalsRowCount=\"1\"><autoFilter ref=\"B3:C7\">"),
        "{part}"
    );
    workbook.remove_rows("Sheet1", 0..1).unwrap();
    let part = member(&workbook, "xl/tables/table1.xml");
    assert!(
        part.contains("ref=\"B2:C7\" totalsRowCount=\"1\"><autoFilter ref=\"B2:C6\"><filterColumn colId=\"1\">"),
        "{part}"
    );
}

#[test]
fn a_table_stating_more_header_and_totals_rows_than_it_has_is_refused_by_name() {
    let table = format!(
        "<table xmlns=\"{NS}\" id=\"1\" name=\"Odd\" displayName=\"Odd\" ref=\"B2:C6\" headerRowCount=\"4294967295\" totalsRowCount=\"4294967295\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"A\"/><tableColumn id=\"2\" name=\"B\"/></tableColumns></table>"
    );
    let mut workbook = book(
        &sheet(
            "",
            "<tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts>",
        ),
        &[("rId1", "table", "../tables/table1.xml")],
        &[("xl/tables/table1.xml", &table)],
    );
    for outcome in [
        workbook.remove_rows("Sheet1", 2..3),
        workbook.insert_rows("Sheet1", 0, 1),
    ] {
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "invalid record value at Sheet1!B2:C6: expected the table Odd to state at most 5 header and totals rows, got 4294967295 and 4294967295"
        );
    }
}

#[test]
fn a_shape_linked_to_a_cell_follows_it() {
    let drawing = "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\"><xdr:twoCellAnchor><xdr:from><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro=\"\" textlink=\"$A$5\"><xdr:nvSpPr><xdr:cNvPr id=\"2\" name=\"Box\"/><xdr:cNvSpPr/></xdr:nvSpPr></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>";
    let mut workbook = book(
        &sheet("", "<drawing r:id=\"rId1\"/>"),
        &[("rId1", "drawing", "../drawings/drawing1.xml")],
        &[("xl/drawings/drawing1.xml", drawing)],
    );
    workbook.insert_rows("Sheet1", 0, 2).unwrap();
    let part = member(&workbook, "xl/drawings/drawing1.xml");
    assert!(part.contains("<xdr:row>5</xdr:row>"), "{part}");
    assert!(part.contains("textlink=\"$A$7\""), "{part}");
    workbook.remove_rows("Sheet1", 6..7).unwrap();
    let part = member(&workbook, "xl/drawings/drawing1.xml");
    assert!(part.contains("textlink=\"#REF!\""), "{part}");
}

#[test]
fn removing_a_sheet_a_slicer_or_timeline_cache_filters_is_refused_by_name() {
    let pivot = format!(
        "<pivotTableDefinition xmlns=\"{NS}\" name=\"PivotTable1\" cacheId=\"1\"><location ref=\"A3:B6\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/></pivotTableDefinition>"
    );
    let table = format!(
        "<table xmlns=\"{NS}\" id=\"7\" name=\"Costs\" displayName=\"Costs\" ref=\"A1:B3\"><autoFilter ref=\"A1:B3\"/><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Item\"/><tableColumn id=\"2\" name=\"Cost\"/></tableColumns></table>"
    );
    let slicer = "<slicerCacheDefinition xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" name=\"Slicer_Region\" sourceName=\"Region\"><pivotTables><pivotTable tabId=\"2\" name=\"PivotTable1\"/></pivotTables><data><tabular pivotCacheId=\"1\"><items count=\"0\"/></tabular></data></slicerCacheDefinition>";
    let table_slicer = "<slicerCacheDefinition xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\" name=\"Slicer_Item\" sourceName=\"Item\"><extLst><x:ext xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" uri=\"{2F2917AC-EB37-4324-AD4E-5DD8C200BD13}\"><x15:tableSlicerCache tableId=\"7\" column=\"1\"/></x:ext></extLst></slicerCacheDefinition>";
    let timeline = "<timelineCacheDefinition xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\" name=\"NativeTimeline_Date\" sourceName=\"Date\"><pivotTables><pivotTable tabId=\"2\" name=\"PivotTable1\"/></pivotTables></timelineCacheDefinition>";
    let open = |caches: &[(&str, &str, &str)]| {
        sheets_book(
            &[
                (
                    "Data",
                    sheet_part("<tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts>"),
                    &[("rId1", "table", "../tables/table1.xml")],
                ),
                (
                    "Pivot",
                    sheet_part(""),
                    &[("rId1", "pivotTable", "../pivotTables/pivotTable1.xml")],
                ),
                ("Notes", sheet_part(""), &[]),
            ],
            caches,
            &[
                ("xl/tables/table1.xml", table.clone()),
                ("xl/pivotTables/pivotTable1.xml", pivot.clone()),
                ("xl/slicerCaches/slicerCache1.xml", slicer.to_owned()),
                ("xl/slicerCaches/slicerCache2.xml", table_slicer.to_owned()),
                ("xl/timelineCaches/timelineCache1.xml", timeline.to_owned()),
            ],
        )
    };
    for (cache, removed) in [
        (
            ("rIdC", "slicerCache", "slicerCaches/slicerCache1.xml"),
            "Pivot",
        ),
        (
            ("rIdC", "slicerCache", "slicerCaches/slicerCache2.xml"),
            "Data",
        ),
        (
            ("rIdC", "timelineCache", "timelineCaches/timelineCache1.xml"),
            "Pivot",
        ),
    ] {
        let mut workbook = open(&[cache]);
        let error = workbook.remove_sheet(removed).unwrap_err();
        assert!(matches!(error, Error::Unsupported { .. }), "{error}");
        assert_eq!(
            error.to_string(),
            format!(
                "filesystem \"xl/{}\" does not support removing a sheet hosting a pivot table or table a slicer or timeline cache names",
                cache.2
            )
        );
        assert_eq!(workbook.sheet_names(), ["Data", "Pivot", "Notes"]);
        // The cache names the pivot or table by its tab's number and its
        // id, which a rename keeps; a sheet it names nothing on goes.
        workbook.rename_sheet(removed, "Renamed").unwrap();
        workbook.remove_sheet("Notes").unwrap();
    }
}

#[test]
fn a_cut_moves_an_array_formula_s_range_with_it_onto_another_sheet() {
    let rows = "<row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\"><f t=\"array\" ref=\"B2:B3\">A2:A3*2</f><v>2</v></c></row><row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\"><v>4</v></c></row>";
    let mut workbook = sheets_book(
        &[
            ("Sheet1", sheet(rows, ""), &[]),
            ("Sheet2", sheet_part(""), &[]),
        ],
        &[],
        &[],
    );
    workbook
        .paste(
            ("Sheet1", "B2:B3".parse().unwrap()),
            ("Sheet2", at("D5")),
            Paste::All,
            true,
        )
        .unwrap();
    let part = member(&workbook, "xl/worksheets/sheet2.xml");
    assert!(
        part.contains("<f t=\"array\" ref=\"D5:D6\">Sheet1!A2:A3*2</f>"),
        "{part}"
    );
}

#[test]
fn a_cut_moves_a_shape_s_link_into_the_cells_it_moves() {
    let drawing = "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\"><xdr:oneCellAnchor><xdr:from><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx=\"1\" cy=\"1\"/><xdr:sp macro=\"\" textlink=\"$A$1\"><xdr:nvSpPr><xdr:cNvPr id=\"2\" name=\"Box\"/><xdr:cNvSpPr/></xdr:nvSpPr></xdr:sp><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>";
    let mut workbook = book(
        &sheet(
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
            "<drawing r:id=\"rId1\"/>",
        ),
        &[("rId1", "drawing", "../drawings/drawing1.xml")],
        &[("xl/drawings/drawing1.xml", drawing)],
    );
    workbook
        .paste(
            ("Sheet1", "A1".parse().unwrap()),
            ("Sheet1", at("C7")),
            Paste::All,
            true,
        )
        .unwrap();
    let part = member(&workbook, "xl/drawings/drawing1.xml");
    // The link follows the cell; the shape stays where it is drawn.
    assert!(part.contains("textlink=\"$C$7\""), "{part}");
    assert!(part.contains("<xdr:row>3</xdr:row>"), "{part}");
}

#[test]
fn vml_note_band_translates_both_anchor_corners_with_its_owner() {
    use yggdryl::excel::Edit;
    // Excel-confirmed anchors: the box begins above the inserted row but
    // follows its owner. The fixture also records native save quantization.
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vml_excel.json")).unwrap();
    let anchor = |value: &serde_json::Value| {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|number| number.as_i64().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let original = anchor(&oracle["initial"]["anchor"]);
    for case in oracle["cases"].as_array().unwrap() {
        let rows = case["insert_row"].is_u64();
        let columns = case["insert_column"].is_u64();
        let expected = anchor(&case["anchor"]);
        let cell = yggdryl::excel::CellRef::new(
            u32::try_from(case["row"].as_u64().unwrap()).unwrap(),
            u32::try_from(case["column"].as_u64().unwrap()).unwrap(),
        );
        let comments = format!(
            r#"<comments xmlns="{NS}"><authors><author>yggdryl</author></authors><commentList><comment ref="A4" authorId="0"><text><t>note</t></text></comment></commentList></comments>"#
        );
        let vml = format!(
            r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="_x0000_s1026"><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{original}</x:Anchor><x:Row>3</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#
        );
        let mut workbook = book(
            &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
            &[
                ("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
                ("comments", "comments", "../comments1.xml"),
            ],
            &[
                ("xl/drawings/vmlDrawing1.vml", &vml),
                ("xl/comments1.xml", &comments),
            ],
        );
        // Save a modeled cell first: the inverse compares package facts after
        // the writer has introduced its dimension and calculation policy.
        workbook.set_entry("Sheet1", at("A4"), "1").unwrap();
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let before = table_member_map(&workbook);
        let mut edits = Vec::new();
        if rows {
            edits.push(Edit::InsertRows {
                sheet: "Sheet1".into(),
                at: 2,
                count: 1,
            });
        }
        if columns {
            edits.push(Edit::InsertColumns {
                sheet: "Sheet1".into(),
                at: 0,
                count: 1,
            });
        }
        let applied = workbook.apply(Edit::Batch(edits)).unwrap();
        assert!(
            member(&workbook, "xl/drawings/vmlDrawing1.vml")
                .contains(&format!("<x:Anchor>{expected}</x:Anchor>"))
        );
        assert!(member(&workbook, "xl/comments1.xml").contains(&format!("ref=\"{cell}\"")));
        let after = table_member_map(&workbook);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), before);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        workbook.apply(undone.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), after);
    }
}

#[test]
fn vml_note_band_refuses_missing_or_outside_anchors_before_mutation() {
    let part = "xl/drawings/vmlDrawing1.vml";
    for (anchor, remove, operation) in [
        ("", false, "moving a VML note without a cell anchor"),
        (
            "<x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor>",
            true,
            "moving a VML note anchor outside the worksheet grid",
        ),
    ] {
        let vml = format!(
            r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="_x0000_s1026" style="position:absolute;margin-left:59.25pt;margin-top:1.5pt;width:144px;height:79px"><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/>{anchor}<x:Row>3</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#
        );
        let mut workbook = book(
            &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
            &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
            &[(part, &vml)],
        );
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revision = workbook.sheet("Sheet1").unwrap().revision();
        let dirty = workbook.is_dirty();
        let result = if remove {
            workbook.remove_rows("Sheet1", 0..1)
        } else {
            workbook.insert_rows("Sheet1", 2, 1)
        };
        assert!(
            matches!(result.unwrap_err(), Error::Unsupported { operation: actual, filesystem }
            if actual == operation && filesystem == part)
        );
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn vml_note_band_preserves_foreign_coordinate_lookalikes() {
    let part = "xl/drawings/vmlDrawing1.vml";
    let other = r#"<a:shape id="foreign"><x:ClientData ObjectType="Note"><x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row></x:ClientData></a:shape>"#;
    let vml = format!(
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:a="urn:foreign">{other}<v:shape id="note"><x:ClientData ObjectType="Note"><x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row><x:Column>0</x:Column><a:Row>3</a:Row><a:Anchor>0, 79, 0, 2, 2, 29, 3, 11</a:Anchor></x:ClientData></v:shape></xml>"#
    );
    let mut workbook = book(
        &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
        &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
        &[(part, &vml)],
    );
    workbook.insert_rows("Sheet1", 2, 1).unwrap();
    let changed = member(&workbook, part);
    assert!(changed.contains(other));
    assert!(changed.contains("<x:Anchor>0, 79, 1, 2, 2, 29, 4, 11</x:Anchor><x:Row>4</x:Row>"));
    assert!(changed.contains("<a:Row>3</a:Row><a:Anchor>0, 79, 0, 2, 2, 29, 3, 11</a:Anchor>"));
}

#[test]
fn vml_note_band_reads_pretty_xml_without_rebinding_foreign_client_data() {
    let part = "xl/drawings/vmlDrawing1.vml";
    let foreign = "<f:ClientData ObjectType=\"Note\"><x:Row>99</x:Row><x:Anchor>9, 1, 9, 1, 10, 1, 10, 1</x:Anchor></f:ClientData>";
    for tail in [false, true] {
        let data = "<x:ClientData ObjectType=\"Note\">\n  <x:MoveWithCells/>\n  <x:SizeWithCells/>\n  <x:Anchor>\n    0, 79, 0, 2, 2, 29, 3, 11</x:Anchor>\n  <x:Row>3</x:Row>\n  <x:Column>0</x:Column>\n </x:ClientData>";
        let content = if tail {
            format!("{data}\n{foreign}")
        } else {
            format!("{foreign}\n{data}")
        };
        let vml = format!(
            "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\" xmlns:f=\"urn:foreign\">\n <v:shape id=\"note\">\n {content}\n </v:shape>\n</xml>"
        );
        let mut workbook = book(
            &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
            &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
            &[(part, &vml)],
        );
        workbook.insert_rows("Sheet1", 2, 1).unwrap();
        let changed = member(&workbook, part);
        assert!(changed.contains(foreign));
        assert!(changed.contains("<x:Anchor>\n    0, 79, 1, 2, 2, 29, 4, 11</x:Anchor>"));
        assert!(changed.contains("<x:Row>4</x:Row>\n  <x:Column>0</x:Column>"));
    }
}

#[test]
fn vml_note_band_refuses_ambiguous_client_data_atomically() {
    let part = "xl/drawings/vmlDrawing1.vml";
    let note = "<x:ClientData ObjectType=\"Note\"><x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row></x:ClientData>";
    let control = "<x:ClientData ObjectType=\"Button\"><x:Row>9</x:Row></x:ClientData>";
    for children in [
        format!("{note}{control}"),
        format!("{control}{note}"),
        format!("{note}{note}"),
    ] {
        let vml = format!(
            "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><v:shape id=\"note\">{children}</v:shape></xml>"
        );
        let mut workbook = book(
            &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
            &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
            &[(part, &vml)],
        );
        workbook.parse_all().unwrap();
        let before = table_member_map(&workbook);
        let revision = workbook.sheet("Sheet1").unwrap().revision();
        let dirty = workbook.is_dirty();
        assert!(matches!(workbook.insert_rows("Sheet1", 2, 1).unwrap_err(),
            Error::InvalidRecord { path, reason } if path == part
                && reason == "expected one ClientData for a VML shape"));
        assert_eq!(table_member_map(&workbook), before);
        assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
        assert_eq!(workbook.is_dirty(), dirty);
    }
}

#[test]
fn vml_note_band_refuses_genuine_controls_without_claiming_their_geometry() {
    let part = "xl/drawings/vmlDrawing1.vml";
    for placement in [
        "",
        "<x:MoveWithCells/><x:SizeWithCells/>",
        "<x:MoveWithCells>False</x:MoveWithCells><x:SizeWithCells>False</x:SizeWithCells>",
    ] {
        for columns in [false, true] {
            let vml = format!(
                r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="button"><x:ClientData ObjectType="Button">{placement}<x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#
            );
            let mut workbook = book(
                &sheet("", r#"<legacyDrawing r:id="vml"/>"#),
                &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
                &[(part, &vml)],
            );
            workbook.parse_all().unwrap();
            let before = table_member_map(&workbook);
            let revision = workbook.sheet("Sheet1").unwrap().revision();
            let dirty = workbook.is_dirty();
            let error = if columns {
                workbook.insert_columns("Sheet1", 0, 1)
            } else {
                workbook.insert_rows("Sheet1", 2, 1)
            }
            .unwrap_err();
            assert!(matches!(error, Error::Unsupported { operation, filesystem }
                if operation == "moving a VML control with unmodeled geometry" && filesystem == part));
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
            assert_eq!(workbook.is_dirty(), dirty);
        }
    }
}

/// Local-only producer for `check_excel_desktop.py shift --active`: Excel's
/// already-saved source is the baseline, so the oracle compares reopened files.
/// Input/output may be overridden with YGGDRYL_EXCEL_SHIFT_INPUT/OUTPUT.
#[test]
#[ignore = "requires a local Excel-authored anchored workbook; run explicitly for the desktop oracle"]
fn export_native_anchored_shift_for_excel() {
    use std::io::Write as _;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let input = std::env::var_os("YGGDRYL_EXCEL_SHIFT_INPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("target/excel-desktop/vml-band-native/packages/before.xlsx"));
    let output = std::env::var_os("YGGDRYL_EXCEL_SHIFT_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("target/excel-desktop/native-anchored-shift.xlsx"));
    let note = output.with_extension("json");
    assert!(
        !output.exists() && !note.exists(),
        "select fresh output paths; no oracle evidence is overwritten"
    );
    let source = std::fs::read(&input).unwrap();
    let mut workbook = Workbook::from_bytes(source).unwrap();
    workbook.insert_rows("Data", 2, 1).unwrap();
    workbook.insert_columns("Data", 0, 1).unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let reopened = Workbook::from_bytes(bytes.clone()).unwrap();
    assert!(reopened.sheet("Data").is_ok());
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&bytes).unwrap();
    let metadata = serde_json::json!({
        "schema_version": 1, "source": "rust/tests/excel/shift.rs",
        "input": input, "output": output,
        "operations": [{"sheet": "Data", "insert_row": 3, "count": 1},
                       {"sheet": "Data", "insert_column": 1, "count": 1}],
        "excel_verified": false,
        "verification": "Run check_excel_desktop.py shift against the same input and this output; retain the Excel version from that result."
    });
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&note)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &metadata).unwrap();
    file.write_all(b"\n").unwrap();
    println!(
        "exported native anchored shift to {}; desktop check is pending",
        output.display()
    );
}

#[test]
fn vml_note_band_refuses_duplicate_coordinate_metadata_atomically() {
    let part = "xl/drawings/vmlDrawing1.vml";
    for (extra, reason) in [
        ("<x:Row>3</x:Row>", "expected one Row for a VML note"),
        (
            "<x:Column>0</x:Column>",
            "expected one Column for a VML note",
        ),
        (
            "<x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor>",
            "expected one Anchor for a VML note",
        ),
    ] {
        for columns in [false, true] {
            let vml = format!(
                r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="note"><x:ClientData ObjectType="Note"><x:Anchor>0, 79, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row><x:Column>0</x:Column>{extra}</x:ClientData></v:shape></xml>"#
            );
            let mut workbook = book(
                &sheet(
                    r#"<row r="4"><c r="A4"><v>1</v></c></row>"#,
                    r#"<legacyDrawing r:id="vml"/>"#,
                ),
                &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
                &[(part, &vml)],
            );
            workbook.parse_all().unwrap();
            let before = table_member_map(&workbook);
            let revision = workbook.sheet("Sheet1").unwrap().revision();
            let dirty = workbook.is_dirty();
            let error = if columns {
                workbook.insert_columns("Sheet1", 0, 1)
            } else {
                workbook.insert_rows("Sheet1", 2, 1)
            }
            .unwrap_err();
            assert!(
                matches!(error, Error::InvalidRecord { path, reason: actual }
                if path == part && actual == reason)
            );
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
            assert_eq!(workbook.is_dirty(), dirty);
        }
    }
}

#[test]
fn vml_note_band_validates_anchor_text_and_both_axes_before_any_delta() {
    let part = "xl/drawings/vmlDrawing1.vml";
    for anchor in [
        "<x:Anchor/>",
        "<x:Anchor></x:Anchor>",
        "<x:Anchor> </x:Anchor>",
        "<x:Anchor><x:ignored/></x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 2, 29, 3</x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 2, 29, 3, 11, 0</x:Anchor>",
        "<x:Anchor>16384, 79, 0, 2, 2, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 1048576, 2, 2, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 16384, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 2, 29, 1048576, 11</x:Anchor>",
        "<x:Anchor>-1, 79, 0, 2, 2, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, -1, 0, 2, 2, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 0, -1, 2, 29, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 2, -1, 3, 11</x:Anchor>",
        "<x:Anchor>0, 79, 0, 2, 2, 29, 3, -1</x:Anchor>",
    ] {
        // The second row edit has delta zero at the Note. Intake must not
        // skip validation just because this particular owner stays put.
        for at_row in [2, 100] {
            let vml = format!(
                r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="note"><x:ClientData ObjectType="Note">{anchor}<x:Row>3</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#
            );
            let mut workbook = book(
                &sheet(
                    r#"<row r="4"><c r="A4"><v>1</v></c></row>"#,
                    r#"<legacyDrawing r:id="vml"/>"#,
                ),
                &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
                &[(part, &vml)],
            );
            workbook.parse_all().unwrap();
            let before = table_member_map(&workbook);
            let revision = workbook.sheet("Sheet1").unwrap().revision();
            let dirty = workbook.is_dirty();
            let error = workbook.insert_rows("Sheet1", at_row, 1).unwrap_err();
            assert!(
                matches!(error, Error::InvalidRecord { path, reason }
                if path == part && reason == "expected eight VML anchor integers with in-grid coordinates and nonnegative offsets"),
                "{anchor} at {at_row}"
            );
            assert_eq!(table_member_map(&workbook), before);
            assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
            assert_eq!(workbook.is_dirty(), dirty);
        }
    }
}

#[test]
fn vml_note_band_keeps_nonnegative_offsets_without_an_invented_pixel_bound() {
    let part = "xl/drawings/vmlDrawing1.vml";
    let vml = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="note"><x:ClientData ObjectType="Note"><x:Anchor>0, 1099511627776, 0, 2, 2, 29, 3, 11</x:Anchor><x:Row>3</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"#;
    let mut workbook = book(
        &sheet(
            r#"<row r="4"><c r="A4"><v>1</v></c></row>"#,
            r#"<legacyDrawing r:id="vml"/>"#,
        ),
        &[("vml", "vmlDrawing", "../drawings/vmlDrawing1.vml")],
        &[(part, vml)],
    );
    workbook.insert_rows("Sheet1", 2, 1).unwrap();
    assert!(
        member(&workbook, part)
            .contains("<x:Anchor>0, 1099511627776, 1, 2, 2, 29, 4, 11</x:Anchor>")
    );
}

fn partial_carried_geometry_book(source: &str, destination: &str) -> Workbook {
    let data = r#"<row r="1"><c r="A1"><v>1</v></c></row>"#;
    let mut workbook = sheets_book(
        &[
            ("Data", sheet(data, source), &[]),
            ("Other", sheet(data, destination), &[]),
        ],
        &[],
        &[],
    );
    // Normalize modeled worksheet facts and the default style registration
    // before comparing exact saved inverses with this synthetic package.
    for name in ["Data", "Other"] {
        workbook
            .sheet_mut(name)
            .unwrap()
            .set_cell(at("A1"), 1.0)
            .unwrap();
    }
    let package = workbook.into_package().unwrap();
    workbook.rebase(package).unwrap();
    workbook.parse_all().unwrap();
    workbook
}

fn partial_carried_geometry_entries(
    workbook: &Workbook,
    part: &str,
    container: &str,
    child: &str,
    identity: &str,
) -> Vec<(String, Vec<String>)> {
    let xml = member(workbook, part);
    let document = yggdryl::xml::from_bytes(xml.as_bytes()).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    let Some(container) = root.child(Some(NS), container) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = container
        .children_in(Some(NS), child)
        .into_iter()
        .map(|entry| {
            let mut ranges: Vec<_> = entry
                .attribute_in(None, "sqref")
                .unwrap()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            ranges.sort();
            (
                entry.attribute_in(None, identity).unwrap().to_owned(),
                ranges,
            )
        })
        .collect();
    entries.sort();
    entries
}

#[test]
fn partial_carried_geometry_cross_sheet_partitions_source_and_destination() {
    use yggdryl::excel::Edit;
    for (container, child, identity, source, destination) in [
        (
            "protectedRanges",
            "protectedRange",
            "name",
            r#"<protectedRanges><protectedRange name="source" sqref="B2:D5" password="ABCD"/></protectedRanges>"#,
            r#"<protectedRanges><protectedRange name="target" sqref="J9:L13" password="FEDC"/></protectedRanges>"#,
        ),
        (
            "ignoredErrors",
            "ignoredError",
            "evalError",
            r#"<ignoredErrors><ignoredError sqref="B2:D5" evalError="1" numberStoredAsText="0"/></ignoredErrors>"#,
            r#"<ignoredErrors><ignoredError sqref="J9:L13" evalError="0" numberStoredAsText="1"/></ignoredErrors>"#,
        ),
    ] {
        let mut workbook = partial_carried_geometry_book(source, destination);
        let before = table_member_map(&workbook);
        let applied = workbook
            .apply(Edit::Paste {
                from: ("Data".into(), "C3:D4".parse().unwrap()),
                to: ("Other".into(), at("J10")),
                what: Paste::All,
                cut: true,
            })
            .unwrap();
        let strings = |ranges: &[&str]| {
            ranges
                .iter()
                .map(|range| (*range).to_owned())
                .collect::<Vec<_>>()
        };
        let source_id = if identity == "name" { "source" } else { "1" };
        let target_id = if identity == "name" { "target" } else { "0" };
        assert_eq!(
            partial_carried_geometry_entries(
                &workbook,
                "xl/worksheets/sheet1.xml",
                container,
                child,
                identity
            ),
            vec![(source_id.into(), strings(&["B2:D2", "B3:B4", "B5:D5"]))]
        );
        let mut expected = vec![
            (source_id.into(), strings(&["J10:K11"])),
            (target_id.into(), strings(&["J12:L13", "J9:L9", "L10:L11"])),
        ];
        expected.sort();
        assert_eq!(
            partial_carried_geometry_entries(
                &workbook,
                "xl/worksheets/sheet2.xml",
                container,
                child,
                identity
            ),
            expected
        );
        let after = table_member_map(&workbook);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        let undo = workbook.apply(applied.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), before);
        let package = workbook.into_package().unwrap();
        workbook.rebase(package).unwrap();
        workbook.apply(undo.inverse.unwrap()).unwrap();
        assert_eq!(table_member_map(&workbook), after);
    }
}

#[test]
fn partial_carried_geometry_same_sheet_source_wins_both_overlap_directions() {
    for (container, child, identity, fragment) in [
        (
            "protectedRanges",
            "protectedRange",
            "name",
            r#"<protectedRanges><protectedRange name="source" sqref="B2:D5" password="ABCD"/></protectedRanges>"#,
        ),
        (
            "ignoredErrors",
            "ignoredError",
            "evalError",
            r#"<ignoredErrors><ignoredError sqref="B2:D5" evalError="1" numberStoredAsText="0"/></ignoredErrors>"#,
        ),
    ] {
        for (target, expected) in [
            ("D4", vec!["B2:D2", "B3:B4", "B5:C5", "D4:E5"]),
            ("B2", vec!["B2:C3", "B4", "B5:D5", "D2"]),
        ] {
            let mut workbook = partial_carried_geometry_book(fragment, "");
            workbook
                .paste(
                    ("Data", "C3:D4".parse().unwrap()),
                    ("Data", at(target)),
                    Paste::All,
                    true,
                )
                .unwrap();
            let entries = partial_carried_geometry_entries(
                &workbook,
                "xl/worksheets/sheet1.xml",
                container,
                child,
                identity,
            );
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].1, expected, "{container}: {target}");
            assert!(
                partial_carried_geometry_entries(
                    &workbook,
                    "xl/worksheets/sheet2.xml",
                    container,
                    child,
                    identity
                )
                .is_empty()
            );
        }
    }
}

/// Each entry is one rule's sparse area list and ordered formula operands.
/// Legacy and x14 spellings are one observed CF/DV behavior here.
fn partial_formula_rules(xml: &str) -> Vec<(Vec<CellRange>, Vec<String>)> {
    use yggdryl::xml::Element;
    const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
    fn formulas(element: &Element<'_>, out: &mut Vec<String>) {
        let recognized = (element.namespace() == Some(NS)
            && matches!(element.local_name(), "formula" | "formula1" | "formula2"))
            || (element.namespace() == Some(XM) && element.local_name() == "f");
        if recognized && let Some(text) = element.text() {
            out.push(text.to_owned());
        }
        for child in element.children() {
            formulas(&child, out);
        }
    }
    fn walk(element: &Element<'_>, out: &mut Vec<(Vec<CellRange>, Vec<String>)>) {
        if [Some(NS), Some(X14)].contains(&element.namespace())
            && matches!(
                element.local_name(),
                "conditionalFormatting" | "dataValidation"
            )
        {
            let sqref = element
                .attribute_in(None, "sqref")
                .map(str::to_owned)
                .or_else(|| {
                    element
                        .child(Some(XM), "sqref")
                        .and_then(|child| child.text().map(str::to_owned))
                })
                .unwrap();
            let ranges: Vec<CellRange> = sqref
                .split_whitespace()
                .map(|area| area.parse().unwrap())
                .collect();
            if element.local_name() == "dataValidation" {
                let mut held = Vec::new();
                formulas(element, &mut held);
                out.push((ranges, held));
            } else {
                for rule in element.children().filter(|child| {
                    child.local_name() == "cfRule"
                        && [Some(NS), Some(X14)].contains(&child.namespace())
                }) {
                    let mut held = Vec::new();
                    formulas(&rule, &mut held);
                    out.push((ranges.clone(), held));
                }
            }
            return;
        }
        for child in element.children() {
            walk(&child, out);
        }
    }
    let document = yggdryl::xml::from_bytes(xml.as_bytes()).unwrap();
    let mut out = Vec::new();
    walk(&Element::root(&document).unwrap(), &mut out);
    out
}

fn partial_formula_meaning(
    rules: Vec<(Vec<CellRange>, Vec<String>)>,
    sheet_name: &str,
) -> std::collections::BTreeMap<CellRef, Vec<Vec<String>>> {
    use yggdryl::excel::Formula;
    let mut cells = std::collections::BTreeMap::<CellRef, Vec<Vec<String>>>::new();
    for (ranges, texts) in rules {
        // Excel's disconnected sqref host is the bounding rectangle's
        // top-left, even where that cell is not itself in the area union.
        let host = CellRef::new(
            ranges.iter().map(|area| area.start().row()).min().unwrap(),
            ranges
                .iter()
                .map(|area| area.start().column())
                .min()
                .unwrap(),
        );
        let formulas: Vec<_> = texts
            .iter()
            .map(|text| Formula::from_file(text, host))
            .collect();
        let mut visited = std::collections::BTreeSet::new();
        for area in ranges {
            assert!(
                area.end().row() < 20 && area.end().column() < 20,
                "native fixture is deliberately small; never expand a production range"
            );
            for row in area.start().row()..=area.end().row() {
                for column in area.start().column()..=area.end().column() {
                    let cell = CellRef::new(row, column);
                    if !visited.insert(cell) {
                        continue;
                    }
                    let values = formulas
                        .iter()
                        .map(|formula| {
                            let value = formula.at(cell).to_string();
                            // These eight native formulas contain no text literals
                            // or quoted sheet names. An explicit self-sheet prefix
                            // and Own have the same meaning; retain foreign ones.
                            assert!(!value.contains('"') && !value.contains('\''));
                            value.replace(&format!("{sheet_name}!"), "")
                        })
                        .collect();
                    cells.entry(cell).or_default().push(values);
                }
            }
        }
    }
    for rules in cells.values_mut() {
        rules.sort();
    }
    cells
}

#[test]
fn partial_carried_formulas_match_native_reference_classes_and_saved_inverses() {
    use yggdryl::excel::Edit;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/carried_formulas_excel.json")).unwrap();
    let extended: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/carried_scalar_disjoint_excel.json")).unwrap();
    let areas: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/carried_area_excel.json")).unwrap();
    for fixture in [&fixture, &extended, &areas] {
        assert_eq!(fixture["provenance"]["excel"]["version"], "16.0");
        assert_eq!(fixture["provenance"]["cleanup_completed"], true);
        assert_eq!(fixture["provenance"]["reopened_without_repair"], true);
        for case in fixture["cases"].as_array().unwrap() {
            let id = case["id"].as_str().unwrap();
            let mut workbook = partial_carried_geometry_book(
                case["before"]["Data"].as_str().unwrap(),
                case["before"]["Other"].as_str().unwrap(),
            );
            let before = table_member_map(&workbook);
            let applied = workbook
                .apply(Edit::Paste {
                    from: (
                        case["source"].as_str().unwrap().into(),
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
            for (name, part) in [
                ("Data", "xl/worksheets/sheet1.xml"),
                ("Other", "xl/worksheets/sheet2.xml"),
            ] {
                let expected_rules = case["after"][name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|rule| {
                        (
                            rule["sqref"]
                                .as_str()
                                .unwrap()
                                .split_whitespace()
                                .map(|area| area.parse().unwrap())
                                .collect(),
                            rule["formulas"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|value| value.as_str().unwrap().to_owned())
                                .collect(),
                        )
                    })
                    .collect();
                let expected = partial_formula_meaning(expected_rules, name);
                let actual =
                    partial_formula_meaning(partial_formula_rules(&member(&workbook, part)), name);
                for cell in expected
                    .keys()
                    .chain(actual.keys())
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    assert_eq!(actual.get(&cell), expected.get(&cell), "{id} {name}!{cell}");
                }
            }
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
            let after = table_member_map(&workbook);
            let undone = workbook.apply(applied.inverse.unwrap()).unwrap();
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
            assert_eq!(table_member_map(&workbook), before, "{id} saved undo");
            workbook.apply(undone.inverse.unwrap()).unwrap();
            let package = workbook.into_package().unwrap();
            workbook.rebase(package).unwrap();
            assert_eq!(table_member_map(&workbook), after, "{id} saved redo");
        }
    }
}

#[test]
fn partial_formula_area_cut_contracts_the_original_area_after_native_edge_removal() {
    // Excel 16.0 build 20430.0, cell-cut-dependencies: moving Data!A1
    // leaves the surviving B1 inside the original area, in either mode.
    for destination in ["Data", "Other"] {
        let mut workbook = with_formulas(&[("D1", "SUM(Data!A1:B1)")]);
        for name in ["Data", "Other"] {
            let sheet = workbook.sheet_mut(name).unwrap();
            sheet.set_cell(at("A1"), 11.0).unwrap();
            sheet.set_cell(at("B1"), 99.0).unwrap();
        }
        workbook
            .paste(
                ("Data", "A1".parse().unwrap()),
                (destination, at("B1")),
                Paste::All,
                true,
            )
            .unwrap();
        assert_eq!(
            spelled(&workbook),
            pairs(&[("D1", "SUM(Data!B1:B1)")]),
            "{destination}"
        );
    }
}

#[test]
fn native_cell_cut_area_edges_and_whole_axes_keep_surviving_references() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cell_cut_area_excel.json")).unwrap();
    assert_eq!(fixture["provenance"]["excel"]["version"], "16.0");
    assert_eq!(fixture["provenance"]["cleanup_completed"], true);
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        assert_eq!(case["reopened_without_repair"], true, "{id}");
        let expression = case["formula"].as_str().unwrap().trim_start_matches('=');
        let mut workbook = with_formulas(&[("H8", expression)]);
        workbook
            .sheet_mut("Other")
            .unwrap()
            .insert_cell(
                Cell::from_scalar(at("H8"), 0.0.into(), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file(expression, at("H8"))),
            )
            .unwrap();
        for name in ["Data", "Other"] {
            let sheet = workbook.sheet_mut(name).unwrap();
            for row in 1..=5 {
                for column in ['A', 'B', 'C', 'D', 'E'] {
                    sheet
                        .set_cell(
                            at(&format!("{column}{row}")),
                            f64::from(row * 10 + u32::from(column as u8 - b'A') + 1),
                        )
                        .unwrap();
                }
            }
        }
        let target = match case["target"].as_str().unwrap() {
            "10:10" => "A10",
            "J:J" => "J1",
            value => value,
        };
        workbook
            .paste(
                (
                    case["source"].as_str().unwrap(),
                    case["block"].as_str().unwrap().parse().unwrap(),
                ),
                (case["destination"].as_str().unwrap(), at(target)),
                Paste::All,
                true,
            )
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        for name in ["Data", "Other"] {
            let cell = workbook.sheet(name).unwrap().cell(at("H8")).unwrap();
            let actual = cell.formula().unwrap().at(at("H8")).to_string();
            assert_eq!(actual, case["after"][name].as_str().unwrap(), "{id} {name}");
        }
    }
}

#[test]
fn partial_formula_area_keeps_reversed_corner_spelling_after_contraction() {
    let mut workbook = with_formulas(&[("H8", "SUM(Data!C3:A1)")]);
    workbook
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("A1"), 11.0)
        .unwrap();
    workbook
        .sheet_mut("Data")
        .unwrap()
        .set_cell(at("B1"), 12.0)
        .unwrap();
    workbook
        .paste(
            ("Data", "A1:A3".parse().unwrap()),
            ("Other", at("J10")),
            Paste::All,
            true,
        )
        .unwrap();
    assert_eq!(spelled(&workbook), pairs(&[("H8", "SUM(Data!C3:B1)")]));
}

#[test]
fn partial_carried_relative_area_refusal_is_located_and_atomic() {
    use yggdryl::excel::Edit;
    for conditional in [true, false] {
        for formula in ["SUM(C3:D4)>0", "SUM($C3:$D4)>0", "SUM(3:4)>0", "SUM(C:D)>0"] {
            for (range, destination) in [("B2:D5", "Other"), ("B2:D5", "Data"), ("C3:D4", "Data")] {
                let fragment = if conditional {
                    format!(
                        r#"<conditionalFormatting sqref="{range}"><cfRule type="expression" priority="1"><formula>{formula}</formula></cfRule></conditionalFormatting>"#
                    )
                } else {
                    format!(
                        r#"<dataValidations count="1"><dataValidation type="custom" sqref="{range}"><formula1>{formula}</formula1></dataValidation></dataValidations>"#
                    )
                };
                let mut workbook = partial_carried_geometry_book(&fragment, "");
                let before = table_member_map(&workbook);
                let revisions =
                    ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
                let dirty = workbook.is_dirty();
                let error = workbook
                    .apply(Edit::Paste {
                        from: ("Data".into(), "C3:D4".parse().unwrap()),
                        to: (destination.into(), at("D4")),
                        what: Paste::All,
                        cut: true,
                    })
                    .unwrap_err();
                assert!(
                    matches!(error, Error::Unsupported { operation, ref filesystem }
                    if operation == "moving host-dependent formula areas from a split or same-sheet carried owner"
                    && filesystem.as_str() == format!("xl/worksheets/sheet1.xml#sqref={range}")),
                    "{conditional} {formula} {range} -> {destination}: {error}"
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
}

#[test]
fn partial_carried_native_relative_areas_refuse_without_publishing_an_approximation() {
    use yggdryl::excel::Edit;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/carried_area_unmodeled_excel.json")).unwrap();
    assert_eq!(fixture["provenance"]["cleanup_completed"], true);
    assert_eq!(fixture["provenance"]["reopened_without_repair"], true);
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 16);
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let mut workbook = partial_carried_geometry_book(
            case["before"]["Data"].as_str().unwrap(),
            case["before"]["Other"].as_str().unwrap(),
        );
        let before = table_member_map(&workbook);
        let revisions = ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision());
        let dirty = workbook.is_dirty();
        let error = workbook
            .apply(Edit::Paste {
                from: (
                    case["source"].as_str().unwrap().into(),
                    case["block"].as_str().unwrap().parse().unwrap(),
                ),
                to: (
                    case["destination"].as_str().unwrap().into(),
                    case["target"].as_str().unwrap().parse().unwrap(),
                ),
                what: Paste::All,
                cut: true,
            })
            .unwrap_err();
        assert!(
            matches!(error, Error::Unsupported { operation, ref filesystem }
            if operation == "moving host-dependent formula areas from a split or same-sheet carried owner"
                && filesystem == "xl/worksheets/sheet1.xml#sqref=B2:D5"),
            "{id}: {error}"
        );
        assert_eq!(table_member_map(&workbook), before, "{id}");
        assert_eq!(
            ["Data", "Other"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions,
            "{id}"
        );
        assert_eq!(workbook.is_dirty(), dirty, "{id}");
    }
}
