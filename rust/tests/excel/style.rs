//! `rust/src/excel/style.rs`: `StyleId`, the `cellXfs` index a cell's `s` states - its sixteen-bit bound, its spelling, and the default a cell stating none has - the values a style resolves to, their vocabularies and their defaults, and `StylePatch` applied through `Workbook::set_style` and read back through `Workbook::cell_style`.

use yggdryl::Scalar;
use yggdryl::excel::{
    Alignment, Border, BorderPreset, BorderStyle, Borders, CellRange, CellRef, CellStyle, Color,
    Edge, Fill, Font, FontScheme, Horizontal, NumberFormat, PatternType, Protection, StyleId,
    StylePatch, Underline, Vertical, VerticalRun, Workbook,
};

use crate::excel_package::one_sheet;

fn refusal(text: &str) -> String {
    StyleId::from_attribute(text).unwrap_err().to_string()
}

#[test]
fn an_index_past_sixteen_bits_or_not_a_decimal_index_is_refused_naming_the_text() {
    assert_eq!(
        refusal("65536"),
        "invalid cell style expression at byte 0: expected a style index from 0 to 65535 in \
         a cell's `s`, got \"65536\""
    );
    for text in ["4294967295", "-1", "1.5", "x", "", "0x10"] {
        assert!(
            refusal(text).ends_with(&format!("got {text:?}")),
            "{text}: {}",
            refusal(text)
        );
    }
}

#[test]
fn an_index_reads_as_its_number_and_the_default_is_index_zero() {
    assert_eq!(StyleId::from_attribute("0").unwrap(), StyleId::DEFAULT);
    assert_eq!(StyleId::from_attribute(" 7 ").unwrap().as_u16(), 7);
    assert_eq!(StyleId::from_attribute("65535").unwrap().as_u16(), u16::MAX);
    assert_eq!(StyleId::default(), StyleId::DEFAULT);
    assert_eq!(StyleId::DEFAULT.as_u16(), 0);
    assert_eq!(StyleId::new(12), StyleId::from(12_u16));
    assert_eq!(u32::from(StyleId::new(u16::MAX)), 65_535);
    assert!(StyleId::new(3) < StyleId::new(40));
}

/// Every member of a vocabulary reads back from its spelling.
macro_rules! round_trips {
    ($($vocabulary:ty),+ $(,)?) => {
        $(
            for member in <$vocabulary>::ALL {
                assert_eq!(
                    <$vocabulary>::from_attribute(member.as_str()).unwrap(),
                    *member,
                    "{}",
                    stringify!($vocabulary)
                );
                assert_eq!(member.to_string(), member.as_str());
            }
        )+
    };
}

#[test]
fn every_style_vocabulary_reads_back_what_it_spells_and_refuses_what_the_schema_does_not() {
    round_trips!(
        Underline,
        VerticalRun,
        FontScheme,
        Horizontal,
        Vertical,
        BorderStyle,
        PatternType
    );
    assert_eq!(Underline::ALL.len(), 5);
    assert_eq!(BorderStyle::ALL.len(), 14);
    assert_eq!(PatternType::ALL.len(), 19);
    assert_eq!(
        Vertical::from_attribute("middle").unwrap_err().to_string(),
        "invalid vertical alignment expression at byte 0: expected one of top, center, bottom, \
         justify, distributed, got \"middle\""
    );
    // The schema's spelling is exact: case is part of it.
    assert!(BorderStyle::from_attribute("Thin").is_err());
    assert_eq!(
        Horizontal::from_attribute(" centerContinuous ").unwrap(),
        Horizontal::CenterContinuous
    );
}

