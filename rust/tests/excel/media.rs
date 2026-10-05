//! `rust/src/excel/media.rs`: the `.xlsx` record medium - the `Excel` wrapper
//! over any byte handle, its held workbook between `open` and `close`, and the
//! free `read_field`, `read_batch_reader` and `overwrite_arrow_reader` doors.

use std::sync::Arc;
use yggdryl::RecordHeader;

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

/// A cold dimension read first probes for a container; private owned-source
/// intake then retains one stream directly, eliminating two copy-stage probes.
const DIMENSION_READ: &str =
    "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1";

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

#[test]
fn projected_result_field_agrees_with_reader_for_declared_and_inferred_excel() {
    let mut media = Excel::new(xlsx());
    let stored = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(&[1, 2], &[Some("AAPL"), None]), &stored)
        .unwrap();
    for opened in [false, true] {
        if opened {
            media.open().unwrap();
        }
        for declared in [false, true] {
            for (select, filter, name, rows) in [
                ("id as key", "key > 1", "key", 1),
                ("id + 1 as next_id", "id > 0", "next_id", 2),
            ] {
                let mut options = stored
                    .clone()
                    .with_select(select)
                    .unwrap()
                    .with_filter(filter)
                    .unwrap();
                if declared {
                    options = options.with_field(trades());
                }
                let field = media.read_arrow_field(&options).unwrap();
                assert_eq!(field.field_len(), 1);
                assert_eq!(field.fields()[0].name(), name);
                let batches = media.read_arrow_reader(&options).unwrap();
                assert_eq!(field.into_arrow_schema().unwrap(), batches.schema());
                assert_eq!(
                    batches
                        .map(|batch| batch.unwrap().num_rows())
                        .sum::<usize>(),
                    rows
                );
                assert_eq!(media.column_size().unwrap(), 2);
            }
        }
    }
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
    assert_eq!(media.options().sheet(), None);
    assert_eq!(media.options().header, RecordHeader::Source);
    assert_eq!(media.options().range(), None);

    let media = media.with_field(trades().with_name("trade"));
    assert_eq!(media.options().field, Some(trades().with_name("trade")));
    assert_eq!(media.options().name.as_str(), "trade");

    // A complete configuration replaces the declared field too.
    let configured = ExcelOptions::new().with_header(RecordHeader::None);
    let mut media = media.with_options(configured.clone());
    assert_eq!(media.options(), &configured);
    assert_eq!(media.options().field, None);

    let media_sheet = Excel::new(xlsx()).with_sheet("Trades");
    assert_eq!(media_sheet.options().sheet(), Some("Trades"));

    media.options_mut().header = RecordHeader::Source;
    media.options_mut().set_field(trades());
    assert_eq!(media.options().header, RecordHeader::Source);
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
        .with_options(
            ExcelOptions::new()
                .with_range(range)
                .with_header(RecordHeader::None),
        )
        .with_sheet("Trades")
        .with_field(trades());
    let options = media.record_options().unwrap();
    let RecordOptions::Excel(excel) = &options else {
        panic!("expected Excel options, got {options:?}");
    };
    assert_eq!(excel, media.options());
    assert_eq!(options.field(), Some(trades()));
    assert_eq!(options.mime_type(), MimeType::XLSX);
    assert_eq!(options.excel_sheet(), Some("Trades"));
    assert_eq!(options.header(), Some(RecordHeader::None));
    assert_eq!(options.excel_range(), Some(range));
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
                .overwrite_prepared_arrow_reader(reader(&[2], &[None]), &ipc)
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
    headless.set_header(RecordHeader::None).unwrap();
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
        "pstream_bytes=1 bound_location=1 media_type=1 is_container=1 parent=1",
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
    let mut media =
        Excel::new(xlsx()).with_options(ExcelOptions::new().with_header(RecordHeader::None));
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
        Media::Excel(_)
    ));
    match named("trades.xlsx").into_media() {
        Holder::Media(media) => assert!(matches!(media.as_ref(), Media::Excel(_))),
        other => panic!("expected a retained workbook medium, got {other:?}"),
    }

    let explicit = Media::excel(Holder::buffer(Buffer::new())).with_field(trades());
    let options = explicit.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Excel(_)));
    assert_eq!(options.field(), Some(trades()));

    let mut media = Media::open(named("trades.xlsx")).unwrap();
    let options = media.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Excel(_)));
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
        assert!(matches!(options, RecordOptions::Excel(_)), "{name}");
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
        Holder::Media(media) => assert!(matches!(media.as_ref(), Media::Excel(_))),
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

#[test]
fn named_table_read_uses_one_extent_for_schema_rows_and_dimensions() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let bytes = named_table_package(&named_table_parts());
    for opened in [false, true] {
        for (name, columns, expected) in [
            ("nAmEs", ["id", "name"], ["[1.0,\"one\"]", "[2.0,\"two\"]"]),
            (
                "Quantities",
                ["year", "qty"],
                ["[2024.0,3.0]", "[2025.0,4.0]"],
            ),
        ] {
            let configured = ExcelOptions::new().with_table(name);
            let mut media = Excel::new(stored(bytes.clone())).with_options(configured.clone());
            if opened {
                media.open().unwrap();
            }
            let options = media.record_options().unwrap();
            let field = media.read_arrow_field(&options).unwrap();
            assert_eq!(
                field.fields().iter().map(Field::name).collect::<Vec<_>>(),
                columns
            );
            assert_eq!(
                media.read_arrow_reader(&options).unwrap().schema(),
                field.clone().into_arrow_schema().unwrap()
            );
            assert_eq!(rows(&media, &options), expected);
            assert_eq!(media.row_size().unwrap(), 2);
            assert_eq!(media.column_size().unwrap(), 2);
            assert_eq!(
                free_rows(
                    read_batch_reader(media.handle(), None, &configured).unwrap(),
                    &field
                ),
                expected
            );
        }
    }
}

#[test]
fn named_table_read_ignores_unselected_bad_values_and_orphan_table_relationships() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace("<v>2024</v>", "<v>not-a-number</v>");
    let rels = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/_rels/sheet1.xml.rels")
        .unwrap()
        .1;
    *rels = rels.replace("</Relationships>", &format!("<Relationship Id=\"Orphan\" Type=\"{}/table\" Target=\"../tables/orphan.xml\"/></Relationships>", crate::excel_package::R_NS));
    parts.push(("xl/tables/orphan.xml", "not table XML".into()));
    let media = Excel::new(stored(named_table_package(&parts)))
        .with_options(ExcelOptions::new().with_table("Names"));
    assert_eq!(
        rows(&media, &media.record_options().unwrap()),
        ["[1.0,\"one\"]", "[2.0,\"two\"]"]
    );
}

#[test]
fn named_table_read_pairs_declared_and_projected_results_with_the_table_body() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let media = Excel::new(stored(named_table_package(&named_table_parts())));
    let options = RecordOptions::from(ExcelOptions::new().with_table("Quantities"))
        .with_select("qty * 2 as doubled")
        .unwrap()
        .with_filter("doubled > 6")
        .unwrap();
    let field = media.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["doubled"]
    );
    assert_eq!(
        media.read_arrow_reader(&options).unwrap().schema(),
        field.clone().into_arrow_schema().unwrap()
    );
    assert_eq!(rows(&media, &options), ["[8.0]"]);
}

