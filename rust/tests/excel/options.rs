//! `rust/src/excel/options.rs`: the settings a workbook read or write takes -
//! the shared record settings, the sheet, the header row and the range - and
//! the one `RecordOptions` variant they are.

use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use yggdryl::arrow::BatchReader;
use yggdryl::excel::{CellRange, CellRef, DEFAULT_SHEET_NAME, Excel, ExcelOptions, Workbook};
use yggdryl::holder::Buffer;
use yggdryl::ipc::IpcOptions;
use yggdryl::media::{DEFAULT_ROOT_NAME, IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, IOBase, IOMedia, Level, MimeType, Scalar, StructType, Url};

/// The rows the fixtures write: `id` and a nullable `symbol`.
fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// Three rows, the second without a symbol.
fn batch() -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT")])),
        ],
    )
    .unwrap()
}

/// An empty in-memory handle declaring the workbook media type.
fn xlsx() -> Buffer {
    Buffer::new().with_media_type(MimeType::XLSX.into())
}

/// A workbook handle holding the three rows, written under `options`.
fn written(options: ExcelOptions) -> Buffer {
    let mut handle = xlsx();
    let options = RecordOptions::from(options).with_field(schema());
    handle.overwrite_arrow_batch(batch(), &options).unwrap();
    handle
}

/// A workbook handle holding the package `workbook` renders.
fn holding(workbook: &Workbook) -> Buffer {
    Buffer::from_bytes(workbook.into_bytes().unwrap()).with_media_type(MimeType::XLSX.into())
}

/// The workbook a handle holds.
fn opened(handle: &Buffer) -> Workbook {
    Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap()
}

