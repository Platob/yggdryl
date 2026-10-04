//! `rust/src/excel/sheet.rs`: `Sheet` and `Row` - the name rules, random access by reference, row and column walks, row moves, and the record `Serie` a sheet lays out and is built from.

use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone};
use yggdryl::{
    RecordHeader,
    excel::{
        Cell, CellKind, CellRange, CellRef, DateSystem, ExcelError, Formula, MAX_CELL_TEXT,
        MAX_COLUMNS, MAX_ROWS, MAX_SHEET_NAME, NumberFormat, Sheet, SheetState, StyleId, Workbook,
        validate_sheet_name,
    },
};

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
        assert_eq!(cell.error(), Some(ExcelError::Num), "{reference}");
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
         from its first day to 9999-12-31, got 1903-12-31T00:00:00.000"
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
    .with_formula(Formula::from_file("A1+B1", at("C1")));
    assert_eq!(sheet.insert_cell(sum.clone()).unwrap(), None);
    assert_eq!(sheet.cell(at("C1")), Some(&sum));
    assert_eq!(
        sheet.cell(at("C1")).unwrap().formula(),
        Some(&Formula::from_file("A1+B1", at("C1")))
    );
    assert_eq!(sheet.scalar(at("C1")), Scalar::from(3.0));

    let replacement = Cell::new(
        at("C1"),
        CellKind::Number,
        NumberFormat::General,
        Scalar::from(4.0),
    );
    assert_eq!(sheet.insert_cell(replacement).unwrap(), Some(sum));

    let mut cell = sheet.cell_mut(at("C1")).unwrap();
    *cell = cell.clone().with_error(ExcelError::NA);
    drop(cell);
    let cell = sheet.cell(at("C1")).unwrap();
    assert_eq!(cell.error(), Some(ExcelError::NA));
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
    let sheet = Sheet::from_serie("Trades", &serie, RecordHeader::Source).unwrap();
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
        .into_serie(
            Some(&field),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(json_rows(&back), json_rows(&serie));

    // Without the header the first record is the first row.
    let bare = Sheet::from_serie("Bare", &serie, RecordHeader::None).unwrap();
    assert_eq!(bare.scalar(at("A1")), Scalar::from(1_i64));
    assert_eq!(bare.dimension(), Some(range("A1:E2")));

    assert_eq!(
        Sheet::from_serie("a/b", &serie, RecordHeader::Source)
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
    let sheet = Sheet::from_serie("Prices", &prices, RecordHeader::Source).unwrap();
    assert_eq!(references(sheet.cells()), vec!["A1", "A2", "A4"]);
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("price"));
    assert_eq!(sheet.scalar(at("A4")), Scalar::from(2.5));

    let run = Serie::new(vec![Scalar::from(1.5)]);
    assert_eq!(
        Sheet::from_serie("Run", &run, RecordHeader::Source)
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
    sheet
        .write_serie(at("B3"), &id_symbol(), RecordHeader::Source)
        .unwrap();
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

    sheet
        .write_serie(at("E7"), &id_symbol(), RecordHeader::None)
        .unwrap();
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
            .write_serie(
                CellRef::new(MAX_ROWS - 1, 0),
                &id_symbol(),
                RecordHeader::Source
            )
            .unwrap_err()
            .to_string(),
        "invalid record value at $: expected 3 rows and 2 columns from A1048576 to fit 1048576 \
         rows by 16384 columns"
    );
    assert_eq!(
        sheet
            .write_serie(
                CellRef::new(0, MAX_COLUMNS - 1),
                &id_symbol(),
                RecordHeader::None
            )
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
            RecordHeader::None,
        )
        .unwrap();
    assert_eq!(sheet.dimension(), Some(range("XFC1048575:XFD1048576")));
}