#[test]
fn named_table_read_validates_actual_membership_through_existing_relationship_rules() {
    use crate::excel_package::{R_NS, named_table_package, named_table_parts};
    for (old, new, reason) in [
        (
            "Target=\"../tables/table1.xml\"".to_owned(),
            "Target=\"../tables/missing.xml\"".to_owned(),
            "got missing",
        ),
        (
            format!("Type=\"{R_NS}/table\" Target=\"../tables/table1.xml\""),
            format!("Type=\"{R_NS}/drawing\" Target=\"../tables/table1.xml\""),
            "expected a table relationship",
        ),
        (
            "Target=\"../tables/table1.xml\"".to_owned(),
            "Target=\"https://example.test/table.xml\" TargetMode=\"External\"".to_owned(),
            "external or absent",
        ),
    ] {
        let mut parts = named_table_parts();
        let rels = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/worksheets/_rels/sheet1.xml.rels")
            .unwrap()
            .1;
        assert!(rels.contains(&old));
        *rels = rels.replace(&old, &new);
        let media = Excel::new(stored(named_table_package(&parts)))
            .with_options(ExcelOptions::new().with_table("Names"));
        let (path, actual) = refusal(media.column_size().unwrap_err());
        assert_eq!(path, "xl/worksheets/sheet1.xml#tablePart[rIdT1]");
        assert!(actual.contains(reason), "{actual}");
    }
}

#[test]
fn named_table_read_missing_and_duplicate_names_refuse_with_identity_locations() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let options = RecordOptions::from(ExcelOptions::new().with_table("Missing"));
    let error = Excel::new(stored(named_table_package(&parts)))
        .read_arrow_field(&options)
        .unwrap_err();
    let (path, reason) = refusal(error);
    assert_eq!(path, "$.table");
    for wanted in [
        "Missing",
        "Data!Names",
        "Data!Quantities",
        "xl/tables/table1.xml",
        "xl/tables/table2.xml",
    ] {
        assert!(reason.contains(wanted), "{reason}");
    }
    let target = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *target = target.replace("Quantities", "nAmEs");
    let data = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *data = data.replace("<tablePart r:id=\"rIdT2\"/>", "");
    let other = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/worksheets/sheet2.xml")
        .unwrap()
        .1;
    *other = crate::excel_package::sheet(
        "",
        "<tableParts count=\"1\"><tablePart r:id=\"rIdT2\"/></tableParts>",
    );
    parts.push(("xl/worksheets/_rels/sheet2.xml.rels", format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdT2\" Type=\"{}/table\" Target=\"../tables/table2.xml\"/></Relationships>", crate::excel_package::R_NS)));
    let options = RecordOptions::from(ExcelOptions::new().with_table("Names"));
    let (path, reason) = refusal(
        Excel::new(stored(named_table_package(&parts)))
            .read_arrow_field(&options)
            .unwrap_err(),
    );
    assert_eq!(path, "$.table");
    for wanted in ["nAmEs", "xl/tables/table1.xml", "xl/tables/table2.xml"] {
        assert!(reason.contains(wanted), "{reason}");
    }
}

#[test]
fn named_table_write_changes_only_selected_body_beside_another_table() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let options = ExcelOptions::new().with_table("Names");
    // The selected table's authoritative column names are id and name.
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let input = RecordBatch::try_new(
        field.clone().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![9, 10])),
            Arc::new(StringArray::from(vec![Some("nine"), Some("ten")])),
        ],
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(field.into_arrow_schema().unwrap(), [input]),
        &options,
    )
    .unwrap();

    let output = handle.read_all_bytes().unwrap();
    let media = Excel::new(stored(output.clone()));
    let names = RecordOptions::from(options);
    let quantities = RecordOptions::from(ExcelOptions::new().with_table("Quantities"));
    assert_eq!(rows(&media, &names), ["[9.0,\"nine\"]", "[10.0,\"ten\"]"]);
    assert_eq!(rows(&media, &quantities), ["[2024.0,3.0]", "[2025.0,4.0]"]);

    let old = Workbook::from_bytes(original).unwrap();
    let new = Workbook::from_bytes(output).unwrap();
    for part in [
        "xl/tables/table1.xml",
        "xl/tables/table2.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
    ] {
        assert_eq!(member(&new, part), member(&old, part), "{part}");
    }
    let sheet = member(&new, "xl/worksheets/sheet1.xml");
    for retained in [
        "r=\"A1\"",
        "r=\"B1\"",
        "r=\"D1\"",
        "r=\"E1\"",
        "r=\"D2\"",
        "r=\"E2\"",
        "r=\"D3\"",
        "r=\"E3\"",
        "r=\"D4\"",
        "r=\"E4\"",
        "<tableParts count=\"2\">",
    ] {
        assert!(sheet.contains(retained), "missing {retained} in {sheet}");
    }
}

#[test]
fn named_table_write_replaces_file_backed_package_and_reopens() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut folder = LocalFolder::temporary().unwrap().path().unwrap();
    folder.push(format!(
        "yggdryl-excel-named-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("tables.xlsx");
    std::fs::write(&path, named_table_package(&named_table_parts())).unwrap();
    let url = Url::from_path(&path).unwrap();
    let mut held = Holder::from_url(&url, std::iter::empty::<(&str, &str)>()).unwrap();
    overwrite_arrow_reader(
        &mut held,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::None),
    )
    .unwrap();
    drop(held);
    let reopened = Excel::new(stored(std::fs::read(&path).unwrap()));
    assert_eq!(
        rows(
            &reopened,
            &RecordOptions::from(ExcelOptions::new().with_table("Names"))
        ),
        ["[9.0,\"nine\"]", "[10.0,\"ten\"]"]
    );
    assert_eq!(
        rows(
            &reopened,
            &RecordOptions::from(ExcelOptions::new().with_table("Quantities"))
        ),
        ["[2024.0,3.0]", "[2025.0,4.0]"]
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(folder).unwrap();
}

#[test]
fn named_table_write_preserves_header_totals_and_other_table() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let options = ExcelOptions::new().with_table("Quantities");
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let input = RecordBatch::try_new(
        field.clone().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![2030, 2031])),
            Arc::new(Int64Array::from(vec![6, 8])),
        ],
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(field.into_arrow_schema().unwrap(), [input]),
        &options,
    )
    .unwrap();
    let output = handle.read_all_bytes().unwrap();
    let media = Excel::new(stored(output.clone()));
    assert_eq!(
        rows(&media, &RecordOptions::from(options)),
        ["[2030.0,6.0]", "[2031.0,8.0]"]
    );
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Names"))
        ),
        ["[1.0,\"one\"]", "[2.0,\"two\"]"]
    );
    let old = Workbook::from_bytes(original).unwrap();
    let new = Workbook::from_bytes(output).unwrap();
    assert_eq!(
        member(&new, "xl/tables/table1.xml"),
        member(&old, "xl/tables/table1.xml")
    );
    assert_eq!(
        member(&new, "xl/tables/table2.xml"),
        member(&old, "xl/tables/table2.xml")
    );
    let sheet = member(&new, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("r=\"D4\""));
    assert!(sheet.contains("r=\"E4\""));
    assert!(sheet.contains("<v>7</v>"));
}

#[test]
fn named_table_write_keeps_row_style_and_opaque_worksheet_child() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet
        .replace(
            "<sheetData>",
            "<sheetPr codeName=\"RetainedCode\"/><sheetData>",
        )
        .replace(
            "<row r=\"2\">",
            "<row r=\"2\" ht=\"23\" customHeight=\"1\">",
        )
        .replace("<c r=\"A2\">", "<c r=\"A2\" s=\"0\">");
    let mut handle = stored(named_table_package(&parts));
    let options = ExcelOptions::new()
        .with_table("Names")
        .with_header(RecordHeader::None);
    overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &options,
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<sheetPr codeName=\"RetainedCode\"/>"));
    assert!(sheet.contains("<row r=\"2\" ht=\"23\" customHeight=\"1\">"));
    assert!(sheet.contains("<c r=\"A2\" s=\"0\">"));
    assert!(sheet.contains("<tableParts count=\"2\">"));
}

