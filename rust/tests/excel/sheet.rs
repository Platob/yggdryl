//! `rust/src/excel/sheet.rs`: `Sheet` and `Row` - the name rules, random access by reference, row and column walks, row moves, and the record `Serie` a sheet lays out and is built from.

use yggdryl::excel::{
    Cell, CellKind, CellRange, CellRef, DateSystem, MAX_CELL_TEXT, MAX_COLUMNS, MAX_ROWS,
    MAX_SHEET_NAME, NumberFormat, Sheet, SheetState, Workbook, validate_sheet_name,
};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

fn range(text: &str) -> CellRange {
    text.parse().unwrap()
}

fn references<'a>(cells: impl Iterator<Item = &'a Cell>) -> Vec<String> {
    cells.map(|cell| cell.reference().to_string()).collect()
}

/// Each column of a record serie as (name, datatype, nullable).
fn columns(serie: &Serie) -> Vec<(String, String, bool)> {
    serie
        .field()
        .unwrap()
        .fields()
        .iter()
        .map(|field| {
            (
                field.name().to_owned(),
                field.dtype().to_string(),
                field.is_nullable(),
            )
        })
        .collect()
}

fn column(name: &str, dtype: &str, nullable: bool) -> (String, String, bool) {
    (name.to_owned(), dtype.to_owned(), nullable)
}

/// Every row of a serie as its JSON text.
fn json_rows(serie: &Serie) -> Vec<String> {
    (0..serie.len())
        .map(|index| serie.scalar(index).unwrap().into_json().unwrap())
        .collect()
}

fn naive(count: i64, unit: TimeUnit) -> Scalar {
    Scalar::datetime64(count, unit, Timezone::NAIVE).unwrap()
}

fn record(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(fields).unwrap()).required_field("row")
}

/// A sheet named `name` holding each (reference, value) pair.
fn sheet_of(name: &str, cells: &[(&str, Scalar)]) -> Sheet {
    let mut sheet = Sheet::new(name).unwrap();
    for (reference, value) in cells {
        sheet.set_cell(at(reference), value.clone()).unwrap();
    }
    sheet
}

/// Two records of an `id` and a nullable `symbol`, the second's symbol null.
fn id_symbol() -> Serie {
    Serie::from_scalars(
        record([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ]),
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        ],
    )
    .unwrap()
}

#[test]
fn a_new_sheet_is_empty_visible_and_1900_based_and_its_state_spells_as_the_file_does() {
    let sheet = Sheet::new("Trades").unwrap();
    assert_eq!(sheet.name(), "Trades");
    assert_eq!(sheet.state(), SheetState::Visible);
    assert_eq!(sheet.date_system(), DateSystem::Year1900);
    assert_eq!(sheet.len(), 0);
    assert!(sheet.is_empty());
    assert_eq!(sheet.dimension(), None);
    assert_eq!(sheet.rows().count(), 0);
    assert_eq!(sheet.cells().count(), 0);

    let mut sheet = sheet
        .with_state(SheetState::Hidden)
        .with_date_system(DateSystem::Year1904);
    assert_eq!(sheet.state(), SheetState::Hidden);
    assert_eq!(sheet.date_system(), DateSystem::Year1904);
    sheet.set_state(SheetState::VeryHidden);
    assert_eq!(sheet.state(), SheetState::VeryHidden);

    for (state, spelling) in [
        (SheetState::Visible, "visible"),
        (SheetState::Hidden, "hidden"),
        (SheetState::VeryHidden, "veryHidden"),
    ] {
        assert_eq!(state.as_str(), spelling);
        assert_eq!(SheetState::from_attribute(Some(spelling)).unwrap(), state);
    }
    assert_eq!(
        SheetState::from_attribute(None).unwrap(),
        SheetState::Visible
    );
    assert_eq!(
        SheetState::from_attribute(Some(" hidden ")).unwrap(),
        SheetState::Hidden
    );
    assert_eq!(
        SheetState::from_attribute(Some("VeryHidden"))
            .unwrap_err()
            .to_string(),
        "invalid sheet state expression at byte 0: expected visible, hidden or veryHidden for a \
         sheet's state, got \"VeryHidden\""
    );
}

