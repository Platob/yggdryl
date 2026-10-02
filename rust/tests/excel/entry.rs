//! `rust/src/excel/entry.rs`: `Entry` - the text a user types into a cell, read as en-US Excel reads it - and the workbook's typed entry doors over it: `set_entry`, `entry_text` and `display_text`.

use smol_str::SmolStr;
use yggdryl::excel::{CellRange, CellRef, DateSystem, Entry, ExcelError, StylePatch, Workbook};
use yggdryl::{Scalar, TimeUnit, Timezone};

/// What `text` typed into A1 reads as.
fn read(text: &str) -> Entry {
    Entry::from_text(text, CellRef::new(0, 0), DateSystem::Year1900)
        .unwrap_or_else(|error| panic!("{text:?}: {error}"))
}

/// A value entry.
fn value(value: Scalar, format: Option<&str>) -> Entry {
    Entry::Value {
        value,
        format: format.map(SmolStr::new),
    }
}

fn time(millis: i32) -> Scalar {
    Scalar::time32(millis, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
}

fn instant(millis: i64) -> Scalar {
    Scalar::datetime64(millis, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
}

#[test]
fn a_number_reads_with_the_format_its_spelling_suggests() {
    for (text, expected) in [
        ("5", value(Scalar::from(5.0), None)),
        ("-5", value(Scalar::from(-5.0), None)),
        ("+5", value(Scalar::from(5.0), None)),
        ("  42  ", value(Scalar::from(42.0), None)),
        ("1,234.5", value(Scalar::from(1234.5), None)),
        ("1,234,567", value(Scalar::from(1_234_567.0), None)),
        ("(12)", value(Scalar::from(-12.0), None)),
        (".5", value(Scalar::from(0.5), None)),
        ("12%", value(Scalar::from(0.12), Some("0%"))),
        ("12.34%", value(Scalar::from(0.1234), Some("0.00%"))),
        ("-3%", value(Scalar::from(-0.03), Some("0%"))),
        (
            "$1,234.50",
            value(Scalar::from(1234.5), Some("\"$\"#,##0.00")),
        ),
        ("$1,234", value(Scalar::from(1234.0), Some("\"$\"#,##0"))),
        ("-$5", value(Scalar::from(-5.0), Some("\"$\"#,##0"))),
        ("$-5.25", value(Scalar::from(-5.25), Some("\"$\"#,##0.00"))),
        ("($5)", value(Scalar::from(-5.0), Some("\"$\"#,##0"))),
        ("1e3", value(Scalar::from(1000.0), Some("0.00E+00"))),
        ("1.5E-10", value(Scalar::from(1.5e-10), Some("0.00E+00"))),
        ("1 1/2", value(Scalar::from(1.5), Some("# ?/?"))),
        ("0 3/4", value(Scalar::from(0.75), Some("# ?/?"))),
        ("-2 1/4", value(Scalar::from(-2.25), Some("# ?/?"))),
        ("3 7/16", value(Scalar::from(3.4375), Some("# ??/??"))),
        ("TRUE", value(Scalar::from(true), None)),
        ("false", value(Scalar::from(false), None)),
    ] {
        assert_eq!(read(text), expected, "{text:?}");
    }
}

#[test]
fn a_date_a_time_or_a_datetime_reads_under_the_built_in_format_excel_gives_it() {
    for (text, expected) in [
        ("1/2/2024", value(Scalar::date32(19_724), Some("m/d/yyyy"))),
        (
            "2024-01-02",
            value(Scalar::date32(19_724), Some("m/d/yyyy")),
        ),
        ("1-2-24", value(Scalar::date32(19_724), Some("m/d/yyyy"))),
        ("12/31/99", value(Scalar::date32(10_956), Some("m/d/yyyy"))),
        ("2/29/2024", value(Scalar::date32(19_782), Some("m/d/yyyy"))),
        ("10:30", value(time(37_800_000), Some("h:mm"))),
        ("10:30:15", value(time(37_815_000), Some("h:mm:ss"))),
        ("10:30 PM", value(time(81_000_000), Some("h:mm AM/PM"))),
        ("12:05 am", value(time(300_000), Some("h:mm AM/PM"))),
        (
            "10:30:15 pm",
            value(time(81_015_000), Some("h:mm:ss AM/PM")),
        ),
        ("7:45:30.25", value(time(27_930_250), Some("h:mm:ss"))),
        (
            "25:30",
            value(
                Scalar::duration64(91_800_000, TimeUnit::Millisecond).unwrap(),
                Some("[h]:mm"),
            ),
        ),
        (
            "36:00:00",
            value(
                Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap(),
                Some("[h]:mm:ss"),
            ),
        ),
        (
            "1/2/2024 10:30",
            value(instant(1_704_191_400_000), Some("m/d/yyyy h:mm")),
        ),
        (
            "1/2/2024 10:30:15 PM",
            value(instant(1_704_234_615_000), Some("m/d/yyyy h:mm")),
        ),
    ] {
        assert_eq!(read(text), expected, "{text:?}");
    }
}

#[test]
fn text_an_error_a_formula_or_nothing_reads_as_itself() {
    assert_eq!(read(""), Entry::Blank);
    assert_eq!(read("abc"), Entry::Text("abc".into()));
    assert_eq!(read("  padded "), Entry::Text("  padded ".into()));
    assert_eq!(read("'123"), Entry::Quoted("123".into()));
    assert_eq!(read("'=A1"), Entry::Quoted("=A1".into()));
    assert_eq!(read("#N/A"), Entry::Error(ExcelError::NA));
    assert_eq!(read("#div/0!"), Entry::Error(ExcelError::Div0));
    assert_eq!(read("#WHAT"), Entry::Text("#WHAT".into()));
    for text in ["=SUM(A1:A3)", "=a1*2", "-A1", "+A1+B1", "-SUM(B1:B2)"] {
        assert!(matches!(read(text), Entry::Formula(_)), "{text:?}");
    }
    let Entry::Formula(formula) = read("=xlookup(a1,b:b,c:c)") else {
        panic!("a formula");
    };
    assert_eq!(
        formula.entry_at(CellRef::new(0, 0)).to_string(),
        "XLOOKUP(A1,B:B,C:C)"
    );
    // Neither a number nor a date: text, the way Excel keeps them.
    for text in [
        "12,34",
        "1,2345",
        "1.2.3",
        "$",
        "12%%",
        "1/2",
        "2/30/2024",
        "13/1/2024",
        "1/1/1899",
        "24:60",
        "13:00 PM",
        "1 3/2",
        "1 1/0",
        "abc 1/2",
        "1e",
        "e5",
        "$1e3",
        // Signs and nothing else: the dash a ledger writes for nothing.
        "-",
        "+",
        "--",
        "- ",
        "+ -",
        "-----",
    ] {
        assert!(
            matches!(read(text), Entry::Text(_)),
            "{text:?}: {:?}",
            read(text)
        );
    }
}

#[test]
fn a_formula_that_does_not_read_is_refused_at_its_byte_in_the_text_typed() {
    let refusal = Entry::from_text("=SUM(A1", CellRef::new(0, 0), DateSystem::Year1900)
        .unwrap_err()
        .to_string();
    assert!(
        refusal.starts_with("invalid formula expression at byte "),
        "{refusal}"
    );
    let refusal = Entry::from_text("=1+", CellRef::new(0, 0), DateSystem::Year1900).unwrap_err();
    let yggdryl::Error::Parse { position, .. } = refusal else {
        panic!("a parse refusal");
    };
    assert_eq!(position, 3);
    assert!(Entry::from_text("-*", CellRef::new(0, 0), DateSystem::Year1900).is_err());
}

#[test]
fn a_date_the_system_has_no_serial_for_is_text() {
    let read_1904 =
        |text: &str| Entry::from_text(text, CellRef::new(0, 0), DateSystem::Year1904).unwrap();
    assert!(matches!(read_1904("1/1/1903"), Entry::Text(_)));
    assert_eq!(
        read_1904("1/1/1904"),
        value(Scalar::date32(-24_107), Some("m/d/yyyy"))
    );
}

/// A workbook with one sheet, `Sheet1`.
fn book() -> Workbook {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet1").unwrap();
    workbook
}

#[test]
fn set_entry_keeps_the_style_and_suggests_a_format_only_over_general() {
    let mut workbook = book();
    let at: CellRef = "B2".parse().unwrap();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &["B2".parse().unwrap()], &bold)
        .unwrap();
    workbook.set_entry("Sheet1", at, "12%").unwrap();
    let style = workbook.cell_style("Sheet1", at).unwrap();
    assert!(style.font.bold);
    assert_eq!(style.number_format, "0%");
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at),
        Scalar::from(0.12)
    );
    // The format stays: a date typed now is its serial under `0%`.
    workbook.set_entry("Sheet1", at, "1/2/2024").unwrap();
    assert_eq!(
        workbook.cell_style("Sheet1", at).unwrap().number_format,
        "0%"
    );
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at),
        Scalar::from(45_293.0)
    );
    assert_eq!(
        workbook.display_text("Sheet1", at).unwrap().unwrap().text,
        "4529300%"
    );
    // A number typed into a date cell is the day it counts to.
    workbook
        .set_entry("Sheet1", "C3".parse().unwrap(), "1/2/2024")
        .unwrap();
    workbook
        .set_entry("Sheet1", "C3".parse().unwrap(), "45293")
        .unwrap();
    assert_eq!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .scalar("C3".parse().unwrap()),
        Scalar::date32(19_724)
    );
}

