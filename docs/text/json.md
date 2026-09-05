# JSON

JSON and JSON Lines over the shared `Scalar` codec.

## Contract

| facet | contract |
| --- | --- |
| Loads | objects to name-sorted `Record`, arrays to `Sequence`, numbers to the narrowest exact natural family |
| Dumps | compact by default; deterministic `Record` key order; `Mapping` keeps insertion order, string keys only |
| Bindings | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return that tree, else natural objects |
| Exact types | `_with_field` / `field=` / `{ field }` recovers `D128`, `Bytes`, temporals from string spellings |
| Lazy | `load_all` pulls a path or stream; `loads_all` decodes held content; iterators fuse after the first error |
| Limits | nullable `max_depth` / `maxDepth`, input bytes, decoded nodes, document count; omitted means safe core defaults |
| Placeholders | refused; see [Placeholders](placeholders.md) |
| Errors | name JSON and the byte offset |

## Use

Loads return only types the JSON grammar proves; dumps interoperate.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::json;

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(
        json::into_utf8(&value)?,
        r#"{"quantity":100,"symbol":"AAPL"}"#
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {"quantity": 100, "symbol": "AAPL"}
    assert json.dumps(value) == b'{"quantity":100,"symbol":"AAPL"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(json.loads(encoded), natural)
    ```

## Inferring entry point

`from_json_scalar`, `from_json_scalar_with_field`, and `into_json_scalar` are the [inferring entry points](index.md) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`.

=== "Rust"

    ```rust
    use yggdryl::{from_json_scalar, into_json_scalar};

    let value = from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?;
    let encoded = into_json_scalar(&value)?;

    assert_eq!(encoded, r#"{"quantity":100,"symbol":"AAPL"}"#);
    assert_eq!(from_json_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)
    encoded = json.dumps(value)

    assert encoded == b'{"quantity":100,"symbol":"AAPL"}'
    assert json.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    assert.ok(json.loads(encoded, { scalar: true }).equals(value))
    ```

## Natural values and exact Fields

Values without JSON syntax use interoperable spellings, with no private marker envelope:

| native value | natural JSON |
| --- | --- |
| `D128`, `D256` | scale-preserving string |
| `Bytes`, `Geospatial` | base64 string |
| date, time, `DateTime64`, duration | ISO string when representable |
| non-finite float | error |
| Mapping with non-string keys | error |

A schemaless reader sees those as strings; pass a native [`Field`](../types/field.md) to recover exact types.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::json;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = json::from_utf8_with_field(r#""12.50""#, &amount)?;

    assert_eq!(decoded, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field
    from yggdryl.text import json

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

A Struct Field yields one ordered row `Sequence` in Rust and a dictionary or object elsewhere; Python `cls=SomeDataclass` materializes it.

## Documents and streams

Rust names the transport in the method; Python and JavaScript keep `loads` / `dumps` and leave caller-owned streams open.

| Rust method | transport |
| --- | --- |
| `from_utf8`, `from_bytes`, `from_reader` | decode one document |
| `into_utf8`, `into_bytes`, `into_writer` | encode one document |
| `_with_field` | exact typing |
| `_all` | whitespace-separated values |
| `from_lines_*`, `JsonLines` | one value per non-empty line |

Reader iterators yield one `Result<Scalar>` at a time; writers stream to `Write`.

=== "Rust"

    ```rust
    use yggdryl::text::json;

    let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;
    let mut destination = Vec::new();
    json::into_writer_all(&rows, &mut destination)?;

    assert_eq!(rows.len(), 2);
    assert_eq!(destination, b"{\"id\":1}\n{\"id\":2}\n");
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import json

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

## Formatting

Rust `Formatting::indented(n)` adds layout and `Formatting::compact()` removes it; neither changes the parsed value.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::json;

    let value = Scalar::from_record([("id", Scalar::from(1_i64))])?;
    let pretty =
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?;

    assert_eq!(pretty, "{\n  \"id\": 1\n}");
    assert_eq!(json::from_utf8(&pretty)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import json

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

## `IOBase` and content coding

[Structured-text I/O](../holder/iobase/values.md) derives JSON and any outer [coding](../coding/index.md) from the handle's `MediaType`, so `quotes.json.gz` reads, writes, and publishes without format arguments. Python accepts `PathLike`; JavaScript accepts path strings, file descriptors, file URLs, and streams.

## Edges

- duplicate object names -> rejected.
- non-finite float, or a `Mapping` with a non-string key -> encode error.
- text naming an existing file -> parsed as JSON, never read; a bare file name is invalid syntax.
- trailing data in a one-document call -> error at the byte offset.
- malformed JSON Lines row -> error at its offset in the original input, preceding lines included.
- invalid UTF-8, or a failed `Field` conversion -> error at the boundary.
- depth above the hard nesting ceiling -> refused, whatever `max_depth` asks.
- placeholder options -> refused; only [YAML](yaml.md) and [TOML](toml.md) substitute.
- streamed Arrow batches with overwrite or append -> [Text records](../media/text.md), which refuses keyed merge since a line has no row identity.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text json::
    cargo bench -p yggdryl --bench text -- codec/json
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/json
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    npm run --prefix node bench:text
    ```
