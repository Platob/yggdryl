//! `rust/src/excel/reader.rs`: the record read path over one worksheet - the
//! field a sheet's cells prove, the header pairing a declared field's columns,
//! the range a read addresses, and every refusal located by sheet and cell.

use arrow_schema::ArrowError;
use yggdryl::RecordHeader;
use yggdryl::excel::ExcelOptions;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, IOMedia, MimeType, Serie, StructType, TimeUnit, Timezone};

use crate::excel_package::{
    content_types, one_sheet, package, root_relationships, workbook, workbook_relationships,
    worksheet,
};

/// An inline string cell.
fn s(reference: &str, text: &str) -> String {
    format!("<c r=\"{reference}\" t=\"inlineStr\"><is><t>{text}</t></is></c>")
}

/// A numeric cell in the General format.
fn n(reference: &str, value: &str) -> String {
    format!("<c r=\"{reference}\"><v>{value}</v></c>")
}

/// A numeric cell under the `cellXfs` index `style`.
fn styled(reference: &str, style: u32, value: &str) -> String {
    format!("<c r=\"{reference}\" s=\"{style}\"><v>{value}</v></c>")
}

/// A boolean cell.
fn b(reference: &str, value: &str) -> String {
    format!("<c r=\"{reference}\" t=\"b\"><v>{value}</v></c>")
}

/// One `<row>` of `cells`, its one-based index stated.
fn row(index: u32, cells: &[String]) -> String {
    format!("<row r=\"{index}\">{}</row>", cells.concat())
}

/// A buffer holding `bytes` under the `.xlsx` media type.
fn xlsx(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into())
}

/// A one-sheet workbook of `rows`, with no shared strings and no styles.
fn sheet(rows: &[String]) -> Buffer {
    xlsx(one_sheet(&rows.concat(), &[], &[], &[]))
}

/// The trades sheet: a header, then three rows, the second lacking `B3`.
fn trades() -> Buffer {
    sheet(&[
        row(
            1,
            &[
                s("A1", "id"),
                s("B1", "symbol"),
                s("C1", "price"),
                s("D1", "live"),
            ],
        ),
        row(
            2,
            &[
                n("A2", "1"),
                s("B2", "AAPL"),
                n("C2", "187.25"),
                b("D2", "1"),
            ],
        ),
        row(3, &[n("A3", "2"), n("C3", "410.5"), b("D3", "0")]),
        row(
            4,
            &[n("A4", "3"), s("B4", "MSFT"), n("C4", "-1.5"), b("D4", "1")],
        ),
    ])
}

/// A record root over `children`.
fn root(children: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(children).unwrap()).required_field("row")
}

/// Record options for a workbook read.
fn excel(options: ExcelOptions) -> RecordOptions {
    RecordOptions::from(options)
}

/// Each child of a record field as `name dtype`, `not null` when required.
fn columns(field: &Field) -> Vec<String> {
    field
        .fields()
        .iter()
        .map(|child| {
            format!(
                "{} {}{}",
                child.name(),
                child.dtype(),
                if child.is_nullable() { "" } else { " not null" }
            )
        })
        .collect()
}

/// Every record a read answers, as its JSON.
fn records(handle: &Buffer, options: &RecordOptions) -> Vec<String> {
    let mut rows = Vec::new();
    for batch in handle.read_arrow_reader(options).unwrap() {
        let batch = batch.unwrap();
        let serie = Serie::from_arrow_batch(None, &batch, Default::default()).unwrap();
        for index in 0..serie.len() {
            rows.push(serie.scalar(index).unwrap().into_json().unwrap());
        }
    }
    rows
}

/// The refusal a read answers, from the reader's construction or its first
/// failing batch - the crate's own error the batch stream carries.
fn refusal(handle: &Buffer, options: &RecordOptions) -> String {
    match handle.read_arrow_reader(options) {
        Err(error) => error.to_string(),
        Ok(reader) => {
            for batch in reader {
                match batch {
                    Err(ArrowError::ExternalError(error)) => return error.to_string(),
                    Err(other) => panic!("expected the crate's refusal, got {other}"),
                    Ok(_) => {}
                }
            }
            panic!("the read answered every batch");
        }
    }
}

#[test]
fn inference_types_numbers_as_float64_text_as_utf8_and_booleans_as_boolean() {
    let handle = trades();
    let field = handle
        .read_arrow_field(&handle.record_options().unwrap())
        .unwrap();
    assert_eq!(field.name(), "row");
    assert!(!field.is_nullable());
    assert_eq!(
        columns(&field),
        [
            "id float64 not null",
            "symbol utf8",
            "price float64 not null",
            "live boolean not null",
        ]
    );
    assert_eq!(handle.column_size().unwrap(), 4);
    assert_eq!(
        records(&handle, &handle.record_options().unwrap()),
        [
            "[1.0,\"AAPL\",187.25,true]",
            "[2.0,null,410.5,false]",
            "[3.0,\"MSFT\",-1.5,true]",
        ]
    );
}

