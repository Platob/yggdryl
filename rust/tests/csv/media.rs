//! `rust/src/csv/media.rs`: the `.csv` and `.tsv` record medium - the free
//! doors over any byte handle and the `Csv` wrapper answering the ordinary
//! `IOMedia` and `IOBase` surfaces - over in-memory buffers, local files,
//! coded names, declared charsets and `Media`.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::arrow::BatchReader;
use yggdryl::csv::{Csv, CsvOptions, overwrite_arrow_reader, read_batch_reader, read_field};
use yggdryl::expression::Selector;
use yggdryl::holder::counted::{Call, Counted};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, Media, RecordOptions};
use yggdryl::{Codec, DataType, Error, Field, IOBase, IOMedia, MediaType, MimeType};

type Row = (i64, Option<String>);

/// The trades table: a required `id` and a nullable `symbol`.
/// One of the two doors a path opens as a holder through.
type Opener = fn(&std::path::Path) -> yggdryl::Result<Holder>;

fn trades_field() -> Field {
    DataType::from_str("struct<id: int64 not null, symbol: utf8>")
        .expect("a valid root")
        .required_field("row")
}

fn batch(rows: &[(i64, Option<&str>)]) -> RecordBatch {
    RecordBatch::try_new(
        trades_field().into_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|(id, _)| *id))),
            Arc::new(StringArray::from_iter(
                rows.iter().map(|(_, symbol)| *symbol),
            )),
        ],
    )
    .expect("a batch")
}

fn reader(batches: Vec<RecordBatch>) -> BatchReader {
    yggdryl::arrow::batch_reader(
        trades_field().into_arrow_schema().expect("an Arrow schema"),
        batches,
    )
}

/// Four rows in two batches.
fn two_batches() -> BatchReader {
    reader(vec![
        batch(&[(1, Some("AAPL")), (2, None)]),
        batch(&[(3, Some("MSFT")), (4, Some("GOOG"))]),
    ])
}

fn owned(rows: &[(i64, Option<&str>)]) -> Vec<Row> {
    rows.iter()
        .map(|(id, symbol)| (*id, symbol.map(str::to_owned)))
        .collect()
}

fn four_rows() -> Vec<Row> {
    owned(&[
        (1, Some("AAPL")),
        (2, None),
        (3, Some("MSFT")),
        (4, Some("GOOG")),
    ])
}

/// Every row a reader yields, as `(id, symbol)`.
fn rows_of(reader: BatchReader) -> Vec<Row> {
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.expect("a batch");
        let ids = batch
            .column_by_name("id")
            .expect("an id column")
            .as_primitive::<Int64Type>();
        let symbols = batch
            .column_by_name("symbol")
            .expect("a symbol column")
            .as_string::<i32>();
        for index in 0..batch.num_rows() {
            rows.push((
                ids.value(index),
                symbols
                    .is_valid(index)
                    .then(|| symbols.value(index).to_owned()),
            ));
        }
    }
    rows
}

/// How many batches and how many rows a reader yields.
fn counts(reader: BatchReader) -> (usize, usize) {
    reader.fold((0, 0), |(batches, rows), batch| {
        (batches + 1, rows + batch.expect("a batch").num_rows())
    })
}

/// An empty buffer whose media type comes from `name`.
fn buffer(name: &str) -> Buffer {
    Buffer::new().with_media_type(MediaType::from_file_name(name))
}

/// A `.csv` buffer holding `document`.
fn stored(document: &str) -> Buffer {
    let mut held = buffer("trades.csv");
    held.write_all_bytes(document.as_bytes())
        .expect("the document is written");
    held
}

fn text(handle: &(impl IOBase + ?Sized)) -> String {
    String::from_utf8(handle.read_all_bytes().expect("the bytes")).expect("UTF-8")
}

/// The path and the reason of an `InvalidRecord` refusal.
fn refusal(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other:?}"),
    }
}

