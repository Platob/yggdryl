//! `rust/src/excel/styles.rs`: which number formats make a cell a date, a time or a duration, read off `cellXfs` and spliced into a package on write.

use std::sync::Arc;

use yggdryl::excel::{CellRef, NumberFormat, Workbook};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::IORecordOptions;
use yggdryl::zip::ZipArchive;
use yggdryl::{
    DataType, Field, IOBase, IOMedia, MimeType, Scalar, Serie, StructType, TimeUnit, Timezone,
};

use crate::excel_package::{
    NS, content_types, one_sheet, package, root_relationships, styles, workbook,
    workbook_relationships, worksheet,
};

/// The member `name` of the package `bytes`, as text.
fn member(bytes: &[u8], name: &str) -> String {
    let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// The format and value of the cell at `reference` of `Sheet1`.
fn cell_of(bytes: &[u8], reference: &str) -> (NumberFormat, Scalar) {
    let workbook = Workbook::from_bytes(bytes.to_vec()).unwrap();
    let cell = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell(reference.parse().unwrap())
        .unwrap()
        .clone();
    (cell.format(), cell.value().clone())
}

/// One row of the four temporal families a record write lays out as serials.
fn temporal_root() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Date32.nullable_field("day"),
            DataType::datetime64(TimeUnit::Second, Timezone::NAIVE)
                .unwrap()
                .nullable_field("at"),
            DataType::time32(TimeUnit::Millisecond)
                .unwrap()
                .nullable_field("clock"),
            DataType::duration64(TimeUnit::Millisecond)
                .unwrap()
                .nullable_field("elapsed"),
        ])
        .unwrap(),
    )
    .required_field("row")
}

