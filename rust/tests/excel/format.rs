//! `rust/src/excel/format.rs`: `FormatCode` - a number format code read once, what it says a number is, the text a value displays as under it - and `Rendered`.

use yggdryl::Scalar;
use yggdryl::excel::{
    CellRange, CellRef, DateSystem, FormatCode, NumberFormat, Rendered, StyleId, StylePatch,
    Workbook,
};

use crate::excel_package::one_sheet;

/// A number.
fn n(value: f64) -> Scalar {
    Scalar::from(value)
}

/// A text.
fn t(text: &str) -> Scalar {
    Scalar::from(text)
}

/// A boolean.
fn b(value: bool) -> Scalar {
    Scalar::from(value)
}

/// What `value` displays as under `code`, in the 1900 date system.
#[track_caller]
fn render(code: &str, value: &Scalar) -> String {
    let format = FormatCode::from_code(code).unwrap_or_else(|error| panic!("{code}: {error}"));
    rendered(&format, value, DateSystem::Year1900)
        .text
        .to_string()
}

/// Render once for the assertion and, during export, retain that same result.
#[track_caller]
fn rendered(format: &FormatCode, value: &Scalar, system: DateSystem) -> Rendered {
    let shown = format.render(value, system);
    record_rendered(format.code(), value, system, &shown);
    shown
}

/// Export runs the original assertions before writing any answers. Thread-local
/// collection cannot include renders from tests the harness runs concurrently.
#[derive(Default)]
struct FormatExport {
    test: &'static str,
    counts: std::collections::BTreeMap<&'static str, usize>,
    cases: Vec<serde_json::Value>,
}

thread_local! {
    static FORMAT_EXPORT: std::cell::RefCell<Option<FormatExport>> = const {
        std::cell::RefCell::new(None)
    };
}

#[track_caller]
fn record_rendered(code: &str, value: &Scalar, system: DateSystem, shown: &Rendered) {
    let line = std::panic::Location::caller().line();
    FORMAT_EXPORT.with(|held| {
        let mut held = held.borrow_mut();
        let Some(export) = held.as_mut() else { return };
        let (value_kind, value_json) = if value == &Scalar::Null {
            ("blank", serde_json::Value::Null)
        } else if let Some(flag) = value.as_bool() {
            ("boolean", serde_json::json!(flag))
        } else if let Some(text) = value.as_str() {
            ("text", serde_json::json!(text))
        } else {
            let number = value
                .as_f64()
                .or_else(|| value.as_i128().map(|number| number as f64))
                .or_else(|| value.as_u128().map(|number| number as f64))
                .or_else(|| system.serial_of(value).unwrap().map(|(serial, _)| serial))
                .unwrap_or_else(|| panic!("unrepresented format input: {value:?}"));
            if number.is_finite() {
                ("number", serde_json::json!(number))
            } else {
                let spelling = if number.is_nan() {
                    "NaN"
                } else if number.is_sign_positive() {
                    "Infinity"
                } else {
                    "-Infinity"
                };
                ("nonfinite", serde_json::json!(spelling))
            }
        };
        let index = export
            .counts
            .get_mut(export.test)
            .expect("registered format test");
        *index += 1;
        export.cases.push(serde_json::json!({
            "id": format!("{}:{index:03}", export.test),
            "source": "rust/tests/excel/format.rs",
            "test": export.test,
            "render_index": *index,
            "line": line,
            "scalar_kind": value.kind().to_string(),
            "value_kind": value_kind,
            "value": value_json,
            "code": code,
            "date_system": system.as_str(),
            "expected": shown.text.as_str(),
            "expected_color": shown.color,
            "expected_fill": shown.fill,
            "expected_shorter": shown.shorter.iter().map(|text| text.as_str()).collect::<Vec<_>>()
        }));
    });
}

