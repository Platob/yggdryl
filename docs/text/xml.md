# XML

One root element as a natural record document, backed by the shared Rust codec.

## Contract

| | |
| --- | --- |
| Root | one element, so a document is a one-entry `Record` keyed by that element's qualified name |
| Proves | character data and nothing else; XML has no number, boolean, or date grammar to prove one with |
| Keys | a bare name is a child element, `@name` an attribute, `#text` an element's own character data |
| Lacks | one spelling apart for the empty string, a list frame, mixed content, a second root, namespace resolution |
| Exact | every leaf needs a [`Field`](../types/field.md), which also reads the shapes a document cannot spell |
| Selector | Python `cls=Scalar`, JavaScript `{ scalar: true }`; omitted returns natural mappings |
| Limits | byte, depth, decoded-node, document; the parser ceiling `MAX_PARSER_DEPTH` is 384 whatever a caller asks; nullable binding options, snake_case in Python and camelCase in JavaScript |
| Errors | name `xml` and a byte offset; `validate_for_write` rejects before a destination opens |
| `IOBase` | `from_io` / `into_io` infer XML and outer [coding](../coding/index.md) from the media type; rows frame as `<records><row/></records>` |

## Use

Rust returns `Scalar`; bindings redirect native mappings through the same codec.

=== "Rust"

    ```rust
    use yggdryl::Scalar;
    use yggdryl::text::xml;

    let value = xml::from_utf8("<trade><symbol>AAPL</symbol><size>100</size></trade>")?;
    let trade = value.get_key_str("trade").expect("the root element");

    assert_eq!(trade.get_key_str("symbol").and_then(Scalar::as_utf8), Some("AAPL"));
    assert_eq!(
        xml::into_utf8(&value)?,
        "<trade><size>100</size><symbol>AAPL</symbol></trade>"
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import xml

    source = "<trade><symbol>AAPL</symbol><size>100</size></trade>"
    natural = xml.loads(source)
    value = xml.loads(source, cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {"trade": {"size": "100", "symbol": "AAPL"}}
    assert xml.dumps(value) == b"<trade><size>100</size><symbol>AAPL</symbol></trade>"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, xml } = require('yggdryl')

    const source = '<trade><symbol>AAPL</symbol><size>100</size></trade>'
    const natural = xml.loads(source)
    const value = xml.loads(source, { scalar: true })
    const encoded = xml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(xml.loads(encoded), natural)
    ```

A `Record` is keyed by name and sorted by it, so written elements and attributes come out in name order rather than in the order a source document listed them.

## Inferring entry point

`from_xml_scalar`, `from_xml_scalar_with_field`, and `into_xml_scalar` are XML's [inferring entry points](index.md), answering the one-entry `Record` a document is.

=== "Rust"

    ```rust
    use yggdryl::{from_xml_scalar, into_xml_scalar, Scalar};

    let value = from_xml_scalar("<row><id>1</id></row>")?;
    let encoded = into_xml_scalar(&value)?;

    assert_eq!(
        value,
        Scalar::from_record([("row", Scalar::from_record([("id", Scalar::from("1"))])?)])?
    );
    assert_eq!(encoded, "<row><id>1</id></row>");
    assert_eq!(from_xml_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import xml

    value = xml.loads("<row><id>1</id></row>", cls=Scalar)
    encoded = xml.dumps(value)

    assert encoded == b"<row><id>1</id></row>"
    assert xml.loads(encoded, cls=Scalar) == value
    assert value.as_py()["row"]["id"] == "1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const value = xml.loads('<row><id>1</id></row>', { scalar: true })
    const encoded = xml.dumps(value)

    assert.equal(encoded.toString(), '<row><id>1</id></row>')
    assert.ok(xml.loads(encoded, { scalar: true }).equals(value))
    assert.equal(value.asJs().row.id, '1')
    ```

## The shape a document has

