//! `rust/src/excel/formula/shape.rs`: a shape - verbatim runs and relative references - shared by every formula that translates into it, and rewritten, never re-parsed, when a sheet it names is renamed or removed.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::{
        held, is_volatile, removed, renamed, shares_shape, sheets, tokens,
    };

    fn at(text: &str) -> CellRef {
        text.parse().unwrap()
    }

    #[test]
    fn a_shape_is_verbatim_runs_between_references_and_function_names() {
        let formula =
            Formula::from_file("SUM( A1 , 'Q 1'!$B$2 )*_xlfn.XLOOKUP(1,C:C,D:D)", at("B3"));
        assert_eq!(
            tokens(&formula),
            [
                "function:|SUM",
                "text:( ",
                "ref:R[-2]C[-1]",
                "text: , ",
                "ref:'Q 1'!R2C2",
                "text: )*",
                "function:_xlfn.|XLOOKUP",
                "text:(1,",
                "ref:C[1]:C[1]",
                "text:,",
                "ref:C[2]:C[2]",
                "text:)",
            ]
        );
        assert_eq!(sheets(&formula), ["Q 1"]);
        assert_eq!(held(&formula), None);
        let single = Formula::from_file("_xlfn.SINGLE(A1:A3)+_xlfn.SINGLE(A1+1)", at("B1"));
        assert_eq!(
            tokens(&single),
            [
                "single:false",
                "ref:RC[-1]:R[2]C[-1]",
                "end:false",
                "text:+",
                "single:true",
                "ref:RC[-1]",
                "text:+1",
                "end:true",
            ]
        );
    }

    #[test]
    fn the_facts_a_reader_asks_are_read_once_off_the_tokens() {
        for (text, sheet_names, volatile, reason) in [
            ("NOW()+Jan:Mar!A1+jan!B1", vec!["Jan", "Mar"], true, None),
            ("offset(A1,1,1)", vec![], true, None),
            (
                "[1]Sheet1!A1",
                vec![],
                false,
                Some("a reference into another workbook"),
            ),
            (
                "Table1[Price]",
                vec![],
                false,
                Some("a structured reference into a table"),
            ),
            ("A1#", vec![], false, Some("a spilled range reference")),
            (
                "_xlfn.ANCHORARRAY(A1)",
                vec![],
                false,
                Some("a dynamic array's own function"),
            ),
            (
                "A1 ~",
                vec![],
                false,
                Some("text the formula grammar does not read"),
            ),
        ] {
            let formula = Formula::from_file(text, at("C3"));
            assert_eq!(sheets(&formula), sheet_names, "{text}");
            assert_eq!(is_volatile(&formula), volatile, "{text}");
            assert_eq!(held(&formula), reason, "{text}");
        }
    }

    #[test]
    fn a_rename_rewrites_the_prefixes_naming_the_sheet_and_nothing_else() {
        let formula = Formula::from_file(
            "Data!A1+'data'!B2+Other!C3+Data:Other!D4+\"Data!A1\"",
            at("E5"),
        );
        let moved = renamed(&formula, "DATA", "Q1 '24").unwrap();
        assert_eq!(
            moved.at(at("E5")).to_string(),
            "'Q1 ''24'!A1+'Q1 ''24'!B2+Other!C3+'Q1 ''24:Other'!D4+\"Data!A1\""
        );
        assert_eq!(sheets(&moved), ["Q1 '24", "Other"]);
        // A shape naming no such sheet is left as it is.
        assert!(renamed_none(&formula, "Missing"));
        // Quotes the file wrote for the old name go where the new needs none.
        let quoted = Formula::from_file("'My Data'!A1", at("B1"));
        assert_eq!(
            renamed(&quoted, "My Data", "Data")
                .unwrap()
                .at(at("B1"))
                .to_string(),
            "Data!A1"
        );
    }

    fn renamed_none(formula: &Formula, from: &str) -> bool {
        renamed(formula, from, "X").is_none()
    }

    #[test]
    fn a_removal_makes_the_references_to_the_sheet_ref_errors_and_draws_a_span_in() {
        let order = ["Jan", "Feb", "Mar", "Apr"];
        let formula = Formula::from_file("Feb!A1+SUM(Jan:Mar!B2)+Apr!C3", at("D4"));
        let gone = removed(&formula, "Feb", &order).unwrap();
        assert_eq!(
            gone.at(at("D4")).to_string(),
            "#REF!A1+SUM(Jan:Mar!B2)+Apr!C3"
        );
        let gone = removed(&formula, "Jan", &order).unwrap();
        assert_eq!(
            gone.at(at("D4")).to_string(),
            "Feb!A1+SUM(Feb:Mar!B2)+Apr!C3"
        );
        let gone = removed(&formula, "Mar", &order).unwrap();
        assert_eq!(
            gone.at(at("D4")).to_string(),
            "Feb!A1+SUM(Jan:Feb!B2)+Apr!C3"
        );
        let two = Formula::from_file("SUM(Jan:Feb!A1)", at("B1"));
        assert_eq!(
            removed(&two, "Jan", &order)
                .unwrap()
                .at(at("B1"))
                .to_string(),
            "SUM(Feb!A1)"
        );
        assert!(removed(&formula, "Other", &order).is_none());
    }

    #[test]
    fn formulas_that_translate_into_one_another_hold_equal_shapes_and_a_clone_holds_the_same_one() {
        let first = Formula::from_file("A1*2", at("B1"));
        let second = Formula::from_file("A2*2", at("B2"));
        assert_eq!(first, second);
        assert!(!shares_shape(&first, &second));
        assert!(shares_shape(&first, &first.clone()));
    }
}

#[test]
fn hostile_file_colon_chain_uses_bounded_stack() {
    use yggdryl::excel::{CellRef, Formula};

    const CHILD: &str = "YGGDRYL_FORMULA_COLON_STACK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let host: CellRef = "A1".parse().unwrap();
                // File intake is total even beyond the entered formula length
                // limit. Colon chains are flat; they do not consume call depth.
                let text = format!("_xlfn.SINGLE({})", vec!["Name"; 20_000].join(":"));
                let formula = Formula::from_file(&text, host);
                assert_eq!(formula.at(host).to_string(), text);
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    // Contain a regression's stack abort so it reports as this failed test.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "formula::shape::hostile_file_colon_chain_uses_bounded_stack",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn nested_implicit_prefixes_lower_iteratively_at_the_same_operand_boundary() {
    use yggdryl::excel::{CellRef, Formula};

    let host: CellRef = "C3".parse().unwrap();
    for (entry, wire) in [
        ("@@A1", "_xlfn.SINGLE(_xlfn.SINGLE(A1))"),
        ("@ @ A1", "_xlfn.SINGLE( _xlfn.SINGLE( A1))"),
        ("@@A1:B2", "_xlfn.SINGLE(_xlfn.SINGLE(A1:B2))"),
        ("@ @ABS(1)+2", "_xlfn.SINGLE( _xlfn.SINGLE(ABS(1)))+2"),
    ] {
        let formula = Formula::from_entry(entry, host).unwrap();
        assert_eq!(formula.at(host).to_string(), wire, "{entry}");
        assert!(formula.is_computed(), "{entry}");
        let held = Formula::from_file(wire, host);
        assert_eq!(held.at(host).to_string(), wire);
        assert!(held.is_computed(), "{wire}");
        assert_eq!(formula, held, "{entry}");
    }
}