/// A fresh folder under the temporary directory, named after `label`.
fn temporary(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!("yggdryl-csv-media-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

// Refusals.

#[test]
fn record_options_of_another_encoding_are_refused_naming_both() {
    let mut media = Csv::new(stored("id,symbol\n1,AAPL\n"));
    let before = media.read_all_bytes().expect("the bytes");
    let ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options");
    let expected = (
        "$.encoding".to_owned(),
        "expected CSV record options, got application/vnd.apache.arrow.stream".to_owned(),
    );
    assert_eq!(refusal(media.read_arrow_field(&ipc).unwrap_err()), expected);
    assert_eq!(
        refusal(
            media
                .overwrite_arrow_reader(two_batches(), &ipc)
                .unwrap_err()
        ),
        expected
    );
    assert_eq!(
        refusal(media.append_arrow_reader(two_batches(), &ipc).unwrap_err()),
        expected
    );
    let mut keyed = ipc.clone();
    keyed.set_merge_by(Selector::from_columns(["id"]));
    assert_eq!(
        refusal(media.merge_arrow_reader(two_batches(), &keyed).unwrap_err()),
        expected
    );
    assert_eq!(
        refusal(
            media
                .overwrite_prepared_arrow_reader(two_batches(), &ipc)
                .unwrap_err()
        ),
        expected
    );
    assert_eq!(media.read_all_bytes().expect("the bytes"), before);
}

#[test]
fn a_merge_without_a_key_is_refused_naming_the_mode() {
    let mut media = Csv::new(stored("id,symbol\n1,AAPL\n"));
    let options = media.record_options().expect("the options");
    let (path, reason) = refusal(
        media
            .merge_arrow_reader(two_batches(), &options)
            .unwrap_err(),
    );
    assert_eq!(path, "$.merge_by");
    assert!(
        reason.contains("requires at least one merge_by column"),
        "{reason}"
    );
}

// The free functions.

#[test]
fn a_missing_document_reads_as_zero_rows_under_the_declared_schema() {
    let held = buffer("trades.csv");
    let declared = trades_field();
    let reader = read_batch_reader(&held, Some(&declared), &CsvOptions::new()).expect("a reader");
    assert_eq!(
        reader.schema(),
        declared
            .clone()
            .into_arrow_schema()
            .expect("an Arrow schema")
    );
    assert_eq!(counts(reader), (0, 0));

    let mut options = CsvOptions::new();
    options.set_field(declared.clone());
    let reader = read_batch_reader(&held, None, &options).expect("a reader");
    assert_eq!(counts(reader), (0, 0));
    assert_eq!(read_field(&held, &options).expect("the field"), declared);

    let reader = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
    assert!(reader.schema().fields().is_empty());
    assert_eq!(counts(reader), (0, 0));
    let (path, reason) = refusal(read_field(&held, &CsvOptions::new()).unwrap_err());
    assert_eq!(path, "$.csv");
    assert_eq!(reason, "an empty document declares no schema");
}

#[test]
fn the_document_is_written_and_read_back_through_the_free_doors() {
    let mut held = buffer("trades.csv");
    overwrite_arrow_reader(&mut held, two_batches(), &CsvOptions::new()).expect("written");
    assert_eq!(text(&held), "id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n");
    let inferred = read_field(&held, &CsvOptions::new()).expect("the field");
    assert_eq!(
        inferred.dtype(),
        &DataType::from_str("struct<id: int64, symbol: utf8>").expect("inferred")
    );
    let reader = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), four_rows());
    let reader =
        read_batch_reader(&held, Some(&trades_field()), &CsvOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), four_rows());
}

#[test]
fn a_document_another_writer_saved_reads_with_its_header() {
    let held = stored("id,symbol\r\n1,AAPL\r\n2,\r\n");
    let reader = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL")), (2, None)]));
}

#[test]
fn the_root_is_named_after_the_options() {
    let held = stored("id,symbol\n1,AAPL\n");
    let mut options = CsvOptions::new();
    options.set_name("trade".into());
    let field = read_field(&held, &options).expect("the field");
    assert_eq!(field.name(), "trade");
    let reader = read_batch_reader(&held, None, &options).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL"))]));
}