#[test]
fn a_style_stating_nothing_is_what_a_cell_stating_no_style_shows() {
    assert_eq!(Underline::default(), Underline::None);
    assert_eq!(VerticalRun::default(), VerticalRun::Baseline);
    assert_eq!(FontScheme::default(), FontScheme::None);
    assert_eq!(Horizontal::default(), Horizontal::General);
    assert_eq!(Vertical::default(), Vertical::Bottom);
    assert_eq!(BorderStyle::default(), BorderStyle::None);
    assert_eq!(PatternType::default(), PatternType::None);

    let font = Font::default();
    assert_eq!((font.name.as_str(), font.size), ("Calibri", 11.0));
    assert!(!font.bold && !font.italic && !font.strike);
    assert_eq!(font.color, None);
    // Locked and shown, as the schema defaults a cell's protection.
    assert_eq!(
        Protection::default(),
        Protection {
            locked: true,
            hidden: false
        }
    );
    // No line anywhere, the edges applying to a range's outline.
    let border = Border::default();
    assert_eq!(border.left, Edge::default());
    assert_eq!(border.left.style, BorderStyle::None);
    assert!(border.outline && !border.diagonal_up && !border.diagonal_down);
    assert_eq!(Fill::default(), Fill::None);
    assert_eq!(Alignment::default().rotation, 0);
    let style = CellStyle::default();
    assert_eq!(style.font, Font::default());
    assert_eq!(style.parent, 0);
    // The format a cell stating none shows, and every fact alike: the
    // default is the style a workbook built from nothing resolves index 0 to.
    assert_eq!(style.number_format, "General");
    let workbook = Workbook::new();
    assert_eq!(
        workbook.style_sheet().unwrap().style(StyleId::DEFAULT),
        Some(&style)
    );
    assert_ne!(
        Color::Theme {
            index: 1,
            tint: 0.0
        },
        Color::Indexed {
            index: 1,
            tint: 0.0
        }
    );
}

/// The member `name` of the package `bytes`, as text.
fn member(bytes: &[u8], name: &str) -> String {
    let archive = std::sync::Arc::new(yggdryl::zip::ZipArchive::new(
        yggdryl::holder::Holder::buffer(yggdryl::holder::Buffer::from_bytes(bytes.to_vec())),
    ));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// A workbook with one sheet, `Sheet1`.
fn book() -> Workbook {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet1").unwrap();
    workbook
}

fn range(text: &str) -> CellRange {
    text.parse().unwrap()
}

fn at(text: &str) -> CellRef {
    text.parse().unwrap()
}

#[test]
fn a_patch_sets_what_it_states_and_keeps_everything_else() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("A1"), "1234.5").unwrap();
    let patch = StylePatch {
        font_name: Some("Arial".into()),
        font_size: Some(14.0),
        bold: Some(true),
        italic: Some(true),
        underline: Some(Underline::Double),
        strike: Some(true),
        font_color: Some(Some(Color::Rgb(0xFF_FF_00_00))),
        fill: Some(Some(Color::Theme {
            index: 4,
            tint: 0.4,
        })),
        horizontal: Some(Horizontal::Center),
        vertical: Some(Vertical::Top),
        wrap: Some(true),
        number_format: Some("#,##0.00".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &patch)
        .unwrap();
    let style = workbook.cell_style("Sheet1", at("A1")).unwrap();
    assert_eq!((style.font.name.as_str(), style.font.size), ("Arial", 14.0));
    assert!(style.font.bold && style.font.italic && style.font.strike);
    assert_eq!(style.font.underline, Underline::Double);
    assert_eq!(style.font.color, Some(Color::Rgb(0xFF_FF_00_00)));
    assert_eq!(
        style.fill,
        Fill::Pattern {
            pattern: PatternType::Solid,
            foreground: Some(Color::Theme {
                index: 4,
                tint: 0.4
            }),
            background: Some(Color::Indexed {
                index: 64,
                tint: 0.0
            }),
        }
    );
    assert_eq!(style.alignment.horizontal, Horizontal::Center);
    assert_eq!(style.alignment.vertical, Vertical::Top);
    assert!(style.alignment.wrap);
    assert_eq!(style.number_format, "#,##0.00");
    assert_eq!(
        workbook
            .display_text("Sheet1", at("A1"))
            .unwrap()
            .unwrap()
            .text,
        "1,234.50"
    );
    // A second patch keeps what the first set and it does not state.
    let plain = StylePatch {
        bold: Some(false),
        font_color: Some(None),
        fill: Some(None),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &plain)
        .unwrap();
    let style = workbook.cell_style("Sheet1", at("A1")).unwrap();
    assert!(!style.font.bold && style.font.italic);
    assert_eq!(style.font.color, None);
    assert_eq!(style.fill, Fill::None);
    assert_eq!(style.number_format, "#,##0.00");
}