/// One registry owns both ordinary tests and the complete executable export.
/// Adding a test here automatically includes its rendering calls in the export.
macro_rules! format_tests {
    ($($(#[$attribute:meta])* fn $name:ident() $body:block)*) => {
        $($(#[$attribute])* fn $name() $body)*

        fn execute_format_checks() {
            $(
                FORMAT_EXPORT.with(|held| {
                    let mut held = held.borrow_mut();
                    let export = held.as_mut().expect("format export active");
                    export.test = stringify!($name);
                    export.counts.insert(export.test, 0);
                });
                $name();
            )*
        }
    };
}

/// What a code says a number is.
fn kind(code: &str) -> NumberFormat {
    FormatCode::from_code(code).unwrap().kind()
}

/// The column letters of zero-based `at`, for the first 26.
fn column(at: usize) -> char {
    char::from(b'A' + u8::try_from(at).unwrap())
}

format_tests! {

/// Every row was rendered by LibreOffice Calc as well - each code set on a
/// cell of a workbook openpyxl wrote, exported as shown - and it displays
/// what Excel does: digits and their rounding half away from zero on the
/// fifteen-digit form, thousands and their scaling, percentages,
/// scientific and engineering notation, fractions, literals, the space and
/// fill characters, currency tags, conditions, sections, text sections,
/// dates, clocks and elapsed times.
#[test]
fn every_rendering_libreoffice_calc_agrees_with() {
    let rows: &[(&str, Scalar, &str)] = &[
        ("0", n(1234.5), "1235"),
        ("0", n(-1234.5), "-1235"),
        ("0", n(0.5), "1"),
        ("0", n(0.49), "0"),
        ("0.00", n(1.005), "1.01"),
        ("0.00", n(1234.5678), "1234.57"),
        ("#,##0", n(1234567.0), "1,234,567"),
        ("#,##0", n(0.0), "0"),
        ("#,##0", n(12.0), "12"),
        ("#,##0.00", n(1234567.891), "1,234,567.89"),
        ("#,##0.00", n(-0.5), "-0.50"),
        ("#,##0", n(-1234567.0), "-1,234,567"),
        ("#.##", n(0.5), ".5"),
        ("0.0#", n(1.5), "1.5"),
        ("0.0#", n(1.25), "1.25"),
        ("0.0#", n(1.255), "1.26"),
        ("00000", n(123.0), "00123"),
        ("000-00-0000", n(123456789.0), "123-45-6789"),
        ("#,##0,", n(1234567.0), "1,235"),
        ("#,##0,,", n(1234567890.0), "1,235"),
        ("0.0,,", n(1234567.0), "1.2"),
        ("0%", n(0.1234), "12%"),
        ("0.00%", n(0.1234), "12.34%"),
        ("0.0%", n(-0.5), "-50.0%"),
        ("0%", n(1.0), "100%"),
        ("0.00E+00", n(12345.0), "1.23E+04"),
        ("0.00E+00", n(0.000123), "1.23E-04"),
        ("0.00E+00", n(0.0), "0.00E+00"),
        ("0.00E-00", n(12345.0), "1.23E04"),
        ("0.00E-00", n(0.000123), "1.23E-04"),
        ("##0.0E+0", n(12345.0), "12.3E+3"),
        ("##0.0E+0", n(0.0012345), "1.2E-3"),
        ("##0.0E+0", n(1234567.0), "1.2E+6"),
        ("0.0E+0", n(999999.0), "1.0E+6"),
        ("# ?/?", n(0.5), " 1/2"),
        ("# ?/?", n(3.0), "3    "),
        ("# ?/?", n(3.5), "3 1/2"),
        ("# ?/?", n(0.0), "0    "),
        ("# ??/??", n(std::f64::consts::PI), "3  1/7 "), // Excel: full convergent, not semiconvergent.
        ("# ??/??", n(0.333), "  1/3 "),
        ("?/?", n(0.75), "3/4"),
        ("# ?/8", n(0.3), " 2/8"),
        ("# ?/4", n(2.75), "2 3/4"),
        ("??/??", n(1.5), " 3/2 "),
        ("# ???/???", n(std::f64::consts::PI), "3  16/113"),
        ("$#,##0_);($#,##0)", n(1234.0), "$1,234 "),
        ("$#,##0_);($#,##0)", n(-1234.0), "($1,234)"),
        ("$#,##0.00_);[Red]($#,##0.00)", n(-1234.5), "($1,234.50)"),
        ("#,##0 ;(#,##0)", n(-5.0), "(5)"),
        ("#,##0.00;(#,##0.00)", n(5.0), "5.00"),
        ("0;-0;\"zero\"", n(0.0), "zero"),
        ("0;-0;\"zero\"", n(-3.0), "-3"),
        ("0;-0;\"zero\";@", n(3.0), "3"),
        ("\"Total: \"0.00", n(5.0), "Total: 5.00"),
        ("0.00\" kg\"", n(5.0), "5.00 kg"),
        ("\\$0.00", n(5.0), "$5.00"),
        ("0_);(0)", n(5.0), "5 "),
        ("[Red]0.00", n(-5.0), "-5.00"),
        ("[Blue]0;[Red]-0", n(-5.0), "-5"),
        ("[>=100]0;[<100]0.00", n(150.0), "150"),
        ("[>=100]0;[<100]0.00", n(50.0), "50.00"),
        ("[<=100]0;[>100]0.0", n(-5.0), "-5"),
        ("[>100]0;0.00", n(-5.0), "-5.00"), // Excel: fallback keeps the implicit minus.
        ("[<0]\"neg \"0;0", n(-5.0), "neg 5"),
        ("[=1]\"one\";0", n(1.0), "one"),
        ("[=1]\"one\";0", n(2.0), "2"),
        ("[$€-407] #,##0.00", n(1234.5), "€ 1,234.50"),
        ("#,##0.00 [$€-407]", n(1234.5), "1,234.50 €"),
        ("[$USD] 0", n(5.0), "USD 5"),
        ("General", n(1234.5), "1234.5"),
        ("General", n(0.1), "0.1"),
        ("General", n(-12345678901.0), "-12345678901"),
        ("m/d/yyyy", n(45292.0), "1/1/2024"),
        ("d-mmm-yy", n(45292.0), "1-Jan-24"),
        ("d-mmm", n(45292.0), "1-Jan"),
        ("mmm-yy", n(45292.0), "Jan-24"),
        ("h:mm AM/PM", n(0.4375), "10:30 AM"),
        ("h:mm:ss AM/PM", n(0.75), "6:00:00 PM"),
        ("h:mm", n(0.4375), "10:30"),
        ("h:mm:ss", n(0.4375), "10:30:00"),
        ("m/d/yyyy h:mm", n(45292.4375), "1/1/2024 10:30"),
        ("[h]:mm:ss", n(1.5), "36:00:00"),
        ("yyyy-mm-dd", n(45292.0), "2024-01-01"),
        ("dddd, mmmm d, yyyy", n(45292.0), "Monday, January 1, 2024"),
        ("ddd", n(45292.0), "Mon"),
        ("mmmmm", n(45292.0), "J"),
        ("yy", n(45292.0), "24"),
        ("hh:mm:ss.000", n(0.123456789), "02:57:46.667"),
        ("[mm]:ss", n(0.0625), "90:00"),
        ("[ss]", n(0.0625), "5400"),
        ("[h]", n(2.75), "66"),
        ("h:mm", n(0.99999), "23:59"),
        ("m/d/yyyy", n(61.0), "3/1/1900"),
        ("dddd", n(1.0), "Sunday"),
        ("dddd", n(61.0), "Thursday"),
        ("h AM/PM", n(0.0), "12 AM"),
        ("h AM/PM", n(0.5), "12 PM"),
        (
            "[$-F800]dddd, mmmm dd, yyyy",
            n(45292.0),
            "Monday, January 1, 2024",
        ),
        ("@", t("text"), "text"),
        ("\"x\"@\"y\"", t("text"), "xtexty"),
        ("0;0;0;\"t:\"@", t("abc"), "t:abc"),
        ("0.00", t("abc"), "abc"),
        ("@", n(5.0), "5"),
        ("*-0", n(5.0), "5"),
        ("0*-", n(5.0), "5"),
        (
            "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)",
            n(1234.0),
            " 1,234 ",
        ),
        ("_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)", n(0.0), " - "),
        (
            "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
            n(-1234.5),
            " $(1,234.50)",
        ),
        (
            "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
            n(0.0),
            " $-   ",
        ),
        ("0.00;;", n(-3.0), ""),
        (";;;", n(5.0), ""),
        ("\"yes\"", n(-5.0), "-yes"), // Excel: a literal section still has an implicit minus.
        ("0.000", n(2.0005), "2.001"),
        ("0.000", n(1000000000000000.0), "1000000000000000.000"),
        ("0", n(1e20), "100000000000000000000"),
        ("#", n(0.4), ""),
        ("?", n(0.0), " "),
        ("??.??", n(1.5), " 1.5 "),
        ("0.0?", n(3.0), "3.0 "),
        ("#,###", n(0.0), ""),
        ("#,###.##", n(1234.5), "1,234.5"),
        ("[>100]\"big\";[<-100]\"small\";0", n(-5.0), "-5"), // Excel: third fallback keeps its sign.
        ("[>100]\"big\";[<-100]\"small\";0", n(-500.0), "small"),
        ("[<=-1]0;[>=1]0;0.00", n(-5.0), "5"), // Excel: a negative-only bound selects magnitude.
        ("[Color10]0", n(5.0), "5"),
        ("[Green]0;[Magenta]0", n(-2.0), "2"),
        ("[Cyan]@", t("x"), "x"),
        ("\"$\"#,##0.00", n(1234.567), "$1,234.57"),
        ("\"$\"#,##0.00", n(-1234.567), "-$1,234.57"),
        ("\"$\"#,##0", n(0.5), "$1"),
        ("#,##0.000", n(-0.0005), "-0.001"),
        ("0.0", n(0.05), "0.1"),
        ("0.0", n(0.15), "0.2"),
        ("0.0", n(0.25), "0.3"),
        ("0.0", n(2.675), "2.7"),
        ("0.00", n(1e-10), "0.00"),
        ("0.00E+00", n(1.5e-10), "1.50E-10"),
        ("0.00E+00", n(1e100), "1.00E+100"),
        ("0.00E+00", n(-12345.0), "-1.23E+04"),
        ("0.0E+00", n(9.96), "1.0E+01"),
        ("0E+0", n(12345.0), "1E+4"),
        ("#E+0", n(0.5), "5E-1"),
        ("# ?/?", n(-1.25), "-1 1/4"),
        ("# ?/?", n(0.95), "1    "),
        ("# ?/?", n(1.97), "2    "),
        ("# ??/??", n(0.01), "0      "), // Excel: 1/100 exceeds the bound; last convergent is zero.
        ("0 0/0", n(1.5), "1 1/2"),
        ("# ?/10", n(0.33), " 3/10"),
        ("# ?/100", n(0.333), " 33/100"),
        ("?/?", n(0.0), "0/1"),
        ("0.0%;(0.0%)", n(-0.1234), "(12.3%)"),
        ("#,##0.00%", n(12.3456), "1,234.56%"),
        ("000.000", n(1.5), "001.500"),
        ("#,##0.00_);(#,##0.00)", n(0.0), "0.00 "),
        ("0.00;-0.00;0.00", n(0.0), "0.00"),
        ("m/d/yy", n(45292.0), "1/1/24"),
        ("mm/dd/yyyy", n(45657.0), "12/31/2024"),
        ("d mmmm yyyy", n(45352.0), "1 March 2024"),
        ("dddd d mmm", n(45353.0), "Saturday 2 Mar"),
        ("mmm d, yyyy", n(45000.75), "Mar 15, 2023"),
        ("yyyy", n(36526.0), "2000"),
        ("hh:mm:ss", n(0.5), "12:00:00"),
        ("hh:mm", n(0.0), "00:00"),
        ("[hh]:mm", n(0.25), "06:00"),
        ("[h]:mm", n(10.5), "252:00"),
        ("[mm]", n(1.0), "1440"),
        ("h:mm:ss.00", n(0.5000001), "12:00:00.01"),
        ("ss.0", n(1e-5), "00.9"),
        ("m", n(45292.0), "1"),
        ("mm", n(45292.0), "01"),
        ("mmm", n(45292.0), "Jan"),
        ("mmmm", n(45292.0), "January"),
        ("h:m", n(0.4375), "10:30"),
        ("h m", n(0.4375), "10 30"),
        ("[h]:mm:ss", n(0.0001), "0:00:09"),
        ("d", n(45292.0), "1"),
        ("dd", n(45292.0), "01"),
        ("hh AM/PM", n(0.75), "06 PM"),
        ("hh:mm a/p", n(0.1), "02:24 a"),
        (
            "yyyy-mm-dd hh:mm:ss.000",
            n(45292.123456789),
            "2024-01-01 02:57:46.667",
        ),
        ("m/d/yyyy", n(2958465.0), "12/31/9999"),
        ("\"x\"0\"y\"", n(5.0), "x5y"),
        ("0 \"units\"", n(1.0), "1 units"),
        ("(0)", n(5.0), "(5)"),
        ("-0", n(5.0), "-5"),
        ("+0", n(5.0), "+5"),
        ("0.00 \\€", n(5.0), "5.00 €"),
        ("@\" suffix\"", t("abc"), "abc suffix"),
        ("\"pre \"@", t("abc"), "pre abc"),
        ("@;@", t("abc"), "abc"),
        ("0;@", t("abc"), "abc"),
        ("0;\"neg\";\"zero\";\"text\"", t("abc"), "text"),
        ("0;;;", t("abc"), ""),
        ("0", t("abc"), "abc"),
        ("General", n(0.0), "0"),
        ("General", n(-0.5), "-0.5"),
        ("General", n(100.0), "100"),
        ("General", n(1e-5), "0.00001"),
        ("General", n(99999999999.0), "99999999999"),
        ("General\" kg\"", n(5.0), "5 kg"),
        ("[Red]General", n(-5.0), "-5"),
        ("General;[Red]-General", n(-5.0), "-5"),
        ("#,##0.00;[Red]-#,##0.00", n(-0.001), "-0.00"),
        ("#", n(123.5), "124"),
        ("#,#", n(1234567.0), "1,234,567"),
        ("0,000", n(5.0), "0,005"),
        ("0.###", n(1.5), "1.5"),
        ("#,##0.###", n(1234.56789), "1,234.568"),
        ("$#,##0.00", n(1234.5), "$1,234.50"),
        ("$ #,##0.00", n(-1234.5), "-$ 1,234.50"),
        ("£#,##0", n(1234.0), "£1,234"),
        ("¥#,##0", n(1234.0), "¥1,234"),
        ("[<-100]0;0", n(-500.0), "500"), // Excel: any nonpositive upper bound selects magnitude.
        ("[<-100]\"a\"0;0", n(-500.0), "a500"), // Excel: the bound suppresses the implicit minus.
        ("0.00;(0.00)", n(-std::f64::consts::PI), "(3.14)"),
        ("#,##0.00_);[Red](#,##0.00)", n(-1234.5), "(1,234.50)"),
        ("#,##0_);[Red](#,##0)", n(1234.5), "1,235 "),
        ("0.00E+00", n(6.02214076e23), "6.02E+23"),
        ("0.000E+00", n(1.234e-6), "1.234E-06"),
        ("#,##0.0", n(999.95), "1,000.0"),
        ("#,##0.0", n(999.94), "999.9"),
        ("0", n(2.5), "3"),
        ("0", n(-2.5), "-3"),
        ("0", n(3.5), "4"),
        ("0.00", n(0.125), "0.13"),
        ("0.00", n(0.375), "0.38"),
        ("0.0", n(1.45), "1.5"),
        ("?/16", n(0.2), "3/16"),
        ("yyyy-mm-dd", n(367.0), "1901-01-01"),
        ("ddd, mmm d", n(44927.0), "Sun, Jan 1"),
        ("mmmm yyyy", n(44927.5), "January 2023"),
        ("h:mm", n(0.999), "23:58"),
        ("[h]:mm:ss", n(2.000011574), "48:00:01"),
        ("[m]:ss", n(0.1), "144:00"),
        ("h:mm:ss", n(0.5), "12:00:00"),
        ("000000", n(123.4), "000123"),
        ("0000.00", n(-12.3), "-0012.30"),
        ("#,##0;-#,##0;-", n(0.0), "-"),
        ("#,##0;-#,##0;-", n(-7.0), "-7"),
        ("0.0;0.0;\"-\"", n(0.0), "-"),
        (
            "\"Balance: \"$#,##0.00;\"Owed: \"$#,##0.00",
            n(-50.0),
            "Owed: $50.00",
        ),
        ("[Blue]#,##0;[Red]-#,##0;[Black]0", n(0.0), "0"),
    ];
    assert!(rows.len() >= 150);
    for (code, value, expected) in rows {
        assert_eq!(render(code, value), *expected, "{code} of {value:?}");
    }
}

/// Recorded by Excel16 build20430 through en-US LCID1033, with original
/// format codes. Every formerly assumed answer here was observed in Excel.
#[test]
fn desktop_excel_confirms_format_signs_time_case_and_fraction_convergents() {
    let observed: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/format_excel_confirmed.json"
    ))
    .unwrap();
    assert_eq!(observed["schema_version"], 1);
    assert_eq!(observed["excel"]["version"], "16.0");
    let cases = observed["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        assert_eq!(
            render(case["code"].as_str().unwrap(), &n(case["value"].as_f64().unwrap())),
            case["actual"].as_str().unwrap(),
            "Excel confirmed {}",
            case["id"]
        );
    }
}

/// The original Excel/LibreOffice disagreement corpus. Desktop-confirmed
/// corrections are identified below; locale substitutions remain en-US policy.
#[test]
fn where_libreoffice_differs_excel_is_followed() {
    for (code, value, expected, why) in [
        (
            "0.00",
            n(-0.001),
            "0.00",
            "Excel16 build20430 confirmed an implicit minus disappears when rounding yields zero",
        ),
        ("0.0", n(-0.04), "0.0", "the same Excel-confirmed rounded-zero rule"),
        ("0%", n(-0.004), "0%", "the same Excel-confirmed rule after percent scaling"),
        (
            "#.##",
            n(1.0),
            "1.",
            "a point with no digit after it stays; LibreOffice drops it",
        ),
        (
            "General",
            n(1_234_567_890_123.0),
            "1.23457E+12",
            "General spells a number in eleven characters; LibreOffice's export writes them all",
        ),
        (
            "General",
            n(std::f64::consts::PI),
            "3.141592654",
            "the same",
        ),
        ("General", n(2.5e15), "2.5E+15", "the same"),
        ("General", n(12_345_678_901.5), "12345678902", "the same"),
        (
            "mm:ss",
            n(0.0123),
            "17:43",
            "a clock is rounded to the unit it shows; LibreOffice truncates",
        ),
        ("m:s", n(0.0123), "17:43", "the same"),
        ("h:mm:ss", n(0.000_011_574), "0:00:01", "the same"),
        ("mm:ss.00", n(0.000_011_574_07), "00:01.00", "the same"),
        (
            "h:mm:ss",
            n(0.999_999_99),
            "0:00:00",
            "the same, carried past midnight",
        ),
        (
            "yyyy-mm-dd hh:mm:ss",
            n(45_292.999_999),
            "2024-01-02 00:00:00",
            "the same, carried into the next day",
        ),
        (
            "m/d/yyyy",
            n(60.0),
            "2/29/1900",
            "the 1900 system has a 29 February 1900; LibreOffice does not",
        ),
        ("m/d/yyyy", n(1.0), "1/1/1900", "serial 1 is 1 January 1900"),
        ("m/d/yyyy", n(1.5), "1/1/1900", "the same"),
        ("yyyy-mm-dd", n(59.0), "1900-02-28", "the same"),
        ("m/d/yyyy", n(0.0), "1/0/1900", "serial 0 is 0 January 1900"),
        (
            "h:mm am/pm",
            n(0.8),
            "7:12 PM",
            "Excel16 build20430 confirmed full AM/PM is uppercase regardless of code case",
        ),
        ("h:mm A/P", n(0.8), "7:12 P", "the one-letter form keeps its case"),
        (
            "[$-F400]h:mm:ss AM/PM",
            n(0.75),
            "6:00:00 PM",
            "[$-F400] is en-US's long time, h:mm:ss AM/PM",
        ),
        (
            "0.00",
            b(true),
            "TRUE",
            "a boolean shows as TRUE or FALSE whatever the format",
        ),
        (
            "mmss.0",
            n(0.0123),
            "1742.7",
            "the code as written; LibreOffice shows its own built-in 47",
        ),
    ] {
        assert_eq!(render(code, &value), expected, "{code} of {value:?}: {why}");
    }
}

#[test]
fn a_serial_no_day_spells_fills_the_cell_with_hashes() {
    for (code, value) in [
        ("m/d/yyyy", -1.0),
        ("h:mm", -0.5),
        ("[h]:mm", -0.5),
        ("m/d/yyyy", 2_958_466.0),
    ] {
        let shown = rendered(&FormatCode::from_code(code).unwrap(), &n(value), DateSystem::Year1900);
        assert_eq!(shown.text, "", "{code} of {value}");
        assert_eq!(shown.fill, Some(('#', 0)), "{code} of {value}");
    }
}

#[test]
fn a_1904_workbook_counts_its_days_from_1904() {
    let render_1904 = |code: &str, value: f64| {
        rendered(&FormatCode::from_code(code).unwrap(), &n(value), DateSystem::Year1904)
            .text
            .to_string()
    };
    assert_eq!(render_1904("m/d/yyyy", 0.0), "1/1/1904");
    assert_eq!(render_1904("dddd", 0.0), "Friday");
    assert_eq!(render_1904("m/d/yyyy", 59.0), "2/29/1904");
    assert_eq!(render_1904("yyyy-mm-dd", 43_830.0), "2024-01-01");
    assert_eq!(render_1904("m/d/yyyy h:mm", 1.5), "1/2/1904 12:00");
}

#[test]
fn date_rendering_uses_the_date_systems_last_serial() {
    let code = FormatCode::from_code("m/d/yyyy").unwrap();
    for (system, last) in [
        (DateSystem::Year1900, 2_958_465.0),
        (DateSystem::Year1904, 2_957_003.0),
    ] {
        assert_eq!(rendered(&code, &n(last), system).text, "12/31/9999");
        let shown = rendered(&code, &n(last + 1.0), system);
        assert_eq!(shown.fill, Some(('#', 0)), "{system:?}: {shown:?}");
        assert!(shown.text.is_empty());
    }
}

#[test]
fn a_negative_1904_date_renders_signed_next_day() {
    let code = FormatCode::from_code("m/d/yyyy").unwrap();
    let shown = rendered(&code, &n(-1.0), DateSystem::Year1904);
    assert_eq!(shown.text, "-1/2/1904");
    assert_eq!(shown.fill, None);
    let old_system = rendered(&code, &n(-1.0), DateSystem::Year1900);
    assert_eq!(old_system.text, "");
    assert_eq!(old_system.fill, Some(('#', 0)));
}

#[test]
fn a_temporal_value_displays_as_its_serial() {
    use yggdryl::{TimeUnit, Timezone};

    let day = Scalar::date32(19_724);
    assert_eq!(render("m/d/yyyy", &day), "1/2/2024");
    assert_eq!(render("0.00", &day), "45293.00");
    assert_eq!(render("General", &day), "45293");
    let at = Scalar::datetime64(1_704_191_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap();
    assert_eq!(render("m/d/yyyy h:mm", &at), "1/2/2024 10:30");
    let clock = Scalar::time32(37_800_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap();
    assert_eq!(render("h:mm AM/PM", &clock), "10:30 AM");
    let elapsed = Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap();
    assert_eq!(render("[h]:mm:ss", &elapsed), "36:00:00");
    assert_eq!(render("#,##0", &Scalar::from(1_234_567_i64)), "1,234,567");
    assert_eq!(render("0.00", &Scalar::Null), "");
}

#[test]
fn a_section_names_its_colour() {
    let code = FormatCode::from_code("[Blue]#,##0;[Red]-#,##0;[Color10]0").unwrap();
    let colour = |value: f64| rendered(&code, &n(value), DateSystem::Year1900).color;
    assert_eq!(colour(5.0), Some(0x00_00FF));
    assert_eq!(colour(-5.0), Some(0xFF_0000));
    assert_eq!(colour(0.0), Some(0x00_8000));
    assert_eq!(
        rendered(&FormatCode::from_code("0.00").unwrap(), &n(1.0), DateSystem::Year1900)
            .color,
        None
    );
    assert_eq!(
        rendered(&FormatCode::from_code("[Magenta]@").unwrap(), &t("x"), DateSystem::Year1900)
            .color,
        Some(0xFF_00FF)
    );
}

#[test]
fn a_fill_character_is_returned_with_where_it_repeats() {
    let shown = |code: &str, value: &Scalar| {
        rendered(&FormatCode::from_code(code).unwrap(), value, DateSystem::Year1900)
    };
    // The accounting layout: the symbol at the left, the number at the right.
    let money = shown("\"$\"* #,##0.00", &n(1234.5));
    assert_eq!(money.text, "$1,234.50");
    assert_eq!(money.fill, Some((' ', 1)));
    let accounting = shown(
        "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
        &n(-1234.5),
    );
    assert_eq!(accounting.text, " $(1,234.50)");
    assert_eq!(accounting.fill, Some((' ', 2)));
    let dashes = shown("0*-", &n(5.0));
    assert_eq!((dashes.text.as_str(), dashes.fill), ("5", Some(('-', 1))));
    let text = shown("@*.", &t("Total"));
    assert_eq!((text.text.as_str(), text.fill), ("Total", Some(('.', 5))));
    // Only the first fill counts; the offset counts characters, not bytes.
    let euro = shown("[$€-407]*x0*y", &n(5.0));
    assert_eq!((euro.text.as_str(), euro.fill), ("€5", Some(('x', 1))));
}

#[test]
fn general_spells_a_number_in_eleven_characters_and_offers_narrower_ones() {
    let general = FormatCode::general();
    let shown = |value: f64| rendered(&general, &n(value), DateSystem::Year1900);
    for (value, text) in [
        (0.0, "0"),
        (1234.5, "1234.5"),
        (-1234.5, "-1234.5"),
        (0.1 + 0.2, "0.3"),
        (1.0 / 3.0, "0.333333333"),
        (2.0 / 3.0, "0.666666667"),
        (123.456_789_012_345, "123.456789"),
        (12_345_678_901.0, "12345678901"),
        (123_456_789_012.0, "1.23457E+11"),
        (-123_456_789_012.0, "-1.23457E+11"),
        (1e11, "1E+11"),
        (1e20, "1E+20"),
        (0.0001234, "0.0001234"),
        (0.000_012_34, "0.00001234"),
        (0.000_012_345_6, "1.23456E-05"),
        (1.5e-10, "1.5E-10"),
        (99_999_999_999.5, "1E+11"),
        (1e100, "1E+100"),
    ] {
        assert_eq!(shown(value).text, text, "{value}");
    }
    assert_eq!(
        shown(1234.5678).shorter,
        ["1234.568", "1234.57", "1234.6", "1235"]
    );
    assert_eq!(
        shown(12_345_678_901.0).shorter,
        ["1.2346E+10", "1.235E+10", "1.23E+10", "1.2E+10", "1E+10"]
    );
    assert_eq!(shown(-0.25).shorter, ["-0.3"]);
    assert!(shown(7.0).shorter.is_empty());
    // A number format offers nothing narrower: its text is what it is.
    let fixed = FormatCode::from_code("0.00").unwrap();
    assert!(
        rendered(&fixed, &n(1234.5678), DateSystem::Year1900)
            .shorter
            .is_empty()
    );
}

#[test]
fn a_code_excel_refuses_is_refused_at_the_byte_it_breaks_at() {
    for (code, position, reason) in [
        ("0.00\"x", 4, "expected the closing \" of a quoted text"),
        ("[Red", 0, "expected the closing ] of a bracket"),
        ("0;0;0;0;0", 7, "expected at most four sections"),
        ("0\\", 1, "expected a character after \\"),
        ("0_", 1, "expected a character after _"),
        ("0*", 1, "expected a character after *"),
        (
            "[Purple]0",
            0,
            "expected a colour, a condition, an elapsed unit or a currency in brackets, got [Purple]",
        ),
        (
            "[Color57]0",
            0,
            "expected a colour from [Color1] to [Color56], got [Color57]",
        ),
        ("[<abc]0", 2, "expected a number in the condition [<abc]"),
    ] {
        let refusal = FormatCode::from_code(code).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            format!("invalid number format expression at byte {position}: {reason}"),
            "{code}"
        );
    }
}

/// Excel takes a code of at most 255 characters: a longer one is refused at
/// the byte of its 256th, however many placeholders it would hold, and one
/// of 255 reads.
#[test]
fn a_code_past_255_characters_is_refused_at_the_256th() {
    let zeros = |count: usize| "0".repeat(count);
    for code in [
        zeros(256),
        zeros(300),
        zeros(200_000),
        format!("0.{}", zeros(70_000)),
        format!("0{}", ",".repeat(70_000)),
        format!("0/{}", zeros(70_000)),
        "#,".repeat(100_000),
    ] {
        let refusal = FormatCode::from_code(&code).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "invalid number format expression at byte 255: expected a code of at most 255 \
             characters",
            "{}",
            &code[..10]
        );
    }
    // The byte of the 256th character, past a character of two bytes.
    let refusal = FormatCode::from_code(&format!("\"é\"{}", zeros(253))).unwrap_err();
    assert!(refusal.to_string().contains("at byte 256:"), "{refusal}");
    let widest = FormatCode::from_code(&zeros(255)).unwrap();
    assert_eq!(
        render(widest.code(), &n(7.0)),
        zeros(255)[..254].to_owned() + "7"
    );
}

/// Excel16 build20430 confirms improper fractions overflow their numeric
/// representation; mixed fractions keep a separate whole part. Nonfinite
/// inputs remain errors rather than fabricated numeric Excel answers.
#[test]
fn a_fraction_refuses_overflow_and_preserves_a_separate_whole_part() {
    for (code, value) in [
        ("??/??", 1e17), ("??/??", 1e18), ("??/??", 1.8e19),
        ("???/???", 1.234_567_890_123_456_8e17),
        ("#/###", 1.234_567_890_123_456_8e17),
        ("?/8", 1e17), ("???????/???????", 1e13 + 0.5), ("??/??", 1e300),
    ] {
        let shown = rendered(&FormatCode::from_code(code).unwrap(), &n(value), DateSystem::Year1900);
        assert_eq!(shown.text.as_str(), "", "{code} of {value}");
        assert_eq!(shown.fill, Some(('#', 0)), "{code} of {value}");
    }
    for (code, value, expected) in [
        ("# ?/?", 1e17, "100000000000000000    "),
        ("# ??/??", 1e18, "1000000000000000000      "),
    ] {
        assert_eq!(render(code, &n(value)), expected, "{code} of {value}");
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for code in ["??/??", "# ?/?", "?/8", "0.00", "m/d/yyyy"] {
            assert_eq!(render(code, &n(value)), "#NUM!", "{code} of {value}");
        }
    }
}

/// Excel16 build20430: every observation from all four fraction probes,
/// including independently tested intermediate-product and rounding bounds.
/// Refused TEXT inputs retain the observed physical hash-fill display.
#[test]
fn desktop_excel_fractions_match_every_observed_boundary_and_control() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/format_excel_fractions.json"
    )).unwrap();
    assert_eq!(fixture["observation_count"], 658);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 658);
    let mut failures = Vec::new();
    for case in cases {
        let code = case["code"].as_str().unwrap();
        let value = n(case["value"].as_f64().unwrap());
        let shown = rendered(&FormatCode::from_code(code).unwrap(), &value, DateSystem::Year1900);
        let fill = case["fill"].as_array().map(|_| ('#', 0));
        if shown.text.as_str() != case["expected"].as_str().unwrap() || shown.fill != fill {
            failures.push(format!("{}: {code:?}, {value:?}: expected {:?}/{fill:?}, got {:?}/{:?}",
                case["id"].as_str().unwrap(), case["expected"], shown.text, shown.fill));
        }
    }
    assert!(failures.is_empty(), "{} native fraction mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn a_code_reads_back_as_written_and_compares_by_its_text() {
    let code: FormatCode = "#,##0.00".parse().unwrap();
    assert_eq!(code.code(), "#,##0.00");
    assert_eq!(code.to_string(), "#,##0.00");
    assert_eq!(code, FormatCode::from_code("#,##0.00").unwrap());
    assert_ne!(code, FormatCode::from_code("#,##0.0").unwrap());
    assert!(FormatCode::general().is_general());
    assert!(FormatCode::from_code("general").unwrap().is_general());
    assert!(FormatCode::default().is_general());
    assert!(!code.is_general());
}