/// 2024-01-01, 2024-01-01T06:00:00, 12:00 and 36 hours.
fn temporal_row() -> Scalar {
    Scalar::from_sequence([
        Scalar::date32(19_723),
        Scalar::datetime64(1_704_088_800, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::time32(43_200_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap(),
    ])
}

/// Write the temporal row as records over `bytes` (a package, or nothing)
/// and answer the package the handle holds after.
fn write_temporal_row(bytes: Vec<u8>) -> Vec<u8> {
    let root = temporal_root();
    let batch = Serie::from_scalars(root.clone(), [temporal_row()])
        .unwrap()
        .into_arrow_batch()
        .unwrap();
    let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap().with_field(root);
    handle.overwrite_arrow_batch(batch, &options).unwrap();
    handle.read_all_bytes().unwrap()
}

/// The values the temporal row reads back as, one per format.
fn read_back_row(bytes: &[u8]) {
    let (format, value) = cell_of(bytes, "A2");
    assert_eq!(format, NumberFormat::Date);
    assert_eq!(value, Scalar::date32(19_723));
    let (format, value) = cell_of(bytes, "B2");
    assert_eq!(format, NumberFormat::DateTime);
    assert_eq!(
        value,
        Scalar::datetime64(1_704_088_800_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
    );
    let (format, value) = cell_of(bytes, "C2");
    assert_eq!(format, NumberFormat::Time);
    assert_eq!(
        value,
        Scalar::time32(43_200_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
    );
    let (format, value) = cell_of(bytes, "D2");
    assert_eq!(format, NumberFormat::Duration);
    assert_eq!(
        value,
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap()
    );
}

#[test]
fn every_builtin_date_id_reads_as_a_date() {
    for id in [14, 15, 16, 17, 27, 28, 29, 30, 31, 36, 50, 51, 54, 57, 58] {
        assert_eq!(
            NumberFormat::builtin(id),
            Some(NumberFormat::Date),
            "numFmtId {id}"
        );
    }
}

#[test]
fn every_builtin_time_id_reads_as_a_time() {
    for id in [18, 19, 20, 21, 32, 33, 34, 35, 45, 47, 52, 53, 55, 56] {
        assert_eq!(
            NumberFormat::builtin(id),
            Some(NumberFormat::Time),
            "numFmtId {id}"
        );
    }
}

#[test]
fn builtin_22_is_a_datetime_and_46_the_elapsed_duration() {
    assert_eq!(NumberFormat::builtin(22), Some(NumberFormat::DateTime));
    assert_eq!(NumberFormat::builtin(46), Some(NumberFormat::Duration));
}

#[test]
fn the_builtin_number_formats_read_as_general() {
    for id in (0..=13).chain(37..=44).chain([48, 49]) {
        assert_eq!(
            NumberFormat::builtin(id),
            Some(NumberFormat::General),
            "numFmtId {id}"
        );
    }
}

#[test]
fn an_id_no_builtin_table_lists_answers_none() {
    for id in [23, 24, 25, 26, 59, 100, 163, 164, 165, u32::MAX] {
        assert_eq!(NumberFormat::builtin(id), None, "numFmtId {id}");
    }
    // Exactly the ids ECMA-376 implies are built in, and nothing past 58.
    let listed: Vec<u32> = (0..=400)
        .filter(|id| NumberFormat::builtin(*id).is_some())
        .collect();
    let expected: Vec<u32> = (0..=22).chain(27..=58).collect();
    assert_eq!(listed, expected);
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
        ("m/d/yy h:mm", NumberFormat::DateTime),
        ("[h]:mm:ss", NumberFormat::Duration),
    ] {
        assert_eq!(NumberFormat::from_code(code), format, "{code}");
    }
}

#[test]
fn m_alone_is_a_month_and_beside_a_clock_is_minutes() {
    assert_eq!(NumberFormat::from_code("mm"), NumberFormat::Date);
    assert_eq!(NumberFormat::from_code("mmmm"), NumberFormat::Date);
    assert_eq!(NumberFormat::from_code("h:mm"), NumberFormat::Time);
    assert_eq!(NumberFormat::from_code("mm:ss"), NumberFormat::Time);
}

#[test]
fn only_the_first_section_of_a_code_counts() {
    assert_eq!(
        NumberFormat::from_code("0.00;[Red]yyyy-mm-dd"),
        NumberFormat::General
    );
    assert_eq!(
        NumberFormat::from_code("yyyy-mm-dd;0.00"),
        NumberFormat::Date
    );
    assert_eq!(NumberFormat::from_code("hh:mm;@"), NumberFormat::Time);
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
        assert_eq!(
            NumberFormat::from_code(code),
            NumberFormat::General,
            "{code}"
        );
    }
    // A literal beside a real token leaves the token's reading.
    assert_eq!(NumberFormat::from_code("yyyy \"h\""), NumberFormat::Date);
    assert_eq!(NumberFormat::from_code("\\d hh:mm"), NumberFormat::Time);
}

#[test]
fn a_bracketed_colour_locale_or_condition_is_a_literal() {
    assert_eq!(NumberFormat::from_code("[Red]0.00"), NumberFormat::General);
    assert_eq!(NumberFormat::from_code("[>=100]0"), NumberFormat::General);
    assert_eq!(NumberFormat::from_code("[Red]hh:mm"), NumberFormat::Time);
    assert_eq!(
        NumberFormat::from_code("[$-409]yyyy-mm-dd"),
        NumberFormat::Date
    );
    assert_eq!(
        NumberFormat::from_code("[$-F800]dddd, mmmm dd, yyyy"),
        NumberFormat::Date
    );
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
        assert_eq!(
            NumberFormat::from_code(code),
            NumberFormat::Duration,
            "{code}"
        );
    }
    // Two units in one bracket are not an elapsed unit.
    assert_eq!(NumberFormat::from_code("[hm]0"), NumberFormat::General);
}

#[test]
fn general_text_and_number_codes_are_general() {
    for code in [
        "General", "", "@", "0", "0.00", "#,##0.00", "0.00E+00", "0%", "# ?/?",
    ] {
        assert_eq!(
            NumberFormat::from_code(code),
            NumberFormat::General,
            "{code}"
        );
    }
}

#[test]
fn all_lists_the_formats_in_the_cellxfs_order_they_are_written() {
    assert_eq!(
        NumberFormat::ALL,
        [
            NumberFormat::General,
            NumberFormat::Date,
            NumberFormat::DateTime,
            NumberFormat::DateTimeFraction,
            NumberFormat::Time,
            NumberFormat::Duration,
        ]
    );
    for (index, format) in NumberFormat::ALL.into_iter().enumerate() {
        assert_eq!(format.style_index(), index as u32, "{format:?}");
    }
}

#[test]
fn each_format_states_the_code_it_is_written_under() {
    let codes: Vec<Option<&str>> = NumberFormat::ALL
        .into_iter()
        .map(NumberFormat::code)
        .collect();
    assert_eq!(
        codes,
        vec![
            None,
            Some("yyyy-mm-dd"),
            Some("yyyy-mm-dd hh:mm:ss"),
            Some("yyyy-mm-dd hh:mm:ss.000"),
            Some("hh:mm:ss"),
            Some("[h]:mm:ss"),
        ]
    );
    // Each code reads back as its format, the fraction included: a sheet
    // this crate writes round-trips cell for cell, format and all.
    for format in NumberFormat::ALL {
        assert_eq!(
            NumberFormat::from_code(format.code().unwrap_or("General")),
            format
        );
    }
}

#[test]
fn only_general_is_not_temporal_and_general_is_the_default() {
    let temporal: Vec<bool> = NumberFormat::ALL
        .into_iter()
        .map(NumberFormat::is_temporal)
        .collect();
    assert_eq!(temporal, vec![false, true, true, true, true, true]);
    assert_eq!(NumberFormat::default(), NumberFormat::General);
}

#[test]
fn a_custom_date_code_the_part_declares_makes_a_number_cell_a_date() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c><c r=\"B1\"><v>45292</v></c></row>",
        &[],
        &[(164, "yyyy-mm-dd")],
        &[0, 164],
    );
    let (format, value) = cell_of(&bytes, "A1");
    assert_eq!(format, NumberFormat::Date);
    assert_eq!(value.into_json().unwrap(), "\"2024-01-01\"");
    let (format, value) = cell_of(&bytes, "B1");
    assert_eq!(format, NumberFormat::General);
    assert_eq!(value, Scalar::from(45_292.0));
}