#[test]
fn a_quote_states_quote_prefix_and_any_other_entry_drops_it() {
    let mut workbook = book();
    let at: CellRef = "A1".parse().unwrap();
    workbook.set_entry("Sheet1", at, "'00123").unwrap();
    assert!(workbook.cell_style("Sheet1", at).unwrap().quote_prefix);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at),
        Scalar::from("00123")
    );
    assert_eq!(
        workbook.entry_text("Sheet1", at).unwrap().as_deref(),
        Some("'00123")
    );
    workbook.set_entry("Sheet1", at, "123").unwrap();
    assert!(!workbook.cell_style("Sheet1", at).unwrap().quote_prefix);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at),
        Scalar::from(123.0)
    );
}

#[test]
fn nothing_typed_clears_the_content_and_keeps_the_style() {
    let mut workbook = book();
    let (plain, styled): (CellRef, CellRef) = ("A1".parse().unwrap(), "B1".parse().unwrap());
    workbook.set_entry("Sheet1", plain, "5").unwrap();
    workbook.set_entry("Sheet1", styled, "5%").unwrap();
    let replaced = workbook.set_entry("Sheet1", plain, "").unwrap();
    assert_eq!(
        replaced.map(|cell| cell.value().clone()),
        Some(Scalar::from(5.0))
    );
    assert!(workbook.sheet("Sheet1").unwrap().cell(plain).is_none());
    workbook.set_entry("Sheet1", styled, "").unwrap();
    let cell = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell(styled)
        .unwrap()
        .clone();
    assert!(cell.is_null());
    assert_eq!(
        workbook.cell_style("Sheet1", styled).unwrap().number_format,
        "0%"
    );
    assert_eq!(
        workbook.entry_text("Sheet1", styled).unwrap().as_deref(),
        Some("")
    );
    assert_eq!(
        workbook
            .entry_text("Sheet1", "Z9".parse().unwrap())
            .unwrap(),
        None
    );
}