#[test]
fn a_range_of_ten_thousand_cells_in_three_styles_interns_three() {
    let mut workbook = book();
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    for row in 0..100 {
        for column in 0..100 {
            sheet
                .set_cell(CellRef::new(row, column), f64::from(row * column))
                .unwrap();
        }
    }
    // Three source styles: the default, a percentage and a date.
    let percent = StylePatch {
        number_format: Some("0%".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1:CV10")], &percent)
        .unwrap();
    let day = StylePatch {
        number_format: Some("m/d/yyyy".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A11:CV20")], &day)
        .unwrap();
    let before = workbook.style_sheet().unwrap().len();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1:CV100")], &bold)
        .unwrap();
    assert_eq!(workbook.style_sheet().unwrap().len(), before + 3);
    // The same patch again finds what it appended.
    workbook
        .set_style("Sheet1", &[range("A1:CV100")], &bold)
        .unwrap();
    assert_eq!(workbook.style_sheet().unwrap().len(), before + 3);
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell_count(), 10_000);
    let styles: std::collections::BTreeSet<StyleId> =
        sheet.cells().map(|cell| cell.style()).collect();
    assert_eq!(styles.len(), 3);
}

#[test]
fn an_empty_cell_of_a_bounded_range_takes_the_style_and_a_whole_column_or_row_takes_its_own() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("B5"), "7").unwrap();
    workbook.set_entry("Sheet1", at("D3"), "8").unwrap();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("F1:G2")], &bold)
        .unwrap();
    assert_eq!(workbook.sheet("Sheet1").unwrap().cell_count(), 6);
    assert!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .cell(at("G2"))
            .unwrap()
            .is_null()
    );
    // Whole columns: the column's own style, its present cell patched,
    // no cell put.
    let italic = StylePatch {
        italic: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B:C")], &italic)
        .unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell_count(), 6);
    assert!(sheet.column_style(1).is_some() && sheet.column_style(2).is_some());
    assert_eq!(sheet.column_style(1), sheet.column_style(2));
    assert!(sheet.column_style(3).is_none());
    assert!(workbook.cell_style("Sheet1", at("B5")).unwrap().font.italic);
    assert!(
        workbook
            .cell_style("Sheet1", at("C900"))
            .unwrap()
            .font
            .italic
    );
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().column_width(1),
        workbook.sheet("Sheet1").unwrap().default_column_width()
    );
    // Whole rows alike.
    let underline = StylePatch {
        underline: Some(Underline::Single),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("3:4")], &underline)
        .unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell_count(), 6);
    assert!(sheet.row_style(2).is_some() && sheet.row_style(3).is_some());
    let d3 = workbook.cell_style("Sheet1", at("D3")).unwrap();
    assert_eq!(d3.font.underline, Underline::Single);
    assert!(!d3.font.italic);
    assert_eq!(
        workbook
            .cell_style("Sheet1", at("Z4"))
            .unwrap()
            .font
            .underline,
        Underline::Single
    );
    // A cell put in a styled row takes the row's style, then the patch.
    workbook.set_style("Sheet1", &[range("Z4")], &bold).unwrap();
    let z4 = workbook.cell_style("Sheet1", at("Z4")).unwrap();
    assert!(z4.font.bold && z4.font.underline == Underline::Single);
    // Saved and opened again, all of it stands.
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert!(
        reopened
            .cell_style("Sheet1", at("C900"))
            .unwrap()
            .font
            .italic
    );
    assert_eq!(
        reopened
            .cell_style("Sheet1", at("A4"))
            .unwrap()
            .font
            .underline,
        Underline::Single
    );
    assert!(reopened.cell_style("Sheet1", at("G1")).unwrap().font.bold);
}

#[test]
fn a_whole_column_patch_splits_the_columns_it_reaches_into() {
    let mut workbook = book();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B:E")], &bold)
        .unwrap();
    let italic = StylePatch {
        italic: Some(true),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("D:G")], &italic)
        .unwrap();
    let style = |column: &str| {
        workbook
            .cell_style("Sheet1", at(&format!("{column}7")))
            .unwrap()
    };
    assert!(style("B").font.bold && !style("B").font.italic);
    assert!(style("D").font.bold && style("D").font.italic);
    assert!(style("E").font.bold && style("E").font.italic);
    assert!(!style("F").font.bold && style("F").font.italic);
    assert!(!style("H").font.bold && !style("H").font.italic);
    let bytes = workbook.into_bytes().unwrap();
    let part = member(&bytes, "xl/worksheets/sheet1.xml");
    let cols = &part[part.find("<cols>").unwrap()..part.find("</cols>").unwrap()];
    assert_eq!(cols.matches("<col ").count(), 3, "{cols}");
}