#[test]
fn inference_reads_a_temporal_style_as_the_temporal_its_format_names() {
    // cellXfs: General, a date (14), a datetime (22), a time (21) and the
    // elapsed `[h]:mm:ss` (46).
    let handle = xlsx(one_sheet(
        &[
            row(
                1,
                &[
                    s("A1", "day"),
                    s("B1", "stamp"),
                    s("C1", "clock"),
                    s("D1", "span"),
                ],
            ),
            row(
                2,
                &[
                    styled("A2", 1, "45292"),
                    styled("B2", 2, "45292.5"),
                    styled("C2", 3, "0.75"),
                    styled("D2", 4, "1.5"),
                ],
            ),
        ]
        .concat(),
        &[],
        &[],
        &[0, 14, 22, 21, 46],
    ));
    let options = handle.record_options().unwrap();
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        [
            "day date32 not null",
            "stamp datetime64(ms) not null",
            "clock time32(ms) not null",
            "span duration64(ms) not null",
        ]
    );
    assert_eq!(
        records(&handle, &options),
        ["[\"2024-01-01\",\"2024-01-01T12:00:00.000\",\"18:00:00.000\",\"PT129600.000S\"]"]
    );
}

#[test]
fn a_serial_counts_from_the_workbooks_own_date_system() {
    let content_types = content_types(1, false, true);
    let root_relationships = root_relationships();
    let relationships = workbook_relationships(1, false, true);
    let workbook = workbook(&["Sheet1"], true);
    let sheet = worksheet(&[row(1, &[s("A1", "day")]), row(2, &[styled("A2", 1, "0")])].concat());
    let styles = crate::excel_package::styles(&[], &[0, 14]);
    let handle = xlsx(package(&[
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root_relationships),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &relationships),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/styles.xml", &styles),
    ]));
    assert_eq!(
        records(&handle, &handle.record_options().unwrap()),
        ["[\"1904-01-01\"]"]
    );
}

#[test]
fn a_header_over_no_value_infers_a_nullable_null_column() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", "note")]),
        row(2, &[n("A2", "1")]),
        row(3, &[n("A3", "2")]),
    ]);
    let options = handle.record_options().unwrap();
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "note null"]
    );
    assert_eq!(records(&handle, &options), ["[1.0,null]", "[2.0,null]"]);
}

#[test]
fn a_blank_header_over_values_is_named_by_its_letters_and_over_none_is_dropped() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", ""), s("C1", "")]),
        row(2, &[n("A2", "1"), s("B2", "x")]),
        row(3, &[n("A3", "2")]),
    ]);
    let options = handle.record_options().unwrap();
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "B utf8"]
    );
    assert_eq!(records(&handle, &options), ["[1.0,\"x\"]", "[2.0,null]"]);
}

#[test]
fn without_a_header_the_first_row_is_data_and_the_columns_are_named_by_their_letters() {
    let handle = sheet(&[
        row(1, &[n("A1", "1"), s("B1", "AAPL")]),
        row(2, &[n("A2", "2"), s("B2", "MSFT")]),
    ]);
    let options = excel(ExcelOptions::new().with_header(RecordHeader::None));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["A float64 not null", "B utf8 not null"]
    );
    assert_eq!(
        records(&handle, &options),
        ["[1.0,\"AAPL\"]", "[2.0,\"MSFT\"]"]
    );
}

#[test]
fn a_header_naming_one_column_twice_is_refused_at_the_ranges_first_cell() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", "id")]),
        row(2, &[n("A2", "1"), n("B2", "2")]),
    ]);
    assert_eq!(
        handle
            .read_arrow_field(&handle.record_options().unwrap())
            .unwrap_err()
            .to_string(),
        "invalid record value at Sheet1!A1: the header names no valid columns: invalid \
         StructType datatype: duplicate field name \"id\""
    );
}

#[test]
fn a_later_cell_of_another_datatype_is_refused_naming_the_sheet_and_the_cell() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", "price")]),
        row(2, &[n("A2", "1"), n("B2", "2.5")]),
        row(3, &[n("A3", "2"), s("B3", "n/a")]),
    ]);
    let options = handle.record_options().unwrap();
    assert_eq!(
        handle.read_arrow_field(&options).unwrap_err().to_string(),
        "invalid record value at Sheet1!B3: expected float64 like the column's first value, got \
         utf8; declare a field to read the column as one datatype"
    );
    assert_eq!(
        refusal(&handle, &options),
        "invalid record value at Sheet1!B3: expected float64 like the column's first value, got \
         utf8; declare a field to read the column as one datatype"
    );
}

#[test]
fn inference_reports_the_first_mismatch_in_row_order_across_columns() {
    for (header, rows, expected) in [
        (
            RecordHeader::Source,
            vec![
                row(1, &[s("A1", "left"), s("B1", "right")]),
                row(2, &[n("A2", "1"), n("B2", "2")]),
                row(3, &[n("A3", "3"), s("B3", "bad")]),
                row(4, &[s("A4", "bad"), n("B4", "4")]),
            ],
            "Sheet1!B3",
        ),
        (
            RecordHeader::None,
            vec![
                row(1, &[n("A1", "1"), n("B1", "2")]),
                row(2, &[n("A2", "3"), s("B2", "bad")]),
                row(3, &[s("A3", "bad"), n("B3", "4")]),
            ],
            "Sheet1!B2",
        ),
    ] {
        let handle = sheet(&rows);
        let options = excel(ExcelOptions::new().with_header(header));
        let error = handle.read_arrow_field(&options).unwrap_err().to_string();
        assert!(error.contains(expected), "{header:?}: {error}");
        assert!(refusal(&handle, &options).contains(expected));
    }
}