#[test]
fn extend_from_serie_appends_below_the_last_row_from_the_first_column_without_a_header() {
    let mut sheet = Sheet::from_serie("Book", &id_symbol(), RecordHeader::Source).unwrap();
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
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
            .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
            .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
        .into_serie(None, RecordHeader::None, ArrowCastOptions::default())
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
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
        .unwrap();
    assert_eq!(rows.len(), 0);
    let root = rows.field().unwrap();
    assert_eq!(root.name(), "row");
    assert!(!root.is_nullable());
    assert!(root.fields().is_empty());

    let declared = record([DataType::Int64.required_field("id")]);
    let rows = Sheet::new("Empty")
        .unwrap()
        .into_serie(
            Some(&declared),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
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
            .with_error(ExcelError::NA),
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
        .into_serie(
            Some(&field),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
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
            .into_serie(Some(&prices), RecordHeader::Source, strict)
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
            .into_serie(Some(&counts), RecordHeader::Source, strict)
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!A3: expected int64, got string"
    );
    let lenient = halves
        .into_serie(
            Some(&counts),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(json_rows(&lenient), vec!["[7]", "[null]"]);

    // A required column refuses whatever `safe` says, the error cell by name.
    let quantities = record([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("qty"),
    ]);
    assert_eq!(
        declared_sheet()
            .into_serie(
                Some(&quantities),
                RecordHeader::Source,
                ArrowCastOptions::default()
            )
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!C3: expected a int64 value, got the error #N/A"
    );

    // Shared header pairing now refuses the missing required column before
    // reading any body row, with the same located error as the stream.
    let venues = record([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.nullable_field("price"),
        DataType::Int64.nullable_field("qty"),
        DataType::Date32.nullable_field("day"),
        DataType::utf8().required_field("venue"),
    ]);
    assert_eq!(
        declared_sheet()
            .into_serie(
                Some(&venues),
                RecordHeader::Source,
                ArrowCastOptions::default()
            )
            .unwrap_err()
            .to_string(),
        "invalid record value at $.venue: expected the column in the header of Sheet1, got [symbol, price, qty, day]"
    );
}

/// The writer must use the held sheet, not copy an untouched ZIP member.
#[test]
fn noncanonical_temporal_serials_survive_an_unrelated_held_sheet_edit() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>59</v></c>\
         <c r=\"B1\" s=\"1\"><v>60</v></c>\
         <c r=\"C1\" s=\"2\"><v>45292.000000001</v></c></row>",
        &[],
        &[],
        &[0, 14, 22],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    // The typed model cannot distinguish the 1900 phantom day from 59,
    // nor the sub-millisecond fraction from the nearest millisecond.
    assert_eq!(sheet.scalar(at("A1")), sheet.scalar(at("B1")));
    assert_eq!(sheet.cell(at("B1")).unwrap().format(), NumberFormat::Date);
    assert_eq!(
        sheet.cell(at("C1")).unwrap().format(),
        NumberFormat::DateTime
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("D1"), 7.0)
        .unwrap();
    let written = workbook.into_bytes().unwrap();
    let part = part_text(&written, "xl/worksheets/sheet1.xml");
    assert!(part.contains("<c r=\"D1\"><v>7</v></c>"), "{part}");
    let number_at = |reference: &str| -> f64 {
        let marker = format!("<c r=\"{reference}\"");
        let cell = part
            .split_once(&marker)
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0;
        cell.split_once("<v>")
            .unwrap()
            .1
            .split_once("</v>")
            .unwrap()
            .0
            .parse()
            .unwrap()
    };
    for (reference, original) in [("A1", "59"), ("B1", "60"), ("C1", "45292.000000001")] {
        assert_eq!(
            number_at(reference).to_bits(),
            original.parse::<f64>().unwrap().to_bits(),
            "{reference} changed in {part}"
        );
    }
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
        .into_serie(None, RecordHeader::Source, ArrowCastOptions::default())
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
            .with_formula(Formula::from_file("B1+1", at("I1"))),
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
    assert_eq!(back.cell_count(), sheet.cell_count());
    // A temporal cell reads back under the style the save interned for its
    // format; every other cell under the default it was written with.
    let styles = read.style_sheet().unwrap();
    for (written, read) in sheet.cells().zip(back.cells()) {
        match written.format().code() {
            Some(code) => assert_eq!(
                styles.style(read.style()).unwrap().number_format,
                code,
                "{}",
                written.reference()
            ),
            None => assert_eq!(read.style(), StyleId::DEFAULT, "{}", written.reference()),
        }
        assert_eq!(
            &read.clone().with_style(StyleId::DEFAULT),
            written,
            "{}",
            written.reference()
        );
    }
}

/// The worksheet part `part` of the package `bytes`, as text.
fn part_text(bytes: &[u8], part: &str) -> String {
    let archive = std::sync::Arc::new(yggdryl::zip::ZipArchive::new(
        yggdryl::holder::Holder::buffer(yggdryl::holder::Buffer::from_bytes(bytes.to_vec())),
    ));
    String::from_utf8(archive.read_member(part).unwrap()).unwrap()
}

/// `Sheet1` of a one-sheet package holding `sheet_data`, with `cellXfs` of
/// `xfs` General formats and the shared string `s`.
fn opened(sheet_data: &str, xfs: usize) -> Workbook {
    Workbook::from_bytes(crate::excel_package::one_sheet(
        sheet_data,
        &["s"],
        &[],
        &vec![0; xfs],
    ))
    .unwrap()
}

#[test]
fn the_cell_count_and_the_dimension_follow_every_edit_without_a_walk() {
    let mut sheet = Sheet::new("Span").unwrap();
    assert_eq!((sheet.cell_count(), sheet.dimension()), (0, None));
    sheet.set_cell(at("C2"), 1.0).unwrap();
    sheet.set_cell(at("E4"), 2.0).unwrap();
    sheet.set_cell(at("B3"), 3.0).unwrap();
    assert_eq!(sheet.cell_count(), 3);
    assert_eq!(sheet.dimension(), Some(range("B2:E4")));
    // A replaced cell is not a second one.
    sheet.set_cell(at("C2"), 4.0).unwrap();
    assert_eq!(sheet.cell_count(), 3);

    // An edge column emptied moves the edge to the next column holding one.
    sheet.remove_cell(at("B3")).unwrap();
    assert_eq!(sheet.dimension(), Some(range("C2:E4")));
    sheet.remove_cell(at("E4")).unwrap();
    assert_eq!(sheet.dimension(), Some(range("C2:C2")));
    sheet.set_cell(at("XFD1048576"), 5.0).unwrap();
    assert_eq!(sheet.dimension(), Some(range("C2:XFD1048576")));
    sheet.remove_cell(at("C2")).unwrap();
    assert_eq!(sheet.dimension(), Some(range("XFD1048576:XFD1048576")));
    assert_eq!(sheet.remove_cell(at("C2")), None);
    sheet.remove_cell(at("XFD1048576")).unwrap();
    assert_eq!((sheet.cell_count(), sheet.dimension()), (0, None));
    assert!(sheet.is_empty());

    // Moving rows or columns keeps every cell counted; removing them
    // uncounts theirs.
    for reference in ["A1", "B5", "D9", "D10"] {
        sheet.set_cell(at(reference), 1.0).unwrap();
    }
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    workbook.insert_rows("Span", 2, 3).unwrap();
    let sheet = workbook.sheet("Span").unwrap();
    assert_eq!(sheet.cell_count(), 4);
    assert_eq!(sheet.dimension(), Some(range("A1:D13")));
    workbook.insert_columns("Span", 1, 2).unwrap();
    let sheet = workbook.sheet("Span").unwrap();
    assert_eq!(sheet.dimension(), Some(range("A1:F13")));
    workbook.remove_rows("Span", 0..8).unwrap();
    let sheet = workbook.sheet("Span").unwrap();
    assert_eq!(sheet.cell_count(), 2);
    assert_eq!(sheet.dimension(), Some(range("F4:F5")));
    assert_eq!(references(sheet.cells()), ["F4", "F5"]);
    workbook.remove_columns("Span", 0..5).unwrap();
    let sheet = workbook.sheet("Span").unwrap();
    assert_eq!(references(sheet.cells()), ["A4", "A5"]);
    assert_eq!(sheet.dimension(), Some(range("A4:A5")));
}

#[test]
fn every_change_through_a_mutable_borrow_moves_the_revision_and_nothing_else_does() {
    let mut sheet = Sheet::new("Edits").unwrap();
    assert_eq!(sheet.revision(), 0);
    let mut last = 0;
    let mut moved = |sheet: &Sheet, expected: bool, what: &str| {
        assert_eq!(sheet.revision() > last, expected, "{what}");
        last = sheet.revision();
    };

    sheet.set_cell(at("A1"), 1.0).unwrap();
    moved(&sheet, true, "set_cell");
    sheet
        .insert_cell(Cell::new(
            at("B1"),
            CellKind::Number,
            NumberFormat::General,
            Scalar::from(2.0),
        ))
        .unwrap();
    moved(&sheet, true, "insert_cell");
    assert!(sheet.set_cell(CellRef::new(0, MAX_COLUMNS), 1.0).is_err());
    moved(&sheet, false, "a refused set_cell");
    assert_eq!(sheet.remove_cell(at("Z9")), None);
    moved(&sheet, false, "removing an absent cell");
    sheet.remove_cell(at("B1")).unwrap();
    moved(&sheet, true, "remove_cell");
    assert!(sheet.cell_mut(at("Z9")).is_none());
    moved(&sheet, false, "cell_mut of an absent cell");
    let _ = sheet.cell_mut(at("A1")).unwrap();
    moved(&sheet, true, "cell_mut of a present cell");

    sheet.set_name("Edits").unwrap();
    moved(&sheet, false, "renaming to the same name");
    sheet.set_name("Renamed").unwrap();
    moved(&sheet, true, "set_name");
    sheet.set_state(SheetState::Visible);
    moved(&sheet, false, "the same state");
    sheet.set_state(SheetState::Hidden);
    moved(&sheet, true, "set_state");
    sheet.set_date_system(DateSystem::Year1900);
    moved(&sheet, false, "the same date system");
    sheet.set_date_system(DateSystem::Year1904);
    moved(&sheet, true, "set_date_system");

    sheet.set_row_height(0..2, None).unwrap();
    moved(&sheet, false, "clearing a height no row states");
    sheet.set_row_height(0..2, Some(30.0)).unwrap();
    moved(&sheet, true, "set_row_height");
    sheet.set_column_width(3..4, Some(12.0)).unwrap();
    moved(&sheet, true, "set_column_width");
    sheet.set_column_width(3..4, Some(12.0)).unwrap();
    moved(&sheet, false, "the same width");
    sheet.set_rows_hidden(4..5, false).unwrap();
    moved(&sheet, false, "showing a row shown");
    sheet.set_columns_hidden(0..1, true).unwrap();
    moved(&sheet, true, "set_columns_hidden");
    assert!(!sheet.unmerge(range("A1:C3")));
    moved(&sheet, false, "unmerging where no merge is");
    sheet.extend_from_serie(&id_symbol()).unwrap();
    moved(&sheet, true, "extend_from_serie");

    let _ = (
        sheet.cell(at("A2")),
        sheet.scalar(at("A2")),
        sheet.dimension(),
        sheet.cell_count(),
        sheet.slice(range("A1:B2")),
        sheet.cells().count(),
    );
    moved(&sheet, false, "reads");

    // A revision is how often a sheet changed, never what it holds.
    let same = sheet_of("Renamed", &[]);
    assert_eq!(same.revision(), 0);
    assert_eq!(sheet.slice(range("A1:A1")).revision(), 0);
    let mut twice = sheet_of("Twice", &[("A1", Scalar::from(1.0))]);
    let once = twice.clone();
    twice.set_cell(at("A1"), 1.0).unwrap();
    assert_ne!(twice.revision(), once.revision());
    assert_eq!(twice, once);
}

#[test]
fn a_row_holds_its_cells_in_column_order_however_they_were_set() {
    let mut sheet = Sheet::new("Order").unwrap();
    for reference in ["Z1", "C1", "AA1", "A1", "M1"] {
        sheet.set_cell(at(reference), reference).unwrap();
    }
    let row = sheet.row(0).unwrap();
    assert_eq!(references(row.cells()), ["A1", "C1", "M1", "Z1", "AA1"]);
    assert_eq!(row.len(), 5);
    assert_eq!(row.cell(12).unwrap().text(), "M1");
    assert!(row.cell(13).is_none());
    assert_eq!(
        references(sheet.cells_in(range("B1:Z9"))),
        ["C1", "M1", "Z1"]
    );
    assert_eq!(references(sheet.cells_in(range("D1:L1"))).len(), 0);
    assert_eq!(references(sheet.column(26)), ["AA1"]);
}

#[test]
fn what_a_few_cells_state_beside_their_value_moves_with_the_cell_and_goes_with_it() {
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\" cm=\"1\"><f>SEQUENCE(2)</f><v>1</v></c><c r=\"B1\" vm=\"2\"><v>2</v></c>\
         <c r=\"C1\" t=\"s\" ph=\"1\"><v>0</v></c></row>\
         <row r=\"3\"><c r=\"A3\" cm=\"1\"><v>3</v></c><c r=\"B3\" vm=\"1\" ph=\"true\"><v>4</v></c></row>",
        1,
    );
    // A sheet only read is carried as the file spelled it.
    workbook.sheet("Sheet1").unwrap();
    let written = workbook.into_bytes().unwrap();
    let part = part_text(&written, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<c r=\"A1\" cm=\"1\"><f>SEQUENCE(2)</f><v>1</v></c>"),
        "{part}"
    );
    // Changed, it is written from its cells. The interim reading: a formula
    // is written without the array attributes a dynamic array's `<f>`
    // states, so its `cm` - which declares that array - is held with the
    // cell and left out of the part rather than name an array the plain
    // `<f>` does not.
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("D9"), 0.0)
        .unwrap();
    let written = workbook.into_bytes().unwrap();
    let part = part_text(&written, "xl/worksheets/sheet1.xml");
    for spelled in [
        "<c r=\"A1\"><f>SEQUENCE(2)</f><v>1</v></c>",
        "<c r=\"B1\" vm=\"2\"><v>2</v></c>",
        "<c r=\"C1\" t=\"s\" ph=\"1\"><v>0</v></c>",
        "<c r=\"A3\" cm=\"1\"><v>3</v></c>",
        "<c r=\"B3\" vm=\"1\" ph=\"1\"><v>4</v></c>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }

    // A moved row carries them; a replaced or removed cell drops its own.
    workbook.insert_rows("Sheet1", 1, 1).unwrap();
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    sheet.set_cell(at("B1"), 5.0).unwrap();
    sheet.remove_cell(at("C1")).unwrap();
    sheet.set_cell(at("C1"), "back").unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(part.contains("<c r=\"A4\" cm=\"1\"><v>3</v></c>"), "{part}");
    assert!(
        part.contains("<c r=\"B4\" vm=\"1\" ph=\"1\"><v>4</v></c>"),
        "{part}"
    );
    assert!(part.contains("<c r=\"B1\"><v>5</v></c>"), "{part}");
    assert!(!part.contains("ph=\"1\"><v>0</v>"), "{part}");
    assert!(!part.contains("r=\"A3\""), "{part}");

    // Removed rows take theirs along; the rows below keep theirs.
    workbook.remove_rows("Sheet1", 0..1).unwrap();
    let sliced = workbook.sheet("Sheet1").unwrap().slice(range("A3:A3"));
    workbook.insert_sheet(sliced).unwrap();
    // A sheet put in place of another is a new one: it takes a part of its
    // own, and the part it replaced goes.
    let written = workbook.into_bytes().unwrap();
    assert!(
        yggdryl::zip::ZipArchive::new(yggdryl::holder::Holder::buffer(
            yggdryl::holder::Buffer::from_bytes(written.clone())
        ))
        .get_entry("xl/worksheets/sheet1.xml")
        .unwrap()
        .is_none()
    );
    let part = part_text(&written, "xl/worksheets/sheet2.xml");
    assert!(part.contains("<c r=\"A3\" cm=\"1\"><v>3</v></c>"), "{part}");
    assert!(!part.contains("vm="), "{part}");
    assert!(!part.contains("SEQUENCE"), "{part}");
}

