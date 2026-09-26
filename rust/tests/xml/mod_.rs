//! `rust/src/xml/mod.rs`: natural XML values, the field that types them, and
//! the single bounded document they travel in.

use std::io::{Cursor, Read};

use yggdryl::holder::Buffer;
use yggdryl::text::{self, Format, TextCodec, Xml};
use yggdryl::xml as yxml;
use yggdryl::{
    Charset, DataType, Error, Field, IOBase, Limits, MediaType, MimeType, Scalar, StructType, Url,
    from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar,
};

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).unwrap()
}

fn order() -> Scalar {
    record([(
        "order",
        record([
            ("@id", Scalar::from("7")),
            ("symbol", Scalar::from("AAPL & co")),
            (
                "leg",
                Scalar::from_sequence([Scalar::from("1"), Scalar::from("2")]),
            ),
            ("note", Scalar::Null),
        ]),
    )])
}

const ORDER: &str =
    "<order id=\"7\"><leg>1</leg><leg>2</leg><note/><symbol>AAPL &amp; co</symbol></order>";

#[test]
fn the_inferring_entry_points_answer_the_same_document_from_text_or_bytes() {
    assert_eq!(from_xml_scalar(ORDER).unwrap(), order());
    assert_eq!(from_xml_scalar(ORDER.as_bytes()).unwrap(), order());
    assert_eq!(into_xml_scalar(&order()).unwrap(), ORDER);
    assert_eq!(yxml::from_utf8(ORDER).unwrap(), order());
    assert_eq!(yxml::from_bytes(ORDER.as_bytes()).unwrap(), order());
    assert_eq!(
        yxml::from_reader(Cursor::new(ORDER.as_bytes())).unwrap(),
        order()
    );
    assert_eq!(yxml::into_bytes(&order()).unwrap(), ORDER.as_bytes());
    let mut sink = Vec::new();
    yxml::into_writer(&order(), &mut sink).unwrap();
    assert_eq!(sink, ORDER.as_bytes());
}

#[test]
fn the_document_reads_as_a_one_element_collection() {
    assert_eq!(yxml::from_utf8_all(ORDER).unwrap(), vec![order()]);
    assert_eq!(
        yxml::from_bytes_all(ORDER.as_bytes()).unwrap(),
        vec![order()]
    );
    assert_eq!(
        yxml::from_reader_all(Cursor::new(ORDER.as_bytes())).unwrap(),
        vec![order()]
    );
    let mut source = Cursor::new(ORDER.as_bytes());
    let documents: Vec<_> = yxml::from_reader_iter(&mut source)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(documents, vec![order()]);

    assert_eq!(yxml::into_utf8_all(&[order()]).unwrap(), ORDER);
    assert_eq!(yxml::into_bytes_all(&[order()]).unwrap(), ORDER.as_bytes());
    let error = yxml::into_utf8_all(&[order(), order()]).unwrap_err();
    assert!(error.to_string().contains("multiple documents"), "{error}");
    let error = yxml::into_utf8_all(&[]).unwrap_err();
    assert!(error.to_string().contains("exactly one"), "{error}");
}

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn the_reader_yields_one_document_under_its_limits() {
    let mut reader = yxml::Reader::new(OneByte(Cursor::new(ORDER.as_bytes())));
    assert_eq!(reader.len(), 1);
    assert_eq!(reader.next().unwrap().unwrap(), order());
    assert_eq!(reader.byte_offset(), ORDER.len());
    assert!(reader.next().is_none());

    let short = Limits::new(8, 8, 64, 1);
    let error = yxml::from_reader_with_limits(Cursor::new(ORDER.as_bytes()), short).unwrap_err();
    assert!(error.to_string().contains("byte limit"), "{error}");

    let none = Limits::new(8, 1024, 64, 0);
    let error = yxml::from_reader_with_limits(Cursor::new(ORDER.as_bytes()), none).unwrap_err();
    assert!(error.to_string().contains("document limit"), "{error}");
}