#[test]
fn explicit_flat_inference_reports_type_mismatch_before_later_malformed_number() {
    for (header, rows, expected) in [
        (
            RecordHeader::Source,
            vec![
                row(1, &[s("A1", "Qty")]),
                row(2, &[n("A2", "1")]),
                row(3, &[s("A3", "bad")]),
                row(4, &[n("A4", "oops")]),
            ],
            "Sheet1!A3",
        ),
        (
            RecordHeader::None,
            vec![
                row(1, &[n("A1", "1")]),
                row(2, &[s("A2", "bad")]),
                row(3, &[n("A3", "oops")]),
            ],
            "Sheet1!A2",
        ),
    ] {
        let handle = sheet(&rows);
        let options = excel(ExcelOptions::new().with_header(header));
        let error = handle.read_arrow_field(&options).unwrap_err().to_string();
        assert!(error.contains(expected), "{header:?}: {error}");
        assert!(refusal(&handle, &options).contains(expected));
    }
}

#[test]
fn infer_declared_numeric_field_uses_source_when_text_labels_cannot_be_data() {
    let handle = sheet(&[
        row(1, &[s("A1", "Qty")]),
        row(2, &[s("A2", "1")]),
        row(3, &[s("A3", "2")]),
    ]);
    let options = excel(
        ExcelOptions::new()
            .with_header(RecordHeader::Infer)
            .with_field(root([DataType::Float64.required_field("Qty")])),
    );
    assert_eq!(records(&handle, &options), ["[1.0]", "[2.0]"]);
}

#[test]
fn infer_declared_field_uses_none_when_required_name_cannot_pair_with_source_header() {
    let handle = sheet(&[row(1, &[s("A1", "Qty")]), row(2, &[s("A2", "abc")])]);
    let options = excel(
        ExcelOptions::new()
            .with_header(RecordHeader::Infer)
            .with_field(root([DataType::utf8().required_field("Value")])),
    );
    assert_eq!(records(&handle, &options), ["[\"Qty\"]", "[\"abc\"]"]);
}

#[test]
fn a_shared_string_index_past_the_table_is_refused_naming_the_cell() {
    let handle = xlsx(one_sheet(
        &[
            row(1, &["<c r=\"A1\" t=\"s\"><v>0</v></c>".to_owned()]),
            row(2, &["<c r=\"A2\" t=\"s\"><v>5</v></c>".to_owned()]),
        ]
        .concat(),
        &["symbol", "AAPL"],
        &[],
        &[],
    ));
    // The kind alone proves the column's datatype; the row read resolves the
    // index and refuses it.
    let options = handle.record_options().unwrap();
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["symbol utf8 not null"]
    );
    assert_eq!(
        refusal(&handle, &options),
        "invalid record value at Sheet1!A2: expected a shared string index below 2, got 5"
    );
}

#[test]
fn a_range_limits_the_rows_and_columns_and_its_first_row_is_the_header() {
    let handle = sheet(&[
        row(1, &[s("A1", "report")]),
        row(2, &[n("A2", "7"), s("B2", "junk")]),
        row(
            3,
            &[
                s("A3", "skip"),
                s("B3", "symbol"),
                s("C3", "price"),
                s("D3", "extra"),
            ],
        ),
        row(
            4,
            &[s("A4", "x"), s("B4", "AAPL"), n("C4", "1.5"), n("D4", "9")],
        ),
        row(
            5,
            &[s("A5", "x"), s("B5", "MSFT"), n("C5", "2.5"), n("D5", "9")],
        ),
    ]);
    let options = excel(ExcelOptions::new().with_range("B3:C".parse().unwrap()));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["symbol utf8 not null", "price float64 not null"]
    );
    assert_eq!(
        records(&handle, &options),
        ["[\"AAPL\",1.5]", "[\"MSFT\",2.5]"]
    );
    let mut media = handle.record_options().unwrap();
    media
        .set_excel_range(Some("B3:C".parse().unwrap()))
        .unwrap();
    assert_eq!(
        records(&handle, &media),
        ["[\"AAPL\",1.5]", "[\"MSFT\",2.5]"]
    );
}

#[test]
fn an_open_column_range_reads_every_row_of_its_columns() {
    let handle = sheet(&[
        row(
            1,
            &[
                s("A1", "id"),
                s("B1", "symbol"),
                s("C1", "price"),
                s("D1", "note"),
            ],
        ),
        row(
            2,
            &[
                s("A2", "one"),
                s("B2", "AAPL"),
                n("C2", "1.5"),
                n("D2", "1"),
            ],
        ),
        row(
            3,
            &[
                n("A3", "2"),
                s("B3", "MSFT"),
                n("C3", "2.5"),
                s("D3", "two"),
            ],
        ),
    ]);
    let options = excel(ExcelOptions::new().with_range("B:C".parse().unwrap()));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["symbol utf8 not null", "price float64 not null"]
    );
    assert_eq!(
        records(&handle, &options),
        ["[\"AAPL\",1.5]", "[\"MSFT\",2.5]"]
    );
}

#[test]
fn a_row_range_reads_only_its_rows_the_first_naming_the_columns() {
    let handle = sheet(&[
        row(1, &[s("A1", "report"), s("B1", "draft")]),
        row(2, &[n("A2", "1"), n("B2", "2")]),
        row(3, &[s("A3", "id"), s("B3", "symbol")]),
        row(4, &[n("A4", "1"), s("B4", "AAPL")]),
        row(5, &[n("A5", "2"), s("B5", "MSFT")]),
        row(6, &[s("A6", "total"), n("B6", "2")]),
    ]);
    let options = excel(ExcelOptions::new().with_range("3:5".parse().unwrap()));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "symbol utf8 not null"]
    );
    assert_eq!(
        records(&handle, &options),
        ["[1.0,\"AAPL\"]", "[2.0,\"MSFT\"]"]
    );
}