#[test]
fn a_locale_digit_system_displays_as_general() {
    for code in ["[DBNum1]0", "[DBNum2][$-804]General", "[$-2000000]0.00"] {
        assert_eq!(render(code, &n(1234.5)), "1234.5", "{code}");
    }
}

#[test]
fn with_decimals_adds_or_drops_places_in_every_number_section() {
    for (code, delta, expected) in [
        ("0.00", 1, "0.000"),
        ("0.00", -1, "0.0"),
        ("0.00", -2, "0"),
        ("0.00", -5, "0"),
        ("0", 2, "0.00"),
        ("0%", 1, "0.0%"),
        ("#,##0.0", -1, "#,##0"),
        ("#,##0,", 1, "#,##0.0,"),
        ("General", 1, "0.0"),
        ("General", -1, "0"),
        ("[Red]General", 2, "[Red]0.00"),
        ("0.00E+00", -2, "0E+00"),
        ("0E+00", 1, "0.0E+00"),
        (
            "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
            1,
            "\"$\"#,##0.000_);[Red]\\(\"$\"#,##0.000\\)",
        ),
        ("0.00;[Red]-0.00;\"zero\";@", -1, "0.0;[Red]-0.0;\"zero\";@"),
        ("m/d/yyyy", 1, "m/d/yyyy"),
        ("@", 1, "@"),
        ("# ?/?", 1, "# ?/?"),
    ] {
        let widened = FormatCode::from_code(code).unwrap().with_decimals(delta);
        assert_eq!(widened.code(), expected, "{code} by {delta}");
    }
    let widened = FormatCode::from_code("0.00%").unwrap().with_decimals(1);
    let widened = rendered(&widened, &n(0.123_45), DateSystem::Year1900);
    assert_eq!(widened.text, "12.345%");
}

