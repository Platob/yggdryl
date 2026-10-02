//! `rust/src/excel/fill.rs`: `Workbook::fill` - the fill handle: a series each line of the source spells (numbers, dates, times, counted text, day and month names, quarters) or copies of it, formulas translated, in any of four directions.

use yggdryl::excel::{Cell, CellRef, DateSystem, FillMode, Formula, StylePatch, Workbook};
use yggdryl::{Error, Scalar};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

/// A workbook whose `Sheet1` holds `cells`, each typed as text.
fn typed(cells: &[(&str, &str)]) -> Workbook {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet1").unwrap();
    for (cell, text) in cells {
        workbook.set_entry("Sheet1", at(cell), text).unwrap();
    }
    workbook
}

/// What each cell of `cells` shows.
fn shown(workbook: &Workbook, cells: &[&str]) -> Vec<String> {
    cells
        .iter()
        .map(|cell| {
            workbook
                .display_text("Sheet1", at(cell))
                .unwrap()
                .map_or_else(String::new, |shown| shown.text.to_string())
        })
        .collect()
}

fn fill(workbook: &mut Workbook, source: &str, target: &str, mode: FillMode) {
    workbook
        .fill(
            "Sheet1",
            source.parse().unwrap(),
            target.parse().unwrap(),
            mode,
        )
        .unwrap();
}

#[test]
fn numbers_step_linearly_and_one_number_copies() {
    let mut workbook = typed(&[
        ("A1", "1"),
        ("A2", "3"),
        ("B1", "7"),
        ("C1", "1"),
        ("C2", "2"),
        ("C3", "4"),
        ("D1", "0.1"),
        ("D2", "0.2"),
    ]);
    fill(&mut workbook, "A1:D1", "A1:D1", FillMode::Series);
    fill(&mut workbook, "A1:A2", "A1:A5", FillMode::Series);
    fill(&mut workbook, "B1", "B1:B3", FillMode::Series);
    fill(&mut workbook, "C1:C3", "C1:C5", FillMode::Series);
    fill(&mut workbook, "D1:D2", "D1:D4", FillMode::Series);
    assert_eq!(shown(&workbook, &["A3", "A4", "A5"]), ["5", "7", "9"]);
    assert_eq!(shown(&workbook, &["B2", "B3"]), ["7", "7"]);
    // Steps that are no series follow the least-squares line through them.
    assert_eq!(
        shown(&workbook, &["C4", "C5"]),
        ["5.333333333", "6.833333333"]
    );
    // Within fifteen digits, as Excel keeps a number.
    assert_eq!(shown(&workbook, &["D3", "D4"]), ["0.3", "0.4"]);
}

#[test]
fn dates_step_by_the_day_the_month_or_the_year_their_spelling_shows() {
    let mut workbook = typed(&[
        ("A1", "1/30/2024"),
        ("B1", "10/31/2023"),
        ("B2", "12/31/2023"),
        ("E1", "1/31/2024"),
        ("E2", "2/29/2024"),
        ("C1", "3/15/2020"),
        ("C2", "3/15/2021"),
        ("D1", "1/1/2024"),
        ("D2", "1/8/2024"),
    ]);
    fill(&mut workbook, "A1", "A1:A3", FillMode::Series);
    fill(&mut workbook, "B1:B2", "B1:B4", FillMode::Series);
    fill(&mut workbook, "C1:C2", "C1:C3", FillMode::Series);
    fill(&mut workbook, "D1:D2", "D1:D3", FillMode::Series);
    fill(&mut workbook, "E1:E2", "E1:E3", FillMode::Series);
    assert_eq!(shown(&workbook, &["A2", "A3"]), ["1/31/2024", "2/1/2024"]);
    // By the month the day stays - or the month's last day where it has no
    // such day.
    assert_eq!(shown(&workbook, &["B3", "B4"]), ["2/29/2024", "4/30/2024"]);
    // Two days of the month apart step by their days.
    assert_eq!(shown(&workbook, &["E3"]), ["3/29/2024"]);
    assert_eq!(shown(&workbook, &["C3"]), ["3/15/2022"]);
    assert_eq!(shown(&workbook, &["D3"]), ["1/15/2024"]);
}

#[test]
fn times_counted_text_names_and_quarters_follow_their_series() {
    let mut workbook = typed(&[
        ("A1", "9:00"),
        ("B1", "Item 7"),
        ("C1", "Batch 008"),
        ("C2", "Batch 010"),
        ("D1", "Mon"),
        ("E1", "JANUARY"),
        ("F1", "Q3"),
        ("G1", "Qtr 1"),
        ("H1", "sat"),
        ("H2", "mon"),
    ]);
    fill(&mut workbook, "A1:B1", "A1:B4", FillMode::Series);
    fill(&mut workbook, "D1:F1", "D1:F4", FillMode::Series);
    fill(&mut workbook, "C1:C2", "C1:C4", FillMode::Series);
    fill(&mut workbook, "H1:H2", "H1:H4", FillMode::Series);
    assert_eq!(shown(&workbook, &["A2", "A3"]), ["10:00", "11:00"]);
    assert_eq!(shown(&workbook, &["B2", "B4"]), ["Item 8", "Item 10"]);
    // Zero padding kept, the step the two cells show.
    assert_eq!(shown(&workbook, &["C3", "C4"]), ["Batch 012", "Batch 014"]);
    assert_eq!(shown(&workbook, &["D2", "D4"]), ["Tue", "Thu"]);
    assert_eq!(shown(&workbook, &["E2", "E3"]), ["FEBRUARY", "MARCH"]);
    assert_eq!(shown(&workbook, &["F2", "F3", "F4"]), ["Q4", "Q1", "Q2"]);
    // A list steps by the step its cells show, round the week.
    assert_eq!(shown(&workbook, &["H3", "H4"]), ["wed", "fri"]);
    // `Qtr 1` is no quarter spelling but text counting on.
    fill(&mut workbook, "G1", "G1:G2", FillMode::Series);
    assert_eq!(shown(&workbook, &["G2"]), ["Qtr 2"]);
}