#[test]
fn row_size_counts_every_row_inside_the_range_less_the_header() {
    let handle = sheet(&[
        row(1, &[s("A1", "id")]),
        row(2, &[s("A2", "a")]),
        "<row r=\"3\"/>".to_owned(),
        row(4, &[s("A4", "c")]),
        row(6, &[s("A6", "e")]),
    ]);
    assert_eq!(handle.row_size().unwrap(), 4);
    let options = |excel: ExcelOptions| {
        let mut options = handle.record_options().unwrap();
        options.set_excel_range(excel.range()).unwrap();
        options.set_header(excel.header).unwrap();
        options
    };
    let counted = |options: RecordOptions| records(&handle, &options).len();
    assert_eq!(counted(options(ExcelOptions::new())), 4);
    assert_eq!(
        counted(options(ExcelOptions::new().with_header(RecordHeader::None))),
        5
    );
    assert_eq!(
        counted(options(
            ExcelOptions::new().with_range("2:4".parse().unwrap())
        )),
        2
    );
    // The empty row declared in the part is a record with every cell absent.
    assert_eq!(records(&handle, &options(ExcelOptions::new()))[1], "[null]");
}

#[test]
fn a_declared_field_reads_numbers_through_its_own_value_contract() {
    let handle = sheet(&[
        row(
            1,
            &[
                s("A1", "qty"),
                s("B1", "amount"),
                s("C1", "ratio"),
                s("D1", "label"),
            ],
        ),
        row(
            2,
            &[n("A2", "3"), n("B2", "12.5"), n("C2", "0.25"), n("D2", "7")],
        ),
        row(
            3,
            &[n("A3", "4"), n("B3", "7"), n("C3", "1"), n("D3", "1.5")],
        ),
    ]);
    let options = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("qty"),
        DataType::decimal128(10, 2)
            .unwrap()
            .required_field("amount"),
        DataType::Float64.required_field("ratio"),
        DataType::utf8().required_field("label"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[3,\"12.50\",0.25,\"7\"]", "[4,\"7.00\",1.0,\"1.5\"]"]
    );
}

#[test]
fn a_fraction_under_a_required_int64_column_is_refused_naming_the_cell() {
    let handle = sheet(&[
        row(1, &[s("A1", "qty")]),
        row(2, &[n("A2", "3")]),
        row(3, &[n("A3", "2.5")]),
    ]);
    let options = handle
        .record_options()
        .unwrap()
        .with_field(root([DataType::Int64.required_field("qty")]));
    assert_eq!(
        refusal(&handle, &options),
        "invalid record value at Sheet1!A3: expected int64, got string"
    );
}

#[test]
fn safe_nulls_an_unconvertible_value_in_a_nullable_column_and_unsafe_refuses_it() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", "price")]),
        row(2, &[n("A2", "1"), s("B2", "n/a")]),
        row(3, &[n("A3", "2"), n("B3", "2.5")]),
    ]);
    let nullable = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("price"),
    ]));
    assert_eq!(records(&handle, &nullable), ["[1,null]", "[2,2.5]"]);

    assert_eq!(
        refusal(&handle, &nullable.clone().with_safe(false)),
        "invalid record value at Sheet1!B2: expected float64, got string"
    );

    let required = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ]));
    assert_eq!(
        refusal(&handle, &required),
        "invalid record value at Sheet1!B2: expected float64, got string"
    );
}

#[test]
fn an_error_cell_is_a_value_no_column_holds() {
    let handle = sheet(&[
        row(1, &[s("A1", "id"), s("B1", "ratio")]),
        row(
            2,
            &[
                n("A2", "1"),
                "<c r=\"B2\" t=\"e\"><v>#DIV/0!</v></c>".to_owned(),
            ],
        ),
    ]);
    // Inference reads an error cell as absent.
    let options = handle.record_options().unwrap();
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "ratio null"]
    );
    let declared = options.with_field(root([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("ratio"),
    ]));
    assert_eq!(records(&handle, &declared), ["[1,null]"]);
    assert_eq!(
        refusal(&handle, &declared.with_safe(false)),
        "invalid record value at Sheet1!B2: expected a float64 value, got the error #DIV/0!"
    );
}

