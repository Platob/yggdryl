//! `rust/src/xml/parser.rs`: what the document reading tolerates, decodes and
//! refuses, every refusal naming the byte where it stopped.

use yggdryl::xml;
use yggdryl::{Error, Limits, Scalar};

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).unwrap()
}

fn text(value: &str) -> Scalar {
    Scalar::from(value)
}

/// The root element's value.
fn root(document: &str) -> Scalar {
    let parsed = xml::from_utf8(document).unwrap_or_else(|error| panic!("{document}: {error}"));
    let entries = parsed.as_struct().expect("a document is a record");
    assert_eq!(entries.len(), 1, "one document element");
    entries.values().next().cloned().unwrap()
}

/// The refusal a malformed document is met with.
fn refusal(document: &str) -> (usize, String) {
    match xml::from_utf8(document).expect_err("a refusal") {
        Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "xml");
            (position, reason.to_string())
        }
        other => panic!("{other}"),
    }
}

#[test]
fn a_document_is_a_record_naming_its_root_element() {
    assert_eq!(
        xml::from_utf8("<order>1</order>").unwrap(),
        record([("order", text("1"))])
    );
    assert_eq!(
        xml::from_utf8("<ns:order>1</ns:order>").unwrap(),
        record([("ns:order", text("1"))]),
        "a prefix is part of the name the document spells"
    );
}

#[test]
fn a_prolog_comments_instructions_and_a_mark_surround_the_root() {
    let value = xml::from_utf8(
        "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <!DOCTYPE order SYSTEM \"order.dtd\">\n\
         <!-- issued -->\n<?xml-stylesheet href=\"none\"?>\n\
         <order><!-- inside --><?pi data?><id>7</id></order>\n<!-- trailer -->\n\t ",
    )
    .unwrap();
    assert_eq!(value, record([("order", record([("id", text("7"))]))]));
}

#[test]
fn a_leaf_is_its_text_a_self_closed_element_is_null_and_an_empty_body_is_empty_text() {
    assert_eq!(root("<a>text</a>"), text("text"));
    assert_eq!(
        root("<a/>"),
        Scalar::Null,
        "self-closed, the element says nothing"
    );
    assert_eq!(
        root("<a></a>"),
        text(""),
        "opened and closed, the element is present and its text is empty"
    );
    assert_eq!(
        root("<a k=\"v\"></a>"),
        record([("@k", text("v")), ("#text", text(""))]),
        "beside an attribute the empty body is an empty text entry"
    );
    assert_eq!(
        root("<a k=\"v\"/>"),
        record([("@k", text("v"))]),
        "and the self-closed element has no text entry"
    );
    assert_eq!(
        root("<a>  </a>"),
        text("  "),
        "a leaf keeps its text exactly"
    );
    assert_eq!(
        root("<a>\n  12\n</a>"),
        text("\n  12\n"),
        "a leaf keeps its text exactly, whitespace included"
    );
    assert_eq!(
        root("<a><![CDATA[]]></a>"),
        text(""),
        "an empty CDATA section is empty text, not absence"
    );
}

#[test]
fn attributes_are_prefixed_entries_and_text_beside_them_is_the_text_key() {
    assert_eq!(
        root("<a id=\"7\" xmlns=\"urn:x\">text</a>"),
        record([
            ("@id", text("7")),
            ("@xmlns", text("urn:x")),
            ("#text", text("text")),
        ])
    );
    assert_eq!(
        root("<a id='7'/>"),
        record([("@id", text("7"))]),
        "single quotes are quotes too, and an attribute alone is a record"
    );
    assert_eq!(
        root("<a k=\"one\ttwo\nthree\"/>"),
        record([("@k", text("one two three"))]),
        "attribute values are normalized as XML reads them"
    );
    assert_eq!(
        root("<a k=\"&lt;&amp;&quot;&#x41;\"/>"),
        record([("@k", text("<&\"A"))]),
        "references resolve inside attribute values"
    );
}

#[test]
fn children_are_entries_and_a_repeated_name_is_a_sequence_in_document_order() {
    assert_eq!(
        root("<a><b>1</b><c>2</c><b>3</b><d/></a>"),
        record([
            ("b", Scalar::from_sequence([text("1"), text("3")])),
            ("c", text("2")),
            ("d", Scalar::Null),
        ])
    );
    assert_eq!(
        root("<a><b><c>1</c></b><b><c>2</c><c>3</c></b></a>"),
        record([(
            "b",
            Scalar::from_sequence([
                record([("c", text("1"))]),
                record([("c", Scalar::from_sequence([text("2"), text("3")]))]),
            ])
        )])
    );
}

