use std::io::{Cursor, Read};

use yggdryl::text::xml as yxml;
use yggdryl::text::{Format, Formatting, Indent, Structured, TextCodec, Xml};
use yggdryl::{
    DataType, Error, Field, IOBase, Limits, MimeType, Scalar, Url, from_xml_scalar,
    from_xml_scalar_with_field, holder::Buffer, into_xml_scalar,
};

/// One reader that never answers more than a byte, so a decoder is proven to
/// join what it reads rather than to depend on one call holding a document.
struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn a_document_is_the_one_entry_record_its_element_names() {
    let value = from_xml_scalar("<trade id='7'><symbol>AAPL</symbol></trade>").unwrap();
    assert_eq!(
        value,
        Scalar::from_record([(
            "trade",
            Scalar::from_record([("@id", Scalar::from("7")), ("symbol", Scalar::from("AAPL")),])
                .unwrap(),
        )])
        .unwrap()
    );
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        r#"<trade id="7"><symbol>AAPL</symbol></trade>"#
    );
    assert_eq!(
        from_xml_scalar(into_xml_scalar(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn a_name_that_repeats_is_a_sequence_in_document_order() {
    let value = from_xml_scalar("<row><leg>1</leg><leg>2</leg><leg>3</leg></row>").unwrap();
    assert_eq!(
        value,
        Scalar::from_record([(
            "row",
            Scalar::from_record([(
                "leg",
                Scalar::from_sequence([Scalar::from("1"), Scalar::from("2"), Scalar::from("3"),]),
            )])
            .unwrap(),
        )])
        .unwrap()
    );
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        "<row><leg>1</leg><leg>2</leg><leg>3</leg></row>"
    );
}

#[test]
fn an_element_holding_nothing_is_null_and_writes_itself_closed() {
    let value = from_xml_scalar("<row><id/><name></name></row>").unwrap();
    let row = value.get_key_str("row").unwrap();
    assert!(row.get_key_str("id").unwrap().is_null());
    assert!(row.get_key_str("name").unwrap().is_null());
    assert_eq!(into_xml_scalar(&value).unwrap(), "<row><id/><name/></row>");
}

#[test]
fn layout_is_not_content_but_a_cdata_section_is() {
    assert_eq!(
        from_xml_scalar("<row>\n  <note>  spaced  </note>\n</row>")
            .unwrap()
            .get_key_str("row")
            .and_then(|row| row.get_key_str("note"))
            .and_then(Scalar::as_utf8),
        Some("spaced")
    );
    assert_eq!(
        from_xml_scalar("<row><note><![CDATA[  kept  ]]></note></row>")
            .unwrap()
            .get_key_str("row")
            .and_then(|row| row.get_key_str("note"))
            .and_then(Scalar::as_utf8),
        Some("  kept  ")
    );
}

#[test]
fn references_resolve_only_where_a_definition_exists() {
    assert_eq!(
        from_xml_scalar("<row>a &amp; b &#65; &#x42;</row>")
            .unwrap()
            .get_key_str("row")
            .and_then(Scalar::as_utf8),
        Some("a & b A B")
    );
    let error = from_xml_scalar("<!DOCTYPE row [<!ENTITY x 'y'>]><row>&x;</row>")
        .unwrap_err()
        .to_string();
    assert!(error.contains("&x;"), "{error}");
}

#[test]
fn markup_a_value_carries_is_escaped_and_reads_back_unchanged() {
    let value = Scalar::from_record([(
        "row",
        Scalar::from_record([
            ("@note", Scalar::from("a \"quoted\" <tag>\nnext")),
            ("body", Scalar::from("1 < 2 && 3 > 2")),
        ])
        .unwrap(),
    )])
    .unwrap();
    let encoded = into_xml_scalar(&value).unwrap();
    assert!(encoded.contains("&quot;"), "{encoded}");
    assert!(encoded.contains("&#10;"), "{encoded}");
    assert!(encoded.contains("&lt;"), "{encoded}");
    assert_eq!(from_xml_scalar(&encoded).unwrap(), value);
}

#[test]
fn a_character_xml_cannot_carry_is_refused_rather_than_written() {
    let value = Scalar::from_record([("row", Scalar::from("a\u{ffff}b"))]).unwrap();
    let error = into_xml_scalar(&value).unwrap_err().to_string();
    assert!(error.contains("U+FFFF"), "{error}");
}

#[test]
fn an_element_holding_only_empty_sequences_closes_itself() {
    let value = Scalar::from_record([(
        "row",
        Scalar::from_record([("leg", Scalar::from_sequence([]))]).unwrap(),
    )])
    .unwrap();
    assert_eq!(into_xml_scalar(&value).unwrap(), "<row/>");
}

#[test]
fn a_field_types_what_the_document_element_holds() {
    let amount = Field::new("amount", DataType::decimal128(10, 2).unwrap(), false);
    let field = Field::new(
        "row",
        DataType::from_fields([amount, DataType::Int64.required_field("id")]).unwrap(),
        false,
    );
    let value =
        from_xml_scalar_with_field("<trade><amount>12.50</amount><id>7</id></trade>", &field)
            .unwrap();
    assert_eq!(
        value,
        Scalar::from_sequence([Scalar::d128(1250, 2), Scalar::from(7_i64)])
    );
}

#[test]
fn nesting_past_the_implementation_ceiling_is_refused_whatever_the_limits_say() {
    let depth = yxml::MAX_PARSER_DEPTH + 1;
    let deep = format!("{}x{}", "<a>".repeat(depth), "</a>".repeat(depth));
    let generous = Limits::new(usize::MAX, 64 * 1024 * 1024, 10_000_000, 1_024);
    let error = yxml::from_utf8_with_limits(&deep, generous)
        .unwrap_err()
        .to_string();
    assert!(error.contains("depth"), "{error}");
}

#[test]
fn a_declaration_naming_another_encoding_is_reported() {
    let error = from_xml_scalar("<?xml version='1.0' encoding='ISO-8859-1'?><row>x</row>")
        .unwrap_err()
        .to_string();
    assert!(error.contains("utf-8"), "{error}");
    // The declaration this codec can honor is accepted.
    assert!(from_xml_scalar("<?xml version='1.0' encoding='UTF-8'?><row>x</row>").is_ok());
}

#[test]
fn a_document_has_exactly_one_element_and_nothing_beside_it() {
    for input in ["<a/><b/>", "text<a/>", "<a></b>", "<a>"] {
        assert!(from_xml_scalar(input).is_err(), "{input}");
    }
}

#[test]
fn the_limits_bound_bytes_nodes_and_documents() {
    let input = "<row><id>1</id></row>";
    assert!(yxml::from_utf8_with_limits(input, Limits::new(64, 4, 1_000, 8)).is_err());
    assert!(yxml::from_utf8_with_limits(input, Limits::new(64, 1 << 20, 1, 8)).is_err());
    let mut reader = Cursor::new(input.as_bytes());
    assert!(
        yxml::from_reader_iter_with_limits(&mut reader, Limits::new(64, 1 << 20, 1_000, 0))
            .next()
            .unwrap()
            .is_err()
    );
}

#[test]
fn every_transport_reads_the_same_document() {
    let input = "<row><id>1</id></row>";
    let expected = from_xml_scalar(input).unwrap();
    assert_eq!(yxml::from_utf8(input).unwrap(), expected);
    assert_eq!(yxml::from_bytes(input.as_bytes()).unwrap(), expected);
    assert_eq!(yxml::from_reader(Cursor::new(input)).unwrap(), expected);
    assert_eq!(
        yxml::from_reader(OneByte(Cursor::new(input))).unwrap(),
        expected
    );
    assert_eq!(yxml::from_utf8_all(input).unwrap(), vec![expected.clone()]);
    let mut reader = Cursor::new(input.as_bytes());
    assert_eq!(
        yxml::from_reader_iter(&mut reader)
            .collect::<Result<Vec<_>, Error>>()
            .unwrap(),
        vec![expected]
    );
}

#[test]
fn a_byte_order_mark_is_not_content() {
    assert_eq!(
        from_xml_scalar("\u{feff}<row>1</row>").unwrap(),
        from_xml_scalar("<row>1</row>").unwrap()
    );
}

#[test]
fn the_layout_a_dump_asks_for_changes_bytes_and_not_meaning() {
    let value = from_xml_scalar("<row><a>1</a><b><c>2</c></b></row>").unwrap();
    let indented = yxml::into_utf8_with_formatting(&value, Formatting::indented(2)).unwrap();
    assert_eq!(
        indented,
        "<row>\n  <a>1</a>\n  <b>\n    <c>2</c>\n  </b>\n</row>"
    );
    assert_eq!(from_xml_scalar(&indented).unwrap(), value);
    assert_eq!(
        yxml::into_utf8_with_formatting(&value, Formatting::default().with_indent(Indent::None))
            .unwrap(),
        yxml::into_utf8(&value).unwrap()
    );
}

#[test]
fn xml_is_one_document_so_it_writes_one_value() {
    let value = from_xml_scalar("<row>1</row>").unwrap();
    assert!(yxml::into_utf8_all(std::slice::from_ref(&value)).is_ok());
    let error = yxml::into_utf8_all(&[value.clone(), value])
        .unwrap_err()
        .to_string();
    assert!(error.contains("one document element"), "{error}");
}

#[test]
fn the_format_vocabulary_names_xml_everywhere_it_is_asked() {
    assert_eq!(Format::from_str("xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_extension(".xml").unwrap(), Format::Xml);
    assert_eq!(Format::Xml.mime_type(), MimeType::XML);
    assert_eq!(Format::from_mime_type(&MimeType::XML).unwrap(), Format::Xml);
    assert_eq!(Format::Xml.extension(), "xml");
    assert_eq!(Xml.format(), Format::Xml);
    assert!(!Xml.is_multi_document());
    assert_eq!(
        Structured::for_url(&Url::from_str("file:///rows.xml.gz").unwrap()).unwrap(),
        Structured::Xml
    );
    assert_eq!(Structured::from_format(Format::Xml), Structured::Xml);
}

#[test]
fn content_inference_reads_markup_as_xml() {
    assert_eq!(
        yggdryl::text::infer_format(b"<?xml version=\"1.0\"?><row>1</row>").unwrap(),
        Format::Xml
    );
    assert_eq!(
        yggdryl::text::infer_format(b"  <row>1</row>").unwrap(),
        Format::Xml
    );
    // A document that only looks like markup is still whatever it is.
    assert_eq!(
        yggdryl::text::infer_format(b"{\"a\":1}").unwrap(),
        Format::Json
    );
}

#[test]
fn a_handle_named_xml_reads_and_writes_one_scalar() {
    let media = Url::from_str("file:///trade.xml.gz").unwrap().media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_record([(
        "trade",
        Scalar::from_record([("quantity", Scalar::from("2"))]).unwrap(),
    )])
    .unwrap();
    handle.write_scalar(&value).unwrap();
    assert_eq!(handle.read_scalar(None).unwrap(), value);

    let field = Field::from_str("trade: struct<quantity: int64 not null> not null").unwrap();
    assert_eq!(
        handle.read_scalar(Some(&field)).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64)])
    );
}
