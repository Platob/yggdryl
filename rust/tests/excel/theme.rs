//! `rust/src/excel/theme.rs`: `Theme` - the colours of a workbook's theme part, Office's default when it has none - and the colours a style states resolved through it and the palette.

use yggdryl::excel::{Color, Theme, Workbook};

use crate::excel_package::{content_types, package, root_relationships, workbook, worksheet};

/// A one-sheet package whose theme part is `theme`.
fn themed(theme: &str) -> Vec<u8> {
    let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>\
        </Relationships>";
    package(&[
        (
            "[Content_Types].xml",
            content_types(1, false, false).as_str(),
        ),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", worksheet("").as_str()),
        ("xl/theme/theme1.xml", theme),
    ])
}

/// A theme part whose scheme holds `slots`, each `(name, element)`.
fn theme_part(slots: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Custom\">\
         <a:themeElements><a:clrScheme name=\"Custom\">{slots}</a:clrScheme>\
         <a:fontScheme name=\"Custom\"><a:majorFont><a:latin typeface=\"Arial\"/></a:majorFont></a:fontScheme>\
         </a:themeElements></a:theme>"
    )
}

#[test]
fn the_default_theme_is_office_2013_and_later() {
    let theme = Theme::default();
    assert_eq!(theme, Theme::OFFICE);
    assert_eq!(
        theme.scheme(),
        &[
            0x00_0000, 0xFF_FFFF, 0x44_546A, 0xE7_E6E6, 0x44_72C4, 0xED_7D31, 0xA5_A5A5, 0xFF_C000,
            0x5B_9BD5, 0x70_AD47, 0x05_63C1, 0x95_4F72,
        ]
    );
}

#[test]
fn a_style_index_counts_each_light_and_dark_pair_the_other_way_round() {
    let theme = Theme::OFFICE;
    let scheme = theme.scheme();
    assert_eq!(theme.color(0), Some(scheme[1]));
    assert_eq!(theme.color(1), Some(scheme[0]));
    assert_eq!(theme.color(2), Some(scheme[3]));
    assert_eq!(theme.color(3), Some(scheme[2]));
    for index in 4..=11 {
        assert_eq!(theme.color(index), Some(scheme[usize::from(index)]));
    }
    assert_eq!(theme.color(12), None);
    assert_eq!(theme.color(u8::MAX), None);
}

#[test]
fn a_workbook_reads_the_theme_its_relationship_names() {
    let bytes = themed(&theme_part(
        "<a:dk1><a:sysClr val=\"windowText\" lastClr=\"111111\"/></a:dk1>\
         <a:lt1><a:sysClr val=\"window\" lastClr=\"FEFEFE\"/></a:lt1>\
         <a:dk2><a:srgbClr val=\"1F497D\"/></a:dk2>\
         <a:lt2><a:srgbClr val=\"EEECE1\"/></a:lt2>\
         <a:accent1><a:srgbClr val=\"4F81BD\"><a:lumMod val=\"75000\"/></a:srgbClr></a:accent1>\
         <a:accent2><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:accent2>",
    ));
    let opened = Workbook::from_bytes(bytes).unwrap();
    let theme = opened.theme().unwrap();
    assert_eq!(theme.color(1), Some(0x11_1111));
    assert_eq!(theme.color(0), Some(0xFE_FEFE));
    assert_eq!(theme.color(3), Some(0x1F_497D));
    assert_eq!(theme.color(2), Some(0xEE_ECE1));
    assert_eq!(theme.color(4), Some(0x4F_81BD));
    // A slot the part states in a form this reader does not, or leaves
    // out, is Office's.
    assert_eq!(theme.color(5), Some(Theme::OFFICE.scheme()[5]));
    assert_eq!(theme.color(11), Some(Theme::OFFICE.scheme()[11]));
    // Read once: the same theme answers again.
    assert!(std::ptr::eq(theme, opened.theme().unwrap()));
}

