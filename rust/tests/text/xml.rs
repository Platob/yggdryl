use std::io::{Cursor, Read};

use yggdryl::text::{Format, Formatting, Indent, Limits, Structured, TextCodec, Xml, xml};
use yggdryl::{
    DataType, DataTypeId, Error, Field, MimeType, Scalar, TimeUnit, Url, from_xml_scalar,
    from_xml_scalar_with_field, into_xml_scalar,
};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

fn reason(error: &Error) -> String {
    error.to_string()
}

#[test]
fn a_document_is_one_root_element_keyed_by_its_name() {
    let value = from_xml_scalar("<trade><symbol>AAPL</symbol></trade>").unwrap();
    assert_eq!(
        value,
        Scalar::from_record([(
            "trade",
            Scalar::from_record([("symbol", Scalar::from("AAPL"))]).unwrap(),
        )])
        .unwrap()
    );
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        "<trade><symbol>AAPL</symbol></trade>"
    );
}

#[test]
fn attributes_and_character_data_are_keyed_apart_from_elements() {
    let value = from_xml_scalar(r#"<trade id="7" ns:seq="2">filled</trade>"#).unwrap();
    let trade = value.get_key_str("trade").unwrap();
    assert_eq!(trade.get_key_str("@id").and_then(Scalar::as_str), Some("7"));
    assert_eq!(
        trade.get_key_str("@ns:seq").and_then(Scalar::as_str),
        Some("2")
    );
    assert_eq!(
        trade.get_key_str("#text").and_then(Scalar::as_str),
        Some("filled")
    );
    // No XML name starts with either character, so neither can collide with a
    // child element of the same name.
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        r#"<trade id="7" ns:seq="2">filled</trade>"#
    );
}

#[test]
fn a_repeated_element_is_a_sequence_in_document_order() {
    let value = from_xml_scalar("<row><tag>a</tag><tag>b</tag><other/></row>").unwrap();
    let row = value.get_key_str("row").unwrap();
    assert_eq!(
        row.get_key_str("tag").unwrap(),
        &Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")])
    );
    assert_eq!(row.get_key_str("other"), Some(&Scalar::Null));
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        "<row><other/><tag>a</tag><tag>b</tag></row>"
    );
}

#[test]
fn an_element_with_no_character_data_is_absence() {
    for document in ["<row/>", "<row></row>"] {
        assert_eq!(
            from_xml_scalar(document).unwrap(),
            Scalar::from_record([("row", Scalar::Null)]).unwrap(),
            "{document}"
        );
    }
    // The two spellings are one document, so an empty string is written and
    // read back as absence rather than as a second empty value.
    let empty = Scalar::from_record([("row", Scalar::from(""))]).unwrap();
    assert_eq!(into_xml_scalar(&empty).unwrap(), "<row></row>");
    assert_eq!(
        from_xml_scalar("<row></row>").unwrap(),
        Scalar::from_record([("row", Scalar::Null)]).unwrap()
    );
}

#[test]
fn whitespace_between_elements_is_layout_and_inside_a_leaf_is_content() {
    let laid_out = from_xml_scalar("<row>\n  <id>1</id>\n</row>").unwrap();
    assert_eq!(
        laid_out,
        from_xml_scalar("<row><id>1</id></row>").unwrap(),
        "whitespace between child elements lays a document out"
    );
    assert_eq!(
        from_xml_scalar("<row> 1 </row>")
            .unwrap()
            .get_key_str("row")
            .and_then(Scalar::as_str),
        Some(" 1 "),
        "a leaf's character data is what it says"
    );
}

#[test]
fn mixed_content_is_refused_where_it_starts() {
    let error = from_xml_scalar("<row>text<id>1</id></row>").unwrap_err();
    assert!(
        reason(&error).contains("character data or child elements, not both"),
        "{error}"
    );
    // The writer refuses the same document rather than emitting one no reader
    // of this crate would accept back.
    let mixed = Scalar::from_record([(
        "row",
        Scalar::from_record([("#text", Scalar::from("text")), ("id", Scalar::from(1))]).unwrap(),
    )])
    .unwrap();
    assert!(
        reason(&into_xml_scalar(&mixed).unwrap_err()).contains("not both"),
        "the writer refuses mixed content"
    );
}

