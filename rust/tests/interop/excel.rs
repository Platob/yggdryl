//! The Excel exchange with an external implementation.
//!
//! `scripts/check_excel_interop.py` drives this target twice around an
//! openpyxl round trip: the first run writes a workbook openpyxl must read,
//! the second reads workbooks openpyxl wrote. The reading half prints
//! `SKIPPED` when the external workbook is absent - the driver fails on that
//! word - so a skipped half can never read as a pass.
//!
//! The rows are spelled here and in the driver identically: the serial
//! boundaries of the 1900 date system, text with leading and trailing
//! spaces, control characters and a literal `_x0041_`, and a null in every
//! nullable column.

use std::sync::Arc;

use arrow_array::{
    BooleanArray, Date32Array, DurationMillisecondArray, Float64Array, Int64Array, RecordBatch,
    StringArray, TimestampMillisecondArray,
};
use yggdryl::excel::{CellRef, DateSystem, Workbook};
use yggdryl::holder::Holder;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Field, IOMedia, Scalar, StructType, TimeUnit, Timezone};

// CopyFile and mapped reads cannot share the exchange fixtures on Windows.
static EXCHANGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("excel-interop");
    path
}

/// The record field both sides write and read.
fn field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
            DataType::Date32.nullable_field("traded"),
            DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)
                .expect("a naive datetime")
                .required_field("at"),
            DataType::duration64(TimeUnit::Millisecond)
                .expect("a duration")
                .required_field("took"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// Days since the Unix epoch of the dates the rows carry.
const DAY_1900_01_01: i32 = -25_567;
const DAY_1900_02_28: i32 = -25_509;
const DAY_1900_03_01: i32 = -25_508;
const DAY_2024_02_29: i32 = 19_782;

/// The third symbol: a control character, a carriage return and a literal
/// escape run, each of which the writer escapes as `_xHHHH_`.
const ESCAPED_SYMBOL: &str = "SOH\u{1}CR\rX_x0041_";

/// The rows both sides assert.
fn rows() -> RecordBatch {
    let long = format!("€ {}", "x".repeat(300));
    RecordBatch::try_new(
        field().into_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(StringArray::from(vec![
                Some(" 12"),
                Some("a "),
                Some(ESCAPED_SYMBOL),
                None,
                Some(long.as_str()),
            ])),
            Arc::new(Float64Array::from(vec![0.0, -1.5, 1e-9, 1e15, 0.1 + 0.2])),
            Arc::new(BooleanArray::from(vec![true, false, true, false, true])),
            Arc::new(Date32Array::from(vec![
                Some(DAY_1900_01_01),
                Some(DAY_1900_02_28),
                Some(DAY_1900_03_01),
                None,
                Some(DAY_2024_02_29),
            ])),
            Arc::new(TimestampMillisecondArray::from(vec![
                1_704_164_645_678,   // 2024-01-02T03:04:05.678
                0,                   // 1970-01-01T00:00:00
                951_868_799_000,     // 2000-02-29T23:59:59
                253_402_214_400_000, // 9999-12-31T00:00:00
                1_709_208_000_000,   // 2024-02-29T12:00:00
            ])),
            Arc::new(DurationMillisecondArray::from(vec![
                3_600_000, 0, 90_000_000, 1, 43_200_500,
            ])),
        ],
    )
    .expect("a batch")
}

#[test]
fn writes_a_workbook_for_the_external_reader() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let path = dir.join("from-rust.xlsx");
    let _ = std::fs::remove_file(&path);

    let mut handle = Holder::file(&path).expect("a local workbook");
    let options = handle
        .record_options()
        .expect("Excel options")
        .with_field(field());
    handle
        .overwrite_arrow_batch(rows(), &options)
        .expect("the workbook writes");

    // Proven here before the other side reads it: the crate reads back what
    // it wrote, cell for cell, under the declared field.
    let read: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch"))
        .collect();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0], rows());
    println!("excel-interop: wrote");
}

