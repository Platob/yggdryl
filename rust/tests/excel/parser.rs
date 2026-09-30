//! `rust/src/excel/parser.rs`: the worksheet grammar - the running row and column cursors, every `t`, `<v>`, `<is>` and `<f>` spelling, what is skipped, and each refusal located by sheet, cell and byte.

use arrow_array::RecordBatch;
use yggdryl::excel::{CellKind, CellRef, STRICT_NAMESPACE, Workbook};
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, IOMedia, MimeType, Scalar, Serie, TimeUnit, Timezone};

use crate::excel_package::{
    NS, content_types, package, root_relationships, workbook, workbook_relationships, worksheet,
};

/// A one-sheet package whose worksheet part is `part` exactly as spelled.
fn package_with_part(part: &str) -> Vec<u8> {
    let types = content_types(1, false, false);
    let root = root_relationships();
    let relationships = workbook_relationships(1, false, false);
    let book = workbook(&["Sheet1"], false);
    package(&[
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root.as_str()),
        ("xl/workbook.xml", book.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships.as_str()),
        ("xl/worksheets/sheet1.xml", part),
    ])
}

/// The workbook of one sheet, `Sheet1`, whose worksheet part is `part`.
fn book_with_part(part: &str) -> Workbook {
    Workbook::from_bytes(package_with_part(part)).unwrap()
}

/// The workbook of one sheet whose `<sheetData>` holds `sheet_data`.
fn book(sheet_data: &str) -> Workbook {
    book_with_part(&worksheet(sheet_data))
}

/// What parsing the part `part` of `Sheet1` refuses with.
fn refusal(part: &str) -> String {
    book_with_part(part)
        .sheet("Sheet1")
        .unwrap_err()
        .to_string()
}

/// The byte just past the first `tag` in `part`, where the reader stands
/// once it has read that tag.
fn end_of(part: &str, tag: &str) -> usize {
    part.find(tag).expect("the tag is in the part") + tag.len()
}

fn at(reference: &str) -> CellRef {
    reference.parse().unwrap()
}

/// An `.xlsx` handle over `bytes`.
fn handle(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into())
}

/// The handle's record options, the header row as `header` says.
fn options(handle: &Buffer, header: bool) -> RecordOptions {
    let mut options = handle.record_options().unwrap();
    options.set_header(header).unwrap();
    options
}

/// Every record of `batches` under `field`, each rendered as JSON.
fn rows_json(field: &Field, batches: &[RecordBatch]) -> Vec<String> {
    let mut rows = Vec::new();
    for batch in batches {
        let serie = Serie::from_arrow_batch(Some(field), batch, Default::default()).unwrap();
        for index in 0..serie.len() {
            rows.push(serie.scalar(index).unwrap().into_json().unwrap());
        }
    }
    rows
}

#[test]
fn a_row_without_r_is_the_row_after_the_one_before_it() {
    let book = book(
        "<row><c r=\"A1\"><v>1</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>3</v></c></row>\
         <row><c><v>4</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    let indexes: Vec<u32> = sheet.rows().map(|row| row.index()).collect();
    assert_eq!(indexes, vec![0, 2, 3]);
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(1.0));
    assert_eq!(sheet.scalar(at("A2")), Scalar::Null);
    assert_eq!(sheet.scalar(at("A3")), Scalar::from(3.0));
    assert_eq!(sheet.scalar(at("A4")), Scalar::from(4.0));
}