#[test]
fn a_border_preset_draws_on_the_range() {
    let mut workbook = book();
    let outside = StylePatch {
        borders: Some(Borders {
            preset: BorderPreset::Outside,
            style: BorderStyle::Medium,
            color: Some(Color::Rgb(0xFF_00_00_FF)),
        }),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B2:D4")], &outside)
        .unwrap();
    let border =
        |workbook: &Workbook, cell: &str| workbook.cell_style("Sheet1", at(cell)).unwrap().border;
    let line = Edge {
        style: BorderStyle::Medium,
        color: Some(Color::Rgb(0xFF_00_00_FF)),
    };
    let none = Edge::default();
    let corner = border(&workbook, "B2");
    assert_eq!(
        (corner.top, corner.left, corner.bottom, corner.right),
        (line, line, none, none)
    );
    let middle = border(&workbook, "C3");
    assert_eq!(
        (middle.top, middle.left, middle.bottom, middle.right),
        (none, none, none, none)
    );
    let edge = border(&workbook, "D3");
    assert_eq!(
        (edge.top, edge.left, edge.bottom, edge.right),
        (none, none, none, line)
    );
    let last = border(&workbook, "D4");
    assert_eq!(
        (last.top, last.left, last.bottom, last.right),
        (none, none, line, line)
    );
    // The interior shows as it did, so no cell is put there: only the
    // eight edge cells are.
    assert!(workbook.sheet("Sheet1").unwrap().cell(at("C3")).is_none());
    assert_eq!(workbook.sheet("Sheet1").unwrap().cell_count(), 8);
    for (preset, cell, expected) in [
        (BorderPreset::Bottom, "C4", (false, false, true, false)),
        (BorderPreset::Bottom, "C3", (false, false, false, false)),
        (BorderPreset::Top, "C2", (true, false, false, false)),
        (BorderPreset::Left, "B3", (false, true, false, false)),
        (BorderPreset::Right, "D3", (false, false, false, true)),
        (BorderPreset::All, "C3", (true, true, true, true)),
    ] {
        let mut workbook = book();
        let patch = StylePatch {
            borders: Some(Borders {
                preset,
                ..Borders::default()
            }),
            ..StylePatch::default()
        };
        workbook
            .set_style("Sheet1", &[range("B2:D4")], &patch)
            .unwrap();
        let border = workbook.cell_style("Sheet1", at(cell)).unwrap().border;
        let drawn = |edge: Edge| edge.style == BorderStyle::Thin;
        assert_eq!(
            (
                drawn(border.top),
                drawn(border.left),
                drawn(border.bottom),
                drawn(border.right)
            ),
            expected,
            "{preset} at {cell}"
        );
    }
    // Thick outside draws thick whatever the style, and none takes every
    // edge off.
    let thick = StylePatch {
        borders: Some(Borders {
            preset: BorderPreset::ThickOutside,
            ..Borders::default()
        }),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B2:D4")], &thick)
        .unwrap();
    assert_eq!(border(&workbook, "B2").top.style, BorderStyle::Thick);
    let clear = StylePatch {
        borders: Some(Borders {
            preset: BorderPreset::None,
            ..Borders::default()
        }),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B2:D4")], &clear)
        .unwrap();
    assert_eq!(border(&workbook, "B2"), Border::default());
    assert_eq!(border(&workbook, "D4"), Border::default());
}

#[test]
fn a_number_format_changes_what_a_number_is() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("A1"), "45292").unwrap();
    workbook.set_entry("Sheet1", at("A2"), "-3").unwrap();
    let day = StylePatch {
        number_format: Some("m/d/yyyy".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1:A2")], &day)
        .unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::date32(19_723));
    assert_eq!(sheet.cell(at("A1")).unwrap().format(), NumberFormat::Date);
    // No day spells -3: it stays a number, displayed as hashes.
    assert_eq!(sheet.scalar(at("A2")), Scalar::from(-3.0));
    let shown = workbook.display_text("Sheet1", at("A2")).unwrap().unwrap();
    assert_eq!((shown.text.as_str(), shown.fill), ("", Some(('#', 0))));
    let number = StylePatch {
        number_format: Some("0.00".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &number)
        .unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(45_292.0));
    assert_eq!(
        sheet.cell(at("A1")).unwrap().format(),
        NumberFormat::General
    );
    // General clears a format.
    let general = StylePatch {
        number_format: Some("general".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &general)
        .unwrap();
    assert_eq!(
        workbook
            .cell_style("Sheet1", at("A1"))
            .unwrap()
            .number_format,
        "General"
    );
    // Saved and read again, a number under a date format no day spells
    // reads as the number it is.
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("A2")),
        Scalar::from(-3.0)
    );
}

#[test]
fn a_built_in_code_interns_to_its_id_and_any_other_to_a_declared_one() {
    let mut workbook = book();
    for (cell, code) in [
        ("A1", "0.00%"),
        ("A2", "m/d/yyyy"),
        ("A3", "#,##0.00"),
        ("A4", "0.000"),
        (
            "A5",
            r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#,
        ),
    ] {
        let patch = StylePatch {
            number_format: Some(code.into()),
            ..StylePatch::default()
        };
        workbook
            .set_style("Sheet1", &[range(cell)], &patch)
            .unwrap();
    }
    let bytes = workbook.into_bytes().unwrap();
    let styles = member(&bytes, "xl/styles.xml");
    for id in ["10", "14", "4", "164", "165"] {
        assert!(
            styles.contains(&format!("numFmtId=\"{id}\"")),
            "{id}: {styles}"
        );
    }
    assert!(
        styles.contains("<numFmt numFmtId=\"164\" formatCode=\"0.000\"/>"),
        "{styles}"
    );
    assert!(!styles.contains("numFmtId=\"44\""), "{styles}");
    let reopened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(
        reopened
            .cell_style("Sheet1", at("A1"))
            .unwrap()
            .number_format,
        "0.00%"
    );
    assert_eq!(
        reopened
            .cell_style("Sheet1", at("A4"))
            .unwrap()
            .number_format,
        "0.000"
    );
}

#[test]
fn decimals_widen_each_source_format_once() {
    let mut workbook = book();
    for (cell, text) in [
        ("A1", "12%"),
        ("A2", "$5.25"),
        ("A3", "7"),
        ("A4", "1/2/2024"),
    ] {
        workbook.set_entry("Sheet1", at(cell), text).unwrap();
    }
    let wider = StylePatch {
        decimals: Some(1),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1:A4")], &wider)
        .unwrap();
    let code = |workbook: &Workbook, cell: &str| {
        workbook
            .cell_style("Sheet1", at(cell))
            .unwrap()
            .number_format
    };
    assert_eq!(code(&workbook, "A1"), "0.0%");
    assert_eq!(code(&workbook, "A2"), "\"$\"#,##0.000");
    assert_eq!(code(&workbook, "A3"), "0.0");
    assert_eq!(code(&workbook, "A4"), "m/d/yyyy");
    let narrower = StylePatch {
        decimals: Some(-2),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1:A3")], &narrower)
        .unwrap();
    assert_eq!(code(&workbook, "A1"), "0%");
    assert_eq!(code(&workbook, "A2"), "\"$\"#,##0.0");
    assert_eq!(code(&workbook, "A3"), "0");
}

#[test]
fn a_patch_no_cell_can_take_is_refused_and_changes_nothing() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("A1"), "5").unwrap();
    let before = workbook.style_sheet().unwrap().len();
    let revision = workbook.sheet("Sheet1").unwrap().revision();
    for (patch, reason) in [
        (
            StylePatch {
                number_format: Some("0.00".into()),
                decimals: Some(1),
                ..StylePatch::default()
            },
            "invalid record value at $.decimals: expected a number format or decimals in one patch, got both",
        ),
        (
            StylePatch {
                decimals: Some(16),
                ..StylePatch::default()
            },
            "invalid record value at $.decimals: expected decimals from -15 to 15, got 16",
        ),
        (
            StylePatch {
                font_size: Some(0.5),
                ..StylePatch::default()
            },
            "invalid record value at $.font_size: expected a font size from 1 to 409 points, got 0.5",
        ),
        (
            StylePatch {
                font_name: Some("".into()),
                ..StylePatch::default()
            },
            "invalid record value at $.font_name: expected a font name of 1 to 31 characters, got \"\"",
        ),
        (
            StylePatch {
                number_format: Some("[DBNum1]0".into()),
                ..StylePatch::default()
            },
            "invalid record value at $.number_format: expected a format in en-US digits, got \"[DBNum1]0\", which names a locale's own",
        ),
        (
            StylePatch {
                bold: Some(true),
                number_format: Some("0.00\"x".into()),
                ..StylePatch::default()
            },
            "invalid number format expression at byte 4: expected the closing \" of a quoted text",
        ),
    ] {
        let refusal = workbook
            .set_style("Sheet1", &[range("A1:B2")], &patch)
            .unwrap_err();
        assert_eq!(refusal.to_string(), reason);
    }
    assert_eq!(workbook.style_sheet().unwrap().len(), before);
    assert_eq!(workbook.sheet("Sheet1").unwrap().revision(), revision);
    assert_eq!(workbook.sheet("Sheet1").unwrap().cell_count(), 1);
    // A sheet the workbook does not hold is refused naming it.
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    assert!(
        workbook
            .set_style("Nowhere", &[range("A1")], &bold)
            .is_err()
    );
    let huge = workbook
        .set_style("Sheet1", &[range("A1:Z100000")], &bold)
        .unwrap_err()
        .to_string();
    // The bound counts the blank cells a range would put: its area but the
    // cells it holds.
    assert!(
        huge.ends_with(
            "expected ranges putting at most 2097152 blank cells, or whole rows or columns, got \
             2599999"
        ),
        "{huge}"
    );
}

#[test]
fn a_style_appended_part_way_through_a_refused_patch_is_taken_back() {
    // A part naming the highest number format id there is leaves no id for
    // a new code: the font the patch interned first goes with the refusal.
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        &[],
        &[(u32::MAX, "0.0")],
        &[0],
    );
    let mut workbook = Workbook::from_bytes(bytes).unwrap();
    let before = workbook.style_sheet().unwrap().len();
    let patch = StylePatch {
        bold: Some(true),
        number_format: Some("0.0000".into()),
        ..StylePatch::default()
    };
    let refusal = workbook
        .set_style("Sheet1", &[range("A1")], &patch)
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("got none left past"), "{refusal}");
    assert_eq!(workbook.style_sheet().unwrap().len(), before);
    assert!(!workbook.is_dirty());
    // The styles part a save writes is the one read.
    let saved = workbook.into_bytes().unwrap();
    assert!(!member(&saved, "xl/styles.xml").contains("<b/>"));
}

