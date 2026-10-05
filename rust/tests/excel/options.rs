//! `rust/src/excel/options.rs`: the settings a workbook read or write takes -
//! the shared record settings, the sheet, the header row and the range - and
//! the one `RecordOptions` variant they are.

use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use yggdryl::RecordHeader;

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
    assert_eq!(options.sheet(), None);
    assert_eq!(options.header, RecordHeader::Source);
    assert_eq!(options.range(), None);
    assert_eq!(options.cells().unwrap(), CellRange::all());
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
    assert_eq!(sheet.sheet(), Some("Trades"));
    assert_eq!(sheet.header, RecordHeader::Source);
    assert_eq!(sheet.range(), None);

    let header = ExcelOptions::new().with_header(RecordHeader::None);
    assert_eq!(header.sheet(), None);
    assert_eq!(header.header, RecordHeader::None);
    assert_eq!(header.range(), None);

    let ranged = ExcelOptions::new().with_range(range);
    assert_eq!(ranged.sheet(), None);
    assert_eq!(ranged.header, RecordHeader::Source);
    assert_eq!(ranged.range(), Some(range));

    // A later call replaces the earlier one, and the rest is untouched.
    let replaced = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(RecordHeader::None)
        .with_range(range)
        .with_sheet("Quotes")
        .with_header(RecordHeader::Source);
    assert_eq!(replaced.sheet(), Some("Quotes"));
    assert_eq!(replaced.header, RecordHeader::Source);
    assert_eq!(replaced.range(), Some(range));
    assert_eq!(replaced.name(), DEFAULT_ROOT_NAME);
    assert_eq!(replaced.field(), None);
}

