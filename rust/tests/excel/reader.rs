//! `rust/src/excel/reader.rs`: the record read path over one worksheet - the
//! field a sheet's cells prove, the header pairing a declared field's columns,
//! the range a read addresses, and every refusal located by sheet and cell.

use arrow_schema::ArrowError;
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
    let options = excel(ExcelOptions::new().with_header(false));
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
        options.set_excel_range(excel.range).unwrap();
        options.set_header(excel.header).unwrap();
        options
    };
    let counted = |options: RecordOptions| records(&handle, &options).len();
    assert_eq!(counted(options(ExcelOptions::new())), 4);
    assert_eq!(counted(options(ExcelOptions::new().with_header(false))), 5);
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
        ["[3,\"12.5\",0.25,\"7\"]", "[4,\"7\",1.0,\"1.5\"]"]
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
fn a_boolean_cell_reads_every_spelling_the_boolean_reader_reads_into_a_declared_column() {
    let handle = sheet(&[
        row(1, &[s("A1", "a"), s("B1", "b")]),
        row(2, &[b("A2", "yes"), b("B2", " Off ")]),
        row(3, &[b("A3", "N"), b("B3", "True")]),
    ]);
    let options = handle.record_options().unwrap().with_field(root([
        DataType::Boolean.required_field("a"),
        DataType::Boolean.required_field("b"),
    ]));
    assert_eq!(records(&handle, &options), ["[true,false]", "[false,true]"]);
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
            .with_header(false)
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
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root_relationships),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", relationships),
        ("xl/chartsheets/sheet1.xml", &chart),
        ("xl/worksheets/sheet1.xml", &data),
        ("xl/worksheets/sheet2.xml", &lookup),
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
