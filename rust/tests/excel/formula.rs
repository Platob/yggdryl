//! `rust/src/excel/formula.rs`: `Formula` - one shape per formula, its file and entry spellings, what a user types refused at the byte of its first syntax error, and the per-sheet interning a parse shares shapes through.

fn native_function_fixture(text: &str, count: u64) {
    use yggdryl::Scalar;
    use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
    let fixture: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(fixture["native_run_passed"], true);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["native_cache_comparisons_equal"], count * 2);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len() as u64, count);
    let mut seen = 0;
    let mut addresses = std::collections::HashSet::new();
    for case in cases {
        assert!(addresses.insert((case["date_system"].as_str().unwrap(), case["sheet"].as_str().unwrap(), case["cell"].as_str().unwrap())),
            "native workbooks with overlapping cell addresses must replay separately: {}", case["id"]);
    }
    for (year, system) in [("1900", DateSystem::Year1900), ("1904", DateSystem::Year1904)] {
        let mut book = Workbook::new();
        book.set_date_system(system);
        for name in ["Values", "CycleShape", "Cases"] { book.add_sheet(name).unwrap(); }
        for (name, cells) in fixture["source_cells"].as_object().unwrap() {
            for (address, value) in cells.as_object().unwrap() {
                let value = if let Some(value) = value.as_bool() { Scalar::from(value) }
                    else if let Some(value) = value.as_str() { Scalar::from(value) }
                    else { Scalar::from(value.as_f64().unwrap()) };
                book.sheet_mut(name).unwrap().set_cell(address.parse().unwrap(), value).unwrap();
            }
        }
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            book.sheet_mut(case["sheet"].as_str().unwrap()).unwrap().insert_cell(
                Cell::from_scalar(at, Scalar::from(-777.0), system).unwrap()
                    .with_formula(Formula::from_file(case["wire_formula"].as_str().unwrap(), at)),
            ).unwrap();
        }
        let report = book.calculate_all().unwrap();
        assert_eq!((report.evaluated as u64, report.uncomputed, report.circular_count), (count / 2, 0, 0));
        for case in cases.iter().filter(|case| case["date_system"] == year) {
            seen += 1;
            let at: CellRef = case["cell"].as_str().unwrap().parse().unwrap();
            let cell = book.sheet(case["sheet"].as_str().unwrap()).unwrap().cell(at).unwrap();
            let cache = &case["saved_cache"];
            match cache["type"].as_str().unwrap() {
                "n" => assert_eq!(cell.value().as_f64().unwrap().to_bits(),
                    cache["value_text"].as_str().unwrap().parse::<f64>().unwrap().to_bits(), "{}", case["id"]),
                "b" => assert_eq!(cell.value().as_bool(), Some(cache["value_text"] == "1"), "{}", case["id"]),
                "e" => assert_eq!(cell.error().map(|error| error.as_str()), cache["value_text"].as_str(), "{}", case["id"]),
                "str" => assert_eq!(cell.value().as_str(), Some(cache["value_text"].as_str().unwrap_or("")), "{}", case["id"]),
                kind => panic!("unexpected native cache kind {kind}"),
            }
        }
        let revisions = ["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision());
        assert_eq!(book.calculate_all().unwrap().evaluated as u64, count / 2);
        assert_eq!(["Values", "CycleShape", "Cases"].map(|name| book.sheet(name).unwrap().revision()), revisions);
        assert_eq!(book.recalculate().unwrap().evaluated,
            fixture["incremental_evaluated_per_epoch"].as_u64().unwrap_or(0));
    }
    assert_eq!(seen, count);
}

#[path = "formula/aggregate.rs"]
mod aggregate;
#[path = "formula/criteria.rs"]
mod criteria;
#[path = "formula/eval.rs"]
mod eval;
#[path = "formula/functions.rs"]
mod functions;
#[path = "formula/graph.rs"]
mod graph;
#[path = "formula/lexer.rs"]
mod lexer;
#[path = "formula/number.rs"]
mod number;
#[path = "formula/parser.rs"]
mod parser;
#[path = "formula/reference.rs"]
mod reference;
#[path = "formula/shape.rs"]
mod shape;
#[path = "formula/value.rs"]
mod value;

