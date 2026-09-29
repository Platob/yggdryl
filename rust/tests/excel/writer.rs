//! `rust/src/excel/writer.rs`: the streamed worksheet writer - the part a
//! record stream becomes, cell by cell, and the refusals that leave the handle
//! as it was.

use std::sync::Arc;

use arrow_array::{Array, Float64Array, RecordBatch, RecordBatchIterator};
use arrow_schema::{ArrowError, SchemaRef};
use yggdryl::arrow::BatchReader;
use yggdryl::excel::{
    CellKind, CellRef, DateSystem, Excel, ExcelOptions, NAMESPACE, NumberFormat,
    RELATIONSHIPS_NAMESPACE, Workbook,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::IORecordOptions;
use yggdryl::zip::ZipArchive;
use yggdryl::{
    DataType, Error, Field, IOBase, IOMedia, MimeType, Scalar, Serie, StructType, TimeUnit,
    Timezone,
};

use crate::excel_package::{
    content_types, package, root_relationships, styles, workbook, workbook_relationships, worksheet,
};

/// The worksheet part a one-sheet write produces.
const SHEET1: &str = "xl/worksheets/sheet1.xml";

/// A record root over `children`.
fn root(children: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(children).unwrap()).required_field("row")
}

/// An empty buffer under the `.xlsx` media type.
fn xlsx() -> Buffer {
    Buffer::new().with_media_type(MimeType::XLSX.into())
}

/// One Arrow batch of `rows` under `field`.
fn batch(field: &Field, rows: Vec<Scalar>) -> RecordBatch {
    Serie::from_scalars(field.clone(), rows)
        .unwrap()
        .into_arrow_batch()
        .unwrap()
}

/// A reader yielding `items` in order under `schema`.
fn stream(schema: SchemaRef, items: Vec<Result<RecordBatch, ArrowError>>) -> BatchReader {
    Box::new(RecordBatchIterator::new(items, schema))
}

/// The text of the member `part` of the package `bytes`.
fn member(bytes: &[u8], part: &str) -> String {
    let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
        bytes.to_vec(),
    ))));
    String::from_utf8(archive.read_member(part).unwrap()).unwrap()
}

/// The first worksheet part a handle holds.
fn sheet_part(handle: &impl IOBase) -> String {
    member(&handle.read_all_bytes().unwrap(), SHEET1)
}

/// The `<sheetData>` element of a worksheet part.
fn sheet_data(xml: &str) -> &str {
    let start = xml.find("<sheetData>").expect("the part holds sheetData");
    let end = xml.find("</sheetData>").expect("sheetData closes") + "</sheetData>".len();
    &xml[start..end]
}

/// The `<row>` element numbered `index` (one-based) of a worksheet part.
fn row(xml: &str, index: u32) -> &str {
    let open = format!("<row r=\"{index}\">");
    let start = xml
        .find(&open)
        .unwrap_or_else(|| panic!("row {index} is written: {xml}"));
    let end = start + xml[start..].find("</row>").expect("the row closes") + "</row>".len();
    &xml[start..end]
}

/// An inline string cell as the writer spells one, `text` already escaped.
fn text(reference: &str, text: &str) -> String {
    format!(
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>"
    )
}

/// A numeric cell with no style.
fn number(reference: &str, value: &str) -> String {
    format!("<c r=\"{reference}\"><v>{value}</v></c>")
}

/// Write one batch of `rows` under `field` into an empty handle through the
/// record surface, and answer the handle.
fn written(field: &Field, rows: Vec<Scalar>) -> Buffer {
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_batch(batch(field, rows), &options)
        .unwrap();
    handle
}

