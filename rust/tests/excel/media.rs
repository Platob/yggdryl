//! `rust/src/excel/media.rs`: the `.xlsx` record medium - the `Excel` wrapper
//! over any byte handle, its held workbook between `open` and `close`, and the
//! free `read_field`, `read_batch_reader` and `overwrite_arrow_reader` doors.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::arrow::BatchReader;
use yggdryl::excel::{
    CellRange, Excel, ExcelOptions, Workbook, overwrite_arrow_reader, read_batch_reader, read_field,
};
use yggdryl::expression::Selector;
use yggdryl::holder::counted::{Calls, Counted};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::local::LocalFolder;
use yggdryl::media::{IORecordOptions, Media, RecordOptions};
use yggdryl::{
    DataType, Error, Field, IOBase, IOKind, IOMedia, MediaType, MimeType, Scalar, Serie,
    StructType, Url,
};

/// What a dimension read asks of the handle beneath the wrapper while no
/// workbook is held: whether the handle is a container - whose leaves'
/// workbooks answer instead, one by one - then one fresh open of the
/// workbook, which is one streamed copy of the package into a buffer of its
/// own and the three metadata questions that decide it cannot be reopened in
/// place.
const DIMENSION_READ: &str =
    "pstream_bytes=1 bound_location=3 media_type=1 is_container=1 parent=1";

/// The trades table: a required `id` and a nullable `symbol`.
fn trades() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .unwrap(),
    )
    .required_field("row")
}

/// One batch of the trades table.
fn batch(ids: &[i64], symbols: &[Option<&str>]) -> RecordBatch {
    RecordBatch::try_new(
        trades().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(symbols.to_vec())),
        ],
    )
    .unwrap()
}

/// A reader over one batch of the trades table.
fn reader(ids: &[i64], symbols: &[Option<&str>]) -> BatchReader {
    yggdryl::arrow::batch_reader(trades().into_arrow_schema().unwrap(), [batch(ids, symbols)])
}

/// An empty buffer declaring a workbook.
fn xlsx() -> Buffer {
    Buffer::new().with_media_type(MimeType::XLSX.into())
}

/// A buffer declaring a workbook and holding `bytes`.
fn stored(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(MimeType::XLSX.into())
}

/// A holder whose media type comes from `name`.
fn named(name: &str) -> Holder {
    Holder::buffer(Buffer::new().with_media_type(MediaType::from_file_name(name)))
}

/// Every row `media` reads under `options`, each rendered as JSON under the
/// field the same options read.
fn rows(media: &dyn IOMedia, options: &RecordOptions) -> Vec<String> {
    let field = media.read_arrow_field(options).unwrap();
    let mut rendered = Vec::new();
    for batch in media.read_arrow_reader(options).unwrap() {
        let serie =
            Serie::from_arrow_batch(Some(&field), &batch.unwrap(), Default::default()).unwrap();
        for index in 0..serie.len() {
            rendered.push(serie.scalar(index).unwrap().into_json().unwrap());
        }
    }
    rendered
}

/// Every row a free read yields under `field`, rendered as JSON.
fn free_rows(reader: BatchReader, field: &Field) -> Vec<String> {
    let mut rendered = Vec::new();
    for batch in reader {
        let serie =
            Serie::from_arrow_batch(Some(field), &batch.unwrap(), Default::default()).unwrap();
        for index in 0..serie.len() {
            rendered.push(serie.scalar(index).unwrap().into_json().unwrap());
        }
    }
    rendered
}

/// The path and the reason of an invalid record.
fn refusal(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other:?}"),
    }
}

/// Run `operation` and assert what it asked of the handle, as the tally
/// renders it.
fn costs(what: &str, calls: &Arc<Calls>, expected: &str, operation: impl FnOnce()) {
    calls.reset();
    operation();
    assert_eq!(calls.snapshot().to_string(), expected, "{what}");
}

/// The root a handle holding no rows states: `row`, with no columns.
fn empty_root() -> Field {
    DataType::from(StructType::from_fields(std::iter::empty::<Field>()).unwrap())
        .required_field("row")
}

// The wrapper.