#[test]
fn a_cell_borrowed_through_cell_mut_is_back_at_its_reference_whatever_was_put_there() {
    let mut sheet = sheet_of(
        "Kept",
        &[("A1", Scalar::from(1.0)), ("B1", Scalar::from(2.0))],
    );
    let off_grid = CellRef::new(0, 20_000);
    let mut cell = sheet.cell_mut(at("A1")).unwrap();
    *cell = cell.clone().at(off_grid).with_error(ExcelError::NA);
    assert_eq!(cell.reference(), off_grid);
    drop(cell);
    let cell = sheet.cell(at("A1")).unwrap();
    assert_eq!(
        (cell.reference(), cell.error()),
        (at("A1"), Some(ExcelError::NA))
    );
    assert_eq!(sheet.cell(off_grid), None);
    assert_eq!(sheet.remove_cell(off_grid), None);
    assert_eq!(
        (sheet.cell_count(), sheet.dimension()),
        (2, Some(range("A1:B1")))
    );
    assert_eq!(
        references(sheet.slice(range("A1:C1")).cells()),
        ["A1", "B1"]
    );

    // A borrow forgotten rather than dropped cannot put the cell back; the
    // sheet still answers every verb around the stray cell, never panics.
    let mut cell = sheet.cell_mut(at("B1")).unwrap();
    *cell = cell.clone().at(off_grid);
    std::mem::forget(cell);
    let _ = sheet.dimension();
    let _ = sheet.slice(CellRange::all());
    let _ = sheet.remove_cell(off_grid);
    sheet.set_cell(at("C1"), 3.0).unwrap();
    let _ = sheet.remove_cell(at("C1"));
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    let _ = workbook.into_bytes();
}

#[test]
fn a_sheet_read_from_a_part_is_at_revision_zero_until_it_changes() {
    let mut workbook = opened("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", 1);
    assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), 0);
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    assert_eq!(sheet.revision(), 0);
    sheet.set_name("Sheet1").unwrap();
    sheet.set_state(SheetState::Visible);
    sheet.set_date_system(DateSystem::Year1900);
    assert_eq!(sheet.remove_cell(at("Z9")), None);
    assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), 0);
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 2.0)
        .unwrap();
    assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), 1);
}

#[test]
fn a_declared_column_refuses_an_unrecognized_error_by_the_literal_the_file_held() {
    let workbook = opened(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\" t=\"e\"><v>#FUTURE!</v></c></row>",
        1,
    );
    let sheet = workbook.sheet("Sheet1").unwrap().clone();
    let field = record([DataType::Float64.required_field("s")]);
    assert_eq!(
        sheet
            .into_serie(
                Some(&field),
                RecordHeader::Source,
                ArrowCastOptions::default()
            )
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!A2: expected a float64 value, got the error #FUTURE!"
    );
}

#[test]
fn an_error_cell_stating_no_error_holds_nothing_and_is_written_back_as_no_error() {
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\" t=\"e\"/><c r=\"B1\" t=\"e\"><v></v></c>\
         <c r=\"C1\" t=\"e\"><f>NEXT()</f></c><c r=\"D1\" t=\"e\"><v> </v></c></row>",
        1,
    );
    let sheet = workbook.sheet("Sheet1").unwrap();
    for reference in ["A1", "B1", "C1", "D1"] {
        let cell = sheet.cell(at(reference)).unwrap();
        assert_eq!(
            (cell.error(), cell.error_text(), cell.kind(), cell.value()),
            (None, "", CellKind::Number, &Scalar::Null),
            "{reference}"
        );
    }
    assert_eq!(
        sheet.cell(at("C1")).unwrap().formula(),
        Some(&Formula::from_file("NEXT()", at("C1")))
    );

    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("E1"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(!part.contains("t=\"e\""), "{part}");
    for spelled in [
        "<c r=\"A1\"></c>",
        "<c r=\"B1\"></c>",
        "<c r=\"C1\"><f>NEXT()</f></c>",
        "<c r=\"D1\"></c>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
}

#[test]
fn a_cell_of_kind_e_holding_no_error_is_refused_naming_it_when_the_part_is_written() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Faults").unwrap();
    sheet
        .insert_cell(Cell::new(
            at("B2"),
            CellKind::Error,
            NumberFormat::General,
            Scalar::Null,
        ))
        .unwrap();
    assert_eq!(
        workbook.into_bytes().unwrap_err().to_string(),
        "invalid record value at Faults!B2: expected the error an error cell holds, got a \
         cell of kind e holding none"
    );

    // An unrecognized error with no literal has nothing a part can spell.
    let sheet = workbook.sheet_mut("Faults").unwrap();
    sheet
        .insert_cell(
            Cell::new(
                at("B2"),
                CellKind::Number,
                NumberFormat::General,
                Scalar::from(1.0),
            )
            .with_error(ExcelError::Unrecognized),
        )
        .unwrap();
    assert_eq!(
        workbook.into_bytes().unwrap_err().to_string(),
        "invalid record value at Faults!B2: expected the literal an unrecognized error was \
         read with, got none"
    );
}

#[test]
fn an_unrecognized_error_is_written_back_as_the_literal_it_was_read_with() {
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\" t=\"e\"><f>FUTURE()</f><v>#FUTURE!</v></c>\
         <c r=\"B1\" t=\"e\"><v>#SPILL!</v></c></row>",
        1,
    );
    let sheet = workbook.sheet("Sheet1").unwrap();
    let future = sheet.cell(at("A1")).unwrap();
    assert_eq!(future.error(), Some(ExcelError::Unrecognized));
    assert_eq!(future.error_text(), "#FUTURE!");
    assert_eq!(sheet.scalar(at("A1")), Scalar::Null);
    assert_eq!(
        sheet.cell(at("B1")).unwrap().error(),
        Some(ExcelError::Spill)
    );

    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("C1"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<c r=\"A1\" t=\"e\"><f>FUTURE()</f><v>#FUTURE!</v></c>"),
        "{part}"
    );
    assert!(
        part.contains("<c r=\"B1\" t=\"e\"><v>#SPILL!</v></c>"),
        "{part}"
    );
}

#[test]
fn a_shared_formula_group_is_one_shape_every_dependent_holds_translated_at_its_cell() {
    // A dependent may come before its master in the part: it is resolved
    // once the part is read.
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f t=\"shared\" si=\"1\"/><v>7</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><f t=\"shared\" ref=\"A2:A3\" si=\"0\">A1+1</f><v>2</v></c>\
         <c r=\"B2\"><f t=\"shared\" ref=\"B1:B2\" si=\"1\">$A$1*A2</f><v>2</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><f t=\"shared\" si=\"0\"/><v>3</v></c></row>",
        1,
    );
    let sheet = workbook.sheet("Sheet1").unwrap();
    let formula = |reference: &str| {
        let cell = sheet.cell(at(reference)).unwrap();
        cell.formula()
            .map(|formula| formula.at(cell.reference()).to_string())
    };
    assert_eq!(formula("A2").as_deref(), Some("A1+1"));
    assert_eq!(formula("A3").as_deref(), Some("A2+1"));
    assert_eq!(formula("B2").as_deref(), Some("$A$1*A2"));
    assert_eq!(formula("B1").as_deref(), Some("$A$1*A1"));
    // Each dependent keeps the value its master cached for it.
    assert_eq!(sheet.scalar(at("A3")), Scalar::from(3.0));
    assert_eq!(
        sheet.cell(at("A3")).unwrap().formula(),
        sheet.cell(at("A2")).unwrap().formula()
    );

    // A sheet written again writes one plain `<f>` per cell, each spelled
    // at its own cell; nothing is written as an empty `<f>`.
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("C1"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(!part.contains("<f></f>"), "{part}");
    assert!(!part.contains("shared"), "{part}");
    for cell in [
        "<c r=\"A2\"><f>A1+1</f><v>2</v></c>",
        "<c r=\"A3\"><f>A2+1</f><v>3</v></c>",
        "<c r=\"B1\"><f>$A$1*A1</f><v>7</v></c>",
        "<c r=\"B2\"><f>$A$1*A2</f><v>2</v></c>",
    ] {
        assert!(part.contains(cell), "{cell} in {part}");
    }

    // A formula whose text is empty is none, however it was built.
    let mut built = Workbook::new();
    built
        .add_sheet("Empty")
        .unwrap()
        .insert_cell(
            Cell::new(
                at("A1"),
                CellKind::Number,
                NumberFormat::General,
                Scalar::from(1.0),
            )
            .with_formula(Formula::from_file("", at("A1"))),
        )
        .unwrap();
    let part = part_text(&built.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(part.contains("<c r=\"A1\"><v>1</v></c>"), "{part}");
}

#[test]
fn a_shared_formula_dependent_with_no_master_is_refused_naming_the_cell_and_the_group() {
    let workbook = opened(
        "<row r=\"1\"><c r=\"A1\"><f t=\"shared\" ref=\"A1:A2\" si=\"0\">1+1</f><v>2</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><f t=\"shared\" si=\"4\"/><v>2</v></c></row>",
        1,
    );
    let error = workbook.sheet("Sheet1").unwrap_err().to_string();
    assert_eq!(
        error,
        "invalid record value at Sheet1!A2: expected the master of shared formula 4 in the \
         part, got a dependent naming a group no cell states the text of"
    );
}

#[test]
fn array_data_table_and_dynamic_array_formulas_keep_what_their_f_states() {
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A2\">{1;2}*2</f><v>2</v></c>\
         <c r=\"B1\" cm=\"1\"><f t=\"array\" ref=\"B1:B3\">_xlfn.SEQUENCE(3)</f><v>1</v></c>\
         <c r=\"C1\"><f ca=\"1\">NOW()</f><v>45000.5</v></c>\
         <c r=\"D1\"><f>TODAY()</f><v>45000</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>4</v></c>\
         <c r=\"E2\"><f t=\"dataTable\" ref=\"E2:F3\" dt2D=\"1\" dtr=\"1\" r1=\"A1\" r2=\"B1\"/><v>9</v></c></row>",
        1,
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("Z9"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    for cell in [
        "<c r=\"A1\"><f t=\"array\" ref=\"A1:A2\">{1;2}*2</f><v>2</v></c>",
        // A dynamic array's anchor keeps its cell metadata with its array.
        "<c r=\"B1\" cm=\"1\"><f t=\"array\" ref=\"B1:B3\">_xlfn.SEQUENCE(3)</f><v>1</v></c>",
        "<c r=\"C1\"><f ca=\"1\">NOW()</f><v>45000.5</v></c>",
        // A volatile formula asks to be calculated every time.
        "<c r=\"D1\"><f ca=\"1\">TODAY()</f><v>45000</v></c>",
        "<c r=\"E2\"><f t=\"dataTable\" ref=\"E2:F3\" dt2D=\"1\" dtr=\"1\" r1=\"A1\" r2=\"B1\"/><v>9</v></c>",
    ] {
        assert!(part.contains(cell), "{cell} in {part}");
    }
}

#[test]
fn a_cell_keeps_the_style_the_file_gave_it_through_an_edit_of_another_cell() {
    let mut workbook = opened(
        "<row r=\"1\"><c r=\"A1\" s=\"3\"><v>1</v></c><c r=\"B1\" s=\"2\" t=\"s\"><v>0</v></c>\
         <c r=\"C1\" s=\"1\"/></row>",
        4,
    );
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell(at("A1")).unwrap().style(), StyleId::new(3));
    assert_eq!(sheet.cell(at("B1")).unwrap().style(), StyleId::new(2));
    // A styled blank is a cell holding nothing.
    assert!(sheet.cell(at("C1")).unwrap().is_null());

    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("D1"), 2.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    for spelled in [
        "<c r=\"A1\" s=\"3\"><v>1</v></c>",
        "<c r=\"B1\" s=\"2\" t=\"s\"><v>0</v></c>",
        "<c r=\"C1\" s=\"1\"></c>",
        "<c r=\"D1\"><v>2</v></c>",
    ] {
        assert!(part.contains(spelled), "{spelled} in {part}");
    }
}

/// `bytes` with each named member's text replaced, every other member as
/// it was.
fn repacked(bytes: &[u8], replaced: &[(&str, String)]) -> Vec<u8> {
    let archive = std::sync::Arc::new(yggdryl::zip::ZipArchive::new(
        yggdryl::holder::Holder::buffer(yggdryl::holder::Buffer::from_bytes(bytes.to_vec())),
    ));
    let members: Vec<(String, String)> = archive
        .entries()
        .unwrap()
        .iter()
        .map(|entry| {
            let name = entry.name().to_owned();
            let text = replaced
                .iter()
                .find(|(part, _)| *part == name)
                .map_or_else(|| part_text(bytes, &name), |(_, text)| text.clone());
            (name, text)
        })
        .collect();
    let parts: Vec<(&str, &str)> = members
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    crate::excel_package::package(&parts)
}

#[test]
fn a_package_this_crate_wrote_keeps_the_styles_excel_added_after_its_formats() {
    // The crate writes the default format and the date format A1 needed;
    // Excel then appends a style of its own and gives it to B1.
    let mut built = Workbook::new();
    let sheet = built.add_sheet("Sheet1").unwrap();
    sheet.set_cell(at("A1"), Scalar::date32(19_723)).unwrap();
    sheet.set_cell(at("B1"), 2.0).unwrap();
    let written = built.into_bytes().unwrap();
    let styles = part_text(&written, "xl/styles.xml");
    assert!(styles.contains("<cellXfs count=\"2\">"), "{styles}");
    let dated = part_text(&written, "xl/worksheets/sheet1.xml");
    let dated = dated[dated.find("<c r=\"A1\"").unwrap()..]
        .split_once("</c>")
        .unwrap()
        .0
        .to_owned();
    let styles = styles
        .replacen("<cellXfs count=\"2\">", "<cellXfs count=\"3\">", 1)
        .replacen(
            "</cellXfs>",
            "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/></cellXfs>",
            1,
        );
    let sheet_part = part_text(&written, "xl/worksheets/sheet1.xml").replacen(
        "<c r=\"B1\">",
        "<c r=\"B1\" s=\"2\">",
        1,
    );
    let styled = repacked(
        &written,
        &[
            ("xl/styles.xml", styles),
            ("xl/worksheets/sheet1.xml", sheet_part),
        ],
    );

    // Opened again, an edit of another cell keeps B1's style and A1's.
    let mut workbook = Workbook::from_bytes(styled.clone()).unwrap();
    assert_eq!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .cell(at("B1"))
            .unwrap()
            .style(),
        StyleId::new(2)
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("C1"), 3.0)
        .unwrap();
    let rewritten = workbook.into_bytes().unwrap();
    let part = part_text(&rewritten, "xl/worksheets/sheet1.xml");
    assert!(part.contains(&dated), "{dated} in {part}");
    assert!(part.contains("<c r=\"B1\" s=\"2\"><v>2</v></c>"), "{part}");
    assert!(part.contains("<c r=\"C1\"><v>3</v></c>"), "{part}");
    // A1's date reads as its own style's format, so nothing was interned:
    // the styles part is carried as it was.
    assert_eq!(
        part_text(&rewritten, "xl/styles.xml"),
        part_text(&styled, "xl/styles.xml")
    );
}