#[test]
fn text_and_numbers_read_into_a_declared_boolean_column() {
    let handle = sheet(&[
        row(1, &[s("A1", "a"), s("B1", "b"), s("C1", "c")]),
        row(2, &[s("A2", "TRUE"), n("B2", "1"), b("C2", "0")]),
        row(3, &[s("A3", "false"), n("B3", "0"), b("C3", "1")]),
    ]);
    let options = handle.record_options().unwrap().with_field(root([
        DataType::Boolean.required_field("a"),
        DataType::Boolean.required_field("b"),
        DataType::Boolean.required_field("c"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[true,true,false]", "[false,false,true]"]
    );
}

#[test]
fn a_serial_reads_into_the_declared_temporal_at_its_unit_whatever_the_cells_style() {
    let handle = sheet(&[
        row(1, &[s("A1", "day"), s("B1", "stamp"), s("C1", "clock")]),
        row(2, &[n("A2", "45292"), n("B2", "45292.5"), n("C2", "0.5")]),
    ]);
    let options = handle.record_options().unwrap().with_field(root([
        DataType::Date32.required_field("day"),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)
            .unwrap()
            .required_field("stamp"),
        DataType::time64(TimeUnit::Microsecond)
            .unwrap()
            .required_field("clock"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[\"2024-01-01\",\"2024-01-01T12:00:00.000000\",\"12:00:00.000000\"]"]
    );
}

#[test]
fn a_d_cell_reads_its_iso_8601_text_into_a_declared_datetime_or_date() {
    let handle = sheet(&[
        row(1, &[s("A1", "at"), s("B1", "on")]),
        row(
            2,
            &[
                "<c r=\"A2\" t=\"d\"><v>2024-01-02T03:04:05</v></c>".to_owned(),
                "<c r=\"B2\" t=\"d\"><v>2024-01-02</v></c>".to_owned(),
            ],
        ),
    ]);
    let options = handle.record_options().unwrap().with_field(root([
        DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)
            .unwrap()
            .required_field("at"),
        DataType::Date32.required_field("on"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[\"2024-01-02T03:04:05.000\",\"2024-01-02\"]"]
    );
}

#[test]
fn an_absent_cell_is_null_in_a_nullable_column_and_refused_in_a_required_one() {
    let handle = trades();
    let nullable = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ]));
    assert_eq!(
        records(&handle, &nullable),
        ["[1,\"AAPL\"]", "[2,null]", "[3,\"MSFT\"]"]
    );

    let required = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ]));
    let refusal = refusal(&handle, &required);
    assert!(refusal.contains("symbol"), "{refusal}");
}

#[test]
fn declared_columns_pair_with_the_header_by_name_in_any_order() {
    let handle = trades();
    let options = handle.record_options().unwrap().with_field(root([
        DataType::Boolean.required_field("live"),
        DataType::Float64.required_field("price"),
        DataType::Int64.required_field("id"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[true,187.25,1]", "[false,410.5,2]", "[true,-1.5,3]"]
    );
}

#[test]
fn without_a_header_declared_columns_pair_by_position_from_the_ranges_first_column() {
    let handle = sheet(&[
        row(
            1,
            &[s("A1", "x"), s("B1", "AAPL"), n("C1", "1.5"), s("D1", "y")],
        ),
        row(
            2,
            &[s("A2", "x"), s("B2", "MSFT"), n("C2", "2.5"), s("D2", "y")],
        ),
    ]);
    let options = excel(
        ExcelOptions::new()
            .with_header(RecordHeader::None)
            .with_range("B:C".parse().unwrap()),
    )
    .with_field(root([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
    ]));
    assert_eq!(
        records(&handle, &options),
        ["[\"AAPL\",1.5]", "[\"MSFT\",2.5]"]
    );
}

#[test]
fn header_none_declared_columns_stop_at_the_explicit_worksheet_range() {
    let handle = sheet(&[
        row(1, &[n("A1", "1"), n("B1", "2"), n("C1", "999")]),
        row(2, &[n("A2", "3"), n("B2", "4"), n("C2", "888")]),
    ]);
    let selected = excel(
        ExcelOptions::new()
            .with_range("A1:B2".parse().unwrap())
            .with_header(RecordHeader::None),
    );
    let nullable = selected.clone().with_field(root([
        DataType::Int64.required_field("first"),
        DataType::Int64.required_field("second"),
        DataType::Int64.nullable_field("outside"),
    ]));
    assert_eq!(records(&handle, &nullable), ["[1,2,null]", "[3,4,null]"]);
    let required = selected.with_field(root([
        DataType::Int64.required_field("first"),
        DataType::Int64.required_field("second"),
        DataType::Int64.required_field("outside"),
    ]));
    let message = refusal(&handle, &required);
    assert!(message.contains("$.outside"), "{message}");
    assert!(message.contains("Sheet1"), "{message}");
}

#[test]
fn header_none_declared_columns_stop_at_the_named_table_body_width() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet
        .replace(
            "<c r=\"E2\"><v>3</v></c>",
            "<c r=\"E2\"><v>3</v></c><c r=\"F2\"><v>999</v></c>",
        )
        .replace(
            "<c r=\"E3\"><v>4</v></c>",
            "<c r=\"E3\"><v>4</v></c><c r=\"F3\"><v>888</v></c>",
        );
    let handle = xlsx(named_table_package(&parts));
    let selected = excel(
        ExcelOptions::new()
            .with_table("Quantities")
            .with_header(RecordHeader::None),
    );
    let nullable = selected.clone().with_field(root([
        DataType::Int64.required_field("year"),
        DataType::Int64.required_field("qty"),
        DataType::Int64.nullable_field("outside"),
    ]));
    assert_eq!(
        records(&handle, &nullable),
        ["[2024,3,null]", "[2025,4,null]"]
    );
    let required = selected.with_field(root([
        DataType::Int64.required_field("year"),
        DataType::Int64.required_field("qty"),
        DataType::Int64.required_field("outside"),
    ]));
    let message = refusal(&handle, &required);
    assert!(message.contains("$.outside"), "{message}");
    assert!(message.contains("Data"), "{message}");
}

#[test]
fn a_declared_column_the_sheet_lacks_is_null_when_nullable_and_refused_by_the_header_when_required()
{
    let handle = trades();
    let nullable = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ]));
    assert_eq!(
        records(&handle, &nullable),
        ["[1,null]", "[2,null]", "[3,null]"]
    );

    let options = handle.record_options().unwrap().with_field(root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
    ]));
    assert_eq!(
        refusal(&handle, &options),
        "invalid record value at $.venue: expected the column in the header of Sheet1, got [id, \
         symbol, price, live]"
    );
}

