# XML

An XML document as one [`Scalar`](../types/scalar.md): the record naming its root element, every attribute, element and text node a key of it; the bindings answer native objects unless asked for the scalar.

## Overview

| | |
| --- | --- |
| Declared by | `application/xml`, `.xml` |
| Build | default |
| Rust | `yggdryl::xml`: `from_utf8`, `from_bytes`, `from_reader` and their `_with_field`, `_with_limits` and `_all` forms; `into_utf8`, `into_bytes`, `into_writer` and their `_with_formatting` forms; `from_xml_scalar`, `from_xml_scalar_with_field` and `into_xml_scalar` at the crate root; `ATTRIBUTE_PREFIX` (`@`), `TEXT_KEY` (`#text`) and `DOCUMENT_ELEMENT` (`data`) name the mapping |
| Python | `yggdryl.xml`: `loads`, `dumps`, `dump` |
| JavaScript | `xml`: `loads`, `load`, `dumps`, `dump`, and `loadStream`/`dumpStream` - the one document over a Node stream |
| Handle | a `.xml` handle reads through `read_scalar` or `read_serie` and writes whole through `write_scalar` or `overwrite_serie`, its rows the elements under one `data` element ([Write](#write)) |
| Refused | an entity a document type declaration would have defined, and on write what XML cannot spell |

## Read

A document is the record naming its root element. An attribute is an `@name` entry, an element's own text beside attributes or children is `#text`, child elements sharing a name are a sequence in document order, a self-closed element (`<a/>`) is null where one with a body (`<a></a>`) is the empty text, and every leaf is text - XML proves nothing else, so a declared field is what types a document, and it reads one repeated element as one item of a sequence column and an element occurring no time as the empty sequence. Under a declared field each leaf's text is read by the one reader its datatype owns - a `boolean` by the [boolean table](../types/numeric/boolean.md#the-one-text-reader), a number by the integer or float grammar, a decimal by its [exact grammar](../types/numeric/decimal.md#text), bytes as RFC 4648 base64. Names keep the prefix the document spells; comments, processing instructions and the document type declaration are skipped, and an entity a declaration would have defined is refused by name. `{{ }}` placeholders resolve after parsing, only when asked, by the rule [YAML](yaml.md#read) states.

=== "Rust"

    ```rust
    use yggdryl::xml;
    use yggdryl::{from_xml_scalar, from_xml_scalar_with_field, Field, Scalar};

    let source = "<order id=\"7\"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>";
    let value = xml::from_utf8(source)?;

    // The record naming its root: `@` for an attribute, a sequence for a
    // repeated element, null for a self-closed one, text for every leaf.
    let order = value.get_key_str("order").expect("the root element");
    assert_eq!(order.get_key_str("@id").and_then(Scalar::as_str), Some("7"));
    assert_eq!(
        order.get_key_str("leg").and_then(Scalar::as_sequence).map(<[Scalar]>::len),
        Some(2)
    );
    assert_eq!(order.get_key_str("note"), Some(&Scalar::Null));
    assert_eq!(from_xml_scalar(source.as_bytes())?, value);

    // A field types the root element's value: one repeated element read once
    // is one item of a sequence column, and text is what the column says.
    let field = Field::from_str(
        "order: struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8> not null",
    )?;
    let typed = from_xml_scalar_with_field(source, &field)?;
    assert_eq!(
        typed,
        Scalar::from_sequence([
            Scalar::from(7),
            Scalar::from("AAPL"),
            Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
            Scalar::Null,
        ])
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field, Scalar, xml

    source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
    natural = xml.loads(source)
    assert natural == {
        "order": {"@id": "7", "symbol": "AAPL", "leg": ["1", "2"], "note": None}
    }
    assert xml.loads(source, cls=Scalar).kind == "struct"
    # An element's own text beside attributes is `#text`, and `<b></b>` the empty text.
    assert xml.loads('<a x="1">hi<b></b></a>') == {"a": {"#text": "hi", "@x": "1", "b": ""}}

    # A field types the root element's value; a dataclass `cls` is that field.
    field = Field(
        "order",
        "struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>",
        nullable=False,
    )
    assert xml.loads(source, field=field) == {"@id": 7, "symbol": "AAPL", "leg": [1, 2], "note": None}

    # Placeholders resolve after parsing, and only when asked.
    assert xml.loads('<cfg port="{{ PORT }}"/>', placeholders={"PORT": 8080}) == {"cfg": {"@port": 8080}}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, xml } = require('yggdryl')

    const source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
    assert.deepEqual(xml.loads(source), {
      order: { '@id': '7', symbol: 'AAPL', leg: ['1', '2'], note: null },
    })
    assert.equal(xml.loads(source, { scalar: true }).kind, 'struct')
    // An element's own text beside attributes is `#text`, and `<b></b>` the empty text.
    assert.deepEqual(xml.loads('<a x="1">hi<b></b></a>'), { a: { '#text': 'hi', '@x': '1', b: '' } })

    // A field types the root element's value.
    const field = new Field(
      'order',
      'struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>',
      false,
    )
    assert.deepEqual(xml.loads(source, { field }), { '@id': 7, symbol: 'AAPL', leg: [1, 2], note: null })
    // One repeated element read once is still a one-item array under the field.
    assert.deepEqual(xml.loads('<order id="1"><symbol>X</symbol><leg>5</leg></order>', { field }).leg, [5])

    // Placeholders resolve after parsing, and only when asked.
    assert.deepEqual(xml.loads('<cfg port="{{ PORT }}"/>', { placeholders: { PORT: 8080 } }), {
      cfg: { '@port': 8080 },
    })
    ```

## Write

Writing is the inverse, one line unless indented - a record's keys sorted, a Python `dict` keeping its own order - and refuses what XML cannot spell: a root with several entries, a sequence inside a sequence, a key that is not an XML name.

As a record medium, a `.xml` handle holds one document element, `data`, with one child element per row named after the root field - `<data><row>...</row></data>` - and reads the rows back as the elements of that name, the root itself when it has none, and no row from an empty root. A charset other than UTF-8 is read off the declaration when neither the handle's media type nor a byte order mark states one, and written back as a declaration for the same reason; the codec itself reads UTF-8, like every other. There is no schema-document door (`DataType::from_xml`, `Field::from_xml`): a schema document types its own leaves, which XML text cannot.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::text::Formatting;
    use yggdryl::{
        into_xml_scalar, xml, DataType, IOBase, IOMedia, MimeType, Scalar, Serie, StructType,
    };

    // The record naming its root writes back as that document, its keys sorted,
    // on one line unless indented.
    let value = xml::from_utf8("<order id=\"7\"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>")?;
    assert_eq!(
        into_xml_scalar(&value)?,
        "<order id=\"7\"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>"
    );
    assert_eq!(
        xml::into_utf8_with_formatting(&value, Formatting::indented(2))?,
        "<order id=\"7\">\n  <leg>1</leg>\n  <leg>2</leg>\n  <note/>\n  <symbol>AAPL</symbol>\n</order>"
    );

    // A root with two entries names no one document element.
    let two = Scalar::from_struct([("a", Scalar::from("1")), ("b", Scalar::from("2"))])?;
    let refused = xml::into_utf8(&two).unwrap_err();
    assert!(refused.to_string().contains("naming the document element"), "{refused}");

    // As a record medium, the rows are `<row>` elements under one `<data>` element.
    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let rows = Serie::from_scalars(
        field,
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        ],
    )?;
    let mut handle = Buffer::new().with_media_type(MimeType::XML.into());
    handle.overwrite_serie(rows.into(), None)?;
    assert_eq!(
        String::from_utf8(handle.read_all_bytes()?)?,
        "<data><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id><symbol/></row></data>"
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase, xml

    # The record naming its root writes back as that document, its keys sorted,
    # on one line unless indented.
    natural = xml.loads('<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>')
    assert xml.dumps(natural) == (
        b'<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>'
    )
    assert xml.dumps(natural, indent=2) == (
        b'<order id="7">\n  <leg>1</leg>\n  <leg>2</leg>\n  <note/>\n  <symbol>AAPL</symbol>\n</order>'
    )

    # What XML cannot spell is refused by name.
    with pytest.raises(ValueError, match="naming the document element"):
        xml.dumps({"a": "1", "b": "2"})
    with pytest.raises(ValueError, match="expected an XML name"):
        xml.dumps({"not a name": "x"})

    # As a record medium, the rows are `<row>` elements under one `<data>` element.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.xml")
    handle.overwrite_serie(pa.table({"id": pa.array([1, 2], pa.int64()), "symbol": ["AAPL", None]}))
    assert handle.read_bytes() == (
        b"<data><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id><symbol/></row></data>"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, xml } = require('yggdryl')

    // The record naming its root writes back as that document, its keys sorted,
    // on one line unless indented.
    const natural = xml.loads('<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>')
    assert.equal(
      xml.dumps(natural).toString(),
      '<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>',
    )
    assert.equal(
      xml.dumps(natural, { indent: 2 }).toString(),
      '<order id="7">\n  <leg>1</leg>\n  <leg>2</leg>\n  <note/>\n  <symbol>AAPL</symbol>\n</order>',
    )

    // What XML cannot spell is refused by name.
    assert.throws(() => xml.dumps({ a: '1', b: '2' }), /naming the document element/)
    assert.throws(() => xml.dumps({ 'not a name': 'x' }), /expected an XML name/)

    // A handle holds one document, written whole: rows are `<row>` elements under `<data>`.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.xml'))
    handle.writeScalar({ data: { row: [{ id: '1', symbol: 'AAPL' }, { id: '2', symbol: null }] } })
    assert.equal(
      handle.readBytes().toString(),
      '<data><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id><symbol/></row></data>',
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Performance

### Rust codec

One release run of the `text` Criterion target's `codec/xml` group on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The record is `{symbol: "MSFT", quantity: 120, price: 413.75, tags: ["closing", "auction"]}` under one `row` element; the typed one a `decimal256(76, 4)`, a `datetime64(s, UTC)` and three bytes, read back under its field. The [JSON](json.md#rust-codec), [YAML](yaml.md#rust-codec) and [TOML](toml.md#rust-codec) pages carry the same rows from the same run.

| door | document | bytes | median | throughput |
| --- | --- | ---: | ---: | ---: |
| `into_bytes` | record | 117 | 1.055 us | 105.8 MiB/s |
| `into_utf8` | record | 117 | 1.105 us | 101.0 MiB/s |
| `into_xml_scalar` | record | 117 | 1.155 us | 96.6 MiB/s |
| `into_writer` | record | 117 | 950.4 ns | 117.4 MiB/s |
| `from_bytes` | record | 117 | 2.068 us | 54.0 MiB/s |
| `from_xml_scalar` | record | 117 | 2.025 us | 55.1 MiB/s |
| `from_utf8` | record | 117 | 2.063 us | 54.1 MiB/s |
| `from_reader` | record | 117 | 2.121 us | 52.6 MiB/s |
| `text::from_utf8_inferred`, the format read off the content | record | 117 | 2.046 us | 54.5 MiB/s |
| `into_bytes` | typed | 88 | 1.394 us | 60.2 MiB/s |
| `from_bytes_with_field` | typed | 88 | 3.762 us | 22.3 MiB/s |
| `from_xml_scalar_with_field` | typed | 88 | 3.824 us | 21.9 MiB/s |
| `from_bytes` | 49 levels deep | 348 | 17.57 us | 18.9 MiB/s |
| `from_bytes` | 1,024 elements | 22,281 | 442.6 us | 48.0 MiB/s |
| `from_bytes` | 1,000 `row` elements | 117,013 | 2.166 ms | 51.5 MiB/s |
| `into_bytes` | 1,000 `row` elements | 117,013 | 951.6 us | 117.3 MiB/s |

```bash
cargo bench -p yggdryl --bench text -- codec/xml/
```

### Bindings

The binding rows - the `XML` rows of `python/benchmarks/text.py` and the `xml/*` rows of `node/benchmarks/text.js` - are stated once a release run on the machine the [JSON](json.md#bindings), [YAML](yaml.md#bindings) and [TOML](toml.md#bindings) binding tables name produces them, so the four formats compare.

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```
