//! `rust/src/excel/formula/reference.rs`: references held relative to their host - rendered translated, off the grid as `#REF!`, a sheet name quoted only where Excel quotes it.

use yggdryl::excel::{CellRef, Formula};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

#[test]
fn a_relative_reference_is_the_distance_from_its_host() {
    let formula = Formula::from_file("B2", at("C3"));
    assert_eq!(formula.at(at("C3")).to_string(), "B2");
    assert_eq!(formula.at(at("D10")).to_string(), "C9");
    assert_eq!(formula.at(at("B3")).to_string(), "A2");
    // One column further left is off the grid.
    assert_eq!(formula.at(at("A3")).to_string(), "#REF!");
    let whole = Formula::from_file("B:B+2:2", at("C3"));
    assert_eq!(whole.at(at("A1")).to_string(), "#REF!+#REF!");
    assert_eq!(
        whole.at(at("XFD1048576")).to_string(),
        "XFC:XFC+1048575:1048575"
    );
}

#[test]
fn an_absolute_coordinate_stays_where_it_was_written() {
    let formula = Formula::from_file("$B$2+$B2+B$2", at("C3"));
    assert_eq!(formula.at(at("E9")).to_string(), "$B$2+$B8+D$2");
}

#[test]
fn a_sheet_name_is_quoted_where_excel_quotes_it_and_kept_quoted_as_the_file_quoted_it() {
    for (text, rendered) in [
        ("Data!A1", "Data!A1"),
        ("'Data'!A1", "'Data'!A1"),
        ("'My Data'!A1", "'My Data'!A1"),
        ("'It''s'!A1", "'It''s'!A1"),
        ("'2024'!A1", "'2024'!A1"),
        ("'A1'!A1", "'A1'!A1"),
        ("'R1C1'!A1", "'R1C1'!A1"),
        ("Données!A1", "Données!A1"),
    ] {
        assert_eq!(
            Formula::from_file(text, at("B1")).at(at("B1")).to_string(),
            rendered,
            "{text}"
        );
    }
    // Entry spells a reference as Excel normalizes it: quotes only where
    // the name needs them.
    for (typed, file) in [
        ("'Data'!a1", "Data!A1"),
        ("'my data'!a1", "'my data'!A1"),
        ("'TRUE'!A1", "'TRUE'!A1"),
        ("'Sheet.1'!A1", "Sheet.1!A1"),
    ] {
        assert_eq!(
            Formula::from_entry(typed, at("B1"))
                .unwrap()
                .at(at("B1"))
                .to_string(),
            file,
            "{typed}"
        );
    }
}

#[test]
fn a_sheet_name_is_quoted_exactly_where_excel_quotes_it() {
    // Entry spells a sheet as Excel normalizes it, so what it renders shows
    // the quoting rule itself: anything but letters, digits, `_` and `.`,
    // a name opening with a digit or a `.`, one reading as a reference -
    // `A1`, `R1C1`, `R`, `C` - and the two booleans.
    for (name, quoted) in [
        ("Sheet1", false),
        ("Q1_2024", false),
        ("Sheet.1", false),
        ("Données", false),
        ("My Sheet", true),
        ("It's", true),
        ("2024", true),
        (".hidden", true),
        ("A1", true),
        ("xfd1048576", true),
        ("XFE1", false),
        ("R", true),
        ("C", true),
        ("R1C1", true),
        ("rc", true),
        ("TRUE", true),
        ("a-b", true),
    ] {
        let typed = format!("'{}'!A1", name.replace('\'', "''"));
        let rendered = Formula::from_entry(&typed, at("B1"))
            .unwrap()
            .at(at("B1"))
            .to_string();
        assert_eq!(rendered.starts_with('\''), quoted, "{name:?}: {rendered}");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::excel::{CellRange, CellRef, Formula, MAX_COLUMNS, MAX_ROWS};
    use yggdryl::internals::excel_formula_reference::{range, relative};

    #[test]
    fn target_range_resolves_coordinates_once_and_preserves_whole_axis_bounds() {
        let authored: CellRef = "D4".parse().unwrap();
        let host: CellRef = "F6".parse().unwrap();
        for (text, expected, shifts) in [
            ("A1", "C3", true),
            ("$A$1", "A1", false),
            ("A$1", "C1", true),
            ("$A1", "A3", true),
            ("C3:A1", "C3:E5", true),
            ("$C$3:$A$1", "A1:C3", false),
            ("1:3", "3:5", true),
            ("$3:$1", "1:3", false),
            ("A:C", "C:E", true),
            ("$C:$A", "A:C", false),
        ] {
            let formula = Formula::from_file(text, authored);
            assert_eq!(relative(&formula), Some(shifts), "{text}");
            assert_eq!(
                range(&formula, host),
                Some(expected.parse::<CellRange>().unwrap()),
                "{text}"
            );
        }
        let full = Formula::from_file("$A$1:$XFD$1048576", authored);
        assert_eq!(range(&full, host), Some(CellRange::all()));
        let rows = Formula::from_file("$1:$1048576", authored);
        let cols = Formula::from_file("$A:$XFD", authored);
        for formula in [rows, cols] {
            let resolved = range(&formula, host).unwrap();
            assert_eq!(resolved.row_size(), MAX_ROWS);
            assert_eq!(resolved.column_size(), MAX_COLUMNS);
        }
    }

    #[test]
    fn target_range_off_grid_refuses_without_clamping_or_confusing_names() {
        let zero = CellRef::new(0, 0);
        let end = CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1);
        for (text, authored, host) in [
            ("A1", CellRef::new(1, 1), zero),
            ("B2", zero, end),
            ("1:2", CellRef::new(1, 0), zero),
            ("A:B", CellRef::new(0, 1), zero),
            ("A1:B2", zero, end),
        ] {
            let formula = Formula::from_file(text, authored);
            assert!(range(&formula, host).is_none(), "{text}");
        }
        let name = Formula::from_file("DefinedRange", zero);
        assert_eq!(range(&name, zero), None);
        assert_eq!(relative(&name), Some(false));
        // A valid target at the last cell stays exact; an edge is not an error.
        let end_ref = Formula::from_file("$XFD$1048576", zero);
        assert_eq!(range(&end_ref, end), Some(CellRange::new(end, end)));
    }
}