#[test]
fn a_cell_without_r_is_the_column_after_the_cell_before_it() {
    let book = book(
        "<row r=\"1\"><c r=\"B1\"><v>2</v></c><c><v>3</v></c><c><v>4</v></c></row>\
         <row r=\"2\"><c><v>1</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell(at("A1")), None);
    assert_eq!(sheet.scalar(at("B1")), Scalar::from(2.0));
    assert_eq!(sheet.scalar(at("C1")), Scalar::from(3.0));
    assert_eq!(sheet.scalar(at("D1")), Scalar::from(4.0));
    // The column cursor starts over at A on every row.
    assert_eq!(sheet.scalar(at("A2")), Scalar::from(1.0));
    assert_eq!(sheet.cells().count(), 4);
}

#[test]
fn a_cell_naming_a_later_column_leaves_the_columns_it_skips_absent() {
    let book = book(
        "<row r=\"1\" spans=\"1:5\" ht=\"15\" customHeight=\"1\">\
         <c r=\"A1\"><v>1</v></c><c r=\"D1\"><v>4</v></c><c><v>5</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.row(0).unwrap().len(), 3);
    assert_eq!(sheet.cell(at("B1")), None);
    assert_eq!(sheet.cell(at("C1")), None);
    assert_eq!(sheet.scalar(at("D1")), Scalar::from(4.0));
    assert_eq!(sheet.scalar(at("E1")), Scalar::from(5.0));
}

#[test]
fn a_cell_stating_no_value_is_present_and_null() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" s=\"0\"/><c r=\"B1\"></c><c r=\"C1\" t=\"b\"/><c r=\"D1\"><v>1</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    for reference in ["A1", "B1"] {
        let cell = sheet.cell(at(reference)).expect("a stated cell is kept");
        assert!(cell.is_null(), "{reference}");
        assert_eq!(cell.kind(), CellKind::Number, "{reference}");
        assert_eq!(cell.value(), &Scalar::Null, "{reference}");
    }
    let boolean = sheet.cell(at("C1")).unwrap();
    assert_eq!(boolean.kind(), CellKind::Boolean);
    assert!(boolean.is_null());
    assert_eq!(sheet.scalar(at("D1")), Scalar::from(1.0));
}

#[test]
fn an_inline_string_reads_the_text_of_its_t() {
    let book = book("<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>hello</t></is></c></row>");
    let sheet = book.sheet("Sheet1").unwrap();
    let cell = sheet.cell(at("A1")).unwrap();
    assert_eq!(cell.kind(), CellKind::InlineString);
    assert_eq!(cell.value(), &Scalar::from("hello"));
    assert_eq!(cell.formula(), None);
}

#[test]
fn an_inline_string_joins_its_rich_runs_and_leaves_out_their_properties() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is>\
         <r><rPr><b/><sz val=\"11\"/><rFont val=\"Calibri\"/></rPr><t>bold</t></r>\
         <r><t xml:space=\"preserve\"> and plain</t></r>\
         </is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("bold and plain"));
}

#[test]
fn phonetic_runs_are_not_the_cells_text() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is>\
         <t>東京</t><rPh sb=\"0\" eb=\"2\"><t>トウキョウ</t></rPh><phoneticPr fontId=\"1\" type=\"noConversion\"/>\
         </is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("東京"));
}

#[test]
fn a_formula_string_cell_reads_its_cached_text_and_keeps_its_formula() {
    let book = book("<row r=\"1\"><c r=\"A1\" t=\"str\"><f>\"a\"&amp;\"b\"</f><v>ab</v></c></row>");
    let sheet = book.sheet("Sheet1").unwrap();
    let cell = sheet.cell(at("A1")).unwrap();
    assert_eq!(cell.kind(), CellKind::FormulaString);
    assert_eq!(cell.value(), &Scalar::from("ab"));
    assert_eq!(cell.formula(), Some("\"a\"&\"b\""));
}

#[test]
fn a_number_cell_keeps_its_formula_beside_its_cached_value() {
    let book =
        book("<row r=\"1\"><c r=\"A1\"><f>1+1</f><v>2</v></c><c r=\"B1\"><f>NOW()</f></c></row>");
    let sheet = book.sheet("Sheet1").unwrap();
    let cached = sheet.cell(at("A1")).unwrap();
    assert_eq!(cached.kind(), CellKind::Number);
    assert_eq!(cached.value(), &Scalar::from(2.0));
    assert_eq!(cached.formula(), Some("1+1"));
    // A formula with no cached value states no value.
    let uncached = sheet.cell(at("B1")).unwrap();
    assert!(uncached.is_null());
    assert_eq!(uncached.formula(), Some("NOW()"));
}