// Coded names and declared charsets.

#[test]
fn a_coded_name_compresses_the_document_and_reads_it_back() {
    for (name, codec, magic) in [
        ("trades.csv.gz", Codec::Gzip, &[0x1F, 0x8B][..]),
        ("trades.csv.zst", Codec::Zstd, &[0x28, 0xB5, 0x2F, 0xFD][..]),
        ("trades.csv.zz", Codec::Zlib, &[0x78][..]),
    ] {
        let mut held = buffer(name);
        assert_eq!(held.media_type().base(), &MimeType::CSV, "{name}");
        overwrite_arrow_reader(&mut held, two_batches(), &CsvOptions::new()).expect("written");
        let encoded = held.read_all_bytes().expect("the bytes");
        assert_eq!(&encoded[..magic.len()], magic, "{name}");
        let decoded = codec.load(&encoded).expect("decodes");
        assert_eq!(
            decoded, b"id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n",
            "{name}"
        );
        let read = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
        assert_eq!(rows_of(read), four_rows(), "{name}");

        // And the wrapper over a holder, every dimension included.
        let mut media = Csv::new(Holder::buffer(buffer(name)));
        let options = media.record_options().expect("the options");
        media
            .overwrite_arrow_reader(two_batches(), &options)
            .expect("written");
        assert_eq!(
            &media.read_all_bytes().expect("the bytes")[..magic.len()],
            magic
        );
        assert_eq!(
            rows_of(media.read_arrow_reader(&options).expect("a reader")),
            four_rows()
        );
        assert_eq!(media.row_size().expect("the rows"), 4, "{name}");
        assert_eq!(media.column_size().expect("the columns"), 2, "{name}");
        media
            .append_arrow_reader(reader(vec![batch(&[(5, Some("IBM"))])]), &options)
            .expect("appended");
        assert_eq!(media.row_size().expect("the rows"), 5, "{name}");
        assert_eq!(
            codec
                .load(&media.read_all_bytes().expect("the bytes"))
                .expect("decodes"),
            b"id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n5,IBM\n",
            "{name}: the header once"
        );
    }
}

#[test]
fn a_tsv_name_reads_and_writes_tabs() {
    let mut media = Csv::new(Holder::buffer(buffer("trades.tsv")));
    let options = media.record_options().expect("the options");
    assert_eq!(options.mime_type(), MimeType::TSV);
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(text(&media), "id\tsymbol\n1\tAAPL\n2\t\n3\tMSFT\n4\tGOOG\n");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
}

// The wrapper.

#[test]
fn the_wrapper_retains_its_options_and_hands_back_its_handle() {
    let media = Csv::new(buffer("trades.csv"));
    assert_eq!(media.options(), &CsvOptions::new());
    let media = media.with_field(trades_field().with_name("trade"));
    assert_eq!(
        media.options().field,
        Some(trades_field().with_name("trade"))
    );
    assert_eq!(media.options().name.as_str(), "trade");

    let configured = CsvOptions::tsv().with_header(false);
    let mut media = media.with_options(configured.clone());
    assert_eq!(media.options(), &configured);
    assert_eq!(media.options().field, None);
    media.options_mut().set_field(trades_field());
    assert_eq!(media.options().field(), Some(trades_field()));

    media
        .handle_mut()
        .write_all_bytes(b"a,b\n")
        .expect("written through the handle");
    assert_eq!(media.handle().size(), 4);
    let held: Buffer = media.into_handle();
    assert_eq!(held.into_bytes(), b"a,b\n");
}

#[test]
fn record_options_are_the_csv_options_carrying_the_declared_field() {
    let media = Csv::new(buffer("trades.csv"))
        .with_options(CsvOptions::new().with_separator(b';').expect("a separator"))
        .with_field(trades_field());
    let options = media.record_options().expect("the options");
    let RecordOptions::Csv(csv) = &options else {
        panic!("expected CSV options, got {options:?}");
    };
    assert_eq!(csv, media.options());
    assert_eq!(csv.separator(), b';');
    assert_eq!(options.field(), Some(trades_field()));
    assert_eq!(options.mime_type(), MimeType::CSV);
}