#[test]
fn text_takes_the_text_section_or_shows_as_itself() {
    assert_eq!(render("0.00", &t("abc")), "abc");
    assert_eq!(render("@", &t("abc")), "abc");
    assert_eq!(render("\"Name: \"@", &t("Ada")), "Name: Ada");
    assert_eq!(render("0;-0;0;[Red]\"<\"@\">\"", &t("x")), "<x>");
    assert_eq!(render("0;-0;0;", &t("x")), "");
    // A number under a text-only code shows as General.
    assert_eq!(render("@", &n(1234.5)), "1234.5");
    assert_eq!(render("\"x\"@", &n(5.0)), "5");
    assert_eq!(render("0", &b(false)), "FALSE");
}


#[test]
fn every_builtin_id_reads_as_its_code_and_its_kind() {
    for (id, format) in [
        (14, NumberFormat::Date),
        (15, NumberFormat::Date),
        (16, NumberFormat::Date),
        (17, NumberFormat::Date),
        (18, NumberFormat::Time),
        (19, NumberFormat::Time),
        (20, NumberFormat::Time),
        (21, NumberFormat::Time),
        (22, NumberFormat::DateTime),
        (45, NumberFormat::Time),
        (46, NumberFormat::Duration),
        (47, NumberFormat::Time),
    ] {
        assert_eq!(
            FormatCode::builtin(id).map(|code| code.kind()),
            Some(format),
            "numFmtId {id}"
        );
    }
    for id in (0..=13).chain(37..=40).chain([48, 49]) {
        assert_eq!(
            FormatCode::builtin(id).map(|code| code.kind()),
            Some(NumberFormat::General),
            "numFmtId {id}"
        );
    }
    assert_eq!(FormatCode::builtin(14).unwrap().code(), "m/d/yyyy");
    assert_eq!(FormatCode::builtin(22).unwrap().code(), "m/d/yyyy h:mm");
    assert_eq!(FormatCode::builtin(0).unwrap().code(), "General");
}

