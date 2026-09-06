//! What a structured text document carries into Arrow rows, and back out.

use crate::holder::Buffer;
use crate::{ArrowShape, ArrowValue, DataType, Field, IOBase, IOMedia, IOMode, Scalar, Url};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("the URL parses")
            .media_type(),
    )
}

fn quote_root() -> Field {
    DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .expect("the root datatype is valid")
    .required_field("row")
}

fn quotes() -> ArrowValue {
    let rows = Scalar::from_sequence([
        Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
        Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
    ]);
    ArrowValue::from_rows(&quote_root(), &rows).expect("the rows materialize")
}

#[test]
fn every_structured_format_round_trips_arrow_rows() {
    for name in ["quotes.json", "quotes.jsonl", "quotes.yaml", "quotes.toml"] {
        let mut target = handle(name);
        target
            .write_arrow_value(quotes(), IOMode::Overwrite)
            .unwrap_or_else(|error| panic!("{name} writes: {error}"));

        let read = target
            .read_arrow_value(Some(&quote_root()))
            .unwrap_or_else(|error| panic!("{name} reads: {error}"));
        assert_eq!(read.shape(), ArrowShape::Batch, "{name}");
        assert_eq!(read.row_size(), Some(2), "{name}");
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            quotes().into_scalar().expect("the rows decode"),
            "{name}"
        );
    }
}

#[test]
fn rows_are_written_with_the_names_their_field_declares() {
    let mut target = handle("quotes.jsonl");
    target
        .write_arrow_value(quotes(), IOMode::Overwrite)
        .expect("the rows write");

    let text = String::from_utf8(target.read_all_bytes().expect("the bytes read"))
        .expect("JSON Lines is UTF-8");
    // A canonical row is positional; the document names it, one row per line.
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains(r#""symbol":"AAPL""#), "{text}");
    assert!(text.contains(r#""size":100"#), "{text}");
}

#[test]
fn a_declared_root_types_the_documents_natural_strings() {
    let mut source = handle("quotes.jsonl");
    source
        .write_all_bytes(br#"{"symbol": "AAPL", "size": 100}"#)
        .expect("the bytes write");

    let widened = DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .required_field("size"),
    ])
    .expect("the root datatype is valid")
    .required_field("row");

    let value = source
        .read_arrow_value(Some(&widened))
        .expect("the declared root types the document");
    assert_eq!(value.field(), &widened);
    assert_eq!(
        value.into_scalar().expect("the rows decode"),
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from("AAPL"),
            Scalar::d128(10_000, 2),
        ])])
    );
}

#[test]
fn an_undeclared_read_names_the_root_the_document_proves() {
    let mut source = handle("quotes.json");
    source
        .write_all_bytes(br#"[{"symbol": "AAPL", "size": 100}]"#)
        .expect("the bytes write");

    let value = source
        .read_arrow_value(None)
        .expect("the document proves a root");
    assert_eq!(value.column_size(), 2);
    assert_eq!(value.row_size(), Some(1));
}

#[test]
fn one_document_that_is_not_a_sequence_is_one_row() {
    let mut source = handle("quote.yaml");
    source
        .write_all_bytes(b"symbol: AAPL\nsize: 100\n")
        .expect("the bytes write");

    let value = source
        .read_arrow_value(Some(&quote_root()))
        .expect("one document is one row");
    assert_eq!(value.row_size(), Some(1));
}

#[test]
fn a_toml_table_travels_under_the_roots_own_name() {
    let mut target = handle("quotes.toml");
    target
        .write_arrow_value(quotes(), IOMode::Overwrite)
        .expect("the rows write");

    let text =
        String::from_utf8(target.read_all_bytes().expect("the bytes read")).expect("TOML is UTF-8");
    // TOML has no top-level sequence, so the rows travel under the root's own
    // name - which is also how the read finds them again.
    assert!(text.starts_with(r#""row" = ["#), "{text}");
    assert!(text.contains(r#""symbol" = "AAPL""#), "{text}");
}

#[test]
fn a_document_is_written_whole_so_only_an_overwrite_applies() {
    let mut target = handle("quotes.json");
    let refused = target
        .write_arrow_value(quotes(), IOMode::Append)
        .expect_err("a document has no append");
    assert!(refused.to_string().contains("overwrite"), "{refused}");
}

#[test]
fn a_compressed_document_reads_and_writes_through_its_coding() {
    let mut target = handle("quotes.jsonl.gz");
    target
        .write_arrow_value(quotes(), IOMode::Overwrite)
        .expect("the rows write");

    // The bytes on the handle are gzip, not JSON Lines.
    let bytes = target.read_all_bytes().expect("the bytes read");
    assert_eq!(&bytes[..2], &[0x1F, 0x8B]);
    assert_eq!(
        target
            .read_arrow_value(Some(&quote_root()))
            .expect("the coding is transparent")
            .row_size(),
        Some(2)
    );
}