#[test]
fn the_wrapper_retains_its_options_and_hands_back_its_handle() {
    let media = Excel::new(xlsx());
    assert_eq!(media.options(), &ExcelOptions::new());
    assert_eq!(media.options().sheet, None);
    assert!(media.options().header);
    assert_eq!(media.options().range, None);

    let media = media.with_field(trades().with_name("trade"));
    assert_eq!(media.options().field, Some(trades().with_name("trade")));
    assert_eq!(media.options().name.as_str(), "trade");

    // A complete configuration replaces the declared field too.
    let configured = ExcelOptions::new().with_header(false);
    let mut media = media.with_options(configured.clone());
    assert_eq!(media.options(), &configured);
    assert_eq!(media.options().field, None);

    let media_sheet = Excel::new(xlsx()).with_sheet("Trades");
    assert_eq!(media_sheet.options().sheet.as_deref(), Some("Trades"));

    media.options_mut().header = true;
    media.options_mut().set_field(trades());
    assert!(media.options().header);
    assert_eq!(media.options().field(), Some(trades()));

    media.handle_mut().write_all_bytes(b"PK").unwrap();
    assert_eq!(media.handle().size(), 2);
    assert_eq!(media.handle().as_slice(), b"PK");
    let held: Buffer = media.into_handle();
    assert_eq!(held.into_bytes(), b"PK");
}

#[test]
fn record_options_are_the_excel_options_the_wrapper_holds() {
    let range: CellRange = "B2:D".parse().unwrap();
    let media = Excel::new(xlsx())
        .with_options(ExcelOptions::new().with_range(range).with_header(false))
        .with_sheet("Trades")
        .with_field(trades());
    let options = media.record_options().unwrap();
    let Some(excel) = options.settings::<ExcelOptions>() else {
        panic!("expected Excel options, got {options:?}");
    };
    assert_eq!(excel, media.options());
    assert_eq!(options.field(), Some(trades()));
    assert_eq!(options.mime_type(), MimeType::XLSX);
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::sheet),
        Some("Trades")
    );
    assert_eq!(options.header(), Some(false));
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range),
        Some(range)
    );
}

#[test]
fn record_options_of_another_encoding_are_refused_naming_both() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    let before = media.read_all_bytes().unwrap();

    let ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).unwrap();
    let expected = (
        "$.encoding".to_owned(),
        "expected Excel record options, got application/vnd.apache.arrow.stream".to_owned(),
    );
    assert_eq!(refusal(media.read_arrow_field(&ipc).unwrap_err()), expected);
    assert_eq!(
        refusal(
            media
                .overwrite_arrow_reader(reader(&[2], &[None]), &ipc)
                .unwrap_err()
        ),
        expected
    );
    assert_eq!(
        refusal(
            media
                .append_arrow_reader(reader(&[2], &[None]), &ipc)
                .unwrap_err()
        ),
        expected
    );
    let mut keyed = ipc.clone();
    keyed.set_merge_by(Selector::from_columns(["id"]));
    assert_eq!(
        refusal(
            media
                .merge_arrow_reader(reader(&[2], &[None]), &keyed)
                .unwrap_err()
        ),
        expected
    );
    // The hidden hook a resumable write publishes through refuses the same.
    assert_eq!(
        refusal(
            media
                .overwrite_prepared_serie(
                    yggdryl::StreamChunkedSerie::from_arrow_reader(
                        None,
                        reader(&[2], &[None]),
                        yggdryl::ArrowCastOptions::new()
                    )
                    .unwrap(),
                    &ipc
                )
                .unwrap_err()
        ),
        expected
    );
    assert_eq!(media.read_all_bytes().unwrap(), before);
}

#[test]
fn an_empty_handle_states_a_root_with_no_columns_and_holds_no_rows() {
    let media = Excel::new(xlsx());
    assert_eq!(media.size(), 0);
    let options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_field(&options).unwrap(), empty_root());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 0);
    let batches = media.read_arrow_reader(&options).unwrap();
    assert!(batches.schema().fields().is_empty());
    assert_eq!(batches.count(), 0);
}