#[test]
fn named_table_write_fills_sparse_body_without_touching_other_columns() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace("<c r=\"A2\"><v>1</v></c>", "");
    let from = sheet.find("<row r=\"3\">").unwrap();
    let until = from + sheet[from..].find("</row>").unwrap() + "</row>".len();
    sheet.replace_range(from..until, "");
    let mut handle = stored(named_table_package(&parts));
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let input = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![9, 10])),
            Arc::new(StringArray::from(vec![Some("nine"), Some("ten")])),
        ],
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(schema, [input]),
        &ExcelOptions::new().with_table("Names"),
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    for cell in ["A2", "B2", "A3", "B3", "D2", "E2", "D4", "E4"] {
        assert!(sheet.contains(&format!("r=\"{cell}\"")), "missing {cell}");
    }
    assert!(!sheet.contains("r=\"D3\""));
    assert!(!sheet.contains("r=\"E3\""));
}

#[test]
fn named_table_write_replaces_selected_formula_with_scalar() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<c r=\"A2\"><v>1</v></c>",
        "<c r=\"A2\"><f>1+1</f><v>2</v></c>",
    );
    let mut handle = stored(named_table_package(&parts));
    let options = ExcelOptions::new()
        .with_table("Names")
        .with_header(RecordHeader::None);
    overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &options,
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("<f>1+1</f>"));
    assert!(sheet.contains("<c r=\"A2\"><v>9</v></c>"));
    assert!(sheet.contains("r=\"D2\""));
}

#[test]
fn named_table_write_accepts_implicit_row_and_cell_coordinates() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet
        .replace("<row r=\"2\"><c r=\"A2\">", "<row><c>")
        .replace(
            "</sheetData>",
            "<row><c r=\"G5\"><v>5</v></c></row></sheetData>",
        );
    let mut handle = stored(named_table_package(&parts));
    overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::None),
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<row><c r=\"A2\"><v>9</v></c>"), "{sheet}");
    assert!(
        sheet.contains("<row><c r=\"G5\"><v>5</v></c></row>"),
        "{sheet}"
    );
}

#[test]
fn named_table_write_replaces_rich_inline_text_without_touching_other_cells() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<is><t>one</t></is>",
        "<is><r><rPr><b/></rPr><t>old rich text</t></r></is>",
    );
    let mut handle = stored(named_table_package(&parts));
    overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::None),
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("old rich text"), "{sheet}");
    let media = Excel::new(stored(handle.read_all_bytes().unwrap()));
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Names"))
        ),
        ["[9.0,\"nine\"]", "[10.0,\"ten\"]"]
    );
    assert!(sheet.contains("<c r=\"D2\"><v>2024</v></c>"), "{sheet}");
}

#[test]
fn named_table_write_inserts_sparse_cell_before_retained_row_extension() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<c r=\"E2\"><v>3</v></c></row>",
        "<extLst><ext uri=\"urn:kept\"/></extLst></row>",
    );
    let mut handle = stored(named_table_package(&parts));
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let input = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![2030, 2031])),
            Arc::new(Int64Array::from(vec![6, 8])),
        ],
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(schema, [input]),
        &ExcelOptions::new().with_table("Quantities"),
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    let row = sheet
        .split("<row r=\"2\">")
        .nth(1)
        .unwrap()
        .split("</row>")
        .next()
        .unwrap();
    assert!(
        row.find("r=\"E2\"").unwrap() < row.find("<extLst>").unwrap(),
        "{row}"
    );
    assert!(
        row.contains("<extLst><ext uri=\"urn:kept\"/></extLst>"),
        "{row}"
    );
    assert!(row.contains("r=\"A2\""), "{row}");
}

#[test]
fn named_table_write_refuses_grouped_formula_master_atomically() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<c r=\"A2\"><v>1</v></c>",
        "<c r=\"A2\"><f t=\"shared\" si=\"0\" ref=\"A2:D2\">1+1</f><v>2</v></c>",
    );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let error = overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &ExcelOptions::new()
            .with_table("Names")
            .with_header(RecordHeader::None),
    )
    .unwrap_err();
    let (path, reason) = refusal(error);
    assert!(path.contains("Data!A2"), "{path}: {reason}");
    assert!(reason.contains("grouped formula"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_write_temporal_cell_keeps_its_font_when_format_changes() {
    use crate::excel_package::{R_NS, member, named_table_package, named_table_parts, styles};
    use arrow_array::Date32Array;
    let mut parts = named_table_parts();
    let types = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "[Content_Types].xml")
        .unwrap()
        .1;
    *types = types.replace("</Types>", "<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>");
    let rels = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/_rels/workbook.xml.rels")
        .unwrap()
        .1;
    *rels = rels.replace("</Relationships>", &format!(
        "<Relationship Id=\"rId3\" Type=\"{R_NS}/styles\" Target=\"styles.xml\"/></Relationships>"));
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<c r=\"D2\"><v>2024</v></c>",
        "<c r=\"D2\" s=\"1\"><v>2024</v></c>",
    );
    let mut style = styles(&[], &[0, 0]);
    style = style
        .replace("<fonts count=\"1\">", "<fonts count=\"2\">")
        .replace(
            "</fonts>",
            "<font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>",
        );
    let old = "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>";
    let at = style.rfind(old).unwrap();
    style.replace_range(
        at..at + old.len(),
        &old.replacen("fontId=\"0\"", "fontId=\"1\"", 1),
    );
    parts.push(("xl/styles.xml", style));
    let mut handle = stored(named_table_package(&parts));
    let field = DataType::from(
        StructType::from_fields([
            DataType::Date32.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let input = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![19_723, 19_724])),
            Arc::new(Int64Array::from(vec![6, 8])),
        ],
    )
    .unwrap();
    overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(schema, [input]),
        &ExcelOptions::new().with_table("Quantities"),
    )
    .unwrap();
    let book = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    let sheet = member(&book, "xl/worksheets/sheet1.xml");
    let cell = sheet.split("<c r=\"D2\" s=\"").nth(1).unwrap();
    let style_id: usize = cell.split('"').next().unwrap().parse().unwrap();
    let styles = member(&book, "xl/styles.xml");
    let xfs = styles
        .split("<cellXfs")
        .nth(1)
        .unwrap()
        .split("</cellXfs>")
        .next()
        .unwrap();
    let entries: Vec<&str> = xfs.split("<xf ").skip(1).collect();
    assert!(entries[style_id].contains("fontId=\"1\""));
    assert!(!entries[style_id].contains("numFmtId=\"0\""));
    assert!(sheet.contains("r=\"D4\""));
}

#[test]
fn named_table_write_missing_selection_is_atomic_and_located() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let error = overwrite_arrow_reader(
        &mut handle,
        reader(&[9], &[Some("nine")]),
        &ExcelOptions::new().with_table("Missing"),
    )
    .unwrap_err();
    let (path, reason) = refusal(error);
    assert_eq!(path, "$.table");
    assert!(reason.contains("Missing"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_write_source_columns_must_match_table_columns_atomically() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    // The incoming schema says symbol, while tableColumns says name.
    let error = overwrite_arrow_reader(
        &mut handle,
        reader(&[9, 10], &[Some("nine"), Some("ten")]),
        &ExcelOptions::new().with_table("Names"),
    )
    .unwrap_err();
    let (path, reason) = refusal(error);
    assert!(
        path.contains("symbol") || path.contains("name") || path.contains("table"),
        "{path}: {reason}"
    );
    assert!(
        reason.contains("name") || reason.contains("symbol"),
        "{reason}"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

fn names_reader(ids: &[i64], labels: &[Option<&str>]) -> BatchReader {
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(labels.to_vec())),
        ],
    )
    .unwrap();
    yggdryl::arrow::batch_reader(schema, [batch])
}