#[test]
fn cells_are_the_range_when_one_is_stated_and_the_whole_grid_otherwise() {
    assert_eq!(ExcelOptions::new().cells().unwrap(), CellRange::all());
    assert_eq!(
        ExcelOptions::new().cells().unwrap().start(),
        CellRef::new(0, 0)
    );
    assert_eq!(
        ExcelOptions::new().cells().unwrap().end(),
        CellRef::new(
            yggdryl::excel::MAX_ROWS - 1,
            yggdryl::excel::MAX_COLUMNS - 1
        )
    );

    let open: CellRange = "A3:F".parse().unwrap();
    let options = ExcelOptions::new().with_range(open);
    assert_eq!(options.cells().unwrap(), open);
    assert_eq!(options.cells().unwrap().start(), CellRef::new(2, 0));
    assert_eq!(options.cells().unwrap().to_string(), "A3:F");
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
        base.clone().with_header(RecordHeader::None),
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
    let mut kept = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(RecordHeader::None);
    kept.set_field(declared);
    assert_eq!(kept.sheet(), Some("Trades"));
    assert_eq!(kept.header, RecordHeader::None);
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
        .with_header(RecordHeader::None)
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
    assert_eq!(options.sheet(), Some("Trades"));
    assert_eq!(options.header, RecordHeader::None);
    assert_eq!(options.range(), Some(range));

    // The composed plan reads back as the same options.
    let respelled = ExcelOptions::new()
        .with_sheet("Trades")
        .with_header(RecordHeader::None)
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
        .with_header(RecordHeader::None)
        .with_range("C3:D".parse().unwrap())
        .with_field(schema())
        .with_max_row_size(9);
    let record = RecordOptions::from(options.clone());
    assert_eq!(record.mime_type(), MimeType::XLSX);
    assert_eq!(record.field(), Some(schema()));
    assert_eq!(record.name(), "row");
    assert_eq!(record.max_row_size(), Some(9));
    assert_eq!(record.excel_sheet(), Some("Trades"));
    assert_eq!(record.header(), Some(RecordHeader::None));
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
    assert_eq!(options.header(), Some(RecordHeader::Source));
    assert_eq!(options.excel_range(), None);

    let range: CellRange = "B2:C9".parse().unwrap();
    options.set_excel_sheet(Some("Trades")).unwrap();
    options.set_header(RecordHeader::None).unwrap();
    options.set_excel_range(Some(range)).unwrap();
    assert_eq!(options.excel_sheet(), Some("Trades"));
    assert_eq!(options.header(), Some(RecordHeader::None));
    assert_eq!(options.excel_range(), Some(range));
    assert_eq!(
        options,
        RecordOptions::Excel(
            ExcelOptions::new()
                .with_sheet("Trades")
                .with_header(RecordHeader::None)
                .with_range(range)
        )
    );

    // `None` clears the sheet and the range back to the defaults.
    options.set_excel_sheet(None).unwrap();
    options.set_excel_range(None).unwrap();
    options.set_header(RecordHeader::Source).unwrap();
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

    for (error, path, encoding, setting) in [
        (
            options.set_excel_sheet(Some("Trades")).unwrap_err(),
            "$.sheet",
            "Excel",
            "a worksheet",
        ),
        (
            options.set_header(RecordHeader::None).unwrap_err(),
            "$.header",
            "CSV or Excel",
            "a header",
        ),
        (
            options
                .set_excel_range(Some("A1:B2".parse().unwrap()))
                .unwrap_err(),
            "$.range",
            "Excel",
            "a cell range",
        ),
    ] {
        let message = error.to_string();
        assert!(message.contains(path), "{message}");
        assert!(
            message.contains(&format!(
                "expected {encoding} options to set {setting}, got application/vnd.apache.arrow.stream options"
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
        RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None)),
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
fn a_directly_constructed_invalid_sheet_name_is_refused_before_declared_read() {
    let handle = xlsx();
    let options = RecordOptions::from(ExcelOptions::new().with_sheet("Q1/Q2")).with_field(schema());
    let error = handle.read_arrow_field(&options).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located sheet-name refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.sheet");
    assert!(reason.contains("Q1/Q2"), "{reason}");
    assert_eq!(handle.size(), 0);
}

#[test]
fn a_directly_constructed_invalid_range_is_refused_before_declared_read() {
    let handle = xlsx();
    let outside = CellRange::new(CellRef::new(1_048_576, 0), CellRef::new(1_048_576, 0));
    let options = RecordOptions::from(ExcelOptions::new().with_range(outside)).with_field(schema());
    let error = handle.read_arrow_field(&options).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located range refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.range");
    assert!(reason.contains("1048576"), "{reason}");
    assert_eq!(handle.size(), 0);
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
    let handle = written(ExcelOptions::new().with_header(RecordHeader::None));
    let sheet = opened(&handle);
    let sheet = sheet.sheet(DEFAULT_SHEET_NAME).unwrap();
    assert_eq!(sheet.scalar(cell("B1")), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(cell("B2")), Scalar::Null);
    assert_eq!(sheet.scalar(cell("B3")), Scalar::from("MSFT"));

    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None));
    let field = handle.read_arrow_field(&options).unwrap();
    let names: Vec<&str> = field.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["A", "B"]);
    assert_eq!(handle.row_size().unwrap(), 2);
    let headerless = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::None));
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

#[test]
fn named_table_selection_is_one_fact_replaced_by_each_selection_builder() {
    use yggdryl::excel::ExcelSelection;
    let range = "B2:C4".parse().unwrap();
    let table = ExcelOptions::new()
        .with_sheet("Data")
        .with_range(range)
        .with_table("Names");
    assert_eq!(
        table.selection,
        ExcelSelection::Table {
            name: "Names".into()
        }
    );
    assert_eq!(table.sheet(), None);
    assert_eq!(table.range(), None);
    assert!(table.cells().is_err());
    assert_eq!(
        table.clone().with_sheet("Other").selection,
        ExcelSelection::Worksheet {
            sheet: Some("Other".into()),
            range: None
        }
    );
    assert_eq!(
        table.with_range(range).selection,
        ExcelSelection::Worksheet {
            sheet: None,
            range: Some(range)
        }
    );
    let mut options = RecordOptions::from(ExcelOptions::new().with_table("Names"));
    assert_eq!(options.excel_table(), Some("Names"));
    let before = options.clone();
    assert!(options.set_excel_table(Some("")).is_err());
    assert_eq!(options, before);
    options.set_excel_table(None).unwrap();
    assert_eq!(options, RecordOptions::from(ExcelOptions::new()));
}