#[test]
fn read_arrow_field_prefers_the_declared_field_else_reads_the_document() {
    let media = Csv::new(stored("id,symbol\n1,AAPL\n"));
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field").dtype(),
        &DataType::from_str("struct<id: int64, symbol: utf8>").expect("inferred")
    );
    let declared =
        DataType::from_str("struct<id: int32 not null, symbol: large_utf8, price: float64>")
            .expect("a root")
            .required_field("trade");
    let media = Csv::new(stored("id,symbol\n1,AAPL\n")).with_field(declared.clone());
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        declared
    );
    let media = Csv::new(buffer("trades.csv")).with_field(declared.clone());
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        declared
    );
}

#[test]
fn overwrite_append_and_merge_through_the_wrapper() {
    let mut media = Csv::new(buffer("trades.csv"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    media
        .overwrite_arrow_reader(reader(vec![batch(&[(7, Some("IBM"))])]), &options)
        .expect("replaced");
    assert_eq!(text(&media), "id,symbol\n7,IBM\n");

    // An append into nothing is the first write; a later one adds records
    // after the tail, the header written once.
    let mut media = Csv::new(buffer("trades.csv"));
    media
        .append_arrow_reader(
            reader(vec![batch(&[(1, Some("AAPL")), (2, None)])]),
            &options,
        )
        .expect("appended");
    media
        .append_arrow_reader(
            reader(vec![batch(&[(3, Some("MSFT")), (4, Some("GOOG"))])]),
            &options,
        )
        .expect("appended");
    assert_eq!(text(&media), "id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );

    // A stored document lacking its final terminator gets one first.
    let mut media = Csv::new(stored("id,symbol\n1,AAPL"));
    media
        .append_arrow_reader(reader(vec![batch(&[(2, None)])]), &options)
        .expect("appended");
    assert_eq!(text(&media), "id,symbol\n1,AAPL\n2,\n");

    // An append casts onto the stored shape: the stored column order wins.
    let mut media = Csv::new(stored("symbol,id\nAAPL,1\n"));
    media
        .append_arrow_reader(reader(vec![batch(&[(2, Some("MSFT"))])]), &options)
        .expect("appended");
    assert_eq!(text(&media), "symbol,id\nAAPL,1\nMSFT,2\n");

    // A merge updates the matched key and appends the rest.
    let mut media = Csv::new(stored("id,symbol\n1,AAPL\n2,\n"));
    let mut keyed = options.clone();
    keyed.set_merge_by(Selector::from_columns(["id"]));
    media
        .merge_arrow_reader(
            reader(vec![batch(&[(2, Some("MSFT")), (3, Some("GOOG"))])]),
            &keyed,
        )
        .expect("merged");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        owned(&[(1, Some("AAPL")), (2, Some("MSFT")), (3, Some("GOOG"))])
    );
}

#[test]
fn a_write_under_a_dialect_reads_back_under_the_same_dialect() {
    // The stored shape a write completes onto is read under the options'
    // own separator: a `;` header is two columns, not one.
    let mut media = Csv::new(buffer("trades.csv"))
        .with_options(CsvOptions::new().with_separator(b';').expect("a separator"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(reader(vec![batch(&[(1, Some("AAPL"))])]), &options)
        .expect("written");
    media
        .append_arrow_reader(reader(vec![batch(&[(2, Some("MSFT"))])]), &options)
        .expect("appended");
    assert_eq!(text(&media), "id;symbol\n1;AAPL\n2;MSFT\n");
    assert_eq!(media.column_size().expect("the columns"), 2);
    let mut keyed = options.clone();
    keyed.set_merge_by(Selector::from_columns(["id"]));
    media
        .merge_arrow_reader(reader(vec![batch(&[(2, Some("GOOG"))])]), &keyed)
        .expect("merged");
    assert_eq!(text(&media), "id;symbol\n1;AAPL\n2;GOOG\n");
}

#[test]
fn row_size_and_column_size_answer_from_the_document_or_the_declared_field() {
    let mut media = Csv::new(buffer("trades.csv"));
    assert_eq!(media.row_size().expect("the rows"), 0);
    assert_eq!(media.column_size().expect("the columns"), 0);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(media.row_size().expect("the rows"), 4);
    assert_eq!(media.column_size().expect("the columns"), 2);

    let wide = DataType::from_str("struct<id: int64, symbol: utf8, venue: utf8>")
        .expect("a root")
        .required_field("row");
    let declared = Csv::new(buffer("trades.csv")).with_field(wide);
    assert_eq!(declared.column_size().expect("the columns"), 3);
    assert_eq!(declared.row_size().expect("the rows"), 0);

    // A header alone is columns and no rows.
    let header = Csv::new(stored("id,symbol\n"));
    assert_eq!(header.column_size().expect("the columns"), 2);
    assert_eq!(header.row_size().expect("the rows"), 0);
    let options = header.record_options().expect("the options");
    assert_eq!(
        header
            .read_arrow_field(&options)
            .expect("the field")
            .dtype(),
        &DataType::from_str("struct<id: utf8, symbol: utf8>").expect("all text")
    );
    assert_eq!(
        counts(header.read_arrow_reader(&options).expect("a reader")),
        (0, 0)
    );
}

#[test]
fn a_commit_cadence_publishes_every_bounded_prefix() {
    let three = || {
        reader(vec![batch(&[
            (1, Some("AAPL")),
            (2, None),
            (3, Some("MSFT")),
        ])])
    };
    let counted = Counted::new(buffer("trades.csv"));
    let calls = Arc::clone(counted.calls());
    let mut media = Csv::new(counted);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(three(), &options)
        .expect("written");
    assert_eq!(calls.get(Call::Truncate), 1, "one publication");

    let counted = Counted::new(buffer("trades.csv"));
    let calls = Arc::clone(counted.calls());
    let mut media = Csv::new(counted);
    media.options_mut().commit_row_size = Some(1);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(three(), &options)
        .expect("written");
    // The first cadence overwrites, the later two append in place.
    assert_eq!(calls.get(Call::Truncate), 1, "one truncation");
    assert_eq!(calls.get(Call::Pwrite), 3, "one write per cadence");
    assert_eq!(text(&media), "id,symbol\n1,AAPL\n2,\n3,MSFT\n");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        owned(&[(1, Some("AAPL")), (2, None), (3, Some("MSFT"))])
    );

    let mut zero = options.clone();
    zero.set_commit_row_size(Some(0));
    let (path, _) = refusal(media.overwrite_arrow_reader(three(), &zero).unwrap_err());
    assert_eq!(path, "$.commit_row_size");
}

#[test]
fn open_caches_the_schema_until_close() {
    let counted = Counted::new(buffer("trades.csv"));
    let calls = Arc::clone(counted.calls());
    let mut media = Csv::new(counted);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");

    // Closed, every ask reads the document.
    calls.reset();
    media.read_arrow_field(&options).expect("the field");
    media.read_arrow_field(&options).expect("the field");
    assert_eq!(calls.get(Call::PstreamBytes), 2);

    // Open, one read answers the schema and the width.
    calls.reset();
    media.open().expect("opened");
    assert!(media.opened());
    media.read_arrow_field(&options).expect("the field");
    media.read_arrow_field(&options).expect("the field");
    assert_eq!(media.column_size().expect("the columns"), 2);
    assert_eq!(calls.get(Call::PstreamBytes), 1);

    // A different root name is answered from the same cache.
    let mut renamed = options.clone();
    renamed.set_name("trade".into());
    assert_eq!(
        media.read_arrow_field(&renamed).expect("the field").name(),
        "trade"
    );
    assert_eq!(calls.get(Call::PstreamBytes), 1);

    // A write while open drops the cache.
    media
        .write_all_bytes(b"id,symbol,venue\n1,AAPL,XNAS\n")
        .expect("replaced");
    assert_eq!(media.column_size().expect("the columns"), 3);
    media.clear().expect("cleared");
    assert_eq!(media.column_size().expect("the columns"), 0);

    media.close().expect("closed");
    assert!(!media.opened());
    calls.reset();
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written afresh");
    media.read_arrow_field(&options).expect("the field");
    media.read_arrow_field(&options).expect("the field");
    assert_eq!(calls.get(Call::PstreamBytes), 2);
}

#[test]
fn the_byte_surface_mirrors_the_handle() {
    let mut media = Csv::new(buffer("trades.csv"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(media.size(), media.handle().size());
    assert_eq!(
        media.read_range_bytes(0, 9).expect("a range"),
        b"id,symbol".to_vec()
    );
    assert_eq!(media.media_type().base(), &MimeType::CSV);
    assert!(media.is_tabular());
    assert!(!media.is_atomic());
    media.clear().expect("cleared");
    assert_eq!(media.size(), 0);
    assert_eq!(media.row_size().expect("the rows"), 0);
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    media.open().expect("opened");
    media.remove(false).expect("removed");
    assert!(!media.opened());
    assert_eq!(media.size(), 0);
}

// The record surface's bounds and clauses.

#[test]
fn the_record_surface_applies_batches_limits_offsets_filters_and_selections() {
    let mut media = Csv::new(buffer("trades.csv"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");

    let batched = options.clone().with_batch_row_size(3);
    assert_eq!(
        counts(media.read_arrow_reader(&batched).expect("a reader")),
        (2, 4)
    );
    let limited = options.clone().with_max_row_size(2);
    assert_eq!(
        rows_of(media.read_arrow_reader(&limited).expect("a reader")),
        owned(&[(1, Some("AAPL")), (2, None)])
    );
    let offset = options.clone().with_row_offset(3);
    assert_eq!(
        rows_of(media.read_arrow_reader(&offset).expect("a reader")),
        owned(&[(4, Some("GOOG"))])
    );
    let filtered = options
        .clone()
        .with_filter("symbol is not null and id > 1")
        .expect("a filter");
    assert_eq!(
        rows_of(media.read_arrow_reader(&filtered).expect("a reader")),
        owned(&[(3, Some("MSFT")), (4, Some("GOOG"))])
    );
    let selected = options.clone().with_select("id").expect("a selector");
    let batches: Vec<RecordBatch> = media
        .read_arrow_reader(&selected)
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("the batches");
    assert_eq!(batches[0].num_columns(), 1);
    assert_eq!(
        media
            .read_arrow_field(&selected)
            .expect("the field")
            .field_len(),
        1
    );
    // A byte bound closes a batch early too.
    let bytes = options.clone().with_batch_byte_size(1);
    assert_eq!(
        counts(media.read_arrow_reader(&bytes).expect("a reader")),
        (4, 4)
    );
}

// Local files and `Media`.

#[test]
fn a_local_csv_file_is_written_read_appended_and_reopened() {
    let root = temporary("local");
    let missing = Holder::file(root.join("missing.csv")).expect("a file");
    assert_eq!(
        counts(
            read_batch_reader(&missing, Some(&trades_field()), &CsvOptions::new())
                .expect("a reader")
        ),
        (0, 0)
    );
    let path = root.join("trades.csv");
    let mut file = Holder::file(&path).expect("a file");
    overwrite_arrow_reader(&mut file, two_batches(), &CsvOptions::new()).expect("written");
    assert_eq!(
        std::fs::read(&path).expect("on disk"),
        b"id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n"
    );
    let reopened = Holder::file(&path).expect("a file");
    assert_eq!(
        read_field(&reopened, &CsvOptions::new())
            .expect("the field")
            .dtype(),
        &DataType::from_str("struct<id: int64, symbol: utf8>").expect("inferred")
    );
    assert_eq!(
        rows_of(read_batch_reader(&reopened, None, &CsvOptions::new()).expect("a reader")),
        four_rows()
    );

    // Through the holder's own record surface: the name picks the encoding.
    let mut holder = Holder::file(&path).expect("a file");
    let options = holder.record_options().expect("options");
    assert!(matches!(options, RecordOptions::Csv(_)));
    holder
        .append_arrow_reader(reader(vec![batch(&[(5, Some("IBM"))])]), &options)
        .expect("appended");
    assert_eq!(holder.row_size().expect("the rows"), 5);
    assert_eq!(holder.column_size().expect("the columns"), 2);
    assert!(
        std::fs::read(&path)
            .expect("on disk")
            .ends_with(b"4,GOOG\n5,IBM\n")
    );

    // Compressed by name.
    let coded = root.join("trades.csv.gz");
    let mut media = Csv::new(Holder::file(&coded).expect("a file"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(&std::fs::read(&coded).expect("on disk")[..2], &[0x1F, 0x8B]);
    let media = Csv::new(Holder::file(&coded).expect("a file"));
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    assert_eq!(media.row_size().expect("the rows"), 4);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_composed_local_handle_reads_its_coded_document_through_the_coding() {
    // The shape `Holder::from_url` and both bindings build for `trades.csv.gz`:
    // the CSV medium over a gzip view over the location. A reader that
    // outlives the borrow reopens the location for a stream of its own, and
    // the bytes there are the coded ones.
    let root = temporary("composed");
    let openers: [(&str, Opener); 2] = [
        ("local", |path| Holder::local(path)),
        ("file", |path| Holder::file(path)),
    ];
    for (label, open) in openers {
        let path = root.join(format!("{label}.csv.gz"));
        let mut holder = open(&path).expect("a location").into_declared_media();
        assert!(
            matches!(&holder, Holder::Media(media) if matches!(media.as_ref(), Media::Csv(_))),
            "{label}"
        );
        let options = holder.record_options().expect("the options");
        assert!(matches!(options, RecordOptions::Csv(_)), "{label}");
        holder
            .overwrite_arrow_reader(two_batches(), &options)
            .expect("written");
        let stored = std::fs::read(&path).expect("on disk");
        assert_eq!(&stored[..2], &[0x1F, 0x8B], "{label}");
        assert_eq!(
            Codec::Gzip.load(&stored).expect("decodes"),
            b"id,symbol\n1,AAPL\n2,\n3,MSFT\n4,GOOG\n",
            "{label}"
        );

        // Identical calls on the way back; only the name says gzip.
        let holder = open(&path).expect("a location").into_declared_media();
        assert_eq!(
            holder
                .read_arrow_field(&options)
                .expect("the field")
                .dtype(),
            &DataType::from_str("struct<id: int64, symbol: utf8>").expect("inferred"),
            "{label}"
        );
        assert_eq!(holder.row_size().expect("the rows"), 4, "{label}");
        assert_eq!(
            rows_of(holder.read_arrow_reader(&options).expect("a reader")),
            four_rows(),
            "{label}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn media_open_binds_the_csv_implementation_by_name() {
    for name in ["trades.csv", "trades.tsv", "trades.csv.zst"] {
        let mut media = Media::open(Holder::buffer(buffer(name)))
            .expect("a media")
            .with_field(trades_field());
        assert!(matches!(media, Media::Csv(_)), "{name}");
        let options = media.record_options().expect("options");
        media
            .overwrite_arrow_reader(two_batches(), &options)
            .expect("written");
        assert_eq!(
            rows_of(media.read_arrow_reader(&options).expect("a reader")),
            four_rows(),
            "{name}"
        );
        assert_eq!(
            media.read_arrow_field(&options).expect("the field"),
            trades_field()
        );
        assert_eq!(media.row_size().expect("the rows"), 4, "{name}");
    }
    let mut holder = Holder::buffer(buffer("trades.csv")).into_media();
    assert!(matches!(&holder, Holder::Media(media) if matches!(media.as_ref(), Media::Csv(_))));
    holder.open().expect("opened");
    assert!(holder.opened());
}