#[test]
fn a_document_has_exactly_one_root_element() {
    let error = from_xml_scalar("<a/><b/>").unwrap_err();
    assert!(
        reason(&error).contains("exactly one root element"),
        "{error}"
    );
    assert!(reason(&from_xml_scalar("").unwrap_err()).contains("expected one XML root element"));
    assert!(reason(&from_xml_scalar("bare text").unwrap_err()).contains("outside the root"));

    let two = Scalar::from_record([("a", Scalar::Null), ("b", Scalar::Null)]).unwrap();
    assert!(reason(&into_xml_scalar(&two).unwrap_err()).contains("exactly one root element"));
    assert!(reason(&into_xml_scalar(&Scalar::from(1)).unwrap_err()).contains("one root element"));
}

#[test]
fn only_predefined_entities_and_character_references_resolve() {
    assert_eq!(
        from_xml_scalar("<row>a &amp; b &lt;c&gt; &#65; &apos;</row>")
            .unwrap()
            .get_key_str("row")
            .and_then(Scalar::as_str),
        Some("a & b <c> A '")
    );

    // No document-declared entity is ever expanded, so neither an external
    // reference nor a recursive one is reachable through this parser.
    let declared = "<!DOCTYPE lolz [<!ENTITY lol \"lol\">]><lolz>&lol;</lolz>";
    let error = from_xml_scalar(declared).unwrap_err();
    assert!(
        reason(&error).contains("lol"),
        "the refusal names it: {error}"
    );
    let error = from_xml_scalar("<row>&external;</row>").unwrap_err();
    assert!(reason(&error).contains("external"), "{error}");
}

#[test]
fn a_comment_an_instruction_and_a_doctype_annotate_without_valuing() {
    let document = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                    <!-- a note -->\n\
                    <!DOCTYPE row>\n\
                    <?render mode=\"fast\"?>\n\
                    <row><id>1</id><!-- inside --></row>";
    assert_eq!(
        from_xml_scalar(document).unwrap(),
        from_xml_scalar("<row><id>1</id></row>").unwrap()
    );
}

#[test]
fn a_declaration_that_disagrees_with_the_bytes_is_refused() {
    let error =
        from_xml_scalar("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><row/>").unwrap_err();
    assert!(
        reason(&error).contains("UTF-8 encoding declaration"),
        "{error}"
    );
    let error = from_xml_scalar("<?xml version=\"2.0\"?><row/>").unwrap_err();
    assert!(reason(&error).contains("1.0 or 1.1"), "{error}");
}

#[test]
fn a_namespace_prefix_is_kept_exactly_as_written() {
    // A record is keyed by name and sorted by it, so the attributes come back
    // in name order rather than in the order the document listed them.
    let document =
        r#"<ns:trade xmlns="urn:default" xmlns:ns="urn:example"><ns:id>1</ns:id></ns:trade>"#;
    let value = from_xml_scalar(document).unwrap();
    let trade = value.get_key_str("ns:trade").unwrap();
    assert_eq!(
        trade.get_key_str("@xmlns:ns").and_then(Scalar::as_str),
        Some("urn:example")
    );
    assert_eq!(
        trade.get_key_str("@xmlns").and_then(Scalar::as_str),
        Some("urn:default")
    );
    assert!(trade.get_key_str("ns:id").is_some());
    assert_eq!(into_xml_scalar(&value).unwrap(), document);
}

#[test]
fn cdata_is_content_and_never_unescaped() {
    let value = from_xml_scalar("<row><![CDATA[a <b> & c]]></row>").unwrap();
    assert_eq!(
        value.get_key_str("row").and_then(Scalar::as_str),
        Some("a <b> & c")
    );
    // One section and one escaped spelling are one document.
    assert_eq!(
        into_xml_scalar(&value).unwrap(),
        "<row>a &lt;b&gt; &amp; c</row>"
    );
}

#[test]
fn text_that_would_not_survive_a_read_is_written_as_a_reference() {
    let value = Scalar::from_record([(
        "row",
        Scalar::from_record([
            ("@note", Scalar::from("one\ttab\nline\rreturn")),
            ("#text", Scalar::from("body\r\nsecond")),
        ])
        .unwrap(),
    )])
    .unwrap();
    let encoded = into_xml_scalar(&value).unwrap();
    assert!(encoded.contains("&#9;"), "{encoded}");
    assert!(encoded.contains("&#10;"), "{encoded}");
    assert!(encoded.contains("&#13;"), "{encoded}");
    assert_eq!(from_xml_scalar(&encoded).unwrap(), value);

    // XML 1.0 has no spelling at all for the other control characters.
    let control = Scalar::from_record([("row", Scalar::from("\u{1}"))]).unwrap();
    assert!(
        reason(&into_xml_scalar(&control).unwrap_err()).contains("U+0001"),
        "the refusal names the character"
    );
}

