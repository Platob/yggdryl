//! `rust/src/excel/styles.rs`: `StyleSheet` - every cell format a styles part resolves to, which number formats make a cell a date, a time or a duration, and the append-only splice a save writes.

use std::sync::Arc;

use yggdryl::excel::{
    Alignment, Border, BorderStyle, CellRef, Color, Edge, Fill, Font, FontScheme, FormatCode,
    Horizontal, NumberFormat, PatternType, Protection, StyleId, Underline, Vertical, VerticalRun,
    Workbook,
};
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
fn all_lists_every_format_general_first_and_each_reads_back_by_its_name() {
    assert_eq!(
        NumberFormat::ALL,
        [
            NumberFormat::General,
            NumberFormat::Date,
            NumberFormat::Time,
            NumberFormat::DateTime,
            NumberFormat::DateTimeFraction,
            NumberFormat::Duration,
        ]
    );
    for format in NumberFormat::ALL {
        assert_eq!(format.as_str().parse::<NumberFormat>().unwrap(), format);
    }
}

#[test]
fn each_format_states_the_code_it_is_written_under() {
    let codes: Vec<Option<&str>> = NumberFormat::ALL
        .into_iter()
        .map(NumberFormat::code)
        .collect();
    // `h:mm:ss` and `[h]:mm:ss` are the built-in formats 21 and 46.
    assert_eq!(
        codes,
        vec![
            None,
            Some("yyyy-mm-dd"),
            Some("h:mm:ss"),
            Some("yyyy-mm-dd hh:mm:ss"),
            Some("yyyy-mm-dd hh:mm:ss.000"),
            Some("[h]:mm:ss"),
        ]
    );
    // Each code reads back as its format, the fraction included: a sheet
    // this crate writes round-trips cell for cell, format and all.
    for format in NumberFormat::ALL {
        assert_eq!(
            FormatCode::from_code(format.code().unwrap_or("General"))
                .unwrap()
                .kind(),
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
    assert_eq!(value.into_json().unwrap(), "\"2024-01-01T06:00:00.000\"");
    let (format, value) = cell_of(&bytes, "D1");
    assert_eq!(format, NumberFormat::Duration);
    assert_eq!(
        value,
        Scalar::duration64(129_600_000, TimeUnit::Millisecond).unwrap()
    );
}

#[test]
fn a_style_index_past_sixteen_bits_is_refused_naming_the_cell() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>45292</v></c><c r=\"B1\" s=\"65536\"><v>1</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let workbook = Workbook::from_bytes(bytes.clone()).unwrap();
    let refusal = workbook.sheet("Sheet1").unwrap_err().to_string();
    assert!(
        refusal.starts_with("invalid record value at Sheet1!B1: at byte "),
        "{refusal}"
    );
    assert!(
        refusal.ends_with("expected a style index from 0 to 65535 in a cell's `s`, got \"65536\""),
        "{refusal}"
    );
    // The record path reads the part through the same parser.
    let mut handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    handle.set_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap();
    let read = handle
        .read_arrow_reader(&options)
        .and_then(|mut reader| reader.try_fold(0, |rows, batch| Ok(rows + batch?.num_rows())));
    let refusal = read.unwrap_err().to_string();
    assert!(refusal.contains("Sheet1!B1"), "{refusal}");
    assert!(refusal.contains("65536"), "{refusal}");
}

#[test]
fn a_style_index_past_cellxfs_is_general() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"2\"><v>45292</v></c><c r=\"B1\" s=\"65535\"><v>1</v></c></row>",
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
    // An index naming no cell format reads as the default style, which is
    // what a save writes it back as; one naming a format is kept.
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(
        sheet.cell("A1".parse().unwrap()).unwrap().style(),
        StyleId::DEFAULT
    );
    assert_eq!(
        sheet.cell("B1".parse().unwrap()).unwrap().style(),
        StyleId::DEFAULT
    );
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45292</v></c></row>",
        &[],
        &[],
        &[0, 14],
    );
    let workbook = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .cell("A1".parse().unwrap())
            .unwrap()
            .style(),
        StyleId::new(1)
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
    assert_eq!(value.into_json().unwrap(), "\"2024-01-01T12:00:00.000\"");
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
        (
            "[Content_Types].xml",
            content_types(1, false, true).as_str(),
        ),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, true).as_str(),
        ),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ("xl/styles.xml", broken),
    ]);
    let workbook = Workbook::from_bytes(bytes).unwrap();
    let refusal = workbook.sheet("Sheet1").unwrap_err().to_string();
    assert!(refusal.contains("invalid xlsx data at byte"), "{refusal}");
}