#[test]
fn a_sheet_name_is_refused_by_every_rule_excel_refuses_one_by() {
    let long = "a".repeat(MAX_SHEET_NAME + 1);
    let cases: Vec<(&str, String)> = vec![
        ("", "expected a sheet name, got the empty text".to_owned()),
        (
            &long,
            format!("expected a sheet name of at most 31 characters, got 32 in \"{long}\""),
        ),
        ("a\\b", r#"expected a sheet name without any of \ / ? * [ ] :, got '\\' in "a\\b""#.to_owned()),
        ("a/b", r#"expected a sheet name without any of \ / ? * [ ] :, got '/' in "a/b""#.to_owned()),
        ("a?b", r#"expected a sheet name without any of \ / ? * [ ] :, got '?' in "a?b""#.to_owned()),
        ("a*b", r#"expected a sheet name without any of \ / ? * [ ] :, got '*' in "a*b""#.to_owned()),
        ("a[b", r#"expected a sheet name without any of \ / ? * [ ] :, got '[' in "a[b""#.to_owned()),
        ("a]b", r#"expected a sheet name without any of \ / ? * [ ] :, got ']' in "a]b""#.to_owned()),
        ("a:b", r#"expected a sheet name without any of \ / ? * [ ] :, got ':' in "a:b""#.to_owned()),
        (
            "'Trades",
            r#"expected a sheet name that neither opens nor closes with an apostrophe, got "'Trades""#
                .to_owned(),
        ),
        (
            "Trades'",
            r#"expected a sheet name that neither opens nor closes with an apostrophe, got "Trades'""#
                .to_owned(),
        ),
        ("History", "expected a sheet name other than the reserved `History`".to_owned()),
        ("hISTORY", "expected a sheet name other than the reserved `History`".to_owned()),
    ];
    for (name, reason) in cases {
        let expected = format!("invalid record value at $.sheet: {reason}");
        assert_eq!(
            validate_sheet_name(name).unwrap_err().to_string(),
            expected,
            "{name:?}"
        );
        assert_eq!(
            Sheet::new(name).unwrap_err().to_string(),
            expected,
            "{name:?}"
        );
    }
}

#[test]
fn a_name_counts_characters_not_bytes_and_a_refused_rename_keeps_the_old_name() {
    let accented = "é".repeat(MAX_SHEET_NAME);
    assert_eq!(accented.len(), 62);
    for name in [accented.as_str(), "it's", "History 2", "Q1 (draft)"] {
        assert_eq!(Sheet::new(name).unwrap().name(), name);
    }

    let mut sheet = Sheet::new("Trades").unwrap();
    sheet.set_name("Fills").unwrap();
    assert_eq!(sheet.name(), "Fills");
    assert_eq!(
        sheet.set_name("Fills/2024").unwrap_err().to_string(),
        r#"invalid record value at $.sheet: expected a sheet name without any of \ / ? * [ ] :, got '/' in "Fills/2024""#
    );
    assert_eq!(sheet.name(), "Fills");
}

#[test]
fn set_cell_spells_each_value_as_the_kind_and_format_that_reads_it_and_keeps_the_value() {
    let noon_millis = 19_723 * 86_400_000 + 43_200_000;
    let cases = [
        (
            "A1",
            Scalar::from("AAPL"),
            CellKind::SharedString,
            NumberFormat::General,
        ),
        (
            "B1",
            Scalar::from(7_i64),
            CellKind::Number,
            NumberFormat::General,
        ),
        (
            "C1",
            Scalar::from(187.23),
            CellKind::Number,
            NumberFormat::General,
        ),
        (
            "D1",
            Scalar::from(true),
            CellKind::Boolean,
            NumberFormat::General,
        ),
        (
            "E1",
            Scalar::date32(19_723),
            CellKind::Number,
            NumberFormat::Date,
        ),
        (
            "F1",
            naive(noon_millis, TimeUnit::Millisecond),
            CellKind::Number,
            NumberFormat::DateTimeFraction,
        ),
        (
            "G1",
            naive(noon_millis / 1_000, TimeUnit::Second),
            CellKind::Number,
            NumberFormat::DateTime,
        ),
        (
            "H1",
            Scalar::time64(43_200_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            CellKind::Number,
            NumberFormat::Time,
        ),
        (
            "I1",
            Scalar::duration64(5_400_000, TimeUnit::Millisecond).unwrap(),
            CellKind::Number,
            NumberFormat::Duration,
        ),
        (
            "J1",
            Scalar::datetime64(noon_millis, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
            CellKind::SharedString,
            NumberFormat::General,
        ),
    ];
    let mut sheet = Sheet::new("Kinds").unwrap();
    for (reference, value, _, _) in &cases {
        assert_eq!(
            sheet.set_cell(at(reference), value.clone()).unwrap(),
            None,
            "{reference}"
        );
    }
    for (reference, value, kind, format) in cases {
        let cell = sheet.cell(at(reference)).unwrap();
        assert_eq!(cell.reference(), at(reference));
        assert_eq!(cell.kind(), kind, "{reference}");
        assert_eq!(cell.format(), format, "{reference}");
        assert_eq!(cell.value(), &value, "{reference}");
        assert_eq!(sheet.scalar(at(reference)), value, "{reference}");
    }
    assert_eq!(sheet.len(), 1);
    assert_eq!(sheet.scalar(at("K1")), Scalar::Null);
    assert_eq!(sheet.scalar(at("A2")), Scalar::Null);
    assert!(sheet.cell(at("K1")).is_none());
}

#[test]
fn a_float_that_is_no_number_is_the_num_error_and_a_null_is_a_cell_holding_nothing() {
    let mut sheet = Sheet::new("Edges").unwrap();
    sheet.set_cell(at("A1"), f64::NAN).unwrap();
    sheet.set_cell(at("B1"), f64::INFINITY).unwrap();
    for reference in ["A1", "B1"] {
        let cell = sheet.cell(at(reference)).unwrap();
        assert_eq!(cell.kind(), CellKind::Error, "{reference}");
        assert_eq!(cell.error(), Some("#NUM!"), "{reference}");
        assert_eq!(cell.text(), "#NUM!", "{reference}");
        assert!(cell.is_null(), "{reference}");
        assert_eq!(sheet.scalar(at(reference)), Scalar::Null);
    }

    // A null is put, not removed: the cell stands, holding no value.
    sheet.set_cell(at("C1"), 1.5).unwrap();
    let replaced = sheet.set_cell(at("C1"), Scalar::Null).unwrap().unwrap();
    assert_eq!(replaced.value(), &Scalar::from(1.5));
    let empty = sheet.cell(at("C1")).unwrap();
    assert!(empty.is_null());
    assert_eq!(empty.kind(), CellKind::Number);
    assert_eq!(empty.text(), "");
    assert_eq!(sheet.scalar(at("C1")), Scalar::Null);
    assert_eq!(sheet.dimension(), Some(range("A1:C1")));
}

#[test]
fn setting_a_cell_answers_the_cell_it_replaced() {
    let mut sheet = Sheet::new("Trades").unwrap();
    assert_eq!(sheet.set_cell(at("B2"), "AAPL").unwrap(), None);
    let replaced = sheet.set_cell(at("B2"), 187.23).unwrap().unwrap();
    assert_eq!(replaced.reference(), at("B2"));
    assert_eq!(replaced.kind(), CellKind::SharedString);
    assert_eq!(replaced.value(), &Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(at("B2")), Scalar::from(187.23));
    assert_eq!(sheet.cells().count(), 1);
}

#[test]
fn a_reference_off_the_grid_or_a_text_too_long_for_a_cell_is_refused_leaving_the_sheet_untouched() {
    let mut sheet = Sheet::new("Grid").unwrap();
    assert_eq!(
        sheet
            .set_cell(CellRef::new(MAX_ROWS, 0), 1.0)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected a cell within 1048576 rows and 16384 columns \
         (A1 to XFD1048576), got row 1048577 column 1"
    );
    let outside = Cell::new(
        CellRef::new(0, MAX_COLUMNS),
        CellKind::Number,
        NumberFormat::General,
        Scalar::from(1.0),
    );
    assert_eq!(
        sheet.insert_cell(outside).unwrap_err().to_string(),
        "invalid record value at $: expected a cell within 1048576 rows and 16384 columns \
         (A1 to XFD1048576), got row 1 column 16385"
    );
    assert_eq!(
        sheet
            .set_cell(at("A1"), "x".repeat(MAX_CELL_TEXT + 1).as_str())
            .unwrap_err()
            .to_string(),
        "invalid record value at A1: expected at most 32767 characters in a cell, got 32768"
    );
    assert!(sheet.is_empty());

    // The last cell of the grid and a text of exactly the limit both fit.
    sheet
        .set_cell(CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1), 1.0)
        .unwrap();
    sheet
        .set_cell(at("A1"), "x".repeat(MAX_CELL_TEXT).as_str())
        .unwrap();
    assert_eq!(sheet.dimension(), Some(range("A1:XFD1048576")));
}

#[test]
fn a_temporal_is_set_only_where_the_sheets_date_system_spells_it() {
    // 1904-01-01 is day -24107 of the Unix epoch; the day before is not in
    // the 1904 system, and is in the 1900 one.
    let mut sheet = Sheet::new("Mac")
        .unwrap()
        .with_date_system(DateSystem::Year1904);
    sheet.set_cell(at("A1"), Scalar::date32(-24_107)).unwrap();
    let refusal = sheet
        .set_cell(at("A2"), Scalar::date32(-24_108))
        .unwrap_err()
        .to_string();
    assert_eq!(
        refusal,
        "invalid record value at A2: expected an instant the 1904 date system spells, \
         from its first day to 9999-12-31, got 1903-12-31T00:00:00"
    );
    assert!(sheet.cell(at("A2")).is_none());

    let mut sheet = Sheet::new("Windows").unwrap();
    sheet.set_cell(at("A2"), Scalar::date32(-24_108)).unwrap();
    assert_eq!(sheet.scalar(at("A2")), Scalar::date32(-24_108));
}

#[test]
fn insert_cell_places_a_prebuilt_cell_and_cell_mut_edits_one_in_place() {
    let mut sheet = Sheet::new("Sums").unwrap();
    let sum = Cell::new(
        at("C1"),
        CellKind::Number,
        NumberFormat::General,
        Scalar::from(3.0),
    )
    .with_formula("A1+B1");
    assert_eq!(sheet.insert_cell(sum.clone()).unwrap(), None);
    assert_eq!(sheet.cell(at("C1")), Some(&sum));
    assert_eq!(sheet.cell(at("C1")).unwrap().formula(), Some("A1+B1"));
    assert_eq!(sheet.scalar(at("C1")), Scalar::from(3.0));

    let replacement = Cell::new(
        at("C1"),
        CellKind::Number,
        NumberFormat::General,
        Scalar::from(4.0),
    );
    assert_eq!(sheet.insert_cell(replacement).unwrap(), Some(sum));

    let cell = sheet.cell_mut(at("C1")).unwrap();
    *cell = cell.clone().with_error("#N/A");
    let cell = sheet.cell(at("C1")).unwrap();
    assert_eq!(cell.error(), Some("#N/A"));
    assert_eq!(cell.kind(), CellKind::Error);
    assert_eq!(sheet.scalar(at("C1")), Scalar::Null);
    assert!(sheet.cell_mut(at("D1")).is_none());
}

#[test]
fn remove_cell_takes_the_cell_out_and_drops_a_row_it_leaves_empty() {
    let mut sheet = sheet_of(
        "Trades",
        &[
            ("A1", Scalar::from("a")),
            ("B1", Scalar::from("b")),
            ("C3", Scalar::from("c")),
        ],
    );
    assert_eq!(sheet.len(), 2);
    assert_eq!(
        sheet.remove_cell(at("B1")).unwrap().value(),
        &Scalar::from("b")
    );
    assert_eq!(sheet.len(), 2);
    assert_eq!(sheet.remove_cell(at("B1")), None);
    assert_eq!(sheet.remove_cell(at("Z9")), None);
    assert_eq!(sheet.remove_cell(at("A1")).unwrap().reference(), at("A1"));
    assert_eq!(sheet.len(), 1);
    assert!(sheet.row(0).is_none());
    assert_eq!(sheet.dimension(), Some(range("C3")));
    sheet.remove_cell(at("C3")).unwrap();
    assert!(sheet.is_empty());
    assert_eq!(sheet.dimension(), None);
}

#[test]
fn the_dimension_is_the_rectangle_bounding_every_cell() {
    let sheet = sheet_of(
        "Span",
        &[
            ("C2", Scalar::from(1.0)),
            ("A5", Scalar::from(2.0)),
            ("B7", Scalar::from(3.0)),
        ],
    );
    assert_eq!(sheet.dimension(), Some(range("A2:C7")));
    assert_eq!(sheet.dimension().unwrap().to_string(), "A2:C7");
    assert_eq!(sheet.len(), 3);

    let single = sheet_of("One", &[("D4", Scalar::from(true))]);
    assert_eq!(single.dimension(), Some(range("D4")));
}

#[test]
fn rows_cells_columns_and_ranges_walk_in_row_then_column_order() {
    let mut sheet = Sheet::new("Walk").unwrap();
    for reference in ["C5", "A1", "B3", "A5", "B1", "C1"] {
        sheet.set_cell(at(reference), reference).unwrap();
    }
    assert_eq!(
        sheet.rows().map(|row| row.index()).collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    assert_eq!(
        references(sheet.cells()),
        vec!["A1", "B1", "C1", "B3", "A5", "C5"]
    );
    assert_eq!(references(sheet.column(1)), vec!["B1", "B3"]);
    assert_eq!(references(sheet.column(2)), vec!["C1", "C5"]);
    assert_eq!(sheet.column(3).count(), 0);

    let first = sheet.row(0).unwrap();
    assert_eq!(first.index(), 0);
    assert_eq!(first.len(), 3);
    assert!(!first.is_empty());
    assert_eq!(references(first.cells()), vec!["A1", "B1", "C1"]);
    assert_eq!(first.cell(1).unwrap().value(), &Scalar::from("B1"));
    assert!(first.cell(3).is_none());
    assert!(sheet.row(1).is_none());

    assert_eq!(
        references(sheet.cells_in(range("B1:C3"))),
        vec!["B1", "C1", "B3"]
    );
    assert_eq!(references(sheet.cells_in(range("A:A"))), vec!["A1", "A5"]);
    assert_eq!(references(sheet.cells_in(range("5:5"))), vec!["A5", "C5"]);
    assert_eq!(references(sheet.cells_in(range("A3:B"))), vec!["B3", "A5"]);
    assert_eq!(sheet.cells_in(range("D1:F9")).count(), 0);
}

#[test]
fn a_slice_is_a_sheet_of_the_cells_inside_the_range_keeping_their_references() {
    let sheet = sheet_of(
        "Window",
        &[
            ("A1", Scalar::from(1.0)),
            ("B2", Scalar::from(2.0)),
            ("C3", Scalar::from(3.0)),
            ("D4", Scalar::from(4.0)),
        ],
    )
    .with_state(SheetState::Hidden)
    .with_date_system(DateSystem::Year1904);
    let sliced = sheet.slice(range("B2:C3"));
    assert_eq!(references(sliced.cells()), vec!["B2", "C3"]);
    assert_eq!(sliced.scalar(at("C3")), Scalar::from(3.0));
    assert_eq!(sliced.scalar(at("A1")), Scalar::Null);
    assert_eq!(sliced.name(), "Window");
    assert_eq!(sliced.state(), SheetState::Hidden);
    assert_eq!(sliced.date_system(), DateSystem::Year1904);
    assert_eq!(sliced.dimension(), Some(range("B2:C3")));
    assert_eq!(sheet.len(), 4);
    assert!(sheet.slice(range("E5:F6")).is_empty());
}

#[test]
fn inserting_rows_moves_every_row_from_the_point_down_and_refuses_pushing_one_off_the_grid() {
    let mut sheet = sheet_of(
        "Moves",
        &[
            ("A1", Scalar::from("a")),
            ("A2", Scalar::from("b")),
            ("B3", Scalar::from("c")),
        ],
    );
    sheet.insert_rows(1, 2).unwrap();
    assert_eq!(references(sheet.cells()), vec!["A1", "A4", "B5"]);
    assert_eq!(sheet.scalar(at("A4")), Scalar::from("b"));
    assert_eq!(sheet.scalar(at("B5")), Scalar::from("c"));
    assert_eq!(sheet.scalar(at("A2")), Scalar::Null);
    assert_eq!(sheet.row(3).unwrap().index(), 3);
    assert_eq!(sheet.cell(at("B5")).unwrap().row(), 4);

    let unchanged = sheet.clone();
    sheet.insert_rows(0, 0).unwrap();
    sheet.insert_rows(9, 100).unwrap();
    assert_eq!(sheet, unchanged);

    let mut full = sheet_of(
        "Full",
        &[("A1", Scalar::from(1.0)), ("A1048576", Scalar::from(2.0))],
    );
    let before = full.clone();
    assert_eq!(
        full.insert_rows(0, 1).unwrap_err().to_string(),
        "invalid record value at $: expected the moved rows to stay within 1048576 rows, got row \
         1048576 moving by 1"
    );
    assert_eq!(full, before);
    assert_eq!(
        full.insert_rows(5, u32::MAX).unwrap_err().to_string(),
        format!(
            "invalid record value at $: expected the moved rows to stay within 1048576 rows, got \
             row 1048576 moving by {}",
            u32::MAX
        )
    );
    assert_eq!(full, before);
}

#[test]
fn removing_rows_drops_them_and_moves_every_row_below_up() {
    let mut sheet = sheet_of(
        "Moves",
        &[
            ("A1", Scalar::from("a")),
            ("A2", Scalar::from("b")),
            ("A3", Scalar::from("c")),
            ("B5", Scalar::from("d")),
        ],
    );
    sheet.remove_rows(1..3);
    assert_eq!(references(sheet.cells()), vec!["A1", "B3"]);
    assert_eq!(sheet.scalar(at("B3")), Scalar::from("d"));
    assert_eq!(sheet.cell(at("B3")).unwrap().reference(), at("B3"));
    assert_eq!(sheet.row(2).unwrap().index(), 2);
    assert_eq!(sheet.dimension(), Some(range("A1:B3")));

    let unchanged = sheet.clone();
    sheet.remove_rows(1..1);
    sheet.remove_rows(10..20);
    assert_eq!(sheet, unchanged);
}

#[test]
fn a_sheet_from_a_record_serie_writes_the_header_then_one_row_per_record_leaving_nulls_empty() {
    let noon = 19_723 * 86_400_000 + 43_200_000;
    let field = record([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::Boolean.required_field("live"),
        DataType::Date32.nullable_field("traded"),
        DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)
            .unwrap()
            .required_field("at"),
    ]);
    let serie = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from("AAPL"),
                Scalar::from(true),
                Scalar::date32(19_723),
                naive(noon, TimeUnit::Millisecond),
            ]),
            Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::Null,
                Scalar::from(false),
                Scalar::Null,
                naive(noon + 1_000, TimeUnit::Millisecond),
            ]),
        ],
    )
    .unwrap();
    let sheet = Sheet::from_serie("Trades", &serie, true).unwrap();
    assert_eq!(sheet.name(), "Trades");
    assert_eq!(sheet.dimension(), Some(range("A1:E3")));
    let header: Vec<String> = sheet
        .row(0)
        .unwrap()
        .cells()
        .map(|cell| cell.text().into_owned())
        .collect();
    assert_eq!(header, vec!["id", "symbol", "live", "traded", "at"]);
    assert_eq!(sheet.scalar(at("A2")), Scalar::from(1_i64));
    assert_eq!(sheet.scalar(at("B2")), Scalar::from("AAPL"));
    assert_eq!(sheet.cell(at("C2")).unwrap().kind(), CellKind::Boolean);
    assert_eq!(sheet.cell(at("D2")).unwrap().format(), NumberFormat::Date);
    assert_eq!(sheet.scalar(at("D2")), Scalar::date32(19_723));
    assert_eq!(
        sheet.cell(at("E2")).unwrap().format(),
        NumberFormat::DateTimeFraction
    );
    assert_eq!(sheet.scalar(at("C3")), Scalar::from(false));
    assert!(sheet.cell(at("B3")).is_none());
    assert!(sheet.cell(at("D3")).is_none());

    // Read back under its own field, the sheet is the serie's rows.
    let back = sheet
        .clone()
        .into_serie(Some(&field), true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(json_rows(&back), json_rows(&serie));

    // Without the header the first record is the first row.
    let bare = Sheet::from_serie("Bare", &serie, false).unwrap();
    assert_eq!(bare.scalar(at("A1")), Scalar::from(1_i64));
    assert_eq!(bare.dimension(), Some(range("A1:E2")));

    assert_eq!(
        Sheet::from_serie("a/b", &serie, true)
            .unwrap_err()
            .to_string(),
        r#"invalid record value at $.sheet: expected a sheet name without any of \ / ? * [ ] :, got '/' in "a/b""#
    );
}

#[test]
fn a_serie_that_is_not_a_record_is_the_one_column_named_as_it_is_and_a_run_is_refused() {
    let prices = Serie::from_scalars(
        DataType::Float64.nullable_field("price"),
        [Scalar::from(1.5), Scalar::Null, Scalar::from(2.5)],
    )
    .unwrap();
    let sheet = Sheet::from_serie("Prices", &prices, true).unwrap();
    assert_eq!(references(sheet.cells()), vec!["A1", "A2", "A4"]);
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("price"));
    assert_eq!(sheet.scalar(at("A4")), Scalar::from(2.5));

    let run = Serie::new(vec![Scalar::from(1.5)]);
    assert_eq!(
        Sheet::from_serie("Run", &run, true)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: a schema-free run declares no field"
    );
}

#[test]
fn write_serie_lays_rows_out_from_an_anchor_with_or_without_the_header_replacing_cells_there() {
    let mut sheet = sheet_of(
        "Book",
        &[
            ("A1", Scalar::from("keep")),
            ("B3", Scalar::from("old")),
            ("C4", Scalar::from("stale")),
        ],
    );
    sheet.write_serie(at("B3"), &id_symbol(), true).unwrap();
    assert_eq!(
        references(sheet.cells()),
        vec!["A1", "B3", "C3", "B4", "C4", "B5"]
    );
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("keep"));
    assert_eq!(sheet.scalar(at("B3")), Scalar::from("id"));
    assert_eq!(sheet.scalar(at("C3")), Scalar::from("symbol"));
    assert_eq!(sheet.scalar(at("B4")), Scalar::from(1_i64));
    assert_eq!(sheet.scalar(at("C4")), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(at("B5")), Scalar::from(2_i64));

    sheet.write_serie(at("E7"), &id_symbol(), false).unwrap();
    assert_eq!(
        references(sheet.cells_in(range("E7:F8"))),
        vec!["E7", "F7", "E8"]
    );
    assert_eq!(sheet.scalar(at("E7")), Scalar::from(1_i64));
    assert_eq!(sheet.scalar(at("F7")), Scalar::from("AAPL"));
    assert_eq!(sheet.dimension(), Some(range("A1:F8")));
}

#[test]
fn write_serie_refuses_rows_or_columns_that_would_leave_the_grid_before_writing_any() {
    let mut sheet = Sheet::new("Edge").unwrap();
    assert_eq!(
        sheet
            .write_serie(CellRef::new(MAX_ROWS - 1, 0), &id_symbol(), true)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected 3 rows and 2 columns from A1048576 to fit 1048576 \
         rows by 16384 columns"
    );
    assert_eq!(
        sheet
            .write_serie(CellRef::new(0, MAX_COLUMNS - 1), &id_symbol(), false)
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected 2 rows and 2 columns from XFD1 to fit 1048576 rows \
         by 16384 columns"
    );
    assert!(sheet.is_empty());

    // Two records and no header end exactly on the last row.
    sheet
        .write_serie(
            CellRef::new(MAX_ROWS - 2, MAX_COLUMNS - 2),
            &id_symbol(),
            false,
        )
        .unwrap();
    assert_eq!(sheet.dimension(), Some(range("XFC1048575:XFD1048576")));
}

#[test]
fn extend_from_serie_appends_below_the_last_row_from_the_first_column_without_a_header() {
    let mut sheet = Sheet::from_serie("Book", &id_symbol(), true).unwrap();
    sheet.extend_from_serie(&id_symbol()).unwrap();
    assert_eq!(sheet.dimension(), Some(range("A1:B5")));
    assert_eq!(sheet.scalar(at("A4")), Scalar::from(1_i64));
    assert_eq!(sheet.scalar(at("B4")), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(at("A5")), Scalar::from(2_i64));
    assert!(sheet.cell(at("B5")).is_none());

    let mut offset = sheet_of(
        "Offset",
        &[("B2", Scalar::from("x")), ("D2", Scalar::from("y"))],
    );
    offset.extend_from_serie(&id_symbol()).unwrap();
    assert_eq!(
        references(offset.cells_in(range("A3:D4"))),
        vec!["B3", "C3", "B4"]
    );
    assert_eq!(offset.scalar(at("B3")), Scalar::from(1_i64));

    let mut empty = Sheet::new("Empty").unwrap();
    empty.extend_from_serie(&id_symbol()).unwrap();
    assert_eq!(empty.scalar(at("A1")), Scalar::from(1_i64));
    assert_eq!(empty.dimension(), Some(range("A1:B2")));
}

#[test]
fn into_serie_types_each_column_by_its_first_present_value_nullable_where_a_row_lacks_the_cell() {
    let sheet = sheet_of(
        "Trades",
        &[
            ("A1", Scalar::from("symbol")),
            ("B1", Scalar::from("price")),
            ("C1", Scalar::from("live")),
            ("D1", Scalar::from("day")),
            ("A2", Scalar::from("AAPL")),
            ("B2", Scalar::from(187.5)),
            ("C2", Scalar::from(true)),
            ("D2", Scalar::date32(19_724)),
            ("A3", Scalar::from("MSFT")),
            ("B3", Scalar::from(410.25)),
            ("C3", Scalar::Null),
            ("D3", Scalar::date32(19_725)),
            ("A5", Scalar::from("IBM")),
            ("B5", Scalar::from(-1.0)),
            ("D5", Scalar::date32(19_726)),
        ],
    );
    let rows = sheet
        .into_serie(None, true, ArrowCastOptions::default())
        .unwrap();
    let root = rows.field().unwrap();
    assert_eq!(root.name(), "row");
    assert!(!root.is_nullable());
    assert_eq!(
        columns(&rows),
        vec![
            column("symbol", "utf8", false),
            column("price", "float64", false),
            column("live", "boolean", true),
            column("day", "date32", false),
        ]
    );
    // Row 4 holds no cell and is no record.
    assert_eq!(
        json_rows(&rows),
        vec![
            r#"["AAPL",187.5,true,"2024-01-02"]"#,
            r#"["MSFT",410.25,null,"2024-01-03"]"#,
            r#"["IBM",-1.0,null,"2024-01-04"]"#,
        ]
    );
}

#[test]
fn into_serie_refuses_a_later_value_of_another_datatype_naming_the_sheet_and_the_cell() {
    let sheet = sheet_of(
        "Sheet1",
        &[
            ("A1", Scalar::from("id")),
            ("B1", Scalar::from("px")),
            ("C1", Scalar::from("when")),
            ("A2", Scalar::from(1.0)),
            ("B2", Scalar::from(2.0)),
            ("C2", Scalar::date32(19_723)),
            ("A3", Scalar::from(2.0)),
            ("B3", Scalar::from(3.0)),
            ("C3", Scalar::from("later")),
        ],
    );
    assert_eq!(
        sheet
            .into_serie(None, true, ArrowCastOptions::default())
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!C3: expected date32 like the column's first value, got \
         utf8; declare a field to read the column as one datatype"
    );
}

#[test]
fn into_serie_names_an_unheaded_column_by_its_letter_and_drops_one_with_no_header_and_no_value() {
    let sheet = sheet_of(
        "Gaps",
        &[
            ("A1", Scalar::from("id")),
            ("B1", Scalar::from("")),
            ("C1", Scalar::from("note")),
            ("A2", Scalar::from(1.0)),
            ("D2", Scalar::from("x")),
            ("A3", Scalar::from(2.0)),
        ],
    );
    let rows = sheet
        .into_serie(None, true, ArrowCastOptions::default())
        .unwrap();
    // B has an empty header over no value: no column. C is named over no
    // value: a null column. D has no header: named by its letter.
    assert_eq!(
        columns(&rows),
        vec![
            column("id", "float64", false),
            column("note", "null", true),
            column("D", "utf8", true),
        ]
    );
    assert_eq!(
        json_rows(&rows),
        vec![r#"[1.0,null,"x"]"#, "[2.0,null,null]"]
    );
}

#[test]
fn into_serie_names_a_column_by_the_text_its_header_cell_displays_and_refuses_a_name_used_twice() {
    let sheet = sheet_of(
        "Sheet1",
        &[
            ("A1", Scalar::from(2024.0)),
            ("B1", Scalar::from(true)),
            ("A2", Scalar::from(1.0)),
            ("B2", Scalar::from("x")),
        ],
    );
    let rows = sheet
        .into_serie(None, true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(
        columns(&rows),
        vec![
            column("2024", "float64", false),
            column("true", "utf8", false)
        ]
    );

    let twice = sheet_of(
        "Sheet1",
        &[
            ("A1", Scalar::from("x")),
            ("B1", Scalar::from("x")),
            ("A2", Scalar::from(1.0)),
            ("B2", Scalar::from(2.0)),
        ],
    );
    assert_eq!(
        twice
            .into_serie(None, true, ArrowCastOptions::default())
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!A1: the header names no valid columns: invalid StructType \
         datatype: duplicate field name \"x\""
    );
}

#[test]
fn into_serie_without_a_header_names_every_column_by_its_letters_and_reads_every_row() {
    let sheet = sheet_of(
        "Bare",
        &[
            ("B2", Scalar::from("a")),
            ("C2", Scalar::from(1.0)),
            ("B4", Scalar::from("b")),
        ],
    );
    let rows = sheet
        .into_serie(None, false, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(
        columns(&rows),
        vec![column("B", "utf8", false), column("C", "float64", true)]
    );
    assert_eq!(json_rows(&rows), vec![r#"["a",1.0]"#, r#"["b",null]"#]);
}

#[test]
fn into_serie_of_an_empty_sheet_is_no_record_under_an_empty_root_or_the_declared_field() {
    let rows = Sheet::new("Empty")
        .unwrap()
        .into_serie(None, true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(rows.len(), 0);
    let root = rows.field().unwrap();
    assert_eq!(root.name(), "row");
    assert!(!root.is_nullable());
    assert!(root.fields().is_empty());

    let declared = record([DataType::Int64.required_field("id")]);
    let rows = Sheet::new("Empty")
        .unwrap()
        .into_serie(Some(&declared), true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(rows.len(), 0);
    assert_eq!(rows.field(), Some(&declared));
}

/// A sheet whose header names `symbol`, `price`, `qty` and `day`, its
/// second record holding text under `price` and an error under `qty`.
fn declared_sheet() -> Sheet {
    let mut sheet = sheet_of(
        "Sheet1",
        &[
            ("A1", Scalar::from("symbol")),
            ("B1", Scalar::from("price")),
            ("C1", Scalar::from("qty")),
            ("D1", Scalar::from("day")),
            ("A2", Scalar::from("AAPL")),
            ("B2", Scalar::from(187.5)),
            ("C2", Scalar::from(7.0)),
            ("D2", Scalar::from("2024-01-02")),
            ("A3", Scalar::from("MSFT")),
            ("B3", Scalar::from("n/a")),
            ("D3", Scalar::date32(19_725)),
        ],
    );
    sheet
        .insert_cell(
            Cell::new(
                at("C3"),
                CellKind::Number,
                NumberFormat::General,
                Scalar::Null,
            )
            .with_error("#N/A"),
        )
        .unwrap();
    sheet
}

#[test]
fn into_serie_under_a_field_pairs_columns_by_header_name_and_reads_each_cell_through_its_contract()
{
    let field = record([
        DataType::Float64.nullable_field("price"),
        DataType::utf8().required_field("symbol"),
        DataType::Int64.nullable_field("qty"),
        DataType::Date32.required_field("day"),
        DataType::utf8().nullable_field("venue"),
    ]);
    let rows = declared_sheet()
        .into_serie(Some(&field), true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(rows.field(), Some(&field));
    // `7` written as a number lands in int64 and `2024-01-02` written as
    // text in date32; `n/a` and `#N/A` are null under `safe`; `venue` is a
    // column the sheet lacks.
    assert_eq!(
        json_rows(&rows),
        vec![
            r#"[187.5,"AAPL",7,"2024-01-02",null]"#,
            r#"[null,"MSFT",null,"2024-01-03",null]"#,
        ]
    );
}

#[test]
fn into_serie_under_a_field_refuses_what_a_column_cannot_hold_unless_it_is_safe_and_nullable() {
    let strict = ArrowCastOptions::default().with_safe(false);
    let prices = record([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.nullable_field("price"),
    ]);
    assert_eq!(
        declared_sheet()
            .into_serie(Some(&prices), true, strict)
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!B3: expected float64, got string"
    );
    let halves = sheet_of(
        "Sheet1",
        &[
            ("A1", Scalar::from("n")),
            ("A2", Scalar::from(7.0)),
            ("A3", Scalar::from(2.5)),
        ],
    );
    let counts = record([DataType::Int64.nullable_field("n")]);
    assert_eq!(
        halves
            .clone()
            .into_serie(Some(&counts), true, strict)
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!A3: expected int64, got string"
    );
    let lenient = halves
        .into_serie(Some(&counts), true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(json_rows(&lenient), vec!["[7]", "[null]"]);

    // A required column refuses whatever `safe` says, the error cell by name.
    let quantities = record([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("qty"),
    ]);
    assert_eq!(
        declared_sheet()
            .into_serie(Some(&quantities), true, ArrowCastOptions::default())
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!C3: expected a int64 value, got the error #N/A"
    );

    // A required column the sheet lacks is refused naming the column.
    let venues = record([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.nullable_field("price"),
        DataType::Int64.nullable_field("qty"),
        DataType::Date32.nullable_field("day"),
        DataType::utf8().required_field("venue"),
    ]);
    assert_eq!(
        declared_sheet()
            .into_serie(Some(&venues), true, ArrowCastOptions::default())
            .unwrap_err()
            .to_string(),
        "invalid record value at $.row.venue: non-nullable field received null"
    );
}

#[test]
fn a_sheet_read_from_a_package_types_its_numbers_as_float64_and_its_date_styled_serials_as_dates() {
    let bytes = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>day</t></is></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\" s=\"1\"><v>45293</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>2.5</v></c><c r=\"B3\" s=\"1\"><v>45294</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A2")), Scalar::from(1.0));
    assert_eq!(sheet.cell(at("B2")).unwrap().format(), NumberFormat::Date);
    let rows = sheet
        .clone()
        .into_serie(None, true, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(
        columns(&rows),
        vec![
            column("id", "float64", false),
            column("day", "date32", false)
        ]
    );
    assert_eq!(
        json_rows(&rows),
        vec![r#"[1.0,"2024-01-02"]"#, r#"[2.5,"2024-01-03"]"#]
    );
}

#[test]
fn a_sheet_round_trips_through_a_workbook_cell_for_cell() {
    let noon = 19_723 * 86_400_000 + 43_200_000;
    let mut sheet = sheet_of(
        "Trades",
        &[
            ("A1", Scalar::from("a<b & \"c\" \u{1} _x0041_")),
            ("B1", Scalar::from(187.23)),
            ("C1", Scalar::from(true)),
            ("D1", Scalar::date32(19_723)),
            ("E1", naive(noon + 123, TimeUnit::Millisecond)),
            (
                "F1",
                Scalar::duration64(5_400_000, TimeUnit::Millisecond).unwrap(),
            ),
            (
                "G1",
                Scalar::time32(43_200_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            ),
            ("H1", Scalar::from(f64::NAN)),
            ("A3", Scalar::from("second")),
            ("XFD1048576", Scalar::from(-0.5)),
        ],
    );
    sheet
        .insert_cell(
            Cell::new(
                at("I1"),
                CellKind::Number,
                NumberFormat::General,
                Scalar::from(188.23),
            )
            .with_formula("B1+1"),
        )
        .unwrap();
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet.clone()).unwrap();
    let read = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    let mut back = read.sheet("Trades").unwrap().clone();
    assert_eq!(back.dimension(), sheet.dimension());
    // The millisecond datetime keeps its value; its format is left out of
    // the pin (it reads back as `DateTime`).
    assert_eq!(back.scalar(at("E1")), sheet.scalar(at("E1")));
    back.remove_cell(at("E1")).unwrap();
    sheet.remove_cell(at("E1")).unwrap();
    for (written, read) in sheet.cells().zip(back.cells()) {
        assert_eq!(read, written, "{}", written.reference());
    }
    assert_eq!(back, sheet);
}