#[test]
fn only_the_ecma_table_ids_are_builtin() {
    // 41 to 44 and the East Asian ids are declared by whoever uses them.
    let listed: Vec<u32> = (0..=400)
        .filter(|id| FormatCode::builtin(*id).is_some())
        .collect();
    let expected: Vec<u32> = (0..=22).chain(37..=40).chain(45..=49).collect();
    assert_eq!(listed, expected);
}

#[test]
fn an_undeclared_east_asian_or_accounting_id_reads_as_what_it_displays() {
    // A cell naming a locale's date or time id the part does not declare
    // displays - and reads - as the en-US format of that kind; 41 to 44 as
    // the accounting formats, which are numbers.
    let ids = [0, 27, 31, 36, 50, 58, 32, 35, 52, 56, 41, 44];
    let row: String = ids
        .iter()
        .enumerate()
        .skip(1)
        .map(|(at, _)| format!("<c r=\"{}1\" s=\"{at}\"><v>45292</v></c>", column(at)))
        .collect();
    let bytes = one_sheet(&format!("<row r=\"1\">{row}</row>"), &[], &[], &ids);
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let styles = workbook.style_sheet().unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    for (at, id) in ids.iter().enumerate().skip(1) {
        let expected = match id {
            27..=31 | 36 | 50 | 51 | 54 | 57 | 58 => (NumberFormat::Date, "m/d/yyyy"),
            32..=35 | 52 | 53 | 55 | 56 => (NumberFormat::Time, "h:mm:ss"),
            _ => (NumberFormat::General, ""),
        };
        let style = StyleId::new(u16::try_from(at).unwrap());
        let format = styles.format(style).unwrap();
        assert_eq!(format.kind(), expected.0, "numFmtId {id}");
        if !expected.1.is_empty() {
            assert_eq!(format.code(), expected.1, "numFmtId {id}");
        }
        let cell = sheet
            .cell(format!("{}1", column(at)).parse().unwrap())
            .unwrap();
        assert_eq!(cell.format(), expected.0, "numFmtId {id}");
    }
    assert!(
        styles
            .format(StyleId::new(10))
            .unwrap()
            .code()
            .starts_with("_(* #,##0_)")
    );
}