#[test]
fn a_name_a_document_cannot_spell_is_refused_by_name() {
    for name in ["", "1st", "a b", "a>b"] {
        let value = Scalar::from_record([(name, Scalar::Null)]).unwrap();
        let error = into_xml_scalar(&value).unwrap_err();
        assert!(
            reason(&error).contains("expected an XML name"),
            "{name}: {error}"
        );
    }
    // A prefixed name and an underscore are names.
    for name in ["ns:total", "_private", "a-b.c"] {
        let value = Scalar::from_record([(name, Scalar::Null)]).unwrap();
        assert!(into_xml_scalar(&value).is_ok(), "{name}");
    }
}

#[test]
fn an_attribute_holds_one_text_value() {
    let nested = Scalar::from_record([(
        "row",
        Scalar::from_record([("@id", Scalar::from_record([("a", Scalar::Null)]).unwrap())])
            .unwrap(),
    )])
    .unwrap();
    assert!(
        reason(&into_xml_scalar(&nested).unwrap_err()).contains("one text value"),
        "an attribute cannot hold a record"
    );
}

#[test]
fn a_sequence_is_repeated_elements_and_never_nests() {
    let nested = Scalar::from_record([(
        "row",
        Scalar::from_record([(
            "tag",
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from("a")])]),
        )])
        .unwrap(),
    )])
    .unwrap();
    assert!(
        reason(&into_xml_scalar(&nested).unwrap_err()).contains("repeats an element"),
        "a sequence inside a sequence has no XML spelling"
    );
    // An empty sequence is no element at all.
    let empty = Scalar::from_record([(
        "row",
        Scalar::from_record([("tag", Scalar::from_sequence([]))]).unwrap(),
    )])
    .unwrap();
    assert_eq!(into_xml_scalar(&empty).unwrap(), "<row/>");
}

#[test]
fn every_leaf_is_character_data_until_a_field_types_it() {
    let document = "<row><n>1</n><f>1.5</f><b>true</b></row>";
    let natural = from_xml_scalar(document).unwrap();
    let row = natural.get_key_str("row").unwrap();
    for name in ["n", "f", "b"] {
        assert_eq!(
            row.get_key_str(name).map(Scalar::id),
            Some(DataTypeId::Utf8),
            "{name} is text until a field says otherwise"
        );
    }

    let field = Field::from_str("row: struct<n: int64, f: float64, b: bool> not null").unwrap();
    assert_eq!(
        from_xml_scalar_with_field(document, &field).unwrap(),
        Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(1.5_f64),
            Scalar::from(true),
        ])
    );
}

#[test]
fn field_directed_xml_restores_exact_leaves_inside_a_record() {
    let decimal = DataType::decimal128(18, 2).unwrap();
    let field = DataType::from_fields([
        decimal.clone().required_field("price"),
        DataType::Date32.required_field("day"),
        DataType::Binary.required_field("payload"),
        DataType::Uuid.required_field("key"),
        DataType::Duration64(TimeUnit::Second).required_field("held"),
    ])
    .unwrap()
    .required_field("row");

    let row = Scalar::from_record([
        ("price", decimal.scalar(Scalar::from(1250)).unwrap()),
        (
            "day",
            DataType::Date32.scalar(Scalar::from("2024-01-02")).unwrap(),
        ),
        ("payload", Scalar::from(vec![0_u8, 255])),
        (
            "key",
            DataType::Uuid
                .scalar(Scalar::from("67e55044-10b1-426f-9247-bb680e5fe0c8"))
                .unwrap(),
        ),
        (
            "held",
            DataType::Duration64(TimeUnit::Second)
                .scalar(Scalar::from(90_i64))
                .unwrap(),
        ),
    ])
    .unwrap();

    let document = Scalar::from_record([("row", row.clone())]).unwrap();
    let encoded = into_xml_scalar(&document).unwrap();
    assert!(encoded.contains("<price>1250.00</price>"), "{encoded}");
    assert!(encoded.contains("<day>2024-01-02</day>"), "{encoded}");
    assert!(encoded.contains("<payload>AP8=</payload>"), "{encoded}");
    assert!(encoded.contains("<held>PT90S</held>"), "{encoded}");

    let decoded = from_xml_scalar_with_field(&encoded, &field).unwrap();
    assert_eq!(decoded, field.from_natural_value(row).unwrap());
}