#[test]
fn a_theme_colour_that_is_not_six_hex_digits_is_refused() {
    let bytes = themed(&theme_part("<a:dk1><a:srgbClr val=\"12345\"/></a:dk1>"));
    let refusal = Workbook::from_bytes(bytes)
        .unwrap()
        .theme()
        .unwrap_err()
        .to_string();
    assert!(
        refusal.ends_with("expected six hex digits of RGB for a theme colour, got \"12345\""),
        "{refusal}"
    );
}

#[test]
fn a_workbook_saved_and_opened_again_keeps_its_theme() {
    let bytes = themed(&theme_part(
        "<a:accent1><a:srgbClr val=\"123456\"/></a:accent1>",
    ));
    let mut opened = Workbook::from_bytes(bytes).unwrap();
    opened
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), 1.0)
        .unwrap();
    let again = Workbook::from_bytes(opened.into_bytes().unwrap()).unwrap();
    assert_eq!(again.theme().unwrap().color(4), Some(0x12_3456));
}

#[test]
fn a_theme_tint_keeps_hue_while_changing_luminance() {
    let workbook = Workbook::new();
    let styles = workbook.style_sheet().unwrap();
    let theme = Theme::OFFICE;
    // A coloured theme entry keeps its hue: lighter is lighter, darker darker.
    let lighter = styles
        .resolve(
            &Color::Theme {
                index: 4,
                tint: 0.4,
            },
            &theme,
        )
        .unwrap();
    let darker = styles
        .resolve(
            &Color::Theme {
                index: 4,
                tint: -0.25,
            },
            &theme,
        )
        .unwrap();
    let blue = |rgb: u32| rgb & 0xFF;
    assert!(blue(lighter) > 0xC4 && blue(darker) < 0xC4);
}

#[test]
fn an_rgb_indexed_or_automatic_colour_resolves_as_the_part_says() {
    let workbook = Workbook::new();
    let styles = workbook.style_sheet().unwrap();
    let theme = Theme::OFFICE;
    assert_eq!(
        styles.resolve(&Color::Rgb(0xFF_12_34_56), &theme),
        Some(0x12_3456)
    );
    assert_eq!(
        styles.resolve(&Color::Rgb(0x00_12_34_56), &theme),
        Some(0x12_3456)
    );
    for (index, expected) in [
        (8, 0x00_0000),
        (9, 0xFF_FFFF),
        (10, 0xFF_0000),
        (63, 0x33_3333),
    ] {
        assert_eq!(
            styles.resolve(&Color::Indexed { index, tint: 0.0 }, &theme),
            Some(expected),
            "indexed {index}"
        );
    }
    // The system foreground and background are the reader's own.
    assert_eq!(
        styles.resolve(
            &Color::Indexed {
                index: 64,
                tint: 0.0
            },
            &theme
        ),
        None
    );
    assert_eq!(
        styles.resolve(
            &Color::Indexed {
                index: 65,
                tint: 0.0
            },
            &theme
        ),
        None
    );
    assert_eq!(styles.resolve(&Color::Auto, &theme), None);
}

