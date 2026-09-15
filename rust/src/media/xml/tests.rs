//! Tests for the parts of the XML codec a caller cannot reach.
//!
//! The document walk and the row walk are `pub(crate)`: what a caller sees is
//! the record surface over a handle, which `rust/tests/media/xml.rs` covers.
//! These pin the mapping itself, where a refusal is cheapest to read.

use crate::text::Limits;
use crate::{Result, Scalar};

use super::document::{read_document, read_rows};

fn document(input: &str) -> Result<Scalar> {
    read_document(input.as_bytes(), Limits::default())
}

fn rows(input: &str) -> Result<(smol_str::SmolStr, Vec<Scalar>)> {
    read_rows(input.as_bytes(), Limits::default(), None).map(|(name, rows, _)| (name, rows))
}

#[test]
fn an_attribute_and_a_child_element_are_one_namespace_of_columns() {
    let value = document(r#"<Order id="1"><Px>9.5</Px></Order>"#).unwrap();
    assert_eq!(value.get_key_str("id").and_then(Scalar::as_str), Some("1"));
    assert_eq!(
        value.get_key_str("Px").and_then(Scalar::as_str),
        Some("9.5")
    );
}

#[test]
fn a_leaf_is_its_characters_and_an_empty_element_is_the_empty_string() {
    assert_eq!(document("<a>text</a>").unwrap(), Scalar::from("text"));
    assert_eq!(document("<a/>").unwrap(), Scalar::from(""));
    assert_eq!(document("<a></a>").unwrap(), Scalar::from(""));
}

#[test]
fn a_repeated_child_is_one_sequence_in_document_order() {
    let value = document("<r><L>1</L><L>2</L></r>").unwrap();
    assert_eq!(
        value.get_key_str("L"),
        Some(&Scalar::from_sequence([
            Scalar::from("1"),
            Scalar::from("2")
        ]))
    );
}

#[test]
fn entities_and_cdata_are_content_and_a_run_is_accumulated_not_replaced() {
    // A text run splits at every entity boundary, so keeping the last event
    // would keep a fragment.
    assert_eq!(document("<a>x &lt; y</a>").unwrap(), Scalar::from("x < y"));
    assert_eq!(document("<a>&#65;&#x42;</a>").unwrap(), Scalar::from("AB"));
    assert_eq!(
        document("<a><![CDATA[<raw>]]></a>").unwrap(),
        Scalar::from("<raw>")
    );
}

#[test]
fn an_attribute_value_is_normalized_rather_than_left_escaped() {
    let value = document(r#"<a v="A&amp;B"/>"#).unwrap();
    assert_eq!(value.get_key_str("v").and_then(Scalar::as_str), Some("A&B"));
}

#[test]
fn a_namespace_declaration_is_a_binding_and_never_a_column() {
    let value = document(r#"<a xmlns="u" xmlns:p="v" id="1"/>"#).unwrap();
    assert_eq!(value.get_key_str("id").and_then(Scalar::as_str), Some("1"));
    assert!(value.get_key_str("xmlns").is_none());
    assert!(value.get_key_str("p").is_none());
}

#[test]
fn xsi_nil_is_null_and_absence_is_null() {
    let value = document(
        r#"<r xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><a xsi:nil="true"/></r>"#,
    )
    .unwrap();
    assert_eq!(value.get_key_str("a"), Some(&Scalar::Null));
}

#[test]
fn the_shapes_the_row_model_cannot_hold_are_refused_by_name() {
    for (input, expected) in [
        ("<r>text<c/></r>", "got both"),
        (r#"<r id="1"><id>2</id></r>"#, "to be spelled one way"),
        ("<r/>after", "content after it"),
        ("<r><a>", "expected every element to close"),
        (
            "<!DOCTYPE r [<!ENTITY x \"y\">]><r/>",
            "internal DTD subset",
        ),
        ("<a>&unknown;</a>", "predefined entities"),
    ] {
        let error = document(input).unwrap_err().to_string();
        assert!(error.contains(expected), "{input:?} gave {error}");
    }
}

#[test]
fn rows_are_the_document_elements_children_and_must_agree_on_one_name() {
    let (name, values) = rows("<rows><Order id='1'/><Order id='2'/></rows>").unwrap();
    assert_eq!(name, "Order");
    assert_eq!(values.len(), 2);
    assert_eq!(
        values[1].get_key_str("id").and_then(Scalar::as_str),
        Some("2")
    );

    let error = rows("<rows><A/><B/></rows>").unwrap_err().to_string();
    assert!(error.contains("expected every row to be <A>"), "{error}");
}

#[test]
fn the_budgets_bound_a_document_and_say_where_they_stopped() {
    let deep = "<a>".repeat(40) + &"</a>".repeat(40);
    let error = read_document(deep.as_bytes(), Limits::new(8, 4096, 1_000, 8))
        .unwrap_err()
        .to_string();
    assert!(error.contains("nesting depth limit exceeded"), "{error}");

    let wide = format!("<r>{}</r>", "<c/>".repeat(40));
    let error = read_document(wide.as_bytes(), Limits::new(64, 4096, 8, 8))
        .unwrap_err()
        .to_string();
    assert!(error.contains("decoded node limit exceeded"), "{error}");
}

use super::writer::{escape_text, write_rows};
use crate::text::{Formatting, Indent};

fn written(rows: &[Scalar]) -> String {
    let field = Scalar::from_sequence(rows.to_vec())
        .inferred_struct_field()
        .unwrap()
        .with_name("row");
    let mut out = Vec::new();
    write_rows(
        &mut out,
        "rows",
        &field,
        rows,
        Formatting::new().with_indent(Indent::None),
    )
    .unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn the_characters_a_parser_would_rewrite_are_written_as_references() {
    // Line-ending normalization turns a literal carriage return into a
    // newline, so a reference is the only spelling that survives a round trip.
    let mut out = String::new();
    escape_text("a\rb&c<d>e", &mut out);
    assert_eq!(out, "a&#xD;b&amp;c&lt;d&gt;e");
}

#[test]
fn a_written_document_reads_back_to_the_value_that_was_written() {
    let rows = vec![
        Scalar::from_record([
            ("id", Scalar::from("1")),
            ("note", Scalar::from("x < y & z")),
        ])
        .unwrap(),
        Scalar::from_record([("id", Scalar::from("2")), ("note", Scalar::from("\ttab"))]).unwrap(),
    ];
    let encoded = written(&rows);
    let (name, read, _) = read_rows(encoded.as_bytes(), Limits::default(), None).unwrap();
    assert_eq!(name, "row");
    assert_eq!(read, rows, "{encoded}");
}

#[test]
fn a_name_no_element_can_be_called_is_refused_rather_than_written() {
    let rows = vec![Scalar::from_record([("not a name", Scalar::from("1"))]).unwrap()];
    let field = Scalar::from_sequence(rows.clone())
        .inferred_struct_field()
        .unwrap()
        .with_name("row");
    let mut out = Vec::new();
    let error = write_rows(&mut out, "rows", &field, &rows, Formatting::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains("an XML element can be called"), "{error}");
}

#[test]
fn characters_beside_attributes_are_the_elements_own_value() {
    // `<Amt Ccy="EUR">9.50</Amt>` is the commonest shape in a real document
    // and the text is the point of it; dropping it would be silent loss.
    let value = document(r#"<Amt Ccy="EUR">9.50</Amt>"#).unwrap();
    assert_eq!(
        value.get_key_str("Ccy").and_then(Scalar::as_str),
        Some("EUR")
    );
    assert_eq!(
        value.get_key_str("value").and_then(Scalar::as_str),
        Some("9.50")
    );

    // Characters beside *elements* are still mixed content and still refused.
    let error = document("<r>text<c/></r>").unwrap_err().to_string();
    assert!(error.contains("got both"), "{error}");

    // An attribute already called `value` is claiming the name twice.
    let error = document(r#"<Amt value="x">9.50</Amt>"#)
        .unwrap_err()
        .to_string();
    assert!(error.contains("got an attribute beside"), "{error}");
}

#[test]
fn a_prefixed_sibling_is_skipped_without_eating_the_document() {
    // `read_to_end` matches the end tag as written, so a local name would
    // never find `</p:meta>` and would run to the end of the input.
    let (name, rows, _) = read_rows(
        b"<feed xmlns:p='u'><p:meta>x</p:meta><Trade><id>1</id></Trade></feed>",
        Limits::default(),
        Some("Trade"),
    )
    .unwrap();
    assert_eq!(name, "Trade");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get_key_str("id").and_then(Scalar::as_str),
        Some("1")
    );
}

#[test]
fn a_named_row_element_is_found_wherever_the_wrapper_puts_it() {
    let (_, rows, _) = read_rows(
        b"<feed><data><Trade><id>1</id></Trade><Trade><id>2</id></Trade></data></feed>",
        Limits::default(),
        Some("Trade"),
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn the_record_walk_refuses_content_after_the_document_element() {
    for input in [
        "<rows><r><a>1</a></r></rows><more/>",
        "<rows><r><a>1</a></r></rows>junk",
        "<rows/>junk",
        "<rows/><other/>",
    ] {
        let error = read_rows(input.as_bytes(), Limits::default(), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("content after it"), "{input:?} gave {error}");
    }
}

#[test]
fn rows_bound_in_two_namespaces_are_two_vocabularies() {
    let error = read_rows(
        b"<rows xmlns:a='u1' xmlns:b='u2'><a:row><x>1</x></a:row><b:row><x>2</x></b:row></rows>",
        Limits::default(),
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("expected one namespace for the row"),
        "{error}"
    );
}

#[test]
fn a_prefix_nothing_declared_is_a_broken_document_rather_than_a_column() {
    // Two undeclared prefixes would otherwise fold onto one local name and
    // merge two vocabularies into one column without a word.
    for input in [
        "<r><p:x>1</p:x><q:x>2</q:x></r>",
        "<p:a/>",
        r#"<a p:v="1"/>"#,
    ] {
        let error = document(input).unwrap_err().to_string();
        assert!(error.contains("bound to nothing"), "{input:?} gave {error}");
    }
}

#[test]
fn a_document_carries_only_what_xml_can_carry() {
    // XML 1.0's Char production, on the way in and on the way out. Neither a
    // raw byte nor a reference can spell these, so both are refused.
    for input in [
        "<a>\u{0}</a>",
        "<a>&#0;</a>",
        "<a>&#x8;</a>",
        "<a>&#xFFFF;</a>",
    ] {
        assert!(document(input).is_err(), "{input:?} was accepted");
    }
    // A malformed reference is not a character either.
    for input in ["<a>&#X41;</a>", "<a>&#+65;</a>", "<a>&#x+41;</a>"] {
        assert!(document(input).is_err(), "{input:?} was accepted");
    }
    let error = crate::media::xml::into_utf8("a", &Scalar::from("\u{0}"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("XML can carry"), "{error}");
}

#[test]
fn line_endings_are_normalized_before_the_document_is_read() {
    // A conforming processor folds CRLF and a lone CR into one newline before
    // parsing, CDATA included, so two documents differing only in line ending
    // are one document.
    for input in ["<a>x\r\ny</a>", "<a>x\ry</a>", "<a><![CDATA[x\ry]]></a>"] {
        assert_eq!(document(input).unwrap(), Scalar::from("x\ny"), "{input:?}");
    }
}

#[test]
fn a_document_declaring_another_charset_is_refused_rather_than_misread() {
    // Text crosses the boundary once, before this parser runs. A prolog naming
    // a different charset is the document saying it is not the one decoded.
    let error =
        crate::media::xml::from_bytes(b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><a>x</a>")
            .unwrap_err()
            .to_string();
    assert!(error.contains("already decoded to UTF-8"), "{error}");

    // The one it is in is fine, spelled either way.
    for declaration in ["UTF-8", "utf8"] {
        let input = format!("<?xml version=\"1.0\" encoding=\"{declaration}\"?><a>x</a>");
        assert_eq!(document(&input).unwrap(), Scalar::from("x"));
    }
}

#[test]
fn every_name_a_read_can_produce_a_write_can_spell() {
    // U+00B7 is a legal XML NameChar, so a column read under that name has to
    // be writable again.
    let value = document("<r><a\u{b7}b>1</a\u{b7}b></r>").unwrap();
    assert!(value.get_key_str("a\u{b7}b").is_some());
    let written = crate::media::xml::into_utf8("r", &value).unwrap();
    assert!(written.contains("a\u{b7}b"), "{written}");
    assert_eq!(document(&written).unwrap(), value);
}

#[test]
fn a_mapping_named_by_strings_is_the_record_it_describes() {
    // A host runtime's dictionary arrives as a mapping rather than a record.
    // It is the same named shape, so it is written as the element it
    // describes rather than rendered as text.
    let value = Scalar::from_mapping([(Scalar::from("symbol"), Scalar::from("AAPL"))]).unwrap();
    let document = crate::media::xml::into_utf8("Order", &value).unwrap();
    assert!(document.contains("<symbol>AAPL</symbol>"), "{document}");

    // A mapping keyed by anything else has no element name to be written
    // under, and says so.
    let positional = Scalar::from_mapping([(Scalar::from(1), Scalar::from("x"))]).unwrap();
    let error = crate::media::xml::into_utf8("r", &positional)
        .unwrap_err()
        .to_string();
    assert!(error.contains("an element can hold"), "{error}");
}
