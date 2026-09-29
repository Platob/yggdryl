//! `rust/src/excel/shared_strings.rs`: the shared string table read - runs joined, phonetics left out, `_xHHHH_` decoded, indexes proven - and written - each text interned once, `ST_Xstring` escapes, counts.

use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::{RecordBatch, StringArray};
use yggdryl::excel::{CellKind, CellRef, NAMESPACE, Workbook};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::IORecordOptions;
use yggdryl::zip::ZipArchive;
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

use crate::excel_package::{
    NS, content_types, one_sheet, package, root_relationships, workbook, workbook_relationships,
    worksheet,
};

/// The shared strings part holding `items`, each an `si` element as spelled.
fn table(items: &[&str]) -> String {
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <sst xmlns=\"{NS}\" count=\"{count}\" uniqueCount=\"{count}\">",
        count = items.len()
    );
    for item in items {
        text.push_str(item);
    }
    text.push_str("</sst>");
    text
}

/// A one-sheet package, `Sheet1` holding `sheet_data`, whose shared strings
/// part is `strings` exactly as spelled.
fn package_with(sheet_data: &str, strings: &str) -> Vec<u8> {
    let types = content_types(1, true, false);
    let root = root_relationships();
    let relationships = workbook_relationships(1, true, false);
    let book = workbook(&["Sheet1"], false);
    let sheet = worksheet(sheet_data);
    package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ("xl/sharedStrings.xml", strings),
    ])
}

/// Column A referencing the indexes `0..count` of the table, one per row.
fn every_index(count: usize) -> String {
    (0..count)
        .map(|index| {
            format!(
                "<row r=\"{row}\"><c r=\"A{row}\" t=\"s\"><v>{index}</v></c></row>",
                row = index + 1
            )
        })
        .collect()
}

/// What each `si` of `items` reads as, through the cell referencing it.
fn read_items(items: &[&str]) -> Vec<Scalar> {
    let bytes = package_with(&every_index(items.len()), &table(items));
    let book = Workbook::from_bytes(bytes).unwrap();
    let sheet = book.sheet("Sheet1").unwrap();
    (0..items.len())
        .map(|row| sheet.scalar(CellRef::new(u32::try_from(row).unwrap(), 0)))
        .collect()
}

/// Text values, as a cell holds them.
fn texts(values: &[&str]) -> Vec<Scalar> {
    values.iter().map(|value| Scalar::from(*value)).collect()
}

/// The member `name` of the package `bytes`, as text.
fn member(bytes: &[u8], name: &str) -> String {
    let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(name).unwrap()).unwrap()
}

/// The refusal parsing `Sheet1` of the package `bytes` answers.
fn sheet_refusal(bytes: Vec<u8>) -> String {
    let book = Workbook::from_bytes(bytes).unwrap();
    book.sheet("Sheet1").unwrap_err().to_string()
}

/// One `si` as the crate writes it.
fn written_item(text: &str) -> String {
    format!("<si><t xml:space=\"preserve\">{text}</t></si>")
}

/// The shared strings part the crate writes for `items` and `count` references.
fn written_table(count: usize, items: &[&str]) -> String {
    let mut text = format!(
        "<sst xmlns=\"{NAMESPACE}\" count=\"{count}\" uniqueCount=\"{}\">",
        items.len()
    );
    for item in items {
        text.push_str(&written_item(item));
    }
    text.push_str("</sst>");
    text
}

/// A workbook of one sheet holding `values` down column A, as bytes.
fn column_workbook(values: &[&str]) -> Vec<u8> {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Sheet1").unwrap();
    for (row, value) in values.iter().enumerate() {
        sheet
            .set_cell(CellRef::new(u32::try_from(row).unwrap(), 0), *value)
            .unwrap();
    }
    book.into_bytes().unwrap()
}

/// Column A of `Sheet1` in the package `bytes`, `count` rows down.
fn read_column(bytes: Vec<u8>, count: usize) -> Vec<Scalar> {
    let book = Workbook::from_bytes(bytes).unwrap();
    let sheet = book.sheet("Sheet1").unwrap();
    (0..count)
        .map(|row| sheet.scalar(CellRef::new(u32::try_from(row).unwrap(), 0)))
        .collect()
}