#[test]
fn a_boolean_cell_reads_one_as_true_and_zero_as_false() {
    let book =
        book("<row r=\"1\"><c r=\"A1\" t=\"b\"><v>1</v></c><c r=\"B1\" t=\"b\"><v>0</v></c></row>");
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cell(at("A1")).unwrap().kind(), CellKind::Boolean);
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(true));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from(false));
}

#[test]
fn a_d_cell_reads_its_iso_8601_text() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"d\"><v>2024-01-02</v></c>\
         <c r=\"B1\" t=\"d\"><v>2024-01-02T12:30:00</v></c>\
         <c r=\"C1\" t=\"d\"><v>12:30:00.123456</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    let day = sheet.cell(at("A1")).unwrap();
    assert_eq!(day.kind(), CellKind::Date);
    assert_eq!(day.value().dtype().unwrap(), DataType::Date32);
    assert_eq!(day.value().into_json().unwrap(), "\"2024-01-02\"");
    assert_eq!(
        sheet.scalar(at("B1")).into_json().unwrap(),
        "\"2024-01-02T12:30:00.000\""
    );
    assert_eq!(
        sheet.scalar(at("C1")),
        Scalar::time64(45_000_123_456, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()
    );
}

#[test]
fn an_error_cell_is_null_and_names_its_error() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"e\"><f>1/0</f><v>#DIV/0!</v></c><c r=\"B1\" t=\"e\"><v>#N/A</v></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    let divided = sheet.cell(at("A1")).unwrap();
    assert_eq!(divided.kind(), CellKind::Error);
    assert_eq!(divided.value(), &Scalar::Null);
    assert_eq!(divided.error(), Some("#DIV/0!"));
    assert_eq!(divided.formula(), Some("1/0"));
    assert_eq!(sheet.cell(at("B1")).unwrap().error(), Some("#N/A"));
    assert_eq!(sheet.scalar(at("B1")), Scalar::Null);
}

#[test]
fn a_self_closed_row_is_a_row_with_no_cell_that_moves_the_cursor() {
    let bytes = package_with_part(&worksheet(
        "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row><row r=\"2\"/><row><c r=\"A3\"><v>3</v></c></row>",
    ));
    // The row after the self-closed row 2 is row 3, which its cell agrees with.
    let book = Workbook::from_bytes(bytes.clone()).unwrap();
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.row(1), None);
    assert_eq!(sheet.scalar(at("A3")), Scalar::from(3.0));

    // A record read yields it, as a record stating nothing; the handle's
    // own options take row 1 as the header, so two records remain there.
    let handle = handle(bytes);
    assert_eq!(handle.row_size().unwrap(), 2);
    let options = options(&handle, false);
    let field = handle.read_arrow_field(&options).unwrap();
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options.clone().with_field(field.clone()))
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(
        rows_json(&field, &batches),
        vec!["[1.0]", "[null]", "[3.0]"]
    );
}

#[test]
fn an_empty_sheet_data_is_an_empty_sheet() {
    let part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"{NS}\"><dimension ref=\"A1\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>"
    );
    let book = book_with_part(&part);
    let sheet = book.sheet("Sheet1").unwrap();
    assert!(sheet.is_empty());
    assert_eq!(sheet.len(), 0);
    assert_eq!(sheet.dimension(), None);

    let handle = handle(package_with_part(&part));
    let options = options(&handle, true);
    assert_eq!(handle.read_arrow_field(&options).unwrap().field_len(), 0);
}

#[test]
fn a_worksheet_part_without_sheet_data_is_refused() {
    let part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"{NS}\"><dimension ref=\"A1\"/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>"
    );
    assert_eq!(
        refusal(&part),
        "invalid record value at Sheet1: expected a worksheet part holding sheetData, got a part without one"
    );
}