/// A foreign two-sheet package, `Keep` and `Data`, under the 1904 date
/// system, whose styles part states three cell formats.
fn foreign() -> Vec<u8> {
    package(&[
        ("[Content_Types].xml", &content_types(2, false, true)),
        ("_rels/.rels", &root_relationships()),
        ("xl/workbook.xml", &workbook(&["Keep", "Data"], true)),
        (
            "xl/_rels/workbook.xml.rels",
            &workbook_relationships(2, false, true),
        ),
        (
            "xl/worksheets/sheet1.xml",
            &worksheet("<row r=\"1\"><c r=\"A1\"><v>42</v></c></row>"),
        ),
        (
            "xl/worksheets/sheet2.xml",
            &worksheet("<row r=\"1\"><c r=\"A1\"><v>7</v></c></row>"),
        ),
        ("xl/styles.xml", &styles(&[], &[0, 14, 2])),
    ])
}

/// The two-column root most tests write: a required id and a nullable text.
fn trades() -> Field {
    root([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("s"),
    ])
}

/// One `trades` record.
fn trade(id: i64, s: Option<&str>) -> Scalar {
    Scalar::from_sequence([Scalar::from(id), s.map_or(Scalar::Null, Scalar::from)])
}

#[test]
fn a_write_is_the_worksheet_element_around_the_header_and_the_rows_with_no_dimension() {
    let handle = written(&trades(), vec![trade(1, Some("AAPL")), trade(2, None)]);
    let expected = format!(
        "<worksheet xmlns=\"{NAMESPACE}\" xmlns:r=\"{RELATIONSHIPS_NAMESPACE}\"><sheetData>\
         <row r=\"1\">{}{}</row>\
         <row r=\"2\">{}{}</row>\
         <row r=\"3\">{}</row>\
         </sheetData></worksheet>",
        text("A1", "id"),
        text("B1", "s"),
        number("A2", "1"),
        text("B2", "AAPL"),
        number("A3", "2"),
    );
    assert_eq!(sheet_part(&handle), expected);
}

#[test]
fn the_header_row_names_each_column_as_an_inline_string_escaped_like_any_text() {
    let field = root([
        DataType::Int64.nullable_field("a<b"),
        DataType::Int64.nullable_field("_x0041_"),
        DataType::Int64.nullable_field("tab\there"),
    ]);
    let handle = written(&field, vec![]);
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 1),
        format!(
            "<row r=\"1\">{}{}{}</row>",
            text("A1", "a&lt;b"),
            text("B1", "_x005F_x0041_"),
            text("C1", "tab\there")
        )
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("a<b"));
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from("_x0041_"));
    assert_eq!(
        sheet.scalar("C1".parse().unwrap()),
        Scalar::from("tab\there")
    );
}

#[test]
fn an_empty_reader_writes_the_header_row_alone() {
    let field = trades();
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_reader(
            stream(field.clone().into_arrow_schema().unwrap(), vec![]),
            &options,
        )
        .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"1\">{}{}</row></sheetData>",
            text("A1", "id"),
            text("B1", "s")
        )
    );
}

#[test]
fn integers_are_written_as_plain_digits_with_no_type() {
    let field = root([
        DataType::Int64.nullable_field("i"),
        DataType::UInt64.nullable_field("u"),
    ]);
    let handle = written(
        &field,
        vec![
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(u64::MAX)]),
            Scalar::from_sequence([Scalar::from(-3_i64), Scalar::from(0_u64)]),
        ],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        format!(
            "<row r=\"2\">{}{}</row>",
            number("A2", "1"),
            number("B2", "18446744073709551615")
        )
    );
    assert_eq!(
        row(&xml, 3),
        format!(
            "<row r=\"3\">{}{}</row>",
            number("A3", "-3"),
            number("B3", "0")
        )
    );
}

#[test]
fn floats_are_written_by_their_shortest_round_trip() {
    let field = root([
        DataType::Float64.required_field("a"),
        DataType::Float64.required_field("b"),
        DataType::Float64.required_field("c"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::from(0.1),
            Scalar::from(-1.0e-3),
            Scalar::from(1.0e300),
        ])],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        format!(
            "<row r=\"2\">{}{}{}</row>",
            number("A2", "0.1"),
            number("B2", "-0.001"),
            number("C2", "1e300")
        )
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A2".parse().unwrap()), Scalar::from(0.1));
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from(-1.0e-3));
    assert_eq!(sheet.scalar("C2".parse().unwrap()), Scalar::from(1.0e300));
}