#[test]
fn a_rich_text_item_reads_as_its_runs_concatenated() {
    let read = read_items(&[
        "<si><r><t>a</t></r><r><rPr><b/><sz val=\"11\"/><rFont val=\"Calibri\"/></rPr><t>b</t></r></si>",
        "<si><r><rPr><i/></rPr><t xml:space=\"preserve\">Apple </t></r><r><t>Inc.</t></r></si>",
    ]);
    assert_eq!(read, texts(&["ab", "Apple Inc."]));
}

#[test]
fn phonetic_runs_and_phonetic_properties_are_left_out_of_an_item() {
    let read = read_items(&[
        "<si><t>東京</t><rPh sb=\"0\" eb=\"2\"><t>トウキョウ</t></rPh><phoneticPr fontId=\"1\" type=\"noConversion\"/></si>",
        "<si><r><t>大</t></r><r><t>阪</t></r><rPh sb=\"0\" eb=\"1\"><t>オオ</t></rPh><rPh sb=\"1\" eb=\"2\"><t>サカ</t></rPh><phoneticPr fontId=\"1\"></phoneticPr></si>",
        "<si><t>after</t></si>",
    ]);
    assert_eq!(read, texts(&["東京", "大阪", "after"]));
}

#[test]
fn whitespace_between_the_elements_of_an_item_is_not_its_text() {
    let read = read_items(&[
        "\n  <si>\n    <r>\n      <t>a</t>\n    </r>\n    <r>\n      <t>b</t>\n    </r>\n    <rPh sb=\"0\" eb=\"1\">\n      <t>x</t>\n    </rPh>\n  </si>\n",
        "  <si>\n    <t>c</t>\n  </si>\n",
    ]);
    assert_eq!(read, texts(&["ab", "c"]));
}

#[test]
fn a_preserved_text_keeps_its_leading_and_trailing_spaces() {
    let read = read_items(&[
        "<si><t xml:space=\"preserve\">  two spaces  </t></si>",
        "<si><t xml:space=\"preserve\"> </t></si>",
        "<si><r><t xml:space=\"preserve\">a </t></r><r><t xml:space=\"preserve\"> b</t></r></si>",
    ]);
    assert_eq!(read, texts(&["  two spaces  ", " ", "a  b"]));
}

#[test]
fn an_empty_item_reads_as_the_empty_text_and_keeps_its_index() {
    let read = read_items(&[
        "<si/>",
        "<si><t/></si>",
        "<si><t></t></si>",
        "<si><r><t/></r></si>",
        "<si><t>x</t></si>",
    ]);
    assert_eq!(read, texts(&["", "", "", "", "x"]));
}

#[test]
fn an_escape_in_an_item_decodes_to_the_character_it_spells() {
    let read = read_items(&[
        "<si><t>a_x000D_b</t></si>",
        "<si><t>_x0001_</t></si>",
        "<si><t>tab_x0009_</t></si>",
        "<si><t>_x004a__x004B_</t></si>",
        "<si><t>nul_x0000_</t></si>",
        "<si><r><t>run_x000D_</t></r><r><t>_x000A_end</t></r></si>",
    ]);
    assert_eq!(
        read,
        texts(&["a\rb", "\u{1}", "tab\t", "JK", "nul\u{0}", "run\r\nend"])
    );
}

#[test]
fn an_escaped_underscore_reads_back_as_a_literal_escape_run() {
    let read = read_items(&[
        "<si><t>_x005F_x0041_</t></si>",
        "<si><t>a_x005F_x000D_b</t></si>",
        "<si><t>_x005F_</t></si>",
    ]);
    assert_eq!(read, texts(&["_x0041_", "a_x000D_b", "_"]));
}

#[test]
fn a_run_that_is_not_an_escape_is_left_as_is() {
    let read = read_items(&[
        "<si><t>_x00ZZ_</t></si>",
        "<si><t>_x004</t></si>",
        "<si><t>_x0041</t></si>",
        "<si><t>_X0041_</t></si>",
        "<si><t>_x_</t></si>",
        "<si><t>_x00410_</t></si>",
        "<si><t>_x00é_</t></si>",
    ]);
    assert_eq!(
        read,
        texts(&[
            "_x00ZZ_", "_x004", "_x0041", "_X0041_", "_x_", "_x00410_", "_x00é_"
        ])
    );
}

