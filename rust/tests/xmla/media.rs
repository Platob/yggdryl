//! `rust/src/xmla/media.rs`: the `.xmla` record medium - the free
//! `read_field`, `read_batch_reader` and `overwrite_arrow_reader` over any
//! byte handle, and the `Xmla` wrapper answering the ordinary `IOMedia` and
//! `IOBase` surfaces - over in-memory buffers, local files, gzip-coded names
//! and `Media`, against literal documents another writer could have saved and
//! the documents this crate writes.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type};
use arrow_array::{Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::ArrowError;
use yggdryl::arrow::BatchReader;
use yggdryl::expression::Selector;
use yggdryl::holder::counted::{Call, Calls, Counted};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::{IORecordOptions, Media, RecordOptions};
use yggdryl::xmla::{
    Content, Method, Response, Xmla, XmlaOptions, overwrite_arrow_reader, read_batch_reader,
    read_field,
};
use yggdryl::{
    Charset, Codec, DataType, Error, Field, Filter, IOBase, IOKind, IOMedia, Level, MediaType,
    MimeType, StructType,
};

const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const XMLA: &str = "urn:schemas-microsoft-com:xml-analysis";
const ROWSET: &str = "urn:schemas-microsoft-com:xml-analysis:rowset";
const EMPTY: &str = "urn:schemas-microsoft-com:xml-analysis:empty";
const MDDATASET: &str = "urn:schemas-microsoft-com:xml-analysis:mddataset";
const EXCEPTION: &str = "urn:schemas-microsoft-com:xml-analysis:exception";
const XSD: &str = "http://www.w3.org/2001/XMLSchema";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The trades schema as another writer spells it under the `xsd` prefix: a
/// required `id` of `xsd:long` and a nullable `symbol` of `xsd:string`.
const TRADES_SCHEMA: &str = concat!(
    "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xsd:element name=\"root\"><xsd:complexType>",
    "<xsd:sequence minOccurs=\"0\" maxOccurs=\"unbounded\">",
    "<xsd:element name=\"row\" type=\"row\"/>",
    "</xsd:sequence></xsd:complexType></xsd:element>",
    "<xsd:complexType name=\"row\"><xsd:sequence>",
    "<xsd:element sql:field=\"id\" name=\"id\" type=\"xsd:long\"/>",
    "<xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\" minOccurs=\"0\"/>",
    "</xsd:sequence></xsd:complexType>",
    "</xsd:schema>",
);

/// The trades schema with a third, nullable `venue` column.
const VENUE_SCHEMA: &str = concat!(
    "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xsd:complexType name=\"row\"><xsd:sequence>",
    "<xsd:element sql:field=\"id\" name=\"id\" type=\"xsd:long\"/>",
    "<xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\" minOccurs=\"0\"/>",
    "<xsd:element sql:field=\"venue\" name=\"venue\" type=\"xsd:string\" minOccurs=\"0\"/>",
    "</xsd:sequence></xsd:complexType>",
    "</xsd:schema>",
);

/// The two rows of the trades table: `AAPL`, and a null symbol.
const TRADES_ROWS: &str = "<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>";

type Row = (i64, Option<String>);

/// The trades table: a required `id` and a nullable `symbol`.
fn trades_field() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
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

/// A `.xmla` buffer holding `document`.
fn stored(document: &str) -> Buffer {
    let mut held = buffer("trades.xmla");
    held.write_all_bytes(document.as_bytes())
        .expect("the document is written");
    held
}

/// A bare rowset `root` as another writer spells it.
fn bare_root(schema: &str, rows: &str) -> String {
    format!(
        "<root xmlns=\"{ROWSET}\" xmlns:xsd=\"{XSD}\" xmlns:xsi=\"{XSI}\">{schema}{rows}</root>"
    )
}

/// A SOAP 1.1 message under the `soap` prefix whose body is `response`
/// holding `root` in its `return`.
fn envelope(response: &str, root: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <soap:Envelope xmlns:soap=\"{SOAP}\"><soap:Body>\
         <{response} xmlns=\"{XMLA}\"><return>{root}</return></{response}>\
         </soap:Body></soap:Envelope>"
    )
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

/// Where and why an XMLA, a rowset, an XML or a SOAP refusal failed: the
/// record path of an `InvalidRecord`, the format of a `Codec` failure, the
/// kind of an `InvalidDataType`, each beside its reason.
fn described(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        Error::Codec { format, reason, .. } => (format.to_owned(), reason.to_string()),
        Error::InvalidDataType { kind, reason } => (kind.to_string(), reason.to_string()),
        other => panic!("expected a located refusal, got {other:?}"),
    }
}

/// The refusal `read_field` and `read_batch_reader` both answer for `held`,
/// asserting they agree.
fn read_refusal(held: &(impl IOBase + ?Sized)) -> (String, String) {
    let field = described(read_field(held, &XmlaOptions::new()).unwrap_err());
    let rows = match read_batch_reader(held, None, &XmlaOptions::new()) {
        Ok(_) => panic!("expected the rows to be refused as the field was: {field:?}"),
        Err(yggdryl::arrow::Error::Core(error)) => described(error),
        Err(other) => panic!("expected a core refusal, got {other:?}"),
    };
    assert_eq!(field, rows, "one refusal, whichever door asks");
    field
}

/// A fresh folder under the temporary directory, named after `label`.
fn temporary(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!("yggdryl-xmla-media-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

// Refusals.

#[test]
fn an_empty_document_declares_no_schema() {
    let error = read_field(&buffer("trades.xmla"), &XmlaOptions::new()).unwrap_err();
    assert_eq!(
        refusal(error),
        (
            "$.xmla".to_owned(),
            "an empty document declares no schema".to_owned()
        )
    );
}

#[test]
fn a_document_that_is_neither_an_envelope_nor_a_rowset_root_is_refused_by_name() {
    let held = stored("<rowset xmlns=\"urn:example:other\"><row/></rowset>");
    let error = read_field(&held, &XmlaOptions::new()).unwrap_err();
    assert_eq!(
        refusal(error),
        (
            "$.xmla".to_owned(),
            "expected a SOAP envelope or a rowset `root`, got `rowset`".to_owned()
        )
    );
    let message = read_batch_reader(&held, Some(&trades_field()), &XmlaOptions::new())
        .err()
        .expect("a refusal")
        .to_string();
    assert!(message.contains("got `rowset`"), "{message}");
}

#[test]
fn a_root_in_a_namespace_that_is_not_the_rowset_one_is_refused() {
    // Only the rowset namespace, or none at all, names a bare rowset: a
    // dataset or an empty answer is only ever read out of its envelope.
    for namespace in ["urn:example:other", MDDATASET, EMPTY, XMLA] {
        let held = stored(&format!("<root xmlns=\"{namespace}\"/>"));
        assert_eq!(
            read_refusal(&held),
            (
                "$.xmla".to_owned(),
                "expected a SOAP envelope or a rowset `root`, got `root`".to_owned()
            ),
            "{namespace}"
        );
    }
}

#[test]
fn a_malformed_document_is_refused() {
    let held = stored("<root xmlns=\"urn:schemas-microsoft-com:xml-analysis:rowset\"><row>");
    let (format, reason) = read_refusal(&held);
    assert_eq!(format, "xml");
    assert!(reason.contains("<row> never closed"), "{reason}");
    // A declared field reads nothing a malformed document could give it.
    let error = read_batch_reader(&held, Some(&trades_field()), &XmlaOptions::new())
        .err()
        .expect("a refusal");
    assert!(error.to_string().contains("never closed"), "{error}");
}

#[test]
fn a_fault_is_refused_as_the_error_its_detail_carries() {
    let held = stored(&format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\"><SOAP-ENV:Body><SOAP-ENV:Fault>\
         <faultcode>SOAP-ENV:Server</faultcode>\
         <faultstring>the cube is not processed</faultstring>\
         <detail><Error xmlns=\"{EXCEPTION}\" ErrorCode=\"3238658121\" \
         Description=\"the cube is not processed\" Source=\"provider\" HelpFile=\"\"/></detail>\
         </SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"
    ));
    let (path, reason) = refusal(read_field(&held, &XmlaOptions::new()).unwrap_err());
    assert_eq!(path, "$.xmla");
    assert!(reason.contains("the cube is not processed"), "{reason}");
    assert!(reason.contains("3238658121"), "{reason}");
    let message = read_batch_reader(&held, Some(&trades_field()), &XmlaOptions::new())
        .err()
        .expect("a refusal")
        .to_string();
    assert!(message.contains("the cube is not processed"), "{message}");
}

#[test]
fn a_multidimensional_dataset_has_no_rows_to_read() {
    let held = stored(&envelope(
        "ExecuteResponse",
        &format!("<root xmlns=\"{MDDATASET}\"><OlapInfo/><Axes/><CellData/></root>"),
    ));
    let expected = (
        "$.xmla".to_owned(),
        "the document holds a multidimensional dataset, which has no rows to read".to_owned(),
    );
    assert_eq!(
        refusal(read_field(&held, &XmlaOptions::new()).unwrap_err()),
        expected
    );
    let message = read_batch_reader(&held, Some(&trades_field()), &XmlaOptions::new())
        .err()
        .expect("a refusal")
        .to_string();
    assert!(message.contains(&expected.1), "{message}");
}

#[test]
fn an_envelope_holding_a_request_rather_than_a_response_is_refused() {
    let held = stored(&format!(
        "<soap:Envelope xmlns:soap=\"{SOAP}\"><soap:Body>\
         <Discover xmlns=\"{XMLA}\"><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>\
         </soap:Body></soap:Envelope>"
    ));
    let (path, reason) = refusal(read_field(&held, &XmlaOptions::new()).unwrap_err());
    assert_eq!(path, "$.xmla");
    assert!(
        reason.contains("`DiscoverResponse` or `ExecuteResponse`"),
        "{reason}"
    );
    assert!(reason.contains("`Discover`"), "{reason}");
}

#[test]
fn a_rowset_with_no_schema_and_no_declared_field_is_refused() {
    let held = stored(&bare_root("", TRADES_ROWS));
    let (_, reason) = refusal(read_field(&held, &XmlaOptions::new()).unwrap_err());
    assert_eq!(
        reason,
        "the rowset carries no schema and no field is declared to read its rows by"
    );
    let message = read_batch_reader(&held, None, &XmlaOptions::new())
        .err()
        .expect("a refusal")
        .to_string();
    assert!(message.contains("carries no schema"), "{message}");
}

#[test]
fn a_cell_its_column_cannot_read_is_refused_naming_the_row() {
    let held = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id>1</id></row><row><id>one hundred</id></row>",
    ));
    assert_eq!(
        read_refusal(&held),
        (
            "$[1]".to_owned(),
            "invalid record value at $.row.id: expected int64, got string".to_owned()
        )
    );
}