#[test]
fn an_interval_travels_as_the_parts_it_counts() {
    for (unit, parts) in [
        (TimeUnit::YearMonth, vec![Scalar::from(14_i64)]),
        (
            TimeUnit::DayTime,
            vec![Scalar::from(2_i64), Scalar::from(3_i64)],
        ),
        (
            TimeUnit::MonthDayNano,
            vec![
                Scalar::from(1_i64),
                Scalar::from(2_i64),
                Scalar::from(3_i64),
            ],
        ),
    ] {
        let interval = DataType::Interval(unit);
        let value = if parts.len() == 1 {
            interval.scalar(parts[0].clone()).unwrap()
        } else {
            interval
                .scalar(Scalar::from_sequence(parts.clone()))
                .unwrap()
        };
        let field = DataType::from_fields([interval.clone().required_field("span")])
            .unwrap()
            .required_field("row");
        let document = Scalar::from_record([(
            "row",
            Scalar::from_record([("span", value.clone())]).unwrap(),
        )])
        .unwrap();

        let encoded = into_xml_scalar(&document).unwrap();
        assert_eq!(
            encoded.matches("<span>").count(),
            parts.len(),
            "{unit}: {encoded}"
        );
        let decoded = from_xml_scalar_with_field(&encoded, &field).unwrap();
        assert_eq!(
            decoded.as_sequence().unwrap()[0].dtype().unwrap(),
            interval,
            "{unit}"
        );
        assert_eq!(decoded.as_sequence().unwrap()[0], value, "{unit}");
    }
}

#[test]
fn a_field_reads_the_three_shapes_a_document_cannot_spell() {
    let field = Field::from_str(
        "row: struct<id: int64 not null, tags: list<utf8 not null> not null, note: utf8> not null",
    )
    .unwrap();

    // One occurrence of a repeated element is one item.
    let one = from_xml_scalar_with_field("<row><id>1</id><tags>a</tags></row>", &field).unwrap();
    let values = one.as_sequence().unwrap();
    assert_eq!(
        values[1],
        Scalar::from_sequence([Scalar::from("a")]),
        "one occurrence is a one-item list"
    );
    assert_eq!(
        values[2],
        Scalar::Null,
        "an absent nullable element is absent"
    );

    // No occurrence at all is the empty list, which is the one absence XML
    // has no other spelling for.
    let none = from_xml_scalar_with_field("<row><id>1</id></row>", &field).unwrap();
    assert_eq!(
        none.as_sequence().unwrap()[1],
        Scalar::from_sequence([]),
        "a non-null list with no occurrence is empty"
    );

    // The root element's own name is what the field already names, so a
    // document written elsewhere reads under the declared root.
    let foreign = from_xml_scalar_with_field("<trade><id>1</id></trade>", &field).unwrap();
    assert_eq!(foreign.as_sequence().unwrap()[0], Scalar::from(1_i64));
}

#[test]
fn a_declared_name_the_document_does_not_carry_is_named_in_the_refusal() {
    let field = Field::from_str("row: struct<id: int64 not null> not null").unwrap();
    let error = from_xml_scalar_with_field("<row><other>1</other></row>", &field).unwrap_err();
    assert!(reason(&error).contains("other"), "{error}");
}

#[test]
fn formatting_changes_bytes_and_never_meaning() {
    let value = from_xml_scalar("<row><a>x</a><b><c>y</c></b></row>").unwrap();
    let compact = xml::into_utf8(&value).unwrap();
    assert_eq!(compact, "<row><a>x</a><b><c>y</c></b></row>");

    let indented = xml::into_utf8_with_formatting(
        &value,
        Formatting::default().with_indent(Indent::Spaces(2)),
    )
    .unwrap();
    assert_eq!(
        indented,
        "<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>"
    );
    assert_eq!(from_xml_scalar(&indented).unwrap(), value);

    let tabs =
        xml::into_utf8_with_formatting(&value, Formatting::default().with_indent(Indent::Tabs))
            .unwrap();
    assert!(tabs.contains("\n\t<a>x</a>"), "{tabs}");
    assert_eq!(from_xml_scalar(&tabs).unwrap(), value);

    // A leaf's character data is never laid out, because the layout would be
    // part of the value.
    let leaf = xml::into_utf8_with_formatting(
        &Scalar::from_record([("row", Scalar::from("x"))]).unwrap(),
        Formatting::default().with_indent(Indent::Spaces(4)),
    )
    .unwrap();
    assert_eq!(leaf, "<row>x</row>");
}