#[test]
fn a_formula_entry_is_held_and_spelled_back_as_typed() {
    let mut workbook = book();
    let at: CellRef = "C1".parse().unwrap();
    workbook.set_entry("Sheet1", at, "=sum(a1:b1)*2").unwrap();
    let cell = workbook.sheet("Sheet1").unwrap().cell(at).unwrap().clone();
    assert_eq!(cell.formula().unwrap().at(at).to_string(), "SUM(A1:B1)*2");
    assert_eq!(
        workbook.entry_text("Sheet1", at).unwrap().as_deref(),
        Some("=SUM(A1:B1)*2")
    );
    workbook.set_entry("Sheet1", at, "-A1").unwrap();
    assert_eq!(
        workbook.entry_text("Sheet1", at).unwrap().as_deref(),
        Some("=-A1")
    );
    let refusal = workbook.set_entry("Sheet1", at, "=SUM(").unwrap_err();
    assert!(matches!(refusal, yggdryl::Error::Parse { .. }));
    // A refused entry leaves the cell as it was.
    assert_eq!(
        workbook.entry_text("Sheet1", at).unwrap().as_deref(),
        Some("=-A1")
    );
}

#[test]
fn display_text_renders_the_cell_through_its_format() {
    let mut workbook = book();
    for (at, text) in [
        ("A1", "1234.5"),
        ("A2", "$1,234.50"),
        ("A3", "12.5%"),
        ("A4", "1/2/2024"),
        ("A5", "10:30 PM"),
        ("A6", "36:00:00"),
        ("A7", "#N/A"),
        ("A8", "TRUE"),
        ("A9", "text"),
        ("A10", "1 1/2"),
    ] {
        workbook
            .set_entry("Sheet1", at.parse().unwrap(), text)
            .unwrap();
    }
    let shown = |at: &str| {
        workbook
            .display_text("Sheet1", at.parse().unwrap())
            .unwrap()
            .map(|rendered| rendered.text.to_string())
    };
    assert_eq!(shown("A1").as_deref(), Some("1234.5"));
    assert_eq!(shown("A2").as_deref(), Some("$1,234.50"));
    assert_eq!(shown("A3").as_deref(), Some("12.50%"));
    assert_eq!(shown("A4").as_deref(), Some("1/2/2024"));
    assert_eq!(shown("A5").as_deref(), Some("10:30 PM"));
    assert_eq!(shown("A6").as_deref(), Some("36:00:00"));
    assert_eq!(shown("A7").as_deref(), Some("#N/A"));
    assert_eq!(shown("A8").as_deref(), Some("TRUE"));
    assert_eq!(shown("A9").as_deref(), Some("text"));
    assert_eq!(shown("A10").as_deref(), Some("1 1/2"));
    assert_eq!(shown("B1"), None);
}