#[test]
fn a_handle_declaring_a_charset_other_than_utf8_is_refused_before_a_byte_is_written() {
    // US-ASCII is refused too: it cannot spell every cell a rowset holds.
    for (charset, name) in [
        (Charset::Cp1252, "windows-1252"),
        (Charset::Ascii, "us-ascii"),
        (Charset::Utf16Le, "utf-16le"),
    ] {
        let mut held = Buffer::new()
            .with_media_type(MediaType::from_file_name("trades.xmla").with_charset(charset));
        let error =
            overwrite_arrow_reader(&mut held, two_batches(), &XmlaOptions::new()).unwrap_err();
        assert_eq!(
            refusal(error),
            (
                "$.xmla".to_owned(),
                format!("an XMLA document is written in UTF-8; the handle declares {name}")
            )
        );
        assert_eq!(held.size(), 0);
    }
}

#[test]
fn a_column_a_rowset_cannot_spell_is_refused_and_the_document_stands() {
    let mut held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    let before = held.read_all_bytes().expect("the bytes");
    let tagged = StructType::from_fields([
        DataType::Int64.required_field("id"),
        Field::new(
            "tags",
            DataType::from_str("map<utf8, int64>").expect("a map"),
            true,
        ),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let schema = tagged.into_arrow_schema().expect("an Arrow schema");
    let batches =
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [RecordBatch::new_empty(schema)]);
    let (_, reason) =
        refusal(overwrite_arrow_reader(&mut held, batches, &XmlaOptions::new()).unwrap_err());
    assert!(reason.contains("column `tags` is a map"), "{reason}");
    assert_eq!(held.read_all_bytes().expect("the bytes"), before);
}

#[test]
fn a_batch_that_fails_mid_stream_leaves_the_stored_document_untouched() {
    for options in [XmlaOptions::new(), XmlaOptions::new().without_envelope()] {
        let mut held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
        let before = held.read_all_bytes().expect("the bytes");
        let failing: BatchReader = Box::new(RecordBatchIterator::new(
            vec![
                Ok(batch(&[(9, Some("IBM"))])),
                Err(ArrowError::ComputeError("the feed dropped".to_owned())),
            ],
            trades_field().into_arrow_schema().expect("an Arrow schema"),
        ));
        let message = overwrite_arrow_reader(&mut held, failing, &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("the feed dropped"), "{message}");
        assert_eq!(held.read_all_bytes().expect("the bytes"), before);
    }
}

#[test]
fn record_options_of_another_encoding_are_refused_naming_both() {
    let mut media = Xmla::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let before = media.read_all_bytes().expect("the bytes");
    let ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options");
    let expected = (
        "$.encoding".to_owned(),
        "expected XMLA record options, got application/vnd.apache.arrow.stream".to_owned(),
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
    // The hidden hook a resumable write publishes through refuses the same.
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
    let mut media = Xmla::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
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
    let held = buffer("trades.xmla");
    let declared = trades_field();

    let reader = read_batch_reader(&held, Some(&declared), &XmlaOptions::new()).expect("a reader");
    assert_eq!(
        reader.schema(),
        declared
            .clone()
            .into_arrow_schema()
            .expect("an Arrow schema")
    );
    assert_eq!(counts(reader), (0, 0));

    // The options' field stands in for an absent argument.
    let mut options = XmlaOptions::new();
    options.set_field(declared.clone());
    let reader = read_batch_reader(&held, None, &options).expect("a reader");
    assert_eq!(
        reader.schema(),
        declared
            .clone()
            .into_arrow_schema()
            .expect("an Arrow schema")
    );
    assert_eq!(counts(reader), (0, 0));
    assert_eq!(read_field(&held, &options).expect("the field"), declared);

    // With nothing declared, the empty schema.
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert!(reader.schema().fields().is_empty());
    assert_eq!(counts(reader), (0, 0));
}

#[test]
fn the_default_write_is_an_execute_response_envelope_holding_the_schema_and_the_rows() {
    let mut held = buffer("trades.xmla");
    overwrite_arrow_reader(&mut held, two_batches(), &XmlaOptions::new()).expect("written");
    let document = text(&held);
    assert!(
        document.starts_with(&format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\">"
        )),
        "{document}"
    );
    assert!(
        document.contains(&format!(
            "<ExecuteResponse xmlns=\"{XMLA}\"><return><root xmlns=\"{ROWSET}\""
        )),
        "{document}"
    );
    assert!(document.contains("<xsd:schema"), "{document}");
    assert!(
        document.contains("<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>"),
        "{document}"
    );
    assert!(
        document
            .ends_with("</root></return></ExecuteResponse></SOAP-ENV:Body></SOAP-ENV:Envelope>"),
        "{document}"
    );

    let response = Response::from_bytes(document.as_bytes(), None).expect("a response");
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.rows().expect("a rowset").len(), 4);

    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        trades_field()
    );
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), four_rows());
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(counts(reader), (1, 4), "one document, one batch");
}

#[test]
fn a_discover_method_writes_a_discover_response() {
    let mut held = buffer("trades.xmla");
    let options = XmlaOptions::new().with_method(Method::Discover);
    overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
    let document = text(&held);
    assert!(
        document.contains(&format!("<DiscoverResponse xmlns=\"{XMLA}\"><return>")),
        "{document}"
    );
    assert!(!document.contains("ExecuteResponse"), "{document}");
    let response = Response::from_bytes(document.as_bytes(), None).expect("a response");
    assert_eq!(response.method(), Method::Discover);
    let reader = read_batch_reader(&held, None, &options).expect("a reader");
    assert_eq!(rows_of(reader), four_rows());
}

#[test]
fn without_an_envelope_the_bare_rowset_root_is_written_and_read_back() {
    let mut held = buffer("trades.xmla");
    let options = XmlaOptions::new().without_envelope();
    overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
    let document = text(&held);
    assert!(
        document.starts_with(&format!("<root xmlns=\"{ROWSET}\"")),
        "{document}"
    );
    assert!(document.ends_with("</root>"), "{document}");
    assert!(!document.contains("Envelope"), "{document}");
    assert!(!document.contains("Response"), "{document}");

    // A read accepts either, whatever the options say.
    for options in [XmlaOptions::new(), options] {
        assert_eq!(
            read_field(&held, &options).expect("the field"),
            trades_field()
        );
        let reader = read_batch_reader(&held, None, &options).expect("a reader");
        assert_eq!(rows_of(reader), four_rows());
    }
}

#[test]
fn schema_content_writes_the_columns_and_no_rows() {
    for options in [
        XmlaOptions::new().with_content(Content::Schema),
        XmlaOptions::new()
            .with_content(Content::Schema)
            .without_envelope(),
    ] {
        let mut held = buffer("trades.xmla");
        overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
        let document = text(&held);
        assert!(document.contains("<xsd:schema"), "{document}");
        assert!(!document.contains("<row>"), "{document}");
        assert_eq!(
            read_field(&held, &options).expect("the field"),
            trades_field()
        );
        let reader = read_batch_reader(&held, None, &options).expect("a reader");
        assert_eq!(
            reader.schema(),
            trades_field().into_arrow_schema().expect("an Arrow schema")
        );
        assert_eq!(counts(reader).1, 0);
    }
}

#[test]
fn data_content_writes_rows_a_declared_field_reads() {
    for options in [
        XmlaOptions::new().with_content(Content::Data),
        XmlaOptions::new()
            .with_content(Content::Data)
            .without_envelope(),
    ] {
        let mut held = buffer("trades.xmla");
        overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
        let document = text(&held);
        assert!(!document.contains("xsd:schema"), "{document}");
        assert!(document.contains("<row><id>1</id>"), "{document}");

        // Nothing states the columns.
        let (_, reason) = refusal(read_field(&held, &options).unwrap_err());
        assert!(reason.contains("carries no schema"), "{reason}");

        // A declared field types the rows.
        let reader = read_batch_reader(&held, Some(&trades_field()), &options).expect("a reader");
        assert_eq!(rows_of(reader), four_rows());
        let mut declared = options.clone();
        declared.set_field(trades_field());
        assert_eq!(
            read_field(&held, &declared).expect("the field"),
            trades_field()
        );
        let media = Xmla::new(held).with_options(declared);
        assert_eq!(media.row_size().expect("the rows"), 4);
    }
}

#[test]
fn none_content_writes_an_empty_root_a_declared_field_reads_as_zero_rows() {
    for options in [
        XmlaOptions::new().with_content(Content::None),
        XmlaOptions::new()
            .with_content(Content::None)
            .without_envelope(),
    ] {
        let mut held = buffer("trades.xmla");
        overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
        let document = text(&held);
        assert!(!document.contains("<xsd:"), "{document}");
        assert!(!document.contains("<row"), "{document}");
        assert!(
            document.contains(&format!("<root xmlns=\"{ROWSET}\"")),
            "{document}"
        );
        let (_, reason) = refusal(read_field(&held, &options).unwrap_err());
        assert_eq!(
            reason,
            "the rowset carries no schema and no field is declared to read its rows by"
        );
        let reader = read_batch_reader(&held, Some(&trades_field()), &options).expect("a reader");
        assert_eq!(
            reader.schema(),
            trades_field().into_arrow_schema().expect("an Arrow schema")
        );
        assert_eq!(counts(reader), (1, 0), "a held document is one batch");
    }
    let bare = XmlaOptions::new()
        .with_content(Content::None)
        .without_envelope();
    let mut held = buffer("trades.xmla");
    overwrite_arrow_reader(&mut held, two_batches(), &bare).expect("written");
    assert!(text(&held).ends_with("\"></root>"), "{}", text(&held));
}