#[test]
fn a_float_that_is_not_a_number_is_written_as_the_num_error() {
    let field = root([
        DataType::Float64.required_field("nan"),
        DataType::Float64.required_field("inf"),
        DataType::Float32.required_field("neg"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::from(f64::NAN),
            Scalar::from(f64::INFINITY),
            Scalar::from(f32::NEG_INFINITY),
        ])],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        "<row r=\"2\"><c r=\"A2\" t=\"e\"><v>#NUM!</v></c><c r=\"B2\" t=\"e\"><v>#NUM!</v></c>\
         <c r=\"C2\" t=\"e\"><v>#NUM!</v></c></row>"
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let cell = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell("C2".parse().unwrap())
        .unwrap();
    assert_eq!(cell.kind(), CellKind::Error);
    assert_eq!(cell.error(), Some("#NUM!"));
    assert!(cell.is_null());
}

#[test]
fn a_boolean_is_written_as_t_b_one_or_zero() {
    let field = root([DataType::Boolean.nullable_field("live")]);
    let handle = written(
        &field,
        vec![
            Scalar::from_sequence([Scalar::from(true)]),
            Scalar::from_sequence([Scalar::from(false)]),
        ],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        "<row r=\"2\"><c r=\"A2\" t=\"b\"><v>1</v></c></row>"
    );
    assert_eq!(
        row(&xml, 3),
        "<row r=\"3\"><c r=\"A3\" t=\"b\"><v>0</v></c></row>"
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A2".parse().unwrap()), Scalar::from(true));
    assert_eq!(sheet.scalar("A3".parse().unwrap()), Scalar::from(false));
}

#[test]
fn text_is_an_inline_string_with_entities_and_x_escapes_that_read_back_as_itself() {
    let values = [
        "a\u{1}b\rc_x0041_",
        "<&>",
        "line\nbreak\ttab",
        "_x0041\u{1}",
    ];
    let field = root([DataType::utf8().required_field("s")]);
    let handle = written(
        &field,
        values
            .iter()
            .map(|value| Scalar::from_sequence([Scalar::from(*value)]))
            .collect(),
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        format!(
            "<row r=\"2\">{}</row>",
            text("A2", "a_x0001_b_x000D_c_x005F_x0041_")
        )
    );
    assert_eq!(
        row(&xml, 3),
        format!("<row r=\"3\">{}</row>", text("A3", "&lt;&amp;&gt;"))
    );
    assert_eq!(
        row(&xml, 4),
        format!("<row r=\"4\">{}</row>", text("A4", "line\nbreak\ttab"))
    );
    assert_eq!(
        row(&xml, 5),
        format!("<row r=\"5\">{}</row>", text("A5", "_x005F_x0041_x0001_"))
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    for (offset, value) in values.iter().enumerate() {
        let cell = sheet.cell(CellRef::new(offset as u32 + 1, 0)).unwrap();
        assert_eq!(cell.kind(), CellKind::InlineString);
        assert_eq!(cell.value(), &Scalar::from(*value));
    }
}

#[test]
fn a_null_cell_is_omitted_while_an_empty_text_is_written() {
    let field = root([
        DataType::Int64.nullable_field("i"),
        DataType::utf8().nullable_field("s"),
    ]);
    let handle = written(
        &field,
        vec![
            Scalar::from_sequence([Scalar::Null, Scalar::from("")]),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
            Scalar::from_sequence([Scalar::from(4_i64), Scalar::Null]),
        ],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        format!("<row r=\"2\">{}</row>", text("B2", ""))
    );
    // A record of nulls still takes its row, so the numbering stays the
    // stream's.
    assert_eq!(row(&xml, 3), "<row r=\"3\"></row>");
    assert_eq!(
        row(&xml, 4),
        format!("<row r=\"4\">{}</row>", number("A4", "4"))
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert!(sheet.cell("A2".parse().unwrap()).is_none());
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from(""));
}

#[test]
fn a_fixed_width_string_is_written_without_its_nul_padding() {
    let field = root([DataType::fixed_utf8(4).unwrap().required_field("code")]);
    let handle = written(&field, vec![Scalar::from_sequence([Scalar::from("ab")])]);
    assert_eq!(
        row(&sheet_part(&handle), 2),
        format!("<row r=\"2\">{}</row>", text("A2", "ab"))
    );
}

#[test]
fn a_naive_temporal_is_its_serial_under_the_style_of_its_format() {
    let field = root([
        DataType::Date32.required_field("d"),
        DataType::datetime64(TimeUnit::Second, Timezone::NAIVE)
            .unwrap()
            .required_field("dts"),
        DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)
            .unwrap()
            .required_field("dtms"),
        DataType::time64(TimeUnit::Microsecond)
            .unwrap()
            .required_field("t"),
        DataType::duration64(TimeUnit::Millisecond)
            .unwrap()
            .required_field("du"),
        DataType::Date64.required_field("d64"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::date32(19_723),
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1_700_000_000_123, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::time64(43_200_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::duration64(90_000_000, TimeUnit::Millisecond).unwrap(),
            Scalar::date64(86_400_000),
        ])],
    );
    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 2),
        "<row r=\"2\"><c r=\"A2\" s=\"1\"><v>45292</v></c><c r=\"B2\" s=\"2\"><v>45244.92592592593</v></c>\
         <c r=\"C2\" s=\"3\"><v>45244.92592734953</v></c><c r=\"D2\" s=\"4\"><v>0.5</v></c>\
         <c r=\"E2\" s=\"5\"><v>1.0416666666666667</v></c><c r=\"F2\" s=\"1\"><v>25570</v></c></row>"
    );
    assert_eq!(NumberFormat::Date.style_index(), 1);
    assert_eq!(NumberFormat::DateTimeFraction.style_index(), 3);

    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    let json = |reference: &str| {
        sheet
            .scalar(reference.parse().unwrap())
            .into_json()
            .unwrap()
    };
    assert_eq!(json("A2"), "\"2024-01-01\"");
    assert_eq!(json("B2"), "\"2023-11-14T22:13:20.000\"");
    assert_eq!(json("C2"), "\"2023-11-14T22:13:20.123\"");
    assert_eq!(json("D2"), "\"12:00:00.000\"");
    assert_eq!(json("F2"), "\"1970-01-02\"");
    assert_eq!(
        sheet.scalar("E2".parse().unwrap()),
        Scalar::duration64(90_000_000, TimeUnit::Millisecond).unwrap()
    );
}

