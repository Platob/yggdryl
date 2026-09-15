# XML and Arrow

The record surface: a handle whose media type says XML answers [`IOMedia`](../../holder/iobase/records.md) like any other encoding, and the name is all it takes.

## Read rows

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia, Url};
    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///trades.xml")?.media_type());
    handle.write_all_bytes(
        b"<rows><Trade><id>1</id></Trade><Trade><id>2</id></Trade></rows>",
    )?;

    // The name picked the encoding; nothing else in the call changes.
    let options = handle.record_options()?;
    let rows: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()).unwrap_or(0))
        .sum();
    assert_eq!(rows, 2);
    ```

Rows are the document element's element children, and they must agree on one name. A document whose children disagree is a shape this cannot publish one field for, and it says so rather than picking the first.

## A wrapper that holds more than rows

`row_element` names the row, which reads a feed, a SOAP body, or an XMLA response whose wrapper carries metadata beside the rows. It is looked for wherever it sits.

It is a read setting, so it travels in the options a read takes. [`IOMedia::row_size`](../../holder/iobase/records.md) describes the whole media value and takes none, so it answers under the handle's own retained options - set them through `Xml::options_mut` when a count has to agree with a named-row read.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia, Url};
    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///feed.xml")?.media_type());
    handle.write_all_bytes(
        b"<rss><channel><title>Example</title>\
          <item><guid>1</guid></item><item><guid>2</guid></item></channel></rss>",
    )?;

    let mut options = handle.record_options()?;
    if let RecordOptions::Xml(xml) = &mut options {
        xml.row_element = Some("item".into());
    }
    let rows: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()).unwrap_or(0))
        .sum();
    assert_eq!(rows, 2);
    ```

## Structure is inferred; type never is

Every leaf on the wire is text. A read given no field infers the shape a document proves and types every leaf `utf8`. A read given a [`Field`](../../types/field.md) crosses each leaf through that field's own value contract - the same contract every other codec uses - so `9.50` becomes a decimal at the scale the column declares.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Url};
    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("Trade");

    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///typed.xml")?.media_type());
    handle.write_all_bytes(b"<rows><Trade id='7'/></rows>")?;

    let options = handle.record_options()?.with_field(schema);
    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    ```

## A schema is where the types come from

An XML Schema answers a `Field`, and the declared-field path above does the rest. There is no schema-shaped option and no second reader.

=== "Rust"

    ```rust
    use yggdryl::media::xml;
    use yggdryl::{DataType, Limits};
    let schema = br#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
      <xsd:element name="row"><xsd:complexType><xsd:sequence>
        <xsd:element name="size" type="xsd:unsignedInt"/>
      </xsd:sequence></xsd:complexType></xsd:element>
    </xsd:schema>"#;

    let field = xml::field_from_xsd(schema, Limits::default(), None)?;
    let children = field.dtype().as_fields().unwrap_or_default();
    assert_eq!(children[0].dtype(), &DataType::UInt32);
    ```

This is exactly how an XMLA rowset is read: the response carries its own `xsd:schema` inline, so the schema and the rows arrive in one document.

### What a schema maps, and what it keeps

The eight bounded integers, the two floats, the binaries, the boolean and a decimal carrying **both** digit facets map onto a datatype exactly. The rest travel as text, with `xml:type` recording what the schema called them - because widening them would invent a fact:

| Declared | Becomes | Because |
| --- | --- | --- |
| `xs:long` … `xs:unsignedByte` | `Int64` … `UInt8` | The eight bounded integers are the only XSD integers with a width |
| `xs:decimal` with `totalDigits` **and** `fractionDigits` | `decimal(p, s)` | Only both facets name a precision |
| `xs:decimal` bare, `xs:integer` | `utf8` **+ `xml:type`** | Unbounded in magnitude *and* scale; no width at all |
| `xs:dateTimeStamp`, or `explicitTimezone` stated | `datetime64` | An offset that is settled is an instant |
| `xs:dateTime` with the offset optional | `utf8` **+ `xml:type`** | One column with an offset and one without are two columns |
| `xs:gYear`, `xs:gMonth`, … | `utf8` **+ `xml:type`** | A partial calendar value is not an instant |
| `xs:anyURI`, `xs:QName` | `utf8` **+ `xml:type`** | Any string in XSD 1.1; a QName means what its bindings say |
| `xs:error`, `xs:NOTATION` | **refused** | One has no valid instance; the other means only what its own schema means |

`minOccurs="0"` and `nillable` make a column nullable, `maxOccurs` greater than one makes it a list, and `use="required"` on an attribute makes it a column that is always there.

## A document remembers how it was spelled

A read records which columns were attributes and which namespaces they were bound in, on the field it answers, under the [`xml:` vocabulary](../../types/protocol.md). A write reads it back, so a document read under a field and written back under the same field comes out the way it went in.

| Property | Says |
| --- | --- |
| `xml:kind` | `element`, `attribute`, or the element's own `text` |
| `xml:namespace` | the URI the name was bound in |
| `xml:name` | the wire spelling, when it is not the column's name |
| `xml:type` | what a schema called a column that travels as text |

## Writing

A write emits one spelling for an undeclared column - a child element - and the declared spelling for a column whose field carries one. `null` is absence: the attribute or element is simply not emitted.

`field_into_xsd` is the reverse of `field_from_xsd`, and a field read from a schema, written as a schema, and read again is the same field.

## What a read costs

A document states no schema and carries no index: there is no header to read a field out of and no footer to count rows in. So `read_arrow_field`, `row_size` and `column_size` all read the document. That is a property of the encoding, not a shortcut taken here.

XML does not compress internally, so - unlike Avro and Parquet - a handle declaring an outer content coding reads and writes through that coding rather than refusing it.

Numbers, and the command that regenerates them, are on the [index page](index.md#performance).