#[test]
fn an_empty_answer_reads_as_zero_rows_and_declares_no_schema() {
    let held = stored(&envelope(
        "ExecuteResponse",
        &format!("<root xmlns=\"{EMPTY}\"/>"),
    ));
    assert_eq!(
        refusal(read_field(&held, &XmlaOptions::new()).unwrap_err()).1,
        "an empty document declares no schema"
    );
    let reader =
        read_batch_reader(&held, Some(&trades_field()), &XmlaOptions::new()).expect("a reader");
    assert_eq!(
        reader.schema(),
        trades_field().into_arrow_schema().expect("an Arrow schema")
    );
    assert_eq!(counts(reader), (0, 0));
}

#[test]
fn a_bare_root_another_writer_saved_is_read_with_its_schema() {
    let held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        trades_field()
    );
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL")), (2, None)]));
}

#[test]
fn the_root_is_named_after_the_options() {
    let held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    let mut options = XmlaOptions::new();
    options.set_name("trade".into());
    let field = read_field(&held, &options).expect("the field");
    assert_eq!(field.name(), "trade");
    assert_eq!(field, trades_field().with_name("trade"));
}

#[test]
fn a_root_under_a_prefix_and_a_schema_under_another_are_read() {
    let schema = TRADES_SCHEMA.replace("xsd:", "xs:");
    let document = format!(
        "<rs:root xmlns:rs=\"{ROWSET}\" xmlns:xs=\"{XSD}\">{schema}\
         <rs:row><rs:id>1</rs:id><rs:symbol>AAPL</rs:symbol></rs:row>\
         <rs:row><rs:id>2</rs:id></rs:row></rs:root>"
    );
    let held = stored(&document);
    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        trades_field()
    );
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL")), (2, None)]));
}

#[test]
fn an_envelope_saved_off_the_wire_under_another_prefix_is_read() {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <soap:Envelope xmlns:soap=\"{SOAP}\">\n  \
         <soap:Header><Session xmlns=\"{XMLA}\" SessionId=\"42\"/></soap:Header>\n  \
         <soap:Body>\n    <DiscoverResponse xmlns=\"{XMLA}\">\n      <return>\n        {}\n      \
         </return>\n    </DiscoverResponse>\n  </soap:Body>\n</soap:Envelope>\n",
        bare_root(TRADES_SCHEMA, TRADES_ROWS)
    );
    let held = stored(&document);
    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        trades_field()
    );
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL")), (2, None)]));
}

#[test]
fn a_nil_or_absent_cell_reads_as_null() {
    let held = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id>1</id><symbol xsi:nil=\"true\"/></row><row><id>2</id></row>",
    ));
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, None), (2, None)]));
}

#[test]
fn a_document_in_a_declared_legacy_charset_is_decoded_on_read() {
    let mut bytes = bare_root(TRADES_SCHEMA, "<row><id>1</id><symbol>Soci")
        .trim_end_matches("</root>")
        .as_bytes()
        .to_vec();
    bytes.extend_from_slice(b"\xE9t\xE9</symbol></row></root>");
    let mut held = Buffer::new()
        .with_media_type(MediaType::from_file_name("trades.xmla").with_charset(Charset::Cp1252));
    held.write_all_bytes(&bytes).expect("written");
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), owned(&[(1, Some("Soci\u{e9}t\u{e9}"))]));
}

#[test]
fn a_document_carrying_its_schema_is_read_by_it_and_cast_onto_the_declared_field() {
    let held = stored(&bare_root(
        VENUE_SCHEMA,
        "<row><id>1</id><symbol>AAPL</symbol><venue>XNAS</venue></row>",
    ));
    let declared = trades_field();
    // Undeclared, the document's three columns.
    assert_eq!(
        read_field(&held, &XmlaOptions::new())
            .expect("the field")
            .field_len(),
        3
    );
    // Declared, the batch already stands under the declared field: `venue`
    // is dropped by the cast, not refused as an unknown cell.
    let reader = read_batch_reader(&held, Some(&declared), &XmlaOptions::new()).expect("a reader");
    assert_eq!(
        reader.schema(),
        declared.into_arrow_schema().expect("an Arrow schema")
    );
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL"))]));
}