/// The `id` column of every batch a reader yields, in order.
fn ids(reader: BatchReader) -> Vec<i64> {
    reader
        .flat_map(|batch| {
            let batch = batch.unwrap();
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

/// The standard library's hash of one options value.
fn std_hash(value: &ExcelOptions) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn cell(text: &str) -> CellRef {
    text.parse().unwrap()
}

// The workbook settings.

#[test]
fn new_addresses_the_first_worksheet_with_a_header_row_over_the_whole_sheet() {
    let options = ExcelOptions::new();
    assert_eq!(options.sheet, None);
    assert!(options.header);
    assert_eq!(options.range, None);
    assert_eq!(options.cells(), CellRange::all());
}

#[test]
fn every_shared_setting_starts_at_the_default_of_every_encoding() {
    let options = ExcelOptions::new();
    assert_eq!(options.name(), DEFAULT_ROOT_NAME);
    assert_eq!(options.name(), "row");
    assert_eq!(options.field(), None);
    assert!(options.filter().is_always_true());
    assert!(options.select().is_all());
    assert!(options.merge_by().is_empty());
    assert!(options.safe());
    assert_eq!(options.batch_byte_size(), None);
    assert_eq!(options.batch_row_size(), None);
    assert_eq!(options.max_row_size(), None);
    assert_eq!(options.row_offset(), None);
    assert_eq!(options.max_byte_size(), None);
    assert_eq!(options.commit_batch_num(), None);
    assert_eq!(options.level(), Level::DEFAULT);
    assert!(options.plan().is_empty());
}

#[test]
fn new_and_default_are_one_value() {
    assert_eq!(ExcelOptions::default(), ExcelOptions::new());
    assert_eq!(
        std_hash(&ExcelOptions::default()),
        std_hash(&ExcelOptions::new())
    );
    assert_eq!(
        ExcelOptions::default().cmp(&ExcelOptions::new()),
        Ordering::Equal
    );
}

#[test]
fn with_sheet_with_header_and_with_range_each_replace_only_their_own_setting() {
    let range: CellRange = "B2:D9".parse().unwrap();

    let sheet = ExcelOptions::new().with_sheet("Trades");
    assert_eq!(sheet.sheet.as_deref(), Some("Trades"));
    assert!(sheet.header);
    assert_eq!(sheet.range, None);

    let header = ExcelOptions::new().with_header(false);
    assert_eq!(header.sheet, None);
    assert!(!header.header);
    assert_eq!(header.range, None);

    let ranged = ExcelOptions::new().with_range(range);
    assert_eq!(ranged.sheet, None);
    assert!(ranged.header);
    assert_eq!(ranged.range, Some(range));

    // A later call replaces the earlier one, and the rest is untouched.
    let replaced = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(false)
        .with_range(range)
        .with_sheet("Quotes")
        .with_header(true);
    assert_eq!(replaced.sheet.as_deref(), Some("Quotes"));
    assert!(replaced.header);
    assert_eq!(replaced.range, Some(range));
    assert_eq!(replaced.name(), DEFAULT_ROOT_NAME);
    assert_eq!(replaced.field(), None);
}

#[test]
fn cells_are_the_range_when_one_is_stated_and_the_whole_grid_otherwise() {
    assert_eq!(ExcelOptions::new().cells(), CellRange::all());
    assert_eq!(ExcelOptions::new().cells().start(), CellRef::new(0, 0));
    assert_eq!(
        ExcelOptions::new().cells().end(),
        CellRef::new(
            yggdryl::excel::MAX_ROWS - 1,
            yggdryl::excel::MAX_COLUMNS - 1
        )
    );

    let open: CellRange = "A3:F".parse().unwrap();
    let options = ExcelOptions::new().with_range(open);
    assert_eq!(options.cells(), open);
    assert_eq!(options.cells().start(), CellRef::new(2, 0));
    assert_eq!(options.cells().to_string(), "A3:F");
}

#[test]
fn the_workbook_settings_are_part_of_the_value_its_order_and_its_hash() {
    let base = ExcelOptions::new()
        .with_sheet("Trades")
        .with_range("A1:B4".parse().unwrap());
    let same = base.clone();
    assert_eq!(base, same);
    assert_eq!(std_hash(&base), std_hash(&same));
    assert_eq!(base.cmp(&same), Ordering::Equal);

    for changed in [
        base.clone().with_sheet("Quotes"),
        base.clone().with_header(false),
        base.clone().with_range("A1:B5".parse().unwrap()),
        ExcelOptions::new().with_range("A1:B4".parse().unwrap()),
    ] {
        assert_ne!(base, changed);
        assert_ne!(std_hash(&base), std_hash(&changed), "{changed:?}");
        assert_ne!(base.cmp(&changed), Ordering::Equal);
        assert_eq!(base.cmp(&changed), changed.cmp(&base).reverse());
    }
}

// The shared settings, through the one trait every encoding answers.

#[test]
fn declaring_a_field_names_the_root_and_stores_it_non_null() {
    let declared = schema().with_name("trade").with_nullable(true);
    let options = ExcelOptions::new().with_field(declared.clone());
    assert_eq!(options.name(), "trade");
    assert_eq!(options.field(), Some(declared.clone().with_nullable(false)));
    assert!(!options.field.as_ref().unwrap().is_nullable());
    assert_eq!(options.require_field().unwrap().name(), "trade");

    // The workbook settings are not the declaration's and survive it.
    let mut kept = ExcelOptions::new().with_sheet("Trades").with_header(false);
    kept.set_field(declared);
    assert_eq!(kept.sheet.as_deref(), Some("Trades"));
    assert!(!kept.header);
}

#[test]
fn renaming_the_root_renames_the_declared_field_and_taking_it_keeps_the_name() {
    let mut options = ExcelOptions::new().with_field(schema());
    options.set_name("trade".into());
    assert_eq!(options.name(), "trade");
    assert_eq!(options.field().unwrap().name(), "trade");
    assert_eq!(options.field().unwrap().dtype(), schema().dtype());

    let taken = options.take_field().unwrap();
    assert_eq!(taken.name(), "trade");
    assert_eq!(options.field(), None);
    assert_eq!(options.name(), "trade");
    let message = options.require_field().unwrap_err().to_string();
    assert!(message.contains("with_field"), "{message}");
}

#[test]
fn every_bound_the_cadence_the_safety_and_the_level_round_trip() {
    let mut options = ExcelOptions::new();
    options.set_safe(false);
    options.set_batch_row_size(Some(3));
    options.set_batch_byte_size(Some(4_096));
    options.set_max_row_size(Some(10));
    options.set_row_offset(Some(2));
    options.set_max_byte_size(Some(1 << 20));
    options.set_commit_batch_num(Some(5));
    options.set_level(Level::BEST);
    assert!(!options.safe());
    assert_eq!(options.batch_row_size(), Some(3));
    assert_eq!(options.batch_byte_size(), Some(4_096));
    assert_eq!(options.max_row_size(), Some(10));
    assert_eq!(options.row_offset(), Some(2));
    assert_eq!(options.max_byte_size(), Some(1 << 20));
    assert_eq!(options.commit_batch_num(), Some(5));
    assert_eq!(options.level(), Level::BEST);

    // The public fields are the same settings the trait reads.
    assert!(!options.safe);
    assert_eq!(options.batch_row_size, Some(3));
    assert_eq!(options.commit_batch_num, Some(5));

    // The builders spell the same values as the setters.
    let built = ExcelOptions::new()
        .with_safe(false)
        .with_batch_row_size(3)
        .with_batch_byte_size(4_096)
        .with_max_row_size(10)
        .with_row_offset(2)
        .with_max_byte_size(1 << 20)
        .with_commit_batch_num(5)
        .with_level(Level::BEST);
    assert_eq!(built, options);

    // Each clears back to unset.
    options.set_batch_row_size(None);
    options.set_commit_batch_num(None);
    assert_eq!(options.batch_row_size(), None);
    assert_eq!(options.commit_batch_num(), None);
}

#[test]
fn the_properties_are_the_sections_of_one_plan_and_the_workbook_settings_are_not() {
    let range: CellRange = "B2:C".parse().unwrap();
    let options = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(false)
        .with_range(range)
        .with_plan("create trade (id int64 not null) select id where id > 1 limit 5")
        .unwrap();
    assert_eq!(options.name(), "trade");
    assert_eq!(
        options.field().unwrap().dtype(),
        &DataType::from(StructType::from_fields([DataType::Int64.required_field("id")]).unwrap())
    );
    assert_eq!(options.select().to_string(), "id");
    assert_eq!(options.filter().to_string(), "id > 1");
    assert_eq!(options.max_row_size(), Some(5));
    assert_eq!(
        options.plan().to_string(),
        "create trade (id int64 not null) select id where id > 1 limit 5"
    );

    // A plan leaves the sheet, the header and the range as they were.
    assert_eq!(options.sheet.as_deref(), Some("Trades"));
    assert!(!options.header);
    assert_eq!(options.range, Some(range));

    // The composed plan reads back as the same options.
    let respelled = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(false)
        .with_range(range)
        .with_plan(options.plan())
        .unwrap();
    assert_eq!(respelled, options);
}

#[test]
fn a_plan_that_orders_rows_is_refused_and_leaves_the_options_unchanged() {
    let mut options = ExcelOptions::new()
        .with_sheet("Trades")
        .with_filter("id > 1")
        .unwrap();
    let before = options.clone();
    let plan = "select id where id > 2 order by id".parse().unwrap();
    let message = options.set_plan(plan).unwrap_err().to_string();
    assert!(message.contains("$.order_by"), "{message}");
    assert!(
        message.contains("record options hold no `order by` section"),
        "{message}"
    );
    assert_eq!(options, before);
}

// The one `RecordOptions` variant.

#[test]
fn the_xlsx_media_type_names_the_excel_variant() {
    let options = RecordOptions::for_mime_type(&MimeType::XLSX).unwrap();
    assert_eq!(options, RecordOptions::Excel(ExcelOptions::new()));
    assert_eq!(options.mime_type(), MimeType::XLSX);
    assert_eq!(
        MimeType::XLSX.to_string(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );

    let media_type = Url::from_str("file:///book.xlsx").unwrap().media_type();
    assert_eq!(media_type.base(), &MimeType::XLSX);
    assert_eq!(
        RecordOptions::for_media_type(&media_type).unwrap(),
        RecordOptions::Excel(ExcelOptions::new())
    );

    // An encoding no variant covers names the workbook among those that are.
    let message = RecordOptions::for_mime_type(&MimeType::GZIP)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "{message}"
    );
}

#[test]
fn the_options_convert_into_the_excel_variant_unchanged() {
    let options = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(false)
        .with_range("C3:D".parse().unwrap())
        .with_field(schema())
        .with_max_row_size(9);
    let record = RecordOptions::from(options.clone());
    assert_eq!(record.mime_type(), MimeType::XLSX);
    assert_eq!(record.field(), Some(schema()));
    assert_eq!(record.name(), "row");
    assert_eq!(record.max_row_size(), Some(9));
    assert_eq!(record.excel_sheet(), Some("Trades"));
    assert_eq!(record.header(), Some(false));
    assert_eq!(record.excel_range(), Some("C3:D".parse().unwrap()));
    let RecordOptions::Excel(inner) = record else {
        panic!("Excel options convert into the Excel variant");
    };
    assert_eq!(inner, options);
}

#[test]
fn the_variant_reads_and_writes_the_workbook_settings() {
    let mut options = RecordOptions::from(ExcelOptions::new());
    assert_eq!(options.excel_sheet(), None);
    assert_eq!(options.header(), Some(true));
    assert_eq!(options.excel_range(), None);

    let range: CellRange = "B2:C9".parse().unwrap();
    options.set_excel_sheet(Some("Trades")).unwrap();
    options.set_header(false).unwrap();
    options.set_excel_range(Some(range)).unwrap();
    assert_eq!(options.excel_sheet(), Some("Trades"));
    assert_eq!(options.header(), Some(false));
    assert_eq!(options.excel_range(), Some(range));
    assert_eq!(
        options,
        RecordOptions::Excel(
            ExcelOptions::new()
                .with_sheet("Trades")
                .with_header(false)
                .with_range(range)
        )
    );

    // `None` clears the sheet and the range back to the defaults.
    options.set_excel_sheet(None).unwrap();
    options.set_excel_range(None).unwrap();
    options.set_header(true).unwrap();
    assert_eq!(options, RecordOptions::Excel(ExcelOptions::new()));
}

#[test]
fn setting_a_sheet_name_excel_refuses_names_the_rule_and_leaves_the_sheet_as_it_was() {
    let mut options = RecordOptions::from(ExcelOptions::new().with_sheet("Trades"));
    let before = options.clone();
    for (name, reason) in [
        ("", "expected a sheet name, got the empty text"),
        (
            "abcdefghijklmnopqrstuvwxyz012345",
            "expected a sheet name of at most 31 characters, got 32",
        ),
        (
            "Q1/Q2",
            "expected a sheet name without any of \\ / ? * [ ] :, got '/' in \"Q1/Q2\"",
        ),
        ("a:b", "got ':' in \"a:b\""),
        ("[x]", "got '[' in \"[x]\""),
        (
            "'quoted",
            "expected a sheet name that neither opens nor closes with an apostrophe",
        ),
        (
            "history",
            "expected a sheet name other than the reserved `History`",
        ),
    ] {
        let message = options.set_excel_sheet(Some(name)).unwrap_err().to_string();
        assert!(message.contains("$.sheet"), "{name:?}: {message}");
        assert!(message.contains(reason), "{name:?}: {message}");
        assert_eq!(options, before, "{name:?}");
    }
    // Thirty-one characters is the longest name Excel keeps.
    options
        .set_excel_sheet(Some("abcdefghijklmnopqrstuvwxyz01234"))
        .unwrap();
    assert_eq!(
        options.excel_sheet(),
        Some("abcdefghijklmnopqrstuvwxyz01234")
    );
}

#[test]
fn another_encoding_answers_no_workbook_setting_and_refuses_to_set_one() {
    let mut options = RecordOptions::Ipc(IpcOptions::new());
    let before = options.clone();
    assert_eq!(options.excel_sheet(), None);
    assert_eq!(options.header(), None);
    assert_eq!(options.excel_range(), None);

    for (error, path, setting) in [
        (
            options.set_excel_sheet(Some("Trades")).unwrap_err(),
            "$.sheet",
            "a worksheet",
        ),
        (
            options
                .set_excel_range(Some("A1:B2".parse().unwrap()))
                .unwrap_err(),
            "$.range",
            "a cell range",
        ),
    ] {
        let message = error.to_string();
        assert!(message.contains(path), "{message}");
        assert!(
            message.contains(&format!(
                "expected Excel options to set {setting}, got application/vnd.apache.arrow.stream options"
            )),
            "{message}"
        );
    }
    assert_eq!(options, before);
}

#[test]
fn the_encoding_and_every_workbook_setting_are_part_of_the_stable_hash() {
    let base = RecordOptions::from(ExcelOptions::new());
    assert_eq!(
        base.stable_hash(),
        RecordOptions::from(ExcelOptions::new()).stable_hash()
    );
    let changed = [
        RecordOptions::from(ExcelOptions::new().with_sheet("Trades")),
        RecordOptions::from(ExcelOptions::new().with_header(false)),
        RecordOptions::from(ExcelOptions::new().with_range("A1:C3".parse().unwrap())),
        RecordOptions::from(ExcelOptions::new().with_range("A1:C4".parse().unwrap())),
    ];
    for options in &changed {
        assert_ne!(base.stable_hash(), options.stable_hash(), "{options:?}");
    }
    assert_ne!(changed[2].stable_hash(), changed[3].stable_hash());
    // The same shared settings under another encoding hash apart.
    assert_ne!(
        base.stable_hash(),
        RecordOptions::Ipc(IpcOptions::new()).stable_hash()
    );
}

// The record doors, reading the settings.

#[test]
fn a_workbook_handle_answers_the_default_excel_options() {
    assert_eq!(
        xlsx().record_options().unwrap(),
        RecordOptions::Excel(ExcelOptions::new())
    );
    let retained = Excel::new(Buffer::new()).with_sheet("Trades");
    assert_eq!(
        retained.record_options().unwrap(),
        RecordOptions::Excel(ExcelOptions::new().with_sheet("Trades"))
    );
}

#[test]
fn a_workbook_handle_refuses_the_options_of_another_encoding_naming_the_encoding() {
    let expected = "invalid record value at $.encoding: expected Excel record options, got application/vnd.apache.arrow.stream";
    let ipc = RecordOptions::Ipc(IpcOptions::new()).with_field(schema());

    let mut handle = Excel::new(xlsx());
    let read = handle.read_arrow_field(&ipc).unwrap_err().to_string();
    assert!(read.contains(expected), "{read}");
    let overwrite = handle
        .overwrite_arrow_batch(batch(), &ipc)
        .unwrap_err()
        .to_string();
    assert!(overwrite.contains(expected), "{overwrite}");
    let append = handle
        .append_arrow_batch(batch(), &ipc)
        .unwrap_err()
        .to_string();
    assert!(append.contains(expected), "{append}");
    // Nothing was written by a refused call.
    assert_eq!(handle.size(), 0);
}

#[test]
fn without_a_sheet_a_write_names_the_default_sheet_and_a_read_takes_the_first_worksheet() {
    let handle = written(ExcelOptions::new());
    assert_eq!(opened(&handle).sheet_names(), vec![DEFAULT_SHEET_NAME]);
    assert_eq!(DEFAULT_SHEET_NAME, "Sheet1");

    let mut workbook = Workbook::new();
    let alpha = workbook.add_sheet("Alpha").unwrap();
    alpha.set_cell(cell("A1"), "id").unwrap();
    alpha.set_cell(cell("A2"), 7.0).unwrap();
    let beta = workbook.add_sheet("Beta").unwrap();
    beta.set_cell(cell("A1"), "id").unwrap();
    beta.set_cell(cell("A2"), 8.0).unwrap();
    beta.set_cell(cell("A3"), 9.0).unwrap();
    let handle = holding(&workbook);
    let declared = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let first = RecordOptions::from(ExcelOptions::new()).with_field(declared.clone());
    assert_eq!(ids(handle.read_arrow_reader(&first).unwrap()), vec![7]);
    let named = RecordOptions::from(ExcelOptions::new().with_sheet("Beta")).with_field(declared);
    assert_eq!(ids(handle.read_arrow_reader(&named).unwrap()), vec![8, 9]);
}

#[test]
fn the_sheet_a_read_or_write_addresses_is_compared_without_case() {
    let mut handle = written(ExcelOptions::new().with_sheet("Trades"));
    assert_eq!(opened(&handle).sheet_names(), vec!["Trades"]);

    let shouted =
        RecordOptions::from(ExcelOptions::new().with_sheet("TRADES")).with_field(schema());
    assert_eq!(
        ids(handle.read_arrow_reader(&shouted).unwrap()),
        vec![1, 2, 3]
    );

    // A write under another case replaces that sheet and keeps its name.
    let two = RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![5, 6])),
            Arc::new(StringArray::from(vec![Some("IBM"), None])),
        ],
    )
    .unwrap();
    let lower = RecordOptions::from(ExcelOptions::new().with_sheet("trades")).with_field(schema());
    handle.overwrite_arrow_batch(two, &lower).unwrap();
    assert_eq!(opened(&handle).sheet_names(), vec!["Trades"]);
    assert_eq!(ids(handle.read_arrow_reader(&shouted).unwrap()), vec![5, 6]);
}