#[test]
fn a_zoned_datetime_is_written_as_its_text_with_its_offset() {
    let paris = Timezone::from_str("Europe/Paris").unwrap();
    let field = root([
        DataType::datetime64(TimeUnit::Millisecond, Timezone::UTC)
            .unwrap()
            .required_field("utc"),
        DataType::datetime64(TimeUnit::Second, paris)
            .unwrap()
            .required_field("paris"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::datetime64(1_700_000_000_123, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
            Scalar::datetime64(0, TimeUnit::Second, paris).unwrap(),
        ])],
    );
    assert_eq!(
        row(&sheet_part(&handle), 2),
        format!(
            "<row r=\"2\">{}{}</row>",
            text("A2", "2023-11-14T22:13:20.123Z"),
            text("B2", "1970-01-01T01:00:00+01:00[Europe/Paris]")
        )
    );
}

#[test]
fn a_nested_value_is_written_as_its_json_text() {
    let field = root([
        "struct<a: int64, b: utf8>"
            .parse::<DataType>()
            .unwrap()
            .required_field("st"),
        "serie<int64>"
            .parse::<DataType>()
            .unwrap()
            .required_field("li"),
        "map<utf8, int64>"
            .parse::<DataType>()
            .unwrap()
            .required_field("m"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("x")]),
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
            Scalar::from_mapping([(Scalar::from("k"), Scalar::from(3_i64))]).unwrap(),
        ])],
    );
    assert_eq!(
        row(&sheet_part(&handle), 2),
        format!(
            "<row r=\"2\">{}{}{}</row>",
            text("A2", "[7,\"x\"]"),
            text("B2", "[1,2]"),
            text("C2", "{\"k\":3}")
        )
    );
}