#[test]
fn a_declared_field_reads_a_column_by_the_name_its_schema_declares() {
    // Another writer's element name need not be the `_xHHHH_` spelling of
    // the column: `sql:field` names the column the element holds.
    let schema = concat!(
        "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
        "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
        "<xsd:complexType name=\"row\"><xsd:sequence>",
        "<xsd:element sql:field=\"Order Id\" name=\"OrderId\" type=\"xsd:long\"/>",
        "</xsd:sequence></xsd:complexType>",
        "</xsd:schema>",
    );
    let document = bare_root(schema, "<row><OrderId>7</OrderId></row>");
    let declared = StructType::from_fields([DataType::Int64.required_field("Order Id")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row");

    // Undeclared, the schema names the column.
    let held = stored(&document);
    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        declared
    );

    // Declared, the same document reads the same row.
    let media = Xmla::new(stored(&document)).with_field(declared.clone());
    let options = media.record_options().expect("the options");
    let batches: Vec<RecordBatch> = media
        .read_arrow_reader(&options)
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("the batches");
    let ids: Vec<i64> = batches
        .iter()
        .flat_map(|batch| {
            batch
                .column_by_name("Order Id")
                .expect("the column")
                .as_primitive::<Int64Type>()
                .values()
                .to_vec()
        })
        .collect();
    assert_eq!(ids, [7]);
}

#[test]
fn a_declared_field_projects_a_document_that_carries_more_columns() {
    let media = Xmla::new(stored(&bare_root(
        VENUE_SCHEMA,
        "<row><id>1</id><symbol>AAPL</symbol><venue>XNAS</venue></row>",
    )))
    .with_field(trades_field());
    let options = media.record_options().expect("the options");
    let reader = media.read_arrow_reader(&options).expect("a reader");
    assert_eq!(
        reader.schema(),
        trades_field().into_arrow_schema().expect("an Arrow schema")
    );
    assert_eq!(rows_of(reader), owned(&[(1, Some("AAPL"))]));
}

#[test]
fn unicode_escaped_empty_and_padded_text_round_trips_under_encoded_names() {
    let field = StructType::from_fields([
        DataType::Int64.required_field("Order Id"),
        DataType::utf8().nullable_field("Venue name"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let values = [
        Some("Z\u{fc}rich \u{20ac} \u{65e5}\u{672c}"),
        Some("<&>\"'"),
        Some("  padded  "),
        Some(""),
        None,
    ];
    let schema = field.clone().into_arrow_schema().expect("an Arrow schema");
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from_iter_values(0..values.len() as i64)),
            Arc::new(StringArray::from_iter(values)),
        ],
    )
    .expect("a batch");
    let mut held = buffer("orders.xmla");
    overwrite_arrow_reader(
        &mut held,
        yggdryl::arrow::batch_reader(schema, [batch]),
        &XmlaOptions::new(),
    )
    .expect("written");
    let document = text(&held);
    assert!(
        document.contains("sql:field=\"Order Id\" name=\"Order_x0020_Id\""),
        "{document}"
    );
    assert!(document.contains("<Venue_x0020_name>"), "{document}");
    assert!(!document.contains("<&>"), "{document}");

    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        field
    );
    let read: Vec<Option<String>> = read_batch_reader(&held, None, &XmlaOptions::new())
        .expect("a reader")
        .flat_map(|batch| {
            let batch = batch.expect("a batch");
            let names = batch
                .column_by_name("Venue name")
                .expect("the column")
                .as_string::<i32>()
                .clone();
            (0..batch.num_rows())
                .map(|index| names.is_valid(index).then(|| names.value(index).to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        read,
        values
            .iter()
            .map(|value| value.map(str::to_owned))
            .collect::<Vec<_>>()
    );
}

// The wrapper.

#[test]
fn the_wrapper_retains_its_options_and_hands_back_its_handle() {
    let media = Xmla::new(buffer("trades.xmla"));
    assert_eq!(media.options(), &XmlaOptions::new());
    assert!(media.options().envelope);
    assert_eq!(media.options().method, Method::Execute);
    assert_eq!(media.options().content, Content::SchemaData);

    let media = media.with_field(trades_field().with_name("trade"));
    assert_eq!(
        media.options().field,
        Some(trades_field().with_name("trade"))
    );
    assert_eq!(media.options().name.as_str(), "trade");

    // A complete configuration replaces the declared field too.
    let configured = XmlaOptions::new()
        .without_envelope()
        .with_method(Method::Discover)
        .with_content(Content::Data);
    let mut media = media.with_options(configured.clone());
    assert_eq!(media.options(), &configured);
    assert_eq!(media.options().field, None);

    media.options_mut().envelope = true;
    media.options_mut().set_field(trades_field());
    assert!(media.options().envelope);
    assert_eq!(media.options().field(), Some(trades_field()));

    media
        .handle_mut()
        .write_all_bytes(b"<root/>")
        .expect("written through the handle");
    assert_eq!(media.handle().size(), 7);
    assert_eq!(media.handle().as_slice(), b"<root/>");
    let held: Buffer = media.into_handle();
    assert_eq!(held.into_bytes(), b"<root/>");
}

#[test]
fn record_options_are_the_xmla_options_carrying_the_declared_field() {
    let media = Xmla::new(buffer("trades.xmla"))
        .with_options(XmlaOptions::new().without_envelope())
        .with_field(trades_field());
    let options = media.record_options().expect("the options");
    let RecordOptions::Xmla(xmla) = &options else {
        panic!("expected XMLA options, got {options:?}");
    };
    assert_eq!(xmla, media.options());
    assert!(!xmla.envelope);
    assert_eq!(options.field(), Some(trades_field()));
    assert_eq!(options.mime_type(), MimeType::XMLA);
}

#[test]
fn read_arrow_field_prefers_the_declared_field_else_reads_the_document() {
    let held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    let bytes = held.read_all_bytes().expect("the bytes");

    let media = Xmla::new(held);
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );

    let declared = StructType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::large_utf8().nullable_field("symbol"),
        DataType::Float64.nullable_field("price"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("trade");
    let media = Xmla::new(
        Buffer::from_bytes(bytes).with_media_type(MediaType::from_file_name("trades.xmla")),
    )
    .with_field(declared.clone());
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        declared
    );

    // Declared over a missing document too.
    let media = Xmla::new(buffer("trades.xmla")).with_field(declared.clone());
    let options = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        declared
    );
}

#[test]
fn read_arrow_reader_reads_the_rows_as_one_batch() {
    let mut media = Xmla::new(buffer("trades.xmla"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(
        counts(media.read_arrow_reader(&options).expect("a reader")),
        (1, 4)
    );
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );

    // The column-shaped read answers the same rows.
    let columns = media
        .read_arrow(None)
        .expect("a serie reader")
        .collect::<Result<Vec<_>, _>>()
        .expect("the columns");
    assert_eq!(columns.iter().map(yggdryl::Serie::len).sum::<usize>(), 4);
}

#[test]
fn a_missing_document_reads_as_no_rows_through_the_wrapper() {
    let media = Xmla::new(buffer("trades.xmla")).with_field(trades_field());
    let options = media.record_options().expect("the options");
    assert_eq!(
        counts(media.read_arrow_reader(&options).expect("a reader")),
        (0, 0)
    );
    let bare = Xmla::new(buffer("trades.xmla"));
    let (_, reason) = refusal(
        bare.read_arrow_field(&bare.record_options().expect("the options"))
            .unwrap_err(),
    );
    assert_eq!(reason, "an empty document declares no schema");
}

#[test]
fn overwrite_arrow_reader_takes_every_batch_and_replaces_the_rows() {
    let mut media = Xmla::new(buffer("trades.xmla"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    assert!(text(&media).contains("ExecuteResponse"));

    media
        .overwrite_arrow_reader(reader(vec![batch(&[(7, Some("IBM"))])]), &options)
        .expect("replaced");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        owned(&[(7, Some("IBM"))])
    );
}

#[test]
fn append_arrow_reader_adds_rows_after_the_stored_ones_in_the_same_shape() {
    let mut media =
        Xmla::new(buffer("trades.xmla")).with_options(XmlaOptions::new().without_envelope());
    let options = media.record_options().expect("the options");

    // An append into nothing is the first write.
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
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    let document = text(&media);
    assert!(document.starts_with("<root "), "{document}");
    assert!(!document.contains("Envelope"), "{document}");
}

#[test]
fn merge_arrow_reader_updates_matches_and_appends_misses() {
    let mut media = Xmla::new(buffer("trades.xmla"));
    let mut options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(
            reader(vec![batch(&[(1, Some("AAPL")), (2, None)])]),
            &options,
        )
        .expect("written");
    options.set_merge_by(Selector::from_columns(["id"]));
    media
        .merge_arrow_reader(
            reader(vec![batch(&[(2, Some("MSFT")), (3, Some("GOOG"))])]),
            &options,
        )
        .expect("merged");
    let plain = media.record_options().expect("the options");
    assert_eq!(
        rows_of(media.read_arrow_reader(&plain).expect("a reader")),
        owned(&[(1, Some("AAPL")), (2, Some("MSFT")), (3, Some("GOOG"))])
    );
    assert!(text(&media).contains("ExecuteResponse"));
}

#[test]
fn a_data_only_document_is_overwritten_and_appended_under_its_declared_field() {
    let mut media = Xmla::new(buffer("trades.xmla"))
        .with_options(XmlaOptions::new().with_content(Content::Data))
        .with_field(trades_field());
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(reader(vec![batch(&[(1, Some("AAPL"))])]), &options)
        .expect("the first write");
    assert!(!text(&media).contains("xsd:schema"));
    media
        .overwrite_arrow_reader(reader(vec![batch(&[(2, Some("MSFT"))])]), &options)
        .expect("a document this medium wrote is overwritten");
    media
        .append_arrow_reader(reader(vec![batch(&[(3, None)])]), &options)
        .expect("a document this medium wrote is appended to");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        owned(&[(2, Some("MSFT")), (3, None)])
    );
}

#[test]
fn row_size_and_column_size_answer_from_the_document_or_the_declared_field() {
    let mut media = Xmla::new(buffer("trades.xmla"));
    assert_eq!(media.row_size().expect("the rows"), 0);
    assert_eq!(media.column_size().expect("the columns"), 0);

    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(media.row_size().expect("the rows"), 4);
    assert_eq!(media.column_size().expect("the columns"), 2);

    let wide = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let declared = Xmla::new(buffer("trades.xmla")).with_field(wide);
    assert_eq!(declared.column_size().expect("the columns"), 3);
    assert_eq!(declared.row_size().expect("the rows"), 0);
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

    let counted = Counted::new(buffer("trades.xmla"));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(three(), &options)
        .expect("written");
    assert_eq!(calls.get(Call::Truncate), 1, "one publication");

    let counted = Counted::new(buffer("trades.xmla"));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    media.options_mut().commit_row_size = Some(1);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(three(), &options)
        .expect("written");
    assert_eq!(calls.get(Call::Truncate), 3, "one publication per row");
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
    let counted = Counted::new(buffer("trades.xmla"));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");

    // Closed, every ask reads the document.
    calls.reset();
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(calls.get(Call::ReadAllBytes), 2);

    // Open, one read answers the schema and the width.
    calls.reset();
    media.open().expect("opened");
    assert!(media.opened());
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(media.column_size().expect("the columns"), 2);
    assert_eq!(calls.get(Call::ReadAllBytes), 1);

    // A different root name is answered from the same cache.
    let mut renamed = options.clone();
    renamed.set_name("trade".into());
    assert_eq!(
        media.read_arrow_field(&renamed).expect("the field"),
        trades_field().with_name("trade")
    );
    assert_eq!(calls.get(Call::ReadAllBytes), 1);

    media.close().expect("closed");
    assert!(!media.opened());
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(calls.get(Call::ReadAllBytes), 2);
}

#[test]
fn a_write_while_open_drops_the_cached_schema() {
    let mut media = Xmla::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let options = media.record_options().expect("the options");
    media.open().expect("opened");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );

    media
        .write_all_bytes(bare_root(VENUE_SCHEMA, "").as_bytes())
        .expect("replaced");
    assert_eq!(
        media
            .read_arrow_field(&options)
            .expect("the field")
            .field_len(),
        3
    );
    assert_eq!(media.column_size().expect("the columns"), 3);

    // Rows overwritten onto a stored document land under its field.
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("rewritten");
    assert_eq!(
        media
            .read_arrow_field(&options)
            .expect("the field")
            .field_len(),
        3
    );
    assert_eq!(media.row_size().expect("the rows"), 4);

    media.clear().expect("cleared");
    assert_eq!(
        refusal(media.read_arrow_field(&options).unwrap_err()).1,
        "an empty document declares no schema"
    );
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written afresh");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    media.close().expect("closed");
}

#[test]
fn the_byte_surface_mirrors_the_handle() {
    let mut media = Xmla::new(buffer("trades.xmla"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");

    assert_eq!(media.size(), media.handle().size());
    assert!(media.size() > 0);
    assert_eq!(
        media.read_all_bytes().expect("the bytes"),
        media.handle().read_all_bytes().expect("the bytes")
    );
    assert_eq!(
        media.read_range_bytes(0, 5).expect("a range"),
        b"<?xml".to_vec()
    );
    assert_eq!(media.media_type(), media.handle().media_type());
    assert_eq!(media.media_type().base(), &MimeType::XMLA);
    assert!(media.is_tabular(), "a rowset document is a record encoding");
    assert!(
        !media.is_atomic(),
        "a record encoding is never one whole value"
    );

    media.clear().expect("cleared");
    assert_eq!(media.size(), 0);
    assert_eq!(media.row_size().expect("the rows"), 0);
    assert_eq!(
        counts(media.read_arrow_reader(&options).expect("a reader")),
        (0, 0)
    );

    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    media.open().expect("opened");
    media.remove(false).expect("removed");
    assert!(!media.opened());
    assert_eq!(media.size(), 0);
}

#[test]
fn a_gzip_coded_name_compresses_the_document_and_reads_it_back() {
    // The free functions over a buffer named `.xmla.gz`.
    let mut held = buffer("trades.xmla.gz");
    assert_eq!(held.media_type().base(), &MimeType::XMLA);
    overwrite_arrow_reader(&mut held, two_batches(), &XmlaOptions::new()).expect("written");
    let encoded = held.read_all_bytes().expect("the bytes");
    assert_eq!(&encoded[..2], &[0x1F, 0x8B]);
    let decoded = Codec::Gzip.load(&encoded).expect("gzip");
    assert!(decoded.starts_with(b"<?xml"));
    let reader = read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader");
    assert_eq!(rows_of(reader), four_rows());

    // And the wrapper over one.
    let mut media = Xmla::new(Holder::buffer(buffer("trades.xmla.gz")));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(
        &media.read_all_bytes().expect("the bytes")[..2],
        &[0x1F, 0x8B]
    );
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    assert_eq!(media.row_size().expect("the rows"), 4);
}

#[test]
fn a_local_xmla_file_is_written_read_and_reopened() {
    let root = temporary("local");

    // A missing file holds no rows yet.
    let missing = Holder::file(root.join("missing.xmla")).expect("a file");
    assert_eq!(
        counts(
            read_batch_reader(&missing, Some(&trades_field()), &XmlaOptions::new())
                .expect("a reader")
        ),
        (0, 0)
    );
    assert_eq!(
        refusal(read_field(&missing, &XmlaOptions::new()).unwrap_err()).1,
        "an empty document declares no schema"
    );

    // The free functions.
    let path = root.join("trades.xmla");
    let mut file = Holder::file(&path).expect("a file");
    overwrite_arrow_reader(&mut file, two_batches(), &XmlaOptions::new()).expect("written");
    assert!(std::fs::read(&path).expect("on disk").starts_with(b"<?xml"));
    let reopened = Holder::file(&path).expect("a file");
    assert_eq!(
        read_field(&reopened, &XmlaOptions::new()).expect("the field"),
        trades_field()
    );
    assert_eq!(
        rows_of(read_batch_reader(&reopened, None, &XmlaOptions::new()).expect("a reader")),
        four_rows()
    );

    // The wrapper, compressed by name.
    let coded = root.join("trades.xmla.gz");
    let mut media = Xmla::new(Holder::file(&coded).expect("a file"));
    let options = media.record_options().expect("the options");
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(&std::fs::read(&coded).expect("on disk")[..2], &[0x1F, 0x8B]);
    let media = Xmla::new(Holder::file(&coded).expect("a file"));
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn media_binds_xmla_by_name_and_reads_and_writes_the_same() {
    let mut media = Media::open(Holder::buffer(buffer("trades.xmla"))).expect("a media");
    assert!(matches!(media, Media::Xmla(_)));
    let options = media.record_options().expect("the options");
    assert!(matches!(options, RecordOptions::Xmla(_)));
    media
        .overwrite_arrow_reader(two_batches(), &options)
        .expect("written");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(media.row_size().expect("the rows"), 4);
    assert_eq!(media.column_size().expect("the columns"), 2);
    let bytes = media.into_handle().read_all_bytes().expect("the bytes");
    assert!(String::from_utf8_lossy(&bytes).contains("ExecuteResponse"));

    // Named explicitly over a buffer whose name says nothing.
    let media = Media::xmla(Holder::buffer(Buffer::from_bytes(bytes))).with_field(trades_field());
    let options = media.record_options().expect("the options");
    assert_eq!(options.field(), Some(trades_field()));
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
}

// What a document's bytes earn before any rowset is read.

#[test]
fn a_byte_the_declared_charset_does_not_assign_is_refused_naming_the_charset_and_the_position() {
    let mut bytes = format!("<root xmlns=\"{ROWSET}\">").into_bytes();
    let position = bytes.len();
    bytes.push(0xFF);
    bytes.extend_from_slice(b"</root>");
    let mut held = buffer("trades.xmla");
    held.write_all_bytes(&bytes).expect("written");
    match read_field(&held, &XmlaOptions::new()).unwrap_err() {
        Error::Codec {
            format,
            position: at,
            reason,
        } => {
            assert_eq!((format, at), ("utf-8", position));
            assert!(reason.contains("0xff"), "{reason}");
        }
        other => panic!("expected the charset's refusal, got {other:?}"),
    }
    assert_eq!(read_refusal(&held).0, "utf-8");
}

#[test]
fn a_coded_name_whose_bytes_are_not_that_coding_is_refused_before_any_xml_is_read() {
    let mut held = buffer("trades.xmla.gz");
    held.write_all_bytes(bare_root(TRADES_SCHEMA, TRADES_ROWS).as_bytes())
        .expect("written");
    let error = read_field(&held, &XmlaOptions::new()).unwrap_err();
    assert!(matches!(error, Error::Io(_)), "{error:?}");
    assert!(error.to_string().contains("invalid gzip header"), "{error}");
    let media = Xmla::new(held);
    assert!(matches!(media.row_size().unwrap_err(), Error::Io(_)));
}

#[test]
fn a_document_holding_no_element_is_refused_where_its_root_was_expected() {
    for document in ["   ", "<?xml version=\"1.0\"?>", "<!-- nothing to read -->"] {
        match read_field(&stored(document), &XmlaOptions::new()).unwrap_err() {
            Error::Codec {
                format,
                position,
                reason,
            } => {
                assert_eq!(format, "xml", "{document:?}");
                assert_eq!(position, document.len(), "{document:?}");
                assert_eq!(reason, "expected the root element", "{document:?}");
            }
            other => panic!("expected the XML refusal, got {other:?}"),
        }
    }
}

#[test]
fn an_element_that_only_resembles_an_envelope_or_a_root_is_refused_by_its_qualified_name() {
    // SOAP 1.2's envelope is not SOAP 1.1's, a prefix does not make a foreign
    // element a rowset, and an `Envelope` outside the SOAP namespace is none.
    for (document, name) in [
        (
            "<env:Envelope xmlns:env=\"http://www.w3.org/2003/05/soap-envelope\">\
             <env:Body/></env:Envelope>"
                .to_owned(),
            "env:Envelope",
        ),
        (
            "<x:rowset xmlns:x=\"urn:example:other\"/>".to_owned(),
            "x:rowset",
        ),
        (format!("<x:root xmlns:x=\"{MDDATASET}\"/>"), "x:root"),
        (format!("<Envelope xmlns=\"{XMLA}\"/>"), "Envelope"),
    ] {
        assert_eq!(
            read_refusal(&stored(&document)),
            (
                "$.xmla".to_owned(),
                format!("expected a SOAP envelope or a rowset `root`, got `{name}`")
            )
        );
    }
}

#[test]
fn an_envelope_that_is_not_one_response_holding_one_root_is_refused_naming_what_it_lacks() {
    let message =
        |body: &str| format!("<soap:Envelope xmlns:soap=\"{SOAP}\">{body}</soap:Envelope>");
    for (document, expected) in [
        (
            message(""),
            (
                "xml",
                "expected one `Body` element under `soap:Envelope`, found none",
            ),
        ),
        (
            message("<soap:Body/>"),
            ("soap", "the SOAP body holds no element"),
        ),
        (
            message("<soap:Body><a/><b/></soap:Body>"),
            (
                "soap",
                "the SOAP body holds several elements where one method call is expected",
            ),
        ),
        (
            message("<soap:Body><ExecuteResponse xmlns=\"urn:example:other\"/></soap:Body>"),
            (
                "$.xmla",
                "expected `DiscoverResponse` or `ExecuteResponse` in \
                 \"urn:schemas-microsoft-com:xml-analysis\", got `ExecuteResponse`",
            ),
        ),
        (
            message(&format!(
                "<soap:Body><ExecuteResponse xmlns=\"{XMLA}\"/></soap:Body>"
            )),
            (
                "xml",
                "expected one `return` element under `ExecuteResponse`, found none",
            ),
        ),
        (
            envelope("ExecuteResponse", ""),
            ("$.xmla", "the response's `return` holds no `root`"),
        ),
        (
            envelope(
                "ExecuteResponse",
                &format!("<root xmlns=\"{EMPTY}\"/><root xmlns=\"{EMPTY}\"/>"),
            ),
            (
                "$.xmla",
                "the response's `return` holds several `root` elements; a response carries one result",
            ),
        ),
        (
            envelope("ExecuteResponse", "<root xmlns=\"urn:example:other\"/>"),
            (
                "$.xmla",
                "the response's `root` is in \"urn:example:other\", which names no result this crate reads",
            ),
        ),
    ] {
        assert_eq!(
            read_refusal(&stored(&document)),
            (expected.0.to_owned(), expected.1.to_owned()),
            "{document}"
        );
    }
}

#[test]
fn an_error_the_provider_reported_inside_the_rowset_is_refused_rather_than_read_as_fewer_rows() {
    // olap4j writes it under `Messages`; a provider may write it bare.
    for reported in [
        "<Messages><Error ErrorCode=\"12\" Description=\"the cube is gone\"/></Messages>",
        "<Error ErrorCode=\"12\" Description=\"the cube is gone\"/>",
    ] {
        let held = stored(&bare_root(
            TRADES_SCHEMA,
            &format!("<row><id>1</id></row>{reported}"),
        ));
        assert_eq!(
            read_refusal(&held),
            (
                "$.rowset".to_owned(),
                "the provider reported an error inside the rowset: the cube is gone (12)"
                    .to_owned()
            )
        );
        let media = Xmla::new(held).with_field(trades_field());
        let error = media.row_size().unwrap_err();
        assert!(error.to_string().contains("the cube is gone"), "{error}");
    }
}

#[test]
fn a_fault_is_refused_by_every_question_the_wrapper_answers_from_the_document() {
    let held = stored(&format!(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\"><SOAP-ENV:Body><SOAP-ENV:Fault>\
         <faultcode>SOAP-ENV:Client</faultcode><faultstring>no such cube</faultstring>\
         </SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"
    ));
    let media = Xmla::new(held);
    let options = media.record_options().expect("the options");
    for error in [
        media.row_size().unwrap_err(),
        media.column_size().unwrap_err(),
        media.read_arrow_field(&options).unwrap_err(),
    ] {
        let (path, reason) = refusal(error);
        assert_eq!(path, "$.xmla");
        assert!(reason.contains("SOAP-ENV:Client"), "{reason}");
        assert!(reason.contains("no such cube"), "{reason}");
    }
}

// What a rowset's rows earn.

#[test]
fn a_row_missing_a_required_cell_or_holding_a_nil_one_is_refused_naming_the_row_and_the_column() {
    let missing = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id>1</id></row><row><symbol>IBM</symbol></row>",
    ));
    let (path, reason) = read_refusal(&missing);
    assert_eq!(path, "$[1]");
    assert!(
        reason.ends_with("`id` is missing from the row, and the rowset declares it required"),
        "{reason}"
    );

    let nil = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id xsi:nil=\"true\"/><symbol>IBM</symbol></row>",
    ));
    assert_eq!(
        read_refusal(&nil),
        (
            "$[0]".to_owned(),
            "invalid record value at $.row.id: non-nullable field received null".to_owned()
        )
    );
}