#[test]
fn indentation_between_children_is_dropped_and_mixed_content_is_kept() {
    assert_eq!(
        root("<a>\n  <b>1</b>\n  <c>2</c>\n</a>"),
        record([("b", text("1")), ("c", text("2"))])
    );
    assert_eq!(
        root("<p>Hello <b>world</b>!</p>"),
        record([("#text", text("Hello !")), ("b", text("world"))]),
        "text around children is joined into the text key"
    );
    assert_eq!(
        root("<a k=\"1\">\n  <b>2</b>\n</a>"),
        record([("@k", text("1")), ("b", text("2"))])
    );
}

#[test]
fn character_data_is_decoded_from_references_and_cdata() {
    assert_eq!(
        root("<a>&lt;p&gt; &amp; &quot;q&quot; &apos;s&apos;</a>"),
        text("<p> & \"q\" 's'")
    );
    assert_eq!(root("<a>&#65;&#x42;&#x1F600;</a>"), text("AB\u{1F600}"));
    assert_eq!(
        root("<a><![CDATA[<not>&markup;]]> and &amp; more</a>"),
        text("<not>&markup; and & more"),
        "CDATA is literal and joins the text around it"
    );
    assert_eq!(
        root("<a>line\r\nbreak\rhere</a>"),
        text("line\nbreak\nhere"),
        "end-of-line normalization is XML's own"
    );
}

#[test]
fn a_nil_element_is_null_and_a_nil_element_with_content_is_read() {
    assert_eq!(root("<a xsi:nil=\"true\"/>"), Scalar::Null);
    assert_eq!(root("<a xsi:nil=\"1\">  </a>"), Scalar::Null);
    assert_eq!(
        root("<a xsi:nil=\"true\"></a>"),
        Scalar::Null,
        "nil with an empty body is the absence it declares"
    );
    assert_eq!(
        root("<a xsi:nil=\"true\" k=\"1\"/>"),
        record([("@k", text("1")), ("@xsi:nil", text("true"))]),
        "nil beside another attribute is one attribute among them"
    );
    assert_eq!(
        root("<a xsi:nil=\"false\"/>"),
        record([("@xsi:nil", text("false"))]),
        "nil declared false is an ordinary attribute"
    );
    assert_eq!(
        root("<a xsi:nil=\"true\">1</a>"),
        record([("@xsi:nil", text("true")), ("#text", text("1"))]),
        "an element that says nil and carries a value is read as it is"
    );
}

#[test]
fn malformed_documents_are_refused_naming_the_position() {
    let (position, reason) = refusal("<a><b></a>");
    assert!(
        reason.contains("</a>") || reason.contains("`a`") || reason.contains("b"),
        "{reason}"
    );
    assert!(position > 0, "{position}");

    let (_, reason) = refusal("<a>1</a><b/>");
    assert!(reason.contains("after the root element"), "{reason}");

    let (_, reason) = refusal("text<a/>");
    assert!(reason.contains("outside the root element"), "{reason}");

    let (_, reason) = refusal("<a/>trailing");
    assert!(reason.contains("outside the root element"), "{reason}");

    let (_, reason) = refusal("<a><b>");
    assert!(
        reason.contains("never closed") || reason.contains("b"),
        "{reason}"
    );

    let (_, reason) = refusal("");
    assert!(reason.contains("expected the root element"), "{reason}");

    let (_, reason) = refusal("  <!-- only a comment -->  ");
    assert!(reason.contains("expected the root element"), "{reason}");

    let (_, reason) = refusal("<a>&custom;</a>");
    assert!(reason.contains("&custom;"), "{reason}");

    let (_, reason) = refusal("<a>&#0;</a>");
    assert!(!reason.is_empty(), "{reason}");

    let (_, reason) = refusal("<a k=\"1\" k=\"2\"/>");
    assert!(reason.to_lowercase().contains("duplicate"), "{reason}");

    let (_, reason) = refusal("<a k=1/>");
    assert!(!reason.is_empty(), "{reason}");

    let (_, reason) = refusal("<a></b>");
    assert!(!reason.is_empty(), "{reason}");
}