#[test]
fn the_byte_surface_mirrors_the_handle() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), None]), &options)
        .unwrap();

    assert!(media.size() > 0);
    assert_eq!(media.size(), media.handle().size());
    assert_eq!(
        media.read_all_bytes().unwrap(),
        media.handle().read_all_bytes().unwrap()
    );
    // A package is a ZIP archive, opening with a local file header.
    assert_eq!(
        media.read_range_bytes(0, 4).unwrap(),
        b"PK\x03\x04".to_vec()
    );
    assert_eq!(media.media_type(), media.handle().media_type());
    assert_eq!(media.media_type().base(), &MimeType::XLSX);
    assert_eq!(media.url(), media.handle().url());
    assert_eq!(media.kind(), IOKind::Memory);
    assert!(media.is_tabular(), "a workbook is a record encoding");
    assert!(
        !media.is_atomic(),
        "a record encoding is never one whole value"
    );
}

#[test]
fn rows_written_through_the_wrapper_read_back_under_the_inferred_or_the_declared_field() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), None]), &options)
        .unwrap();

    // A number cell is a float64 and a column that lacks a cell is nullable.
    let inferred = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .unwrap(),
    )
    .required_field("row");
    assert_eq!(media.read_arrow_field(&options).unwrap(), inferred);
    assert_eq!(rows(&media, &options), ["[1.0,\"AAPL\"]", "[2.0,null]"]);
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(media.column_size().unwrap(), 2);

    let declared = options.clone().with_field(trades());
    assert_eq!(rows(&media, &declared), ["[1,\"AAPL\"]", "[2,null]"]);

    // The rows are the default sheet's, under a header row.
    let workbook = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Sheet1"]);
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from("id"));
    assert_eq!(sheet.scalar("B1".parse().unwrap()), Scalar::from("symbol"));
    assert_eq!(sheet.scalar("A2".parse().unwrap()), Scalar::from(1.0));
    assert_eq!(sheet.scalar("B3".parse().unwrap()), Scalar::Null);
}

// The held workbook.

#[test]
fn open_holds_the_workbook_so_the_schema_and_the_row_count_cost_the_package_once() {
    let counted = Counted::new(xlsx());
    let calls = Arc::clone(counted.calls());
    let mut media = Excel::new(counted);
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), Some("MSFT")]), &options)
        .unwrap();

    // Closed, every ask opens the package afresh.
    costs("a closed schema", &calls, DIMENSION_READ, || {
        assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);
    });
    costs("a closed schema again", &calls, DIMENSION_READ, || {
        assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);
    });

    costs("open", &calls, "open=1", || media.open().unwrap());
    assert!(media.opened());
    costs(
        "the first schema while open",
        &calls,
        DIMENSION_READ,
        || {
            assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);
        },
    );
    costs(
        "the schema, rows and width while open",
        &calls,
        "none",
        || {
            assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);
            assert_eq!(media.row_size().unwrap(), 2);
            assert_eq!(media.column_size().unwrap(), 2);
        },
    );
    // Another root name and another header reading are answered from the
    // same held workbook: without a header the header row is a data row,
    // so its text disagrees with the numbers under it.
    let mut renamed = options.clone();
    renamed.set_name("trade".into());
    let mut headless = options.clone();
    headless.set_header(false).unwrap();
    costs("another reading while open", &calls, "none", || {
        assert_eq!(media.read_arrow_field(&renamed).unwrap().name(), "trade");
        assert_eq!(
            refusal(media.read_arrow_field(&headless).unwrap_err()),
            (
                "Sheet1!A2".to_owned(),
                "expected utf8 like the column's first value, got float64; \
                 declare a field to read the column as one datatype"
                    .to_owned()
            )
        );
    });
    // The rows themselves stream from a package opened for the read, which
    // asks whether the handle is a container - forwarded as that question,
    // never derived from the kind, which on a store is a request.
    costs(
        "the rows while open",
        &calls,
        "pstream_bytes=1 bound_location=3 media_type=1 is_container=1 parent=1",
        || assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 1),
    );

    costs("close", &calls, "close=1", || media.close().unwrap());
    assert!(!media.opened());
    costs("a schema after close", &calls, DIMENSION_READ, || {
        assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);
    });
}