#[test]
fn a_code_spells_a_date_a_time_a_datetime_or_a_duration() {
    for (code, format) in [
        ("yyyy-mm-dd", NumberFormat::Date),
        ("d-mmm-yy", NumberFormat::Date),
        ("m/d/yyyy", NumberFormat::Date),
        ("YYYY-MM-DD", NumberFormat::Date),
        ("hh:mm:ss", NumberFormat::Time),
        ("h:mm AM/PM", NumberFormat::Time),
        ("HH:MM:SS", NumberFormat::Time),
        ("mm:ss.0", NumberFormat::Time),
        ("yyyy-mm-dd hh:mm:ss", NumberFormat::DateTime),
        ("yyyy-mm-dd hh:mm:ss.000", NumberFormat::DateTimeFraction),
        ("m/d/yy h:mm", NumberFormat::DateTime),
        ("[h]:mm:ss", NumberFormat::Duration),
    ] {
        assert_eq!(kind(code), format, "{code}");
    }
}

#[test]
fn m_alone_is_a_month_and_beside_a_clock_is_minutes() {
    assert_eq!(kind("mm"), NumberFormat::Date);
    assert_eq!(kind("mmmm"), NumberFormat::Date);
    assert_eq!(kind("h:mm"), NumberFormat::Time);
    assert_eq!(kind("mm:ss"), NumberFormat::Time);
}