#[test]
fn batch_row_size_splits_the_rows_and_the_row_bounds_skip_and_cap_them() {
    let handle = trades();
    let split = handle.record_options().unwrap().with_batch_row_size(2);
    let sizes: Vec<usize> = handle
        .read_arrow_reader(&split)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .collect();
    assert_eq!(sizes, [2, 1]);

    let bounded = handle
        .record_options()
        .unwrap()
        .with_row_offset(1)
        .with_max_row_size(1);
    assert_eq!(records(&handle, &bounded), ["[2.0,null,410.5,false]"]);
}

#[test]
fn a_filter_keeps_only_the_rows_it_names() {
    let handle = trades();
    let options = handle
        .record_options()
        .unwrap()
        .with_filter("price > 100")
        .unwrap();
    assert_eq!(
        records(&handle, &options),
        ["[1.0,\"AAPL\",187.25,true]", "[2.0,null,410.5,false]"]
    );
}

#[test]
fn an_empty_sheet_states_no_columns_and_reads_no_rows() {
    let handle = sheet(&[]);
    let options = handle.record_options().unwrap();
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.name(), "row");
    assert!(columns(&field).is_empty());
    assert_eq!(handle.row_size().unwrap(), 0);
    assert!(records(&handle, &options).is_empty());

    let declared = options.with_field(root([DataType::utf8().nullable_field("symbol")]));
    let reader = handle.read_arrow_reader(&declared).unwrap();
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(
        reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>(),
        0
    );
}

/// A workbook of two worksheets, `First` and `Second`.
fn two_sheets() -> Buffer {
    let content_types = content_types(2, false, false);
    let root_relationships = root_relationships();
    let relationships = workbook_relationships(2, false, false);
    let workbook = workbook(&["First", "Second"], false);
    let first = worksheet(&[row(1, &[s("A1", "id")]), row(2, &[n("A2", "1")])].concat());
    let second = worksheet(&[row(1, &[s("A1", "name")]), row(2, &[s("A2", "two")])].concat());
    xlsx(package(&[
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root_relationships),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &relationships),
        ("xl/worksheets/sheet1.xml", &first),
        ("xl/worksheets/sheet2.xml", &second),
    ]))
}

#[test]
fn a_read_addresses_the_sheet_named_compared_without_case_else_the_first() {
    let handle = two_sheets();
    assert_eq!(
        records(&handle, &handle.record_options().unwrap()),
        ["[1.0]"]
    );
    let second = excel(ExcelOptions::new().with_sheet("second"));
    assert_eq!(
        columns(&handle.read_arrow_field(&second).unwrap()),
        ["name utf8 not null"]
    );
    assert_eq!(records(&handle, &second), ["[\"two\"]"]);
    let mut media = handle.record_options().unwrap();
    media.set_excel_sheet(Some("Second")).unwrap();
    assert_eq!(records(&handle, &media), ["[\"two\"]"]);
}

#[test]
fn a_sheet_the_workbook_lacks_reads_as_the_empty_stream() {
    // A missing resource reads as nothing, and so does a sheet the package
    // does not hold: no rows under the declared field, an empty root without
    // one. The random-access door, Workbook::sheet, is the strict one.
    let handle = two_sheets();
    let options = excel(ExcelOptions::new().with_sheet("Missing"));
    let inferred = handle.read_arrow_field(&options).unwrap();
    assert_eq!(inferred.field_len(), 0);
    assert_eq!(inferred.name(), "row");
    // The handle's own count is the first sheet's, which the options do not
    // reach; the missing sheet counts nothing.
    assert_eq!(handle.row_size().unwrap(), 1);
    assert_eq!(
        yggdryl::excel::read_field(&handle, &ExcelOptions::new().with_sheet("Missing"))
            .unwrap()
            .field_len(),
        0
    );
    assert_eq!(
        handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        0
    );
    let declared = options.with_field(
        DataType::from(StructType::from_fields([DataType::utf8().required_field("name")]).unwrap())
            .required_field("row"),
    );
    let reader = handle.read_arrow_reader(&declared).unwrap();
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(
        reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>(),
        0
    );
}

#[test]
fn a_chartsheet_is_skipped_by_default_and_refused_by_name_while_a_hidden_sheet_reads() {
    let content_types = content_types(2, false, false);
    let root_relationships = root_relationships();
    let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet\" Target=\"chartsheets/sheet1.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/>\
        </Relationships>";
    let workbook = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <workbook xmlns=\"{}\" xmlns:r=\"{}\"><sheets>\
         <sheet name=\"Chart1\" sheetId=\"1\" r:id=\"rId1\"/>\
         <sheet name=\"Data\" sheetId=\"2\" r:id=\"rId2\"/>\
         <sheet name=\"Lookup\" sheetId=\"3\" state=\"hidden\" r:id=\"rId3\"/>\
         </sheets></workbook>",
        crate::excel_package::NS,
        crate::excel_package::R_NS,
    );
    let chart = format!("<chartsheet xmlns=\"{}\"/>", crate::excel_package::NS);
    let data = worksheet(&[row(1, &[s("A1", "id")]), row(2, &[n("A2", "1")])].concat());
    let lookup = worksheet(&[row(1, &[s("A1", "code")]), row(2, &[s("A2", "XNYS")])].concat());
    let handle = xlsx(package(&[
        ("[Content_Types].xml", content_types.as_str()),
        ("_rels/.rels", root_relationships.as_str()),
        ("xl/workbook.xml", workbook.as_str()),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/chartsheets/sheet1.xml", chart.as_str()),
        ("xl/worksheets/sheet1.xml", data.as_str()),
        ("xl/worksheets/sheet2.xml", lookup.as_str()),
    ]));
    assert_eq!(
        records(&handle, &handle.record_options().unwrap()),
        ["[1.0]"]
    );
    let chart = excel(ExcelOptions::new().with_sheet("Chart1"));
    assert_eq!(
        handle.read_arrow_field(&chart).unwrap_err().to_string(),
        "invalid record value at $.Chart1: expected a worksheet, got the chartsheet `Chart1`, \
         which holds no cells"
    );
    let lookup = excel(ExcelOptions::new().with_sheet("Lookup"));
    assert_eq!(records(&handle, &lookup), ["[\"XNYS\"]"]);
}