use std::collections::HashSet;

use yggdryl::Error;
use yggdryl::excel::{CellRef, Formula};

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

/// The byte and the reason `text` is refused at as entry.
fn refusal(text: &str) -> (usize, String) {
    match Formula::from_entry(text, at("A1")).unwrap_err() {
        Error::Parse {
            target: "formula",
            position,
            reason,
        } => (position, reason.to_string()),
        other => panic!("expected a formula refusal, got {other:?}"),
    }
}

#[test]
fn any_text_is_a_formula_and_renders_back_as_the_file_spelled_it() {
    for text in [
        "SUM(A1:A2)",
        "_xlfn.XLOOKUP(A1, B:B, C:C)",
        "\"a\"&\"b\"",
        "1 +  2",
        "[1]Sheet1!A1",
        "'Q1 ''24'!B2",
        "SUM(",
        "",
        "Table1[[#This Row],[Price]]*2",
        "A1#",
        "{1,2;3,4}",
        "IF(A1>=0,\"yes\",\"no\")",
        "#REF!",
        "Sheet1!#REF!",
        "a ~ b",
    ] {
        let formula = Formula::from_file(text, at("C3"));
        assert_eq!(formula.at(at("C3")).to_string(), text);
    }
}

#[test]
fn a_formula_renders_translated_at_every_other_host() {
    let formula = Formula::from_file("A1*$B$1+A$1-$A1+SUM(A:A,1:1)", at("C1"));
    assert_eq!(
        formula.at(at("D3")).to_string(),
        "B3*$B$1+B$1-$A3+SUM(B:B,3:3)"
    );
    // A reference moved off the grid is `#REF!`, the rest of the text kept.
    assert_eq!(
        formula.at(at("A1")).to_string(),
        "#REF!*$B$1+#REF!-$A1+SUM(#REF!,1:1)"
    );
}

#[test]
fn formulas_that_translate_into_one_another_are_equal_and_hash_alike() {
    let first = Formula::from_file("A1*B1", at("C1"));
    let second = Formula::from_file("A2*B2", at("C2"));
    let other = Formula::from_file("A1*B2", at("C1"));
    assert_eq!(first, second);
    assert_ne!(first, other);
    let set: HashSet<Formula> = [first.clone(), second, other].into_iter().collect();
    assert_eq!(set.len(), 2);
    assert_eq!(first.clone(), first);
    // Debug spells the shape the one way that reads the same at every host.
    assert_eq!(format!("{first:?}"), "Formula(\"RC[-2]*RC[-1]\")");
    assert_eq!(
        format!("{:?}", Formula::from_file("$A$1+B:B", at("C1"))),
        "Formula(\"R1C1+C[-1]:C[-1]\")"
    );
}

#[test]
fn the_entry_spelling_drops_the_file_s_prefixes_and_spells_implicit_intersection_as_at() {
    let host = at("B2");
    let formula = Formula::from_file(
        "_xlfn.XLOOKUP(A2,_xlfn._xlws.SORT(C:C),D:D)+_xlfn.SINGLE(A1:A9)+_xlfn.SINGLE(A1+A2)",
        host,
    );
    assert_eq!(
        formula.entry_at(host).to_string(),
        "XLOOKUP(A2,SORT(C:C),D:D)+@A1:A9+@(A1+A2)"
    );
    assert_eq!(
        formula.at(host).to_string(),
        "_xlfn.XLOOKUP(A2,_xlfn._xlws.SORT(C:C),D:D)+_xlfn.SINGLE(A1:A9)+_xlfn.SINGLE(A1+A2)"
    );
}