#[test]
fn a_declared_field_answers_the_schema_and_the_width_without_a_read() {
    let counted = Counted::new(xlsx());
    let calls = Arc::clone(counted.calls());
    let media = Excel::new(counted).with_field(trades());
    let options = media.record_options().unwrap();
    costs("the declared schema and width", &calls, "none", || {
        assert_eq!(media.read_arrow_field(&options).unwrap(), trades());
        assert_eq!(media.column_size().unwrap(), 2);
    });
}

#[test]
fn a_write_while_open_drops_the_held_workbook() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    let one_row = media.read_all_bytes().unwrap();

    media.open().unwrap();
    assert_eq!(media.row_size().unwrap(), 1);
    media
        .overwrite_arrow_batch(
            batch(&[1, 2, 3], &[Some("AAPL"), Some("MSFT"), Some("IBM")]),
            &options,
        )
        .unwrap();
    assert_eq!(media.row_size().unwrap(), 3);
    assert_eq!(
        rows(&media, &options),
        ["[1.0,\"AAPL\"]", "[2.0,\"MSFT\"]", "[3.0,\"IBM\"]"]
    );

    // Bytes written under the wrapper drop it too.
    media.write_all_bytes(&one_row).unwrap();
    assert_eq!(media.row_size().unwrap(), 1);
    assert_eq!(rows(&media, &options), ["[1.0,\"AAPL\"]"]);
    assert!(media.opened());
    media.close().unwrap();
}

#[test]
fn clear_empties_the_handle_and_remove_deletes_it_and_closes_the_wrapper() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    media.open().unwrap();
    assert_eq!(media.row_size().unwrap(), 1);

    media.clear().unwrap();
    assert_eq!(media.size(), 0);
    assert!(media.opened(), "clearing keeps the scope open");
    assert_eq!(media.read_arrow_field(&options).unwrap(), empty_root());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);

    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    assert_eq!(media.row_size().unwrap(), 1);
    media.remove(false).unwrap();
    assert!(!media.opened(), "removing ends the scope");
    assert_eq!(media.size(), 0);
    assert_eq!(media.row_size().unwrap(), 0);
}

// Writes onto a stored workbook.

#[test]
fn append_adds_rows_after_the_stored_ones_typed_by_the_stored_sheet() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), Some("MSFT")]), &options)
        .unwrap();
    media
        .append_arrow_batch(batch(&[3], &[Some("IBM")]), &options)
        .unwrap();

    // The stored sheet reads its ids as numbers, so the appended int64 ids
    // land as float64 beside them, and a declared field reads all three back.
    assert_eq!(
        rows(&media, &options),
        ["[1.0,\"AAPL\"]", "[2.0,\"MSFT\"]", "[3.0,\"IBM\"]"]
    );
    assert_eq!(
        rows(&media, &options.clone().with_field(trades())),
        ["[1,\"AAPL\"]", "[2,\"MSFT\"]", "[3,\"IBM\"]"]
    );
    let workbook = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Sheet1"]);
    assert_eq!(
        workbook
            .sheet("Sheet1")
            .unwrap()
            .scalar("B4".parse().unwrap()),
        Scalar::from("IBM")
    );
}

#[test]
fn merge_updates_the_rows_its_key_matches_and_appends_the_misses() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(
            batch(&[1, 2, 3], &[Some("AAPL"), Some("MSFT"), Some("IBM")]),
            &options,
        )
        .unwrap();
    let mut keyed = options.clone();
    keyed.set_merge_by(Selector::from_columns(["id"]));
    media
        .merge_arrow_batch(batch(&[2, 4], &[Some("GOOG"), Some("AMZN")]), &keyed)
        .unwrap();
    assert_eq!(
        rows(&media, &options),
        [
            "[1.0,\"AAPL\"]",
            "[2.0,\"GOOG\"]",
            "[3.0,\"IBM\"]",
            "[4.0,\"AMZN\"]"
        ]
    );
}

#[test]
fn a_merge_without_a_key_is_refused_naming_the_mode() {
    let mut media = Excel::new(xlsx());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    let before = media.read_all_bytes().unwrap();
    assert_eq!(
        refusal(
            media
                .merge_arrow_batch(batch(&[1], &[Some("MSFT")]), &options)
                .unwrap_err()
        ),
        (
            "$.merge_by".to_owned(),
            "write mode merge requires at least one merge_by column".to_owned()
        )
    );
    assert_eq!(media.read_all_bytes().unwrap(), before);
}