#[test]
fn a_style_index_the_workbook_does_not_hold_is_refused_at_save_naming_the_cell() {
    // A workbook built from nothing holds one cell format, the default.
    let mut built = Workbook::new();
    built
        .add_sheet("Styled")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("A1"), Scalar::from(1.0), DateSystem::Year1900)
                .unwrap()
                .with_style(StyleId::new(3)),
        )
        .unwrap();
    assert_eq!(
        built.into_bytes().unwrap_err().to_string(),
        "invalid record value at Styled!A1: expected a style the workbook's 1 cell formats hold, \
         got index 3"
    );

    // An opened package holds its own, and an index past them is refused
    // the same way; one of them is written as it is.
    let mut workbook = opened("<row r=\"1\"><c r=\"A1\" s=\"1\"><v>1</v></c></row>", 2);
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    sheet
        .insert_cell(
            Cell::from_scalar(at("B1"), Scalar::from(2.0), DateSystem::Year1900)
                .unwrap()
                .with_style(StyleId::new(40)),
        )
        .unwrap();
    assert_eq!(
        workbook.into_bytes().unwrap_err().to_string(),
        "invalid record value at Sheet1!B1: expected a style the workbook's 2 cell formats hold, \
         got index 40"
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .insert_cell(
            Cell::from_scalar(at("B1"), Scalar::from(2.0), DateSystem::Year1900)
                .unwrap()
                .with_style(StyleId::new(1)),
        )
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(part.contains("<c r=\"A1\" s=\"1\"><v>1</v></c>"), "{part}");
    assert!(part.contains("<c r=\"B1\" s=\"1\"><v>2</v></c>"), "{part}");
}

#[test]
fn a_rich_inline_string_keeps_its_runs_while_it_holds_the_text_they_spell() {
    let rich = "<c r=\"A1\" t=\"inlineStr\"><is><r><rPr><b/></rPr><t>be</t></r><r><t>ta</t></r>\
                <rPh sb=\"0\" eb=\"1\"><t>B</t></rPh></is></c>";
    let mut workbook = opened(&format!("<row r=\"1\">{rich}</row>"), 1);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::from("beta")
    );
    // Written again for an edit elsewhere, the runs and the phonetic text
    // are the bytes the part held.
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(part.contains(rich), "{part}");
    // A cell holding other text since writes its own: the runs spelled the
    // text it was read with.
    {
        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        let mut cell = sheet.cell_mut(at("A1")).unwrap();
        *cell = Cell::new(
            at("A1"),
            CellKind::InlineString,
            NumberFormat::General,
            Scalar::from("gamma"),
        );
    }
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<c r=\"A1\" t=\"inlineStr\"><is><t>gamma</t></is></c>"),
        "{part}"
    );
}

#[cfg(feature = "internals")]
mod internal {

    #[test]
    fn sheet_changes_stay_inactive_for_ordinary_editing_and_record_import() {
        use yggdryl::internals::excel_sheet::changes_active;
        let mut sheet = Sheet::new("Data").unwrap();
        sheet.set_cell(at("A1"), 1.0).unwrap();
        sheet
            .write_serie(at("C4"), &super::id_symbol(), yggdryl::RecordHeader::Source)
            .unwrap();
        {
            let mut cell = sheet.cell_mut(at("A1")).unwrap();
            *cell = cell
                .clone()
                .with_formula(yggdryl::excel::Formula::from_file("1+1", at("A1")));
        }
        sheet.remove_cell(at("C4"));
        sheet.set_rows_hidden(2..4, true).unwrap();
        assert!(!changes_active(&sheet));
        assert!(!changes_active(&sheet.clone()));
    }