#[test]
fn named_table_write_grows_into_empty_columns_beside_stationary_table() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let table = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/tables/table1.xml")
        .unwrap()
        .1;
    *table = table.replace(
        "<tableColumns count=",
        "<autoFilter ref=\"A1:B3\"/><tableColumns count=",
    );
    let original = named_table_package(&parts);
    let old = Workbook::from_bytes(original.clone()).unwrap();
    let mut handle = stored(original);
    overwrite_arrow_reader(
        &mut handle,
        names_reader(&[9, 10, 11], &[Some("nine"), Some("ten"), Some("eleven")]),
        &ExcelOptions::new().with_table("Names"),
    )
    .unwrap();
    let output = handle.read_all_bytes().unwrap();
    let new = Workbook::from_bytes(output.clone()).unwrap();
    let media = Excel::new(stored(output));
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Names"))
        ),
        ["[9.0,\"nine\"]", "[10.0,\"ten\"]", "[11.0,\"eleven\"]"]
    );
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Quantities"))
        ),
        ["[2024.0,3.0]", "[2025.0,4.0]"]
    );
    assert!(member(&new, "xl/tables/table1.xml").contains("ref=\"A1:B4\""));
    assert!(member(&new, "xl/tables/table1.xml").contains("<autoFilter ref=\"A1:B4\"/>"));
    assert_eq!(
        member(&new, "xl/tables/table2.xml"),
        member(&old, "xl/tables/table2.xml")
    );
    let sheet = member(&new, "xl/worksheets/sheet1.xml");
    for cell in ["A4", "B4", "D4", "E4"] {
        assert!(
            sheet.contains(&format!("r=\"{cell}\"")),
            "missing {cell}: {sheet}"
        );
    }
    assert!(
        sheet.contains("<v>7</v>"),
        "adjacent totals changed: {sheet}"
    );
}

#[test]
fn named_table_write_shrinks_without_moving_adjacent_table() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let old = Workbook::from_bytes(original.clone()).unwrap();
    let mut handle = stored(original);
    overwrite_arrow_reader(
        &mut handle,
        names_reader(&[9], &[Some("nine")]),
        &ExcelOptions::new().with_table("Names"),
    )
    .unwrap();
    let output = handle.read_all_bytes().unwrap();
    let new = Workbook::from_bytes(output.clone()).unwrap();
    let media = Excel::new(stored(output));
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Names"))
        ),
        ["[9.0,\"nine\"]"]
    );
    assert_eq!(
        rows(
            &media,
            &RecordOptions::from(ExcelOptions::new().with_table("Quantities"))
        ),
        ["[2024.0,3.0]", "[2025.0,4.0]"]
    );
    assert!(member(&new, "xl/tables/table1.xml").contains("ref=\"A1:B2\""));
    assert_eq!(
        member(&new, "xl/tables/table2.xml"),
        member(&old, "xl/tables/table2.xml")
    );
    let sheet = member(&new, "xl/worksheets/sheet1.xml");
    assert!(
        !sheet.contains("r=\"A3\""),
        "former body cell remained: {sheet}"
    );
    assert!(
        !sheet.contains("r=\"B3\""),
        "former body cell remained: {sheet}"
    );
    for cell in ["D3", "E3", "D4", "E4"] {
        assert!(
            sheet.contains(&format!("r=\"{cell}\"")),
            "missing {cell}: {sheet}"
        );
    }
}

#[test]
fn named_table_growth_refuses_occupied_target_atomically() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace("<row r=\"4\">", "<row r=\"4\"><c r=\"A4\"><v>777</v></c>");
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            names_reader(&[9, 10, 11], &[Some("nine"), Some("ten"), Some("eleven")]),
            &ExcelOptions::new().with_table("Names"),
        )
        .unwrap_err(),
    );
    assert!(
        path.contains("A4") || path.contains("Names"),
        "{path}: {reason}"
    );
    assert!(
        reason.contains("occupied") || reason.contains("overlap"),
        "{reason}"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_resize_zero_body_refuses_atomically() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            names_reader(&[], &[]),
            &ExcelOptions::new().with_table("Names"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("Names"), "{path}: {reason}");
    assert!(
        reason.contains("one") || reason.contains("zero"),
        "{reason}"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

fn named_totals_resize_parts() -> Vec<(&'static str, String)> {
    use crate::excel_package::named_table_parts;
    let mut parts = named_table_parts();
    let worksheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet.replace(
        "<c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c><c r=\"E4\"><v>7</v></c>",
        "<c r=\"C4\"><v>91</v></c><c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c><c r=\"E4\"><f>SUBTOTAL(109,[qty])</f><v>7</v></c><c r=\"G4\"><v>92</v></c>",
    ).replace("</sheetData>",
        "<row r=\"5\"><c r=\"C5\"><v>93</v></c><c r=\"G5\"><v>94</v></c></row></sheetData>");
    let table = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/tables/table2.xml")
        .unwrap()
        .1;
    *table = table.replace(
        "totalsRowCount=\"1\"><tableColumns",
        "totalsRowCount=\"1\"><autoFilter ref=\"D1:E3\"/><tableColumns",
    );
    parts
}

fn named_totals_resize_package() -> Vec<u8> {
    crate::excel_package::named_table_package(&named_totals_resize_parts())
}

fn quantities_reader(rows: usize) -> BatchReader {
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let years = (0..rows).map(|row| 2030 + row as i64).collect::<Vec<_>>();
    let quantities = (0..rows).map(|row| 6 + row as i64).collect::<Vec<_>>();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(years)),
            Arc::new(Int64Array::from(quantities)),
        ],
    )
    .unwrap();
    yggdryl::arrow::batch_reader(schema, [batch])
}

fn totals_two_styles(parts: &mut Vec<(&'static str, String)>) {
    use crate::excel_package::{R_NS, styles};
    let types = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "[Content_Types].xml")
        .unwrap()
        .1;
    *types = types.replace("</Types>",
        "<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/></Types>");
    let relationships = &mut parts
        .iter_mut()
        .find(|(name, _)| *name == "xl/_rels/workbook.xml.rels")
        .unwrap()
        .1;
    *relationships = relationships.replace("</Relationships>",
        &format!("<Relationship Id=\"rId3\" Type=\"{R_NS}/styles\" Target=\"styles.xml\"/></Relationships>"));
    parts.push(("xl/styles.xml", styles(&[], &[0, 14])));
}