/// Typing a cell's entry text into it again leaves the cell as it is:
/// value, style and formula.
#[test]
fn typing_a_cell_s_entry_text_again_leaves_the_cell_as_it_is() {
    let texts = [
        "0",
        "5",
        "-5",
        "3.14159",
        "1234567.891",
        "0.000123",
        "123456789012345",
        "1e20",
        "1.5E-10",
        "12345678901234567",
        "0.12345678901234567",
        "12%",
        "12.34%",
        "-0.5%",
        "$1,234.50",
        "$7",
        "(12)",
        "1 1/2",
        "3 7/16",
        "TRUE",
        "false",
        "#N/A",
        "#REF!",
        "hello",
        "  spaced  ",
        "'123",
        "'=A1",
        "'TRUE",
        "1/2/2024",
        "12/31/1999",
        "2/29/2000",
        "10:30",
        "10:30:15",
        "10:30 PM",
        "12:00 AM",
        "7:45:30.25",
        "25:30",
        "36:00:00",
        "100:00:01",
        "1/2/2024 10:30",
        "1/2/2024 10:30:15 PM",
        "=A1+B1",
        "=SUM(A1:A3)",
        "=xlookup(a1,b:b,c:c)",
        "-A1",
        "=\"text\"&A1",
        "-",
        "--",
        "",
    ];
    for start in [
        None,
        Some("0.00"),
        Some("m/d/yyyy"),
        Some("[h]:mm:ss"),
        Some("@"),
    ] {
        for (index, text) in texts.iter().enumerate() {
            // A billionth of a day is below the millisecond a temporal
            // cell holds, so it is not the cell it was typed as.
            if text.contains('E') && start.is_some_and(|code| code.contains(['d', 'h'])) {
                continue;
            }
            let mut workbook = book();
            let at = CellRef::new(u32::try_from(index).unwrap(), 1);
            if let Some(code) = start {
                let patch = StylePatch {
                    number_format: Some(code.into()),
                    ..StylePatch::default()
                };
                workbook
                    .set_style("Sheet1", &[CellRange::new(at, at)], &patch)
                    .unwrap();
            }
            workbook.set_entry("Sheet1", at, text).unwrap();
            let first = workbook.sheet("Sheet1").unwrap().cell(at).cloned();
            let style = workbook.cell_style("Sheet1", at).unwrap();
            let Some(entry) = workbook.entry_text("Sheet1", at).unwrap() else {
                assert!(first.is_none(), "{text:?} under {start:?}");
                continue;
            };
            // A clock or a datetime under a number format is a serial of
            // more digits than an entry spells, which Excel's formula bar
            // rounds alike; a number typed keeps fifteen digits, and spells
            // back as typed.
            let fifteen =
                |number: f64| format!("{number:.14e}").parse::<f64>().ok() == Some(number);
            let temporal = matches!(
                Entry::from_text(text, at, DateSystem::Year1900),
                Ok(Entry::Value { value, .. })
                    if !matches!(value, Scalar::Float64(_) | Scalar::Boolean(_))
            );
            if temporal
                && first
                    .as_ref()
                    .and_then(|cell| cell.value().as_f64())
                    .is_some_and(|number| !fifteen(number))
            {
                continue;
            }
            workbook.set_entry("Sheet1", at, &entry).unwrap();
            let second = workbook.sheet("Sheet1").unwrap().cell(at).cloned();
            assert_eq!(
                first, second,
                "{text:?} under {start:?}: typed back as {entry:?}"
            );
            assert_eq!(
                style,
                workbook.cell_style("Sheet1", at).unwrap(),
                "{text:?} under {start:?}: typed back as {entry:?}"
            );
            assert_eq!(
                workbook.entry_text("Sheet1", at).unwrap(),
                Some(entry.clone()),
                "{text:?} under {start:?}"
            );
        }
    }
}