| XML | `Scalar` |
| --- | --- |
| `<a>text</a>` | the text, under `a` |
| `<a/>`, `<a></a>` | `Null`, under `a`, until a field that refuses absence reads the empty value |
| `<a id="1">text</a>` | a record of `@id` and `#text` |
| `<a><b/><c/></a>` | a record of `b` and `c` |
| `<a><b/><b/></a>` | a sequence of two, under `b` |
| `<a>text<b/></a>` | refused: mixed content orders nothing |

An attribute keys behind `@` and an element's own character data keys `#text`. No XML name may start with either character, so neither can collide with a child element of the same name. Repeated children of one name become a sequence in document order, and a sequence of sequences has no spelling at all.

`<a/>` and `<a></a>` are one document, so an element with no character data is absence, and a schema-free read of an empty string answers `Null`. Whitespace between child elements lays a document out; whitespace inside a leaf is that leaf's value, with its line endings normalized the way every XML reader normalizes them.

=== "Rust"

    ```rust
    use yggdryl::{from_xml_scalar, into_xml_scalar, Scalar};

    let value = from_xml_scalar(r#"<row id="7"><tag>a</tag><tag>b</tag><none/></row>"#)?;
    let row = value.get_key_str("row").expect("the root element");

    assert_eq!(row.get_key_str("@id"), Some(&Scalar::from("7")));
    assert_eq!(
        row.get_key_str("tag"),
        Some(&Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")]))
    );
    assert_eq!(row.get_key_str("none"), Some(&Scalar::Null));
    assert_eq!(
        into_xml_scalar(&value)?,
        r#"<row id="7"><none/><tag>a</tag><tag>b</tag></row>"#
    );

    // Mixed content is refused by the reader and by the writer alike.
    assert!(from_xml_scalar("<row>text<id>1</id></row>").is_err());
    ```

=== "Python"

    ```python
    from yggdryl.text import xml

    decoded = xml.loads(b'<row id="7"><tag>a</tag><tag>b</tag><none/></row>')

    assert decoded == {"row": {"@id": "7", "none": None, "tag": ["a", "b"]}}
    assert xml.dumps(decoded) == b'<row id="7"><none/><tag>a</tag><tag>b</tag></row>'
    assert xml.loads("<row/>") == xml.loads("<row></row>") == {"row": None}
    assert xml.dumps({"row": ""}) == b"<row></row>"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const decoded = xml.loads('<row id="7"><tag>a</tag><tag>b</tag><none/></row>')

    assert.deepEqual(decoded, { row: { '@id': '7', none: null, tag: ['a', 'b'] } })
    assert.equal(
      xml.dumps(decoded).toString(),
      '<row id="7"><none/><tag>a</tag><tag>b</tag></row>',
    )
    assert.throws(() => xml.loads('<row>text<id>1</id></row>'))
    ```

## Entities, annotations, and namespaces

Only the five predefined entities - `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` - and character references resolve. No document-declared entity is ever expanded, so a `<!ENTITY>` declaration buys a reference nothing: the refusal names the entity instead. Comments, processing instructions, and the DOCTYPE are ignored. A `CDATA` section is content and is written back escaped, because one section and one escaped spelling are one document.

No XML declaration is emitted. One is accepted on read, and it must not disagree with UTF-8 or name a version other than 1.0 or 1.1.

A prefixed name - `ns:total`, `xmlns:ns` - is kept exactly as written, so namespaces survive a round trip without this layer resolving any of them.

