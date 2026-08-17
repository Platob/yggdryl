# YAML

`yggdryl::yaml` reads and writes YAML as the shared [`Value`](text.md), one document or a stream of them.

=== "Rust"

    ```rust
    use yggdryl::yaml;

    let value = yaml::from_str("symbol: AAPL\nquantity: 2\n")?;
    assert_eq!(
        value.get_key_str("symbol").and_then(|value| value.as_str()),
        Some("AAPL"),
    );

    let encoded = yaml::to_vec(&value)?;
    assert_eq!(yaml::from_slice(&encoded)?, value);

    // One document means one.
    assert!(yaml::from_str("id: 1\n---\nid: 2\n").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    value = yaml.loads("symbol: AAPL\nquantity: 2\n")
    assert value == {"symbol": "AAPL", "quantity": 2}

    encoded = yaml.dumps(value)
    assert isinstance(encoded, bytes)
    assert yaml.loads(encoded) == value

    # One document means one.
    try:
        yaml.loads("id: 1\n---\nid: 2\n")
    except ValueError as error:
        assert "expected one YAML document" in str(error)
    else:
        raise AssertionError("a second document must be reported")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = yaml.loads('symbol: AAPL\nquantity: 2\n')
    assert.deepEqual(value, { symbol: 'AAPL', quantity: 2 })

    const encoded = yaml.dumps(value)
    assert.deepEqual(yaml.loads(encoded), value)

    // One document means one.
    assert.throws(() => yaml.loads('id: 1\n---\nid: 2\n'), /one YAML document/)
    ```

A single-document call decodes exactly one document and reports a second one at the byte it starts
on, so feeding a stream to it fails instead of silently dropping data. The document-stream calls are
below.

Encoding writes one flow document per value, with every mapping key spelled explicitly:
`{? "symbol" : "AAPL", ? "quantity" : 2}`. Explicit keys are what let a mapping keyed by a sequence
or a boolean survive a round trip, which YAML's plain `key: value` form cannot carry. Reading is not
restricted to what writing produces: block syntax, anchors and aliases, and the core schema's own
spellings all decode.

The bindings are byte-first. `dumps` returns `bytes` in Python and a `Buffer` in JavaScript, never
text, so what you hold is what goes on the wire; decoding returns native objects rather than a
`Value`.

## Documents

=== "Rust"

    ```rust
    use yggdryl::{Value, yaml};

    let documents = yaml::from_str_all("id: 1\n---\nid: 2\n---\nnull\n")?;
    assert_eq!(documents.len(), 3);
    assert_eq!(documents[2], Value::Null);

    let encoded = yaml::to_vec_all(&documents)?;
    assert!(std::str::from_utf8(&encoded)?.contains("\n---\n"));
    assert_eq!(yaml::from_slice_all(&encoded)?, documents);
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    documents = list(yaml.loads_all("id: 1\n---\nid: 2\n---\nnull\n"))
    assert documents == [{"id": 1}, {"id": 2}, None]

    encoded = yaml.dumps_all(documents)
    assert b"\n---\n" in encoded
    assert list(yaml.loads_all(encoded)) == documents
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const documents = yaml.loadsAll('id: 1\n---\nid: 2\n---\nnull\n')
    assert.deepEqual(documents, [{ id: 1 }, { id: 2 }, null])

    const encoded = yaml.dumpAll(documents)
    assert.match(encoded.toString(), /\n---\n/)
    assert.deepEqual(yaml.loadsAll(encoded), documents)
    ```

Writing a stream puts `---` between documents and never before the first, so a one-document stream
is byte-identical to a single-document write.

A null document is still a document. `null`, `~`, and an explicit `---` with nothing after it all
yield a value, at any position in the stream, so document *n* of the input is document *n* of the
output. Anything that dropped them would turn a positional stream into a guess.

These calls hold every document at once. The reader below holds one.