#[test]
fn entry_takes_the_prefix_a_later_function_is_stored_under_and_at_becomes_single() {
    let host = at("C3");
    for (typed, file) in [
        ("concat(a1,\"x\")", "_xlfn.CONCAT(A1,\"x\")"),
        ("Sort(A1:A3)", "_xlfn._xlws.SORT(A1:A3)"),
        ("sum(a1:b2)", "SUM(A1:B2)"),
        ("@A1:A3", "_xlfn.SINGLE(A1:A3)"),
        ("@INDEX(A:A,1)+1", "_xlfn.SINGLE(INDEX(A:A,1))+1"),
        ("@(A1+A2)", "_xlfn.SINGLE(A1+A2)"),
        ("'Sheet1'!a1 + true", "Sheet1!A1 + TRUE"),
        ("'Q1 ''24'!B2", "'Q1 ''24'!B2"),
        ("iferror(1/0,#n/a)", "IFERROR(1/0,#N/A)"),
    ] {
        let formula = Formula::from_entry(typed, host).unwrap();
        assert_eq!(formula.at(host).to_string(), file, "{typed}");
        // What the file holds reads back as the same shape.
        assert_eq!(Formula::from_file(file, host), formula, "{typed}");
    }
    // The entry spelling of what was entered is what a user would type.
    let formula = Formula::from_entry("@A1:A3+xlookup(1,A:A,B:B)", host).unwrap();
    assert_eq!(
        formula.entry_at(host).to_string(),
        "@A1:A3+XLOOKUP(1,A:A,B:B)"
    );
}

#[test]
fn entry_refuses_a_syntax_error_at_its_byte() {
    for (text, position, reason) in [
        ("", 0, "expected a formula"),
        ("   ", 0, "expected a formula"),
        ("SUM(A1", 3, "expected ) to close this"),
        ("A1)", 2, "expected no )"),
        ("\"open", 0, "expected the closing \" of a text"),
        ("1+", 2, "expected an operand at the end"),
        ("*2", 0, "expected an operand before *"),
        ("1+*2", 2, "expected an operand before *"),
        ("A1 2", 3, "expected an operator before \"2\""),
        (
            "A1,B1",
            2,
            "expected , between the arguments of a call or inside parentheses",
        ),
        ("()", 1, "expected an operand before )"),
        ("1;2", 1, "expected ; only between the rows of an array"),
        ("A1 ~ 2", 3, "expected a formula, got \"~\""),
        ("@", 1, "expected an operand at the end"),
    ] {
        let (at, why) = refusal(text);
        assert_eq!((at, why.as_str()), (position, reason), "{text:?}");
    }
    // What a user may type: empty arguments, unary signs, percent, unions in
    // parentheses, intersections, arrays and spills.
    for text in [
        "IF(A1,,1)",
        "NOW()",
        "-A1%",
        "--1",
        "SUM((A1:A3,C1:C3))",
        "A1:C3 B2:B9",
        "{1,2;3,4}",
        "SUM(A1#)",
        "Table1[Price]*2",
        "1E+3+.5",
    ] {
        assert!(Formula::from_entry(text, at("A1")).is_ok(), "{text}");
    }
}

#[test]
fn entry_refuses_text_past_excel_s_length_and_nesting() {
    let long = "1+".repeat(4_096) + "1";
    let (position, reason) = refusal(&long);
    assert_eq!(position, 8_192);
    assert_eq!(reason, "expected at most 8192 characters");
    let deep = "(".repeat(65) + "1" + &")".repeat(65);
    let (position, reason) = refusal(&deep);
    assert_eq!(position, 64);
    assert_eq!(reason, "expected at most 64 levels of nesting");
    let fine = "(".repeat(64) + "1" + &")".repeat(64);
    assert!(Formula::from_entry(&fine, at("A1")).is_ok());
}

#[test]
fn a_formula_holding_what_is_carried_rather_than_computed_says_so() {
    for (text, computed) in [
        ("SUM(A1:A3)*2", true),
        ("[1]Sheet1!A1", false),
        ("Table1[Price]", false),
        ("SUM(A1#)", false),
        ("_xlfn.ANCHORARRAY(A1)", false),
        ("A1 ~ 2", false),
        ("\"open", false),
    ] {
        assert_eq!(
            Formula::from_file(text, at("B1")).is_computed(),
            computed,
            "{text}"
        );
    }
}
#[test]
fn unknown_and_prefix_only_functions_remain_held_without_losing_their_spelling() {
    let host = at("C3");
    for file in [
        "MYSTERY(A1)",
        "_xlfn._xlws.FILTER(A1:A3,B1:B3)",
        "_xlfn.LAMBDA(x,x+1)(2)",
    ] {
        let formula = Formula::from_file(file, host);
        assert!(!formula.is_computed(), "{file}");
        assert_eq!(formula.at(host).to_string(), file);
    }
    let typed = Formula::from_entry("filter(a1:a3,b1:b3)", host).unwrap();
    assert_eq!(
        typed.at(host).to_string(),
        "_xlfn._xlws.FILTER(A1:A3,B1:B3)"
    );
    assert!(!typed.is_computed());
    assert!(
        Formula::from_entry("SUM(A1:A3)", host)
            .unwrap()
            .is_computed()
    );
}