#[test]
fn a_fresh_package_interns_the_formats_its_columns_write_after_the_default() {
    let bytes = write_temporal_row(Vec::new());
    let styles_xml = member(&bytes, "xl/styles.xml");
    // Two codes the built-in table lacks are declared from 164, in column
    // order; `h:mm:ss` and `[h]:mm:ss` are the built-ins 21 and 46.
    assert!(
        styles_xml.contains(
            "<numFmts count=\"2\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/></numFmts><fonts"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains(
            "<cellXfs count=\"5\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
             <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
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
    for (reference, index) in [("A2", 1), ("B2", 2), ("C2", 3), ("D2", 4)] {
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
    // The part's own formats stand where they were, and the two codes the
    // columns need that neither it nor the built-in table has take ids
    // above the one it declares.
    assert!(
        styles_xml.contains(
            "<numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"0.000\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"166\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains(
            "<cellXfs count=\"7\">\
             <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"166\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"21\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
             <xf numFmtId=\"46\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>"
        ),
        "{styles_xml}"
    );
    // The crate's cells state the indexes appended past the three kept.
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    for (reference, index) in [("A2", 3), ("B2", 4), ("C2", 5), ("D2", 6)] {
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
    // The list goes in at its place in the schema, before the fonts.
    assert!(
        styles_xml.contains(
            "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <numFmts count=\"2\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"165\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/></numFmts><fonts"
        ),
        "{styles_xml}"
    );
    // `m/d/yyyy` (built-in 14) reads as a date but is not the code the
    // crate writes a date under, so the date takes an entry of its own.
    assert!(styles_xml.contains("<cellXfs count=\"6\">"), "{styles_xml}");
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet_xml.contains("<c r=\"A2\" s=\"2\">"), "{sheet_xml}");
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
    assert_eq!(
        when.value().into_json().unwrap(),
        "\"2024-01-01T12:00:00.000\""
    );
}

/// A one-sheet package whose styles part is `styles_part` and whose sheet
/// holds `sheet_data`.
fn styled(sheet_data: &str, styles_part: &str) -> Vec<u8> {
    package(&[
        (
            "[Content_Types].xml",
            content_types(1, false, true).as_str(),
        ),
        ("_rels/.rels", root_relationships().as_str()),
        ("xl/workbook.xml", workbook(&["Sheet1"], false).as_str()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_relationships(1, false, true).as_str(),
        ),
        ("xl/worksheets/sheet1.xml", worksheet(sheet_data).as_str()),
        ("xl/styles.xml", styles_part),
    ])
}

/// A styles part stating one of everything the model reads, and one of
/// everything it carries.
fn rich_styles() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <styleSheet xmlns=\"{NS}\">\
         <numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.00%\"/></numFmts>\
         <fonts count=\"2\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font>\
         <font><b/><i val=\"1\"/><strike val=\"true\"/><condense/><extend/><outline/><shadow/><u val=\"double\"/>\
         <vertAlign val=\"superscript\"/><sz val=\"14.5\"/><color rgb=\"FFFF0000\"/><name val=\"Arial\"/><family val=\"2\"/>\
         <charset val=\"0\"/><scheme val=\"none\"/></font></fonts>\
         <fills count=\"4\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill>\
         <fill><patternFill patternType=\"solid\"><fgColor theme=\"4\" tint=\"-0.25\"/><bgColor indexed=\"64\"/></patternFill></fill>\
         <fill><gradientFill degree=\"90\"><stop position=\"0\"><color rgb=\"FF00FF00\"/></stop></gradientFill></fill></fills>\
         <borders count=\"2\"><border><left/><right/><top/><bottom/><diagonal/></border>\
         <border diagonalUp=\"1\" diagonalDown=\"true\" outline=\"0\"><left style=\"thin\"><color rgb=\"FF000000\"/></left>\
         <right style=\"medium\"><color theme=\"1\"/></right><top style=\"dashed\"><color auto=\"1\"/></top>\
         <bottom style=\"double\"><color indexed=\"8\"/></bottom><diagonal style=\"hair\"/>\
         <vertical style=\"thick\"/><horizontal style=\"dotted\"/></border></borders>\
         <cellStyleXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
         <xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
         <cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
         <xf numFmtId=\"164\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"1\" quotePrefix=\"1\" applyNumberFormat=\"1\">\
         <alignment horizontal=\"centerContinuous\" vertical=\"top\" textRotation=\"45\" wrapText=\"1\" indent=\"2\" \
         relativeIndent=\"-1\" justifyLastLine=\"1\" shrinkToFit=\"true\" readingOrder=\"2\"/>\
         <protection locked=\"0\" hidden=\"1\"/></xf>\
         <xf numFmtId=\"42\" fontId=\"0\" fillId=\"3\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
         <cellStyles count=\"2\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/><cellStyle name=\"Loud\" xfId=\"1\"/></cellStyles>\
         <dxfs count=\"1\"><dxf><font><b/><sz val=\"20\"/></font><numFmt numFmtId=\"170\" formatCode=\"0.0\"/>\
         <fill><patternFill><bgColor rgb=\"FFFFFF00\"/></patternFill></fill><border><left style=\"thin\"/></border></dxf></dxfs>\
         <tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/>\
         <colors><indexedColors><rgbColor rgb=\"FF000000\"/><rgbColor rgb=\"00FFFFFF\"/></indexedColors>\
         <mruColors><color rgb=\"FF123456\"/></mruColors></colors>\
         <extLst><ext uri=\"{{EB79DEF2-80B8-43e5-95BD-54CBDDF9020C}}\"><x14:slicerStyles defaultSlicerStyle=\"SlicerStyleLight1\" \
         xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"/></ext></extLst>\
         </styleSheet>"
    )
}

#[test]
fn a_style_sheet_resolves_each_cell_format_into_every_fact_it_points_at() {
    let workbook = Workbook::from_bytes(styled("", &rich_styles())).unwrap();
    let sheet = workbook.style_sheet().unwrap();
    assert_eq!(sheet.len(), 3);
    assert!(!sheet.is_empty());
    assert!(sheet.style(StyleId::new(3)).is_none());
    assert_eq!(
        sheet.indexed_colors(),
        Some(&[0xFF00_0000, 0x00FF_FFFF][..])
    );

    let plain = sheet.style(StyleId::DEFAULT).unwrap();
    assert_eq!(plain.number_format, "General");
    assert_eq!(plain.font.name, "Calibri");
    assert_eq!(plain.font.size, 11.0);
    assert_eq!(
        plain.font.color,
        Some(Color::Theme {
            index: 1,
            tint: 0.0
        })
    );
    assert_eq!(plain.font.family, Some(2));
    assert_eq!(plain.font.scheme, Some(FontScheme::Minor));
    assert_eq!(plain.fill, Fill::None);
    assert_eq!(plain.border, Border::default());
    assert_eq!(plain.alignment, Alignment::default());
    assert_eq!(plain.protection, Protection::default());
    assert!(!plain.quote_prefix);

    let loud = sheet.style(StyleId::new(1)).unwrap();
    assert_eq!(loud.number_format, "0.00%");
    assert_eq!(
        loud.font,
        Font {
            name: "Arial".into(),
            size: 14.5,
            bold: true,
            italic: true,
            underline: Underline::Double,
            strike: true,
            color: Some(Color::Rgb(0xFFFF_0000)),
            vertical: VerticalRun::Superscript,
            family: Some(2),
            scheme: Some(FontScheme::None),
            charset: Some(0),
            outline: true,
            shadow: true,
            condense: true,
            extend: true,
        }
    );
    assert_eq!(
        loud.fill,
        Fill::Pattern {
            pattern: PatternType::Solid,
            foreground: Some(Color::Theme {
                index: 4,
                tint: -0.25
            }),
            background: Some(Color::Indexed {
                index: 64,
                tint: 0.0
            }),
        }
    );
    let edge = |style, color| Edge { style, color };
    assert_eq!(
        loud.border,
        Border {
            left: edge(BorderStyle::Thin, Some(Color::Rgb(0xFF00_0000))),
            right: edge(
                BorderStyle::Medium,
                Some(Color::Theme {
                    index: 1,
                    tint: 0.0
                })
            ),
            top: edge(BorderStyle::Dashed, Some(Color::Auto)),
            bottom: edge(
                BorderStyle::Double,
                Some(Color::Indexed {
                    index: 8,
                    tint: 0.0
                })
            ),
            diagonal: edge(BorderStyle::Hair, None),
            vertical: edge(BorderStyle::Thick, None),
            horizontal: edge(BorderStyle::Dotted, None),
            diagonal_up: true,
            diagonal_down: true,
            outline: false,
        }
    );
    assert_eq!(
        loud.alignment,
        Alignment {
            horizontal: Horizontal::CenterContinuous,
            vertical: Vertical::Top,
            rotation: 45,
            wrap: true,
            indent: 2,
            relative_indent: -1,
            justify_last_line: true,
            shrink_to_fit: true,
            reading_order: 2,
        }
    );
    assert_eq!(
        loud.protection,
        Protection {
            locked: false,
            hidden: true
        }
    );
    assert!(loud.quote_prefix);
    assert_eq!(loud.parent, 1);

    // A gradient is carried as the element the part states; an id the part
    // does not declare displays with its en-US code - here 42, one of the
    // accounting formats Excel declares whenever it uses them.
    let other = sheet.style(StyleId::new(2)).unwrap();
    match &other.fill {
        Fill::Gradient(raw) => assert_eq!(
            std::str::from_utf8(raw).unwrap(),
            "<gradientFill degree=\"90\"><stop position=\"0\"><color rgb=\"FF00FF00\"/></stop></gradientFill>"
        ),
        other => panic!("expected a gradient, got {other:?}"),
    }
    assert_eq!(
        other.number_format,
        r#"_("$"* #,##0_);_("$"* \(#,##0\);_("$"* "-"_);_(@_)"#
    );
}

#[test]
fn an_id_the_part_does_not_declare_displays_with_its_builtin_code() {
    let part = styles(&[], &[0, 14, 22, 21, 30, 55, 200]);
    let workbook = Workbook::from_bytes(styled("", &part)).unwrap();
    let sheet = workbook.style_sheet().unwrap();
    let code = |index: u16| {
        sheet
            .style(StyleId::new(index))
            .unwrap()
            .number_format
            .clone()
    };
    assert_eq!(code(1), "m/d/yyyy");
    assert_eq!(code(2), "m/d/yyyy h:mm");
    assert_eq!(code(3), "h:mm:ss");
    // An East Asian locale id reads as what it is in en-US; one no table
    // lists is General.
    assert_eq!(code(4), "m/d/yyyy");
    assert_eq!(code(5), "h:mm:ss");
    assert_eq!(code(6), "General");
}

#[test]
fn a_styles_attribute_the_schema_does_not_spell_is_refused_at_its_element() {
    for (element, reason) in [
        (
            "<b val=\"maybe\"/>",
            "expected 1, true, 0 or false for `val`, got \"maybe\"",
        ),
        (
            "<u val=\"triple\"/>",
            "expected one of none, single, double, singleAccounting, doubleAccounting, got \
             \"triple\"",
        ),
        (
            "<sz val=\"big\"/>",
            "expected a number for a font size, got \"big\"",
        ),
        (
            "<color rgb=\"red\"/>",
            "expected eight hex digits of ARGB for a colour, got \"red\"",
        ),
    ] {
        let part = format!(
            "<styleSheet xmlns=\"{NS}\"><fonts count=\"1\"><font>{element}</font></fonts>\
             <cellXfs count=\"1\"><xf numFmtId=\"0\"/></cellXfs></styleSheet>"
        );
        let byte = part.find(element).unwrap();
        let workbook = Workbook::from_bytes(styled("", &part)).unwrap();
        assert_eq!(
            workbook.style_sheet().unwrap_err().to_string(),
            format!("invalid xlsx data at byte {byte}: {reason}"),
            "{element}"
        );
        // The sheet reads through the same styles, so it is refused alike.
        assert!(workbook.sheet("Sheet1").is_err());
    }
}

#[test]
fn a_save_appends_to_the_styles_and_carries_everything_the_model_does_not_read() {
    let original = rich_styles();
    let mut workbook = Workbook::from_bytes(styled("", &original)).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let written = member(&bytes, "xl/styles.xml");
    // The one entry the date needed and the code it names go after the
    // part's own, their counts stated again - the code past the 170 a
    // differential format declares, the id space every `numFmt` shares; the
    // font a `dxf` states is not one of the fonts, and every list the model
    // does not read is as it was.
    let expected = original
        .replacen("<numFmts count=\"1\">", "<numFmts count=\"2\">", 1)
        .replacen(
            "</numFmts>",
            "<numFmt numFmtId=\"171\" formatCode=\"yyyy-mm-dd\"/></numFmts>",
            1,
        )
        .replacen("<cellXfs count=\"3\">", "<cellXfs count=\"4\">", 1)
        .replacen(
            "</cellXfs>",
            "<xf numFmtId=\"171\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>",
            1,
        );
    assert_eq!(written, expected);
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains("<c r=\"A1\" s=\"3\"><v>45292</v></c>"),
        "{sheet_xml}"
    );
    // Read again, the styles hold the entry at the index the cell states.
    let reopened = Workbook::from_bytes(bytes).unwrap();
    let sheet = reopened.style_sheet().unwrap();
    assert_eq!(sheet.len(), 4);
    assert_eq!(
        sheet.style(StyleId::new(3)).unwrap().number_format,
        "yyyy-mm-dd"
    );
    assert_eq!(sheet.style(StyleId::new(1)).unwrap().font.name, "Arial");
}

#[test]
fn a_new_code_takes_an_id_past_163_and_every_declared_id_and_a_redeclared_builtin_is_not_reused() {
    // The part declares 21 as a code of its own, so `h:mm:ss` cannot name
    // it; the new codes take ids past the highest declared.
    let original = one_sheet("", &[], &[(21, "0.00"), (170, "0.000")], &[0]);
    let bytes = write_temporal_row(original);
    let styles_xml = member(&bytes, "xl/styles.xml");
    assert!(
        styles_xml.contains(
            "<numFmts count=\"5\"><numFmt numFmtId=\"21\" formatCode=\"0.00\"/>\
             <numFmt numFmtId=\"170\" formatCode=\"0.000\"/>\
             <numFmt numFmtId=\"171\" formatCode=\"yyyy-mm-dd\"/>\
             <numFmt numFmtId=\"172\" formatCode=\"yyyy-mm-dd hh:mm:ss\"/>\
             <numFmt numFmtId=\"173\" formatCode=\"h:mm:ss\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    // `[h]:mm:ss` is still the built-in 46, which the part leaves alone.
    assert!(
        styles_xml.contains("<xf numFmtId=\"46\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>"),
        "{styles_xml}"
    );
    read_back_row(&bytes);
}

#[test]
fn a_style_the_part_already_holds_is_reused_and_the_part_carried_as_it_was() {
    // Each temporal code has an entry of the default style, so none is
    // appended and the part is copied as it is stored.
    let original = one_sheet(
        "",
        &[],
        &[
            (164, "yyyy-mm-dd"),
            (165, "yyyy-mm-dd hh:mm:ss"),
            (166, "h:mm:ss"),
        ],
        &[0, 46, 166, 165, 164],
    );
    let bytes = write_temporal_row(original.clone());
    assert_eq!(
        member(&bytes, "xl/styles.xml"),
        member(&original, "xl/styles.xml")
    );
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    for (reference, index) in [("A2", 4), ("B2", 3), ("C2", 2), ("D2", 1)] {
        assert!(
            sheet_xml.contains(&format!("<c r=\"{reference}\" s=\"{index}\">")),
            "{reference}: {sheet_xml}"
        );
    }
    read_back_row(&bytes);
}

#[test]
fn a_workbook_holding_as_many_cell_formats_as_excel_opens_refuses_one_more() {
    // 64,000 entries, none of them a date: the date a sheet writes would be
    // the 64,001st.
    let original = one_sheet("", &[], &[], &vec![0; yggdryl::excel::MAX_CELL_FORMATS]);
    let mut workbook = Workbook::from_bytes(original).unwrap();
    assert_eq!(
        workbook.style_sheet().unwrap().len(),
        yggdryl::excel::MAX_CELL_FORMATS
    );
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    assert_eq!(
        workbook.into_bytes().unwrap_err().to_string(),
        "invalid record value at $.styles: expected at most 64000 cell formats in a workbook, \
         which is as many as Excel opens, got one more"
    );
}

/// A styles part stating fonts, fills and borders but no cell format:
/// `cell_xfs` is spelled as it stands, `""` for no list at all.
fn formatless(cell_xfs: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <styleSheet xmlns=\"{NS}\">\
         <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
         <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
         <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
         {cell_xfs}\
         <cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
         </styleSheet>"
    )
}

#[test]
fn a_styles_part_listing_no_cell_format_saves_a_cell_stating_none_and_appends_past_the_default() {
    for cell_xfs in ["", "<cellXfs count=\"0\"/>"] {
        let original = formatless(cell_xfs);
        let bytes = styled("<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>", &original);
        let mut workbook = Workbook::from_bytes(bytes).unwrap();
        // The model holds the entry index 0 names whether or not the part
        // lists it.
        assert_eq!(workbook.style_sheet().unwrap().len(), 1, "{cell_xfs}");

        // A number beside the unstyled one saves, the styles copied as
        // stored: nothing needed an entry.
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .set_cell("B1".parse().unwrap(), 2.5)
            .unwrap();
        let numbers = workbook.into_bytes().unwrap();
        assert_eq!(member(&numbers, "xl/styles.xml"), original, "{cell_xfs}");
        let sheet_xml = member(&numbers, "xl/worksheets/sheet1.xml");
        assert!(
            sheet_xml.contains("<c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2.5</v></c>"),
            "{sheet_xml}"
        );

        // A date needs one: the default goes in first, so the date's entry
        // is 1 and every cell stating no `s` still shows General.
        workbook
            .sheet_mut("Sheet1")
            .unwrap()
            .set_cell("C1".parse().unwrap(), Scalar::date32(19_723))
            .unwrap();
        let dated = workbook.into_bytes().unwrap();
        let styles_xml = member(&dated, "xl/styles.xml");
        assert!(
            styles_xml.contains(
                "<cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
                 <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>\
                 <cellStyles"
            ),
            "{cell_xfs}: {styles_xml}"
        );
        let sheet_xml = member(&dated, "xl/worksheets/sheet1.xml");
        assert!(
            sheet_xml.contains("<c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2.5</v></c><c r=\"C1\" s=\"1\"><v>45292</v></c>"),
            "{sheet_xml}"
        );
        assert_eq!(
            cell_of(&dated, "A1"),
            (NumberFormat::General, Scalar::from(1.0))
        );
        assert_eq!(
            cell_of(&dated, "B1"),
            (NumberFormat::General, Scalar::from(2.5))
        );
        assert_eq!(
            cell_of(&dated, "C1"),
            (NumberFormat::Date, Scalar::date32(19_723))
        );
    }
}

#[test]
fn a_record_write_into_a_styles_part_listing_no_cell_format_keeps_index_0_the_default() {
    let root = DataType::from(
        StructType::from_fields([
            DataType::Date32.nullable_field("day"),
            DataType::Float64.nullable_field("price"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let batch = Serie::from_scalars(
        root.clone(),
        [Scalar::from_sequence([
            Scalar::date32(19_723),
            Scalar::from(2.5),
        ])],
    )
    .unwrap()
    .into_arrow_batch()
    .unwrap();
    let mut handle =
        Buffer::from_bytes(styled("", &formatless(""))).with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap().with_field(root);
    handle.overwrite_arrow_batch(batch, &options).unwrap();
    let bytes = handle.read_all_bytes().unwrap();
    // The date's entry is 1; the number and the header state no `s` and
    // read as what they are.
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::General, Scalar::from("day"))
    );
    assert_eq!(
        cell_of(&bytes, "A2"),
        (NumberFormat::Date, Scalar::date32(19_723))
    );
    assert_eq!(
        cell_of(&bytes, "B2"),
        (NumberFormat::General, Scalar::from(2.5))
    );
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains("<c r=\"A2\" s=\"1\"><v>45292</v></c><c r=\"B2\"><v>2.5</v></c>"),
        "{sheet_xml}"
    );
}

#[test]
fn a_cell_format_showing_a_pivot_button_or_stating_an_apply_flag_false_is_never_reused() {
    // Entries 1 and 2 resolve to the default style under `yyyy-mm-dd`, but
    // one shows a pivot table's field button and the other takes its number
    // format from its named style: a date written under either would show
    // the button, or not show as a date.
    let original = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <styleSheet xmlns=\"{NS}\">\
         <numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/></numFmts>\
         <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
         <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
         <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
         <cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
         <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" pivotButton=\"1\"/>\
         <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"0\"/></cellXfs>\
         </styleSheet>"
    );
    let mut workbook = Workbook::from_bytes(styled("", &original)).unwrap();
    let styles = workbook.style_sheet().unwrap();
    assert_eq!(styles.style(StyleId::new(1)), styles.style(StyleId::new(2)));
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let styles_xml = member(&bytes, "xl/styles.xml");
    // The entry appended names the code the part declares, and the two it
    // passed over are carried as they were.
    assert!(
        styles_xml.contains(
            "pivotButton=\"1\"/><xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"0\"/>\
             <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs>"
        ),
        "{styles_xml}"
    );
    assert!(styles_xml.contains("<cellXfs count=\"4\">"), "{styles_xml}");
    let sheet_xml = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet_xml.contains("<c r=\"A1\" s=\"3\"><v>45292</v></c>"),
        "{sheet_xml}"
    );
}

#[test]
fn a_part_naming_the_highest_number_format_id_refuses_a_new_code() {
    let original = one_sheet("", &[], &[(u32::MAX, "0.0")], &[0]);
    let mut workbook = Workbook::from_bytes(original).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    assert_eq!(
        workbook.into_bytes().unwrap_err().to_string(),
        "invalid record value at $.styles: expected a number format id past every one the \
         styles name for the code \"yyyy-mm-dd\", got none left past 4294967295"
    );
}

#[test]
fn a_new_code_takes_an_id_past_every_one_a_cell_format_names_without_a_declaration() {
    // Entry 1 names 169, which the part never declares: its cell shows
    // General, and keeps showing it after a save adds a code.
    let original = one_sheet(
        "<row r=\"1\"><c r=\"B1\" s=\"1\"><v>5</v></c></row>",
        &[],
        &[],
        &[0, 169],
    );
    assert_eq!(
        cell_of(&original, "B1"),
        (NumberFormat::General, Scalar::from(5.0))
    );
    let mut workbook = Workbook::from_bytes(original).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let styles_xml = member(&bytes, "xl/styles.xml");
    assert!(
        styles_xml.contains(
            "<numFmts count=\"1\"><numFmt numFmtId=\"170\" formatCode=\"yyyy-mm-dd\"/></numFmts>"
        ),
        "{styles_xml}"
    );
    assert_eq!(
        cell_of(&bytes, "B1"),
        (NumberFormat::General, Scalar::from(5.0))
    );
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::Date, Scalar::date32(19_723))
    );
}