#[test]
fn a_row_going_backwards_is_refused_at_the_cursor_and_the_byte() {
    let part = worksheet(
        "<row r=\"3\"><c r=\"A3\"><v>1</v></c></row><row r=\"2\"><c r=\"A2\"><v>0</v></c></row>",
    );
    let byte = end_of(&part, "<row r=\"2\">");
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!A3: at byte {byte}: expected rows in ascending order, got row 2 after row 3"
        )
    );

    // A row stated twice is not ascending either.
    let part = worksheet("<row r=\"2\"/><row r=\"2\"/>");
    let byte = part.rfind("<row r=\"2\"/>").unwrap() + "<row r=\"2\"/>".len();
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!A2: at byte {byte}: expected rows in ascending order, got row 2 after row 2"
        )
    );
}

#[test]
fn a_row_number_outside_the_grid_is_refused() {
    for stated in ["0", "1048577", "two"] {
        let part = worksheet(&format!("<row r=\"{stated}\"><c><v>1</v></c></row>"));
        let byte = end_of(&part, &format!("<row r=\"{stated}\">"));
        assert_eq!(
            refusal(&part),
            format!(
                "invalid record value at Sheet1!A1: at byte {byte}: expected a row number from 1 to 1048576, got \"{stated}\""
            ),
            "{stated}"
        );
    }
}

#[test]
fn a_cell_of_another_row_than_its_row_is_refused() {
    let part = worksheet("<row r=\"1\"><c r=\"A2\"><v>1</v></c></row>");
    let byte = end_of(&part, "<c r=\"A2\">");
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!A2: at byte {byte}: expected a cell of row 1, got A2"
        )
    );
}

#[test]
fn a_cell_going_backwards_in_its_row_is_refused() {
    let part = worksheet("<row r=\"1\"><c r=\"C1\"><v>1</v></c><c r=\"B1\"><v>2</v></c></row>");
    let byte = end_of(&part, "<c r=\"B1\">");
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!B1: at byte {byte}: expected cells in ascending column order, got B1 after column C"
        )
    );
}

#[test]
fn a_running_column_past_the_last_column_of_the_grid_is_refused() {
    let part = worksheet("<row r=\"1\"><c r=\"XFD1\"><v>1</v></c><c><v>2</v></c></row>");
    let byte = part.rfind("<c>").unwrap() + "<c>".len();
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!XFD1: at byte {byte}: expected at most 16384 columns, got column 16385"
        )
    );
}

#[test]
fn a_cell_type_the_schema_does_not_list_is_refused() {
    let part = worksheet("<row r=\"1\"><c r=\"A1\" t=\"x\"><v>1</v></c></row>");
    let byte = end_of(&part, "<c r=\"A1\" t=\"x\">");
    assert_eq!(
        refusal(&part),
        format!(
            "invalid record value at Sheet1!A1: at byte {byte}: expected one of n, s, str, inlineStr, b, d, e for a cell's `t`, got \"x\""
        )
    );
}

#[test]
fn a_number_cell_whose_text_is_no_number_is_refused_by_its_cell() {
    let data = "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c></row>\
                <row r=\"2\"><c r=\"A2\"><v>3</v></c><c r=\"B2\"><v>abc</v></c></row>";
    let bytes = package_with_part(&worksheet(data));
    let book = Workbook::from_bytes(bytes.clone()).unwrap();
    let handle = handle(bytes);
    let refusals = [
        book.sheet("Sheet1").unwrap_err().to_string(),
        handle
            .read_arrow_field(&options(&handle, false))
            .unwrap_err()
            .to_string(),
    ];
    for refused in refusals {
        assert!(
            refused.starts_with("invalid record value at Sheet1!B2: "),
            "{refused}"
        );
        assert!(
            refused.ends_with("expected a number in a numeric cell, got \"abc\""),
            "{refused}"
        );
    }
}

#[test]
fn elements_beside_sheet_data_are_ignored() {
    let part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"{NS}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
         <sheetPr><tabColor rgb=\"FF00FF00\"/></sheetPr>\
         <dimension ref=\"A1:B1\"/>\
         <sheetViews><sheetView workbookViewId=\"0\"><selection activeCell=\"B1\" sqref=\"B1\"/></sheetView></sheetViews>\
         <sheetFormatPr defaultRowHeight=\"15\"/>\
         <cols><col min=\"1\" max=\"2\" width=\"12\" customWidth=\"1\"/></cols>\
         <sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\" t=\"inlineStr\"><is><t>kept</t></is></c></row></sheetData>\
         <mergeCells count=\"1\"><mergeCell ref=\"A1:B1\"/></mergeCells>\
         <pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>\
         <extLst><ext uri=\"{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}\"><v>9</v></ext></extLst>\
         </worksheet>"
    );
    let book = book_with_part(&part);
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.cells().count(), 2);
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(1.0));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("kept"));
}

