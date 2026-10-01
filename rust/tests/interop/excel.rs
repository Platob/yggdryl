//! The Excel exchange with an external implementation.
//!
//! `scripts/check_excel_interop.py` drives this target twice around an
//! openpyxl round trip: the first run writes a workbook openpyxl must read,
//! the second reads workbooks openpyxl wrote and edits two of them - one
//! styled, one carrying every worksheet feature the crate keeps - which
//! openpyxl then reads back. The reading half prints `SKIPPED` when the
//! external workbook is absent - the driver fails on that word - so a
//! skipped half can never read as a pass.
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
use yggdryl::excel::{CellRef, DateSystem, Frozen, Workbook};
use yggdryl::excel::{NumberFormat, StyleId};
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

/// A workbook styled through the ribbon's door, for openpyxl to read the
/// styles back: a bold header on a solid yellow fill with an outline, a
/// number under a custom format, one under a built-in format, and a date.
#[test]
fn writes_a_styled_workbook_for_the_external_reader() {
    use yggdryl::excel::{BorderPreset, BorderStyle, Borders, CellRange, Color, StylePatch};

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let path = dir.join("from-rust-styled.xlsx");
    let _ = std::fs::remove_file(&path);

    let mut workbook = Workbook::new();
    workbook.add_sheet("Styled").expect("a sheet");
    let cell = |text: &str| -> CellRef { text.parse().expect("a reference") };
    for (at, text) in [
        ("A1", "Total"),
        ("B1", "1234.5"),
        ("C1", "0.125"),
        ("D1", "1/2/2024"),
    ] {
        workbook
            .set_entry("Styled", cell(at), text)
            .expect("an entry");
    }
    let header = StylePatch {
        bold: Some(true),
        fill: Some(Some(Color::Rgb(0xFF_FF_FF_00))),
        borders: Some(Borders {
            preset: BorderPreset::Outside,
            style: BorderStyle::Thin,
            color: None,
        }),
        ..StylePatch::default()
    };
    let one = |at: &str| -> CellRange { at.parse().expect("a range") };
    workbook
        .set_style("Styled", &[one("A1")], &header)
        .expect("the header's style");
    let custom = StylePatch {
        number_format: Some("#,##0.000 \"kg\"".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Styled", &[one("B1")], &custom)
        .expect("a custom format");
    let percent = StylePatch {
        number_format: Some("0.00%".into()),
        ..StylePatch::default()
    };
    workbook
        .set_style("Styled", &[one("C1")], &percent)
        .expect("a built-in format");
    assert_eq!(
        workbook
            .display_text("Styled", cell("B1"))
            .expect("a display")
            .map(|shown| shown.text),
        Some("1,234.500 kg".into())
    );
    let mut handle = Holder::file(&path).expect("a local workbook");
    workbook
        .write_into(&mut handle)
        .expect("the workbook writes");
    println!("excel-interop: wrote styled");
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
    assert_eq!(json("F3"), "\"1970-01-01T00:00:00.000\"");
    assert_eq!(json("F4"), "\"2000-02-29T23:59:59.000\"");
    assert_eq!(json("F5"), "\"9999-12-31T00:00:00.000\"");
    assert_eq!(json("F6"), "\"2024-02-29T12:00:00.000\"");
    // A timedelta openpyxl wrote under `[h]:mm:ss` is a duration.
    assert_eq!(json("G2"), "\"PT3600.000S\"");
    assert_eq!(json("G4"), "\"PT90000.000S\"");
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

#[test]
fn edits_the_styled_workbook_the_external_writer_produced_keeping_its_styles() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-styled.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let mut workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    assert_eq!(workbook.sheet_names(), ["Styled", "Other"]);
    let reference = |text: &str| text.parse::<CellRef>().expect("a reference");
    // The styles openpyxl wrote resolve: a bold font, a percent format.
    let (header, rate) = {
        let sheet = workbook.sheet("Styled").expect("the styled sheet");
        let header = sheet.cell(reference("A1")).expect("A1").style();
        let rate = sheet.cell(reference("B2")).expect("B2");
        assert_eq!(rate.value(), &Scalar::from(0.25));
        assert_eq!(rate.format(), NumberFormat::General);
        (header, rate.style())
    };
    let styles = workbook.style_sheet().expect("the styles");
    assert!(styles.style(header).expect("A1's style").font.bold);
    assert_eq!(
        styles.style(rate).expect("B2's style").number_format,
        "0.00%"
    );
    assert_ne!(rate, StyleId::DEFAULT);

    // Beside the text, a cell of each temporal the styles part has no
    // format for: the save appends a code and an entry per format, which
    // the other side must read as what they spell.
    let edited_cells = [
        (
            "D3",
            Scalar::datetime64(1_704_067_200_250, TimeUnit::Millisecond, Timezone::NAIVE)
                .expect("a datetime"),
            NumberFormat::DateTimeFraction,
        ),
        (
            "E3",
            Scalar::time32(45_296_000, TimeUnit::Millisecond, Timezone::NAIVE).expect("a time"),
            NumberFormat::Time,
        ),
        (
            "F3",
            Scalar::duration64(129_600_000, TimeUnit::Millisecond).expect("a duration"),
            NumberFormat::Duration,
        ),
    ];
    {
        let sheet = workbook.sheet_mut("Styled").expect("the styled sheet");
        sheet
            .set_cell(reference("C3"), "edited by yggdryl")
            .expect("the edit");
        for (at, value, _) in &edited_cells {
            sheet
                .set_cell(reference(at), value.clone())
                .expect("the temporal edit");
        }
    }
    let edited = exchange_dir().join("from-rust-edited.xlsx");
    let _ = std::fs::remove_file(&edited);
    workbook
        .write_into(&mut Holder::file(&edited).expect("a local workbook"))
        .expect("the workbook saves");
    assert!(!workbook.is_dirty());

    // Proven here too before the other side reads it.
    let reopened = Workbook::open(Holder::file(&edited).expect("a local workbook"))
        .expect("the edited workbook opens");
    let sheet = reopened.sheet("Styled").expect("the styled sheet");
    assert_eq!(sheet.cell(reference("B2")).expect("B2").style(), rate);
    assert_eq!(
        sheet.scalar(reference("C3")),
        Scalar::from("edited by yggdryl")
    );
    for (at, value, format) in &edited_cells {
        let cell = sheet.cell(reference(at)).expect("the edited cell");
        assert_eq!(cell.format(), *format, "{at}");
        assert_eq!(cell.value(), value, "{at}");
    }
    println!("excel-interop: edited");
}

#[test]
fn edits_one_cell_of_the_workbook_the_external_writer_filled_with_every_feature() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-fidelity.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let reference = |text: &str| text.parse::<CellRef>().expect("a reference");
    let mut workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    assert_eq!(workbook.sheet_names(), ["Data", "Report"]);
    {
        // What openpyxl wrote about the grid reads as the layout it states.
        let sheet = workbook.sheet("Data").expect("the data sheet");
        assert_eq!(
            sheet.frozen(),
            Some(Frozen {
                rows: 1,
                columns: 1
            })
        );
        assert_eq!(
            sheet
                .merges()
                .map(|range| range.to_string())
                .collect::<Vec<_>>(),
            ["F1:G1"]
        );
        assert_eq!(sheet.column_width(0), 18.0);
        assert_eq!(sheet.row_height(1), 24.0);
        assert_eq!(sheet.row_height(7), 30.0);
        assert!(sheet.is_row_hidden(7));
        // A filled column of formulas is one shape.
        let formula = |at: &str| {
            sheet
                .cell(reference(at))
                .and_then(|cell| cell.formula())
                .cloned()
                .expect("a formula")
        };
        assert_eq!(formula("D2").at(reference("D2")).to_string(), "B2*C2");
        assert_eq!(formula("D5"), formula("D2"));
        assert_eq!(sheet.scalar(reference("A7")), Scalar::from("bold plain"));
    }
    assert_eq!(
        workbook
            .defined_names()
            .map(|name| (name.name().to_owned(), name.text()))
            .collect::<Vec<_>>()[0],
        ("Rate".to_owned(), "0.07".to_owned())
    );
    {
        let sheet = workbook.sheet_mut("Data").expect("the data sheet");
        sheet
            .set_cell(reference("A5"), "Kiwi, edited")
            .expect("the edit");
        sheet.set_cell(reference("E2"), 50.0).expect("the edit");
        // One cell of the other sheet too, so it is written again from its
        // cells: its formulas over `Data` and over a defined name included.
        workbook
            .sheet_mut("Report")
            .expect("the report sheet")
            .set_cell(reference("A3"), "written again")
            .expect("the edit");
    }
    let edited = exchange_dir().join("from-rust-fidelity.xlsx");
    let _ = std::fs::remove_file(&edited);
    workbook
        .write_into(&mut Holder::file(&edited).expect("a local workbook"))
        .expect("the workbook saves");

    let reopened = Workbook::open(Holder::file(&edited).expect("a local workbook"))
        .expect("the edited workbook opens");
    let sheet = reopened.sheet("Data").expect("the data sheet");
    assert_eq!(sheet.scalar(reference("A5")), Scalar::from("Kiwi, edited"));
    assert_eq!(
        sheet.frozen(),
        Some(Frozen {
            rows: 1,
            columns: 1
        })
    );
    assert_eq!(sheet.row_height(7), 30.0);
    let report = reopened.sheet("Report").expect("the report sheet");
    let written = |at: &str| {
        report
            .cell(reference(at))
            .and_then(|cell| cell.formula())
            .map(|formula| formula.at(reference(at)).to_string())
    };
    assert_eq!(written("A1").as_deref(), Some("SUM(Data!C2:C5)"));
    assert_eq!(written("A2").as_deref(), Some("Rate*Data!C2"));
    assert_eq!(
        report.scalar(reference("A3")),
        Scalar::from("written again")
    );
    println!("excel-interop: fidelity");
}

#[test]
fn cuts_openpyxl_conditional_format_validation_and_hyperlink_across_sheets() {
    use yggdryl::excel::{Edit, Paste};

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-carried.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let mut workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    assert_eq!(workbook.sheet_names(), ["Data", "Other"]);
    let applied = workbook
        .apply(Edit::Paste {
            from: ("Data".into(), "B2:D3".parse().expect("source range")),
            to: ("Other".into(), "J10".parse().expect("destination cell")),
            what: Paste::All,
            cut: true,
        })
        .expect("the cross-sheet cut");
    let inverse = applied.inverse.expect("the saved inverse");

    // Save this exact workbook state, then keep its origin while applying the
    // inverse: a new Workbook cannot accept another workbook's saved edit.
    let moved = exchange_dir().join("from-rust-carried.xlsx");
    let package = workbook.into_package().expect("the moved package");
    std::fs::write(&moved, package.as_bytes()).expect("the moved file");
    workbook.rebase(package).expect("adopt the saved package");
    workbook.apply(inverse).expect("undo after the save");
    let undone = exchange_dir().join("from-rust-carried-undone.xlsx");
    workbook
        .write_into(&mut Holder::file(&undone).expect("a local workbook"))
        .expect("the restored workbook saves");
    println!("excel-interop: carried cut");
}

#[test]
fn reads_openpyxl_named_tables_and_preserves_them_on_a_workbook_save() {
    use yggdryl::excel::ExcelOptions;
    use yggdryl::media::RecordOptions;

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-regions.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let handle = Holder::file(&path).expect("an openpyxl workbook");
    for (table, names, expected) in [
        (
            "Quantities",
            ["Region\nYear", "Notes\r\nDetail"],
            vec!["[2024.0,3.0]", "[null,null]", "[2025.0,4.0]"],
        ),
        (
            "Headless",
            ["Year", "Amount"],
            vec!["[2024.0,12.0]", "[2025.0,19.0]"],
        ),
    ] {
        let options = RecordOptions::from(ExcelOptions::new().with_table(table));
        let field = handle
            .read_arrow_field(&options)
            .expect("the named-table field");
        assert_eq!(
            field.fields().iter().map(Field::name).collect::<Vec<_>>(),
            names
        );
        let reader = handle
            .read_arrow_reader(&options)
            .expect("the named-table reader");
        assert_eq!(
            reader.schema(),
            field.clone().into_arrow_schema().expect("Arrow field")
        );
        let mut actual = Vec::new();
        for batch in reader {
            let batch = batch.expect("a named-table batch");
            let rows = yggdryl::Serie::from_arrow_batch(Some(&field), &batch, Default::default())
                .expect("the named-table rows");
            for index in 0..rows.len() {
                actual.push(
                    rows.scalar(index)
                        .expect("a row")
                        .into_json()
                        .expect("JSON"),
                );
            }
        }
        assert_eq!(actual, expected, "{table}");
    }

    // Preserve both table parts while changing an unrelated worksheet cell.
    let mut workbook = Workbook::open(Holder::file(&path).expect("an openpyxl workbook"))
        .expect("the workbook opens");
    assert_eq!(
        workbook
            .sheet("Data")
            .unwrap()
            .cell("B1".parse().unwrap())
            .unwrap()
            .value(),
        &Scalar::from("Notes\r\nDetail")
    );
    let total = workbook
        .sheet("Data")
        .unwrap()
        .cell("B5".parse().unwrap())
        .unwrap();
    assert_eq!(
        total.formula().unwrap().at(total.reference()).to_string(),
        "SUBTOTAL(109,B2:B4)"
    );
    workbook
        .set_entry("Data", "J8".parse().expect("a cell"), "edited by yggdryl")
        .expect("the separate cell edit");
    let edited = exchange_dir().join("from-rust-regions-edited.xlsx");
    workbook
        .write_into(&mut Holder::file(&edited).expect("the edited workbook"))
        .expect("the workbook saves");
    let reopened = Workbook::open(Holder::file(&edited).unwrap()).unwrap();
    assert_eq!(
        reopened
            .sheet("Data")
            .unwrap()
            .cell("B1".parse().unwrap())
            .unwrap()
            .value(),
        &Scalar::from("Notes\r\nDetail")
    );
    let total = reopened
        .sheet("Data")
        .unwrap()
        .cell("B5".parse().unwrap())
        .unwrap();
    assert_eq!(
        total.formula().unwrap().at(total.reference()).to_string(),
        "SUBTOTAL(109,B2:B4)"
    );
    println!("excel-interop: named tables");
}

#[test]
fn resizes_openpyxl_named_table_and_writes_both_shapes_for_outside_readers() {
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};
    use yggdryl::media::RecordOptions;

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-named-resize.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let source = Holder::file(&path).expect("an openpyxl workbook");
    for (name, labels) in [
        ("Names", ["id", "name"]),
        ("Stationary", ["kept_id", "kept_name"]),
    ] {
        let options = RecordOptions::from(ExcelOptions::new().with_table(name));
        let field = source
            .read_arrow_field(&options)
            .expect("the outside table field");
        assert_eq!(
            field.fields().iter().map(Field::name).collect::<Vec<_>>(),
            labels
        );
    }
    let input_field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = input_field.into_arrow_schema().unwrap();
    for (mode, ids, labels, expected) in [
        (
            "grow",
            vec![9_i64, 10, 11, 12],
            vec!["nine", "ten", "eleven", "twelve"],
            4,
        ),
        ("shrink", vec![9_i64], vec!["nine"], 1),
    ] {
        let output = exchange_dir().join(format!("from-rust-named-{mode}.xlsx"));
        std::fs::copy(&path, &output).expect("a separate output fixture");
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(labels)),
            ],
        )
        .unwrap();
        let mut handle = Holder::file(&output).expect("the output workbook");
        overwrite_arrow_reader(
            &mut handle,
            yggdryl::arrow::batch_reader(schema.clone(), [batch]),
            &ExcelOptions::new().with_table("Names"),
        )
        .expect("selected table resizes");
        drop(handle);
        let reopened = Holder::file(&output).expect("the written workbook");
        let options = RecordOptions::from(ExcelOptions::new().with_table("Names"));
        let count: usize = reopened
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(count, expected, "{mode}");
        let stationary = RecordOptions::from(ExcelOptions::new().with_table("Stationary"));
        let count: usize = reopened
            .read_arrow_reader(&stationary)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(count, 2, "{mode} left the stationary table");
    }
    println!("excel-interop: resized named table");
}