#[test]
fn a_prefixed_styles_part_gains_its_entries_under_its_own_prefix() {
    let original = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <x:styleSheet xmlns:x=\"{NS}\">\
         <x:fonts count=\"1\"><x:font><x:sz val=\"11\"/><x:name val=\"Calibri\"/></x:font></x:fonts>\
         <x:fills count=\"2\"><x:fill><x:patternFill patternType=\"none\"/></x:fill><x:fill><x:patternFill patternType=\"gray125\"/></x:fill></x:fills>\
         <x:borders count=\"1\"><x:border><x:left/><x:right/><x:top/><x:bottom/><x:diagonal/></x:border></x:borders>\
         <x:cellXfs count=\"1\"><x:xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></x:cellXfs>\
         </x:styleSheet>"
    );
    let mut workbook = Workbook::from_bytes(styled("", &original)).unwrap();
    workbook
        .sheet_mut("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), Scalar::date32(19_723))
        .unwrap();
    let bytes = workbook.into_bytes().unwrap();
    let styles_xml = member(&bytes, "xl/styles.xml");
    // The list the part lacked goes in at its place under the part's prefix,
    // and so does the entry appended to the one it has.
    assert!(
        styles_xml.contains(
            "<x:styleSheet xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <x:numFmts count=\"1\"><x:numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/></x:numFmts><x:fonts"
        ),
        "{styles_xml}"
    );
    assert!(
        styles_xml.contains(
            "<x:cellXfs count=\"2\"><x:xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
             <x:xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></x:cellXfs>"
        ),
        "{styles_xml}"
    );
    // Read in its namespace, every element of the part is SpreadsheetML.
    let document = yggdryl::xml::from_bytes(styles_xml.as_bytes()).unwrap();
    let root = yggdryl::xml::Element::root(&document).unwrap();
    assert!(root.is(Some(NS), "styleSheet"));
    for list in root.children() {
        assert!(list.is_in(NS, list.local_name()), "{}", list.name());
        for entry in list.children() {
            assert!(entry.is_in(NS, entry.local_name()), "{}", entry.name());
        }
    }
    assert_eq!(
        cell_of(&bytes, "A1"),
        (NumberFormat::Date, Scalar::date32(19_723))
    );
}

