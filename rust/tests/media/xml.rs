//! XML as a record encoding, reached the way a caller reaches one.
//!
//! Everything here goes through `yggdryl::` and a handle whose media type says
//! XML, because that is the surface a caller has: the name picks the encoding
//! and nothing else in the call changes.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, IOBase, IOMedia, Scalar, Url};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn written(name: &str, document: &str) -> Buffer {
    let mut handle = handle(name);
    handle.write_all_bytes(document.as_bytes()).unwrap();
    handle
}

#[test]
fn the_name_alone_makes_a_document_answer_the_record_surface() {
    let handle = written(
        "trades.xml",
        "<rows><Trade><id>1</id><symbol>AAPL</symbol></Trade>\
         <Trade><id>2</id><symbol>MSFT</symbol></Trade></rows>",
    );
    let options = handle.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Xml(_)));

    let field = handle.read_arrow_field(&options).unwrap();
    assert_eq!(field.field_len(), 2);
    assert_eq!(handle.row_size().unwrap(), 2);

    // The media names the root after the element the document repeats, which
    // a bare handle's schema read cannot know to ask for.
    let media =
        yggdryl::media::Media::open(yggdryl::holder::Holder::buffer(handle.clone())).unwrap();
    assert_eq!(
        media
            .read_arrow_field(&media.record_options().unwrap())
            .unwrap()
            .name(),
        "Trade"
    );

    let rows: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
}

#[test]
fn a_declared_field_types_the_text_a_document_carries() {
    // Every leaf on the wire is text; the declaration is what makes one an
    // integer, through the same value contract every other codec uses.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("Trade");

    let handle = written(
        "typed.xml",
        "<rows><Trade id='7'><symbol>AAPL</symbol></Trade></rows>",
    );
    let options = handle.record_options().unwrap().with_field(schema.clone());
    let batches: Vec<_> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    let ids = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(ids.value(0), 7);
}

#[test]
fn rows_written_as_xml_read_back_as_the_rows_that_were_written() {
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("Trade");
    let arrow_schema = schema.clone().into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap();

    let mut handle = handle("out.xml");
    let options = handle.record_options().unwrap().with_field(schema);
    handle
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(arrow_schema, [batch]),
            &options,
        )
        .unwrap();

    // It is also just bytes, and the bytes are a document.
    let encoded = String::from_utf8(handle.read_all_bytes().unwrap()).unwrap();
    assert!(
        encoded.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{encoded}"
    );
    assert!(encoded.contains("<Trade>"), "{encoded}");

    let read: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(read, 2, "{encoded}");
}

#[test]
fn an_empty_handle_is_an_empty_table_rather_than_a_refusal() {
    let handle = handle("empty.xml");
    let options = handle.record_options().unwrap();
    assert_eq!(handle.row_size().unwrap(), 0);
    let rows: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 0);
}

#[test]
fn a_document_the_row_model_cannot_hold_is_refused_by_name() {
    let handle = written("mixed.xml", "<rows><r>text<c/></r></rows>");
    let options = handle.record_options().unwrap();
    let error = match handle.read_arrow_reader(&options) {
        Ok(_) => panic!("expected mixed content to be refused"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("got both"), "{error}");
}

#[test]
fn the_row_element_can_be_named_when_a_wrapper_holds_more_than_rows() {
    let handle = written(
        "named.xml",
        "<feed><meta>ignored</meta><Trade><id>1</id></Trade><Trade><id>2</id></Trade></feed>",
    );
    let mut options = handle.record_options().unwrap();
    let RecordOptions::Xml(xml) = &mut options else {
        panic!("expected XML options");
    };
    xml.row_element = Some("Trade".into());

    let rows: usize = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
}

#[test]
fn a_document_reads_through_the_content_coding_its_name_declares() {
    // XML does not compress internally, so an outer coding is the handle's to
    // own - unlike Avro and Parquet, which refuse one.
    let schema = DataType::from_fields([DataType::utf8().nullable_field("id")])
        .unwrap()
        .required_field("row");
    let value = Scalar::from_sequence([Scalar::from_record([("id", Scalar::from("1"))]).unwrap()]);
    let mut handle = handle("gzipped.xml.gz");
    let options = handle.record_options().unwrap().with_field(schema.clone());
    handle
        .write_arrow(
            yggdryl::arrow::ArrowScalar::from_rows(&schema, &value).unwrap(),
            yggdryl::IOMode::Overwrite,
            Some(&options),
        )
        .unwrap();
    assert_eq!(handle.row_size().unwrap(), 1);
}

#[test]
fn a_field_declared_over_a_document_is_what_column_size_answers() {
    let handle = written("cols.xml", "<rows><r><a>1</a><b>2</b><c>3</c></r></rows>");
    assert_eq!(handle.column_size().unwrap(), 3);

    let declared = Field::from_str("r: struct<a: utf8> not null").unwrap();
    let options = handle.record_options().unwrap().with_field(declared);
    assert_eq!(handle.read_arrow_field(&options).unwrap().field_len(), 1);
}