#[test]
fn entry_refuses_jagged_array_rows_at_the_closing_brace() {
    for text in ["{1,2;3}", "{1;2,3}"] {
        let (position, reason) = refusal(text);
        assert_eq!(position, text.len() - 1, "{text}");
        assert!(reason.contains("array row"), "{text}: {reason}");
        assert!(reason.contains("expected"), "{text}: {reason}");
        assert!(reason.contains("got"), "{text}: {reason}");
    }
}

#[test]
fn malformed_file_syntax_is_carried_lazily_and_kept_verbatim() {
    let host = at("C3");
    let formula = Formula::from_file("SUM(A1", host);
    assert!(!formula.is_computed());
    assert_eq!(formula.at(host).to_string(), "SUM(A1");
}

#[test]
fn entry_array_constants_refuse_nonconstant_members_at_their_byte() {
    // Excel's array-constant grammar excludes references, formulas, calls,
    // nested arrays, and percentage notation, even though each is valid
    // outside a constant. See the parser preparation note's Microsoft links.
    for (text, position) in [
        ("{A1,2}", 1),
        ("{Rate,2}", 1),
        ("{SUM(1),2}", 1),
        ("{_xlfn.CONCAT(\"a\"),2}", 1),
        ("{{1,2},{3,4}}", 1),
        ("{(1),2}", 1),
        ("{1+2,3}", 2),
        ("{1%,2}", 2),
        ("{-TRUE,2}", 2),
        ("{--1,2}", 2),
        ("{1 2,3}", 3),
    ] {
        assert_eq!(
            refusal(text),
            (position, "expected a constant value in an array".into()),
            "{text}",
        );
    }
}

#[test]
fn entry_array_constants_refuse_ragged_rows_at_their_delimiter() {
    for (text, position, expected, actual) in [
        ("{1,2;3}", 6, 2, 1),
        ("{1;2,3}", 6, 1, 2),
        ("{1,2;3;4,5}", 6, 2, 1),
        ("{1,2;3,4;5}", 10, 2, 1),
    ] {
        assert_eq!(
            refusal(text),
            (
                position,
                format!("expected {expected} columns in an array row, got {actual}"),
            ),
            "{text}",
        );
    }
}

#[test]
fn entry_array_constants_accept_scalar_literals_and_outer_expressions() {
    for text in [
        "{1}",
        "{1,2;3,4}",
        "{-1,2E-3;4.5,-6}",
        "{TRUE,FALSE;#N/A,#VALUE!}",
        "{\"a\"\"b\",\"\";\"c\",\"d\"}",
        "SUM({1,2;3,4})+5%",
        "{1,2}+{3,4}",
        "{ 1 , 2 ; 3 , 4 }",
    ] {
        let formula = Formula::from_entry(text, at("C3")).unwrap();
        assert_eq!(formula.at(at("C3")).to_string(), text, "{text}");
    }
}

#[test]
fn entry_array_refusal_does_not_change_total_file_intake() {
    for text in ["{A1,2}", "{SUM(1),2}", "{1,2;3}", "{{1,2},{3,4}}"] {
        assert!(Formula::from_entry(text, at("C3")).is_err(), "{text}");
        let formula = Formula::from_file(text, at("C3"));
        assert_eq!(formula.at(at("C3")).to_string(), text);
    }
}

