//! `rust/src/excel/formula/text.rs`: source-resolved text semantics.

use yggdryl::excel::{Cell, CellRef, DateSystem, Formula, Workbook};
use yggdryl::Scalar;

#[test]
fn text_length_preserves_blank_error_and_unresolved_coercion_boundaries() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    let cases = [
        ("LEN(\"\")", Some(0.0), None),
        ("LEN(\"A\u{1f600}Z\")", Some(4.0), None),
        ("LEN(\"e\u{301}\")", Some(2.0), None),
        ("LEN(Z99)", Some(0.0), None),
        ("LEN(1/0)", None, Some("#DIV/0!")),
        ("LEN(TRUE)", Some(4.0), None),
        ("LEN(2.5)", Some(3.0), None),
    ];
    for (row, (formula, _, _)) in cases.iter().enumerate() {
        let at = CellRef::new(u32::try_from(row).unwrap(), 0);
        sheet.insert_cell(Cell::from_scalar(at, Scalar::from(777.0), DateSystem::Year1900).unwrap()
            .with_formula(Formula::from_entry(formula, at).unwrap())).unwrap();
    }
    let result = book.calculate_all().unwrap();
    assert_eq!((result.evaluated, result.uncomputed), (7, 0));
    for (row, (_, number, error)) in cases.iter().enumerate() {
        let cell = book.sheet("Data").unwrap().cell(CellRef::new(u32::try_from(row).unwrap(), 0)).unwrap();
        assert_eq!(cell.error().map(|error| error.as_str()), *error);
        if error.is_none() { assert_eq!(cell.value().as_f64(), Some(number.unwrap_or(777.0))); }
    }
    assert_eq!(book.recalculate().unwrap().evaluated, 0);
}

#[test]
fn text_functions_keep_literal_content_and_function_specific_windows() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Data").unwrap();
    let cases = [
        ("T(TRUE)", ""),
        ("CLEAN(\" A \" )", " A "),
        ("TRIM(\"  A  B  \")", "A B"),
        ("LEFT(\"\u{1f600}Z\",1)", "\u{1f600}"),
        ("RIGHT(\"A\u{1f600}\",1)", "\u{1f600}"),
        ("MID(\"A\u{1f600}Z\",2,2)", "\u{1f600}"),
        ("REPT(\"ab\",2.9)", "abab"),
        ("SUBSTITUTE(\"aaa\",\"aa\",\"b\")", "ba"),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        let at = CellRef::new(u32::try_from(row).unwrap(), 0);
        sheet.insert_cell(Cell::from_scalar(at, Scalar::Null, DateSystem::Year1900).unwrap()
            .with_formula(Formula::from_entry(formula, at).unwrap())).unwrap();
    }
    assert_eq!(book.calculate_all().unwrap().evaluated,8);
    for (row, (_, expected)) in cases.iter().enumerate() {
        assert_eq!(book.sheet("Data").unwrap().scalar(CellRef::new(u32::try_from(row).unwrap(),0)).as_str(),Some(*expected));
    }
    assert_eq!(book.recalculate().unwrap().evaluated,0);
}

#[test]
fn text_join_keeps_control_references_scalar_and_body_references_as_ranges() {
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap();
    book.add_sheet("Cases").unwrap();
    for (address,text) in [("A1","|"),("B1","a"),("B2","b")] {
        book.sheet_mut("Values").unwrap().set_cell(address.parse().unwrap(),text).unwrap();
    }
    // Taking all of the delimiter's column as a dependency would make a
    // false cycle through A2; legacy scalar intersection reads only A1.
    book.set_entry("Values","A2".parse().unwrap(),"=Cases!A1").unwrap();
    book.set_entry("Cases","A1".parse().unwrap(),"=TEXTJOIN(Values!A:A,TRUE,Values!B1:B2)").unwrap();
    let report=book.calculate_all().unwrap();
    assert_eq!((report.evaluated,report.uncomputed,report.circular_count),(2,0,0));
    for sheet in ["Values","Cases"] {
        let address=if sheet=="Values" { "A2" } else { "A1" };
        assert_eq!(book.sheet(sheet).unwrap().scalar(address.parse().unwrap()).as_str(),Some("a|b"));
    }
    book.sheet_mut("Values").unwrap().set_cell("B2".parse().unwrap(),"c").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated,2);
    assert_eq!(book.sheet("Cases").unwrap().scalar("A1".parse().unwrap()).as_str(),Some("a|c"));
}