#[test]
fn a_sheet_name_excel_refuses_is_refused_by_the_write_that_would_add_it() {
    let mut handle = xlsx();
    let options =
        RecordOptions::from(ExcelOptions::new().with_sheet("History")).with_field(schema());
    let message = handle
        .overwrite_arrow_batch(batch(), &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.sheet"), "{message}");
    assert!(
        message.contains("expected a sheet name other than the reserved `History`"),
        "{message}"
    );
    assert_eq!(handle.size(), 0);
}

#[test]
fn without_a_header_the_first_row_is_data_and_the_columns_are_named_by_their_letters() {
    let handle = written(ExcelOptions::new().with_header(false));
    let sheet = opened(&handle);
    let sheet = sheet.sheet(DEFAULT_SHEET_NAME).unwrap();
    assert_eq!(sheet.scalar(cell("B1")), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(cell("B2")), Scalar::Null);
    assert_eq!(sheet.scalar(cell("B3")), Scalar::from("MSFT"));

    let options = RecordOptions::from(ExcelOptions::new().with_header(false));
    let field = handle.read_arrow_field(&options).unwrap();
    let names: Vec<&str> = field.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["A", "B"]);
    assert_eq!(handle.row_size().unwrap(), 2);
    let headerless = RecordOptions::from(ExcelOptions::new().with_header(false));
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&headerless)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 3);
}