#[test]
fn a_fill_is_interned_past_the_two_fills_excel_reserves() {
    // The fixture's styles part lists a single `none` fill.
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        &[],
        &[],
        &[0],
    );
    let mut workbook = Workbook::from_bytes(bytes).unwrap();
    let patch = StylePatch {
        fill: Some(Some(Color::Rgb(0xFF_FF_FF_00))),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &patch)
        .unwrap();
    let saved = workbook.into_bytes().unwrap();
    let styles = member(&saved, "xl/styles.xml");
    let fills = &styles[styles.find("<fills").unwrap()..styles.find("</fills>").unwrap()];
    assert!(fills.starts_with("<fills count=\"3\">"), "{fills}");
    assert!(fills.contains("patternType=\"gray125\""), "{fills}");
    assert!(styles.contains("fillId=\"2\""), "{styles}");
    let reopened = Workbook::from_bytes(saved).unwrap();
    assert_eq!(
        reopened.cell_style("Sheet1", at("A1")).unwrap().fill,
        Fill::Pattern {
            pattern: PatternType::Solid,
            foreground: Some(Color::Rgb(0xFF_FF_FF_00)),
            background: Some(Color::Indexed {
                index: 64,
                tint: 0.0
            }),
        }
    );
}

#[test]
fn a_named_font_leaves_the_theme_and_an_indent_takes_an_edge() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("A1"), "x").unwrap();
    let patch = StylePatch {
        font_name: Some("Arial".into()),
        indent: Some(2),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &patch)
        .unwrap();
    let style = workbook.cell_style("Sheet1", at("A1")).unwrap();
    assert_eq!(style.font.scheme, None);
    assert_eq!(style.alignment.indent, 2);
    assert_eq!(style.alignment.horizontal, Horizontal::Left);
    let right = StylePatch {
        horizontal: Some(Horizontal::Right),
        indent: Some(1),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("A1")], &right)
        .unwrap();
    assert_eq!(
        workbook
            .cell_style("Sheet1", at("A1"))
            .unwrap()
            .alignment
            .horizontal,
        Horizontal::Right
    );
}