// Sheets.

#[test]
fn a_named_sheet_is_written_beside_the_others_and_read_by_its_name_without_case() {
    let mut media = Excel::new(xlsx()).with_sheet("Trades");
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    let workbook = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Trades"]);

    // A sheet the workbook lacks is added after the others.
    let mut quotes = Excel::new(media.into_handle()).with_sheet("Quotes");
    let options = quotes.record_options().unwrap();
    quotes
        .overwrite_arrow_batch(batch(&[7, 8], &[Some("MSFT"), None]), &options)
        .unwrap();
    let workbook = Workbook::from_bytes(quotes.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Trades", "Quotes"]);

    // No sheet named is the first worksheet.
    let first = Excel::new(quotes.into_handle());
    let options = first.record_options().unwrap();
    assert_eq!(rows(&first, &options), ["[1.0,\"AAPL\"]"]);
    let upper = Excel::new(first.into_handle()).with_sheet("QUOTES");
    let options = upper.record_options().unwrap();
    assert_eq!(rows(&upper, &options), ["[7.0,\"MSFT\"]", "[8.0,null]"]);
}

#[test]
fn a_sheet_the_workbook_lacks_reads_as_nothing_and_a_write_adds_it() {
    let mut media = Excel::new(xlsx()).with_sheet("Trades");
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();

    // Reading the absent sheet is the empty stream a missing resource is.
    let mut quotes = Excel::new(media.into_handle()).with_sheet("Quotes");
    let options = quotes.record_options().unwrap();
    assert_eq!(quotes.read_arrow_field(&options).unwrap().field_len(), 0);
    assert_eq!(quotes.row_size().unwrap(), 0);
    assert_eq!(quotes.column_size().unwrap(), 0);
    assert_eq!(quotes.read_arrow_reader(&options).unwrap().count(), 0);

    // An append onto it is a write that adds the sheet beside the others.
    quotes
        .append_arrow_batch(batch(&[7], &[Some("MSFT")]), &options)
        .unwrap();
    let workbook = Workbook::from_bytes(quotes.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), ["Trades", "Quotes"]);
    assert_eq!(
        workbook
            .sheet("Quotes")
            .unwrap()
            .scalar("B2".parse().unwrap()),
        Scalar::from("MSFT")
    );
    assert_eq!(quotes.row_size().unwrap(), 1);
}

#[test]
fn a_workbook_with_no_worksheet_reads_as_an_empty_root_and_a_write_adds_the_default_sheet() {
    use crate::excel_package::{
        content_types, package, root_relationships, workbook, workbook_relationships,
    };

    let types = content_types(0, false, false);
    let root = root_relationships();
    let relationships = workbook_relationships(0, false, false);
    let book = workbook(&[], false);
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &root),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", &relationships),
    ]);

    let media = Excel::new(stored(bytes.clone()));
    let options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_field(&options).unwrap(), empty_root());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 0);
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);

    // With no worksheet at all, a named one is not refused: there is none.
    let named = Excel::new(stored(bytes.clone())).with_sheet("Trades");
    let options = named.record_options().unwrap();
    assert_eq!(named.read_arrow_field(&options).unwrap(), empty_root());

    let mut media = Excel::new(stored(bytes));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();
    let written = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    assert_eq!(
        written.sheet_names(),
        vec![yggdryl::excel::DEFAULT_SHEET_NAME]
    );
    assert_eq!(rows(&media, &options), ["[1.0,\"AAPL\"]"]);
}

#[test]
fn a_sheet_name_excel_refuses_is_refused_before_a_byte_is_written() {
    let mut media = Excel::new(xlsx()).with_sheet("a/b");
    let options = media.record_options().unwrap();
    assert_eq!(
        refusal(
            media
                .overwrite_arrow_batch(batch(&[1], &[None]), &options)
                .unwrap_err()
        ),
        (
            "$.sheet".to_owned(),
            "expected a sheet name without any of \\ / ? * [ ] :, got '/' in \"a/b\"".to_owned()
        )
    );
    assert_eq!(media.size(), 0);
}