#[test]
fn limits_bound_bytes_depth_nodes_and_documents() {
    let deep = format!("{}{}", "<a>".repeat(40), "</a>".repeat(40));
    let error = xml::from_utf8_with_limits(&deep, Limits::new(8, 1 << 20, 1 << 20, 1)).unwrap_err();
    assert!(
        reason(&error).contains("nesting depth limit exceeded"),
        "{error}"
    );

    let error =
        xml::from_utf8_with_limits("<row><a/><b/><c/></row>", Limits::new(64, 1 << 20, 2, 1))
            .unwrap_err();
    assert!(reason(&error).contains("node limit exceeded"), "{error}");

    let error = xml::from_utf8_with_limits("<row/>", Limits::new(64, 3, 1 << 20, 1)).unwrap_err();
    assert!(
        reason(&error).contains("input byte limit exceeded"),
        "{error}"
    );

    let error =
        xml::from_utf8_with_limits("<row/>", Limits::new(64, 1 << 20, 1 << 20, 0)).unwrap_err();
    assert!(
        reason(&error).contains("document limit exceeded"),
        "{error}"
    );
}

#[test]
fn the_parser_ceiling_holds_whatever_a_caller_asks_for() {
    let depth = yggdryl::text::xml::MAX_PARSER_DEPTH + 8;
    let deep = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    let limits = Limits::new(usize::MAX, 1 << 24, 1 << 24, 1);
    assert!(
        reason(&xml::from_utf8_with_limits(&deep, limits).unwrap_err())
            .contains("nesting depth limit exceeded"),
        "an adversarial caller limit cannot turn nesting into stack exhaustion"
    );
}

#[test]
fn a_reader_yields_exactly_one_document() {
    let document = "<row><id>1</id></row>";
    assert_eq!(
        xml::from_reader(OneByte(Cursor::new(document))).unwrap(),
        from_xml_scalar(document).unwrap()
    );
    assert_eq!(
        xml::from_reader_all(Cursor::new(document)).unwrap().len(),
        1
    );

    let mut source = Cursor::new(document);
    let values = xml::from_reader_iter(&mut source)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(values.len(), 1);

    let error = xml::into_utf8_all(&[
        from_xml_scalar("<a/>").unwrap(),
        from_xml_scalar("<b/>").unwrap(),
    ])
    .unwrap_err();
    assert!(reason(&error).contains("multiple documents"), "{error}");
}