#[test]
fn named_table_with_totals_moves_totals_for_positive_body_resize() {
    use crate::excel_package::member;
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/named_totals_excel.json")).unwrap();
    assert_eq!(oracle["passed"], true);
    let original = named_totals_resize_package();
    let old = Workbook::from_bytes(original.clone()).unwrap();
    let old_table = member(&old, "xl/tables/table2.xml");
    for (key, attribute) in [("range", "ref"), ("filter", "autoFilter ref")] {
        let expected = oracle["source_table"][key].as_str().unwrap();
        assert!(
            old_table.contains(&format!("{attribute}=\"{expected}\"")),
            "{old_table}"
        );
    }
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let cases = oracle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let mode = case["mode"].as_str().unwrap();
        let rows = match mode {
            "shrink" => 1,
            "equal" => 2,
            "grow" => 3,
            _ => panic!("{mode}"),
        };
        let years = (0..rows).map(|row| 2030 + row as i64).collect::<Vec<_>>();
        let quantities = (0..rows).map(|row| 6 + row as i64).collect::<Vec<_>>();
        assert_eq!(
            case["expected_sum"].as_i64(),
            Some(quantities.iter().copied().sum())
        );
        let input = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(years)),
                Arc::new(Int64Array::from(quantities)),
            ],
        )
        .unwrap();
        let mut handle = stored(original.clone());
        overwrite_arrow_reader(
            &mut handle,
            yggdryl::arrow::batch_reader(schema.clone(), [input]),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap();
        let new = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
        let table = member(&new, "xl/tables/table2.xml");
        let expected = &case["table"];
        let range = expected["range"].as_str().unwrap();
        let filter = expected["filter"].as_str().unwrap();
        assert!(table.contains(&format!("ref=\"{range}\"")), "{table}");
        assert!(
            table.contains(&format!("<autoFilter ref=\"{filter}\"")),
            "{table}"
        );
        assert!(table.contains("totalsRowCount=\"1\""), "{table}");
        assert_eq!(
            member(&new, "xl/tables/table1.xml"),
            member(&old, "xl/tables/table1.xml")
        );
        let data = new.sheet("Data").unwrap();
        for (at, value) in [("C4", 91.0), ("G4", 92.0), ("C5", 93.0), ("G5", 94.0)] {
            assert_eq!(
                data.scalar(at.parse().unwrap()),
                value.into(),
                "{mode}: {at}"
            );
        }
        let xml = member(&new, "xl/worksheets/sheet1.xml");
        let totals_at = case["totals_cell"].as_str().unwrap();
        let formula = case["totals_formula"]
            .as_str()
            .unwrap()
            .strip_prefix('=')
            .unwrap();
        assert!(
            xml.contains(&format!("r=\"{totals_at}\"><f>{formula}</f>")),
            "{mode}: {xml}"
        );
        if mode != "equal" {
            assert!(
                !xml.contains(&format!("<f>{formula}</f><v>7</v>")),
                "{mode}: {xml}"
            );
        }
        match mode {
            "shrink" => {
                assert!(data.scalar("D4".parse().unwrap()).is_null());
                assert!(data.scalar("E4".parse().unwrap()).is_null());
            }
            "equal" => assert!(xml.contains("r=\"D4\" t=\"inlineStr\""), "{xml}"),
            "grow" => {
                assert_eq!(data.scalar("D4".parse().unwrap()), 2032.0.into());
                assert_eq!(data.scalar("E4".parse().unwrap()), 8.0.into());
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn named_totals_move_keeps_unrelated_totals_row_cells_with_unreadable_styles() {
    use crate::excel_package::{member, named_table_package};
    for (label, old_cell, raw_cell) in [
        (
            "implicit-left",
            "<c r=\"C4\"><v>91</v></c>",
            "<c s=\"not-a-style\"><v>91</v></c>",
        ),
        (
            "explicit-right",
            "<c r=\"G4\"><v>92</v></c>",
            "<c r=\"G4\" s=\"not-a-style\"><v>92</v></c>",
        ),
    ] {
        let mut parts = named_totals_resize_parts();
        let worksheet = &mut parts
            .iter_mut()
            .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        *worksheet = worksheet.replace(old_cell, raw_cell);
        let original = named_table_package(&parts);
        let mut handle = stored(original);
        overwrite_arrow_reader(
            &mut handle,
            quantities_reader(3),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap();
        let result = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
        let xml = member(&result, "xl/worksheets/sheet1.xml");
        assert!(xml.contains(raw_cell), "{label}: {xml}");
    }
}

#[test]
fn named_totals_move_refuses_unreadable_selected_style_atomically() {
    use crate::excel_package::named_table_package;
    let mut parts = named_totals_resize_parts();
    let worksheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet.replace(
        "<c r=\"D4\" t=\"inlineStr\">",
        "<c r=\"D4\" s=\"not-a-style\" t=\"inlineStr\">",
    );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            quantities_reader(3),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("Data!"), "{path}: {reason}");
    assert!(
        reason.contains("style") || reason.contains("integer"),
        "{reason}"
    );
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_totals_equal_height_keeps_existing_a1_formula() {
    use crate::excel_package::{member, named_table_package};
    let mut parts = named_totals_resize_parts();
    let worksheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet
        .replace("SUBTOTAL(109,[qty])", "SUM(E2:E3)+E3")
        .replace(
            "<row r=\"4\">",
            "<row r=\"4\" xml:space=\"preserve\" xmlns:q=\"urn:unrelated\">",
        );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    overwrite_arrow_reader(
        &mut handle,
        quantities_reader(2),
        &ExcelOptions::new().with_table("Quantities"),
    )
    .unwrap();
    let result = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
    assert!(member(&result, "xl/worksheets/sheet1.xml").contains("r=\"E4\"><f>SUM(E2:E3)+E3</f>"));
}

#[test]
fn named_totals_move_refuses_a1_grouped_and_scope_atomically() {
    use crate::excel_package::named_table_package;
    for (kind, row_change, cell_change, reason) in [
        ("a1", "<row r=\"4\">", "<f>SUM(E2:E3)+E3</f>", "A1"),
        ("split-a1", "<row r=\"4\">", "<f>A<![CDATA[1]]></f>", "A1"),
        ("nested-f", "<row r=\"4\">", "<f><t>A1</t></f>", "text-only"),
        (
            "grouped",
            "<row r=\"4\">",
            "<f t=\"shared\" si=\"0\">1+1</f>",
            "grouped formula",
        ),
        (
            "foreign-f",
            "<row r=\"4\">",
            "<f xmlns=\"urn:foreign\">A1</f>",
            "plain value",
        ),
        (
            "reset-f",
            "<row r=\"4\">",
            "<f xmlns=\"\">A1</f>",
            "plain value",
        ),
        (
            "foreign-v",
            "<row r=\"4\">",
            "<f>SUBTOTAL(109,[qty])</f>",
            "plain value",
        ),
        (
            "scope",
            "<row r=\"4\" xml:space=\"preserve\">",
            "<f>SUBTOTAL(109,[qty])</f>",
            "namespace",
        ),
    ] {
        let mut parts = named_totals_resize_parts();
        let worksheet = &mut parts
            .iter_mut()
            .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        *worksheet = worksheet
            .replace("<row r=\"4\">", row_change)
            .replace("<f>SUBTOTAL(109,[qty])</f>", cell_change);
        if kind == "foreign-v" {
            *worksheet = worksheet.replace("<v>7</v>", "<v xmlns=\"urn:foreign\">7</v>");
        }
        let original = named_table_package(&parts);
        let mut handle = stored(original.clone());
        let (path, actual) = refusal(
            overwrite_arrow_reader(
                &mut handle,
                quantities_reader(
                    if kind.starts_with("foreign-")
                        || kind == "reset-f"
                        || kind == "split-a1"
                        || kind == "nested-f"
                    {
                        1
                    } else {
                        3
                    },
                ),
                &ExcelOptions::new().with_table("Quantities"),
            )
            .unwrap_err(),
        );
        assert!(
            path.contains("Data!") || path.contains("Quantities"),
            "{kind}: {path}: {actual}"
        );
        assert!(actual.contains(reason), "{kind}: {path}: {actual}");
        assert_eq!(handle.read_all_bytes().unwrap(), original, "{kind}");
    }
}

#[test]
fn named_totals_move_late_reader_error_is_atomic() {
    use arrow_schema::ArrowError;
    let original = named_totals_resize_package();
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("year"),
            DataType::Int64.required_field("qty"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let first = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![2030])),
            Arc::new(Int64Array::from(vec![6])),
        ],
    )
    .unwrap();
    let reader: BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        [
            Ok(first),
            Err(ArrowError::ComputeError("late totals input".into())),
        ]
        .into_iter(),
        schema,
    ));
    let mut handle = stored(original.clone());
    let error = overwrite_arrow_reader(
        &mut handle,
        reader,
        &ExcelOptions::new().with_table("Quantities"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("late totals input"), "{error}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_totals_growth_refuses_merge_at_new_totals_row() {
    use crate::excel_package::named_table_package;
    let mut parts = named_totals_resize_parts();
    let worksheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet.replace(
        "<tableParts count=\"2\">",
        "<mergeCells count=\"1\"><mergeCell ref=\"D5:E5\"/></mergeCells><tableParts count=\"2\">",
    );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            quantities_reader(3),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("Quantities"), "{path}: {reason}");
    assert!(reason.contains("merged"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_totals_move_keeps_allocated_default_styles_from_excel() {
    use crate::excel_package::{member, named_table_package};
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/allocated_totals_styles_excel.json")).unwrap();
    for case in evidence["cases"].as_array().unwrap() {
        let presence = case["id"].as_str().unwrap();
        let mut parts = named_totals_resize_parts();
        totals_two_styles(&mut parts);
        let styles = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/styles.xml")
            .unwrap()
            .1;
        let fonts =
            "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>";
        assert_eq!(styles.matches(fonts).count(), 1);
        *styles = styles.replace(fonts,
            "<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>");
        let xf = "<xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>";
        assert_eq!(styles.matches(xf).count(), 1);
        *styles = styles.replace(xf,
            "<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>");
        let sheet = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        *sheet = sheet
            .replace("<row r=\"4\">", "<row r=\"4\" s=\"1\" customFormat=\"1\">")
            .replace("<row r=\"5\">", "<row r=\"5\" s=\"0\" customFormat=\"1\">");
        let cell = "<c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c>";
        assert_eq!(sheet.matches(cell).count(), 1);
        match presence {
            "absent_s" => {}
            "explicit_zero" => {
                *sheet = sheet.replace(
                    cell,
                    "<c r=\"D4\" s=\"0\" t=\"inlineStr\"><is><t>Total</t></is></c>",
                )
            }
            "unallocated" => *sheet = sheet.replace(cell, ""),
            other => panic!("unexpected native fixture presence {other}"),
        }
        let original = named_table_package(&parts);
        let before = Workbook::from_bytes(original.clone()).unwrap();
        assert_eq!(
            before
                .cell_style("Data", "D4".parse().unwrap())
                .unwrap()
                .font
                .bold,
            case["before"]["cells"]["D4"]["font_bold"]
                .as_bool()
                .unwrap(),
            "{presence}"
        );
        let mut handle = stored(original.clone());
        let written = overwrite_arrow_reader(
            &mut handle,
            quantities_reader(3),
            &ExcelOptions::new().with_table("Quantities"),
        );
        if presence == "unallocated" {
            // Native Resize creates the declared label with the source's bold
            // style. This unsupported synthesis remains an atomic refusal.
            let (path, reason) = refusal(written.unwrap_err());
            assert_eq!(path, "Data!D5");
            assert!(reason.contains("style"), "{reason}");
            assert_eq!(handle.read_all_bytes().unwrap(), original);
            assert_eq!(case["reopened"]["cells"]["D5"]["value2"], "Total");
            assert_eq!(case["reopened"]["cells"]["D5"]["font_bold"], true);
            continue;
        }
        written.unwrap();
        let after = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
        let expected = &case["reopened"]["cells"]["D5"];
        assert_eq!(
            after.sheet("Data").unwrap().scalar("D5".parse().unwrap()),
            Scalar::from(expected["value2"].as_str().unwrap())
        );
        let style = after.cell_style("Data", "D5".parse().unwrap()).unwrap();
        assert_eq!(
            style.font.bold,
            expected["font_bold"].as_bool().unwrap(),
            "{presence}"
        );
        assert_eq!(
            style.number_format,
            expected["number_format_en_us"].as_str().unwrap()
        );
        let sheet = member(&after, "xl/worksheets/sheet1.xml");
        assert!(
            sheet.contains("r=\"E5\"><f>SUBTOTAL(109,[qty])</f>"),
            "{sheet}"
        );
        assert!(!sheet.contains("r=\"E5\"><f>SUBTOTAL(109,[qty])</f><v>7</v>"));
    }
}

#[test]
fn named_totals_move_refuses_absent_cells_requiring_metadata_synthesis() {
    use crate::excel_package::{member, named_table_package};
    for (kind, absent, from, to) in [
        (
            "label",
            "<c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c>",
            "D4",
            "D5",
        ),
        (
            "function",
            "<c r=\"E4\"><f>SUBTOTAL(109,[qty])</f><v>7</v></c>",
            "E4",
            "E5",
        ),
        (
            "formula",
            "<c r=\"E4\"><f>SUBTOTAL(109,[qty])</f><v>7</v></c>",
            "E4",
            "E5",
        ),
    ] {
        let mut parts = named_totals_resize_parts();
        let sheet = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
            .unwrap()
            .1;
        assert_eq!(sheet.matches(absent).count(), 1);
        *sheet = sheet.replace(absent, "");
        let table = &mut parts
            .iter_mut()
            .find(|(name, _)| *name == "xl/tables/table2.xml")
            .unwrap()
            .1;
        if kind == "label" {
            let column = "<tableColumn id=\"1\" name=\"year\"/>";
            assert_eq!(table.matches(column).count(), 1);
            *table = table.replace(
                column,
                "<tableColumn id=\"1\" name=\"year\" totalsRowLabel=\"Total\"/>",
            );
        } else if kind == "formula" {
            let column = "<tableColumn id=\"2\" name=\"qty\" totalsRowFunction=\"sum\"/>";
            assert_eq!(table.matches(column).count(), 1);
            *table = table.replace(column,
                "<tableColumn id=\"2\" name=\"qty\" totalsRowFunction=\"custom\"><totalsRowFormula>SUM([qty])</totalsRowFormula></tableColumn>");
        }
        let original = named_table_package(&parts);
        let mut same = stored(original.clone());
        overwrite_arrow_reader(
            &mut same,
            quantities_reader(2),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap();
        let unchanged = Workbook::from_bytes(same.read_all_bytes().unwrap()).unwrap();
        assert!(
            unchanged
                .sheet("Data")
                .unwrap()
                .cell(from.parse().unwrap())
                .is_none(),
            "{kind}"
        );
        let before = Workbook::from_bytes(original.clone()).unwrap();
        assert_eq!(
            member(&unchanged, "xl/tables/table2.xml"),
            member(&before, "xl/tables/table2.xml")
        );
        let mut changed = stored(original.clone());
        let (path, reason) = refusal(
            overwrite_arrow_reader(
                &mut changed,
                quantities_reader(3),
                &ExcelOptions::new().with_table("Quantities"),
            )
            .unwrap_err(),
        );
        assert_eq!(path, format!("Data!{to}"), "{kind}");
        assert!(
            reason.contains("physical totals cell")
                && reason.contains(from)
                && reason.contains("Quantities")
                && reason.contains("synthesis"),
            "{kind}: {reason}"
        );
        assert_eq!(changed.read_all_bytes().unwrap(), original, "{kind}");
    }
}

#[test]
fn named_totals_move_refuses_missing_cells_with_changed_row_style() {
    use crate::excel_package::named_table_package;
    let mut parts = named_totals_resize_parts();
    totals_two_styles(&mut parts);
    let worksheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *worksheet = worksheet
        .replace("<row r=\"4\">", "<row r=\"4\" s=\"1\" customFormat=\"1\">")
        .replace("<c r=\"D4\" t=\"inlineStr\"><is><t>Total</t></is></c>", "")
        .replace("<c r=\"E4\"><f>SUBTOTAL(109,[qty])</f><v>7</v></c>", "");
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            quantities_reader(3),
            &ExcelOptions::new().with_table("Quantities"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("Data!"), "{path}: {reason}");
    assert!(reason.contains("style"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_growth_refuses_merged_target_atomically() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet.replace(
        "<tableParts count=\"2\">",
        "<mergeCells count=\"1\"><mergeCell ref=\"A4:B4\"/></mergeCells><tableParts count=\"2\">",
    );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            names_reader(&[9, 10, 11], &[Some("nine"), Some("ten"), Some("eleven")]),
            &ExcelOptions::new().with_table("Names"),
        )
        .unwrap_err(),
    );
    assert!(
        path.contains("Names") || path.contains("A4"),
        "{path}: {reason}"
    );
    assert!(reason.contains("merged"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_shrink_refuses_grouped_formula_in_discarded_row() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut parts = named_table_parts();
    let sheet = &mut parts
        .iter_mut()
        .find(|(part, _)| *part == "xl/worksheets/sheet1.xml")
        .unwrap()
        .1;
    *sheet = sheet
        .replace(
            "<c r=\"A3\"><v>2</v></c>",
            "<c r=\"A3\"><f t=\"shared\" ref=\"A3:A4\" si=\"0\">1+1</f><v>2</v></c>",
        )
        .replace(
            "<row r=\"4\">",
            "<row r=\"4\"><c r=\"A4\"><f t=\"shared\" si=\"0\"/><v>3</v></c>",
        );
    let original = named_table_package(&parts);
    let mut handle = stored(original.clone());
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            names_reader(&[9], &[Some("nine")]),
            &ExcelOptions::new().with_table("Names"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("A3"), "{path}: {reason}");
    assert!(reason.contains("grouped formula"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_growth_late_conversion_refuses_without_writing_handle() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let too_long = "x".repeat(32_768);
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            names_reader(&[9, 10, 11], &[Some("nine"), Some("ten"), Some(&too_long)]),
            &ExcelOptions::new().with_table("Names"),
        )
        .unwrap_err(),
    );
    assert!(path.contains("Data!B4"), "{path}: {reason}");
    assert!(reason.contains("characters"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_write_late_cell_failure_keeps_original_bytes() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let original = named_table_package(&named_table_parts());
    let mut handle = stored(original.clone());
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let first = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![9])),
            Arc::new(StringArray::from(vec![Some("nine")])),
        ],
    )
    .unwrap();
    let too_long = "x".repeat(32_768);
    let second = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![10])),
            Arc::new(StringArray::from(vec![Some(too_long.as_str())])),
        ],
    )
    .unwrap();
    let error = overwrite_arrow_reader(
        &mut handle,
        yggdryl::arrow::batch_reader(schema, [first, second]),
        &ExcelOptions::new().with_table("Names"),
    )
    .unwrap_err();
    let (path, reason) = refusal(error);
    assert!(path.contains("Data!B3"), "{path}: {reason}");
    assert!(reason.contains("characters"), "{reason}");
    assert_eq!(handle.read_all_bytes().unwrap(), original);
}