#[test]
fn named_table_header_zero_keeps_first_data_and_uses_column_names() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *table = table.replace("ref=\"A1:B3\"", "ref=\"A2:B3\" headerRowCount=\"0\"");
    let handle = xlsx(named_table_package(&parts));
    let options = excel(ExcelOptions::new().with_table("Names"));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "name utf8 not null"]
    );
    assert_eq!(
        records(&handle, &options),
        ["[1.0,\"one\"]", "[2.0,\"two\"]"]
    );
    let letters = excel(
        ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::None),
    );
    assert_eq!(
        columns(&handle.read_arrow_field(&letters).unwrap()),
        ["A float64 not null", "B utf8 not null"]
    );
    assert_eq!(
        records(&handle, &letters),
        ["[1.0,\"one\"]", "[2.0,\"two\"]"]
    );
    let declared = root([
        DataType::utf8().required_field("name"),
        DataType::Float64.required_field("id"),
    ]);
    assert_eq!(
        records(&handle, &options.with_field(declared)),
        ["[\"one\",1.0]", "[\"two\",2.0]"]
    );
}

#[test]
fn named_table_header_and_totals_only_state_columns_but_no_records() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *table = table.replace("ref=\"D1:E4\"", "ref=\"D1:E2\"");
    let media = yggdryl::excel::Excel::new(xlsx(named_table_package(&parts)))
        .with_options(ExcelOptions::new().with_table("Quantities"));
    let options = media.record_options().unwrap();
    assert_eq!(
        columns(&media.read_arrow_field(&options).unwrap()),
        ["year null", "qty null"]
    );
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 2);
    assert_eq!(
        media
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        0
    );
}

#[test]
fn named_table_declared_columns_never_fall_back_to_physical_letters() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let handle = xlsx(named_table_package(&named_table_parts()));
    let required = excel(ExcelOptions::new().with_table("Names"))
        .with_field(root([DataType::Float64.required_field("A")]));
    let mut reader = handle.read_arrow_reader(&required).unwrap();
    let error = reader
        .next()
        .expect("a required unknown column must refuse")
        .expect_err("table column id must not be read under a positional alias A")
        .to_string();
    assert!(error.contains("$.A"), "{error}");
    assert!(error.contains("id, name"), "{error}");

    let nullable = excel(ExcelOptions::new().with_table("Names"))
        .with_field(root([DataType::utf8().nullable_field("B")]));
    assert_eq!(records(&handle, &nullable), ["[null]", "[null]"]);
}

#[test]
fn named_table_without_data_still_binds_its_authoritative_column_names() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *table = table.replace("ref=\"D1:E4\"", "ref=\"D1:E2\"");
    let handle = xlsx(named_table_package(&parts));
    let valid = excel(ExcelOptions::new().with_table("Quantities"))
        .with_field(root([DataType::Int64.required_field("year")]));
    assert!(handle.read_arrow_reader(&valid).unwrap().next().is_none());
    let absent = excel(ExcelOptions::new().with_table("Quantities"))
        .with_field(root([DataType::Int64.required_field("absent")]));
    let mut reader = handle.read_arrow_reader(&absent).unwrap();
    let error = reader
        .next()
        .expect("known names bind even without data")
        .expect_err("a no-body table does not invent a required column")
        .to_string();
    assert!(error.contains("$.absent"), "{error}");
    assert!(error.contains("year, qty"), "{error}");
}

#[test]
fn named_table_body_includes_absent_leading_and_trailing_rows() {
    use crate::excel_package::{named_table_package, named_table_parts};
    for headless in [false, true] {
        let mut parts = named_table_parts();
        let table = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/tables/table1.xml")
            .unwrap()
            .1;
        *table = if headless {
            table.replace("ref=\"A1:B3\"", "ref=\"A2:B4\" headerRowCount=\"0\"")
        } else {
            table.replace("ref=\"A1:B3\"", "ref=\"A1:B4\"")
        };
        let sheet = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        *sheet = crate::excel_package::sheet(
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>name</t></is></c></row><row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\" t=\"inlineStr\"><is><t>two</t></is></c></row>",
            "<tableParts count=\"1\"><tablePart r:id=\"rIdT1\"/></tableParts>",
        );
        let handle = xlsx(named_table_package(&parts));
        let options = excel(ExcelOptions::new().with_table("Names"));
        assert_eq!(
            columns(&handle.read_arrow_field(&options).unwrap()),
            ["id float64", "name utf8"]
        );
        assert_eq!(
            records(&handle, &options),
            ["[null,null]", "[2.0,\"two\"]", "[null,null]"]
        );
        let media = yggdryl::excel::Excel::new(handle.clone())
            .with_options(ExcelOptions::new().with_table("Names"));
        assert_eq!(media.row_size().unwrap(), 3);
        assert_eq!(media.column_size().unwrap(), 2);

        let required = options.with_field(root([DataType::Float64.required_field("id")]));
        let mut reader = handle.read_arrow_reader(&required).unwrap();
        let error = reader
            .next()
            .expect("the first body row exists even without XML")
            .expect_err("a required value cannot be absent")
            .to_string();
        assert!(error.contains("Data!A2"), "{error}");
    }
}