/// A cell put by value rather than typed spells an entry that types back
/// as its value: text that would read as anything else after a `'`, which
/// then states `quotePrefix`; a temporal whose style does not read as it -
/// a cell built in memory holds the default style - as the day or clock it
/// shows.
#[test]
fn a_cell_put_by_value_types_back_as_its_value() {
    let values = [
        Scalar::from("TRUE"),
        Scalar::from("007"),
        Scalar::from("=A1"),
        Scalar::from("=SUM("),
        Scalar::from("1/2/2024"),
        Scalar::from("#N/A"),
        Scalar::from("-5"),
        Scalar::from("-"),
        Scalar::from("'abc"),
        Scalar::from("  5  "),
        Scalar::from("12%"),
        Scalar::from("plain"),
        Scalar::from("with space"),
        Scalar::from(1.5),
        Scalar::from(true),
        Scalar::date32(19_724),
        time(37_815_000),
        instant(1_704_191_415_000),
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap(),
    ];
    for (row, value) in values.into_iter().enumerate() {
        let mut workbook = book();
        let at = CellRef::new(u32::try_from(row).unwrap(), 2);
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .set_cell(at, value.clone())
            .unwrap();
        let style = workbook.cell_style("Sheet1", at).unwrap();
        let entry = workbook.entry_text("Sheet1", at).unwrap().unwrap();
        workbook
            .set_entry("Sheet1", at, &entry)
            .unwrap_or_else(|error| panic!("{value:?} spelled {entry:?}: {error}"));
        assert_eq!(
            workbook.sheet("Sheet1").unwrap().scalar(at),
            value,
            "{value:?} spelled {entry:?}"
        );
        let quoted = matches!(value, Scalar::Utf8String(_)) && entry.starts_with('\'');
        assert_eq!(
            workbook.cell_style("Sheet1", at).unwrap(),
            yggdryl::excel::CellStyle {
                quote_prefix: quoted,
                ..style
            },
            "{value:?} spelled {entry:?}"
        );
        assert_eq!(
            quoted,
            matches!(value, Scalar::Utf8String(_))
                && !matches!(value.as_str(), Some("plain" | "with space" | "-")),
            "{value:?} spelled {entry:?}"
        );
    }
}