    #[test]
    fn sheet_changes_track_insert_replace_remove_and_a_forgotten_cell_guard() {
        use yggdryl::excel::{Cell, DateSystem, Formula};
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, pending_changes, track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        sheet.set_cell(at("A1"), 0.0).unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        for value in 1..=100 {
            sheet.set_cell(at("A1"), value).unwrap();
        }
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at("A1"), false)]))
        );
        sheet
            .insert_cell(
                Cell::from_scalar(at("A1"), 0.0.into(), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file("B1+1", at("A1"))),
            )
            .unwrap();
        sheet.set_cell(at("A1"), 2.0).unwrap();
        sheet.set_cell(at("B1"), 3.0).unwrap();
        sheet.remove_cell(at("B1"));
        {
            let mut cell = sheet.cell_mut(at("A1")).unwrap();
            *cell = cell
                .clone()
                .with_formula(Formula::from_file("4+5", at("A1")));
            std::mem::forget(cell);
        }
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at("A1"), true), (at("B1"), false)]))
        );
        acknowledge_changes(&mut sheet);
        assert_eq!(pending_changes(&sheet), Some((generation, false, vec![])));
        sheet.remove_cell(at("A1"));
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at("A1"), true)]))
        );
    }

    #[test]
    fn sheet_changes_clone_and_date_system_builders_cannot_alias_an_active_instance() {
        use yggdryl::excel::DateSystem;
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, pending_changes, track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        sheet.set_cell(at("A1"), 1.0).unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        sheet.set_cell(at("A1"), 2.0).unwrap();
        let cloned = sheet.clone();
        assert_eq!(sheet, cloned, "bookkeeping is not sheet value identity");
        let (clone_generation, structural, points) = pending_changes(&cloned).unwrap();
        assert_ne!(generation, clone_generation);
        assert!(structural);
        assert!(
            points.is_empty(),
            "the full rebuild makes copied pending history unnecessary"
        );
        let revision = sheet.revision();
        let changed = sheet.with_date_system(DateSystem::Year1904);
        assert_eq!(
            changed.revision(),
            revision,
            "builder keeps its established revision contract"
        );
        assert_eq!(
            pending_changes(&changed),
            Some((generation, true, vec![(at("A1"), false)]))
        );
    }

    #[test]
    fn sheet_changes_nested_marks_restore_first_membership_flags_exactly() {
        use yggdryl::excel::{Cell, DateSystem, Formula};
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, change_mark, pending_changes, restore_change_mark, track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        sheet.set_cell(at("A1"), 1.0).unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        sheet.set_cell(at("A1"), 2.0).unwrap();
        let outer = change_mark(&sheet);
        sheet
            .insert_cell(
                Cell::from_scalar(at("A1"), 2.0.into(), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file("9", at("A1"))),
            )
            .unwrap();
        sheet.set_cell(at("B1"), 3.0).unwrap();
        let inner = change_mark(&sheet);
        sheet.set_cell(at("C1"), 4.0).unwrap();
        sheet.set_name("Temporarily renamed").unwrap();
        // The ordinary payload inverse runs before the tiny journal reset.
        sheet.remove_cell(at("C1"));
        sheet.set_name("Data").unwrap();
        restore_change_mark(&mut sheet, inner).unwrap();
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at("A1"), true), (at("B1"), false)]))
        );
        sheet.set_cell(at("A1"), 2.0).unwrap();
        sheet.remove_cell(at("B1"));
        restore_change_mark(&mut sheet, outer).unwrap();
        assert_eq!(sheet.scalar(at("A1")), 2.0.into());
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, false, vec![(at("A1"), false)]))
        );
    }

    #[test]
    fn sheet_changes_refused_and_cosmetic_edits_leave_no_calculation_seed() {
        use yggdryl::excel::{CellRef, Frozen, MAX_ROWS};
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, pending_changes, track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        assert!(sheet.set_cell(CellRef::new(MAX_ROWS, 0), 1.0).is_err());
        assert!(sheet.set_row_height(0..1, Some(f64::NAN)).is_err());
        assert!(sheet.cell_mut(at("A1")).is_none());
        assert!(sheet.remove_cell(at("A1")).is_none());
        sheet.set_row_height(0..1, Some(30.0)).unwrap();
        sheet.set_column_width(0..1, Some(20.0)).unwrap();
        sheet
            .set_frozen(Some(Frozen {
                rows: 1,
                columns: 0,
            }))
            .unwrap();
        assert_eq!(pending_changes(&sheet), Some((generation, false, vec![])));
        sheet.set_rows_hidden(100..101, true).unwrap();
        assert_eq!(pending_changes(&sheet), Some((generation, true, vec![])));
        sheet.set_cell(at("A1"), 3.0).unwrap();
        assert_eq!(
            pending_changes(&sheet),
            Some((generation, true, vec![])),
            "a rebuild does not accumulate redundant point seeds"
        );
    }

    #[test]
    fn sheet_changes_record_writes_remember_only_physically_changed_addresses() {
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, pending_changes, track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        sheet.set_cell(at("A1"), "outside").unwrap();
        sheet.set_cell(at("C4"), "old").unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        sheet
            .write_serie(at("B3"), &super::id_symbol(), yggdryl::RecordHeader::Source)
            .unwrap();
        let expected = ["B3", "C3", "B4", "C4", "B5"]
            .map(|text| (at(text), false))
            .to_vec();
        assert_eq!(pending_changes(&sheet), Some((generation, false, expected)));
        assert_eq!(sheet.scalar(at("A1")), "outside".into());
    }

    #[test]
    fn sheet_changes_cache_publication_is_neutral_and_foreign_or_stale_marks_refuse() {
        use yggdryl::Error;
        use yggdryl::excel::{Cell, DateSystem, Formula};
        use yggdryl::internals::excel_sheet::{
            acknowledge_changes, change_mark, pending_changes, replace_cache, restore_change_mark,
            track_changes,
        };
        let mut sheet = Sheet::new("Data").unwrap();
        sheet
            .insert_cell(
                Cell::from_scalar(at("A1"), 0.0.into(), DateSystem::Year1900)
                    .unwrap()
                    .with_formula(Formula::from_file("1+1", at("A1"))),
            )
            .unwrap();
        let generation = track_changes(&mut sheet);
        acknowledge_changes(&mut sheet);
        replace_cache(&mut sheet, at("A1"), 2.0).unwrap();
        assert_eq!(sheet.scalar(at("A1")), 2.0.into());
        assert_eq!(pending_changes(&sheet), Some((generation, false, vec![])));
        let foreign = change_mark(&sheet);
        let mut other = sheet.clone();
        let before = pending_changes(&other);
        assert!(matches!(
            restore_change_mark(&mut other, foreign),
            Err(Error::Conflict { .. })
        ));
        assert_eq!(pending_changes(&other), before);
        let expired = change_mark(&sheet);
        acknowledge_changes(&mut sheet);
        let before = pending_changes(&sheet);
        assert!(matches!(
            restore_change_mark(&mut sheet, expired),
            Err(Error::Conflict { .. })
        ));
        assert_eq!(pending_changes(&sheet), before);
    }

    use yggdryl::internals::excel_formula::shares_shape;
    use yggdryl::internals::excel_sheet::{has_frame, rewrite_style_ids, set_frame_items_from};

    use super::{Sheet, StyleId, Workbook, at, opened, part_text};

    const MARGINS: &str = "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>";

    fn frame_source() -> Workbook {
        use crate::excel_package::{
            NS, content_types, package, root_relationships, workbook, workbook_relationships,
        };

        let types = content_types(1, false, false);
        let root = root_relationships();
        let book = workbook(&["Sheet1"], false);
        let relationships = workbook_relationships(1, false, false);
        let part = format!(
            "<worksheet xmlns=\"{NS}\"><dimension ref=\"A1\"/><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData>{MARGINS}</worksheet>"
        );
        Workbook::from_bytes(package(&[
            ("[Content_Types].xml", &types),
            ("_rels/.rels", &root),
            ("xl/workbook.xml", &book),
            ("xl/_rels/workbook.xml.rels", &relationships),
            ("xl/worksheets/sheet1.xml", &part),
        ]))
        .unwrap()
    }

    #[test]
    fn raw_style_ids_change_only_owned_numeric_attribute_values() {
        let document = format!(
            "<?xml version='1.0'?>\r\n<!--été--><worksheet xmlns='{}' xmlns:x='urn:custom'>\
             <cols><col min='1' max='2' x:style='vendor' style = ' 0002 ' width='9.140625'/></cols>\
             <sheetData><row r='1' x:s='row-vendor' s = '&#51;'>\
             <c r='A1' x:s='cell-vendor' s = '004'><f>1&#x2b;2</f><v>0003.0000</v></c>\
             <c r='B1'><v>5</v></c></row></sheetData></worksheet><!--tail-->",
            yggdryl::excel::NAMESPACE,
        );
        let expected = document
            .replace("style = ' 0002 '", "style = '12'")
            .replace("s = '&#51;'", "s = '13'")
            .replace("s = '004'", "s = '14'");
        let mut visited = Vec::new();
        let result = rewrite_style_ids(document.as_bytes(), "xl/worksheets/sheet1.xml", |id| {
            visited.push(id.as_u16());
            Some(StyleId::new(id.as_u16() + 10))
        })
        .unwrap();
        assert_eq!(visited, [2, 3, 4]);
        assert_eq!(result.as_deref(), Some(expected.as_bytes()));
    }

    #[test]
    fn raw_style_ids_capture_without_rewriting_and_keep_equal_ids_verbatim() {
        let document = format!(
            "<worksheet xmlns='{}'><cols><col min='1' max='1' style='65535'/></cols>\
             <sheetData><row r='1' s=' 0001 '><c r='A1' s='&#x32;'><v>7.00</v></c>\
             <c r='B1'/></row></sheetData></worksheet>",
            yggdryl::excel::NAMESPACE,
        );
        let mut visited = Vec::new();
        assert_eq!(
            rewrite_style_ids(document.as_bytes(), "capture.xml", |id| {
                visited.push(id.as_u16());
                None
            })
            .unwrap(),
            None,
        );
        assert_eq!(visited, [65535, 1, 2]);
        assert_eq!(
            rewrite_style_ids(document.as_bytes(), "capture.xml", Some).unwrap(),
            None,
        );
    }

    #[test]
    fn raw_style_ids_accept_main_strict_prefix_aliases_and_encoded_namespaces() {
        for namespace in [yggdryl::excel::NAMESPACE, yggdryl::excel::STRICT_NAMESPACE] {
            for declaration in [namespace.to_owned(), namespace.replace("main", "ma&#x69;n")] {
                for document in [
                    format!(
                        "<worksheet xmlns='{declaration}'><sheetData><row r='1'><c r='A1' s='1'/></row></sheetData></worksheet>"
                    ),
                    format!(
                        "<a:worksheet xmlns:a='{declaration}'><a:sheetData><a:row r='1'><a:c r='A1' s='1'/></a:row></a:sheetData></a:worksheet>"
                    ),
                    format!(
                        "<a:worksheet xmlns:a='{declaration}'><b:sheetData xmlns:b='{declaration}'><c:row xmlns:c='{declaration}' r='1'><d:c xmlns:d='{declaration}' r='A1' s='1'/></c:row></b:sheetData></a:worksheet>"
                    ),
                ] {
                    let expected = document.replace("s='1'", "s='2'");
                    let mut visited = Vec::new();
                    let result = rewrite_style_ids(document.as_bytes(), "aliases.xml", |id| {
                        visited.push(id.as_u16());
                        Some(StyleId::new(2))
                    })
                    .unwrap();
                    assert_eq!(visited, [1], "{document}");
                    assert_eq!(result.as_deref(), Some(expected.as_bytes()), "{document}");
                }
            }
        }
    }

    #[test]
    fn raw_style_ids_ignore_foreign_ancestors_leaves_and_other_paths() {
        let main = yggdryl::excel::NAMESPACE;
        for body in [
            "<x:sheetData><row r='1' s='bad'><c r='A1' s='bad'/></row></x:sheetData>",
            "<sheetData><x:row r='1' s='bad'><c r='A1' s='bad'/></x:row></sheetData>",
            "<sheetData><row r='1'><x:c r='A1' s='bad'/></row></sheetData>",
            "<x:cols><col min='1' max='1' style='bad'/></x:cols>",
            "<cols><x:col min='1' max='1' style='bad'/></cols>",
            "<sheetData><unbound:row><c r='A1' s='bad'/></unbound:row></sheetData>",
            "<row r='1' s='bad'><c r='A1' s='bad'/></row>",
            "<cols><wrapper><col min='1' max='1' style='bad'/></wrapper></cols>",
            "<extLst><ext><worksheet><sheetData><row s='bad'><c s='bad'/></row></sheetData></worksheet></ext></extLst>",
        ] {
            let document =
                format!("<worksheet xmlns='{main}' xmlns:x='urn:foreign'>{body}</worksheet>");
            assert_eq!(
                rewrite_style_ids(document.as_bytes(), "foreign.xml", |_| panic!(
                    "visited {document}"
                ))
                .unwrap(),
                None,
            );
        }
        for document in [
            format!(
                "<worksheet xmlns='urn:foreign'><sheetData xmlns='{main}'><row r='1'><c r='A1' s='bad'/></row></sheetData></worksheet>"
            ),
            format!(
                "<worksheet xmlns='{main}'><sheetData><row xmlns=''><c xmlns='{main}' r='A1' s='bad'/></row></sheetData></worksheet>"
            ),
        ] {
            assert_eq!(
                rewrite_style_ids(document.as_bytes(), "foreign.xml", |_| panic!(
                    "visited {document}"
                ))
                .unwrap(),
                None,
            );
        }
    }

    #[test]
    fn raw_style_ids_resume_after_foreign_empty_and_closed_subtrees() {
        let document = format!(
            "<worksheet xmlns='{}' xmlns:x='urn:foreign'>\
             <cols><x:col style='bad'/><col min='1' max='1' style='1'/></cols>\
             <sheetData><x:row s='bad'/><x:row><c r='A1' s='bad'/></x:row>\
             <row r='2' s='1'><x:c s='bad'/><c r='A2' s='1'/></row></sheetData></worksheet>",
            yggdryl::excel::NAMESPACE,
        );
        // Coordinates have the same digits but remain untouched.
        let expected = document
            .replace("style='1'", "style='2'")
            .replace(" s='1'", " s='2'");
        let mut visited = Vec::new();
        let result = rewrite_style_ids(document.as_bytes(), "scope.xml", |id| {
            visited.push(id.as_u16());
            Some(StyleId::new(2))
        })
        .unwrap();
        assert_eq!(visited, [1, 1, 1]);
        assert_eq!(result.as_deref(), Some(expected.as_bytes()));
    }

    #[test]
    fn raw_style_ids_do_not_visit_qualified_namesakes_or_missing_attributes() {
        let document = format!(
            "<worksheet xmlns='{}' xmlns:x='{}'><cols>\
             <col min='1' max='1' x:style='bad' style='1'/>\
             <col min='2' max='2' x:style='bad'/></cols><sheetData>\
             <row r='1' x:s='bad' s='1'><c r='A1' x:s='bad' s='1'/><c r='B1' x:s='bad'/></row>\
             <row r='2' x:s='bad'><c r='A2'/></row></sheetData></worksheet>",
            yggdryl::excel::NAMESPACE,
            yggdryl::excel::NAMESPACE,
        );
        let expected = document
            .replace(" style='1'", " style='2'")
            .replace(" s='1'", " s='2'");
        let mut visited = Vec::new();
        let result = rewrite_style_ids(document.as_bytes(), "attributes.xml", |id| {
            visited.push(id.as_u16());
            Some(StyleId::new(2))
        })
        .unwrap();
        assert_eq!(visited, [1, 1, 1]);
        assert_eq!(result.as_deref(), Some(expected.as_bytes()));
    }

    #[test]
    fn raw_style_ids_refuse_malformed_ids_at_the_part_and_attribute() {
        let main = yggdryl::excel::NAMESPACE;
        for text in ["", "-1", "65536", "1.5", "0x1", "bad"] {
            for (body, location) in [
                (
                    format!("<cols><col min='1' max='1' style='{text}'/></cols>"),
                    "worksheet/cols/col@style",
                ),
                (
                    format!("<sheetData><row r='1' s='{text}'/></sheetData>"),
                    "worksheet/sheetData/row@s",
                ),
                (
                    format!("<sheetData><row r='1'><c r='A1' s='{text}'/></row></sheetData>"),
                    "worksheet/sheetData/row/c@s",
                ),
            ] {
                let document = format!("<worksheet xmlns='{main}'>{body}</worksheet>");
                let error = rewrite_style_ids(document.as_bytes(), "xl/worksheets/bad.xml", |_| {
                    panic!("a malformed style reached the typed callback")
                })
                .unwrap_err();
                let yggdryl::Error::InvalidRecord { path, reason } = error else {
                    panic!("expected a located refusal, got {error:?}");
                };
                assert_eq!(path, format!("xl/worksheets/bad.xml#{location}"));
                assert!(
                    reason.contains("expected a style index from 0 to 65535"),
                    "{reason}"
                );
                assert!(reason.contains(&format!("got {text:?}")), "{reason}");
            }
        }
        let error =
            rewrite_style_ids(b"<worksheet><sheetData>", "broken.xml", |_| None).unwrap_err();
        let yggdryl::Error::InvalidRecord { path, .. } = error else {
            panic!("expected a located XML refusal, got {error:?}");
        };
        assert_eq!(path, "broken.xml");
    }

    #[test]
    fn frame_items_create_a_fresh_frame_once_and_write_its_children() {
        use crate::excel_package::{NS, R_NS};

        let source = frame_source();
        let source = source.sheet("Sheet1").unwrap();
        let mut workbook = Workbook::new();
        let sheet = workbook.add_sheet("Fresh").unwrap();
        sheet.set_cell(at("A1"), 9.0).unwrap();
        let revision = sheet.revision();
        assert!(!has_frame(sheet));
        set_frame_items_from(sheet, Some(source));
        assert!(has_frame(sheet));
        assert_eq!(sheet.revision(), revision + 1);
        set_frame_items_from(sheet, Some(source));
        assert_eq!(sheet.revision(), revision + 1);
        let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert_eq!(
            part,
            format!(
                "<worksheet xmlns=\"{NS}\" xmlns:r=\"{R_NS}\"><dimension ref=\"A1\"/><sheetData><row r=\"1\"><c r=\"A1\"><v>9</v></c></row></sheetData>{MARGINS}</worksheet>"
            )
        );
    }

    #[test]
    fn frame_items_empty_update_preserves_absence_or_the_existing_frame() {
        let mut fresh = Sheet::new("Fresh").unwrap();
        let revision = fresh.revision();
        set_frame_items_from(&mut fresh, None);
        assert!(!has_frame(&fresh));
        assert_eq!(fresh.revision(), revision);

        let mut source = frame_source();
        let sheet = source.sheet_mut("Sheet1").unwrap();
        assert!(has_frame(sheet));
        let revision = sheet.revision();
        set_frame_items_from(sheet, None);
        assert!(has_frame(sheet));
        assert_eq!(sheet.revision(), revision + 1);
        set_frame_items_from(sheet, None);
        assert_eq!(sheet.revision(), revision + 1);
        let part = part_text(&source.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert!(!part.contains("<pageMargins"), "{part}");
        assert!(part.contains("<c r=\"A1\"><v>1</v></c>"), "{part}");
    }

    #[test]
    fn a_parsed_sheet_holds_one_shape_per_distinct_formula() {
        let workbook = opened(
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f>A1*2</f><v>2</v></c>\
             <c r=\"C1\"><f t=\"shared\" ref=\"C1:C3\" si=\"0\">B1+1</f><v>3</v></c></row>\
             <row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><f>A2*2</f><v>4</v></c>\
             <c r=\"C2\"><f t=\"shared\" si=\"0\"/><v>5</v></c></row>\
             <row r=\"3\"><c r=\"B3\"><f>A3*3</f><v>0</v></c><c r=\"C3\"><f t=\"shared\" si=\"0\"/><v>1</v></c></row>",
            1,
        );
        let sheet = workbook.sheet("Sheet1").unwrap();
        let formula = |reference: &str| sheet.cell(at(reference)).unwrap().formula().unwrap();
        // Formulas stated one by one that translate into one another are
        // interned into one shape as the part is read.
        assert!(shares_shape(formula("B1"), formula("B2")));
        assert!(!shares_shape(formula("B1"), formula("B3")));
        // A shared group's dependents hold its master's shape.
        assert!(shares_shape(formula("C1"), formula("C2")));
        assert!(shares_shape(formula("C1"), formula("C3")));
    }
}