=== "Rust"

    ```rust
    use std::io::Cursor;
    use yggdryl::{Value, yaml};

    let mut reader = yaml::Reader::new(Cursor::new("id: 1\n---\nitems: [1, 2\n"));

    let first = reader.next().expect("a first document")?;
    assert_eq!(first.get_key_str("id"), Some(&Value::U64(1)));
    assert!(reader.byte_offset() >= "id: 1\n".len());

    // The second document is malformed, and the reader is done after saying so.
    assert!(reader.next().expect("a second document").is_err());
    assert!(reader.next().is_none());
    ```

=== "Python"

    ```python
    import io

    from yggdryl import yaml

    documents = yaml.load_all(io.BytesIO(b"id: 1\n---\nitems: [1, 2\n"))
    assert next(documents) == {"id": 1}

    # The second document is malformed, and the iterator is done after saying so.
    try:
        next(documents)
    except ValueError as error:
        assert "at byte 23 (document byte 17)" in str(error)
    else:
        raise AssertionError("the malformed document must be reported")

    assert list(documents) == []
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Readable } = require('node:stream')
    const { yaml } = require('yggdryl')

    async function main() {
      const source = Readable.from([
        Buffer.from('id: 1\n---\n'),
        Buffer.from('items: [1, 2\n'),
      ])
      const documents = []

      // The second document is malformed, and the loop ends after saying so.
      await assert.rejects(async () => {
        for await (const document of yaml.loadAllStream(source)) documents.push(document)
      }, /cumulative byte/)

      assert.deepEqual(documents, [{ id: 1 }])
    }

    main()
    ```

One document is decoded per step, so the first is usable before the rest of the stream has been
read - here, before it is even valid. A failure carries both offsets: the cumulative byte in the
stream and the byte inside the document that failed. After a failure the iterator is finished
rather than resynchronized, so a broken document cannot be stepped over into the next one.

`Reader::new` owns its source and `from_reader_iter` borrows one; both take anything readable.
Python's `load_all` accepts a path or any object with `readline`, and JavaScript's `loadAllStream`
takes a Node stream or an async iterable of chunks and yields documents as they arrive.

Parser bounds come from [`Limits`](text.md), and the `_with_limits` entry points take an explicit
one. Depth and node counts are checked per document, `max_documents` caps how many a stream may
hold, and `max_input_bytes` bounds the whole input. Two ceilings are the parser's own and no caller
limit raises them: `MAX_FLOW_DEPTH` (255) for `[` and `{` flow nesting, and `MAX_PARSER_DEPTH`
(384) for block nesting.

## Tags are read, never written

=== "Rust"

    ```rust
    use yggdryl::{Value, yaml};

    let value = Value::from_mapping([
        (Value::from("payload"), Value::from(vec![0_u8, 255])),
    ])?;

    let encoded = yaml::to_vec(&value)?;
    let text = std::str::from_utf8(&encoded)?;
    assert!(!text.contains("!yggdryl"));
    assert!(text.contains(r#""$yggdryl": "bytes""#));

    assert_eq!(yaml::from_slice(&encoded)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    encoded = yaml.dumps({"payload": b"\x00\xff"})
    assert b"!yggdryl" not in encoded
    assert b'"$yggdryl": "bytes"' in encoded
    assert yaml.loads(encoded) == {"payload": b"\x00\xff"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const encoded = yaml.dumps({ payload: Buffer.from([0, 255]) })
    assert.ok(!encoded.toString().includes('!yggdryl'))
    assert.match(encoded.toString(), /"\$yggdryl": "bytes"/)
    assert.deepEqual(yaml.loads(encoded), { payload: Buffer.from([0, 255]) })
    ```

Nothing on the write path emits a YAML tag. A value with no native YAML spelling becomes an ordinary
flow mapping under the `$yggdryl` marker, flat: marker and payload are two entries of one mapping. A
custom `!yggdryl/*` tag would make the document unreadable to every other YAML implementation, while
the envelope is plain YAML any parser loads, under the same `$yggdryl` marker [JSON](json.md) uses.

The same envelope carries what YAML's core schema has no spelling for: bytes as base64 and 128-bit
integers as decimal strings. A non-finite float needs no envelope, because the core schema spells
it natively as `.inf`, `-.inf`, and `.nan`, and reading resolves those spellings back.