#[test]
fn only_the_first_section_of_a_code_counts() {
    assert_eq!(kind("0.00;[Red]yyyy-mm-dd"), NumberFormat::General);
    assert_eq!(kind("yyyy-mm-dd;0.00"), NumberFormat::Date);
    assert_eq!(kind("hh:mm;@"), NumberFormat::Time);
}

#[test]
fn quoted_text_and_escaped_characters_say_nothing() {
    for code in [
        "\"h\" 0.00",
        "0.00 \"days\"",
        "0.00\\h",
        "\\d0",
        "0_s",
        "#,##0_);(#,##0)",
        "*d0",
    ] {
        assert_eq!(kind(code), NumberFormat::General, "{code}");
    }
    // A literal beside a real token leaves the token's reading.
    assert_eq!(kind("yyyy \"h\""), NumberFormat::Date);
    assert_eq!(kind("\\d hh:mm"), NumberFormat::Time);
}

#[test]
fn a_bracketed_colour_locale_or_condition_is_no_date() {
    assert_eq!(kind("[Red]0.00"), NumberFormat::General);
    assert_eq!(kind("[>=100]0"), NumberFormat::General);
    assert_eq!(kind("[Red]hh:mm"), NumberFormat::Time);
    assert_eq!(kind("[$-409]yyyy-mm-dd"), NumberFormat::Date);
    assert_eq!(kind("[$-F800]dddd, mmmm dd, yyyy"), NumberFormat::Date);
    assert_eq!(kind("[$-F400]h:mm:ss AM/PM"), NumberFormat::Time);
}

#[test]
fn a_bracketed_elapsed_unit_spells_a_duration() {
    for code in [
        "[h]:mm:ss",
        "[hh]:mm",
        "[H]:MM",
        "[mm]:ss",
        "[ss].00",
        "[m]",
    ] {
        assert_eq!(kind(code), NumberFormat::Duration, "{code}");
    }
}

#[test]
fn general_text_and_number_codes_are_general() {
    for code in [
        "General", "general", "", "@", "0", "0.00", "#,##0.00", "0.00E+00", "0%", "# ?/?",
    ] {
        assert_eq!(kind(code), NumberFormat::General, "{code}");
    }
}

#[test]
fn a_file_code_this_reader_refuses_reads_as_general_and_keeps_its_text() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c></row>",
        &[],
        &[(164, "[hm]yyyy")],
        &[0, 164],
    );
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let format = workbook
        .style_sheet()
        .unwrap()
        .format(StyleId::new(1))
        .unwrap()
        .clone();
    assert_eq!(format.code(), "[hm]yyyy");
    assert_eq!(format.kind(), NumberFormat::General);
    let cell = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell("A1".parse().unwrap())
        .unwrap();
    assert_eq!(cell.value(), &Scalar::from(45_292.0));
}

/// The ribbon's decimal buttons leave a cell whose code this reader
/// refuses as it is, whatever the code's length.
#[test]
fn decimals_leave_a_file_code_this_reader_refuses_as_it_is() {
    let long = format!("0.{}", "0".repeat(300));
    for code in ["[hm]yyyy", "[Foo]0", "0.00\"x", "0;0;0;0;0", long.as_str()] {
        let bytes = one_sheet(
            "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>1.5</v></c></row>",
            &[],
            &[(164, &code.replace('"', "&quot;"))],
            &[0, 164],
        );
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        let at: CellRef = "A1".parse().unwrap();
        let shown = workbook.display_text("Sheet1", at).unwrap().unwrap();
        record_rendered(code, workbook.sheet("Sheet1").unwrap().cell(at).unwrap().value(), DateSystem::Year1900, &shown);
        assert_eq!(shown.text, "1.5", "{code}");
        for delta in [1, -1, 15, -15] {
            let patch = StylePatch {
                decimals: Some(delta),
                ..StylePatch::default()
            };
            workbook
                .set_style("Sheet1", &[CellRange::new(at, at)], &patch)
                .unwrap();
            assert_eq!(
                workbook.cell_style("Sheet1", at).unwrap().number_format,
                code,
                "{code} by {delta}"
            );
        }
        assert_eq!(workbook.style_sheet().unwrap().len(), 2, "{code}");
    }
}

/// Excel16 build20430: signed 1904 dates/times use the magnitude and
/// the selected section's sign rule; the 1900 system refuses all negatives.
/// The LCID1033 TEXT answer is compared, while UI-localized cell text stays
/// in the fixture as independent physical-display evidence.
#[test]
fn native_negative_date_sections_match_text_or_hash_refusal() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/negative_date_native.json"
    )).unwrap();
    assert_eq!(fixture["observation_count"], 64);
    assert_eq!(fixture["cleanup_completed"], true);
    assert_eq!(fixture["excel_version"], "16.0");
    assert_eq!(fixture["excel_build"], "20430.0");
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 64);
    let mut mismatches = Vec::new();
    for case in cases {
        let system = match case["system"].as_str().unwrap() {
            "1900" => DateSystem::Year1900,
            "1904" => DateSystem::Year1904,
            other => panic!("unknown date system {other}"),
        };
        let code = case["code"].as_str().unwrap();
        let value = case["value"].as_f64().unwrap();
        let shown = rendered(&FormatCode::from_code(code).unwrap(), &n(value), system);
        let expected = case["text"].as_str();
        let matches = match expected {
            Some(expected) => shown.text == expected && shown.fill.is_none(),
            None => case["text_error"].as_str().is_some()
                && shown.text.is_empty() && shown.fill == Some(('#', 0)),
        };
        if !matches {
            mismatches.push(format!("{}: {code:?} {value:?} {system:?}: {:?}/{:?}",
                case["id"].as_str().unwrap(), shown.text, shown.fill));
        }
    }
    assert!(mismatches.is_empty(), "{} native date mismatches:\n{}",
        mismatches.len(), mismatches.join("\n"));
}

