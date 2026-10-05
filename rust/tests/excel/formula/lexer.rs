//! `rust/src/excel/formula/lexer.rs`: the total tokenizer - a reference told from a name, a function and a number by the characters around it, and every byte of any text carried.

use yggdryl::excel::{CellRef, Formula};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

/// `text` held at `A1`, rendered one row down and one column right: every
/// piece read as a reference moves, every other piece stays as written.
fn moved(text: &str) -> String {
    Formula::from_file(text, at("A1")).at(at("B2")).to_string()
}

#[test]
fn cells_areas_rows_and_columns_are_references_and_move() {
    for (text, expected) in [
        ("A1", "B2"),
        ("a1", "B2"),
        ("$A$1", "$A$1"),
        ("A$1", "B$1"),
        ("$A1", "$A2"),
        ("A1:C3", "B2:D4"),
        ("A:C", "B:D"),
        ("$A:$C", "$A:$C"),
        ("1:3", "2:4"),
        ("$1:$3", "$1:$3"),
        ("XFD1048576", "#REF!"),
        ("Sheet1!A1", "Sheet1!B2"),
        ("Sheet1!$A$1:B2", "Sheet1!$A$1:C3"),
        ("'Q1 ''24'!A1", "'Q1 ''24'!B2"),
        ("Jan:Mar!A1", "Jan:Mar!B2"),
        ("'Jan 1:Mar 1'!A1", "'Jan 1:Mar 1'!B2"),
        ("[1]Sheet1!A1", "[1]Sheet1!B2"),
        ("'[Book.xlsx]Q 1'!A1", "'[Book.xlsx]Q 1'!B2"),
        ("#REF!A1", "#REF!B2"),
        ("Sheet1!#REF!", "Sheet1!#REF!"),
    ] {
        assert_eq!(moved(text), expected, "{text}");
    }
}

#[test]
fn what_is_no_reference_stays_as_written() {
    for text in [
        // Called, so a function, although `LOG10` spells a cell.
        "LOG10(100)",
        "SUM(1)",
        // Names: past the grid, too many letters, or letters and digits mixed.
        "XFE1",
        "Revenue",
        "A1B",
        "R1C1",
        "Sheet1!Print_Area",
        "[1]!Rate",
        // Literals, whose digits and letters are no reference.
        "1.5E+3",
        ".5",
        "\"A1\"",
        "TRUE",
        "{1,2;3,4}",
        "#N/A",
        "#DIV/0!",
        // Structured references and what the grammar cannot read.
        "Table1[Price]",
        "[@Price]",
        "Table1[[#This Row],[Price]]",
        "~ x",
        "'unclosed",
    ] {
        assert_eq!(moved(text), text, "{text}");
    }
}

#[test]
fn a_spill_follows_the_reference_it_spills_and_is_held() {
    assert_eq!(moved("SUM(A1#)"), "SUM(B2#)");
    assert!(!Formula::from_file("SUM(A1#)", at("A1")).is_computed());
    // A hash opening an error literal is no spill.
    assert!(Formula::from_file("IFERROR(A1,#N/A)", at("A1")).is_computed());
}

#[test]
fn every_byte_of_any_text_is_kept() {
    for text in [
        "\u{a0}A1\u{a0}",
        "A1\r\n+B1",
        "\"a\"\"b\"",
        "SUM(A1,,B1)",
        "=1",
        "1:",
        "$",
        "é+ü",
        "'Q'",
        "Sheet1!Größe",
        "'Q1'!Größe*2",
        "Sheet1!abcdé",
        "Лист1!Итого",
        "Sheet1!名前",
        "Sheet1!Gé",
    ] {
        assert_eq!(
            Formula::from_file(text, at("A1")).at(at("A1")).to_string(),
            text,
            "{text:?}"
        );
    }
}

#[test]
fn a_name_past_a_sheet_prefix_may_be_any_text() {
    // Each splits a multi-byte character where `#REF!` would end.
    for (text, name) in [
        ("Sheet1!Größe*2", "Sheet1!Größe"),
        ("'Q1'!Größe*2", "'Q1'!Größe"),
        ("Лист1!Итого+1", "Лист1!Итого"),
        ("Sheet1!名前", "Sheet1!名前"),
    ] {
        let formula = Formula::from_file(text, at("A1"));
        assert_eq!(formula.at(at("C9")).to_string(), text, "{text}");
        let typed = Formula::from_entry(text, at("A1")).unwrap();
        assert!(typed.at(at("A1")).to_string().starts_with(name), "{text}");
    }
}

#[test]
fn a_range_beside_a_qualified_reference_is_no_span_of_sheets() {
    // Excel quotes a sheet named like a reference, so an unquoted side
    // reading as one is the range operator's operand.
    for (text, expected) in [
        ("A1:Sheet1!A1", "B2:Sheet1!B2"),
        ("C:C:Sheet1!1:1", "D:D:Sheet1!2:2"),
        ("R1C1:Sheet1!A:A", "R1C1:Sheet1!B:B"),
        ("Sheet1!A1:Sheet1!B2", "Sheet1!B2:Sheet1!C3"),
        // Two plain names are a span of sheets.
        ("Jan:Mar!B2", "Jan:Mar!C3"),
    ] {
        assert_eq!(moved(text), expected, "{text}");
        assert_eq!(
            Formula::from_file(text, at("A1")).at(at("A1")).to_string(),
            text,
            "{text}"
        );
    }
}