#[test]
fn a_value_put_over_an_array_anchor_through_cell_mut_writes_no_array() {
    let mut workbook = Workbook::from_bytes(crate::excel_package::rich_package()).unwrap();
    let report = workbook.sheet_mut("Report").unwrap();
    for anchor in ["B2", "C2"] {
        let mut cell = report.cell_mut(at(anchor)).unwrap();
        *cell = Cell::from_scalar(at(anchor), Scalar::from(5.0), DateSystem::Year1900).unwrap();
    }
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet2.xml");
    assert!(part.contains("<c r=\"B2\"><v>5</v></c>"), "{part}");
    assert!(part.contains("<c r=\"C2\"><v>5</v></c>"), "{part}");
    assert!(!part.contains("t=\"array\""), "{part}");
    assert!(!part.contains("cm="), "{part}");
    // A data table's `<f>` states no text: its anchor keeps it.
    assert!(
        part.contains("<c r=\"D2\"><f t=\"dataTable\" ref=\"D2:D3\" dtr=\"1\" r1=\"A1\"/>"),
        "{part}"
    );
}

#[test]
fn an_array_anchor_still_holding_its_formula_writes_its_array() {
    let mut workbook = Workbook::from_bytes(crate::excel_package::rich_package()).unwrap();
    workbook
        .sheet_mut("Report")
        .unwrap()
        .set_cell(at("E4"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet2.xml");
    assert!(
        part.contains("<c r=\"B2\"><f t=\"array\" ref=\"B2:B4\">Data!B2:B4*2</f>"),
        "{part}"
    );
    assert!(
        part.contains(
            "<c r=\"C2\" cm=\"1\"><f t=\"array\" ref=\"C2:C4\">_xlfn._xlws.SORT(Data!B2:B4)</f>"
        ),
        "{part}"
    );
}

#[test]
fn a_value_set_over_a_styled_cell_keeps_its_style_and_a_new_one_its_row_s() {
    let mut sheet = Sheet::new("Styled").unwrap();
    sheet
        .insert_cell(
            Cell::from_scalar(at("B2"), Scalar::from(1.0), DateSystem::Year1900)
                .unwrap()
                .with_style(StyleId::new(3)),
        )
        .unwrap();
    let replaced = sheet.set_cell(at("B2"), "text").unwrap().unwrap();
    assert_eq!(replaced.style(), StyleId::new(3));
    assert_eq!(sheet.cell(at("B2")).unwrap().style(), StyleId::new(3));
    sheet.set_cell(at("C9"), 2.0).unwrap();
    assert_eq!(sheet.cell(at("C9")).unwrap().style(), StyleId::DEFAULT);
}

#[test]
fn widths_heights_and_hidden_flags_are_set_over_spans_and_cleared_back_to_the_default() {
    let mut sheet = Sheet::new("Layout").unwrap();
    sheet.set_column_width(1..4, Some(20.0)).unwrap();
    sheet.set_columns_hidden(2..3, true).unwrap();
    assert_eq!(sheet.column_width(1), 20.0);
    assert_eq!(sheet.column_width(3), 20.0);
    assert_eq!(sheet.column_width(4), sheet.default_column_width());
    assert!(sheet.is_column_hidden(2) && !sheet.is_column_hidden(1));
    // A width cleared inside a span splits it.
    sheet.set_column_width(2..3, None).unwrap();
    assert_eq!(sheet.column_width(2), sheet.default_column_width());
    assert!(sheet.is_column_hidden(2));
    assert_eq!(sheet.column_width(3), 20.0);
    sheet.set_row_height(0..3, Some(33.0)).unwrap();
    sheet.set_rows_hidden(1..2, true).unwrap();
    assert_eq!(sheet.row_height(2), 33.0);
    assert!(sheet.is_row_hidden(1));
    sheet.set_row_height(0..3, None).unwrap();
    sheet.set_rows_hidden(0..3, false).unwrap();
    assert_eq!(sheet.row_height(1), sheet.default_row_height());
    assert!(!sheet.is_row_hidden(1));
    // What is written reads back.
    let mut workbook = Workbook::new();
    workbook.insert_sheet(sheet).unwrap();
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    let sheet = reopened.sheet("Layout").unwrap();
    assert_eq!(sheet.column_width(3), 20.0);
    assert!(sheet.is_column_hidden(2));
    for (outcome, expected) in [
        (
            sheet.clone().set_column_width(0..1, Some(256.0)),
            "invalid record value at Layout!col: expected a column width from 0 to 255, got 256",
        ),
        (
            sheet.clone().set_row_height(0..1, Some(-1.0)),
            "invalid record value at Layout!row: expected a row height from 0 to 409 points, got -1",
        ),
        (
            sheet.clone().set_columns_hidden(5..5, true),
            "invalid record value at Layout!col: expected columns from 1 to 16384, first before last, got 6 to 5",
        ),
        (
            sheet.clone().set_rows_hidden(0..1_048_577, true),
            "invalid record value at Layout!row: expected rows from 1 to 1048576, first before last, got 1 to 1048577",
        ),
    ] {
        assert_eq!(outcome.unwrap_err().to_string(), expected);
    }
}

#[test]
fn a_merge_keeps_its_first_cell_and_refuses_one_cell_or_an_overlap() {
    let mut sheet = sheet_of(
        "Merges",
        &[
            ("A1", Scalar::from("keep")),
            ("B1", Scalar::from("go")),
            ("B2", Scalar::from("go too")),
        ],
    );
    let cleared = sheet.merge(range("A1:B2")).unwrap();
    assert_eq!(references(cleared.iter()), ["B1", "B2"]);
    assert_eq!(references(sheet.cells()), ["A1"]);
    assert_eq!(
        sheet.merge(range("B2:C3")).unwrap_err().to_string(),
        "invalid record value at Merges!B2:C3: expected a range overlapping no merge, got one \
         overlapping A1:B2"
    );
    assert_eq!(
        sheet.merge(range("D4")).unwrap_err().to_string(),
        "invalid record value at Merges!mergeCell: expected a range of more than one cell to \
         merge, got D4"
    );
    sheet.merge(range("D4:E4")).unwrap();
    // Unmerging takes apart every merge the range meets.
    assert!(sheet.unmerge(range("B2:D4")));
    assert_eq!(sheet.merges().count(), 0);
    assert!(!sheet.unmerge(range("A1")));
}

#[test]
fn ctrl_and_an_arrow_run_to_the_edge_of_a_block_and_ctrl_a_takes_its_region() {
    use yggdryl::excel::Direction;
    let mut sheet = Sheet::new("Nav").unwrap();
    for reference in ["B2", "C2", "D2", "B3", "B4", "F2", "B9"] {
        sheet.set_cell(at(reference), 1.0).unwrap();
    }
    // A styled blank is no content.
    sheet
        .insert_cell(
            Cell::new(
                at("E2"),
                CellKind::Number,
                NumberFormat::General,
                Scalar::Null,
            )
            .with_style(StyleId::new(1)),
        )
        .unwrap();
    for (from, direction, to) in [
        ("B2", Direction::Right, "D2"),
        ("D2", Direction::Right, "F2"),
        ("F2", Direction::Right, "XFD2"),
        ("F2", Direction::Left, "D2"),
        ("B2", Direction::Down, "B4"),
        ("B4", Direction::Down, "B9"),
        ("B9", Direction::Up, "B4"),
        ("B1", Direction::Up, "B1"),
        ("A1", Direction::Left, "A1"),
        ("A7", Direction::Right, "XFD7"),
        ("Z1", Direction::Down, "Z1048576"),
    ] {
        assert_eq!(
            sheet.edge(at(from), direction),
            at(to),
            "{from} {direction:?}"
        );
    }
    assert_eq!(sheet.current_region(at("C3")), range("B2:D4"));
    assert_eq!(sheet.current_region(at("B9")), range("B9"));
    assert_eq!(sheet.current_region(at("H20")), range("H20"));
}

#[test]
fn flat_write_serie_late_value_refusal_preserves_cells_and_revision() {
    let field = record([
        DataType::utf8().required_field("Label"),
        DataType::Float64.required_field("Amount"),
    ]);
    let long = "x".repeat(MAX_CELL_TEXT + 1);
    let serie = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::from("valid"), Scalar::from(1.0)]),
            Scalar::from_sequence([Scalar::from(long), Scalar::from(2.0)]),
        ],
    )
    .unwrap();
    for header in [RecordHeader::Source, RecordHeader::None] {
        let mut sheet = Sheet::new("Sheet1").unwrap();
        sheet.set_cell(at("Z9"), "keep").unwrap();
        let before = sheet.clone();
        let revision = sheet.revision();
        let error = sheet
            .write_serie(at("A1"), &serie, header)
            .unwrap_err()
            .to_string();
        assert!(error.contains("32767") || error.contains("cell"), "{error}");
        assert_eq!(sheet, before, "{header:?}");
        assert_eq!(sheet.revision(), revision, "{header:?}");
    }
}