#[test]
fn named_table_declared_result_schema_is_no_io_but_records_validate_membership() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let counted = Counted::new(stored(named_table_package(&named_table_parts())));
    let calls = Arc::clone(counted.calls());
    let options = RecordOptions::from(ExcelOptions::new().with_table("Missing"))
        .with_field(trades())
        .with_select("id as key")
        .unwrap();
    calls.reset();
    let field = counted.read_arrow_field(&options).unwrap();
    assert_eq!(
        field.fields().iter().map(Field::name).collect::<Vec<_>>(),
        ["key"]
    );
    assert_eq!(calls.snapshot().to_string(), "none");
    let error = match counted.read_arrow_reader(&options) {
        Ok(_) => panic!("a declared schema cannot supply a missing table's records"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("$.table"), "{error}");
    assert!(error.contains("Missing"), "{error}");
}

#[test]
fn named_table_read_after_in_memory_edits_uses_the_unsaved_package_image() {
    use crate::excel_package::{named_table_package, named_table_parts};
    let mut book = Workbook::from_bytes(named_table_package(&named_table_parts())).unwrap();
    book.insert_columns("Data", 0, 1).unwrap();
    book.set_entry("Data", "B2".parse().unwrap(), "7").unwrap();
    book.rename_sheet("Data", "Edited").unwrap();
    // into_bytes writes the live model/part overrides; no save/rebase is needed.
    let media = Excel::new(stored(book.into_bytes().unwrap()))
        .with_options(ExcelOptions::new().with_table("Names"));
    let options = media.record_options().unwrap();
    assert_eq!(rows(&media, &options), ["[7.0,\"one\"]", "[2.0,\"two\"]"]);
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(media.column_size().unwrap(), 2);
}