/// Excel16 build20430: every renderable answer from the 730-case native
/// comparison; 36 TEXT refusals are pinned to the separately observed hash
/// display, and blank/empty-code coercions use Excel's physical cells.
#[test]
fn desktop_excel_semantics_match_every_renderable_observation() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/format_excel_semantics.json"
    )).unwrap();
    assert_eq!(fixture["observation_count"], 730);
    let cases = fixture["renderable_cases"].as_array().unwrap();
    let rejected = fixture["rejected_observations"].as_array().unwrap();
    assert_eq!(cases.len(), 726);
    assert_eq!(rejected.len(), 4);
    assert!(rejected.iter().all(|case| case["code"] == "@;@"
        && case["format_assignment_refused"] == true));
    let mut failures = Vec::new();
    for case in cases {
        let value = match case["value_kind"].as_str().unwrap() {
            "number" => n(case["value"].as_f64().unwrap()),
            "boolean" => b(case["value"].as_bool().unwrap()),
            "text" => t(case["value"].as_str().unwrap()),
            "blank" => Scalar::Null,
            kind => panic!("unrepresented native scalar {kind}"),
        };
        let system = match case["date_system"].as_str().unwrap() {
            "1900" => DateSystem::Year1900,
            "1904" => DateSystem::Year1904,
            system => panic!("unrepresented native date system {system}"),
        };
        let code = case["code"].as_str().unwrap();
        let shown = rendered(&FormatCode::from_code(code).unwrap(), &value, system);
        let fill = case["fill"].as_array().map(|fill|
            (fill[0].as_str().unwrap().chars().next().unwrap(), fill[1].as_u64().unwrap() as usize));
        if shown.text.as_str() != case["expected"].as_str().unwrap() || shown.fill != fill {
            failures.push(format!("{}: {code:?}, {value:?}: expected {:?}/{fill:?}, got {:?}/{:?}",
                case["id"].as_str().unwrap(), case["expected"], shown.text, shown.fill));
        }
    }
    assert!(failures.is_empty(), "{} native mismatches:\n{}", failures.len(), failures.join("\n"));
}


}

/// Every render executed by this file, including nonfinite inputs that Excel
/// cannot represent. No source parsing and no separately maintained vector list.
/// Run `cargo test --locked -p yggdryl --test excel format::export_rendering_vectors_for_excel -- --exact --nocapture`.
#[test]
fn export_rendering_vectors_for_excel() {
    FORMAT_EXPORT.with(|held| {
        assert!(held.borrow().is_none());
        *held.borrow_mut() = Some(FormatExport::default());
    });
    execute_format_checks();
    let export = FORMAT_EXPORT.with(|held| held.borrow_mut().take().unwrap());
    assert_eq!(
        export.counts["where_libreoffice_differs_excel_is_followed"],
        24
    );
    assert_eq!(
        export.counts["a_1904_workbook_counts_its_days_from_1904"],
        5
    );
    assert_eq!(
        export
            .cases
            .iter()
            .filter(|case| case["value_kind"] == "nonfinite")
            .count(),
        15
    );
    let count = export.cases.len();
    assert_eq!(export.counts.values().sum::<usize>(), count);
    let ids: std::collections::BTreeSet<_> = export
        .cases
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), count);
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/excel-desktop/format-cases.json");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let payload = serde_json::json!({
        "schema_version": 1,
        "source": "rust/tests/excel/format.rs",
        "expected_source": "Rust rendering results after every original format test assertion passes",
        "source_tests": export.counts,
        "case_count": count,
        "cases": export.cases
    });
    let mut bytes = serde_json::to_vec_pretty(&payload).unwrap();
    bytes.push(b'\n');
    std::fs::write(&output, bytes).unwrap();
    println!("exported {count} format vectors to {}", output.display());
}

#[cfg(feature = "internals")]
#[test]
fn shared_digits_truncate_decimal_places_without_rounding() {
    use yggdryl::internals::excel_format::truncated_magnitude;
    for (value, places, expected) in [
        (123.456, 1, 123.4_f64),
        (123.456, -1, 120.0),
        (1.9999, 0, 1.0),
        (0.0000123456, 7, 0.0000123),
        (0.9999999999999999, 0, 1.0),
        (0.0, 2, 0.0),
        (1e15, -1, 1e15),
    ] {
        assert_eq!(truncated_magnitude(value, places).unwrap().to_bits(),
            expected.to_bits(), "{value:?}, {places}");
    }
}

#[cfg(feature = "internals")]
#[test]
fn shared_digits_round_away_preserves_exact_quanta_and_carry() {
    use yggdryl::internals::excel_format::rounded_away_magnitude;
    for (value, places, expected) in [
        (1.2, 1, 1.2_f64),        // No discarded nonzero digit.
        (1.0, 0, 1.0),
        (1.201, 1, 1.3),
        (9.99, 1, 10.0),        // Carry crosses the decimal point.
        (999.1, -2, 1_000.0),
        (0.099, 1, 0.1),
        (0.3, 0, 1.0),
        (1.25, i32::MAX, 1.25), // No active digit can be discarded.
    ] {
        assert_eq!(rounded_away_magnitude(value, places).unwrap().to_bits(),
            expected.to_bits(), "{value:?}, {places}");
    }
    // The decimal magnitude lies outside binary64; the numeric policy owns
    // whether that becomes a typed #NUM!, not the stack decimal itself.
    assert!(rounded_away_magnitude(1.25, i32::MIN).is_none_or(|value| !value.is_finite()));
}

#[test]
fn formula_numeric_text_uses_full_precision_without_changing_general_display() {
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap();
    for (row, expression, expected) in [
        (0, "LEFT(1/3,99)", "0.333333333333333"),
        (1, "LEFT(1E-18,99)", "0.000000000000000001"),
        (2, "LEFT(1.2E-18,99)", "1.2E-18"),
        (3, "LEFT(-1.23456789012345E-9,99)", "-1.23456789012345E-09"),
    ] {
        book.set_entry("Data", CellRef::new(row,0), &format!("={expression}")).unwrap();
        assert_eq!(book.calculate_all().unwrap().uncomputed,0);
        assert_eq!(book.sheet("Data").unwrap().scalar(CellRef::new(row,0)).as_str(),Some(expected));
    }
    assert_eq!(render("General", &n(1.0/3.0)), "0.333333333");
}

#[test]
fn text_format_distinguishes_legal_hash_fill_from_overflow_and_preserves_display_colour() {
    let overflow=FormatCode::from_code("[Red]yyyy-mm-dd").unwrap().render(&n(-1.0),DateSystem::Year1900);
    assert_eq!(overflow.fill,Some(('#',0)));
    assert_eq!(overflow.color,Some(0xff0000));
    let fill=FormatCode::from_code("*#0").unwrap().render(&n(2.5),DateSystem::Year1900);
    assert_eq!(fill.text,"3");assert_eq!(fill.fill,Some(('#',0)));
    let mut book=Workbook::new();book.add_sheet("Data").unwrap();
    for (row,formula,text,error) in [
        (0,"TEXT(2.5,\"*#0\")",Some("3"),None),
        (1,"TEXT(-1,\"[Red]yyyy-mm-dd\")",None,Some("#VALUE!")),
        (2,"TEXT(1/3,\"General\")",Some("0.333333333"),None),
        (3,"TEXT(2.5,\"\")",Some(""),None),
    ] {
        let at=CellRef::new(row,0);book.set_entry("Data",at,&format!("={formula}")).unwrap();
        book.calculate_all().unwrap();let cell=book.sheet("Data").unwrap().cell(at).unwrap();
        assert_eq!(cell.error().map(|e|e.as_str()),error);
        if let Some(text)=text {assert_eq!(cell.value().as_str(),Some(text));}
    }
}