#[cfg(feature = "internals")]
mod internal {
    use std::sync::Arc;

    use yggdryl::excel::{
        Alignment, Border, BorderStyle, CellStyle, Color, Edge, Fill, Font, FontScheme, Horizontal,
        PatternType, Protection, StyleId, Underline, Vertical, VerticalRun,
    };
    use yggdryl::internals::excel_styles::{
        binding_info, from_xml, intern, original_xfs, rebind, rebind_all, to_part, written,
    };

    use super::rich_styles;
    use crate::excel_package::NS;

    fn binding_style(bold: bool, italic: bool, strike: bool) -> CellStyle {
        CellStyle {
            font: Font {
                bold,
                italic,
                strike,
                ..Font::default()
            },
            ..CellStyle::default()
        }
    }

    #[test]
    fn style_bindings_normalize_the_original_prefix_and_preserve_it_when_written() {
        for list in ["", "<cellXfs count=\"0\"/>"] {
            let xml = format!("<styleSheet xmlns=\"{NS}\">{list}</styleSheet>");
            let mut source = from_xml(xml.into_bytes()).unwrap();
            assert_eq!(original_xfs(&source), 1);
            assert_eq!(binding_info(&source, &[StyleId::DEFAULT]).0, 0);
            let id = intern(&mut source, &binding_style(true, false, false)).unwrap();
            assert_eq!(id, StyleId::new(1));
            let source = written(source).unwrap();
            assert_eq!(original_xfs(&source), 1);
            assert_eq!(binding_info(&source, &[id, id, StyleId::DEFAULT]).0, 1);
            // Opening those bytes separately starts a new immutable prefix.
            let reopened = from_xml(to_part(&source).unwrap()).unwrap();
            assert_eq!(original_xfs(&reopened), 2);
            assert_eq!(binding_info(&reopened, &[id]).0, 0);
        }
    }