#[test]
fn a_cell_the_schema_does_not_declare_is_refused_by_its_exact_name() {
    // Element names are case sensitive: `ID` is not `id`.
    for (cells, name) in [
        ("<ID>1</ID><id>2</id>", "ID"),
        ("<id>1</id><venue>XNAS</venue>", "venue"),
    ] {
        let held = stored(&bare_root(TRADES_SCHEMA, &format!("<row>{cells}</row>")));
        assert_eq!(
            read_refusal(&held),
            (
                "$[0]".to_owned(),
                format!("invalid record value at $.row: record contains unknown field \"{name}\"")
            )
        );
    }
}

#[test]
fn a_schema_the_rowset_cannot_read_is_refused_by_its_reason() {
    let twice = TRADES_SCHEMA.replace(
        "sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\"",
        "sql:field=\"id\" name=\"id2\" type=\"xsd:string\"",
    );
    let unnamed = TRADES_SCHEMA.replace(
        "sql:field=\"symbol\" name=\"symbol\"",
        "sql:field=\"symbol\"",
    );
    let rowless = format!("<xsd:schema targetNamespace=\"{ROWSET}\"/>");
    let shapeless = format!(
        "<xsd:schema targetNamespace=\"{ROWSET}\"><xsd:complexType name=\"row\"/></xsd:schema>"
    );
    for (schema, expected) in [
        (twice, ("StructType", "duplicate field name \"id\"")),
        (
            unnamed,
            ("$.rowset", "a column declaration without a `name`"),
        ),
        (
            rowless,
            ("$.rowset", "the rowset schema declares no `row` type"),
        ),
        (
            shapeless,
            (
                "$.rowset",
                "the rowset schema's `row` type declares no sequence",
            ),
        ),
    ] {
        assert_eq!(
            read_refusal(&stored(&bare_root(&schema, ""))),
            (expected.0.to_owned(), expected.1.to_owned())
        );
    }
}