#[test]
fn a_palette_the_styles_part_states_replaces_the_default_one() {
    let styles = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
        <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
        <borders count=\"1\"><border/></borders>\
        <cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
        <colors><indexedColors><rgbColor rgb=\"FF010203\"/><rgbColor rgb=\"FF040506\"/></indexedColors></colors>\
        </styleSheet>";
    let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
        </Relationships>";
    let bytes = package(&[
        (
            "[Content_Types].xml",
            content_types(1, false, true).as_str(),
        ),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/worksheets/sheet1.xml", worksheet("").as_str()),
        ("xl/styles.xml", styles),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    let sheet = opened.style_sheet().unwrap();
    let theme = opened.theme().unwrap();
    assert_eq!(
        sheet.resolve(
            &Color::Indexed {
                index: 1,
                tint: 0.0
            },
            theme
        ),
        Some(0x04_0506)
    );
    // An index past the part's palette reads the default one.
    assert_eq!(
        sheet.resolve(
            &Color::Indexed {
                index: 10,
                tint: 0.0
            },
            theme
        ),
        Some(0xFF_0000)
    );
}

/// The workbook's scheme is the theme's first: the schemes an
/// `a:extraClrSchemeLst` offers after it are not read, and a slot stated
/// empty holds no colour for the slot after it.
#[test]
fn only_the_theme_s_own_scheme_is_read_and_an_empty_slot_is_office_s() {
    let theme = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Custom\">\
        <a:themeElements><a:clrScheme name=\"Custom\">\
        <a:dk1/>\
        <a:lt1><a:srgbClr val=\"FAFAFA\"/></a:lt1>\
        <a:accent1><a:srgbClr val=\"123456\"/></a:accent1>\
        </a:clrScheme></a:themeElements>\
        <a:objectDefaults/>\
        <a:extraClrSchemeLst><a:extraClrScheme><a:clrScheme name=\"Extra\">\
        <a:dk1><a:srgbClr val=\"EEEEEE\"/></a:dk1>\
        <a:lt1><a:srgbClr val=\"DDDDDD\"/></a:lt1>\
        <a:accent1><a:srgbClr val=\"ABCDEF\"/></a:accent1>\
        </a:clrScheme></a:extraClrScheme></a:extraClrSchemeLst>\
        </a:theme>";
    let opened = Workbook::from_bytes(themed(theme)).unwrap();
    let theme = opened.theme().unwrap();
    assert_eq!(theme.color(4), Some(0x12_3456));
    assert_eq!(theme.color(0), Some(0xFA_FAFA));
    assert_eq!(theme.color(1), Some(Theme::OFFICE.scheme()[0]));
}

/// A section's `[ColorN]` is the workbook palette's entry `N + 7`: the one
/// the styles part states where it states one, as a style's indexed colour
/// is; a code rendered alone reads the default palette.
#[test]
fn a_section_s_numbered_colour_is_the_workbook_palette_s() {
    let mut palette = String::new();
    for index in 0..11_u32 {
        palette.push_str(&format!(
            "<rgbColor rgb=\"FF{:06X}\"/>",
            if index == 10 { 0x12_3456 } else { index }
        ));
    }
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <numFmts count=\"2\"><numFmt numFmtId=\"164\" formatCode=\"[Color3]0\"/>\
        <numFmt numFmtId=\"165\" formatCode=\"[Color4]0\"/></numFmts>\
        <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
        <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
        <borders count=\"1\"><border/></borders>\
        <cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
        <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
        <xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>\
        <colors><indexedColors>{palette}</indexedColors></colors>\
        </styleSheet>"
    );
    let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
        </Relationships>";
    let bytes = package(&[
        ("[Content_Types].xml", content_types(1, false, true).as_str()),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        (
            "xl/worksheets/sheet1.xml",
            worksheet(
                "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>5</v></c><c r=\"B1\" s=\"2\"><v>6</v></c></row>",
            ).as_str(),
        ),
        ("xl/styles.xml", styles.as_str()),
    ]);
    let opened = Workbook::from_bytes(bytes).unwrap();
    let shown = |at: &str| {
        opened
            .display_text("Sheet1", at.parse().unwrap())
            .unwrap()
            .unwrap()
    };
    assert_eq!(shown("A1").text, "5");
    assert_eq!(shown("A1").color, Some(0x12_3456));
    // Past the part's palette, the default one: [Color4] is green.
    assert_eq!(shown("B1").color, Some(0x00_FF00));
    let alone = yggdryl::excel::FormatCode::from_code("[Color3]0")
        .unwrap()
        .render(
            &yggdryl::Scalar::from(5.0),
            yggdryl::excel::DateSystem::Year1900,
        );
    assert_eq!(alone.color, Some(0xFF_0000));
}

#[test]
fn desktop_oracle_tints_match_theme_and_indexed_colors() {
    let observed: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/theme_excel_confirmed.json")).unwrap();
    assert_eq!(observed["schema_version"], 1);
    assert_eq!(observed["excel"]["version"], "16.0");
    assert_eq!(observed["excel"]["build"], "20430.0");
    let slots: String = [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ]
    .into_iter()
    .map(|name| {
        let rgb = observed["theme_scheme_rgb"][name].as_str().unwrap();
        format!("<a:{name}><a:srgbClr val=\"{rgb}\"/></a:{name}>")
    })
    .collect();
    let theme_xml = theme_part(&slots);
    let palette = observed["indexed_palette_rgb"].as_array().unwrap();
    assert_eq!(palette.len(), 64);
    let palette: String = palette
        .iter()
        .map(|rgb| format!("<rgbColor rgb=\"FF{}\"/>", rgb.as_str().unwrap()))
        .collect();
    let styles = format!(
        "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <fonts count=\"1\"><font/></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
         <borders count=\"1\"><border/></borders><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
         <colors><indexedColors>{palette}</indexedColors></colors></styleSheet>"
    );
    let rels = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>\
        <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>";
    let bytes = package(&[
        (
            "[Content_Types].xml",
            content_types(1, false, true).as_str(),
        ),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/worksheets/sheet1.xml", worksheet("").as_str()),
        ("xl/theme/theme1.xml", theme_xml.as_str()),
        ("xl/styles.xml", styles.as_str()),
    ]);
    let book = Workbook::from_bytes(bytes).unwrap();
    let style = book.style_sheet().unwrap();
    let theme = book.theme().unwrap();
    let cases = observed["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 110);
    assert_eq!(
        cases
            .iter()
            .filter(|case| case["input_tint"].as_f64().is_some_and(|tint| tint != 0.0))
            .count(),
        87
    );
    for case in cases {
        let color = match case["kind"].as_str().unwrap() {
            "theme" => Color::Theme {
                index: u8::try_from(case["index"].as_u64().unwrap()).unwrap(),
                tint: case["input_tint"].as_f64().unwrap(),
            },
            "indexed" => Color::Indexed {
                index: u8::try_from(case["index"].as_u64().unwrap()).unwrap(),
                tint: case["input_tint"].as_f64().unwrap(),
            },
            "rgb" => Color::Rgb(u32::from_str_radix(case["argb"].as_str().unwrap(), 16).unwrap()),
            other => panic!("unexpected color kind {other}"),
        };
        match case["wire_tint"].as_str() {
            Some(wire) => assert_eq!(
                wire.parse::<f64>().unwrap(),
                case["input_tint"].as_f64().unwrap()
            ),
            None => {
                assert!(case["input_tint"].is_null() || case["input_tint"].as_f64() == Some(0.0))
            }
        }
        let bgr = u32::try_from(case["com_color_bgr"].as_u64().unwrap()).unwrap();
        let rgb = ((bgr & 0xff) << 16) | (bgr & 0xff00) | ((bgr >> 16) & 0xff);
        assert_eq!(
            style.resolve(&color, theme),
            Some(rgb),
            "Excel {}",
            case["id"]
        );
    }
}

/// Explicit local exporter for the desktop oracle; ordinary tests use the
/// committed native answers above and never open Excel or write a workbook.
#[test]
#[ignore = "run explicitly to export a local desktop tint workbook"]
fn export_desktop_tint_oracle() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;

    use serde_json::json;
    use yggdryl::excel::{
        BorderPreset, BorderStyle, Borders, CellRange, CellRef, Fill, PatternType, StylePatch,
    };

    let output = PathBuf::from(
        std::env::var_os("YGGDRYL_EXCEL_STYLE_EXPORT_DIR").ok_or_else(|| {
            std::io::Error::other("set YGGDRYL_EXCEL_STYLE_EXPORT_DIR to an output directory")
        })?,
    );
    let observed: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/theme_excel_confirmed.json"))?;
    let cases = observed["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 110);
    let mut workbook =
        Workbook::from_bytes(include_bytes!("fixtures/theme_oracle_base.xlsx").to_vec())?;
    for (index, name) in [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ]
    .into_iter()
    .enumerate()
    {
        let rgb = u32::from_str_radix(observed["theme_scheme_rgb"][name].as_str().unwrap(), 16)?;
        assert_eq!(workbook.theme()?.scheme()[index], rgb, "base theme {name}");
    }
    for (index, value) in observed["indexed_palette_rgb"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let rgb = u32::from_str_radix(value.as_str().unwrap(), 16)?;
        let color = Color::Indexed {
            index: u8::try_from(index)?,
            tint: 0.0,
        };
        assert_eq!(
            workbook.style_sheet()?.resolve(&color, workbook.theme()?),
            Some(rgb),
            "base indexed palette {index}"
        );
    }
    let mut exported = Vec::with_capacity(cases.len());
    let mut inputs = Vec::with_capacity(cases.len());
    for (position, case) in cases.iter().enumerate() {
        let tint = case["input_tint"].as_f64().unwrap_or(0.0);
        let color = match case["kind"].as_str().unwrap() {
            "theme" => Color::Theme {
                index: u8::try_from(case["index"].as_u64().unwrap())?,
                tint,
            },
            "indexed" => Color::Indexed {
                index: u8::try_from(case["index"].as_u64().unwrap())?,
                tint,
            },
            "rgb" => Color::Rgb(u32::from_str_radix(case["argb"].as_str().unwrap(), 16)?),
            other => panic!("unexpected fixture color kind {other}"),
        };
        let row = u32::try_from(position * 2 + 1)?;
        let cell = CellRef::new(row, 1);
        workbook
            .sheet_mut("Styles")?
            .set_cell(CellRef::new(row, 0), case["id"].as_str().unwrap())?;
        workbook.sheet_mut("Styles")?.set_cell(cell, 1234.5)?;
        workbook.set_style(
            "Styles",
            &[CellRange::new(cell, cell)],
            &StylePatch {
                font_color: Some(Some(color)),
                fill: Some(Some(color)),
                borders: Some(Borders {
                    preset: BorderPreset::All,
                    style: BorderStyle::Thin,
                    color: Some(color),
                }),
                ..StylePatch::default()
            },
        )?;
        let rgb = workbook
            .style_sheet()?
            .resolve(&color, workbook.theme()?)
            .unwrap();
        let hex = format!("#{rgb:06X}");
        exported.push(json!({
            "id": case["id"], "file": "styles-from-rust.xlsx", "sheet": "Styles",
            "cell": cell.to_string(),
            "expected": {"font": hex.clone(), "fill": hex.clone(),
                         "borders": {"left": hex.clone(), "right": hex.clone(),
                                     "top": hex.clone(), "bottom": hex}}
        }));
        inputs.push((cell, color, rgb));
    }
    let bytes = workbook.into_bytes()?;
    let reopened = Workbook::from_bytes(bytes.clone())?;
    for (cell, color, rgb) in inputs {
        let style = reopened.cell_style("Styles", cell)?;
        assert_eq!(style.font.color, Some(color), "saved font {cell}");
        let Fill::Pattern {
            pattern: PatternType::Solid,
            foreground: Some(fill),
            ..
        } = style.fill
        else {
            panic!("saved fill at {cell} is not the selected solid color");
        };
        assert_eq!(fill, color, "saved fill {cell}");
        for edge in [
            style.border.left,
            style.border.right,
            style.border.top,
            style.border.bottom,
        ] {
            assert_eq!(edge.color, Some(color), "saved border {cell}");
        }
        assert_eq!(
            reopened.style_sheet()?.resolve(&color, reopened.theme()?),
            Some(rgb),
            "saved tint {cell}"
        );
    }
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("styles-from-rust.xlsx"), bytes)?;
    let payload = json!({
        "schema_version": 1, "case_count": exported.len(), "cases": exported,
        "base_workbook": "rust/tests/excel/fixtures/theme_oracle_base.xlsx",
        "expected_source": "StyleSheet::resolve before save and after Rust reopen",
        "excel_verified": false
    });
    let mut text = serde_json::to_vec_pretty(&payload)?;
    text.push(b'\n');
    std::fs::write(output.join("style-cases.json"), text)?;
    Ok(())
}