=== "Rust"

    ```rust
    use yggdryl::{from_xml_scalar, into_xml_scalar, Scalar};

    let annotated = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                     <!-- a note --><!DOCTYPE row --><?render fast?><row><id>1</id></row>";
    assert_eq!(
        from_xml_scalar(annotated)?,
        from_xml_scalar("<row><id>1</id></row>")?
    );

    let document = r#"<ns:trade xmlns:ns="urn:example"><ns:id>1</ns:id></ns:trade>"#;
    let value = from_xml_scalar(document)?;
    assert_eq!(into_xml_scalar(&value)?, document);

    assert_eq!(
        from_xml_scalar("<row>a &amp; b &#65;</row>")?
            .get_key_str("row")
            .and_then(Scalar::as_utf8),
        Some("a & b A")
    );
    // A declared entity is never expanded, and the refusal names it.
    let refused = from_xml_scalar("<!DOCTYPE d [<!ENTITY lol \"lol\">]><d>&lol;</d>")
        .expect_err("no declared entity is reachable");
    assert!(refused.to_string().contains("lol"), "{refused}");
    ```

=== "Python"

    ```python
    from yggdryl.text import xml

    assert xml.loads('<?xml version="1.0"?><!-- note --><row><id>1</id></row>') == {
        "row": {"id": "1"}
    }
    assert xml.loads("<row>a &amp; b &#65;</row>") == {"row": "a & b A"}
    assert xml.loads("<row><![CDATA[a <b> & c]]></row>") == {"row": "a <b> & c"}

    document = '<ns:trade xmlns:ns="urn:example"><ns:id>1</ns:id></ns:trade>'
    assert xml.dumps(xml.loads(document)) == document.encode()

    try:
        xml.loads('<!DOCTYPE d [<!ENTITY lol "lol">]><d>&lol;</d>')
    except ValueError as error:
        assert "lol" in str(error)
    else:
        raise AssertionError("no declared entity is reachable")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    assert.deepEqual(xml.loads('<?xml version="1.0"?><!-- note --><row><id>1</id></row>'), {
      row: { id: '1' },
    })
    assert.deepEqual(xml.loads('<row>a &amp; b &#65;</row>'), { row: 'a & b A' })
    assert.deepEqual(xml.loads('<row><![CDATA[a <b> & c]]></row>'), { row: 'a <b> & c' })

    const document = '<ns:trade xmlns:ns="urn:example"><ns:id>1</ns:id></ns:trade>'
    assert.equal(xml.dumps(xml.loads(document)).toString(), document)
    assert.throws(() => xml.loads('<!DOCTYPE d [<!ENTITY lol "lol">]><d>&lol;</d>'))
    ```

## Natural values and exact Fields

Every leaf is character data, so a schema-free read answers text and nothing else. A `Field` names the root element, types every leaf, and orders the record into a row; it reads the same spellings the other structured formats write.

| native value | natural XML |
| --- | --- |
| `D128`, `D256` | scale-preserving text |
| bytes / geospatial | base64 text |
| date, time, datetime | ISO text |
| duration | ISO duration text |
| interval | one element per part, each a count |
| null | an empty element |
| a name XML cannot spell | error |
| character data XML cannot spell | `&#9;`, `&#10;`, `&#13;`; any other control character is an error |

=== "Rust"

    ```rust
    use yggdryl::{Field, Scalar, from_xml_scalar};
    use yggdryl::text::xml;

    let document = "<row><size>100</size><price>12.50</price></row>";
    let natural = from_xml_scalar(document)?;
    let row = natural.get_key_str("row").expect("the root element");
    assert_eq!(row.get_key_str("size"), Some(&Scalar::from("100")));

    let field = Field::from_str(
        "row: struct<size: int64 not null, price: decimal128(18, 2) not null> not null",
    )?;
    let typed = xml::from_utf8_with_field(document, &field)?;

    assert_eq!(
        typed,
        Scalar::from_sequence([Scalar::from(100_i64), Scalar::d128(1_250, 2)])
    );
    ```