#[test]
fn a_decimal_is_its_digits_and_other_leaves_are_their_text() {
    let field = root([
        DataType::decimal128(10, 2).unwrap().required_field("price"),
        DataType::Uuid.required_field("id"),
        DataType::binary().required_field("raw"),
    ]);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::from("12.5"),
            Scalar::from("123e4567-e89b-12d3-a456-426614174000"),
            Scalar::from(&b"\x00\x01hi"[..]),
        ])],
    );
    assert_eq!(
        row(&sheet_part(&handle), 2),
        format!(
            "<row r=\"2\">{}{}{}</row>",
            number("A2", "12.50"),
            text("B2", "123e4567-e89b-12d3-a456-426614174000"),
            text("C2", "AAFoaQ==")
        )
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    assert_eq!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .scalar("A2".parse().unwrap()),
        Scalar::from(12.5)
    );
}

#[test]
fn batches_stream_into_one_sheet_with_continuous_row_numbers() {
    let field = trades();
    let schema = field.clone().into_arrow_schema().unwrap();
    let first = batch(&field, vec![trade(1, Some("a"))]);
    let second = batch(&field, vec![trade(2, Some("b")), trade(3, None)]);
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_reader(stream(schema, vec![Ok(first), Ok(second)]), &options)
        .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"1\">{}{}</row><row r=\"2\">{}{}</row><row r=\"3\">{}{}</row>\
             <row r=\"4\">{}</row></sheetData>",
            text("A1", "id"),
            text("B1", "s"),
            number("A2", "1"),
            text("B2", "a"),
            number("A3", "2"),
            text("B3", "b"),
            number("A4", "3"),
        )
    );
}

#[test]
fn without_a_header_the_first_record_takes_the_anchor_cell() {
    let field = trades();
    let schema = field.clone().into_arrow_schema().unwrap();
    let first = batch(&field, vec![trade(1, Some("a"))]);
    let second = batch(&field, vec![trade(2, Some("b")), trade(3, Some("c"))]);
    let mut handle = xlsx();
    let options = ExcelOptions::new()
        .with_header(false)
        .with_range("C3:D9".parse().unwrap());
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(schema, vec![Ok(first), Ok(second)]),
        &options,
    )
    .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"3\">{}{}</row><row r=\"4\">{}{}</row><row r=\"5\">{}{}</row></sheetData>",
            number("C3", "1"),
            text("D3", "a"),
            number("C4", "2"),
            text("D4", "b"),
            number("C5", "3"),
            text("D5", "c"),
        )
    );
}

#[test]
fn with_a_header_the_names_take_the_anchor_row_and_the_records_follow_it() {
    let field = trades();
    let mut handle = xlsx();
    let options = ExcelOptions::new().with_range("B2".parse().unwrap());
    let rows = batch(&field, vec![trade(1, Some("a"))]);
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &options,
    )
    .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"2\">{}{}</row><row r=\"3\">{}{}</row></sheetData>",
            text("B2", "id"),
            text("C2", "s"),
            number("B3", "1"),
            text("C3", "a"),
        )
    );
}