#[test]
fn rows_declared_column_size_refuses_deep_field_at_schema_boundary() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOMedia, StructType};
    use yggdryl::{
        RecordHeader,
        excel::{Excel, ExcelOptions},
    };

    let mut child = DataType::Float64.required_field("value");
    for _ in 0..70 {
        child = DataType::from(StructType::from_fields([child]).unwrap()).required_field("group");
    }
    let root = DataType::from(StructType::from_fields([child]).unwrap()).required_field("row");
    let media = Excel::new(Buffer::new()).with_options(
        ExcelOptions::new()
            .with_header(RecordHeader::Rows(2))
            .with_field(root),
    );
    let error = media.column_size().unwrap_err().to_string();
    assert!(
        error.contains("Field") && error.contains("limit"),
        "{error}"
    );
}

#[test]
fn direct_excel_writer_refuses_an_oversized_null_batch_before_landing() {
    use arrow_array::NullArray;
    use yggdryl::excel::MAX_ROWS;

    let field =
        DataType::from(StructType::from_fields([DataType::Null.nullable_field("Blank")]).unwrap())
            .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let huge = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(NullArray::new(usize::MAX))],
    )
    .unwrap();
    for header in [
        RecordHeader::None,
        RecordHeader::Source,
        RecordHeader::Rows(2),
    ] {
        let mut handle = xlsx();
        let options = ExcelOptions::new().with_header(header);
        let batches = yggdryl::arrow::batch_reader(Arc::clone(&schema), [huge.clone()]);
        let (path, reason) =
            refusal(overwrite_arrow_reader(&mut handle, batches, &options).unwrap_err());
        assert_eq!(path, format!("Sheet1!A{}", MAX_ROWS + 1), "{header:?}");
        assert!(reason.contains("expected at most 1048576 rows"), "{reason}");
        assert_eq!(handle.size(), 0, "{header:?} is atomic");
    }
}

#[test]
fn direct_excel_writer_refuses_directly_constructed_out_of_grid_anchor() {
    use yggdryl::excel::{CellRef, MAX_ROWS};

    for row in [MAX_ROWS, u32::MAX] {
        for empty in [false, true] {
            let mut handle = xlsx();
            let at = CellRef::new(row, 0);
            let options = ExcelOptions::new()
                .with_header(RecordHeader::None)
                .with_range(CellRange::new(at, at));
            let batches = if empty {
                reader(&[], &[])
            } else {
                reader(&[1], &[Some("AAPL")])
            };
            let (path, reason) =
                refusal(overwrite_arrow_reader(&mut handle, batches, &options).unwrap_err());
            // The options boundary locates the invalid selection before
            // the writer reaches its cell guard, including an empty stream.
            assert_eq!(path, "$.range", "row {row}, empty {empty}");
            assert!(
                reason.contains("expected a cell within 1048576 rows"),
                "{reason}"
            );
            assert!(
                reason.contains(&format!("got row {} column 1", u64::from(row) + 1)),
                "{reason}"
            );
            assert_eq!(handle.size(), 0, "row {row}, empty {empty} is atomic");
        }
    }
}

#[test]
fn direct_excel_writer_counts_earlier_batches_before_landing_the_next() {
    use arrow_array::NullArray;
    use yggdryl::excel::MAX_ROWS;

    let field =
        DataType::from(StructType::from_fields([DataType::Null.nullable_field("Blank")]).unwrap())
            .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let batches = [
        RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(NullArray::new(1))]).unwrap(),
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(NullArray::new(MAX_ROWS as usize))],
        )
        .unwrap(),
    ];
    let mut handle = xlsx();
    let options = ExcelOptions::new().with_header(RecordHeader::None);
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut handle,
            yggdryl::arrow::batch_reader(schema, batches),
            &options,
        )
        .unwrap_err(),
    );
    assert_eq!(path, format!("Sheet1!A{}", MAX_ROWS + 1));
    assert!(reason.contains("expected at most 1048576 rows"), "{reason}");
    assert_eq!(handle.size(), 0);
}

