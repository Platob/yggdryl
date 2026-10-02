//! `rust/src/excel/find.rs`: `Workbook::find` and `Workbook::replace` - Find Next's order and wrap, one worksheet or every visible one, values or formulas, case and whole cells, Excel's wildcards, and Replace All over exactly the cells Find Next stops on.

use yggdryl::Error;
use yggdryl::excel::{CellRef, FindOptions, FindScope, SheetState, Within, Workbook};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

fn book() -> Workbook {
    let mut workbook = Workbook::new();
    workbook.add_sheet("One").unwrap();
    workbook.add_sheet("Two").unwrap();
    workbook.add_sheet("Hidden").unwrap();
    workbook.add_sheet("Three").unwrap();
    for (sheet, cell, text) in [
        ("One", "B2", "apple"),
        ("One", "A5", "Apple pie"),
        ("One", "C1", "12%"),
        ("One", "D1", "=C1*2"),
        ("Two", "A1", "pineapple"),
        ("Hidden", "A1", "apple"),
        ("Three", "E9", "APPLE"),
    ] {
        workbook.set_entry(sheet, at(cell), text).unwrap();
    }
    workbook
        .set_sheet_state("Hidden", SheetState::Hidden)
        .unwrap();
    workbook
}

fn options(text: &str, sheet: &str) -> FindOptions {
    FindOptions {
        text: text.into(),
        sheet: sheet.into(),
        ..FindOptions::default()
    }
}

/// Where Find Next goes from `from`.
fn next(workbook: &Workbook, options: &FindOptions, from: Option<&str>) -> Option<String> {
    workbook
        .find(options, from.map(at))
        .unwrap()
        .map(|(sheet, cell)| format!("{sheet}!{cell}"))
}

#[test]
fn find_next_goes_by_rows_from_the_cell_after_and_wraps_round() {
    let workbook = book();
    let sheet = options("apple", "One");
    // Row 2 before row 5; from nowhere, A1 on.
    assert_eq!(next(&workbook, &sheet, None).as_deref(), Some("One!B2"));
    assert_eq!(
        next(&workbook, &sheet, Some("B2")).as_deref(),
        Some("One!A5")
    );
    // Past the last match it wraps to the first.
    assert_eq!(
        next(&workbook, &sheet, Some("A5")).as_deref(),
        Some("One!B2")
    );
    // A lone match answers again.
    let lone = options("pie", "One");
    assert_eq!(
        next(&workbook, &lone, Some("A5")).as_deref(),
        Some("One!A5")
    );
    assert_eq!(next(&workbook, &options("kiwi", "One"), None), None);
}

#[test]
fn a_workbook_find_goes_on_through_the_visible_worksheets_and_round() {
    let workbook = book();
    let everywhere = FindOptions {
        scope: FindScope::Workbook,
        ..options("apple", "One")
    };
    assert_eq!(
        next(&workbook, &everywhere, Some("A5")).as_deref(),
        Some("Two!A1")
    );
    let from_two = FindOptions {
        sheet: "Two".into(),
        ..everywhere.clone()
    };
    // The hidden sheet is passed over; after the last sheet, the first.
    assert_eq!(
        next(&workbook, &from_two, Some("A1")).as_deref(),
        Some("Three!E9")
    );
    let from_three = FindOptions {
        sheet: "Three".into(),
        ..everywhere
    };
    assert_eq!(
        next(&workbook, &from_three, Some("E9")).as_deref(),
        Some("One!B2")
    );
}

#[test]
fn case_whole_cells_wildcards_and_the_text_read_decide_a_match() {
    let workbook = book();
    let case = FindOptions {
        match_case: true,
        scope: FindScope::Workbook,
        ..options("APPLE", "One")
    };
    assert_eq!(next(&workbook, &case, None).as_deref(), Some("Three!E9"));
    let whole = FindOptions {
        entire_cell: true,
        ..options("apple", "Two")
    };
    assert_eq!(next(&workbook, &whole, None), None);
    for (pattern, found) in [
        ("p?e", Some("One!B2")),
        ("p?e ", Some("One!A5")),
        ("a*e", Some("One!B2")),
        ("*pie", Some("One!A5")),
        ("~*", None),
        ("12~%", Some("One!C1")),
        ("12~?", None),
    ] {
        assert_eq!(
            next(&workbook, &options(pattern, "One"), None).as_deref(),
            found,
            "{pattern}"
        );
    }
    // Values read what a cell shows, formulas what typing makes it.
    assert_eq!(
        next(&workbook, &options("12%", "One"), None).as_deref(),
        Some("One!C1")
    );
    assert_eq!(
        next(&workbook, &options("24%", "One"), None).as_deref(),
        None
    );
    let formulas = FindOptions {
        within: Within::Formulas,
        ..options("=c1", "One")
    };
    assert_eq!(next(&workbook, &formulas, None).as_deref(), Some("One!D1"));
    // A cell a merge covers past its first matches nowhere.
    let mut merged = book();
    merged
        .sheet_mut("One")
        .unwrap()
        .set_cell(at("F1"), "apple")
        .unwrap();
    let covered = merged
        .sheet_mut("One")
        .unwrap()
        .merge("E1:F1".parse().unwrap());
    assert!(covered.is_ok());
    assert_eq!(
        next(&merged, &options("apple", "One"), None).as_deref(),
        Some("One!B2")
    );
}