#[test]
fn a_write_anchors_its_rows_at_the_top_left_cell_of_the_range() {
    let range: CellRange = "C3:D".parse().unwrap();
    let handle = written(ExcelOptions::new().with_range(range));
    let workbook = opened(&handle);
    let sheet = workbook.sheet(DEFAULT_SHEET_NAME).unwrap();
    assert_eq!(sheet.scalar(cell("A1")), Scalar::Null);
    assert_eq!(sheet.scalar(cell("C3")), Scalar::from("id"));
    assert_eq!(sheet.scalar(cell("D3")), Scalar::from("symbol"));
    assert_eq!(sheet.scalar(cell("C4")).into_json().unwrap(), "1.0");
    assert_eq!(sheet.scalar(cell("D4")), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(cell("D6")), Scalar::from("MSFT"));
    assert_eq!(sheet.dimension(), Some("C3:D6".parse().unwrap()));

    // Read under the same range, the rows come back as written.
    let options = RecordOptions::from(ExcelOptions::new().with_range(range)).with_field(schema());
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(batches, vec![batch()]);
}

#[test]
fn a_read_takes_the_cells_inside_the_range_its_first_row_naming_the_columns() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Report").unwrap();
    sheet
        .set_cell(cell("A1"), "a title above the table")
        .unwrap();
    sheet.set_cell(cell("B3"), "id").unwrap();
    sheet.set_cell(cell("C3"), "symbol").unwrap();
    sheet.set_cell(cell("E3"), "beside the table").unwrap();
    sheet.set_cell(cell("B4"), 1.0).unwrap();
    sheet.set_cell(cell("C4"), "AAPL").unwrap();
    sheet.set_cell(cell("B5"), 2.0).unwrap();
    sheet.set_cell(cell("C5"), "MSFT").unwrap();
    let handle = holding(&workbook);

    let options = RecordOptions::from(ExcelOptions::new().with_range("B3:C".parse().unwrap()));
    let field = handle.read_arrow_field(&options).unwrap();
    let names: Vec<&str> = field.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["id", "symbol"]);
    let declared = options.clone().with_field(schema());
    assert_eq!(
        ids(handle.read_arrow_reader(&declared).unwrap()),
        vec![1, 2]
    );

    let excel =
        Excel::new(handle).with_options(ExcelOptions::new().with_range("B3:C".parse().unwrap()));
    assert_eq!(excel.row_size().unwrap(), 2);
    assert_eq!(excel.column_size().unwrap(), 2);
}