=== "Python"

    ```python
    import datetime as dt
    from decimal import Decimal

    from yggdryl import Field
    from yggdryl.text import xml

    encoded = xml.dumps(
        {"row": {"price": Decimal("12.50"), "day": dt.date(2026, 8, 15), "payload": b"\x00\xff"}}
    )
    assert encoded == b"<row><price>12.50</price><day>2026-08-15</day><payload>AP8=</payload></row>"

    field = Field(
        "row",
        "struct<day: date32 not null, payload: binary not null,"
        " price: decimal128(18, 2) not null>",
        nullable=False,
    )
    assert xml.loads(encoded, field=field) == {
        "day": dt.date(2026, 8, 15),
        "payload": b"\x00\xff",
        "price": Decimal("12.50"),
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, xml } = require('yggdryl')

    const row = fields.struct(
      'row',
      [fields.decimal128('price', 18, 2, { nullable: false })],
      { nullable: false },
    )
    const decoded = xml.loads('<row><price>12.50</price></row>', { field: row })

    assert.equal(decoded.price.kind, 'd128')
    assert.equal(decoded.price.unscaled, 1250n)
    assert.deepEqual(xml.loads('<row><price>12.50</price></row>'), {
      row: { price: '12.50' },
    })
    ```

### The readings a Field adds

XML states less about shape than the other structured formats do, and a declared `Field` is the only thing that can say what the missing part was.

| Document | Field | Reading |
| --- | --- | --- |
| one root element | the root itself | the root's content is the value, whatever the element is named |
| one `<tag>` occurrence | a list | a list holding that one value |
| no occurrence at all | a non-null list | the empty list |
| `<tag/>`, no character data | a non-null variable-width text or byte field | the empty value |
| the parts of an interval | an interval | its counts, read as integers |
| a `[type, value]` pair | a union | the type id, read as an integer |

The empty element is there because XML spells absence and the empty string the same way, so only a field that refuses absence settles which one was written, and only where the datatype has a value of no characters at all - `utf8`, `ascii`, `binary` and their wide forms, never a fixed width, a coded vocabulary, or a number. An element the document leaves out entirely is still absence, and the value contract still refuses it. The interval and the union type id are there because they are the values the contract reads no text for: an interval's parts are counts of months, days, and nanoseconds, and a union's type id names a declared branch. Every other reading - a string that is a number, a record ordered into a row, an absent nullable element - belongs to the value contract a `Field` already owns.

=== "Rust"

    ```rust
    use yggdryl::{Field, Scalar, from_xml_scalar_with_field};

    let field = Field::from_str(
        "row: struct<id: int64 not null, tags: list<utf8 not null> not null, note: utf8> not null",
    )?;

    let one = from_xml_scalar_with_field("<row><id>1</id><tags>a</tags></row>", &field)?;
    let none = from_xml_scalar_with_field("<row><id>1</id></row>", &field)?;
    let foreign = from_xml_scalar_with_field("<trade><id>1</id></trade>", &field)?;

    assert_eq!(
        one.as_sequence().expect("a row")[1],
        Scalar::from_sequence([Scalar::from("a")])
    );
    assert_eq!(one.as_sequence().expect("a row")[2], Scalar::Null);
    assert_eq!(none.as_sequence().expect("a row")[1], Scalar::from_sequence([]));
    assert_eq!(foreign.as_sequence().expect("a row")[0], Scalar::from(1_i64));

    // An empty element is absence until the field refuses absence; an element
    // the document leaves out entirely stays absence and is refused.
    let required = Field::from_str("row: struct<note: utf8 not null> not null")?;
    assert_eq!(
        from_xml_scalar_with_field("<row><note/></row>", &required)?,
        Scalar::from_sequence([Scalar::from("")])
    );
    assert!(from_xml_scalar_with_field("<row/>", &required).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.text import xml

    field = Field(
        "row",
        "struct<id: int64 not null, tags: list<utf8 not null> not null, note: utf8>",
        nullable=False,
    )

    assert xml.loads("<row><id>1</id><tags>a</tags></row>", field=field) == {
        "id": 1,
        "tags": ["a"],
        "note": None,
    }
    assert xml.loads("<row><id>1</id></row>", field=field)["tags"] == []
    assert xml.loads("<trade><id>1</id></trade>", field=field)["id"] == 1

    span = Field("row", "struct<span: interval(month_day_nano) not null>", nullable=False)
    assert xml.loads(
        "<row><span>1</span><span>2</span><span>3</span></row>", field=span
    ) == {"span": [1, 2, 3]}

    required = Field("row", "struct<note: utf8 not null>", nullable=False)
    nullable = Field("row", "struct<note: utf8>", nullable=False)
    assert xml.loads("<row><note/></row>", field=required) == {"note": ""}
    assert xml.loads("<row><note/></row>", field=nullable) == {"note": None}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, xml } = require('yggdryl')

    const field = new Field(
      'row',
      'struct<id: int64 not null, tags: list<utf8 not null> not null, note: utf8>',
      false,
    )

    assert.deepEqual(xml.loads('<row><id>1</id><tags>a</tags></row>', { field }), {
      id: 1,
      tags: ['a'],
      note: null,
    })
    assert.deepEqual(xml.loads('<row><id>1</id></row>', { field }).tags, [])
    assert.equal(xml.loads('<trade><id>1</id></trade>', { field }).id, 1)

    const required = new Field('row', 'struct<note: utf8 not null>', false)
    const nullable = new Field('row', 'struct<note: utf8>', false)
    assert.deepEqual(xml.loads('<row><note/></row>', { field: required }), { note: '' })
    assert.deepEqual(xml.loads('<row><note/></row>', { field: nullable }), { note: null })
    ```

