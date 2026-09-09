# XML

One document read as the record its element names, backed by the shared Rust codec.

## Contract

| | |
| --- | --- |
| Root | the one-entry `Record` the document element names, so the root's own name survives a round trip |
| Attribute | a field whose name carries `@`; an element's own character data is `#text`; neither is a valid XML name, so neither collides with a child |
| Repetition | a child name that occurs once is that value, a name that occurs more than once is a `Sequence` in document order |
| Proves | strings; every leaf is text, because that is all an element carries |
| Layout | whitespace-only character data is dropped and a text-only element loses its outer whitespace; a CDATA section keeps its bytes exactly |
| Lacks | document order across differing names, interleaving in mixed content, sequences of sequences, a second root |
| Exact | numbers, booleans, decimals, temporals and binary need a [`Field`](../types/field.md), which types the character data |
| References | character references and the five XML predefines resolve; any other reference is an error naming it, so a document type declaration expands nothing |
| Annotations | comments, processing instructions and the document type declaration are ignored, as a YAML tag is |
| Encoding | UTF-8; a declaration naming another encoding is reported rather than transcoded |
| Limits | byte, depth, decoded-node, document, with `MAX_PARSER_DEPTH` as the ceiling a caller's limit cannot raise |
| Errors | name `xml` and a byte offset; `validate_for_write` rejects before a destination opens |
| `IOBase` | `from_io` / `into_io` infer XML and outer [coding](../coding/index.md) from the media type |
| Rows | a document holding one element per row is [Media](../media/xml.md), which adds position |

## Use

```rust
use yggdryl::Scalar;
use yggdryl::text::xml;

let value = xml::from_utf8("<trade id='7'><symbol>AAPL</symbol></trade>")?;
let trade = value.get_key_str("trade").expect("the document element");

assert_eq!(trade.get_key_str("@id").and_then(Scalar::as_utf8), Some("7"));
assert_eq!(
    trade.get_key_str("symbol").and_then(Scalar::as_utf8),
    Some("AAPL")
);
assert_eq!(
    xml::into_utf8(&value)?,
    r#"<trade id="7"><symbol>AAPL</symbol></trade>"#
);
```

## Inferring entry point

`from_xml_scalar`, `from_xml_scalar_with_field`, and `into_xml_scalar` are XML's
[inferring entry points](index.md), answering the `Record` a document is.

```rust
use yggdryl::{from_xml_scalar, into_xml_scalar};

let value = from_xml_scalar("<rows><row><id>1</id></row></rows>")?;
let encoded = into_xml_scalar(&value)?;

assert_eq!(encoded, "<rows><row><id>1</id></row></rows>");
assert_eq!(from_xml_scalar(encoded.as_bytes())?, value);
```

## A Field types what the element holds

An element carries text, so a declared field is what makes it a number, an
instant, or a decimal. The field types what the document element *holds* rather
than the name the wire gave it, so a row reads as that row whatever the element
is called.

```rust
use yggdryl::{from_xml_scalar_with_field, DataType, Field, Scalar};

let field = Field::new(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        Field::new("amount", DataType::decimal128(10, 2)?, false),
    ])?,
    false,
);
let value = from_xml_scalar_with_field(
    "<trade><id>7</id><amount>12.50</amount></trade>",
    &field,
)?;

assert_eq!(
    value,
    Scalar::from_sequence([Scalar::from(7_i64), Scalar::d128(1250, 2)])
);
```

## Natural values and exact Fields

Unspellable Scalars are rejected before a destination opens, never encoded into a
side format.

| input or native value | natural XML behavior |
| --- | --- |
| element with children or attributes | `Record` keyed by child name, `@name`, and `#text` |
| element with only text | that text |
| empty element | `Null`, written `<name/>` |
| repeated child name | `Sequence` in document order, written as repeated elements |
| `D128`, `D256` | digits at the declared scale |
| bytes / geospatial | standard base64 |
| date, time, datetime, duration | the classic spelling |
| sequence of sequences | error |
| a name XML cannot spell, or `U+FFFE` / `U+FFFF` | error |

## Layout

`Formatting` changes bytes and never meaning. The default writes the document
element and nothing around it: no declaration, no document type, no trailing
newline. An explicit indent lays out nested elements, and an element holding its
own text stays on one line because indenting it would change that text.

```rust
use yggdryl::text::{xml, Formatting};

let value = xml::from_utf8("<row><a>1</a><b><c>2</c></b></row>")?;
let indented = xml::into_utf8_with_formatting(&value, Formatting::indented(2))?;

assert_eq!(indented, "<row>\n  <a>1</a>\n  <b>\n    <c>2</c>\n  </b>\n</row>");
assert_eq!(xml::from_utf8(&indented)?, value);
```