#[test]
fn a_nullable_column_takes_a_cell_it_cannot_read_as_null_only_while_safe() {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_sheet("Sheet1").unwrap();
    sheet.set_cell(cell("A1"), "qty").unwrap();
    sheet.set_cell(cell("A2"), 1.0).unwrap();
    sheet.set_cell(cell("A3"), "many").unwrap();
    let handle = holding(&workbook);
    let declared = StructType::from_fields([DataType::Int64.nullable_field("qty")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let safe = RecordOptions::from(ExcelOptions::new()).with_field(declared.clone());
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&safe)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(batches.len(), 1);
    let qty = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(qty.iter().collect::<Vec<_>>(), vec![Some(1), None]);

    let strict = RecordOptions::from(ExcelOptions::new().with_safe(false)).with_field(declared);
    let message = match handle.read_arrow_reader(&strict) {
        Err(error) => error.to_string(),
        Ok(reader) => reader
            .map(|batch| batch.map(|_| ()))
            .collect::<Result<Vec<()>, _>>()
            .unwrap_err()
            .to_string(),
    };
    assert!(message.contains("Sheet1!A3"), "{message}");
    assert!(message.contains("expected int64, got string"), "{message}");
}

#[test]
fn the_batch_size_and_the_row_bounds_reach_the_rows_a_read_yields() {
    let handle = written(ExcelOptions::new());

    let batched =
        RecordOptions::from(ExcelOptions::new().with_batch_row_size(2)).with_field(schema());
    let sizes: Vec<usize> = handle
        .read_arrow_reader(&batched)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .collect();
    assert_eq!(sizes, vec![2, 1]);

    let bounded = RecordOptions::from(ExcelOptions::new().with_row_offset(1).with_max_row_size(1))
        .with_field(schema());
    assert_eq!(ids(handle.read_arrow_reader(&bounded).unwrap()), vec![2]);
}

#[test]
fn a_write_published_in_commits_holds_every_row_in_order() {
    let handle = written(ExcelOptions::new().with_commit_batch_num(2));
    let options = RecordOptions::from(ExcelOptions::new()).with_field(schema());
    let batches: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        ids(yggdryl::arrow::batch_reader(
            schema().into_arrow_schema().unwrap(),
            batches.clone()
        )),
        vec![1, 2, 3]
    );
    let symbols: Vec<Option<String>> = batches
        .iter()
        .flat_map(|batch| {
            batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .map(|value| value.map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        symbols,
        vec![Some("AAPL".to_owned()), None, Some("MSFT".to_owned())]
    );
}