#[test]
fn entry_text_spells_each_value_as_it_would_be_typed() {
    let mut workbook = book();
    for (at, typed, expected) in [
        ("A1", "1/2/2024", "1/2/2024"),
        ("A2", "10:30", "10:30:00 AM"),
        ("A3", "1/2/2024 22:05", "1/2/2024 10:05:00 PM"),
        ("A4", "25:30", "25:30:00"),
        ("A5", "12.5%", "12.5%"),
        ("A6", "$1,234.50", "1234.5"),
        ("A7", "0.1", "0.1"),
        ("A8", "123456789012345678", "123456789012345000"),
        ("A11", "12345678901234567890", "12345678901234500000"),
        ("A12", "123456789012345678901", "1.23456789012345E+20"),
        ("A13", "0.000000001", "0.000000001"),
        ("A9", "true", "TRUE"),
        ("A10", "=a1", "=A1"),
    ] {
        let at: CellRef = at.parse().unwrap();
        workbook.set_entry("Sheet1", at, typed).unwrap();
        assert_eq!(
            workbook.entry_text("Sheet1", at).unwrap().as_deref(),
            Some(expected),
            "{typed:?}"
        );
    }
}

/// Excel keeps fifteen significant digits of a number typed, every digit
/// past them a zero (Microsoft KB 269370): the sixteenth is dropped, not
/// rounded, and what is kept spells back as typed.
#[test]
fn a_number_typed_keeps_fifteen_significant_digits() {
    for (text, expected) in [
        ("1234567890123456", 1_234_567_890_123_450.0),
        ("4111111111111119", 4_111_111_111_111_110.0),
        ("12345678901234567", 12_345_678_901_234_500.0),
        ("0.12345678901234567", 0.123_456_789_012_345),
        ("1.23456789012345678E+20", 1.234_567_890_123_45e20),
        ("-9,999,999,999,999,999", -9_999_999_999_999_990.0),
        ("12.3456789012345678%", 0.123_456_789_012_345),
        ("123456789012345", 123_456_789_012_345.0),
    ] {
        let Entry::Value { value, .. } = read(text) else {
            panic!("{text:?}: a number");
        };
        assert_eq!(value, Scalar::from(expected), "{text:?}");
    }
    let mut workbook = book();
    for (row, text) in [
        "12345678901234567",
        "0.12345678901234567",
        "4111111111111119",
    ]
    .into_iter()
    .enumerate()
    {
        let at = CellRef::new(u32::try_from(row).unwrap(), 0);
        workbook.set_entry("Sheet1", at, text).unwrap();
        let typed = workbook.sheet("Sheet1").unwrap().scalar(at);
        let entry = workbook.entry_text("Sheet1", at).unwrap().unwrap();
        workbook.set_entry("Sheet1", at, &entry).unwrap();
        assert_eq!(
            workbook.sheet("Sheet1").unwrap().scalar(at),
            typed,
            "{text:?} spelled {entry:?}"
        );
    }
}