#[test]
fn flat_extend_from_serie_late_refusal_preserves_sheet_and_revision() {
    let field = record([DataType::utf8().required_field("Label")]);
    let long = "x".repeat(MAX_CELL_TEXT + 1);
    let serie = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::from("valid")]),
            Scalar::from_sequence([Scalar::from(long)]),
        ],
    )
    .unwrap();
    let mut sheet = Sheet::new("Sheet1").unwrap();
    sheet.set_cell(at("A1"), "existing").unwrap();
    let before = sheet.clone();
    let revision = sheet.revision();
    assert!(sheet.extend_from_serie(&serie).is_err());
    assert_eq!(sheet, before);
    assert_eq!(sheet.revision(), revision);
}

#[test]
fn flat_write_serie_failed_on_clean_workbook_does_not_mark_it_dirty() {
    let field = record([DataType::utf8().required_field("Label")]);
    let long = "x".repeat(MAX_CELL_TEXT + 1);
    let serie = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::from("valid")]),
            Scalar::from_sequence([Scalar::from(long)]),
        ],
    )
    .unwrap();
    let mut built = Workbook::new();
    built.add_sheet("Sheet1").unwrap();
    let bytes = built.into_bytes().unwrap();
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    assert!(!opened.is_dirty());
    let revision = opened.sheet("Sheet1").unwrap().revision();
    assert!(
        opened
            .sheet_mut("Sheet1")
            .unwrap()
            .write_serie(at("A1"), &serie, RecordHeader::Source)
            .is_err()
    );
    assert_eq!(opened.sheet("Sheet1").unwrap().revision(), revision);
    assert!(!opened.is_dirty());
}

#[test]
fn flat_write_serie_grid_end_refusal_is_atomic() {
    let field = record([DataType::Float64.required_field("Amount")]);
    let serie = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::from(1.0)]),
            Scalar::from_sequence([Scalar::from(2.0)]),
        ],
    )
    .unwrap();
    let mut sheet = Sheet::new("Sheet1").unwrap();
    let before = sheet.clone();
    assert!(
        sheet
            .write_serie(CellRef::new(MAX_ROWS - 1, 0), &serie, RecordHeader::None)
            .is_err()
    );
    assert_eq!(sheet, before);
    assert_eq!(sheet.revision(), before.revision());
}

#[test]
fn flat_records_preserve_trailing_all_null_rows_in_model_and_workbook() {
    let field = record([DataType::Float64.nullable_field("Amount")]);
    let serie = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_sequence([Scalar::from(1.0)]),
            Scalar::from_sequence([Scalar::Null]),
            Scalar::from_sequence([Scalar::Null]),
        ],
    )
    .unwrap();
    for header in [RecordHeader::Source, RecordHeader::None] {
        let sheet = Sheet::from_serie("Book", &serie, header).unwrap();
        let model = sheet
            .clone()
            .into_serie(Some(&field), header, ArrowCastOptions::default())
            .unwrap();
        assert_eq!(model.len(), 3, "model {header:?}");
        assert_eq!(json_rows(&model), json_rows(&serie), "model {header:?}");
        let mut book = Workbook::new();
        book.insert_sheet(sheet).unwrap();
        let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
        let result = reopened
            .sheet("Book")
            .unwrap()
            .clone()
            .into_serie(Some(&field), header, ArrowCastOptions::default())
            .unwrap();
        assert_eq!(result.len(), 3, "workbook {header:?}");
        assert_eq!(json_rows(&result), json_rows(&serie), "workbook {header:?}");
    }
}