## Documents and streams

A document is one root element, so there is exactly one value in and one value out. Rust `from_utf8`, `from_bytes`, `from_reader` decode it; `into_utf8`, `into_bytes`, `into_writer` encode it. Binding `loads` takes content, paths, descriptors, file URLs, and readers; `dump` returns bytes or text or writes directly.

=== "Rust"

    ```rust
    use yggdryl::text::xml;

    let value = xml::from_utf8("<row><id>1</id></row>")?;
    let mut destination = Vec::new();
    xml::into_writer(&value, &mut destination)?;

    assert_eq!(xml::from_bytes(&destination)?, value);
    assert_eq!(xml::from_reader(destination.as_slice())?, value);
    assert!(xml::into_utf8_all(&[value.clone(), value]).is_err());
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import xml

    destination = io.BytesIO()
    xml.dump({"row": {"id": "1"}}, destination)

    assert destination.getvalue() == b"<row><id>1</id></row>"
    assert xml.loads(destination.getvalue()) == {"row": {"id": "1"}}
    assert xml.dump({"row": None}, utf8=True) == "<row/>"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { xml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-xml-'))
    const target = path.join(root, 'value.xml')
    xml.dump({ row: { id: '1' } }, target)

    assert.deepEqual(xml.load(pathToFileURL(target)), { row: { id: '1' } })
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Records through a handle

A handle whose media type names XML reads and writes rows as well as values. A record write frames the rows as repeated elements named by the root `Field`, inside a document element named `media::XML_DOCUMENT_NAME` - `records`. A read takes the rows from whatever element the document names, so `<trades><row/>...</trades>` written elsewhere reads the same way, and a document element with no children at all holds no rows.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{ArrowShape, ArrowValue, DataType, IOBase, IOMedia, IOMode, Scalar, Url};

    let root = DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])?
    .required_field("row");
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("AAPL"),
        Scalar::from(100_i64),
    ])]);

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.xml")?.media_type());
    handle.write_arrow_value(ArrowValue::from_rows(&root, &rows)?, IOMode::Overwrite)?;

    assert_eq!(
        String::from_utf8(handle.read_all_bytes()?)?,
        "<records><row><size>100</size><symbol>AAPL</symbol></row></records>"
    );

    let read = handle.read_arrow_value(Some(&root))?;
    assert_eq!(read.shape(), ArrowShape::Batch);
    assert_eq!(read.into_scalar()?, rows);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    from yggdryl import Field, IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "quotes.xml")
    handle.write_arrow_value(pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]}))

    assert handle.read_bytes().startswith(b"<records><row>")
    assert handle.read_arrow_value().row_size == 2

    # Rows come from whatever element the document names.
    foreign = root / "trades.xml"
    foreign.write_bytes(
        b"<trades><row><symbol>AAPL</symbol><size>100</size></row></trades>"
    )
    field = Field("row", "struct<symbol: utf8 not null, size: int64 not null>", nullable=False)
    assert IOBase(foreign).read_arrow_value(field).into_arrow_table().to_pylist() == [
        {"symbol": "AAPL", "size": 100}
    ]
    ```