#[test]
fn an_element_inside_the_data_that_the_reader_does_not_keep_is_skipped_with_its_text() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\"><extLst><ext uri=\"x\"><v>9</v></ext></extLst><v>1</v></c>\
         <c r=\"B1\" t=\"inlineStr\"><is><t>a</t><extLst><t>hidden</t></extLst></is></c>\
         <extLst><c r=\"C1\"><v>7</v></c></extLst></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(1.0));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("a"));
    assert_eq!(sheet.cell(at("C1")), None);
    assert_eq!(sheet.cells().count(), 2);
}

#[test]
fn comments_and_processing_instructions_are_not_text() {
    let book = book(
        "<!-- rows follow --><row r=\"1\"><!-- a row --><c r=\"A1\"><?mso-application progid=\"Excel.Sheet\"?><v>1<!-- split -->2</v></c>\
         <c r=\"B1\" t=\"inlineStr\"><is><t>ab<!-- c -->cd<?pi x?></t></is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from(12.0));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("abcd"));
}

#[test]
fn preserved_whitespace_is_kept() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">  two  spaces  </t></is></c>\
         <c r=\"B1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">\n\tline</t></is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("  two  spaces  "));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("\n\tline"));
}

#[test]
fn references_entities_and_cdata_are_the_cells_text() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>a &amp; b &#x41;&#66;&lt;<![CDATA[<c>&amp;]]></t></is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("a & b AB<<c>&amp;"));
}

#[test]
fn an_inline_strings_escapes_are_decoded() {
    let book = book(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>a_x000D_b</t></is></c>\
         <c r=\"B1\" t=\"inlineStr\"><is><t>_x005F_x0041_</t></is></c></row>",
    );
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("a\rb"));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from("_x0041_"));
}

#[test]
fn a_strict_namespace_part_with_prefixed_elements_is_parsed() {
    let part = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <x:worksheet xmlns:x=\"{STRICT_NAMESPACE}\"><x:sheetData>\
         <x:row r=\"1\"><x:c r=\"A1\" t=\"inlineStr\"><x:is><x:t>strict</x:t></x:is></x:c><x:c r=\"B1\"><x:f>1+1</x:f><x:v>2</x:v></x:c></x:row>\
         </x:sheetData></x:worksheet>"
    );
    let book = book_with_part(&part);
    let sheet = book.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar(at("A1")), Scalar::from("strict"));
    assert_eq!(sheet.scalar(at("B1")), Scalar::from(2.0));
    assert_eq!(sheet.cell(at("B1")).unwrap().formula(), Some("1+1"));
}

#[test]
fn three_thousand_rows_stream_as_three_batches_of_a_thousand() {
    let mut data = String::from(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>n</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>label</t></is></c></row>",
    );
    for index in 0..3000 {
        let row = index + 2;
        data.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\"><v>{index}</v></c><c t=\"inlineStr\"><is><t>r{index}</t></is></c></row>"
        ));
    }
    let handle = handle(package_with_part(&worksheet(&data)));
    let options = handle.record_options().unwrap().with_batch_row_size(1000);
    assert_eq!(handle.row_size().unwrap(), 3000);
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.field_len(), 2);
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    let sizes: Vec<usize> = batches.iter().map(RecordBatch::num_rows).collect();
    assert_eq!(sizes, vec![1000, 1000, 1000]);
    let rows = rows_json(&field, &batches);
    assert_eq!(rows.len(), 3000);
    assert_eq!(rows[0], "[0.0,\"r0\"]");
    assert_eq!(rows[1000], "[1000.0,\"r1000\"]");
    assert_eq!(rows[2999], "[2999.0,\"r2999\"]");
}