#[test]
fn flat_extend_appends_after_trailing_all_null_record_rows() {
    let field = record([DataType::Float64.nullable_field("Amount")]);
    let first = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_sequence([Scalar::from(1.0)]),
            Scalar::from_sequence([Scalar::Null]),
            Scalar::from_sequence([Scalar::Null]),
        ],
    )
    .unwrap();
    let next =
        Serie::from_scalars(field.clone(), [Scalar::from_sequence([Scalar::from(2.0)])]).unwrap();
    let mut sheet = Sheet::from_serie("Book", &first, RecordHeader::Source).unwrap();
    sheet.extend_from_serie(&next).unwrap();
    assert_eq!(sheet.scalar(at("A5")), Scalar::from(2.0));
    let model = sheet
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::Source,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(model.len(), 4);
    assert_eq!(
        json_rows(&model),
        vec!["[1.0]", "[null]", "[null]", "[2.0]"]
    );
}

#[test]
fn flat_declared_all_null_unheaded_records_keep_cardinality_without_cells() {
    let field = record([DataType::Float64.nullable_field("Amount")]);
    let serie = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_sequence([Scalar::Null]),
            Scalar::from_sequence([Scalar::Null]),
        ],
    )
    .unwrap();
    let sheet = Sheet::from_serie("Book", &serie, RecordHeader::None).unwrap();
    assert!(sheet.dimension().is_none()); // A cell-only fact, not a record extent.
    let model = sheet
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::None,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(model.len(), 2);
    assert_eq!(json_rows(&model), json_rows(&serie));
    let mut book = Workbook::new();
    book.insert_sheet(sheet).unwrap();
    let reopened = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    let result = reopened
        .sheet("Book")
        .unwrap()
        .clone()
        .into_serie(
            Some(&field),
            RecordHeader::None,
            ArrowCastOptions::default(),
        )
        .unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(json_rows(&result), json_rows(&serie));
}

#[test]
fn infer_direct_sheet_write_refuses_with_explicit_policy_choices() {
    let mut sheet = Sheet::new("Sheet1").unwrap();
    let error = sheet
        .write_serie(at("A1"), &id_symbol(), RecordHeader::Infer)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.header")
            && error.contains("read-only")
            && error.contains("Source")
            && error.contains("Rows(n)"),
        "{error}"
    );
    assert_eq!(sheet.cell_count(), 0);
}

#[test]
fn negative_zero_temporal_serial_keeps_its_bit_on_held_write() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>-0</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("B1"), 7.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    assert!(part.contains("<c r=\"A1\" s=\"1\"><v>-0</v></c>"), "{part}");
}

#[test]
fn exceptional_temporal_serial_survives_a_projection_past_the_last_day() {
    use crate::excel_package::{
        content_types, package, root_relationships, styles, workbook, workbook_relationships,
        worksheet,
    };
    for (system, text) in [
        (DateSystem::Year1900, "2958465.9999999995"),
        (DateSystem::Year1904, "2957003.9999999995"),
    ] {
        let raw: f64 = text.parse().unwrap();
        let projected = system
            .scalar_from_serial(raw, NumberFormat::DateTime)
            .unwrap();
        assert!(
            system.serial_of(&projected).is_err(),
            "the valid serial rounds past the calendar's last day"
        );
        let source = package(&[
            ("[Content_Types].xml", content_types(1, false, true)),
            ("_rels/.rels", root_relationships()),
            (
                "xl/workbook.xml",
                workbook(&["Sheet1"], system == DateSystem::Year1904),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                workbook_relationships(1, false, true),
            ),
            ("xl/styles.xml", styles(&[], &[0, 22])),
            (
                "xl/worksheets/sheet1.xml",
                worksheet(&format!(
                    "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>{text}</v></c></row>"
                )),
            ),
        ]);
        let mut book = Workbook::from_bytes(source).unwrap();
        assert_eq!(book.sheet("Sheet1").unwrap().scalar(at("A1")), projected);
        book.sheet_mut("Sheet1")
            .unwrap()
            .set_cell(at("B1"), 7.0)
            .unwrap();
        let saved = book.into_bytes().unwrap();
        let xml = part_text(&saved, "xl/worksheets/sheet1.xml");
        let cell = xml
            .split_once("<c r=\"A1\"")
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0;
        let value: f64 = cell
            .split_once("<v>")
            .unwrap()
            .1
            .split_once("</v>")
            .unwrap()
            .0
            .parse()
            .unwrap();
        assert_eq!(value.to_bits(), raw.to_bits(), "{system}: {xml}");
        let reopened = Workbook::from_bytes(saved).unwrap();
        assert_eq!(
            reopened.sheet("Sheet1").unwrap().scalar(at("A1")),
            projected
        );
    }
}

#[test]
fn exceptional_serial_does_not_outlive_a_mutated_cell_guard() {
    let source = || {
        crate::excel_package::one_sheet(
            "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>60</v></c></row>",
            &[],
            &[],
            &[0, 14, 22],
        )
    };
    for forget in [false, true] {
        let mut workbook = Workbook::from_bytes(source()).unwrap();
        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        let mut cell = sheet.cell_mut(at("A1")).unwrap();
        *cell = Cell::from_scalar(at("A1"), Scalar::from(61.0), DateSystem::Year1900).unwrap();
        if forget {
            std::mem::forget(cell);
        } else {
            drop(cell);
        }
        let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
        assert!(
            part.contains("<c r=\"A1\"><v>61</v></c>"),
            "forgotten={forget}: {part}"
        );
    }
}

#[test]
fn exceptional_serial_survives_a_style_only_guard_and_clears_on_epoch_change() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>60</v></c>\
         <c r=\"B1\" s=\"2\"><v>45292.000000001</v></c></row>",
        &[],
        &[],
        &[0, 14, 22],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    let modern = workbook.sheet("Sheet1").unwrap().scalar(at("B1"));
    {
        let sheet = workbook.sheet_mut("Sheet1").unwrap();
        let mut cell = sheet.cell_mut(at("A1")).unwrap();
        *cell = cell.clone().with_style(StyleId::new(2));
    }
    let same_epoch = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    let a1 = same_epoch
        .split_once("<c r=\"A1\"")
        .unwrap()
        .1
        .split_once("</c>")
        .unwrap()
        .0;
    assert!(a1.contains("<v>60</v>"), "{same_epoch}");
    // The 1900 phantom date is outside the 1904 calendar; the modern cell
    // below isolates epoch rebasing from that independent refusal.
    workbook.sheet_mut("Sheet1").unwrap().remove_cell(at("A1"));
    workbook.set_date_system(DateSystem::Year1904);
    let expected = DateSystem::Year1904.serial_of(&modern).unwrap().unwrap().0;
    let changed = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    let b1 = changed
        .split_once("<c r=\"B1\"")
        .unwrap()
        .1
        .split_once("</c>")
        .unwrap()
        .0;
    let written: f64 = b1
        .split_once("<v>")
        .unwrap()
        .1
        .split_once("</v>")
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert_eq!(written.to_bits(), expected.to_bits(), "{changed}");
    assert_ne!(
        written.to_bits(),
        "45292.000000001".parse::<f64>().unwrap().to_bits()
    );
}

#[test]
fn builder_epoch_change_discards_old_exceptional_serial_bits() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>45292.000000001</v></c></row>",
        &[],
        &[],
        &[0, 14, 22],
    );
    let opened = Workbook::from_bytes(source).unwrap();
    let original = opened.sheet("Sheet1").unwrap();
    let expected = DateSystem::Year1904
        .serial_of(&original.scalar(at("A1")))
        .unwrap()
        .unwrap()
        .0;
    let sheet = original.clone().with_date_system(DateSystem::Year1904);
    let mut target = Workbook::new();
    target.set_date_system(DateSystem::Year1904);
    target.insert_sheet(sheet).unwrap();
    let part = part_text(&target.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    let a1 = part
        .split_once("<c r=\"A1\"")
        .unwrap()
        .1
        .split_once("</c>")
        .unwrap()
        .0;
    let actual: f64 = a1
        .split_once("<v>")
        .unwrap()
        .1
        .split_once("</v>")
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert_eq!(actual.to_bits(), expected.to_bits(), "{part}");
}

#[test]
fn fractional_date_and_whole_day_time_formats_keep_canonical_numeric_serials() {
    let source = crate::excel_package::one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>1.5</v></c>\
         <c r=\"B1\" s=\"3\"><v>45292.25</v></c></row>",
        &[],
        &[],
        &[0, 14, 22, 21],
    );
    let mut workbook = Workbook::from_bytes(source).unwrap();
    assert!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .cell(at("A1"))
            .unwrap()
            .value()
            .temporal_unit()
            .is_some()
    );
    assert!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .cell(at("B1"))
            .unwrap()
            .value()
            .temporal_unit()
            .is_some()
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("C1"), 1.0)
        .unwrap();
    let part = part_text(&workbook.into_bytes().unwrap(), "xl/worksheets/sheet1.xml");
    for (reference, expected) in [("A1", 1.5_f64), ("B1", 45292.25_f64)] {
        let cell = part
            .split_once(&format!("<c r=\"{reference}\""))
            .unwrap()
            .1
            .split_once("</c>")
            .unwrap()
            .0;
        let actual: f64 = cell
            .split_once("<v>")
            .unwrap()
            .1
            .split_once("</v>")
            .unwrap()
            .0
            .parse()
            .unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits(), "{reference}: {part}");
    }
}

#[test]
fn criteria_typed_text_cache_is_derived_and_does_not_change_sheet_equality() {
    let mut original=Sheet::new("Data").unwrap();
    original.set_cell(at("A1"),Scalar::from_sequence([Scalar::from("same"),Scalar::from(17_i64)])).unwrap();
    let mut other=original.clone();
    let mut guard=other.cell_mut(at("A1")).unwrap();
    *guard=guard.clone();std::mem::forget(guard);
    assert_eq!(original,other,"invalidating a derived spelling preserves source equality");
}

#[test]
fn typed_text_write_reuses_derived_spelling_without_changing_bytes() {
    use crate::excel_package::typed_text_write_cost_book;
    for rows in [1,64] {
        let typed=typed_text_write_cost_book(rows,true);let text=typed_text_write_cost_book(rows,false);
        let typed_bytes=typed.into_bytes().unwrap();let text_bytes=text.into_bytes().unwrap();
        assert_eq!(typed_bytes,text_bytes,"the cached spelling is a derived fact");
        let reopened=Workbook::from_bytes(typed_bytes).unwrap();
        for row in 0..rows {
            assert_eq!(reopened.sheet("Data").unwrap().scalar(CellRef::new(row,0)),text.sheet("Data").unwrap().scalar(CellRef::new(row,0)));
        }
    }
}