#[test]
fn resizes_openpyxl_totals_table_and_writes_all_bands_for_outside_readers() {
    use yggdryl::excel::{ExcelOptions, overwrite_arrow_reader};
    use yggdryl::media::RecordOptions;

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-named-totals.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let source = Holder::file(&path).expect("an openpyxl totals workbook");
    let options = RecordOptions::from(ExcelOptions::new().with_table("Quantities"));
    let field = source
        .read_arrow_field(&options)
        .expect("the outside totals table field");
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["year", "qty"]
    );
    let count: usize = source
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(count, 2, "totals row is outside the selected body");

    let schema = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row")
    .into_arrow_schema()
    .unwrap();
    for (mode, body_rows) in [("shrink", 1_usize), ("equal", 2), ("grow", 3)] {
        let output = exchange_dir().join(format!("from-rust-named-totals-{mode}.xlsx"));
        std::fs::copy(&path, &output).expect("a separate totals output fixture");
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(
                    (0..body_rows)
                        .map(|row| 2030_i64 + row as i64)
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    (0..body_rows)
                        .map(|row| 6_i64 + row as i64)
                        .collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();
        let mut handle = Holder::file(&output).expect("the totals output workbook");
        overwrite_arrow_reader(
            &mut handle,
            yggdryl::arrow::batch_reader(schema.clone(), [batch]),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .expect("selected totals table resizes");
        drop(handle);
        let reopened = Holder::file(&output).expect("the written totals workbook");
        let selected = RecordOptions::from(ExcelOptions::new().with_table("Quantities"));
        let count: usize = reopened
            .read_arrow_reader(&selected)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(count, body_rows, "{mode} selected body cardinality");
        let stationary = RecordOptions::from(ExcelOptions::new().with_table("Names"));
        let count: usize = reopened
            .read_arrow_reader(&stationary)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(count, 2, "{mode} stationary table remains");
    }
    println!("excel-interop: resized named totals");
}

#[test]
fn refuses_to_move_the_external_writers_css_only_note_atomically() {
    use yggdryl::Error;
    use yggdryl::holder::Buffer;
    use yggdryl::zip::ZipArchive;

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-fidelity.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let package = |workbook: &Workbook| {
        let archive = Arc::new(ZipArchive::new(Holder::buffer(Buffer::from_bytes(
            workbook.into_bytes().expect("the workbook serializes"),
        ))));
        archive
            .entries()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry.name().to_owned(),
                    archive.read_member(entry.name()).unwrap(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let mut workbook = Workbook::open(Holder::file(&path).unwrap()).unwrap();
    workbook.parse_all().unwrap();
    let before = package(&workbook);
    let revisions = ["Data", "Report"].map(|name| workbook.sheet(name).unwrap().revision());
    let dirty = workbook.is_dirty();
    for rows in [true, false] {
        let error = if rows {
            workbook.insert_rows("Data", 2, 1)
        } else {
            workbook.insert_columns("Data", 0, 1)
        }
        .unwrap_err();
        assert!(matches!(error, Error::Unsupported { operation, filesystem }
            if operation == "moving a VML note without a cell anchor"
                && filesystem == "xl/drawings/commentsDrawing1.vml"));
        assert_eq!(package(&workbook), before);
        assert_eq!(
            ["Data", "Report"].map(|name| workbook.sheet(name).unwrap().revision()),
            revisions
        );
        assert_eq!(workbook.is_dirty(), dirty);
    }
    println!("excel-interop: missing-anchor refusal");
}

#[test]
fn shifts_every_reference_of_the_workbook_the_external_writer_filled_with_every_feature() {
    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-anchored-fidelity.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let reference = |text: &str| text.parse::<CellRef>().expect("a reference");
    let mut workbook =
        Workbook::open(Holder::file(&path).expect("a local workbook")).expect("the workbook opens");
    // A row opened inside the rows every range covers - the conditional
    // formats, the validation, the filter, the table - and a column in
    // front of every column, inside the frozen one: each reference the
    // workbook states moves, the other sheet's formulas over `Data` too.
    workbook
        .insert_rows("Data", 2, 1)
        .expect("a row opens inside the ranges");
    workbook
        .insert_columns("Data", 0, 1)
        .expect("a column opens in front");
    let shifted = exchange_dir().join("from-rust-shifted.xlsx");
    let _ = std::fs::remove_file(&shifted);
    workbook
        .write_into(&mut Holder::file(&shifted).expect("a local workbook"))
        .expect("the workbook saves");

    // Proven here too before the other side reads it.
    let reopened = Workbook::open(Holder::file(&shifted).expect("a local workbook"))
        .expect("the shifted workbook opens");
    let formula = |sheet: &str, at: &str| {
        reopened
            .sheet(sheet)
            .expect("the sheet")
            .cell(reference(at))
            .and_then(|cell| cell.formula())
            .map(|formula| formula.at(reference(at)).to_string())
    };
    assert_eq!(formula("Data", "E2").as_deref(), Some("C2*D2"));
    assert_eq!(formula("Data", "E6").as_deref(), Some("C6*D6"));
    assert_eq!(formula("Report", "A1").as_deref(), Some("SUM(Data!D2:D6)"));
    assert_eq!(formula("Report", "A2").as_deref(), Some("Rate*Data!D2"));
    let sheet = reopened.sheet("Data").expect("the data sheet");
    assert_eq!(sheet.scalar(reference("B6")), Scalar::from("Kiwi"));
    assert_eq!(sheet.scalar(reference("G1")), Scalar::from("merged"));
    assert_eq!(
        sheet
            .merges()
            .map(|range| range.to_string())
            .collect::<Vec<_>>(),
        ["G1:H1"]
    );
    assert_eq!(
        sheet.frozen(),
        Some(Frozen {
            rows: 1,
            columns: 2
        })
    );
    assert_eq!(sheet.column_width(1), 18.0);
    assert_eq!(sheet.row_height(1), 24.0);
    assert_eq!(sheet.row_height(8), 30.0);
    assert!(sheet.is_row_hidden(8));
    // openpyxl names the filter's range and reads no name reserved to
    // Excel back, so this side proves the name moved with the filter.
    assert!(
        reopened
            .defined_names()
            .any(|name| name.name() == "_xlnm._FilterDatabase" && name.text() == "'Data'!$B$1:$E$6")
    );
    println!("excel-interop: shifted");
}

fn rows_two_exchange_field() -> Field {
    let sales = DataType::from(
        StructType::from_fields([
            DataType::Float64.nullable_field("Units"),
            DataType::Date32.nullable_field("Day"),
        ])
        .unwrap(),
    )
    .required_field("Sales\nGroup");
    DataType::from(
        StructType::from_fields([sales, DataType::utf8().nullable_field("Person\r\nName")])
            .unwrap(),
    )
    .required_field("row")
}

fn rows_two_exchange_serie() -> yggdryl::Serie {
    let field = rows_two_exchange_field();
    yggdryl::Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(2.0), Scalar::date32(19_724)]),
                Scalar::from("Ann"),
            ]),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::Null, Scalar::Null]),
                Scalar::Null,
            ]),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(3.0), Scalar::date32(19_725)]),
                Scalar::from("Bob"),
            ]),
        ],
    )
    .unwrap()
}