#[test]
fn text_find_replace_share_position_intake_without_replacing_utf16_halves() {
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    for (row,formula,number,text,held) in [
        (0,"FIND(\"\",\"\",1)",Some(1.0),None,false),
        (1,"FIND(\"\",\"abc\",4)",Some(4.0),None,false),
        (2,"FIND(\"Z\",\"A\u{1f600}Z\",3)",Some(4.0),None,false),
        (3,"REPLACE(\"A\u{1f600}Z\",2,2,\"X\")",None,Some("AXZ"),false),
        (4,"REPLACE(\"A\u{1f600}Z\",2,1,\"X\")",None,None,true),
    ] {
        let at=CellRef::new(row,0);
        book.sheet_mut("Data").unwrap().insert_cell(Cell::from_scalar(at,Scalar::from("prior"),DateSystem::Year1900).unwrap()
            .with_formula(Formula::from_entry(formula,at).unwrap())).unwrap();
        book.calculate_all().unwrap();
        let value=book.sheet("Data").unwrap().scalar(at);
        if held {assert_eq!(value.as_str(),Some("prior"));}
        else if let Some(number)=number {assert_eq!(value.as_f64(),Some(number));}
        else {assert_eq!(value.as_str(),text);}
    }
}

#[test]
fn text_casing_shares_the_core_string_owner_without_importing_unicode_expansions() {
    use yggdryl::excel::Workbook;
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    let cases=[("LOWER(\"A\u{1f600}Z\")",Some("a\u{1f600}z")),
               ("UPPER(\"a\u{4e2d}z\")",Some("A\u{4e2d}Z")),
               ("LOWER(\"\u{0130}\")",None),("UPPER(\"\u{00df}\")",None)];
    for (row,(formula,_)) in cases.iter().enumerate() {
        book.set_entry("Data",yggdryl::excel::CellRef::new(row as u32,0),&format!("={formula}")).unwrap();
    }
    let report=book.calculate_all().unwrap();assert_eq!((report.evaluated,report.uncomputed),(2,2));
    for (row,(_,expected)) in cases.iter().enumerate() {
        let cell=book.sheet("Data").unwrap().cell(yggdryl::excel::CellRef::new(row as u32,0)).unwrap();
        if let Some(expected)=expected {assert_eq!(cell.value().as_str(),Some(*expected));}
        else {assert!(cell.value().is_null());}
    }
}

#[test]
fn text_character_limits_do_not_guess_a_code_page_or_locale() {
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    let cases=[("CHAR(65.9)",Some("A"),None),
        ("CHAR(0.9)",None,Some("#VALUE!")),("CHAR(256)",None,Some("#VALUE!")),
        ("CHAR(128)",None,None),("PROPER(\"o'NEILL 123abc\")",Some("O'Neill 123Abc"),None),
        ("PROPER(\"A\u{4e2d}B\")",Some("A\u{4e2d}b"),None),
        ("PROPER(\"\u{0130}STANBUL\")",None,None)];
    for (row,(formula,_,_)) in cases.iter().enumerate() {
        book.set_entry("Data",CellRef::new(row as u32,0),&format!("={formula}")).unwrap();
    }
    let report=book.calculate_all().unwrap();assert_eq!((report.evaluated,report.uncomputed),(5,2));
    for (row,(_,text,error)) in cases.iter().enumerate() {
        let cell=book.sheet("Data").unwrap().cell(CellRef::new(row as u32,0)).unwrap();
        assert_eq!(cell.error().map(|v|v.as_str()),*error);
        if let Some(text)=text {assert_eq!(cell.value().as_str(),Some(*text));}
        else if error.is_none() {assert!(cell.value().is_null());}
    }
}