    #[test]
    fn style_bindings_skip_original_opaque_xfs_without_reconstructing_them() {
        let xf = "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" pivotButton=\"1\" applyFont=\"0\" vendor=\"opaque\"/>";
        let xml = format!(
            "<styleSheet xmlns=\"{NS}\"><cellXfs count=\"2\"><xf/>{xf}</cellXfs></styleSheet>"
        );
        let original = from_xml(xml.into_bytes()).unwrap();
        let mut source = original.clone();
        let bold = intern(&mut source, &binding_style(true, false, false)).unwrap();
        let target = Arc::new(original);
        assert_eq!(binding_info(&source, &[StyleId::new(1)]).0, 0);
        let (unchanged, moved) = rebind(&source, &target, &[StyleId::new(1)]).unwrap();
        assert!(Arc::ptr_eq(&target, &unchanged));
        assert!(moved.is_empty());
        let (rebound, moved) = rebind_all(&source, &target).unwrap();
        assert!(moved.is_empty());
        assert_eq!(rebound.style(bold), source.style(bold));
        assert!(
            String::from_utf8(to_part(&rebound).unwrap())
                .unwrap()
                .contains(xf)
        );
        assert_eq!(original_xfs(&rebound), 2);
    }

    #[test]
    fn style_bindings_reuse_equivalent_entries_and_clone_only_for_an_append() {
        let mut source = yggdryl::excel::StyleSheet::default();
        let bold = binding_style(true, false, false);
        let italic = binding_style(false, true, false);
        let from = intern(&mut source, &bold).unwrap();
        let unchanged = Arc::new(source.clone());
        let (same, moved) = rebind_all(&source, &unchanged).unwrap();
        assert!(Arc::ptr_eq(&unchanged, &same));
        assert!(moved.is_empty());

        let mut reordered = yggdryl::excel::StyleSheet::default();
        intern(&mut reordered, &italic).unwrap();
        let existing = intern(&mut reordered, &bold).unwrap();
        let target = Arc::new(reordered);
        let before = to_part(&target).unwrap();
        let (same, moved) = rebind(&source, &target, &[from, from]).unwrap();
        assert!(Arc::ptr_eq(&target, &same));
        assert_eq!(moved, std::collections::HashMap::from([(from, existing)]));
        assert_eq!(same.len(), 3);
        assert_eq!(to_part(&same).unwrap(), before);

        let absent = Arc::new(yggdryl::excel::StyleSheet::default());
        let (grown, moved) = rebind_all(&source, &absent).unwrap();
        assert!(!Arc::ptr_eq(&absent, &grown));
        assert_eq!(absent.len(), 1);
        assert_eq!(grown.len(), 2);
        assert!(moved.is_empty());
        assert_eq!(grown.style(from), Some(&bold));
        assert_eq!(original_xfs(&grown), 1);
    }

