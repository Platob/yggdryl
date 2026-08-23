# JSON

JSON uses ordinary interoperable documents backed by the shared Rust codec.

## Text media and Arrow batches

A JSON document is a raw structured value, not the line-record media surface.
For streamed Arrow batches with explicit overwrite and append behavior, use
[Text media](text.md#text-media-and-arrow-batches). Text deliberately refuses
keyed merge because a line has no stable row identity. JSON Lines is the
multi-value document stream described below.

## Raw shared-Value access

Rust returns `Value`; Python and JavaScript project the same tree into native
objects. Dumps produce JSON accepted by other implementations, and loads return
only types the JSON grammar proves.

=== "Rust"

    ```rust
    use yggdryl::{json, Value};

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Value::as_utf8),
        Some("AAPL")
    );
    assert_eq!(
        json::into_utf8(&value)?,
        r#"{"quantity":100,"symbol":"AAPL"}"#
    );
    ```

=== "Python"

    ```python
    from yggdryl import Value, json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Value)

    assert value.kind == "record"
    assert value.into_python() == natural == {"quantity": 100, "symbol": "AAPL"}
    assert json.dumps(value) == b'{"quantity":100,"symbol":"AAPL"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Value, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { value: true })
    const encoded = json.dumps(value)

    assert.ok(value instanceof Value)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(json.loads(encoded), natural)
    ```

Objects become name-sorted `Record` values, arrays become `Sequence`, and
numbers use the narrowest exact natural family available to the parser.
Duplicate object names are rejected. Python `cls=Value` and JavaScript
`{ value: true }` return that exact native tree directly; omitting the selector
keeps the existing natural Python/JavaScript result.

## Natural values and exact Fields

JSON has syntax for null, booleans, finite numbers, strings, arrays, and
string-key objects. Other native values use interoperable scalar spellings:

| native value | natural JSON |
| --- | --- |
| `D128`, `D256` | scale-preserving string |
| `Bytes`, `Geospatial` | base64 string |
| date, time, `DateTime64`, duration | ISO string when representable |
| non-finite float | error |
| Mapping with non-string keys | error |

There is no private `$yggdryl` envelope. Consequently a schemaless reader sees
`"12.50"`, `"AP8="`, and `"2026-08-15T10:30:00Z"` as strings. Pass a native
[`Field`](field.md) when those spellings must recover exact types:

=== "Rust"

    ```rust
    use yggdryl::{json, DataType, Field, Value};

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = json::from_utf8_with_field(r#""12.50""#, &amount)?;

    assert_eq!(decoded, Value::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, json

    amount = Field("amount", "decimal128(8, 2)", nullable=False)

    assert json.loads('"12.50"', field=amount) == Decimal("12.50")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, json } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = json.loads('"12.50"', { field: amount })

    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.unscaled, 1250n)
    ```

A Struct Field resolves object names into one ordered row `Sequence` in Rust.
Python and JavaScript restore those names as a dictionary or object; Python
may also pass `cls=SomeDataclass` to materialize the already-decoded row.

## Documents and streams

Rust names each transport in the method:

- `from_utf8`, `from_bytes`, and `from_reader` decode one document;
- `into_utf8`, `into_bytes`, and `into_writer` encode one document;
- `_with_field` applies exact typing;
- `_all` consumes whitespace-separated JSON values;
- `from_lines_*` and the `JsonLines` format require one value per non-empty
  line.

Borrowed reader iterators yield one `Result<Value>` at a time and fuse after
the first error. Writers stream directly to `Write`. Python and JavaScript keep
the conventional `loads` / `dumps` names and leave caller-owned streams open.

=== "Rust"

    ```rust
    use yggdryl::json;

    let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;
    let mut destination = Vec::new();
    json::into_writer_all(&rows, &mut destination)?;

    assert_eq!(rows.len(), 2);
    assert_eq!(destination, b"{\"id\":1}\n{\"id\":2}\n");
    ```

=== "Python"

    ```python
    import io

    from yggdryl import json

    rows = list(json.loads_all('{"id":1}\n{"id":2}\n'))
    destination = io.BytesIO()
    json.dump_all(rows, destination)

    assert rows == [{"id": 1}, {"id": 2}]
    assert destination.getvalue() == b'{"id":1}\n{"id":2}\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const rows = json.loadsAll('{"id":1}\n{"id":2}\n')
    const encoded = json.dumpAll(rows)

    assert.deepEqual(rows, [{ id: 1 }, { id: 2 }])
    assert.deepEqual(json.loadsAll(encoded), rows)
    ```

`loads_all` decodes held content; `load_all` lazily pulls a path or readable
stream. A malformed JSON Lines row reports its byte offset in the original
input, including preceding lines.

## Formatting

JSON defaults to compact output. Rust `Formatting::indented(n)` adds spaces and
newlines; `Formatting::compact()` explicitly requests no layout. Formatting
never changes the parsed value.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{json, Value};

    let value = Value::from_record([("id", Value::I64(1))])?;
    let pretty =
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?;

    assert_eq!(pretty, "{\n  \"id\": 1\n}");
    assert_eq!(json::from_utf8(&pretty)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import json

    pretty = json.dumps({"child": {"id": 1}}, indent=2)
    compact = json.dumps({"child": {"id": 1}}, indent=None)

    assert b'\n  "child"' in pretty
    assert compact == b'{"child":{"id":1}}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const pretty = json.dumps({ child: { id: 1 } }, { indent: 2 })
    const compact = json.dumps({ child: { id: 1 } }, { indent: null })

    assert.ok(pretty.includes(Buffer.from('\n  "child"')))
    assert.deepEqual(compact, Buffer.from('{"child":{"id":1}}'))
    ```

Keys are emitted in deterministic Record order. A general `Mapping` keeps its
insertion order but is writable only when every key is a string.

## Placeholders

JSON rejects placeholder options. Configuration substitution is available for
YAML and TOML, while JSON remains an interchange format whose parsed value is
determined only by its bytes and optional Field.

## Limits and errors

Decode options expose the same nullable core limits in every language:
`max_depth` / `maxDepth`, input bytes, decoded nodes, and document count.
Omitted values use the safe core defaults. The parser also keeps a hard nesting
ceiling, so an adversarial caller cannot request a stack-exhausting depth.

Errors name JSON and the byte offset: invalid UTF-8, duplicate keys, trailing
data in a one-document call, a malformed JSON Lines row, non-finite output, and
Field conversion all fail at the boundary.

## `IOBase` and content coding

Generic structured-text I/O derives JSON and any outer coding from the
handle's `MediaType`. A `quotes.json.gz` handle therefore decodes gzip and JSON
without a format or compression argument. The same plan writes and publishes
the complete value.

Python accepts `PathLike` sources and destinations. JavaScript accepts path
strings, file descriptors, file URLs, and streams.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/json.ipynb){ download },
[Python](notebooks/python/json.ipynb){ download },
[JavaScript](notebooks/javascript/json.ipynb){ download }.

<!-- /notebooks -->