/// Grouping and percent are entry grammar; the decimal core only receives
/// proven digit parts. Exponent intake keeps the entry's i32 domain.
#[test]
fn decimal_entry_parts_keep_value_format_and_exponent_domain() {
    for (text, expected, format) in [
        ("0000.00012345678901234567", 0.000_123_456_789_012_345, None),
        ("00012345678901234567", 12_345_678_901_234_500.0, None),
        ("9,999,999,999,999,999.99", 9_999_999_999_999_990.0, None),
        (
            "0.00000012345678901234567%",
            1.234_567_890_123_45e-9,
            Some("0.00%"),
        ),
        (
            "1.23456789012345678E+20",
            1.234_567_890_123_45e20,
            Some("0.00E+00"),
        ),
    ] {
        assert_eq!(
            read(text),
            value(Scalar::from(expected), format),
            "{text:?}"
        );
    }
    for text in ["1e2147483648", "1e-2147483649", "1e-2%", "$1e2", "1,000e2"] {
        assert_eq!(read(text), Entry::Text(text.into()), "{text:?}");
    }
}

/// In a cell formatted as text (`@`) whatever is typed stays text as
/// typed, a formula that would not read included; nothing typed clears it,
/// and a `'` still states `quotePrefix`.
#[test]
fn what_is_typed_into_a_text_cell_stays_text() {
    let mut workbook = book();
    let text = StylePatch {
        number_format: Some("@".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &["A1:A20".parse().unwrap()], &text)
        .unwrap();
    for (row, typed) in [
        "007", "1/2/2024", "=1+1", "=SUM(", "TRUE", "#N/A", "12%", "-A1", "  5  ",
    ]
    .into_iter()
    .enumerate()
    {
        let at = CellRef::new(u32::try_from(row).unwrap(), 0);
        workbook.set_entry("Sheet1", at, typed).unwrap();
        let cell = workbook.sheet("Sheet1").unwrap().cell(at).unwrap().clone();
        assert_eq!(cell.value(), &Scalar::from(typed), "{typed:?}");
        assert!(cell.formula().is_none(), "{typed:?}");
        let style = workbook.cell_style("Sheet1", at).unwrap();
        assert_eq!(style.number_format, "@", "{typed:?}");
        assert!(!style.quote_prefix, "{typed:?}");
        assert_eq!(
            workbook.entry_text("Sheet1", at).unwrap().as_deref(),
            Some(typed),
            "{typed:?}"
        );
    }
    let at: CellRef = "A15".parse().unwrap();
    workbook.set_entry("Sheet1", at, "'abc").unwrap();
    assert!(workbook.cell_style("Sheet1", at).unwrap().quote_prefix);
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at),
        Scalar::from("abc")
    );
    workbook.set_entry("Sheet1", at, "").unwrap();
    assert!(workbook.sheet("Sheet1").unwrap().scalar(at).is_null());
}

/// A refused entry changes nothing: not the cell, not the styles, not
/// whether the workbook is saved.
#[test]
fn a_refused_entry_changes_nothing() {
    let mut saved = book();
    let mut workbook = Workbook::from_bytes(saved.into_bytes().unwrap()).unwrap();
    let at: CellRef = "A1".parse().unwrap();
    let before = workbook.style_sheet().unwrap().len();
    for text in [format!("'{}", "x".repeat(40_000)), "=SUM(".to_owned()] {
        assert!(workbook.set_entry("Sheet1", at, &text).is_err());
        assert_eq!(workbook.style_sheet().unwrap().len(), before);
        assert!(!workbook.is_dirty());
        assert!(workbook.sheet("Sheet1").unwrap().cell(at).is_none());
    }
    // The styles part a save writes is the one read: nothing was appended.
    saved = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(saved.style_sheet().unwrap().len(), before);
}