#[test]
fn named_table_write_keeps_allocated_default_styles_from_excel() {
    use crate::excel_package::{named_table_package, named_table_parts};
    use arrow_array::Date32Array;

    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/allocated_styles_excel.json")).unwrap();
    for case in evidence["cases"].as_array().unwrap() {
        let owner = case["owner"].as_str().unwrap();
        let temporal = case["operation"].as_str().unwrap() == "date";
        for presence in ["absent_s", "explicit_zero", "unallocated"] {
            let mut parts = named_table_parts();
            totals_two_styles(&mut parts);
            let styles = &mut parts
                .iter_mut()
                .find(|(name, _)| *name == "xl/styles.xml")
                .unwrap()
                .1;
            let original_fonts =
                "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>";
            assert_eq!(styles.matches(original_fonts).count(), 1);
            *styles = styles.replace(original_fonts,
                "<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>");
            let original_xf = "<xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>";
            assert_eq!(styles.matches(original_xf).count(), 1);
            *styles = styles.replace(original_xf,
                "<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>");
            let sheet = &mut parts
                .iter_mut()
                .find(|(name, _)| *name == "xl/worksheets/sheet1.xml")
                .unwrap()
                .1;
            match owner {
                "row" => {
                    *sheet =
                        sheet.replace("<row r=\"2\">", "<row r=\"2\" s=\"1\" customFormat=\"1\">")
                }
                "column" => {
                    *sheet = sheet.replace(
                        "<sheetData>",
                        "<cols><col min=\"4\" max=\"4\" style=\"1\"/></cols><sheetData>",
                    )
                }
                other => panic!("unexpected native fixture owner {other}"),
            }
            let cell = "<c r=\"D2\"><v>2024</v></c>";
            assert_eq!(sheet.matches(cell).count(), 1);
            match presence {
                "absent_s" => {}
                "explicit_zero" => {
                    *sheet = sheet.replace(cell, "<c r=\"D2\" s=\"0\"><v>2024</v></c>")
                }
                "unallocated" => *sheet = sheet.replace(cell, ""),
                _ => unreachable!(),
            }
            let at = "D2".parse().unwrap();
            let bytes = named_table_package(&parts);
            let before = Workbook::from_bytes(bytes.clone()).unwrap();
            let expected = &case["observations"][presence];
            assert_eq!(
                before.cell_style("Data", at).unwrap().font.bold,
                expected["before"]["font_bold"].as_bool().unwrap(),
                "native before: {owner}/{presence}/temporal={temporal}"
            );
            let kind = if temporal {
                DataType::Date32
            } else {
                DataType::Int64
            };
            let field = DataType::from(
                StructType::from_fields([
                    kind.required_field("year"),
                    DataType::Int64.required_field("qty"),
                ])
                .unwrap(),
            )
            .required_field("row");
            let schema = field.into_arrow_schema().unwrap();
            let values: arrow_array::ArrayRef = if temporal {
                Arc::new(Date32Array::from(vec![19_723, 19_724]))
            } else {
                Arc::new(Int64Array::from(vec![123, 124]))
            };
            let input = RecordBatch::try_new(
                schema.clone(),
                vec![values, Arc::new(Int64Array::from(vec![6, 8]))],
            )
            .unwrap();
            let mut output = stored(bytes);
            overwrite_arrow_reader(
                &mut output,
                yggdryl::arrow::batch_reader(schema, [input]),
                &ExcelOptions::new().with_table("Quantities"),
            )
            .unwrap();
            let after = Workbook::from_bytes(output.read_all_bytes().unwrap()).unwrap();
            // Excel16.0 build20430: allocated no-s and s=0 cells keep XF0;
            // only a genuinely absent cell inherits the row/column style.
            assert_eq!(
                after.cell_style("Data", at).unwrap().font.bold,
                expected["reopened"]["font_bold"].as_bool().unwrap(),
                "native after: {owner}/{presence}/temporal={temporal}"
            );
            assert_eq!(
                after.sheet("Data").unwrap().scalar("E2".parse().unwrap()),
                Scalar::from(6.0)
            );
        }
    }
}

#[test]
fn named_table_write_inherits_missing_cells_row_or_column_style() {
    use crate::excel_package::{named_table_package, named_table_parts};
    use arrow_array::Date32Array;
    use yggdryl::excel::StylePatch;

    for range in ["2:2", "D:D"] {
        for temporal in [false, true] {
            let mut before =
                Workbook::from_bytes(named_table_package(&named_table_parts())).unwrap();
            before
                .set_style(
                    "Data",
                    &[range.parse().unwrap()],
                    &StylePatch {
                        bold: Some(true),
                        ..StylePatch::default()
                    },
                )
                .unwrap();
            before
                .sheet_mut("Data")
                .unwrap()
                .remove_cell("D2".parse().unwrap())
                .unwrap();
            assert!(
                before
                    .sheet("Data")
                    .unwrap()
                    .cell("D2".parse().unwrap())
                    .is_none()
            );
            assert!(
                before
                    .cell_style("Data", "D2".parse().unwrap())
                    .unwrap()
                    .font
                    .bold
            );
            let mut handle = stored(before.into_bytes().unwrap());
            let first = if temporal {
                DataType::Date32
            } else {
                DataType::Int64
            };
            let field = DataType::from(
                StructType::from_fields([
                    first.required_field("year"),
                    DataType::Int64.required_field("qty"),
                ])
                .unwrap(),
            )
            .required_field("row");
            let schema = field.into_arrow_schema().unwrap();
            let values: arrow_array::ArrayRef = if temporal {
                Arc::new(Date32Array::from(vec![19_723, 19_724]))
            } else {
                Arc::new(Int64Array::from(vec![2030, 2031]))
            };
            let input = RecordBatch::try_new(
                schema.clone(),
                vec![values, Arc::new(Int64Array::from(vec![6, 8]))],
            )
            .unwrap();
            overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(schema, [input]),
                &ExcelOptions::new().with_table("Quantities"),
            )
            .unwrap();
            let after = Workbook::from_bytes(handle.read_all_bytes().unwrap()).unwrap();
            assert!(
                after
                    .cell_style("Data", "D2".parse().unwrap())
                    .unwrap()
                    .font
                    .bold,
                "new body cell must keep its inherited {range} font (temporal={temporal})"
            );
            assert_eq!(
                after.sheet("Data").unwrap().scalar("E2".parse().unwrap()),
                Scalar::from(6.0)
            );
        }
    }
}

#[test]
fn named_table_write_split_and_empty_batches_match_one_batch_exactly() {
    use crate::excel_package::{member, named_table_package, named_table_parts};
    let parts = named_table_parts();
    let original = named_table_package(&parts);
    let ids = [9, 10];
    let labels = [Some("nine"), None];
    let options = ExcelOptions::new()
        .with_table("Names")
        .with_header(RecordHeader::None);
    let mut expected = None;
    for lengths in [&[2][..], &[1, 1][..], &[0, 1, 0, 1, 0, 0][..]] {
        let mut offset = 0;
        let batches = lengths
            .iter()
            .map(|&length| {
                let next = offset + length;
                let result = batch(&ids[offset..next], &labels[offset..next]);
                offset = next;
                result
            })
            .collect::<Vec<_>>();
        assert_eq!(offset, ids.len());
        let mut output = stored(original.clone());
        overwrite_arrow_reader(
            &mut output,
            yggdryl::arrow::batch_reader(trades().into_arrow_schema().unwrap(), batches),
            &options,
        )
        .unwrap();
        let bytes = output.read_all_bytes().unwrap();
        let media = Excel::new(stored(bytes.clone()));
        assert_eq!(
            rows(
                &media,
                &RecordOptions::from(ExcelOptions::new().with_table("Names"))
            ),
            ["[9.0,\"nine\"]", "[10.0,null]"],
            "batches {lengths:?}"
        );
        let book = Workbook::from_bytes(bytes).unwrap();
        // The batch boundary changes neither rewritten XML nor any carried part.
        let actual = parts
            .iter()
            .map(|(name, _)| (*name, member(&book, name)))
            .collect::<std::collections::BTreeMap<_, _>>();
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected, "batches {lengths:?}");
        } else {
            expected = Some(actual);
        }
    }
}