#[test]
fn a_strict_read_refuses_a_cell_the_declared_type_cannot_hold_and_a_safe_one_nulls_it() {
    // The document states `id` as text; the caller declares a nullable int64.
    let text_ids = TRADES_SCHEMA.replace(
        "name=\"id\" type=\"xsd:long\"",
        "name=\"id\" type=\"xsd:string\"",
    );
    let document = bare_root(
        &text_ids,
        "<row><id>1</id><symbol>AAPL</symbol></row><row><id>one</id><symbol>MSFT</symbol></row>",
    );
    let declared = StructType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let held = stored(&document);
    let ids = |reader: BatchReader| -> Vec<Option<i64>> {
        reader
            .flat_map(|batch| {
                let batch = batch.expect("a batch");
                batch
                    .column_by_name("id")
                    .expect("an id column")
                    .as_primitive::<Int64Type>()
                    .iter()
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    let Err(yggdryl::arrow::Error::Core(error)) =
        read_batch_reader(&held, Some(&declared), &XmlaOptions::new())
    else {
        panic!("expected a strict read to refuse `one`");
    };
    assert_eq!(
        refusal(error),
        (
            "$[1]".to_owned(),
            "invalid record value at $.row.id: expected int64, got string".to_owned()
        )
    );

    let mut safe = XmlaOptions::new();
    safe.safe = true;
    let reader = read_batch_reader(&held, Some(&declared), &safe).expect("a reader");
    assert_eq!(
        reader.schema(),
        declared
            .clone()
            .into_arrow_schema()
            .expect("an Arrow schema")
    );
    assert_eq!(ids(reader), [Some(1), None]);

    // The wrapper reads, and counts, under its own `safe`.
    let mut media = Xmla::new(stored(&document)).with_field(declared);
    let strict = media.record_options().expect("the options");
    assert!(media.read_arrow_reader(&strict).is_err());
    assert_eq!(refusal(media.row_size().unwrap_err()).0, "$[1]");
    media.options_mut().safe = true;
    let options = media.record_options().expect("the options");
    assert_eq!(
        ids(media.read_arrow_reader(&options).expect("a reader")),
        [Some(1), None]
    );
    assert_eq!(media.row_size().expect("the rows"), 2);
}

#[test]
fn a_root_in_no_namespace_and_rows_outside_a_prefixed_root_are_read() {
    // No default namespace: the root, the rows and the cells in none.
    let unqualified = format!("<root xmlns:xsd=\"{XSD}\">{TRADES_SCHEMA}{TRADES_ROWS}</root>");
    // A prefixed rowset root whose rows another writer left unqualified.
    let mixed = format!(
        "<rs:root xmlns:rs=\"{ROWSET}\" xmlns:xsd=\"{XSD}\">{TRADES_SCHEMA}{TRADES_ROWS}</rs:root>"
    );
    // Inside a response, a root undeclaring the response's default namespace.
    let undeclared = envelope(
        "ExecuteResponse",
        &format!("<root xmlns=\"\" xmlns:xsd=\"{XSD}\">{TRADES_SCHEMA}{TRADES_ROWS}</root>"),
    );
    for document in [unqualified, mixed, undeclared] {
        let held = stored(&document);
        assert_eq!(
            read_field(&held, &XmlaOptions::new()).expect("the field"),
            trades_field(),
            "{document}"
        );
        assert_eq!(
            rows_of(read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader")),
            owned(&[(1, Some("AAPL")), (2, None)]),
            "{document}"
        );
    }
}

#[test]
fn a_pretty_printed_document_reads_its_cells_as_their_text_stands() {
    let held = stored(&bare_root(
        TRADES_SCHEMA,
        "\n  <row>\n    <id> 7 </id>\n    <symbol>  A B  </symbol>\n  </row>\n  \
         <row>\n    <id>8</id>\n  </row>\n",
    ));
    assert_eq!(
        rows_of(read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader")),
        owned(&[(7, Some("  A B  ")), (8, None)])
    );
}

#[test]
fn an_emptied_text_cell_is_the_empty_string_and_a_self_closed_one_is_null() {
    let held = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id>1</id><symbol></symbol></row><row><id>2</id><symbol/></row>",
    ));
    assert_eq!(
        rows_of(read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader")),
        owned(&[(1, Some("")), (2, None)])
    );
}

#[test]
fn a_column_another_writer_names_by_its_encoded_element_alone_is_decoded() {
    // No `sql:field`: the element's `_xHHHH_` spelling is the column's name,
    // and the cell's entity and character references are its text.
    let schema = TRADES_SCHEMA.replace(
        "sql:field=\"symbol\" name=\"symbol\"",
        "name=\"Prix_x0020__x20AC_\"",
    );
    let held = stored(&bare_root(
        &schema,
        "<row><id>1</id><Prix_x0020__x20AC_>\u{e9}t\u{e9} &amp; &lt;&#x263A;&gt;\
         </Prix_x0020__x20AC_></row>",
    ));
    let field = read_field(&held, &XmlaOptions::new()).expect("the field");
    assert_eq!(
        field
            .fields()
            .iter()
            .map(|child| child.name().to_owned())
            .collect::<Vec<_>>(),
        ["id", "Prix \u{20ac}"]
    );
    let batch = read_batch_reader(&held, None, &XmlaOptions::new())
        .expect("a reader")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(
        batch
            .column_by_name("Prix \u{20ac}")
            .expect("the column")
            .as_string::<i32>()
            .value(0),
        "\u{e9}t\u{e9} & <\u{263a}>"
    );
}

#[test]
fn a_declared_field_orders_and_completes_the_columns_a_document_states() {
    let declared = StructType::from_fields([
        DataType::utf8().nullable_field("symbol"),
        DataType::Float64.nullable_field("price"),
        DataType::Int64.required_field("id"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    let batch = read_batch_reader(&held, Some(&declared), &XmlaOptions::new())
        .expect("a reader")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(
        batch.schema(),
        declared.into_arrow_schema().expect("an Arrow schema")
    );
    assert_eq!(
        batch
            .column_by_name("price")
            .expect("the price")
            .null_count(),
        2,
        "a column the document does not state is completed with nulls"
    );
    assert_eq!(
        rows_of(yggdryl::arrow::batch_reader(batch.schema(), [batch])),
        owned(&[(1, Some("AAPL")), (2, None)])
    );
}

#[test]
fn the_field_argument_wins_over_the_options_field_for_a_missing_document() {
    let mut options = XmlaOptions::new();
    options.set_field(
        StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .expect("a valid root")
            .required_field("row"),
    );
    let reader = read_batch_reader(&buffer("trades.xmla"), Some(&trades_field()), &options)
        .expect("a reader");
    assert_eq!(
        reader.schema(),
        trades_field().into_arrow_schema().expect("an Arrow schema")
    );
}

// What a write earns.

#[test]
fn an_empty_stream_writes_the_schema_and_no_rows() {
    for options in [XmlaOptions::new(), XmlaOptions::new().without_envelope()] {
        let mut held = buffer("trades.xmla");
        overwrite_arrow_reader(&mut held, reader(Vec::new()), &options).expect("written");
        let document = text(&held);
        assert!(
            document.contains(
                "<xsd:element sql:field=\"id\" name=\"id\" type=\"xsd:long\"/>\
                 <xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\" minOccurs=\"0\"/>"
            ),
            "{document}"
        );
        assert!(!document.contains("<row>"), "{document}");
        assert_eq!(
            read_field(&held, &XmlaOptions::new()).expect("the field"),
            trades_field()
        );
        assert_eq!(
            counts(read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader")),
            (1, 0)
        );
    }
}

/// A row carrying a nullable `quote` struct of `bid` and `ask` and a nullable
/// sequence of `sizes`, as a rowset reads the two back.
fn quoted_field() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        Field::new(
            "quote",
            DataType::from(
                StructType::from_fields([
                    DataType::Float64.nullable_field("bid"),
                    DataType::Float64.nullable_field("ask"),
                ])
                .expect("a valid struct"),
            ),
            true,
        ),
        Field::new(
            "sizes",
            DataType::serie(DataType::Int64.nullable_field("item")),
            true,
        ),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

/// Every row of a quoted batch as `(id, bid, ask, sizes)`, a null quote
/// reading as two absent prices.
type Quoted = (i64, Option<f64>, Option<f64>, Vec<i64>);

fn quoted_rows(batch: &RecordBatch) -> Vec<Quoted> {
    let ids = batch
        .column_by_name("id")
        .expect("the ids")
        .as_primitive::<Int64Type>();
    let quote = batch
        .column_by_name("quote")
        .expect("the quote")
        .as_struct();
    let price = |name: &str, index: usize| {
        let prices = quote
            .column_by_name(name)
            .expect("a price")
            .as_primitive::<Float64Type>();
        (quote.is_valid(index) && prices.is_valid(index)).then(|| prices.value(index))
    };
    let sizes = batch
        .column_by_name("sizes")
        .expect("the sizes")
        .as_list::<i32>();
    (0..batch.num_rows())
        .map(|index| {
            (
                ids.value(index),
                price("bid", index),
                price("ask", index),
                sizes
                    .value(index)
                    .as_primitive::<Int64Type>()
                    .values()
                    .to_vec(),
            )
        })
        .collect()
}

#[test]
fn struct_and_sequence_columns_are_nested_and_repeated_elements() {
    // Another writer's document: children named by their elements alone, a
    // one-item sequence, and a row spelling neither column.
    let schema = concat!(
        "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
        "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
        "<xsd:complexType name=\"row\"><xsd:sequence>",
        "<xsd:element name=\"id\" type=\"xsd:long\"/>",
        "<xsd:element name=\"quote\" minOccurs=\"0\"><xsd:complexType><xsd:sequence>",
        "<xsd:element name=\"bid\" type=\"xsd:double\" minOccurs=\"0\"/>",
        "<xsd:element name=\"ask\" type=\"xsd:double\" minOccurs=\"0\"/>",
        "</xsd:sequence></xsd:complexType></xsd:element>",
        "<xsd:element name=\"sizes\" type=\"xsd:long\" minOccurs=\"0\" maxOccurs=\"unbounded\"/>",
        "</xsd:sequence></xsd:complexType>",
        "</xsd:schema>",
    );
    let held = stored(&bare_root(
        schema,
        "<row><id>1</id><quote><bid>1.5</bid></quote><sizes>10</sizes><sizes>20</sizes></row>\
         <row><id>2</id><quote><ask>2.5</ask></quote><sizes>30</sizes></row>\
         <row><id>3</id></row>",
    ));
    assert_eq!(
        read_field(&held, &XmlaOptions::new()).expect("the field"),
        quoted_field()
    );
    let read = read_batch_reader(&held, None, &XmlaOptions::new())
        .expect("a reader")
        .next()
        .expect("one batch")
        .expect("a batch");
    let expected: Vec<Quoted> = vec![
        (1, Some(1.5), None, vec![10, 20]),
        (2, None, Some(2.5), vec![30]),
        (3, None, None, Vec::new()),
    ];
    assert_eq!(quoted_rows(&read), expected);
    let quote = read.column_by_name("quote").expect("the quote");
    assert!(quote.is_null(2), "an absent struct is null");

    // Written back: nested and repeated elements, declared the same way.
    let mut written = buffer("quotes.xmla");
    overwrite_arrow_reader(
        &mut written,
        yggdryl::arrow::batch_reader(read.schema(), [read.clone()]),
        &XmlaOptions::new().without_envelope(),
    )
    .expect("written");
    let document = text(&written);
    for spelled in [
        "<xsd:element sql:field=\"quote\" name=\"quote\" minOccurs=\"0\"><xsd:complexType>",
        "<xsd:element sql:field=\"sizes\" name=\"sizes\" type=\"xsd:long\" minOccurs=\"0\" \
         maxOccurs=\"unbounded\"/>",
        "<row><id>1</id><quote><bid>1.5</bid></quote><sizes>10</sizes><sizes>20</sizes></row>",
        "<row><id>2</id><quote><ask>2.5</ask></quote><sizes>30</sizes></row>",
        "<row><id>3</id></row>",
    ] {
        assert!(document.contains(spelled), "{spelled}\n{document}");
    }
    let reread = read_batch_reader(&written, None, &XmlaOptions::new())
        .expect("a reader")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(reread, read);
}

#[test]
fn the_options_level_reaches_the_content_coding() {
    let mut sizes = Vec::new();
    for level in [Level::NONE, Level::BEST] {
        let mut held = buffer("trades.xmla.gz");
        let mut options = XmlaOptions::new();
        options.level = level;
        overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
        assert_eq!(
            &held.read_all_bytes().expect("the bytes")[..2],
            &[0x1F, 0x8B]
        );
        assert_eq!(
            rows_of(read_batch_reader(&held, None, &options).expect("a reader")),
            four_rows()
        );
        sizes.push(held.size());
    }
    assert!(sizes[0] > sizes[1], "stored beats compressed: {sizes:?}");

    // Without a coding, a level changes no byte.
    let documents: Vec<Vec<u8>> = [Level::NONE, Level::BEST]
        .into_iter()
        .map(|level| {
            let mut held = buffer("trades.xmla");
            let mut options = XmlaOptions::new();
            options.level = level;
            overwrite_arrow_reader(&mut held, two_batches(), &options).expect("written");
            held.read_all_bytes().expect("the bytes")
        })
        .collect();
    assert_eq!(documents[0], documents[1]);
}

// What the wrapper earns.

#[test]
fn a_read_through_the_wrapper_keeps_the_rows_and_columns_its_options_select() {
    let media = Xmla::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let mut options = media.record_options().expect("the options");
    options.set_filter(Filter::from_str("id > 1").expect("a filter"));
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        owned(&[(2, None)])
    );

    let mut options = media.record_options().expect("the options");
    options.set_select(Selector::from_columns(["symbol"]));
    let batch = media
        .read_arrow_reader(&options)
        .expect("a reader")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(
        batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["symbol"]
    );
    assert_eq!(batch.num_rows(), 2);
}

#[test]
fn an_append_onto_a_document_written_without_its_schema_reads_the_stored_rows_by_the_incoming_shape()
 {
    let mut media = Xmla::new(stored(&bare_root("", TRADES_ROWS)))
        .with_options(XmlaOptions::new().without_envelope());
    let options = media.record_options().expect("the options");
    media
        .append_arrow_reader(reader(vec![batch(&[(3, Some("MSFT"))])]), &options)
        .expect("appended");
    assert_eq!(
        rows_of(
            read_batch_reader(&media, Some(&trades_field()), &XmlaOptions::new())
                .expect("a reader")
        ),
        owned(&[(1, Some("AAPL")), (2, None), (3, Some("MSFT"))])
    );
    // The document the append wrote states its schema.
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
}

#[test]
fn an_overwrite_replaces_a_stored_document_that_states_no_rowset() {
    for document in [
        // A fault.
        format!(
            "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\"><SOAP-ENV:Body><SOAP-ENV:Fault>\
             <faultcode>SOAP-ENV:Server</faultcode><faultstring>down</faultstring>\
             </SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"
        ),
        // An answer to a command that answers nothing.
        envelope("ExecuteResponse", &format!("<root xmlns=\"{EMPTY}\"/>")),
        // A request rather than a response.
        format!(
            "<soap:Envelope xmlns:soap=\"{SOAP}\"><soap:Body><Discover xmlns=\"{XMLA}\">\
             <RequestType>DISCOVER_DATASOURCES</RequestType></Discover></soap:Body></soap:Envelope>"
        ),
        // A response whose `return` holds no result.
        envelope("DiscoverResponse", ""),
        // A document that is no XMLA at all.
        "<rowset xmlns=\"urn:example:other\"><row/></rowset>".to_owned(),
        // Rows written with no schema.
        bare_root("", TRADES_ROWS),
    ] {
        let mut media = Xmla::new(stored(&document));
        let options = media.record_options().expect("the options");
        media
            .overwrite_arrow_reader(two_batches(), &options)
            .unwrap_or_else(|error| panic!("{document}: {error}"));
        assert_eq!(
            rows_of(media.read_arrow_reader(&options).expect("a reader")),
            four_rows(),
            "{document}"
        );
    }
}

/// A multidimensional answer as SQL Server Analysis Services saves one: its
/// `root` carries an `xs:schema` describing the dataset's own elements - no
/// rowset `row` type among them - before the axes and the cells.
fn dataset_answer(schema: &str) -> String {
    envelope(
        "ExecuteResponse",
        &format!(
            "<root xmlns=\"{MDDATASET}\" xmlns:xs=\"{XSD}\">{schema}<OlapInfo/><Axes/><CellData/></root>"
        ),
    )
}

/// The `xs:schema` a multidimensional answer describes its members with.
const DATASET_SCHEMA: &str = concat!(
    "<xs:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:mddataset\" ",
    "elementFormDefault=\"qualified\">",
    "<xs:complexType name=\"MemberType\"><xs:sequence>",
    "<xs:element name=\"UName\" type=\"xs:string\"/>",
    "</xs:sequence></xs:complexType>",
    "</xs:schema>",
);

#[test]
fn an_overwrite_replaces_a_stored_multidimensional_answer_whatever_schema_describes_it() {
    for document in [dataset_answer(""), dataset_answer(DATASET_SCHEMA)] {
        let mut media = Xmla::new(stored(&document));
        let options = media.record_options().expect("the options");
        // A read refuses the dataset as the dataset it is, schema or none...
        assert_eq!(
            refusal(media.read_arrow_field(&options).unwrap_err()).1,
            "the document holds a multidimensional dataset, which has no rows to read"
        );
        // ...and an overwrite replaces it, as every stored document that
        // states no rowset is replaced: its `xs:schema` is the dataset's.
        media
            .overwrite_arrow_reader(two_batches(), &options)
            .unwrap_or_else(|error| panic!("{document}: {error}"));
        assert_eq!(
            rows_of(media.read_arrow_reader(&options).expect("a reader")),
            four_rows()
        );
    }
}

#[test]
fn an_append_onto_a_stored_answer_holding_no_rowset_is_refused_as_a_read_is_and_the_document_stands()
 {
    for (document, expected) in [
        (
            format!(
                "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\"><SOAP-ENV:Body><SOAP-ENV:Fault>\
                 <faultcode>SOAP-ENV:Server</faultcode><faultstring>down</faultstring>\
                 </SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"
            ),
            "down",
        ),
        (
            dataset_answer(""),
            "the document holds a multidimensional dataset, which has no rows to read",
        ),
        (
            "<rowset xmlns=\"urn:example:other\"><row/></rowset>".to_owned(),
            "expected a SOAP envelope or a rowset `root`, got `rowset`",
        ),
    ] {
        let mut media = Xmla::new(stored(&document));
        let options = media.record_options().expect("the options");
        let (path, reason) = refusal(
            media
                .append_arrow_reader(two_batches(), &options)
                .unwrap_err(),
        );
        assert_eq!(path, "$.xmla");
        assert!(reason.contains(expected), "{reason}");
        assert_eq!(text(&media), document);
    }

    // An empty answer holds no rows to keep, and takes the appended ones.
    let mut media = Xmla::new(stored(&envelope(
        "ExecuteResponse",
        &format!("<root xmlns=\"{EMPTY}\"/>"),
    )));
    let options = media.record_options().expect("the options");
    media
        .append_arrow_reader(two_batches(), &options)
        .expect("appended");
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
}

#[test]
fn a_write_onto_a_stored_document_that_does_not_parse_is_refused_and_the_document_stands() {
    let document = format!("<root xmlns=\"{ROWSET}\"><row>");
    let mut media = Xmla::new(stored(&document));
    let options = media.record_options().expect("the options");
    for error in [
        media
            .overwrite_arrow_reader(two_batches(), &options)
            .unwrap_err(),
        media
            .append_arrow_reader(two_batches(), &options)
            .unwrap_err(),
    ] {
        let (format, reason) = described(error);
        assert_eq!(format, "xml");
        assert!(reason.contains("never closed"), "{reason}");
        assert_eq!(text(&media), document);
    }
}

/// A rowset whose `at` column is an `xsd:dateTime`, a wall reading or an
/// instant as its rows say.
const ZONED_SCHEMA: &str = concat!(
    "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xsd:complexType name=\"row\"><xsd:sequence>",
    "<xsd:element sql:field=\"id\" name=\"id\" type=\"xsd:long\"/>",
    "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\" minOccurs=\"0\"/>",
    "</xsd:sequence></xsd:complexType>",
    "</xsd:schema>",
);

/// The Arrow type the `at` column of `field` projects to.
fn at_type(field: Field) -> arrow_schema::DataType {
    field
        .into_arrow_schema()
        .expect("an Arrow schema")
        .field_with_name("at")
        .expect("the at column")
        .data_type()
        .clone()
}

/// A zoned document through the wrapper, and the rows it reads.
fn zoned() -> (Xmla<Buffer>, RecordOptions, Vec<RecordBatch>) {
    let media = Xmla::new(stored(&bare_root(
        ZONED_SCHEMA,
        "<row><id>1</id><at>2024-01-02T10:00:00Z</at></row>",
    )))
    .with_options(XmlaOptions::new().without_envelope());
    let options = media.record_options().expect("the options");
    let instant =
        arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()));
    assert_eq!(
        at_type(media.read_arrow_field(&options).expect("the field")),
        instant,
        "rows spelling a zone make the column an instant"
    );
    let read = media
        .read_arrow_reader(&options)
        .expect("a reader")
        .collect::<Result<Vec<_>, _>>()
        .expect("the batches");
    (media, options, read)
}

#[test]
fn an_append_of_the_instants_a_zoned_document_reads_is_written_back_as_instants() {
    let (mut media, options, read) = zoned();
    let schema = read[0].schema();
    let before = media.read_arrow_field(&options).expect("the field");
    media
        .append_arrow_reader(yggdryl::arrow::batch_reader(schema, read), &options)
        .expect("the rows the document read append back onto it");
    assert_eq!(media.row_size().expect("the rows"), 2);
    assert_eq!(media.read_arrow_field(&options).expect("the field"), before);
}

#[test]
fn an_overwrite_onto_a_zoned_document_keeps_its_instants() {
    let (mut media, options, read) = zoned();
    let schema = read[0].schema();
    let before = media.read_arrow_field(&options).expect("the field");
    media
        .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, read), &options)
        .expect("rewritten");
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        before,
        "an instant stays an instant: {}",
        text(&media)
    );
}

#[test]
fn open_is_idempotent_and_opens_the_handle_once() {
    let counted = Counted::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    assert!(format!("{media:?}").starts_with("Xmla"));
    assert!(!media.opened());
    media.open().expect("opened");
    media.open().expect("opened again");
    assert!(media.opened());
    assert_eq!(calls.get(Call::Open), 1);
    media.close().expect("closed");
    assert!(!media.opened());
    assert_eq!(calls.get(Call::Close), 1);
}

/// How many whole reads one `read_arrow_field` costs.
fn field_reads(media: &Xmla<Counted<Buffer>>, calls: &Calls, options: &RecordOptions) -> u64 {
    calls.reset();
    assert_eq!(
        media.read_arrow_field(options).expect("the field"),
        trades_field()
    );
    calls.get(Call::ReadAllBytes)
}

#[test]
fn every_mutation_through_the_wrapper_drops_the_cached_schema() {
    let counted = Counted::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    let options = media.record_options().expect("the options");
    media.open().expect("opened");
    assert_eq!(field_reads(&media, &calls, &options), 1);
    assert_eq!(
        field_reads(&media, &calls, &options),
        0,
        "cached while open"
    );

    media.set_media_type(MediaType::from_file_name("trades.xmla"));
    assert_eq!(
        field_reads(&media, &calls, &options),
        1,
        "after set_media_type"
    );

    assert_eq!(media.pwrite(0, b"<").expect("written"), 1);
    assert_eq!(field_reads(&media, &calls, &options), 1, "after pwrite");

    let size = media.size();
    media.truncate(size).expect("truncated");
    assert_eq!(field_reads(&media, &calls, &options), 1, "after truncate");

    // The hidden hook publishes the prepared rows without reading the
    // stored document back, and drops the cache as every write does.
    calls.reset();
    media
        .overwrite_prepared_arrow_reader(two_batches(), &options)
        .expect("published");
    assert_eq!(calls.get(Call::ReadAllBytes), 0);
    assert_eq!(calls.get(Call::Truncate), 1);
    assert_eq!(
        field_reads(&media, &calls, &options),
        1,
        "after a prepared write"
    );
    assert_eq!(
        rows_of(media.read_arrow_reader(&options).expect("a reader")),
        four_rows()
    );
}

#[test]
fn while_open_the_width_is_read_from_the_document_once() {
    let counted = Counted::new(stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS)));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    let options = media.record_options().expect("the options");
    media.open().expect("opened");
    calls.reset();
    assert_eq!(media.column_size().expect("the columns"), 2);
    assert_eq!(media.column_size().expect("the columns"), 2);
    assert_eq!(
        media.read_arrow_field(&options).expect("the field"),
        trades_field()
    );
    assert_eq!(
        calls.get(Call::ReadAllBytes),
        1,
        "open caches the document's schema until close"
    );
}