// Header and range.

#[test]
fn without_a_header_the_first_row_holds_values_and_the_columns_are_named_by_their_letters() {
    let mut media = Excel::new(xlsx()).with_options(ExcelOptions::new().with_header(false));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), Some("MSFT")]), &options)
        .unwrap();

    let workbook = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.scalar("A1".parse().unwrap()), Scalar::from(1.0));
    assert_eq!(sheet.scalar("B2".parse().unwrap()), Scalar::from("MSFT"));

    let lettered = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("A"),
            DataType::utf8().required_field("B"),
        ])
        .unwrap(),
    )
    .required_field("row");
    assert_eq!(media.read_arrow_field(&options).unwrap(), lettered);
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(rows(&media, &options), ["[1.0,\"AAPL\"]", "[2.0,\"MSFT\"]"]);
}

#[test]
fn a_range_anchors_the_write_at_its_top_left_cell_and_bounds_the_read() {
    let range: CellRange = "C3:D".parse().unwrap();
    let mut media = Excel::new(xlsx()).with_options(ExcelOptions::new().with_range(range));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
        .unwrap();

    let workbook = Workbook::from_bytes(media.read_all_bytes().unwrap()).unwrap();
    let sheet = workbook.sheet("Sheet1").unwrap();
    assert_eq!(sheet.dimension().unwrap().to_string(), "C3:D4");
    assert_eq!(sheet.scalar("C3".parse().unwrap()), Scalar::from("id"));
    assert_eq!(sheet.scalar("D4".parse().unwrap()), Scalar::from("AAPL"));
    assert_eq!(rows(&media, &options), ["[1.0,\"AAPL\"]"]);
    assert_eq!(media.row_size().unwrap(), 1);
}

#[test]
fn a_row_past_the_grid_is_refused_naming_its_cell_and_the_handle_stays_empty() {
    let range: CellRange = "A1048576:B".parse().unwrap();
    let mut media = Excel::new(xlsx()).with_options(ExcelOptions::new().with_range(range));
    let options = media.record_options().unwrap();
    // The header takes the last row, so the one data row would be past it.
    assert_eq!(
        refusal(
            media
                .overwrite_arrow_batch(batch(&[1], &[None]), &options)
                .unwrap_err()
        ),
        (
            "Sheet1!A1048577".to_owned(),
            "expected at most 1048576 rows in a worksheet, got a row 1048577 past them".to_owned()
        )
    );
    assert_eq!(media.size(), 0);
}

// Refusals of the bytes.

#[test]
fn bytes_that_are_not_a_zip_package_are_refused_as_xlsx_and_a_biff_workbook_as_unsupported() {
    let media = Excel::new(stored(b"not a zip at all".to_vec()));
    let options = media.record_options().unwrap();
    let expected = "expected a ZIP package (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), \
                    got: expected at least 22 bytes of zip archive, got 16";
    match media.read_arrow_field(&options).unwrap_err() {
        Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!((format, position, reason.as_str()), ("xlsx", 0, expected));
        }
        other => panic!("expected a codec failure, got {other:?}"),
    }
    assert_eq!(
        media.row_size().unwrap_err().to_string(),
        format!("invalid xlsx data at byte 0: {expected}")
    );

    let biff = Excel::new(stored(vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]));
    for error in [
        biff.read_arrow_field(&options).unwrap_err(),
        biff.read_arrow_reader(&options).err().unwrap(),
    ] {
        match error {
            Error::Unsupported {
                operation,
                filesystem,
            } => {
                assert_eq!(operation, "reading a BIFF or encrypted workbook");
                assert!(filesystem.starts_with("mem://"), "{filesystem}");
            }
            other => panic!("expected an unsupported read, got {other:?}"),
        }
    }
}

// The free doors.