#[test]
fn every_patched_fact_survives_a_save() {
    let mut workbook = book();
    workbook.set_entry("Sheet1", at("B2"), "1234.5").unwrap();
    let patch = StylePatch {
        font_name: Some("Courier New".into()),
        font_size: Some(9.0),
        bold: Some(true),
        underline: Some(Underline::SingleAccounting),
        font_color: Some(Some(Color::Theme {
            index: 5,
            tint: -0.25,
        })),
        fill: Some(Some(Color::Indexed {
            index: 13,
            tint: 0.0,
        })),
        borders: Some(Borders {
            preset: BorderPreset::All,
            style: BorderStyle::DashDot,
            color: Some(Color::Rgb(0xFF_12_34_56)),
        }),
        horizontal: Some(Horizontal::Distributed),
        vertical: Some(Vertical::Center),
        wrap: Some(true),
        number_format: Some("[Blue]#,##0.0;[Red]-#,##0.0".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Sheet1", &[range("B2:C3")], &patch)
        .unwrap();
    let expected = workbook.cell_style("Sheet1", at("B2")).unwrap();
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(reopened.cell_style("Sheet1", at("B2")).unwrap(), expected);
    assert_eq!(reopened.cell_style("Sheet1", at("C3")).unwrap(), expected);
    let shown = reopened.display_text("Sheet1", at("B2")).unwrap().unwrap();
    assert_eq!(
        (shown.text.as_str(), shown.color),
        ("1,234.5", Some(0x00_00FF))
    );
}

/// A date whose style does not read as one - a cell built in memory holds
/// the default style - shows and saves as the crate's date code, and a
/// patch keeps it a date: bold is a bold date, where it once was its
/// serial.
#[test]
fn a_patch_keeps_a_date_whose_style_does_not_read_as_one_a_date() {
    let mut workbook = book();
    let sheet = workbook.sheet_mut("Sheet1").unwrap();
    sheet.set_cell(at("A1"), Scalar::date32(19_724)).unwrap();
    sheet.set_cell(at("B1"), Scalar::date32(19_724)).unwrap();
    assert_eq!(
        workbook
            .cell_style("Sheet1", at("A1"))
            .unwrap()
            .number_format,
        "yyyy-mm-dd"
    );
    assert_eq!(
        workbook
            .display_text("Sheet1", at("A1"))
            .unwrap()
            .unwrap()
            .text,
        "2024-01-02"
    );
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook.set_style("Sheet1", &[range("A1")], &bold).unwrap();
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::date32(19_724)
    );
    let style = workbook.cell_style("Sheet1", at("A1")).unwrap();
    assert!(style.font.bold);
    assert_eq!(style.number_format, "yyyy-mm-dd");
    assert_eq!(
        workbook
            .display_text("Sheet1", at("A1"))
            .unwrap()
            .unwrap()
            .text,
        "2024-01-02"
    );
    // A number typed into it is the day it counts to.
    workbook.set_entry("Sheet1", at("B1"), "45293").unwrap();
    assert_eq!(
        workbook.sheet("Sheet1").unwrap().scalar(at("B1")),
        Scalar::date32(19_724)
    );
    // A number format stated reads the date as its serial.
    let number = StylePatch {
        number_format: Some("0.00".into()),
        ..StylePatch::default()
    };
    let mut serial = book();
    serial
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell(at("A1"), Scalar::date32(19_724))
        .unwrap();
    serial.set_style("Sheet1", &[range("A1")], &number).unwrap();
    assert_eq!(
        serial.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::from(45_293.0)
    );
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.sheet("Sheet1").unwrap().scalar(at("A1")),
        Scalar::date32(19_724)
    );
    assert_eq!(reopened.cell_style("Sheet1", at("A1")).unwrap(), style);
}