#[test]
fn writes_nested_rows_for_the_external_reader() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let path = dir.join("from-rust-rows.xlsx");
    let _ = std::fs::remove_file(&path);
    let serie = rows_two_exchange_serie();
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)))
        .with_field(rows_two_exchange_field());
    let mut handle = Holder::file(&path).expect("a local workbook");
    handle
        .overwrite_arrow_batch(serie.clone().into_arrow_batch().unwrap(), &options)
        .expect("nested rows write");
    let mut read = handle
        .read_arrow_reader(&options)
        .expect("nested rows read");
    let batch = read.next().expect("one batch").expect("a readable batch");
    let back = yggdryl::Serie::from_arrow_batch(
        Some(&rows_two_exchange_field()),
        &batch,
        Default::default(),
    )
    .unwrap();
    assert_eq!(back.len(), serie.len());
    for index in 0..serie.len() {
        assert_eq!(back.scalar(index).unwrap(), serie.scalar(index).unwrap());
    }
    assert!(read.next().is_none());
    println!("excel-interop: wrote rows");
}

#[test]
fn reads_the_nested_rows_the_external_writer_produced() {
    use yggdryl::media::RecordOptions;
    use yggdryl::{RecordHeader, excel::ExcelOptions};

    let _exchange = EXCHANGE.lock().expect("exclusive Excel exchange fixtures");
    let path = exchange_dir().join("from-openpyxl-rows.xlsx");
    if !path.exists() {
        println!("excel-interop: SKIPPED (no {})", path.display());
        return;
    }
    let handle = Holder::file(&path).expect("an openpyxl workbook");
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let inferred = handle.read_arrow_field(&options).expect("nested field");
    assert_eq!(inferred.fields()[0].name(), "Sales\nGroup");
    assert_eq!(
        inferred.fields()[0]
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Units", "Day"]
    );
    assert_eq!(inferred.fields()[1].name(), "Person\r\nName");
    let declared = options.with_field(rows_two_exchange_field());
    let mut read = handle.read_arrow_reader(&declared).expect("nested reader");
    let batch = read.next().expect("one batch").expect("a readable batch");
    let back = yggdryl::Serie::from_arrow_batch(
        Some(&rows_two_exchange_field()),
        &batch,
        Default::default(),
    )
    .unwrap();
    let expected = rows_two_exchange_serie();
    assert_eq!(back.len(), 3);
    for index in 0..expected.len() {
        assert_eq!(back.scalar(index).unwrap(), expected.scalar(index).unwrap());
    }
    assert!(read.next().is_none());
    println!("excel-interop: read rows");
}