#[test]
fn named_table_entirely_absent_body_still_has_rows_and_all_declared_columns() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *table = table.replace("ref=\"A1:B3\"", "ref=\"A2:B4\" headerRowCount=\"0\"");
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = crate::excel_package::sheet(
        "",
        "<tableParts count=\"1\"><tablePart r:id=\"rIdT1\"/></tableParts>",
    );
    for header in [RecordHeader::Source, RecordHeader::None] {
        let handle = xlsx(named_table_package(&parts));
        let options = excel(ExcelOptions::new().with_table("Names").with_header(header));
        let names = if header == RecordHeader::Source {
            ["id null", "name null"]
        } else {
            ["A null", "B null"]
        };
        assert_eq!(columns(&handle.read_arrow_field(&options).unwrap()), names);
        assert_eq!(
            records(&handle, &options),
            ["[null,null]", "[null,null]", "[null,null]"]
        );
        let media = yggdryl::excel::Excel::new(handle)
            .with_options(ExcelOptions::new().with_table("Names").with_header(header));
        assert_eq!(media.row_size().unwrap(), 3);
        assert_eq!(media.column_size().unwrap(), 2);
    }
}

#[test]
fn named_table_dense_body_does_not_change_explicit_worksheet_sparse_rows() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = crate::excel_package::sheet(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>id</t></is></c><c r=\"B1\" t=\"inlineStr\"><is><t>name</t></is></c></row><row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"B3\" t=\"inlineStr\"><is><t>two</t></is></c></row>",
        "",
    );
    let handle = xlsx(named_table_package(&parts));
    let options = excel(
        ExcelOptions::new()
            .with_sheet("Data")
            .with_range("A1:B4".parse().unwrap()),
    );
    assert_eq!(records(&handle, &options), ["[2.0,\"two\"]"]);
}

#[test]
fn header_policy_none_keeps_authoritative_table_body_and_totals() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let parts = named_table_parts();
    for (table, names, expected) in [
        (
            "Names",
            ["A float64 not null", "B utf8 not null"],
            vec!["[1.0,\"one\"]", "[2.0,\"two\"]"],
        ),
        (
            "Quantities",
            ["D float64 not null", "E float64 not null"],
            vec!["[2024.0,3.0]", "[2025.0,4.0]"],
        ),
    ] {
        let handle = xlsx(named_table_package(&parts));
        let options = excel(
            ExcelOptions::new()
                .with_table(table)
                .with_header(RecordHeader::None),
        );
        assert_eq!(columns(&handle.read_arrow_field(&options).unwrap()), names);
        assert_eq!(records(&handle, &options), expected);
        let mut media = yggdryl::excel::Excel::new(handle).with_options(
            ExcelOptions::new()
                .with_table(table)
                .with_header(RecordHeader::None),
        );
        assert_eq!(media.row_size().unwrap(), 2);
        assert_eq!(media.column_size().unwrap(), 2);
        yggdryl::IOBase::open(&mut media).unwrap();
        assert_eq!(media.row_size().unwrap(), 2);
        assert_eq!(media.column_size().unwrap(), 2);
    }
    let mut parts = parts;
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *table = table.replace("ref=\"D1:E4\"", "ref=\"D1:E2\"");
    let handle = xlsx(named_table_package(&parts));
    let options = excel(
        ExcelOptions::new()
            .with_table("Quantities")
            .with_header(RecordHeader::None),
    );
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["D null", "E null"]
    );
    assert!(records(&handle, &options).is_empty());
    let media = yggdryl::excel::Excel::new(handle).with_options(
        ExcelOptions::new()
            .with_table("Quantities")
            .with_header(RecordHeader::None),
    );
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 2);
}

#[test]
fn header_policy_source_keeps_decoded_table_names_exact() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *table = table.replace("name=\"name\"", "name=\" label&amp;key&#10; \"");
    let handle = xlsx(named_table_package(&parts));
    let options = excel(ExcelOptions::new().with_table("Names"));
    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(
        field
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        ["id", " label&key\n "]
    );
    assert_eq!(
        records(&handle, &options),
        ["[1.0,\"one\"]", "[2.0,\"two\"]"]
    );
    let declared = root([DataType::utf8().required_field(" label&key\n ")]);
    assert_eq!(
        records(&handle, &options.with_field(declared)),
        ["[\"one\"]", "[\"two\"]"]
    );
}

#[test]
fn header_policy_source_worksheet_keeps_first_present_header() {
    let handle = sheet(&[
        row(3, &[s("A3", "id"), s("B3", "qty")]),
        row(5, &[n("A5", "7"), n("B5", "8")]),
    ]);
    let options = excel(ExcelOptions::new().with_range("A1:B5".parse().unwrap()));
    assert_eq!(
        columns(&handle.read_arrow_field(&options).unwrap()),
        ["id float64 not null", "qty float64 not null"]
    );
    assert_eq!(records(&handle, &options), ["[7.0,8.0]"]);
}