`read_arrow_value` and `write_arrow_value` are the row pair, and they are Rust and Python only; JavaScript reaches XML documents as values, not as rows. Whole values take the same route in all three: `read_scalar` and `write_scalar` on a handle named `trade.xml.gz` parse and render XML through gzip with no argument.

## Formatting

`Indent::Spaces(n)` and `Indent::Tabs` lay children out one per line; the default is one flat line, and `Indent::None` asks for the same. A leaf's own character data is never laid out, because the layout would become part of the value.

=== "Rust"

    ```rust
    use yggdryl::text::{Formatting, Indent, xml};
    use yggdryl::Scalar;

    let value = xml::from_utf8("<row><a>x</a><b><c>y</c></b></row>")?;
    let indented =
        xml::into_utf8_with_formatting(&value, Formatting::default().with_indent(Indent::Spaces(2)))?;
    let leaf = xml::into_utf8_with_formatting(
        &Scalar::from_record([("row", Scalar::from("x"))])?,
        Formatting::default().with_indent(Indent::Spaces(4)),
    )?;

    assert_eq!(indented, "<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>");
    assert_eq!(xml::from_utf8(&indented)?, value);
    assert_eq!(leaf, "<row>x</row>");
    ```

=== "Python"

    ```python
    from yggdryl.text import xml

    value = {"row": {"a": "x", "b": {"c": "y"}}}
    compact = xml.dumps(value)
    indented = xml.dumps(value, indent=2)

    assert compact == b"<row><a>x</a><b><c>y</c></b></row>"
    assert indented == b"<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>"
    assert xml.dumps(value, indent="\t").startswith(b"<row>\n\t<a>x</a>")
    assert xml.loads(indented) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const value = { row: { a: 'x', b: { c: 'y' } } }
    const compact = xml.dumps(value)
    const indented = xml.dumps(value, { indent: 2 })

    assert.equal(compact.toString(), '<row><a>x</a><b><c>y</c></b></row>')
    assert.equal(
      indented.toString(),
      '<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>',
    )
    assert.deepEqual(xml.loads(indented), value)
    ```

## Limits

`Limits` bounds input bytes, nesting depth, decoded nodes, and document count. The parser's own ceiling `MAX_PARSER_DEPTH` is 384, so an adversarial caller limit cannot turn nesting into stack exhaustion.

=== "Rust"

    ```rust
    use yggdryl::text::{Limits, xml};

    let deep = format!("{}{}", "<a>".repeat(40), "</a>".repeat(40));

    assert!(xml::from_utf8_with_limits(&deep, Limits::new(8, 1 << 20, 1 << 20, 1)).is_err());
    assert!(xml::from_utf8_with_limits("<row/>", Limits::new(64, 3, 1 << 20, 1)).is_err());
    assert!(
        xml::from_utf8_with_limits("<row><a/><b/><c/></row>", Limits::new(64, 1 << 20, 2, 1))
            .is_err()
    );
    assert_eq!(yggdryl::text::xml::MAX_PARSER_DEPTH, 384);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl.text import xml

    deep = "<a>" * 40 + "</a>" * 40

    with pytest.raises(ValueError, match="nesting depth limit exceeded"):
        xml.loads(deep, max_depth=8)
    with pytest.raises(ValueError, match="input byte limit exceeded"):
        xml.loads("<row/>", max_input_bytes=3)
    with pytest.raises(ValueError, match="node limit exceeded"):
        xml.loads("<row><a/><b/><c/></row>", max_nodes=2)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const deep = '<a>'.repeat(40) + '</a>'.repeat(40)

    assert.throws(() => xml.loads(deep, { maxDepth: 8 }), /nesting depth limit exceeded/)
    assert.throws(() => xml.loads('<row/>', { maxInputBytes: 3 }), /input byte limit exceeded/)
    assert.throws(
      () => xml.loads('<row><a/><b/><c/></row>', { maxNodes: 2 }),
      /node limit exceeded/,
    )
    ```