#[test]
fn a_find_is_refused_for_no_text_or_no_sheet() {
    let workbook = book();
    assert!(matches!(
        workbook.find(&options("", "One"), None),
        Err(Error::InvalidRecord { .. })
    ));
    assert!(matches!(
        workbook.find(&options("x", "Missing"), None),
        Err(Error::Absent { .. })
    ));
}

#[test]
fn replace_all_changes_exactly_the_cells_find_next_stops_on() {
    let mut workbook = book();
    let everywhere = FindOptions {
        scope: FindScope::Workbook,
        within: Within::Formulas,
        ..options("apple", "One")
    };
    assert_eq!(workbook.replace(&everywhere, "pear").unwrap(), 4);
    let text = |workbook: &Workbook, sheet: &str, cell: &str| {
        workbook.entry_text(sheet, at(cell)).unwrap().unwrap()
    };
    assert_eq!(text(&workbook, "One", "B2"), "pear");
    assert_eq!(text(&workbook, "One", "A5"), "pear pie");
    assert_eq!(text(&workbook, "Two", "A1"), "pinepear");
    assert_eq!(text(&workbook, "Three", "E9"), "pear");
    // The hidden sheet holds its apple.
    assert_eq!(text(&workbook, "Hidden", "A1"), "apple");
    // A formula is typed again from its replaced text.
    let formulas = FindOptions {
        within: Within::Formulas,
        ..options("C1", "One")
    };
    assert_eq!(workbook.replace(&formulas, "B2").unwrap(), 1);
    assert_eq!(text(&workbook, "One", "D1"), "=B2*2");
}

#[test]
fn a_replace_refusing_one_cell_changes_none() {
    let mut workbook = book();
    let broken = FindOptions {
        within: Within::Formulas,
        ..options("~*2", "One")
    };
    let error = workbook.replace(&broken, ")").unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at One!D1: expected the replaced formula to read, got expected no \
         ) at byte 3"
    );
    assert_eq!(
        workbook.entry_text("One", at("D1")).unwrap().as_deref(),
        Some("=C1*2")
    );
    // Within values, a formula's result is no text to replace: the cell's
    // cached result shows, and replacing it is refused naming the cell.
    let host = at("D1");
    workbook
        .sheet_mut("One")
        .unwrap()
        .insert_cell(
            yggdryl::excel::Cell::from_scalar(
                host,
                0.24.into(),
                yggdryl::excel::DateSystem::Year1900,
            )
            .unwrap()
            .with_formula(yggdryl::excel::Formula::from_file("C1*2", host)),
        )
        .unwrap();
    let values = options("0.24", "One");
    assert_eq!(next(&workbook, &values, None).as_deref(), Some("One!D1"));
    let error = workbook.replace(&values, "1").unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("invalid record value at One!D1"),
        "{error}"
    );
    assert_eq!(
        workbook.entry_text("One", at("D1")).unwrap().as_deref(),
        Some("=C1*2")
    );
}

#[test]
fn a_replace_making_a_cell_too_long_changes_none_and_names_it() {
    let mut workbook = book();
    workbook.set_entry("One", at("A1"), "ab").unwrap();
    workbook.set_entry("One", at("A2"), "abab").unwrap();
    let long = "x".repeat(20_000);
    let error = workbook.replace(&options("ab", "One"), &long).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at One!A2: expected at most 32767 characters in a cell, got 40000"
    );
    assert_eq!(
        workbook.entry_text("One", at("A1")).unwrap().as_deref(),
        Some("ab")
    );
}

#[test]
fn a_wildcard_reads_a_long_cell_once_per_match() {
    // A pattern over the longest text a cell holds, matching nowhere and
    // everywhere: each costs the text's length, never its square.
    let mut workbook = Workbook::new();
    workbook.add_sheet("S").unwrap();
    workbook
        .set_entry("S", at("A1"), &"a".repeat(32_767))
        .unwrap();
    for (pattern, found) in [("a*b*c", false), ("?*a", true), ("*a?a*", true)] {
        let options = FindOptions {
            text: pattern.into(),
            sheet: "S".into(),
            ..FindOptions::default()
        };
        assert_eq!(
            next(&workbook, &options, None).is_some(),
            found,
            "{pattern}"
        );
    }
    let options = FindOptions {
        text: "a?".into(),
        sheet: "S".into(),
        within: Within::Formulas,
        ..FindOptions::default()
    };
    assert_eq!(workbook.replace(&options, "b").unwrap(), 1);
    assert_eq!(
        workbook
            .entry_text("S", at("A1"))
            .unwrap()
            .map(|text| text.len()),
        Some(16_384)
    );
}