/// A save writes a temporal cell whose style does not read as its format
/// under its own style with the format's code: a cell built bold keeps its
/// bold.
#[test]
fn a_temporal_cell_built_in_another_style_saves_in_it() {
    let mut workbook = book();
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    workbook.set_style("Sheet1", &[range("C1")], &bold).unwrap();
    let bolded = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell(at("C1"))
        .unwrap()
        .style();
    for cell in ["A1", "A2"] {
        let built = yggdryl::excel::Cell::from_scalar(
            at(cell),
            Scalar::date32(19_724),
            yggdryl::excel::DateSystem::Year1900,
        )
        .unwrap()
        .with_style(bolded);
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .insert_cell(built)
            .unwrap();
    }
    let expected = workbook.cell_style("Sheet1", at("A1")).unwrap();
    assert!(expected.font.bold);
    assert_eq!(expected.number_format, "yyyy-mm-dd");
    let reopened = Workbook::from_bytes(workbook.into_bytes().unwrap()).unwrap();
    for cell in ["A1", "A2"] {
        assert_eq!(
            reopened.sheet("Sheet1").unwrap().scalar(at(cell)),
            Scalar::date32(19_724)
        );
        assert_eq!(reopened.cell_style("Sheet1", at(cell)).unwrap(), expected);
    }
    // One style written for both: beside the default and the bold one,
    // the bold date.
    assert_eq!(reopened.style_sheet().unwrap().len(), 3);
}