#[test]
fn builtin_ids_in_cellxfs_classify_their_cells() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c><c r=\"B1\" s=\"2\"><v>0.5</v></c>\
         <c r=\"C1\" s=\"3\"><v>45292.25</v></c><c r=\"D1\" s=\"4\"><v>1.5</v></c></row>",
        &[],
        &[],
        &[0, 14, 21, 22, 46],
    );
    let (format, value) = cell_of(&bytes, "A1");
    assert_eq!(format, NumberFormat::Date);
    assert_eq!(value, Scalar::date32(19_723));
    let (format, value) = cell_of(&bytes, "B1");
    assert_eq!(format, NumberFormat::Time);
    assert_eq!(
        value,
        Scalar::time32(43_200_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
    );
    let (format, value) = cell_of(&bytes, "C1");
    assert_eq!(format, NumberFormat::DateTime);
    assert_eq!(value.into_json().unwrap(), "\"2024-01-01T06:00:00\"");
    let (format, value) = cell_of(&bytes, "D1");
    assert_eq!(format, NumberFormat::Duration);
    assert_eq!(
        value,
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap()
    );
}

#[test]
fn a_style_index_past_cellxfs_is_general() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>45292</v></c><c r=\"B1\" s=\"4294967295\"><v>1</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::General, Scalar::from(45_292.0))
    );
    assert_eq!(
        cell_of(&bytes, "B1"),
        (NumberFormat::General, Scalar::from(1.0))
    );
}

#[test]
fn a_package_without_a_styles_part_reads_general_everywhere() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c><c r=\"B1\" s=\"0\"><v>0.5</v></c></row>",
        &[],
        &[],
        &[],
    );
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::General, Scalar::from(45_292.0))
    );
    assert_eq!(
        cell_of(&bytes, "B1"),
        (NumberFormat::General, Scalar::from(0.5))
    );
}

#[test]
fn a_code_the_part_declares_under_a_builtin_id_wins_over_the_builtin() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c></row>",
        &[],
        &[(14, "0.00")],
        &[0, 14],
    );
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::General, Scalar::from(45_292.0))
    );
}

#[test]
fn a_custom_id_the_part_declares_no_code_for_is_general() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c></row>",
        &[],
        &[(164, "yyyy-mm-dd")],
        &[0, 200],
    );
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::General, Scalar::from(45_292.0))
    );
}

#[test]
fn only_cellxfs_indexes_a_cell_style_and_an_xf_without_an_id_is_general() {
    // `cellStyleXfs` states a date and comes first; were it counted, `s="1"`
    // would land on its General neighbour instead of the datetime.
    let styles_part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <styleSheet xmlns=\"{NS}\">\
         <cellStyleXfs count=\"1\"><xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
         <cellXfs count=\"3\"><xf numFmtId=\"0\" xfId=\"0\"/><xf numFmtId=\"22\" xfId=\"0\"/><xf xfId=\"0\"/></cellXfs>\
         </styleSheet>"
    );
    let sheet = worksheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292.5</v></c><c r=\"B1\" s=\"2\"><v>45292</v></c></row>",
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/styles.xml", &styles_part),
    ]);
    let (format, value) = cell_of(&bytes, "A1");
    assert_eq!(format, NumberFormat::DateTime);
    assert_eq!(value.into_json().unwrap(), "\"2024-01-01T12:00:00\"");
    assert_eq!(
        cell_of(&bytes, "B1"),
        (NumberFormat::General, Scalar::from(45_292.0))
    );
}