fn selection_scalar(json: &str) -> Scalar {
    yggdryl::from_json_scalar(json).unwrap()
}

#[test]
fn selection_scalar_constructor_accepts_null_and_both_worksheet_properties() {
    use yggdryl::excel::ExcelSelection;
    let default = ExcelSelection::Worksheet {
        sheet: None,
        range: None,
    };
    assert_eq!(ExcelSelection::from_scalar(&Scalar::Null).unwrap(), default);
    assert_eq!(
        ExcelSelection::from_scalar(&selection_scalar("{}")).unwrap(),
        default
    );
    let expected = ExcelSelection::Worksheet {
        sheet: Some("Data".into()),
        range: Some("B2:C4".parse().unwrap()),
    };
    for json in [
        r#"{"sheet":"Data","range":"B2:C4"}"#,
        r#"{"range":"B2:C4","sheet":"Data"}"#,
    ] {
        assert_eq!(
            ExcelSelection::from_scalar(&selection_scalar(json)).unwrap(),
            expected
        );
    }
    assert_eq!(
        ExcelSelection::from_scalar(&selection_scalar(r#"{"table":"Orders"}"#)).unwrap(),
        ExcelSelection::Table {
            name: "Orders".into()
        }
    );
}

#[test]
fn selection_scalar_update_preserves_absent_members_and_clears_only_active_ones() {
    use yggdryl::excel::ExcelSelection;
    let range = "B2:C4".parse().unwrap();
    let mut selected = ExcelSelection::Worksheet {
        sheet: Some("Data".into()),
        range: Some(range),
    };
    let original = selected.clone();
    selected.set_from_scalar(&selection_scalar("{}")).unwrap();
    assert_eq!(selected, original);
    selected
        .set_from_scalar(&selection_scalar(r#"{"sheet":"Other"}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: Some("Other".into()),
            range: Some(range),
        }
    );
    selected
        .set_from_scalar(&selection_scalar(r#"{"table":null}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: Some("Other".into()),
            range: Some(range),
        }
    );
    selected
        .set_from_scalar(&selection_scalar(r#"{"sheet":null}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: None,
            range: Some(range)
        }
    );
    selected
        .set_from_scalar(&selection_scalar(r#"{"table":"Orders"}"#))
        .unwrap();
    selected.set_from_scalar(&selection_scalar("{}")).unwrap();
    selected
        .set_from_scalar(&selection_scalar(r#"{"range":null}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Table {
            name: "Orders".into()
        }
    );
    selected
        .set_from_scalar(&selection_scalar(r#"{"range":"C3:D8"}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: None,
            range: Some("C3:D8".parse().unwrap()),
        }
    );
    selected
        .set_from_scalar(&selection_scalar(r#"{"table":"Orders"}"#))
        .unwrap();
    selected
        .set_from_scalar(&selection_scalar(r#"{"table":null}"#))
        .unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: None,
            range: None
        }
    );
    selected.set_from_scalar(&Scalar::Null).unwrap();
    assert_eq!(
        selected,
        ExcelSelection::Worksheet {
            sheet: None,
            range: None
        }
    );
}

#[test]
fn selection_scalar_conflicts_and_bad_members_are_located_and_atomic() {
    use yggdryl::excel::ExcelSelection;
    let original = ExcelSelection::Table {
        name: "Orders".into(),
    };
    for (json, paths) in [
        (
            r#"{"table":"Orders","sheet":"Data"}"#,
            vec!["$.selection.table", "$.selection.sheet"],
        ),
        (
            r#"{"table":"Orders","range":"A1:B2"}"#,
            vec!["$.selection.table", "$.selection.range"],
        ),
        (
            r#"{"table":"Orders","sheet":"Data","range":"A1:B2"}"#,
            vec![
                "$.selection.table",
                "$.selection.sheet",
                "$.selection.range",
            ],
        ),
        (r#"{"sheet":"History"}"#, vec!["$.selection.sheet"]),
        (r#"{"range":"bad"}"#, vec!["$.selection.range"]),
        (r#"{"table":""}"#, vec!["$.selection.table"]),
        (r#"{"sheet":7}"#, vec!["$.selection.sheet"]),
        (r#"{"range":false}"#, vec!["$.selection.range"]),
        (r#"{"detect":true}"#, vec!["$.selection.detect"]),
    ] {
        let mut selected = original.clone();
        let message = selected
            .set_from_scalar(&selection_scalar(json))
            .unwrap_err()
            .to_string();
        for path in paths {
            assert!(message.contains(path), "{json}: {message}");
        }
        assert_eq!(selected, original, "{json}");
    }
    for json in [r#""Data""#, "[]", "true"] {
        let message = ExcelSelection::from_scalar(&selection_scalar(json))
            .unwrap_err()
            .to_string();
        assert!(message.contains("$.selection"), "{json}: {message}");
    }
}

#[test]
fn header_scalar_intake_resolves_source_none_and_explicit_null() {
    for (value, expected) in [
        (Scalar::from("source"), RecordHeader::Source),
        (Scalar::from("none"), RecordHeader::None),
        (Scalar::from("infer"), RecordHeader::Infer),
        (Scalar::Null, RecordHeader::None),
        (Scalar::from(true), RecordHeader::Source),
        (Scalar::from(false), RecordHeader::None),
        (Scalar::from(1_i64), RecordHeader::Rows(1)),
        (Scalar::from(2_i64), RecordHeader::Rows(2)),
    ] {
        assert_eq!(RecordHeader::from_scalar(&value).unwrap(), expected);
    }
}

#[test]
fn header_scalar_intake_refuses_other_spellings_and_kinds_at_header() {
    for literal in ["[]", "{}", "\"\"", "\"Source\"", "\"rows\""] {
        let value = yggdryl::from_json_scalar(literal).unwrap();
        let error = RecordHeader::from_scalar(&value).unwrap_err();
        let yggdryl::Error::InvalidRecord { path, reason } = error else {
            panic!("expected a located header refusal for {literal}, got {error}");
        };
        assert_eq!(path.as_str(), "$.header");
        assert!(
            reason.contains("source") && reason.contains("none") && reason.contains("null"),
            "{reason}"
        );
    }
}

#[test]
fn header_scalar_intake_refuses_nonpositive_or_out_of_u32_rows() {
    for count in [0_i64, 4_294_967_296_i64] {
        let error = RecordHeader::from_scalar(&Scalar::from(count)).unwrap_err();
        let yggdryl::Error::InvalidRecord { path, reason } = error else {
            panic!("expected a located Rows refusal, got {error}");
        };
        assert_eq!(path.as_str(), "$.header");
        assert!(reason.contains("1 to 4294967295"), "{reason}");
    }
    assert_eq!(
        RecordHeader::from_scalar(&Scalar::from(1_048_577_i64)).unwrap(),
        RecordHeader::Rows(1_048_577)
    );
}

#[test]
fn excel_grid_bound_refuses_a_typed_header_before_source_io() {
    let handle = xlsx();
    let options =
        RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Rows(1_048_577)));
    let error = handle.read_arrow_field(&options).unwrap_err().to_string();
    assert!(
        error.contains("$.header") && error.contains("1 to 1048576"),
        "{error}"
    );
}

#[test]
fn infer_write_refuses_with_explicit_policy_choices() {
    let mut handle = xlsx();
    let options = RecordOptions::from(ExcelOptions::new().with_header(RecordHeader::Infer))
        .with_field(schema());
    let error = handle
        .overwrite_arrow_batch(batch(), &options)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.header")
            && error.contains("read-only")
            && error.contains("Source")
            && error.contains("Rows(n)"),
        "{error}"
    );
    assert_eq!(handle.size(), 0);
}