#[test]
fn a_surrogate_pair_escape_joins_into_one_character_and_a_lone_half_is_a_replacement() {
    let read = read_items(&[
        "<si><t>_xD83D__xDE00_</t></si>",
        "<si><t>_xD83D_x</t></si>",
        "<si><t>_xDE00_</t></si>",
        "<si><t>_xD83D_</t></si>",
    ]);
    assert_eq!(read, texts(&["😀", "\u{FFFD}x", "\u{FFFD}", "\u{FFFD}"]));
}

#[test]
fn entities_character_references_and_cdata_are_an_items_text() {
    let read = read_items(&[
        "<si><t>&lt;&amp;&gt;&quot;&apos;</t></si>",
        "<si><t>&#65;&#x42;</t></si>",
        "<si><t>a&#13;b</t></si>",
        "<si><t><![CDATA[<b>&amp;</b>]]></t></si>",
    ]);
    assert_eq!(read, texts(&["<&>\"'", "AB", "a\rb", "<b>&amp;</b>"]));
}

#[test]
fn a_raw_carriage_return_in_the_part_reads_as_a_line_feed_which_is_why_the_escape_exists() {
    let read = read_items(&["<si><t>a\r\nb</t></si>", "<si><t>a\rb</t></si>"]);
    assert_eq!(read, texts(&["a\nb", "a\nb"]));
}

#[test]
fn a_shared_string_index_past_the_table_is_refused_naming_the_cell() {
    let bytes = package_with(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>1</v></c></row>\
         <row r=\"4\"><c r=\"C4\" t=\"s\"><v>2</v></c></row>",
        &table(&["<si><t>a</t></si>", "<si><t>b</t></si>"]),
    );
    assert_eq!(
        sheet_refusal(bytes),
        "invalid record value at Sheet1!C4: expected a shared string index below 2, got 2"
    );
}

#[test]
fn a_package_without_a_table_refuses_every_shared_string_index() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"B1\" t=\"s\"><v>0</v></c></row>",
        &[],
        &[],
        &[],
    );
    assert_eq!(
        sheet_refusal(bytes),
        "invalid record value at Sheet1!B1: expected a shared string index below 0, got 0"
    );
}

#[test]
fn a_shared_string_index_that_is_not_a_number_is_refused_naming_the_cell() {
    let strings = table(&["<si><t>a</t></si>"]);
    let word = package_with(
        "<row r=\"2\"><c r=\"A2\" t=\"s\"><v>x</v></c></row>",
        &strings,
    );
    assert_eq!(
        sheet_refusal(word),
        "invalid record value at Sheet1!A2: expected a shared string index, got \"x\""
    );
    let negative = package_with(
        "<row r=\"1\"><c r=\"D1\" t=\"s\"><v>-1</v></c></row>",
        &strings,
    );
    assert_eq!(
        sheet_refusal(negative),
        "invalid record value at Sheet1!D1: expected a shared string index, got \"-1\""
    );
}

#[test]
fn a_shared_string_index_with_surrounding_whitespace_resolves() {
    let bytes = package_with(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v> 1 </v></c></row>",
        &table(&["<si><t>a</t></si>", "<si><t>b</t></si>"]),
    );
    let book = Workbook::from_bytes(bytes).unwrap();
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("b"));
    assert_eq!(
        sheet.cell("A1".parse().unwrap()).unwrap().kind(),
        CellKind::SharedString
    );
}

#[test]
fn an_undefined_entity_in_the_table_is_a_codec_refusal_at_its_byte() {
    let strings = table(&["<si><t>a&nbsp;b</t></si>"]);
    let at = strings.find("&nbsp;").unwrap();
    let bytes = package_with(&every_index(1), &strings);
    assert_eq!(
        sheet_refusal(bytes),
        format!("invalid xlsx data at byte {at}: unknown entity reference `&nbsp;`")
    );
}

#[test]
fn an_ill_formed_table_is_refused_when_a_sheet_first_needs_it() {
    let strings = table(&["<si><t>a</si>"]);
    let at = strings.find("</si>").unwrap();
    let bytes = package_with(&every_index(1), &strings);
    let book = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(book.sheet_names(), vec!["Sheet1"]);
    let refusal = book.sheet("Sheet1").unwrap_err().to_string();
    assert!(
        refusal.starts_with(&format!("invalid xlsx data at byte {at}: ")),
        "{refusal}"
    );
}