#[test]
fn the_format_is_reached_by_every_name_it_has() {
    assert_eq!(Format::from_str("xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_extension(".xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_path("trades.xml").unwrap(), Format::Xml);
    assert_eq!(Format::from_mime_type(&MimeType::XML).unwrap(), Format::Xml);
    assert_eq!(Format::Xml.mime_type(), MimeType::XML);
    assert_eq!(Format::Xml.as_str(), "xml");
    assert_eq!(Format::Xml.extension(), "xml");
    assert!(Format::Xml.is_single_document());
    assert!(Format::ALL.contains(&Format::Xml));

    assert_eq!(
        Structured::for_url(&Url::from_str("file:///trades.xml.gz").unwrap()).unwrap(),
        Structured::Xml
    );
    assert_eq!(Structured::from_format(Format::Xml), Structured::Xml);
    assert_eq!(Structured::from(Xml), Structured::Xml);
    assert!(!Xml.is_multi_document());
    assert_eq!(Xml.mime_type(), MimeType::XML);
    assert_eq!(
        Xml.from_utf8("<row/>").unwrap(),
        from_xml_scalar("<row/>").unwrap()
    );
}

#[test]
fn content_that_opens_a_tag_is_inferred_as_xml() {
    assert_eq!(
        yggdryl::text::infer_format(b"<row><id>1</id></row>").unwrap(),
        Format::Xml
    );
    let (format, value) = yggdryl::text::from_utf8_inferred("  <row><id>1</id></row>").unwrap();
    assert_eq!(format, Format::Xml);
    assert_eq!(value, from_xml_scalar("<row><id>1</id></row>").unwrap());

    // Content that opens a tag but is no document is still read by the format
    // that can read it.
    assert_eq!(
        yggdryl::text::infer_format(b"<<: *anchor\nid: 1\n").unwrap(),
        Format::Yaml
    );
    assert_eq!(
        yggdryl::text::infer_format(b"{\"id\":1}").unwrap(),
        Format::Json
    );
}

#[test]
fn placeholders_substitute_inside_a_document() {
    let loading = yggdryl::text::Loading::new().with_placeholders(
        yggdryl::text::Placeholders::new().with_variable("SYMBOL", Scalar::from("AAPL")),
    );
    let value = yggdryl::text::from_utf8_with(
        "<row><symbol>{{ SYMBOL }}</symbol></row>",
        Format::Xml,
        &loading,
    )
    .unwrap();
    assert_eq!(
        value
            .get_key_str("row")
            .and_then(|row| row.get_key_str("symbol"))
            .and_then(Scalar::as_str),
        Some("AAPL")
    );
}

#[test]
fn a_validation_refuses_before_a_destination_is_opened() {
    let unwritable = Scalar::from_record([("1st", Scalar::Null)]).unwrap();
    assert!(xml::validate_for_write(&unwritable).is_err());
    assert!(xml::validate_for_write(&from_xml_scalar("<row><id>1</id></row>").unwrap()).is_ok());
}

#[test]
fn a_refusal_names_the_byte_it_stopped_at() {
    let Error::Codec {
        format, position, ..
    } = from_xml_scalar("<row><id>1</id>").unwrap_err()
    else {
        panic!("an XML refusal is a codec error");
    };
    assert_eq!(format, "xml");
    assert_eq!(position, 0, "the element that was left open started there");

    let Error::Codec { position, .. } = from_xml_scalar("<a/><b/>").unwrap_err() else {
        panic!("an XML refusal is a codec error");
    };
    assert_eq!(position, 4, "the second root starts there");
}

#[test]
fn a_utf8_refusal_names_where_the_bytes_stop_being_text() {
    let Error::Codec { position, .. } = xml::from_bytes(b"<row>\xff</row>").unwrap_err() else {
        panic!("an XML refusal is a codec error");
    };
    assert_eq!(position, 5);
}

#[test]
fn a_byte_order_mark_and_the_prolog_around_a_root_are_not_content() {
    let expected = from_xml_scalar("<row><id>1</id></row>").unwrap();
    for document in [
        "\u{feff}<row><id>1</id></row>",
        "  \n<row><id>1</id></row>\n  ",
        "<!DOCTYPE row SYSTEM \"http://example.invalid/row.dtd\"><row><id>1</id></row>",
    ] {
        assert_eq!(from_xml_scalar(document).unwrap(), expected, "{document}");
    }
}

#[test]
fn an_attribute_is_read_once_and_normalized_as_the_specification_says() {
    // A repeated attribute names no value, so it is refused rather than
    // resolved to one of the two.
    let error = from_xml_scalar(r#"<a b="1" b="2"/>"#).unwrap_err();
    assert!(reason(&error).contains("duplicated attribute"), "{error}");

    // A literal line break inside an attribute is a space by the time any
    // reader sees it, which is why the writer spells one as a reference.
    assert_eq!(
        from_xml_scalar("<a b=\"x\ny\"/>")
            .unwrap()
            .get_key_str("a")
            .and_then(|a| a.get_key_str("@b"))
            .and_then(Scalar::as_str),
        Some("x y")
    );
    assert_eq!(
        from_xml_scalar("<a b=\"x&#10;y\"/>")
            .unwrap()
            .get_key_str("a")
            .and_then(|a| a.get_key_str("@b"))
            .and_then(Scalar::as_str),
        Some("x\ny")
    );
}

#[test]
fn character_data_arrives_with_its_line_endings_normalized() {
    // Every reader normalizes them, so a carriage return only survives as the
    // reference this crate's writer spells it with.
    assert_eq!(
        from_xml_scalar("<a>x\r\ny</a>")
            .unwrap()
            .get_key_str("a")
            .and_then(Scalar::as_str),
        Some("x\ny")
    );
    let value = Scalar::from_record([("a", Scalar::from("x\r\ny"))]).unwrap();
    assert_eq!(into_xml_scalar(&value).unwrap(), "<a>x&#13;\ny</a>");
    assert_eq!(
        from_xml_scalar(&into_xml_scalar(&value).unwrap()).unwrap(),
        value
    );
}