/// The cells the external writer's `Trades` sheet holds, as this crate reads
/// them: a header row, then the five rows of [`rows`], each cell the value
/// its number format spells.
fn assert_trades(workbook: &Workbook) {
    let sheet = workbook.sheet("Trades").expect("the Trades sheet");
    let cell = |reference: &str| sheet.scalar(reference.parse::<CellRef>().expect("a reference"));
    let json = |reference: &str| cell(reference).into_json().expect("JSON");
    for (column, name) in ["id", "symbol", "price", "live", "traded", "at", "took"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            cell(&format!("{}1", CellRef::column_name(column as u32))),
            Scalar::from(*name)
        );
    }
    // A number is a float64 to a reader of the file: openpyxl wrote the ids
    // as integers, and the file knows no such thing.
    assert_eq!(cell("A2"), Scalar::from(1.0));
    assert_eq!(cell("A6"), Scalar::from(5.0));
    // openpyxl keeps the spaces and writes `_x0041_` as it stands, which
    // Excel and this crate both read as the escape it is: `A`. A tab
    // survives as itself.
    assert_eq!(cell("B2"), Scalar::from(" 12"));
    assert_eq!(cell("B3"), Scalar::from("a "));
    assert_eq!(cell("B4"), Scalar::from("tab\tXA"));
    assert_eq!(cell("B5"), Scalar::Null);
    assert_eq!(
        cell("B6"),
        Scalar::from(format!("€ {}", "x".repeat(300)).as_str())
    );
    assert_eq!(cell("C2"), Scalar::from(0.0));
    assert_eq!(cell("C3"), Scalar::from(-1.5));
    assert_eq!(cell("C4"), Scalar::from(1e-9));
    assert_eq!(cell("C5"), Scalar::from(1e15));
    // openpyxl writes a double at fifteen significant digits, so 0.1 + 0.2
    // comes back as 0.3; this crate's own writer keeps the shortest text
    // that round-trips, which openpyxl reads exactly.
    assert_eq!(cell("C6"), Scalar::from(0.3));
    assert_eq!(cell("D2"), Scalar::from(true));
    assert_eq!(cell("D3"), Scalar::from(false));
    // The serial boundaries: 1, 59, 61, none, and a leap day, each read
    // under the `yyyy-mm-dd` format openpyxl set as a date32.
    assert_eq!(json("E2"), "\"1900-01-01\"");
    assert_eq!(json("E3"), "\"1900-02-28\"");
    assert_eq!(json("E4"), "\"1900-03-01\"");
    assert_eq!(cell("E5"), Scalar::Null);
    assert_eq!(json("E6"), "\"2024-02-29\"");
    assert_eq!(json("F2"), "\"2024-01-02T03:04:05.678\"");
    assert_eq!(json("F3"), "\"1970-01-01T00:00:00\"");
    assert_eq!(json("F4"), "\"2000-02-29T23:59:59\"");
    assert_eq!(json("F5"), "\"9999-12-31T00:00:00\"");
    assert_eq!(json("F6"), "\"2024-02-29T12:00:00\"");
    // A timedelta openpyxl wrote under `[h]:mm:ss` is a duration.
    assert_eq!(json("G2"), "\"PT3600S\"");
    assert_eq!(json("G4"), "\"PT90000S\"");
    assert_eq!(json("G6"), "\"PT43200.500S\"");
}

#[test]
fn reads_the_workbook_the_external_writer_produced() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }

    let workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    assert_eq!(workbook.sheet_names(), ["Trades", "Notes"]);
    assert_eq!(workbook.date_system(), DateSystem::Year1900);
    assert_trades(&workbook);
    assert_eq!(
        workbook
            .sheet("Notes")
            .expect("the Notes sheet")
            .scalar("A1".parse().expect("a reference")),
        Scalar::from("written by openpyxl")
    );

    // The record path, inferred: the header names the columns and each
    // column is what its first value proves.
    let handle = Holder::file(&path).expect("a local workbook");
    let mut options = handle.record_options().expect("Excel options");
    options
        .set_excel_sheet(Some("Trades"))
        .expect("a sheet name");
    let inferred = handle
        .read_arrow_field(&options)
        .expect("the inferred field");
    let names: Vec<&str> = inferred.fields().iter().map(Field::name).collect();
    assert_eq!(
        names,
        ["id", "symbol", "price", "live", "traded", "at", "took"]
    );
    assert_eq!(inferred.fields()[0].dtype(), &DataType::Float64);
    assert_eq!(inferred.fields()[4].dtype(), &DataType::Date32);
    assert!(inferred.fields()[1].is_nullable());
    assert_eq!(handle.row_size().expect("the row count"), 5);

    // Declared, the ids come back as the int64 they were, the escape run as
    // the `A` it spells, and every other cell as the batch this crate writes.
    let declared = options.with_field(field());
    let read: Vec<RecordBatch> = handle
        .read_arrow_reader(&declared)
        .expect("a reader")
        .map(|batch| batch.expect("a batch"))
        .collect();
    assert_eq!(read.len(), 1);
    let expected = rows();
    let symbols = StringArray::from(vec![
        Some(" 12"),
        Some("a "),
        Some("tab\tXA"),
        None,
        Some(format!("€ {}", "x".repeat(300)).as_str()),
    ]);
    let prices = Float64Array::from(vec![0.0, -1.5, 1e-9, 1e15, 0.3]);
    let expected = RecordBatch::try_new(
        expected.schema(),
        expected
            .columns()
            .iter()
            .enumerate()
            .map(|(index, column)| match index {
                1 => Arc::new(symbols.clone()) as arrow_array::ArrayRef,
                2 => Arc::new(prices.clone()) as arrow_array::ArrayRef,
                _ => Arc::clone(column),
            })
            .collect(),
    )
    .expect("the expected batch");
    assert_eq!(read[0], expected);
    println!("excel-interop: read");
}

#[test]
fn reads_the_1904_workbook_the_external_writer_produced() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-1904.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    assert_eq!(workbook.date_system(), DateSystem::Year1904);
    let sheet = workbook.sheet("Sheet").expect("the sheet");
    assert_eq!(
        sheet
            .scalar("A1".parse().expect("a reference"))
            .into_json()
            .expect("JSON"),
        "\"2024-02-29\""
    );
    assert_eq!(
        sheet
            .scalar("A2".parse().expect("a reference"))
            .into_json()
            .expect("JSON"),
        "\"1904-01-01\""
    );
    println!("excel-interop: read 1904");
}