#[test]
fn the_record_reader_resolves_header_and_values_through_the_table() {
    let bytes = package_with(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\" t=\"s\"><v>1</v></c></row>\
         <row r=\"3\"><c r=\"A3\" t=\"s\"><v>2</v></c></row>\
         <row r=\"4\"><c r=\"A4\" t=\"s\"><v>1</v></c></row>",
        &table(&[
            "<si><r><t>sym</t></r><r><t>bol</t></r><rPh sb=\"0\" eb=\"1\"><t>x</t></rPh></si>",
            "<si><t>a_x000D_b</t></si>",
            "<si/>",
        ]),
    );
    let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap();
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.field_len(), 1);
    assert_eq!(field.fields()[0].name(), "symbol");
    assert_eq!(*field.fields()[0].dtype(), DataType::utf8());
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    let column = batches[0].column(0).as_string::<i32>();
    let values: Vec<Option<&str>> = column.iter().collect();
    assert_eq!(values, vec![Some("a\rb"), Some(""), Some("a\rb")]);
}

#[test]
fn the_record_reader_refuses_an_index_past_the_table_naming_the_cell() {
    let bytes = package_with(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\" t=\"s\"><v>7</v></c></row>",
        &table(&["<si><t>symbol</t></si>", "<si><t>x</t></si>"]),
    );
    let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap();
    let refusal = match handle.read_arrow_reader(&options) {
        Err(error) => error.to_string(),
        Ok(reader) => reader
            .map(|batch| batch.map(|_| ()))
            .collect::<Result<Vec<()>, _>>()
            .unwrap_err()
            .to_string(),
    };
    assert!(
        refusal.contains(
            "invalid record value at Sheet1!A2: expected a shared string index below 2, got 7"
        ),
        "{refusal}"
    );
}

#[test]
fn the_workbook_writer_interns_each_distinct_text_once() {
    let mut book = Workbook::new();
    let sheet = book.add_sheet("Trades").unwrap();
    sheet.set_cell("A1".parse().unwrap(), "AAPL").unwrap();
    sheet.set_cell("B1".parse().unwrap(), "MSFT").unwrap();
    sheet.set_cell("A2".parse().unwrap(), "AAPL").unwrap();
    sheet.set_cell("B2".parse().unwrap(), 1.5).unwrap();
    let bytes = book.into_bytes().unwrap();
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(3, &["AAPL", "MSFT"])
    );
    let part = member(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains(
            "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"s\"><v>0</v></c><c r=\"B2\"><v>1.5</v></c></row>"
        ),
        "{part}"
    );
}

#[test]
fn one_table_serves_every_sheet_of_a_workbook() {
    let mut book = Workbook::new();
    book.add_sheet("One")
        .unwrap()
        .set_cell("A1".parse().unwrap(), "AAPL")
        .unwrap();
    book.add_sheet("Two")
        .unwrap()
        .set_cell("A1".parse().unwrap(), "AAPL")
        .unwrap();
    let bytes = book.into_bytes().unwrap();
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(2, &["AAPL"])
    );
    for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        let text = member(&bytes, part);
        assert!(
            text.contains("<c r=\"A1\" t=\"s\"><v>0</v></c>"),
            "{part}: {text}"
        );
    }
    let opened = Workbook::from_bytes(bytes).unwrap();
    assert_eq!(
        opened.sheet("Two").unwrap().scalar("A1".parse().unwrap()),
        Scalar::from("AAPL")
    );
}

#[test]
fn the_workbook_writer_escapes_what_an_item_cannot_carry() {
    let bytes = column_workbook(&[
        "a\rb",
        "\u{1}",
        "nul\u{0}",
        "\u{B}\u{C}\u{1F}",
        "\u{FFFE}\u{FFFF}",
        "_x0041_",
        "a_x0041_b_x0042_",
        "<&>",
        "  padded  ",
        "tab\tline\nfeed",
    ]);
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(
            10,
            &[
                "a_x000D_b",
                "_x0001_",
                "nul_x0000_",
                "_x000B__x000C__x001F_",
                "_xFFFE__xFFFF_",
                "_x005F_x0041_",
                "a_x005F_x0041_b_x005F_x0042_",
                "&lt;&amp;&gt;",
                "  padded  ",
                "tab\tline\nfeed",
            ]
        )
    );
}

#[test]
fn text_holding_no_escape_run_is_written_as_is() {
    let values = [
        "_x",
        "a_xyz_",
        "_x12_",
        "_x00410_",
        "_X0041_",
        "_x00ZZ_",
        "snake_case_x",
        "😀 東京",
    ];
    let bytes = column_workbook(&values);
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(values.len(), &values)
    );
    assert_eq!(read_column(bytes, values.len()), texts(&values));
}