#[test]
fn the_byte_calls_the_wrapper_does_not_change_reach_the_handle_once_each() {
    let document = bare_root(TRADES_SCHEMA, TRADES_ROWS);
    let counted = Counted::new(stored(&document));
    let calls = Arc::clone(counted.calls());
    let mut media = Xmla::new(counted);
    calls.reset();

    let mut head = [0; 5];
    assert_eq!(media.pread(0, &mut head).expect("a read"), 5);
    assert_eq!(&head, b"<root");
    assert_eq!(calls.get(Call::Pread), 1);

    let streamed: Vec<u8> = media
        .pstream_bytes(0, 16)
        .expect("a stream")
        .map(|chunk| chunk.expect("a chunk"))
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(streamed, document.as_bytes());
    assert_eq!(calls.get(Call::PstreamBytes), 1);

    media.flush().expect("flushed");
    assert_eq!(calls.get(Call::Flush), 1);
    assert_eq!(media.kind(), IOKind::Memory);
    assert_eq!(calls.get(Call::Kind), 1);
    assert!(media.url().is_some());
    assert_eq!(calls.get(Call::Url), 1);
    assert_eq!(media.mtime(), None);
    assert_eq!(calls.get(Call::Mtime), 1);
    assert!(media.capacity() >= document.len() as u64);
    assert_eq!(calls.get(Call::Capacity), 1);

    // What a rowset document is, the wrapper answers itself.
    assert!(media.is_tabular());
    assert!(!media.is_atomic());
    assert_eq!(calls.get(Call::IsTabular) + calls.get(Call::IsAtomic), 0);
}