#[test]
fn a_fill_up_or_left_continues_the_series_backward() {
    let mut workbook = typed(&[("C5", "5"), ("C6", "6"), ("E1", "Wed"), ("F1", "Thu")]);
    fill(&mut workbook, "C5:C6", "C2:C6", FillMode::Series);
    fill(&mut workbook, "E1:F1", "B1:F1", FillMode::Series);
    assert_eq!(shown(&workbook, &["C2", "C3", "C4"]), ["2", "3", "4"]);
    assert_eq!(shown(&workbook, &["B1", "C1", "D1"]), ["Sun", "Mon", "Tue"]);
}

#[test]
fn copies_repeat_the_source_each_formula_translated_each_style_kept() {
    let mut workbook = typed(&[("A1", "1"), ("A2", "x"), ("B1", "=A1*2")]);
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &["A2".parse().unwrap()], &bold)
        .unwrap();
    fill(&mut workbook, "A1:A2", "A1:A5", FillMode::Copy);
    fill(&mut workbook, "B1", "B1:B3", FillMode::Series);
    assert_eq!(shown(&workbook, &["A3", "A4", "A5"]), ["1", "x", "1"]);
    assert!(workbook.cell_style("Sheet1", at("A4")).unwrap().font.bold);
    assert_eq!(
        workbook.entry_text("Sheet1", at("B3")).unwrap().as_deref(),
        Some("=A3*2")
    );
    // A reference a copy carries off the grid is `#REF!` for good.
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    let host = at("D2");
    sheet
        .insert_cell(
            Cell::from_scalar(host, 0.0.into(), DateSystem::Year1900)
                .unwrap()
                .with_formula(Formula::from_file("D1", host)),
        )
        .unwrap();
    workbook
        .fill(
            "Sheet1",
            "D2".parse().unwrap(),
            "D1:D2".parse().unwrap(),
            FillMode::Copy,
        )
        .unwrap();
    assert_eq!(
        workbook.entry_text("Sheet1", at("D1")).unwrap().as_deref(),
        Some("=#REF!")
    );
    // A blank source cell clears the cells it continues to.
    let mut workbook = typed(&[("A1", "1"), ("A3", "3")]);
    fill(&mut workbook, "A1:A2", "A1:A4", FillMode::Copy);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("A4")),
        Scalar::Null
    );
    assert!(workbook.sheet("Sheet1").unwrap().cell(at("A3")).is_some());
    fill(&mut workbook, "A1:A2", "A1:A4", FillMode::Copy);
}

#[test]
fn a_target_not_extending_the_source_one_way_is_refused() {
    let mut workbook = typed(&[("B2", "1")]);
    for target in ["B2:C3", "A1:B2", "C2:C4", "B3:B5"] {
        let error = workbook
            .fill(
                "Sheet1",
                "B2".parse().unwrap(),
                target.parse().unwrap(),
                FillMode::Series,
            )
            .unwrap_err();
        assert!(
            matches!(error, Error::InvalidRecord { .. }),
            "{target}: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("expected a target extending the source B2 down, up, right or left"),
            "{error}"
        );
    }
    // The source itself fills nothing.
    workbook
        .fill(
            "Sheet1",
            "B2".parse().unwrap(),
            "B2".parse().unwrap(),
            FillMode::Series,
        )
        .unwrap();
}

#[test]
fn a_fill_past_the_cell_bound_is_refused_before_a_cell_is_built() {
    let mut workbook = typed(&[("A1", "1"), ("B1", "2"), ("C1", "3")]);
    // Three lines of 699,051 cells past the source: one past the bound.
    let error = workbook
        .fill(
            "Sheet1",
            "A1:C1".parse().unwrap(),
            "A1:C699052".parse().unwrap(),
            FillMode::Copy,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at Sheet1!A1:C699052: expected a fill of at most 2097152 cells \
         past its source, got 2097153"
    );
    // The whole grid as a target: refused as fast, never allocated.
    assert!(
        workbook
            .fill(
                "Sheet1",
                "A1:XFD1".parse().unwrap(),
                "A1:XFD1048576".parse().unwrap(),
                FillMode::Series,
            )
            .is_err()
    );
    assert_eq!(shown(&workbook, &["A2", "C2"]), ["", ""]);
}

#[test]
fn counted_text_stops_where_its_number_no_longer_counts_and_counts_down_past_zero_up_again() {
    // A step of fifteen digits reaches past what a count holds.
    let mut workbook = typed(&[("A1", "x999999999999999"), ("A2", "x1")]);
    let error = workbook
        .fill(
            "Sheet1",
            "A1:A2".parse().unwrap(),
            "A1:A30000".parse().unwrap(),
            FillMode::Series,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("expected a series value the grid can hold, got one past it"),
        "{error}"
    );
    assert_eq!(shown(&workbook, &["A3"]), [""]);
    // The digits spell no sign: counted down past zero they count up
    // again, as Excel counts them, the padding kept.
    let mut workbook = typed(&[("B4", "Item 01")]);
    fill(&mut workbook, "B4", "B1:B4", FillMode::Series);
    assert_eq!(
        shown(&workbook, &["B1", "B2", "B3"]),
        ["Item 02", "Item 01", "Item 00"]
    );
}