#[test]
fn input_that_is_not_utf8_is_refused_at_the_first_bad_byte() {
    let (position, reason) = match xml::from_bytes(b"<a>\xff</a>").expect_err("a refusal") {
        Error::Codec {
            position, reason, ..
        } => (position, reason.to_string()),
        other => panic!("{other}"),
    };
    assert_eq!(position, 3);
    assert!(reason.contains("UTF-8"), "{reason}");
}

#[test]
fn limits_bound_bytes_nodes_depth_and_documents() {
    let document = "<a><b><c>1</c></b></a>";
    let generous = Limits::new(8, 1024, 64, 1);
    assert!(xml::from_utf8_with_limits(document, generous).is_ok());

    let shallow = Limits::new(2, 1024, 64, 1);
    let error = xml::from_utf8_with_limits(document, shallow).unwrap_err();
    assert!(error.to_string().contains("depth"), "{error}");

    let few_nodes = Limits::new(8, 1024, 2, 1);
    let error = xml::from_utf8_with_limits(document, few_nodes).unwrap_err();
    assert!(error.to_string().contains("node"), "{error}");

    let short = Limits::new(8, 4, 64, 1);
    let error = xml::from_utf8_with_limits(document, short).unwrap_err();
    assert!(error.to_string().contains("byte"), "{error}");

    let none = Limits::new(8, 1024, 64, 0);
    let error = xml::from_utf8_with_limits(document, none).unwrap_err();
    assert!(error.to_string().contains("document"), "{error}");

    let attributes = "<a k=\"1\" l=\"2\"/>";
    let error = xml::from_utf8_with_limits(attributes, Limits::new(8, 1024, 2, 1)).unwrap_err();
    assert!(
        error.to_string().contains("node"),
        "every attribute is a node: {error}"
    );
}

#[test]
fn the_parser_hard_ceiling_holds_whatever_the_limits_ask() {
    let depth = xml::MAX_PARSER_DEPTH + 1;
    let document = format!("{}{}", "<d>".repeat(depth), "</d>".repeat(depth));
    let error = xml::from_utf8_with_limits(
        &document,
        Limits::new(usize::MAX, usize::MAX, usize::MAX, 1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("hard limit"), "{error}");

    let within = format!(
        "{}{}",
        "<d>".repeat(xml::MAX_PARSER_DEPTH),
        "</d>".repeat(xml::MAX_PARSER_DEPTH)
    );
    assert!(
        xml::from_utf8_with_limits(&within, Limits::new(usize::MAX, usize::MAX, usize::MAX, 1),)
            .is_ok()
    );
}

#[test]
fn names_and_characters_xml_1_0_forbids_are_refused() {
    for (document, at) in [
        ("<1a/>", 0),
        ("<-a/>", 0),
        ("<a 1x=\"2\"/>", 0),
        ("<a.b:c d.e=\"1\"><1/></a.b:c>", 15),
    ] {
        let (position, reason) = refusal(document);
        assert!(
            reason.contains("expected an XML name"),
            "{document}: {reason}"
        );
        assert_eq!(position, at, "{document}: the tag the name opens");
    }
    assert!(xml::from_utf8("<></>").is_err(), "an empty name is no name");

    for document in [
        "<a>&#1;</a>",
        "<a>&#x1;</a>",
        "<a>bell\u{7}</a>",
        "<a k=\"\u{1}\"/>",
        "<a>&#xD800;</a>",
        "<a>&#xFFFE;</a>",
    ] {
        let (_, reason) = refusal(document);
        assert!(!reason.is_empty(), "{document}: {reason}");
    }
    assert_eq!(
        root("<a>\u{9}\u{A}\u{D} \u{E000}\u{10FFFF}</a>"),
        text("\u{9}\u{A}\u{A} \u{E000}\u{10FFFF}"),
        "the characters XML 1.0 allows are read, the line end normalized"
    );
}

#[test]
fn positions_count_the_byte_order_mark_a_document_opens_with() {
    let (bare, reason) = refusal("<a>&custom;</a>");
    let (marked, marked_reason) = refusal("\u{FEFF}<a>&custom;</a>");
    assert_eq!(reason, marked_reason);
    assert_eq!(
        marked,
        bare + "\u{FEFF}".len(),
        "a position is a byte offset into the bytes the caller handed over"
    );
}