## Placeholders

Opt-in, inside character data and attribute values, substituted after parsing and before Field interpretation; see [Placeholders](placeholders.md).

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new()
        .with_placeholders(Placeholders::new().with_variable("SYMBOL", Scalar::from("AAPL")));
    let value = yggdryl::text::from_utf8_with(
        "<row><symbol>{{ SYMBOL }}</symbol></row>",
        Format::Xml,
        &loading,
    )?;

    assert_eq!(
        value
            .get_key_str("row")
            .and_then(|row| row.get_key_str("symbol")),
        Some(&Scalar::from("AAPL"))
    );
    ```

=== "Python"

    ```python
    from yggdryl.text import xml

    decoded = xml.loads(
        "<row><symbol>{{ SYMBOL }}</symbol></row>",
        placeholders={"SYMBOL": "AAPL"},
    )

    assert decoded == {"row": {"symbol": "AAPL"}}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xml } = require('yggdryl')

    const decoded = xml.loads('<row><symbol>{{ SYMBOL }}</symbol></row>', {
      placeholders: { SYMBOL: 'AAPL' },
    })

    assert.deepEqual(decoded, { row: { symbol: 'AAPL' } })
    ```

## Edges

- content that opens a tag -> inferred as XML before JSON, TOML, and YAML; `<<: *anchor` is still YAML.
- no root element, a second root, or text outside the root -> error naming the byte it stopped at.
- character data beside child elements -> refused by the reader and by the writer alike, at the byte the element starts.
- a document-declared entity, an external reference, an unknown entity -> refused by name; nothing is expanded.
- a byte order mark, whitespace around the root, an external DOCTYPE -> not content.
- a repeated attribute on one element -> refused by name rather than resolved to one of the two.
- `\r\n` and `\r` in character data or an attribute value -> normalized to `\n`, and a literal line break in an attribute value to a space, so a carriage return survives only as the `&#13;` the writer spells it with.
- `<tag/>` under a non-null variable-width text or byte field -> the empty value; under a nullable one, absence; under a fixed width, a coded vocabulary, or a number -> absence, refused by the value contract.
- an element the document leaves out entirely -> absence whatever the datatype, refused where the field refuses it.
- an XML declaration naming an encoding other than UTF-8, or a version other than 1.0 or 1.1 -> refused.
- an attribute holding a record or a sequence -> error; an attribute holds one text value.
- a sequence inside a sequence -> error; XML repeats an element instead of framing a list.
- an empty sequence -> no element at all, so `<row/>`.
- a name XML cannot spell - empty, leading digit, containing a space or `>` -> error naming it; `ns:total`, `_private`, and `a-b.c` are names.
- a control character with no XML 1.0 spelling -> error naming it as `U+0001`; tab, newline, and carriage return are written as references so they survive a read.
- `_all` forms and `into_writer_all` -> exactly one value; more than one is refused, and bindings expose no `loadsAll` or `dumpAll`.
- a declared name the document does not carry -> the value contract refuses it by name.
- Arrow batches -> one document element around repeated row elements; `write_arrow_value` takes only `IOMode::Overwrite`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text xml::
    cargo test --features "parquet iceberg" -p yggdryl --lib media::structured::tests
    python scripts/check_xml_interop.py
    cargo bench -p yggdryl --bench text -- codec/xml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/xml
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/xml.test.js
    npm run --prefix node bench:text
    ```