#[test]
fn columns_past_the_last_grid_column_are_refused_before_anything_is_written() {
    let field = trades();
    let rows = batch(&field, vec![trade(1, Some("a"))]);
    let mut handle = xlsx();
    let options = ExcelOptions::new().with_range("XFD1".parse().unwrap());
    let error = yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &options,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $: expected at most 1 columns from XFD1, got 2"
    );
    assert_eq!(handle.size(), 0);

    // One column fits in the last one.
    let single = root([DataType::Int64.required_field("id")]);
    let rows = batch(&single, vec![Scalar::from_sequence([Scalar::from(9_i64)])]);
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &options,
    )
    .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"1\">{}</row><row r=\"2\">{}</row></sheetData>",
            text("XFD1", "id"),
            number("XFD2", "9")
        )
    );
}

#[test]
fn the_last_grid_row_is_written_and_a_row_past_it_is_refused_naming_the_cell() {
    let field = trades();
    let last = ExcelOptions::new().with_range("A1048576".parse().unwrap());

    let mut handle = xlsx();
    let rows = batch(&field, vec![trade(1, Some("a"))]);
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &last.clone().with_header(false),
    )
    .unwrap();
    assert_eq!(
        sheet_data(&sheet_part(&handle)),
        format!(
            "<sheetData><row r=\"1048576\">{}{}</row></sheetData>",
            number("A1048576", "1"),
            text("B1048576", "a")
        )
    );

    // The header takes the last row, so the record has none left.
    let mut handle = xlsx();
    let rows = batch(&field, vec![trade(1, Some("a"))]);
    let error = yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &last,
    )
    .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error:?}");
    assert_eq!(
        error.to_string(),
        "invalid record value at Sheet1!A1048577: expected at most 1048576 rows in a worksheet, got a row \
         1048577 past them"
    );
    assert_eq!(handle.size(), 0);
}

#[test]
fn text_at_the_cell_limit_is_written_and_one_character_more_is_refused_naming_the_cell() {
    // The limit counts characters, not bytes: every `é` is two bytes.
    let field = root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("s"),
    ]);
    let longest = "é".repeat(yggdryl::excel::MAX_CELL_TEXT);
    let handle = written(
        &field,
        vec![Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(longest.as_str()),
        ])],
    );
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let cell = workbook
        .sheet("Sheet1")
        .unwrap()
        .cell("B2".parse().unwrap())
        .unwrap();
    assert_eq!(cell.text().chars().count(), 32_767);
    assert_eq!(cell.value(), &Scalar::from(longest.as_str()));

    let longer = "é".repeat(yggdryl::excel::MAX_CELL_TEXT + 1);
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    let error = handle
        .overwrite_arrow_batch(
            batch(
                &field,
                vec![Scalar::from_sequence([
                    Scalar::from(1_i64),
                    Scalar::from(longer.as_str()),
                ])],
            ),
            &options,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at Sheet1!B2: expected at most 32767 characters in a cell, got 32768"
    );
    assert_eq!(handle.size(), 0);
}

#[test]
fn a_date_the_date_system_does_not_spell_is_refused_naming_the_cell() {
    let field = root([DataType::Date32.required_field("d")]);
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    let error = handle
        .overwrite_arrow_batch(
            batch(
                &field,
                vec![Scalar::from_sequence([Scalar::date32(-25_600)])],
            ),
            &options,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at Sheet1!A2: expected an instant the 1900 date system spells, from its \
         first day to 9999-12-31, got 1899-11-29T00:00:00.000"
    );
    assert_eq!(handle.size(), 0);
}