#[test]
fn the_free_doors_read_and_write_a_plain_buffer_under_excel_options() {
    let mut plain = Buffer::new();
    let options = ExcelOptions::new().with_sheet("Trades");

    // Nothing stored is a root with no columns and no batches, or the
    // declared field's schema and no batches.
    assert_eq!(read_field(&plain, &options).unwrap(), empty_root());
    assert_eq!(
        read_batch_reader(&plain, None, &options).unwrap().count(),
        0
    );
    let declared = read_batch_reader(&plain, Some(&trades()), &options).unwrap();
    assert_eq!(declared.schema(), trades().into_arrow_schema().unwrap());
    assert_eq!(declared.count(), 0);

    overwrite_arrow_reader(&mut plain, reader(&[1, 2], &[Some("AAPL"), None]), &options).unwrap();
    assert_eq!(
        plain.read_range_bytes(0, 4).unwrap(),
        b"PK\x03\x04".to_vec()
    );
    let inferred = DataType::from(
        StructType::from_fields([
            DataType::Float64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .unwrap(),
    )
    .required_field("row");
    assert_eq!(read_field(&plain, &options).unwrap(), inferred);
    assert_eq!(read_field(&plain, &ExcelOptions::new()).unwrap(), inferred);
    assert_eq!(
        free_rows(
            read_batch_reader(&plain, Some(&trades()), &options).unwrap(),
            &trades()
        ),
        ["[1,\"AAPL\"]", "[2,null]"]
    );
    assert_eq!(
        free_rows(
            read_batch_reader(&plain, None, &options).unwrap(),
            &inferred
        ),
        ["[1.0,\"AAPL\"]", "[2.0,null]"]
    );
}

#[test]
fn read_field_answers_a_declared_field_without_reading_the_bytes() {
    let garbage = Buffer::from_bytes(b"garbage".to_vec());
    let mut declared = ExcelOptions::new();
    declared.set_field(trades());
    assert_eq!(read_field(&garbage, &declared).unwrap(), trades());
    assert_eq!(
        read_field(&garbage, &ExcelOptions::new())
            .unwrap_err()
            .to_string(),
        "invalid xlsx data at byte 0: expected a ZIP package \
         (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), \
         got: expected at least 22 bytes of zip archive, got 7"
    );
}

#[test]
fn the_free_overwrite_replaces_the_addressed_sheet_and_carries_the_others_over() {
    let mut plain = Buffer::new();
    overwrite_arrow_reader(
        &mut plain,
        reader(&[1, 2], &[Some("AAPL"), Some("MSFT")]),
        &ExcelOptions::new().with_sheet("Trades"),
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut plain,
        reader(&[9], &[Some("IBM")]),
        &ExcelOptions::new().with_sheet("Other"),
    )
    .unwrap();
    // No sheet named replaces the first worksheet.
    overwrite_arrow_reader(
        &mut plain,
        reader(&[5], &[Some("GOOG")]),
        &ExcelOptions::new(),
    )
    .unwrap();

    let workbook = Workbook::from_bytes(plain.read_all_bytes().unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Trades", "Other"]);
    let trades_sheet = workbook.sheet("Trades").unwrap();
    assert_eq!(
        trades_sheet.scalar("B2".parse().unwrap()),
        Scalar::from("GOOG")
    );
    assert_eq!(trades_sheet.scalar("B3".parse().unwrap()), Scalar::Null);
    assert_eq!(
        workbook
            .sheet("Other")
            .unwrap()
            .scalar("B2".parse().unwrap()),
        Scalar::from("IBM")
    );
}

// Routing by name.

#[test]
fn an_xlsx_name_binds_the_excel_medium() {
    assert!(matches!(
        Media::open(named("trades.xlsx")).unwrap(),
        Media::Registered(wrapper) if wrapper.medium().name() == "excel"
    ));
    match named("trades.xlsx").into_media() {
        Holder::Media(media) => assert!(matches!(
            media.as_ref(),
            Media::Registered(wrapper) if wrapper.medium().name() == "excel"
        )),
        other => panic!("expected a retained workbook medium, got {other:?}"),
    }

    let explicit = Media::open_as(Holder::buffer(Buffer::new()), &MimeType::XLSX)
        .unwrap()
        .with_field(trades());
    let options = explicit.record_options().unwrap();
    assert!(options.settings::<ExcelOptions>().is_some());
    assert_eq!(options.field(), Some(trades()));

    let mut media = Media::open(named("trades.xlsx")).unwrap();
    let options = media.record_options().unwrap();
    assert!(options.settings::<ExcelOptions>().is_some());
    media
        .overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), None]), &options)
        .unwrap();
    assert_eq!(
        media.read_range_bytes(0, 4).unwrap(),
        b"PK\x03\x04".to_vec()
    );
    assert_eq!(
        rows(&media, &options.clone().with_field(trades())),
        ["[1,\"AAPL\"]", "[2,null]"]
    );
}