A string is written plain only when the reader resolves that text to the same string. The decision
comes from the reader itself, so a spelling it reads as a number is quoted, `.inf`, `1_000.5`, and
`0x1F` among them, and a string stays a string across a round trip.

=== "Rust"

    ```rust
    use yggdryl::{Value, yaml};

    // A machine tag on input is semantic: it names a kind the value model has.
    assert_eq!(
        yaml::from_str("!yggdryl/bytes AP8=\n")?,
        Value::from(vec![0_u8, 255]),
    );

    // An application tag names nothing the value model has, so it stays the
    // annotation YAML defines it to be and the node under it is the value.
    let value = yaml::from_str("!vendor:quantity {value: 4}\n")?;
    assert_eq!(value.get_key_str("value"), Some(&Value::U64(4)));

    // A comment is not read either.
    assert_eq!(
        yaml::from_str("# vendor:attacker\n!vendor:quantity {value: 4}\n")?,
        value,
    );
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    # A machine tag on input is semantic.
    assert yaml.loads("!yggdryl/bytes AP8=\n") == b"\x00\xff"

    # An application tag is an annotation, so the node under it is the value.
    assert yaml.loads("!vendor:quantity {value: 4}\n") == {"value": 4}

    # A comment is not read either.
    assert yaml.loads("# vendor:attacker\n!vendor:quantity {value: 4}\n") == {
        "value": 4
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    // A machine tag on input is semantic.
    assert.deepEqual(yaml.loads('!yggdryl/bytes AP8=\n'), Buffer.from([0, 255]))

    // An application tag is an annotation, so the node under it is the value.
    assert.deepEqual(yaml.loads('!vendor:quantity {value: 4}\n'), { value: 4 })

    // A comment is not read either.
    assert.deepEqual(
      yaml.loads('# vendor:attacker\n!vendor:quantity {value: 4}\n'),
      { value: 4 },
    )
    ```

Reading accepts more than writing produces, so a document another producer tagged still loads. What
a tag can say is the difference: `!yggdryl/*` names a kind the value model has and selects it, while
every other tag names something no value can hold, so the node decodes as the plain value it
annotates rather than failing a document this codec can otherwise read in full. A comment is display
text either way, and rewriting one changes no decoded value.

No decoded document names a class in any runtime. A caller that wants one names it in the call -
`cls=` in Python - see [Python](../extensions/python.md) and
[JavaScript](../extensions/javascript.md).

=== "Rust"

    ```rust
    use yggdryl::{Value, yaml};

    let collision = Value::from_mapping([
        (Value::from("$yggdryl"), Value::from("bytes")),
        (Value::from("value"), Value::from("AP8=")),
    ])?;

    let encoded = yaml::to_vec(&collision)?;
    assert!(std::str::from_utf8(&encoded)?.contains(r#""$yggdryl": "mapping""#));
    assert_eq!(yaml::from_slice(&encoded)?, collision);
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    collision = {"$yggdryl": "bytes", "value": "AP8="}

    encoded = yaml.dumps(collision)
    assert b'"$yggdryl": "mapping"' in encoded
    assert yaml.loads(encoded) == collision
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const collision = { $yggdryl: 'bytes', value: 'AP8=' }

    const encoded = yaml.dumps(collision)
    assert.match(encoded.toString(), /"\$yggdryl": "mapping"/)
    assert.deepEqual(yaml.loads(encoded), collision)
    ```

An envelope made of ordinary YAML is one application data can spell by accident. A mapping that
would read back as an envelope is wrapped once more, as
`{"$yggdryl": "mapping", "value": [[key, value], ...]}`, so it decodes as the plain mapping it was
and is never promoted to bytes or to any other kind.

The same [`Value`](text.md) is what [JSON](json.md) and [TOML](toml.md) read and write, and
`yggdryl::text` picks the format at run time when the caller does not know it in advance.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](../notebooks/core_yaml-rust.ipynb){ download },
[Python](../notebooks/core_yaml-python.ipynb){ download },
[JavaScript](../notebooks/core_yaml-javascript.ipynb){ download }.

<!-- /notebooks -->