fn row_field() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::decimal128(10, 2)
            .unwrap()
            .required_field("amount"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

#[test]
fn a_field_directed_read_types_the_document_element_whatever_it_is_called() {
    let source = "<anything><id>7</id><amount>12.50</amount></anything>";
    let expected = Scalar::from_sequence([Scalar::from(7_i64), Scalar::d128(1250, 2)]);
    assert_eq!(
        from_xml_scalar_with_field(source, &row_field()).unwrap(),
        expected
    );
    assert_eq!(
        yxml::from_utf8_with_field(source, &row_field()).unwrap(),
        expected
    );
    assert_eq!(
        yxml::from_reader_with_field(Cursor::new(source.as_bytes()), &row_field()).unwrap(),
        expected
    );
    assert_eq!(
        yxml::from_utf8_all_with_field(source, &row_field()).unwrap(),
        vec![expected.clone()]
    );
    let mut cursor = Cursor::new(source.as_bytes());
    let documents: Vec<_> = yxml::from_reader_iter_with_field(&mut cursor, &row_field())
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(documents, vec![expected]);

    let error =
        from_xml_scalar_with_field("<row><id>seven</id><amount>1</amount></row>", &row_field())
            .unwrap_err();
    assert!(error.to_string().contains("id"), "{error}");
}

#[test]
fn xml_is_a_format_the_vocabulary_names() {
    assert_eq!(Format::from_str("xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_str("application/xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_str("text/xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_extension(".XML").unwrap(), Format::Xml);
    assert_eq!(
        Format::from_path(std::path::Path::new("orders.xml")).unwrap(),
        Format::Xml
    );
    assert_eq!(Format::Xml.as_str(), "xml");
    assert_eq!(Format::Xml.extension(), "xml");
    assert_eq!(Format::Xml.mime_type(), MimeType::XML);
    assert_eq!(MimeType::XML.format(), Some(Format::Xml));
    assert_eq!(
        MimeType::from_str("application/atom+xml").unwrap().format(),
        Some(Format::Xml),
        "a structured syntax suffix names the format"
    );
    assert_eq!(
        MimeType::FIXML.format(),
        None,
        "a FIX frame carrying XML is not a document"
    );
    assert_eq!(
        Format::from_url(&Url::from_str("file:///t.xml.gz").unwrap()).unwrap(),
        Format::Xml
    );
    assert!(Format::ALL.contains(&Format::Xml));
    assert!(!Xml.is_multi_document());
    assert_eq!(Xml.mime_type(), MimeType::XML);
    assert_eq!(Xml.from_utf8(ORDER).unwrap(), order());
    assert_eq!(Xml.into_utf8(&order()).unwrap(), ORDER);
    assert_eq!(
        serde_json::from_slice::<Format>(br#""xml""#).unwrap(),
        Format::Xml
    );
}

#[test]
fn the_format_dispatch_reaches_the_xml_codec() {
    assert_eq!(text::from_utf8(ORDER, Format::Xml).unwrap(), order());
    assert_eq!(
        text::from_bytes(ORDER.as_bytes(), Format::Xml).unwrap(),
        order()
    );
    assert_eq!(
        text::from_reader(Cursor::new(ORDER.as_bytes()), Format::Xml).unwrap(),
        order()
    );
    assert_eq!(
        text::from_utf8_all(ORDER, Format::Xml).unwrap(),
        vec![order()]
    );
    assert_eq!(text::into_utf8(&order(), Format::Xml).unwrap(), ORDER);
    assert_eq!(
        text::into_bytes(&order(), Format::Xml).unwrap(),
        ORDER.as_bytes()
    );
    assert_eq!(text::into_utf8_all(&[order()], Format::Xml).unwrap(), ORDER);
}

#[test]
fn content_inference_takes_only_a_well_formed_document_as_xml() {
    assert_eq!(text::infer_format(ORDER.as_bytes()).unwrap(), Format::Xml);
    assert_eq!(
        text::from_utf8_inferred("  \n<a>1</a>").unwrap(),
        (Format::Xml, record([("a", Scalar::from("1"))]))
    );
    assert_eq!(
        text::from_utf8_inferred("\u{FEFF}<?xml version=\"1.0\"?><a/>")
            .unwrap()
            .0,
        Format::Xml
    );
    // A document opening with `<` that is not XML keeps the reading it had.
    assert_eq!(text::infer_format(b"<<: *base\n").unwrap(), Format::Yaml);
    assert_eq!(text::infer_format(b"{\"a\": 1}").unwrap(), Format::Json);
    assert_eq!(text::infer_format(b"a = 1\n").unwrap(), Format::Toml);
}

#[test]
fn placeholders_resolve_in_an_xml_document_too() {
    let loading = text::Loading::new()
        .with_placeholders(text::Placeholders::new().with_variable("PORT", Scalar::from(8080)));
    let value =
        text::from_utf8_with("<cfg><port>{{ PORT }}</port></cfg>", Format::Xml, &loading).unwrap();
    assert_eq!(
        value,
        record([("cfg", record([("port", Scalar::from(8080))]))])
    );
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

#[test]
fn a_handle_reads_and_writes_the_document_through_its_media_type_and_coding() {
    for name in ["order.xml", "order.xml.gz", "order.xml.zst"] {
        let mut target = handle(name);
        target.write_scalar(&order()).unwrap();
        assert_eq!(target.read_scalar(None).unwrap(), order(), "{name}");
        assert_eq!(Xml.from_io(&target).unwrap(), order(), "{name}");
    }
    let mut target = handle("order.xml");
    Xml.into_io(&order(), &mut target).unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        ORDER.as_bytes(),
        "no declaration for UTF-8"
    );
    assert_eq!(text::from_io(&target).unwrap(), order());
    assert_eq!(
        target
            .read_scalar(Some(
                &Field::from_str(
                    "order: struct<@id: int64, symbol: utf8, leg: serie<utf8>, note: utf8> not null"
                )
                .unwrap()
            ))
            .unwrap(),
        Scalar::from_sequence([
            Scalar::from(7_i64),
            Scalar::from("AAPL & co"),
            Scalar::from_sequence([Scalar::from("1"), Scalar::from("2")]),
            Scalar::Null,
        ]),
        "a field types the document element whatever it is called"
    );
}

#[test]
fn the_transport_reads_the_charset_the_declaration_names_and_writes_one_back() {
    let declared = b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>caf\xe9 \x80</a>";
    let source = Buffer::from_bytes(declared.to_vec())
        .with_media_type(Url::from_str("file:///a.xml").unwrap().media_type());
    assert_eq!(
        source.read_scalar(None).unwrap(),
        record([("a", Scalar::from("café €"))]),
        "the declaration is the one content read a handle without a charset makes"
    );

    let mut target =
        Buffer::new().with_media_type(MediaType::new(MimeType::XML).with_charset(Charset::Cp1252));
    target
        .write_scalar(&record([("a", Scalar::from("café €"))]))
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>caf\xe9 \x80</a>",
        "a charset other than UTF-8 is declared where a reader needs it"
    );
    assert_eq!(
        target.read_scalar(None).unwrap(),
        record([("a", Scalar::from("café €"))])
    );

    let declared_and_marked =
        b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>x</a>";
    let source = Buffer::from_bytes(declared_and_marked.to_vec())
        .with_media_type(Url::from_str("file:///a.xml").unwrap().media_type());
    assert_eq!(
        source.read_scalar(None).unwrap(),
        record([("a", Scalar::from("x"))]),
        "a byte order mark answers before the declaration"
    );

    let unknown = b"<?xml version=\"1.0\" encoding=\"x-nothing\"?><a/>";
    let source = Buffer::from_bytes(unknown.to_vec())
        .with_media_type(Url::from_str("file:///a.xml").unwrap().media_type());
    let error = source.read_scalar(None).unwrap_err();
    assert!(error.to_string().contains("x-nothing"), "{error}");

    let error = yxml::from_bytes(declared).unwrap_err();
    assert!(
        matches!(error, Error::Codec { format: "xml", .. }),
        "the codec itself reads UTF-8 and nothing else: {error}"
    );
}

#[test]
fn a_utf16_handle_opens_with_a_mark_before_its_declaration() {
    let value = record([("a", Scalar::from("café"))]);
    let mut target =
        Buffer::new().with_media_type(MediaType::new(MimeType::XML).with_charset(Charset::Utf16Le));
    target.write_scalar(&value).unwrap();
    let bytes = target.read_all_bytes().unwrap();
    assert!(
        bytes.starts_with(b"\xff\xfe<\0?\0x\0m\0l\0"),
        "the mark, then the declaration, in the charset: {bytes:?}"
    );
    assert_eq!(target.read_scalar(None).unwrap(), value);

    let unmarked = Buffer::from_bytes(bytes)
        .with_media_type(Url::from_str("file:///a.xml").unwrap().media_type());
    assert_eq!(
        unmarked.read_scalar(None).unwrap(),
        value,
        "a handle whose media type names no charset reads the mark first"
    );
}

#[test]
fn the_inferring_field_doors_type_an_xml_document() {
    let field = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let typed = Scalar::from_sequence([Scalar::from(7_i64)]);
    assert_eq!(
        text::from_bytes_inferred_with_field(b"<row><id> 7 </id></row>", &field).unwrap(),
        (Format::Xml, typed.clone())
    );
    assert_eq!(
        text::from_utf8_inferred_with_field("<row><id>7</id></row>", &field).unwrap(),
        (Format::Xml, typed)
    );
}