#[test]
fn a_coded_workbook_name_is_refused_at_every_door() {
    // A workbook is a ZIP package deflated inside: a coding around it would
    // name a file no spreadsheet opens, so every door refuses the name - the
    // write before a byte is written, a read before the package is opened -
    // and says what to drop.
    for (name, codec) in [("trades.xlsx.gz", "gzip"), ("trades.xlsx.zst", "zstd")] {
        let expected = format!(
            "expected an uncompressed xlsx handle, got {codec} coding; a workbook is a ZIP \
             package deflated inside, so drop the {codec} coding from its name"
        );
        let codec_refusal = |error: Error| match error {
            Error::Codec {
                format,
                position,
                reason,
            } => assert_eq!(
                (format, position, reason.as_str()),
                ("xlsx", 0, expected.as_str())
            ),
            other => panic!("{name}: expected a codec refusal, got {other:?}"),
        };

        let mut held = named(name).into_declared_media();
        let options = held.record_options().unwrap();
        assert!(options.settings::<ExcelOptions>().is_some(), "{name}");
        codec_refusal(
            held.overwrite_arrow_batch(batch(&[1], &[Some("AAPL")]), &options)
                .unwrap_err(),
        );
        // Nothing was written.
        assert_eq!(held.size(), 0, "{name}");
        codec_refusal(held.read_arrow_field(&options).unwrap_err());
        codec_refusal(held.read_arrow_reader(&options).err().unwrap());
        codec_refusal(held.row_size().unwrap_err());

        let mut plain = Buffer::new().with_media_type(MediaType::from_file_name(name));
        codec_refusal(
            overwrite_arrow_reader(
                &mut plain,
                reader(&[1], &[Some("AAPL")]),
                &ExcelOptions::new(),
            )
            .unwrap_err(),
        );
        assert!(plain.read_all_bytes().unwrap().is_empty(), "{name}");
        codec_refusal(Error::from(
            read_batch_reader(&plain, None, &ExcelOptions::new())
                .err()
                .unwrap(),
        ));
        codec_refusal(Workbook::open(named(name)).unwrap_err());
    }
}

#[test]
fn a_local_xlsx_url_is_held_written_and_reopened_as_a_workbook() {
    let mut folder = LocalFolder::temporary().unwrap().path().unwrap();
    folder.push(format!("yggdryl-excel-media-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("trades.xlsx");
    let url = Url::from_path(&path).unwrap();
    assert_eq!(url.media_type().base(), &MimeType::XLSX);

    let held = Holder::from_url(&url, std::iter::empty::<(&str, &str)>()).unwrap();
    assert!(matches!(held, Holder::LocalPath(_)));
    let mut held = held.into_declared_media();
    match &held {
        Holder::Media(media) => assert!(matches!(
            media.as_ref(),
            Media::Registered(wrapper) if wrapper.medium().name() == "excel"
        )),
        other => panic!("expected a workbook medium, got {other:?}"),
    }
    let options = held.record_options().unwrap();
    held.overwrite_arrow_batch(batch(&[1, 2], &[Some("AAPL"), None]), &options)
        .unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..4], b"PK\x03\x04");

    let reopened = Holder::from_url(&url, std::iter::empty::<(&str, &str)>())
        .unwrap()
        .into_declared_media();
    let options = reopened.record_options().unwrap().with_field(trades());
    assert_eq!(rows(&reopened, &options), ["[1,\"AAPL\"]", "[2,null]"]);
    assert_eq!(reopened.row_size().unwrap(), 2);
    let workbook =
        Workbook::open(Holder::from_url(&url, std::iter::empty::<(&str, &str)>()).unwrap())
            .unwrap();
    assert_eq!(workbook.sheet_names(), vec!["Sheet1"]);
    let _ = std::fs::remove_dir_all(&folder);
}