    #[test]
    fn style_bindings_budget_distinct_descriptors_and_shared_gradient_bytes() {
        let mut source = yggdryl::excel::StyleSheet::default();
        let font = "N".repeat(512);
        let code = format!("0\"{}\"", "C".repeat(512));
        let gradient = format!("<gradientFill><!--{}--></gradientFill>", "g".repeat(8_192));
        let style = CellStyle {
            number_format: code.clone().into(),
            font: Font {
                name: font.clone().into(),
                ..Font::default()
            },
            fill: Fill::Gradient(Arc::from(gradient.as_bytes())),
            ..CellStyle::default()
        };
        let id = intern(&mut source, &style).unwrap();
        let empty = binding_info(&source, &[]);
        assert_eq!(
            binding_info(&source, &[StyleId::DEFAULT, StyleId::new(u16::MAX)]),
            empty
        );
        let one = binding_info(&source, &[id]);
        assert_eq!(one.0, 1);
        assert!(one.1 >= empty.1 + font.len() + code.len() + gradient.len());
        assert_eq!(binding_info(&source, &[id; 1_024]), one);
    }

    #[test]
    fn style_bindings_refuse_capacity_after_a_prospective_append_atomically() {
        let original_len = yggdryl::excel::MAX_CELL_FORMATS - 2;
        let xml = format!(
            "<styleSheet xmlns=\"{NS}\"><cellXfs count=\"{original_len}\">{}</cellXfs></styleSheet>",
            "<xf/>".repeat(original_len),
        );
        let original = from_xml(xml.into_bytes()).unwrap();
        let mut source = original.clone();
        let bold = intern(&mut source, &binding_style(true, false, false)).unwrap();
        let italic = intern(&mut source, &binding_style(false, true, false)).unwrap();
        let mut current = original;
        intern(&mut current, &binding_style(false, false, true)).unwrap();
        let target = Arc::new(current);
        let held = Arc::clone(&target);
        let before = to_part(&target).unwrap();
        let error = rebind(&source, &target, &[bold, italic]).unwrap_err();
        assert!(matches!(&error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.styles"));
        assert!(
            error
                .to_string()
                .contains("expected at most 64000 cell formats"),
            "{error}"
        );
        assert!(Arc::ptr_eq(&target, &held));
        assert_eq!(target.len(), yggdryl::excel::MAX_CELL_FORMATS - 1);
        assert_eq!(to_part(&target).unwrap(), before);
        assert_eq!(original_xfs(&target), original_len);
    }

    /// A style stating something of every fact a cell format points at.
    fn loud() -> CellStyle {
        CellStyle {
            number_format: "0.000%".into(),
            font: Font {
                name: "Arial".into(),
                size: 12.5,
                bold: true,
                italic: true,
                underline: Underline::Double,
                strike: true,
                color: Some(Color::Theme {
                    index: 4,
                    tint: 0.4,
                }),
                vertical: VerticalRun::Superscript,
                family: Some(2),
                scheme: Some(FontScheme::Minor),
                ..Font::default()
            },
            fill: Fill::Pattern {
                pattern: PatternType::Solid,
                foreground: Some(Color::Rgb(0xFF11_2233)),
                background: Some(Color::Indexed {
                    index: 64,
                    tint: 0.0,
                }),
            },
            border: Border {
                left: Edge {
                    style: BorderStyle::Thin,
                    color: Some(Color::Rgb(0xFF00_0000)),
                },
                right: Edge {
                    style: BorderStyle::Medium,
                    color: Some(Color::Theme {
                        index: 1,
                        tint: -0.25,
                    }),
                },
                top: Edge {
                    style: BorderStyle::Dashed,
                    color: Some(Color::Auto),
                },
                bottom: Edge {
                    style: BorderStyle::Double,
                    color: Some(Color::Indexed {
                        index: 8,
                        tint: 0.0,
                    }),
                },
                diagonal: Edge {
                    style: BorderStyle::Hair,
                    color: None,
                },
                vertical: Edge {
                    style: BorderStyle::Thick,
                    color: None,
                },
                horizontal: Edge {
                    style: BorderStyle::Dotted,
                    color: Some(Color::Rgb(0xFF12_3456)),
                },
                diagonal_up: true,
                diagonal_down: true,
                outline: false,
            },
            alignment: Alignment {
                horizontal: Horizontal::CenterContinuous,
                vertical: Vertical::Top,
                rotation: 45,
                wrap: true,
                indent: 2,
                relative_indent: -1,
                justify_last_line: true,
                shrink_to_fit: true,
                reading_order: 2,
            },
            protection: Protection {
                locked: false,
                hidden: true,
            },
            quote_prefix: true,
            parent: 0,
        }
    }

    /// A style under a built-in code, a gradient carried as it was read,
    /// and the font facts the other leaves out.
    fn gradient() -> CellStyle {
        CellStyle {
            number_format: "0.00".into(),
            font: Font {
                name: "Courier New".into(),
                size: 9.0,
                underline: Underline::Single,
                color: Some(Color::Indexed {
                    index: 10,
                    tint: -0.5,
                }),
                vertical: VerticalRun::Subscript,
                family: Some(3),
                charset: Some(0),
                outline: true,
                shadow: true,
                condense: true,
                extend: true,
                ..Font::default()
            },
            fill: Fill::Gradient(Arc::from(
                &b"<gradientFill degree=\"45\"><stop position=\"0\"><color rgb=\"FF0000FF\"/></stop>\
                   <stop position=\"1\"><color theme=\"5\" tint=\"-0.5\"/></stop></gradientFill>"[..],
            )),
            border: Border::default(),
            alignment: Alignment {
                horizontal: Horizontal::Right,
                indent: 1,
                ..Alignment::default()
            },
            protection: Protection::default(),
            quote_prefix: false,
            parent: 0,
        }
    }

    /// Intern both styles into the part `part`, write it, read it again:
    /// each style is what it was interned as, every entry the part held
    /// resolves as it did, and the part states each count in `counts`.
    fn round_trip(part: String, counts: &[&str]) {
        let original = from_xml(part.into_bytes()).unwrap();
        let held = original.len();
        let mut grown = original.clone();
        // A style the part holds is its entry, whichever list it reads from.
        let default = grown.style(StyleId::DEFAULT).unwrap().clone();
        assert_eq!(intern(&mut grown, &default).unwrap(), StyleId::DEFAULT);
        let styles = [loud(), gradient()];
        let ids: Vec<StyleId> = styles
            .iter()
            .map(|style| intern(&mut grown, style).unwrap())
            .collect();
        assert_eq!(
            ids.iter()
                .map(|id| usize::from(id.as_u16()))
                .collect::<Vec<_>>(),
            [held, held + 1]
        );
        // Interned again, a style is the entry it already has.
        assert_eq!(intern(&mut grown, &styles[0]).unwrap(), ids[0]);
        assert_eq!(grown.len(), held + 2);

        let written = String::from_utf8(to_part(&grown).unwrap()).unwrap();
        for count in counts {
            assert!(written.contains(count), "{count}: {written}");
        }
        let read = from_xml(written.into_bytes()).unwrap();
        assert_eq!(read.len(), held + 2);
        for at in 0..held {
            let id = StyleId::new(u16::try_from(at).unwrap());
            assert_eq!(read.style(id), original.style(id), "entry {at}");
        }
        for (id, style) in ids.iter().zip(&styles) {
            assert_eq!(read.style(*id), Some(style));
        }
    }

    #[test]
    fn every_fact_a_style_states_is_appended_once_and_reads_back_as_interned() {
        // A part stating every list: each is appended to, its count patched.
        // The new code takes the id past the 170 a differential format
        // states; the border the second style needs is the part's first.
        round_trip(
            rich_styles(),
            &[
                "<numFmts count=\"2\">",
                "<numFmt numFmtId=\"171\" formatCode=\"0.000%\"/></numFmts>",
                "<fonts count=\"4\">",
                "<fills count=\"6\">",
                "<borders count=\"3\">",
                "<cellXfs count=\"5\">",
            ],
        );
    }

    #[test]
    fn a_part_lacking_every_list_gains_each_at_its_place_after_the_default_entries() {
        // Each list goes in whole, in the schema's order, the entries index 0
        // names first: the default font, fill and border, the fill Excel
        // reserves, and the default cell format.
        round_trip(
            format!("<styleSheet xmlns=\"{NS}\"/>"),
            &[
                "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
                 <numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.000%\"/></numFmts>\
                 <fonts count=\"3\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font>",
                "<fills count=\"4\"><fill><patternFill patternType=\"none\"/></fill>\
                 <fill><patternFill patternType=\"gray125\"/></fill>",
                "<borders count=\"2\"><border><left/><right/><top/><bottom/><diagonal/></border>",
                "<cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>",
                "</cellXfs></styleSheet>",
            ],
        );
    }
}

#[test]
fn new_styles_keep_the_workbook_namespace_family() {
    use yggdryl::excel::{
        RELATIONSHIPS_NAMESPACE, STRICT_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE, StylePatch,
    };
    let types = content_types(1, false, false);
    let root =
        root_relationships().replace(RELATIONSHIPS_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE);
    let relationships = workbook_relationships(1, false, false)
        .replace(RELATIONSHIPS_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE);
    let document = workbook(&["Sheet1"], false)
        .replace(NS, STRICT_NAMESPACE)
        .replace(RELATIONSHIPS_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE);
    let sheet = worksheet("<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>")
        .replace(NS, STRICT_NAMESPACE)
        .replace(RELATIONSHIPS_NAMESPACE, STRICT_RELATIONSHIPS_NAMESPACE);
    let mut book = Workbook::from_bytes(package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &document),
        ("xl/_rels/workbook.xml.rels", &relationships),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]))
    .unwrap();
    let a1 = "A1".parse().unwrap();
    book.set_style(
        "Sheet1",
        &[a1],
        &StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    )
    .unwrap();
    let first = book.into_package().unwrap();
    book.rebase(first).unwrap();
    book.set_style(
        "Sheet1",
        &[a1],
        &StylePatch {
            italic: Some(true),
            ..StylePatch::default()
        },
    )
    .unwrap();
    let bytes = book.into_bytes().unwrap();
    let styles = member(&bytes, "xl/styles.xml");
    let xml = yggdryl::xml::from_bytes(styles.as_bytes()).unwrap();
    assert!(
        yggdryl::xml::Element::root(&xml)
            .unwrap()
            .is(Some(STRICT_NAMESPACE), "styleSheet"),
        "{styles}"
    );
    let relationships = member(&bytes, "xl/_rels/workbook.xml.rels");
    assert!(
        relationships.contains(&format!("Type=\"{STRICT_RELATIONSHIPS_NAMESPACE}/styles\"")),
        "{relationships}"
    );
    let reopened = Workbook::from_bytes(bytes).unwrap();
    let style = reopened
        .cell_style("Sheet1", "A1".parse().unwrap())
        .unwrap();
    assert!(style.font.bold && style.font.italic);
}


#[test]
fn pivot_number_format_xstring_uses_the_shared_spreadsheet_string_boundary() {
    use yggdryl::excel::{CellRange, StylePatch, Workbook};
    let code = "0\"<&\0_x0041_\"";
    let mut book = Workbook::new();
    book.add_sheet("Data").unwrap().set_cell("A1".parse().unwrap(), 2.0).unwrap();
    book.set_style("Data", &["A1:A1".parse::<CellRange>().unwrap()], &StylePatch {
        number_format: Some(code.into()), ..StylePatch::default()
    }).unwrap();
    let saved = Workbook::from_bytes(book.into_bytes().unwrap()).unwrap();
    let xml = crate::excel_package::member(&saved, "xl/styles.xml");
    assert!(!xml.contains('\0'), "{xml}");
    assert!(xml.contains("&lt;&amp;_x0000__x005F_x0041_"), "{xml}");
    assert_eq!(saved.cell_style("Data", "A1".parse().unwrap()).unwrap().number_format, code);
}