#[test]
fn a_declared_field_answers_the_schema_and_the_width_without_reading_the_document() {
    let counted = Counted::new(stored("<not xml"));
    let calls = Arc::clone(counted.calls());
    let mut options = XmlaOptions::new();
    options.set_field(trades_field());
    assert_eq!(
        read_field(&counted, &options).expect("the field"),
        trades_field()
    );
    let media = Xmla::new(counted).with_field(trades_field());
    let record = media.record_options().expect("the options");
    assert_eq!(
        media.read_arrow_field(&record).expect("the field"),
        trades_field()
    );
    assert_eq!(media.column_size().expect("the columns"), 2);
    assert_eq!(calls.get(Call::ReadAllBytes), 0);

    // Counting the rows reads them, so the document is refused there.
    assert_eq!(described(media.row_size().unwrap_err()).0, "xml");
    // Undeclared, the width is read, and refused likewise.
    let bare = Xmla::new(stored("<not xml"));
    assert_eq!(described(bare.column_size().unwrap_err()).0, "xml");
}

#[test]
fn a_stream_whose_columns_repeat_a_name_is_refused_and_the_document_stands() {
    let mut held = stored(&bare_root(TRADES_SCHEMA, TRADES_ROWS));
    let before = held.read_all_bytes().expect("the bytes");
    let schema = Arc::new(arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, true),
    ]));
    let batches =
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [RecordBatch::new_empty(schema)]);
    assert_eq!(
        described(overwrite_arrow_reader(&mut held, batches, &XmlaOptions::new()).unwrap_err()),
        (
            "StructType".to_owned(),
            "duplicate field name \"id\"".to_owned()
        )
    );
    assert_eq!(held.read_all_bytes().expect("the bytes"), before);
}

#[test]
fn a_whitespace_only_cell_is_text_and_a_carriage_return_survives_the_round_trip() {
    // `xsd:string` preserves whitespace: a cell of blanks is not an empty one.
    let held = stored(&bare_root(
        TRADES_SCHEMA,
        "<row><id>1</id><symbol>   </symbol></row><row><id>2</id><symbol>\n\t</symbol></row>",
    ));
    assert_eq!(
        rows_of(read_batch_reader(&held, None, &XmlaOptions::new()).expect("a reader")),
        owned(&[(1, Some("   ")), (2, Some("\n\t"))])
    );

    // XML folds a literal CR LF into LF on read, so the writer spells the CR
    // as a character reference.
    let rows = [(1, Some("   ")), (2, Some("\n\t")), (3, Some(" a\r\nb "))];
    let mut written = buffer("trades.xmla");
    overwrite_arrow_reader(
        &mut written,
        reader(vec![batch(&rows)]),
        &XmlaOptions::new().without_envelope(),
    )
    .expect("written");
    let document = text(&written);
    assert!(
        document.contains("<row><id>3</id><symbol> a&#13;\nb </symbol></row>"),
        "{document}"
    );
    assert_eq!(
        rows_of(read_batch_reader(&written, None, &XmlaOptions::new()).expect("a reader")),
        owned(&rows)
    );
}