#[test]
fn a_styles_part_that_is_not_xml_is_refused_as_xlsx_data() {
    let sheet = worksheet("<row r=\"1\"><c r=\"A1\" s=\"1\"><v>1</v></c></row>");
    let broken = "<styleSheet><cellXfs count=\"1\"><xf numFmtId=\"14\"/></numFmts></styleSheet>";
    let bytes = package(&[
        ("[Content_Types].xml", &content_types(1, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(1, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/styles.xml", broken),
    ]);
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let refusal = workbook.sheet("Sheet1").unwrap_err().to_string();
    assert!(refusal.contains("invalid xlsx data at byte"), "{refusal}");
}

#[test]
fn a_fresh_package_carries_the_six_cell_formats_this_crate_writes() {
    let bytes = write_temporal_row(Vec::new());
    let styles_xml = member(&bytes, "xl/styles.xml");
    assert!(
        styles_xml.contains(
            "<numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/>\
             <numFmt numFmtId=\"166\" formatCode=\"yyyy-mm-dd hh:mm:ss.000\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains(
            "<cellXfs count=\"6\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
             <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"166\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"21\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"46\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains("<cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/>"),
        "{styles_xml}"
    );
    // Each temporal cell states its format's own index.
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    for (reference, index) in [("A2", 1), ("B2", 2), ("C2", 4), ("D2", 5)] {
        assert!(
            sheet_xml.contains(&format!("<c r=\"{reference}\" s=\"{index}\">")),
            "{reference}: {sheet_xml}"
        );
    }
    read_back_row(&bytes);
}

#[test]
fn overwriting_a_package_keeps_its_cellxfs_and_offsets_the_crate_styles_past_them() {
    // The sheet written over states no rows, so the records take their own
    // shape rather than a stored header's.
    let original = one_sheet("", &[], &[(164, "0.000")], &[0, 164, 14]);
    let bytes = write_temporal_row(original);
    let styles_xml = member(&bytes, "xl/styles.xml");
    // The part's own formats stand where they were, and the crate's three
    // custom codes take ids above the one it declares.
    assert!(
        styles_xml.contains(
            "<numFmts count=\"4\"><numFmt numFmtId=\"164\" formatCode=\"0.000\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"166\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/>\
             <numFmt numFmtId=\"167\" formatCode=\"yyyy-mm-dd hh:mm:ss.000\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains(
            "<cellXfs count=\"9\">\
             <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
             <xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>"
        ),
        "{styles_xml}"
    );
    // The crate's cells state their format's index past the three kept.
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    for (reference, index) in [("A2", 4), ("B2", 5), ("C2", 7), ("D2", 8)] {
        assert!(
            sheet_xml.contains(&format!("<c r=\"{reference}\" s=\"{index}\">")),
            "{reference}: {sheet_xml}"
        );
    }
    read_back_row(&bytes);
}

#[test]
fn a_package_without_numfmts_gains_the_crate_codes_from_164() {
    let original = one_sheet("", &[], &[], &[0, 14]);
    let bytes = write_temporal_row(original);
    let styles_xml = member(&bytes, "xl/styles.xml");
    assert!(
        styles_xml.contains(
            "<numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/>\
             <numFmt numFmtId=\"166\" formatCode=\"yyyy-mm-dd hh:mm:ss.000\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    assert!(styles_xml.contains("<cellXfs count=\"8\">"), "{styles_xml}");
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet_xml.contains("<c r=\"A2\" s=\"3\">"), "{sheet_xml}");
    read_back_row(&bytes);
}

#[test]
fn overwriting_one_sheet_keeps_the_formats_the_other_sheets_state() {
    let sheet1 = worksheet("");
    let sheet2 = worksheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c><c r=\"B1\" s=\"2\"><v>45292.5</v></c></row>",
    );
    let styles_part = styles(&[(164, "yyyy-mm-dd")], &[0, 164, 22]);
    let original = package(&[
        ("[Content_Types].xml", &content_types(2, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Sheet1", "Sheet2"], false)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(2, false, true),
        ),
        ("xl/worksheets/sheet1.xml", &sheet1),
        ("xl/worksheets/sheet2.xml", &sheet2),
        ("xl/styles.xml", &styles_part),
    ]);
    let bytes = write_temporal_row(original);
    read_back_row(&bytes);
    let workbook = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Sheet1", "Sheet2"]);
    let other = workbook.sheet("Sheet2").unwrap();
    let day = other.cell(CellRef::new(0, 0)).unwrap();
    assert_eq!(day.format(), NumberFormat::Date);
    assert_eq!(day.value(), &Scalar::date32(19_723));
    let when = other.cell(CellRef::new(0, 1)).unwrap();
    assert_eq!(when.format(), NumberFormat::DateTime);
    assert_eq!(when.value().into_json().unwrap(), "\"2024-01-01T12:00:00\"");
}