#[test]
fn entry_uses_the_typed_function_signature_for_arity() {
    for (text, count) in [
        ("ABS(1,2)", 2),
        ("ROUND(1)", 1),
        ("IF(1)", 1),
        ("IF(1,2,3,4)", 4),
    ] {
        let (position, reason) = refusal(text);
        assert_eq!(position, text.find('(').unwrap(), "{text}");
        assert!(reason.contains("expected"), "{text}: {reason}");
        assert!(reason.contains(&format!("got {count}")), "{text}: {reason}");
    }
    assert!(Formula::from_entry("IF(1,2)", at("A1")).is_ok());
    assert!(Formula::from_entry("IF(1,,3)", at("A1")).is_ok());
    assert!(Formula::from_entry("INDEX((C2:C3,C4:C5),1,1,2)", at("A1")).is_ok());
    let unknown = Formula::from_entry("MYFUNC(1)", at("A1")).unwrap();
    assert!(!unknown.is_computed());
}

#[test]
fn optional_call_slot_survives_space_before_close() {
    let host = at("A1");
    for text in ["IF(TRUE, )", "IF(FALSE,42, )"] {
        let entry = Formula::from_entry(text, host).unwrap();
        assert!(entry.is_computed(), "{text}");
        assert_eq!(entry.at(host).to_string(), text, "{text}");
        let file = Formula::from_file(text, host);
        assert!(file.is_computed(), "{text}");
    }
}

#[test]
fn native_clock_observations_pin_bounds_without_random_bits() {
    use yggdryl::excel::DateSystem;
    let fixture: serde_json::Value = serde_json::from_str(include_str!("fixtures/clock_native.json")).unwrap();
    assert_eq!(fixture["native"]["cleanup_completed"], true);
    let cases = fixture["observations"].as_array().unwrap();
    assert_eq!(cases.len(), 20);
    let find = |system: &str, formula: &str| {
        cases.iter().find(|case| case["system"] == system && case["authored_formula"] == formula).unwrap()
    };
    let number = |case: &serde_json::Value, phase: &str| {
        assert_eq!(case[phase]["kind"], "number");
        case[phase]["value"].as_f64().unwrap()
    };
    for phase in ["first", "after_save"] {
        for system in ["1900", "1904"] {
            let today = number(find(system, "TODAY()"), phase);
            let now = number(find(system, "NOW()"), phase);
            let date_system = if system == "1900" { DateSystem::Year1900 } else { DateSystem::Year1904 };
            let millis = date_system.millis_from_serial(now).unwrap();
            assert_eq!(date_system.serial_from_millis(millis).unwrap().to_bits(), now.to_bits(),
                "{system} {phase} native NOW must lie on the existing millisecond serial grid");
            assert_eq!(today.fract(), 0.0);
            assert!((today..today + 1.0).contains(&now));
            let fraction = number(find(system, "NOW()-TODAY()"), phase);
            assert!((0.0..1.0).contains(&fraction));
            let rand = number(find(system, "RAND()"), phase);
            assert!((0.0..1.0).contains(&rand));
            let between = number(find(system, "RANDBETWEEN(-5,7)"), phase);
            assert!(between.fract() == 0.0 && (-5.0..=7.0).contains(&between));
            assert_eq!(number(find(system, "RANDBETWEEN(7,7)"), phase), 7.0);
            assert_eq!(find(system, "RANDBETWEEN(8,7)")[phase]["literal"], "#NUM!");
        }
        let old = number(find("1900", "TODAY()"), phase);
        let modern = number(find("1904", "TODAY()"), phase);
        assert_eq!(old - modern, 1_462.0);
    }
}
#[path = "formula/text.rs"]
mod text;

// P5 public documented function catalog.

#[test]
fn documented_function_catalog_uses_the_one_parser_registry() {
    let entries: Vec<_> = yggdryl::excel::Formula::functions().collect();
    assert_eq!(entries.len(), 159);
    assert!(entries.windows(2).all(|pair| pair[0].name < pair[1].name));
    let sum = entries.iter().find(|entry| entry.name == "SUM").expect("SUM");
    assert_eq!(sum.category, "Math and trigonometry");
    assert_eq!(sum.signature, "SUM(number1, [number2], ...)");
    assert!(!sum.description.is_empty());
    assert!(!entries.iter().any(|entry| entry.name == "SINGLE"));
}