#[test]
fn a_write_into_a_named_sheet_replaces_that_sheet_alone() {
    let before = foreign();
    let field = root([DataType::Date32.required_field("d")]);
    let rows = batch(
        &field,
        vec![Scalar::from_sequence([Scalar::date32(19_723)])],
    );
    let mut handle = Buffer::from_bytes(before.clone()).with_media_type(MimeType::XLSX.into());
    // Sheet names compare without case: `data` addresses `Data`.
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &ExcelOptions::new().with_sheet("data"),
    )
    .unwrap();
    let after = handle.read_all_bytes().unwrap();
    assert_eq!(member(&after, SHEET1), member(&before, SHEET1));
    let workbook = Workbook::from_bytes(after).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Keep", "Data"]);
    assert_eq!(
        workbook
            .sheet("Keep")
            .unwrap()
            .scalar("A1".parse().unwrap()),
        Scalar::from(42.0)
    );
    let data = workbook.sheet("Data").unwrap();
    assert_eq!(data.scalar("A1".parse().unwrap()), Scalar::from("d"));
    assert_eq!(data.len(), 2);
}

#[test]
fn a_joined_workbook_takes_its_serials_from_its_date_system_and_its_styles_after_its_own() {
    // The foreign package counts in the 1904 system and states three cell
    // formats, so the crate's date style is the fourth after its own.
    let field = root([DataType::Date32.required_field("d")]);
    let rows = batch(
        &field,
        vec![Scalar::from_sequence([Scalar::date32(19_723)])],
    );
    let mut handle = Buffer::from_bytes(foreign()).with_media_type(MimeType::XLSX.into());
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &ExcelOptions::new().with_sheet("Data"),
    )
    .unwrap();
    let after = handle.read_all_bytes().unwrap();
    assert_eq!(
        sheet_data(&member(&after, "xl/worksheets/sheet2.xml")),
        format!(
            "<sheetData><row r=\"1\">{}</row><row r=\"2\"><c r=\"A2\" s=\"4\"><v>43830</v></c></row></sheetData>",
            text("A1", "d")
        )
    );
    let workbook = Workbook::from_bytes(after).unwrap();
    assert_eq!(workbook.date_system(), DateSystem::Year1904);
    let cell = workbook
        .sheet("Data")
        .unwrap()
        .cell("A2".parse().unwrap())
        .unwrap();
    assert_eq!(cell.format(), NumberFormat::Date);
    assert_eq!(cell.value(), &Scalar::date32(19_723));
}

#[test]
fn a_sheet_the_workbook_lacks_is_added_after_its_sheets() {
    let before = foreign();
    let field = trades();
    let rows = batch(&field, vec![trade(1, Some("a"))]);
    let mut handle = Buffer::from_bytes(before.clone()).with_media_type(MimeType::XLSX.into());
    yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(rows.schema(), vec![Ok(rows)]),
        &ExcelOptions::new().with_sheet("Fresh"),
    )
    .unwrap();
    let after = handle.read_all_bytes().unwrap();
    assert_eq!(member(&after, SHEET1), member(&before, SHEET1));
    assert_eq!(
        member(&after, "xl/worksheets/sheet2.xml"),
        member(&before, "xl/worksheets/sheet2.xml")
    );
    assert_eq!(
        sheet_data(&member(&after, "xl/worksheets/sheet3.xml")),
        format!(
            "<sheetData><row r=\"1\">{}{}</row><row r=\"2\">{}{}</row></sheetData>",
            text("A1", "id"),
            text("B1", "s"),
            number("A2", "1"),
            text("B2", "a")
        )
    );
    let workbook = Workbook::from_bytes(after).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Keep", "Data", "Fresh"]);
}