#[test]
fn escaped_text_reads_back_as_itself_after_a_round_trip() {
    let values = [
        "a\rb",
        "\r\n",
        "\u{1}",
        "nul\u{0}",
        "\u{B}\u{C}\u{1F}",
        "_x0041_",
        "_x005F_",
        "_x0041_x0042_",
        "__x0041_",
        "_x\u{1}",
        "_x12\r",
        "<&>\"'",
        "  padded  ",
        "\ttab\n",
        "\u{FFFE}\u{FFFF}",
        "😀",
        "_xD83D_",
        "_xD83D__xDE00_",
    ];
    let bytes = column_workbook(&values);
    assert_eq!(read_column(bytes, values.len()), texts(&values));
}

#[test]
fn a_rewritten_package_keeps_the_index_of_every_string_it_held() {
    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\" t=\"s\"><v>1</v></c></row>",
        &["id", "AAPL", "x_x0001_"],
        &[],
        &[],
    );
    let mut book = Workbook::from_bytes(bytes).unwrap();
    let sheet = book.sheet_mut("Sheet1").unwrap();
    sheet.set_cell("B1".parse().unwrap(), "new").unwrap();
    sheet.set_cell("A3".parse().unwrap(), "AAPL").unwrap();
    let written = book.into_bytes().unwrap();
    assert_eq!(
        member(&written, "xl/sharedStrings.xml"),
        written_table(4, &["id", "AAPL", "x_x0001_", "new"])
    );
    let part = member(&written, "xl/worksheets/sheet1.xml");
    assert!(
        part.contains("<c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>3</v></c>"),
        "{part}"
    );
    assert!(part.contains("<c r=\"A3\" t=\"s\"><v>1</v></c>"), "{part}");
    let opened = Workbook::from_bytes(written).unwrap();
    let sheet = opened.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from("new"));
    assert_eq!(sheet.scalar("A3".parse().unwrap()), Scalar::from("AAPL"));
}

#[test]
fn a_workbook_holding_no_text_writes_an_empty_table() {
    let mut book = Workbook::new();
    book.add_sheet("Sheet1")
        .unwrap()
        .set_cell("A1".parse().unwrap(), 1.0)
        .unwrap();
    let bytes = book.into_bytes().unwrap();
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(0, &[])
    );
}

#[test]
fn the_record_writer_writes_inline_strings_and_leaves_the_table_empty() {
    let field = DataType::from(
        StructType::from_fields([DataType::utf8().nullable_field("sym_x0041_bol")]).unwrap(),
    )
    .required_field("row");
    let batch = RecordBatch::try_new(
        field.clone().into_arrow_schema().unwrap(),
        vec![Arc::new(StringArray::from(vec![
            Some("a\rb"),
            Some("  x  "),
            None,
            Some("_x0041_"),
            Some("AAPL"),
            Some("AAPL"),
        ]))],
    )
    .unwrap();
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap().with_field(field.clone());
    handle
        .overwrite_arrow_batch(batch.clone(), &options)
        .unwrap();

    let bytes = handle.read_all_bytes().unwrap();
    assert_eq!(
        member(&bytes, "xl/sharedStrings.xml"),
        written_table(0, &[])
    );
    let part = member(&bytes, "xl/worksheets/sheet1.xml");
    for cell in [
        "<c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">sym_x005F_x0041_bol</t></is></c>",
        "<c r=\"A2\" t=\"inlineStr\"><is><t xml:space=\"preserve\">a_x000D_b</t></is></c>",
        "<c r=\"A3\" t=\"inlineStr\"><is><t xml:space=\"preserve\">  x  </t></is></c>",
        "<c r=\"A5\" t=\"inlineStr\"><is><t xml:space=\"preserve\">_x005F_x0041_</t></is></c>",
        "<c r=\"A7\" t=\"inlineStr\"><is><t xml:space=\"preserve\">AAPL</t></is></c>",
    ] {
        assert!(part.contains(cell), "{cell} in {part}");
    }
    assert!(!part.contains("t=\"s\""), "{part}");

    let read = handle
        .read_arrow_field(&handle.record_options().unwrap())
        .unwrap();
    assert_eq!(read.fields()[0].name(), "sym_x0041_bol");
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches, vec![batch]);
}
