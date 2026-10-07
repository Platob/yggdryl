//! `rust/src/excel/mod.rs`: the medium end to end - records written under a
//! `.xlsx` name, read back as records, and opened as a workbook.

use std::sync::Arc;

use arrow_array::{BooleanArray, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::excel::{CellRef, Workbook};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, Scalar, StructType};

fn schema() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
            DataType::Date32.nullable_field("traded"),
        ])
        .unwrap(),
    )
    .required_field("row")
}

fn batch() -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("a b")])),
            Arc::new(Float64Array::from(vec![187.23, 410.5, -1.0e-3])),
            Arc::new(BooleanArray::from(vec![true, false, true])),
            Arc::new(Date32Array::from(vec![Some(19_723), None, Some(0)])),
        ],
    )
    .unwrap()
}

#[test]
fn records_round_trip_through_a_workbook() {
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap().with_field(schema());
    handle.overwrite_arrow_batch(batch(), &options).unwrap();

    let read = handle
        .read_arrow_field(&handle.record_options().unwrap())
        .unwrap();
    assert_eq!(read.field_len(), 5);
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0], batch());
    assert_eq!(handle.row_size().unwrap(), 3);
    assert_eq!(handle.column_size().unwrap(), 5);

    let workbook = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Sheet1"]);
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("id"));
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(CellRef::new(2, 1)), Scalar::Null);
    assert_eq!(sheet.scalar("C2".parse().unwrap()), Scalar::from(187.23));
    assert_eq!(sheet.scalar("D3".parse().unwrap()), Scalar::from(false));
    assert_eq!(
        sheet.scalar("E4".parse().unwrap()).dtype().unwrap(),
        DataType::Date32
    );
    let inferred = handle
        .read_arrow_field(&handle.record_options().unwrap())
        .unwrap();
    println!("{inferred:#}");
}

#[test]
fn a_foreign_package_reads_its_shared_strings_styles_and_inline_text() {
    use crate::excel_package::one_sheet;
    use yggdryl::excel::NumberFormat;

    let bytes = one_sheet(
        "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c><c r=\"C1\" t=\"inlineStr\"><is><t>when</t></is></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>1</v></c><c r=\"B2\" t=\"s\"><v>2</v></c><c r=\"C2\" s=\"1\"><v>45292.5</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>2</v></c><c r=\"C3\" s=\"2\"><v>45293</v></c></row>",
        &["id", "symbol", "AAPL"],
        &[(164, "yyyy-mm-dd hh:mm")],
        &[0, 164, 14],
    );
    let workbook = Workbook::from_bytes(bytes.clone()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("id"));
    assert_eq!(sheet.scalar("C1".parse().unwrap()), Scalar::from("when"));
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from("AAPL"));
    let when = sheet.cell("C2".parse().unwrap()).unwrap();
    assert_eq!(when.format(), NumberFormat::DateTime);
    assert_eq!(when.value().into_json().unwrap(), "\"2024-01-01T12:00:00\"");
    let day = sheet.cell("C3".parse().unwrap()).unwrap();
    assert_eq!(day.format(), NumberFormat::Date);
    assert_eq!(day.value().into_json().unwrap(), "\"2024-01-02\"");

    // A column whose cells disagree is refused by the cell that disagrees,
    // never widened: C2 is a datetime and C3 a date.
    let handle = Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap();
    let refusal = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(refusal.contains("Sheet1!C3"), "{refusal}");
    assert!(refusal.contains("expected datetime64(ms)"), "{refusal}");
    assert!(refusal.contains("got date32"), "{refusal}");

    // Declared, the column reads under the field's own contract.
    let declared = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::datetime64(yggdryl::TimeUnit::Second, yggdryl::Timezone::NAIVE)
                .unwrap()
                .required_field("when"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options.with_field(declared.clone()))
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    let rows =
        yggdryl::Serie::from_arrow_batch(Some(&declared), &batches[0], Default::default()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.scalar(0).unwrap().into_json().unwrap(),
        "[1,\"AAPL\",\"2024-01-01T12:00:00\"]"
    );
    assert_eq!(
        rows.scalar(1).unwrap().into_json().unwrap(),
        "[2,null,\"2024-01-02T00:00:00\"]"
    );
}