#[test]
fn the_excel_wrapper_writes_the_sheet_it_names() {
    let field = trades();
    let mut excel = Excel::new(xlsx()).with_sheet("Trades");
    let options = excel.record_options().unwrap();
    excel
        .overwrite_arrow_batch(batch(&field, vec![trade(1, Some("a"))]), &options)
        .unwrap();
    let bytes = excel.into_handle().read_all_bytes().unwrap();
    let workbook = Workbook::from_bytes(bytes.clone()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Trades"]);
    assert_eq!(
        row(&member(&bytes, SHEET1), 2),
        format!(
            "<row r=\"2\">{}{}</row>",
            number("A2", "1"),
            text("B2", "a")
        )
    );
}

#[test]
fn a_commit_cadence_publishes_every_record_in_order() {
    let field = trades();
    let mut handle = xlsx();
    let mut options = handle.record_options().unwrap();
    options.set_commit_row_size(Some(2));
    let label = |id: i64| format!("r{id}");
    let records: Vec<Scalar> = (1..=5).map(|id| trade(id, Some(&label(id)))).collect();
    handle
        .overwrite_arrow_batch(batch(&field, records), &options)
        .unwrap();

    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 1),
        format!("<row r=\"1\">{}{}</row>", text("A1", "id"), text("B1", "s"))
    );
    for id in 1..=5_u32 {
        let at = id + 1;
        assert_eq!(
            row(&xml, at),
            format!(
                "<row r=\"{at}\">{}{}</row>",
                number(&format!("A{at}"), &id.to_string()),
                text(&format!("B{at}"), &label(i64::from(id)))
            )
        );
    }
    assert!(!xml.contains("<row r=\"7\">"), "{xml}");
    assert_eq!(handle.row_size().unwrap(), 5);
}

#[test]
fn an_append_writes_its_records_below_the_stored_ones_under_the_stored_field() {
    let field = trades();
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_batch(batch(&field, vec![trade(1, Some("a"))]), &options)
        .unwrap();
    handle
        .append_arrow_batch(
            batch(&field, vec![trade(2, Some("b")), trade(3, Some("c"))]),
            &options,
        )
        .unwrap();

    let xml = sheet_part(&handle);
    assert_eq!(
        row(&xml, 1),
        format!("<row r=\"1\">{}{}</row>", text("A1", "id"), text("B1", "s"))
    );
    assert!(!xml.contains("<row r=\"5\">"), "{xml}");
    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from("a"));
    assert_eq!(sheet.scalar("B3".parse().unwrap()), Scalar::from("b"));
    assert_eq!(sheet.scalar("B4".parse().unwrap()), Scalar::from("c"));

    // The stored sheet states its numbers as float64, and the appended ids
    // are that column's.
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    let ids = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("float64 ids");
    assert_eq!(ids.values().to_vec(), vec![1.0, 2.0, 3.0]);
    assert_eq!(ids.null_count(), 0);
}

#[test]
fn a_source_failing_mid_stream_leaves_the_handle_as_it_was() {
    let field = trades();
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_batch(batch(&field, vec![trade(1, Some("a"))]), &options)
        .unwrap();
    let before = handle.read_all_bytes().unwrap();

    let schema = field.clone().into_arrow_schema().unwrap();
    let items = vec![
        Ok(batch(&field, vec![trade(2, Some("b"))])),
        Err(ArrowError::ComputeError("the source broke".into())),
    ];
    let error = handle
        .overwrite_arrow_reader(stream(schema, items), &options)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Arrow schema error: Compute error: the source broke"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), before);
}

#[test]
fn a_value_refused_in_a_later_batch_keeps_its_type_and_leaves_the_handle_as_it_was() {
    let field = trades();
    let mut handle = xlsx();
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_batch(batch(&field, vec![trade(1, Some("a"))]), &options)
        .unwrap();
    let before = handle.read_all_bytes().unwrap();

    let longer = "x".repeat(yggdryl::excel::MAX_CELL_TEXT + 1);
    let schema = field.clone().into_arrow_schema().unwrap();
    let items = vec![
        Ok(batch(&field, vec![trade(2, Some("b"))])),
        Ok(batch(&field, vec![trade(3, Some(&longer))])),
    ];
    // The archive reading the part saw an I/O failure; the caller gets the
    // refusal the writer raised.
    let error = yggdryl::excel::overwrite_arrow_reader(
        &mut handle,
        stream(schema, items),
        &ExcelOptions::new(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error:?}");
    assert_eq!(
        error.to_string(),
        "invalid record value at Sheet1!B3: expected at most 32767 characters in a cell, got 32768"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), before);
}