/// Ranges that overlap patch each cell once, with every edge each of them
/// draws on it: as two patches one after the other draw, and never a
/// decimal place twice.
#[test]
fn overlapping_ranges_patch_each_cell_once_with_every_edge_drawn() {
    let outside = StylePatch {
        borders: Some(Borders {
            preset: BorderPreset::Outside,
            style: BorderStyle::Thin,
            color: None,
        }),
        ..StylePatch::default()
    };
    let mut together = book();
    together
        .set_style("Sheet1", &[range("A1:B2"), range("B2:C3")], &outside)
        .unwrap();
    let mut apart = book();
    for part in ["A1:B2", "B2:C3"] {
        apart.set_style("Sheet1", &[range(part)], &outside).unwrap();
    }
    let line = Edge {
        style: BorderStyle::Thin,
        color: None,
    };
    let shared = together.cell_style("Sheet1", at("B2")).unwrap().border;
    assert_eq!(
        (shared.top, shared.left, shared.bottom, shared.right),
        (line, line, line, line)
    );
    for cell in ["A1", "A2", "B1", "B2", "B3", "C2", "C3"] {
        assert_eq!(
            together.cell_style("Sheet1", at(cell)).unwrap(),
            apart.cell_style("Sheet1", at(cell)).unwrap(),
            "{cell}"
        );
    }
    // Whole columns and whole rows the same way.
    let mut columns = book();
    columns
        .set_style("Sheet1", &[range("B:C"), range("C:D")], &outside)
        .unwrap();
    let column = |workbook: &Workbook, name: &str| {
        workbook
            .cell_style("Sheet1", at(&format!("{name}7")))
            .unwrap()
            .border
    };
    let shared = column(&columns, "C");
    assert_eq!((shared.left, shared.right), (line, line));
    assert_eq!(column(&columns, "B").left, line);
    assert_eq!(column(&columns, "B").right, Edge::default());
    assert_eq!(column(&columns, "D").right, line);
    let mut rows = book();
    rows.set_style("Sheet1", &[range("2:3"), range("3:4")], &outside)
        .unwrap();
    let shared = rows.cell_style("Sheet1", at("F3")).unwrap().border;
    assert_eq!((shared.top, shared.bottom), (line, line));
    // A decimal place is added once to a cell two ranges hold.
    let mut decimals = book();
    for cell in ["A1", "B2", "C3"] {
        decimals.set_entry("Sheet1", at(cell), "1.5").unwrap();
    }
    let wider = StylePatch {
        decimals: Some(1),
        ..StylePatch::default()
    };
    decimals
        .set_style("Sheet1", &[range("A1:B2"), range("B2:C3")], &wider)
        .unwrap();
    for cell in ["A1", "B2", "C3"] {
        assert_eq!(
            decimals
                .cell_style("Sheet1", at(cell))
                .unwrap()
                .number_format,
            "0.0",
            "{cell}"
        );
    }
}