#[test]
fn text_format_replaces_one_cached_code_and_preserves_error_dependencies() {
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    book.set_entry("Data",CellRef::new(0,0),"2.5").unwrap();
    book.set_entry("Data",CellRef::new(0,1),"'0.00").unwrap();
    book.set_entry("Data",CellRef::new(0,2),"=TEXT(A1,B1)").unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated,1);
    assert_eq!(book.sheet("Data").unwrap().scalar(CellRef::new(0,2)).as_str(),Some("2.50"));
    for (code,expected) in [("'0",Some("3")),("'[bad]",None),("'0.00",Some("2.50"))] {
        book.set_entry("Data",CellRef::new(0,1),code).unwrap();
        assert_eq!(book.recalculate().unwrap().evaluated,1);
        let cell=book.sheet("Data").unwrap().cell(CellRef::new(0,2)).unwrap();
        if let Some(expected)=expected {assert_eq!(cell.value().as_str(),Some(expected));}
        else {assert_eq!(cell.error().map(|e|e.as_str()),Some("#VALUE!"));}
    }
    book.set_entry("Data",CellRef::new(0,0),"=1/0").unwrap();
    assert_eq!(book.recalculate().unwrap().evaluated,2);
    assert_eq!(book.sheet("Data").unwrap().cell(CellRef::new(0,2)).unwrap().error().map(|e|e.as_str()),Some("#DIV/0!"));
}

#[test]
fn text_format_bounds_repeated_text_before_expanding_each_placeholder() {
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    book.sheet_mut("Data").unwrap().set_cell(CellRef::new(0,0),"x".repeat(32767)).unwrap();
    book.set_entry("Data",CellRef::new(0,1),"=TEXT(A1,\"@@\")").unwrap();
    assert_eq!(book.calculate_all().unwrap().evaluated,1);
    assert_eq!(book.sheet("Data").unwrap().cell(CellRef::new(0,1)).unwrap().error().map(|e|e.as_str()),Some("#VALUE!"));
}

#[test]
fn boolean_text_conversion_matches_explicit_en_us_native_evaluation() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../fixtures/boolean_text_en_us_native.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    let mut book = Workbook::new();
    book.add_sheet("Values").unwrap().set_cell(CellRef::new(0, 0), true).unwrap();
    book.sheet_mut("Values").unwrap().set_cell(CellRef::new(1, 0), false).unwrap();
    book.add_sheet("Cases").unwrap();
    for case in cases {
        let at = case["cell"].as_str().unwrap().parse().unwrap();
        book.set_entry("Cases", at, &format!("={}", case["wire_formula"].as_str().unwrap())).unwrap();
    }
    let report = book.calculate_all().unwrap();
    assert_eq!((report.evaluated, report.uncomputed), (12, 0));
    for case in cases {
        let cell = book.sheet("Cases").unwrap().cell(case["cell"].as_str().unwrap().parse().unwrap()).unwrap();
        let expected = &case["en_us"]["raw"];
        match expected["variant"].as_str().unwrap() {
            "str" => assert_eq!(cell.value().as_str(), expected["value"].as_str(), "{}", case["id"]),
            "float" => assert_eq!(cell.value().as_f64().map(f64::to_bits), expected["value"].as_f64().map(f64::to_bits), "{}", case["id"]),
            "bool" => assert_eq!(cell.value().as_bool(), expected["value"].as_bool(), "{}", case["id"]),
            other => panic!("unexpected observed Boolean conversion result {other}"),
        }
    }
}
