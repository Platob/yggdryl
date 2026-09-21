# JSON

JSON and JSON Lines as one scheme over the shared [`Scalar`](../../types/scalar.md) codec.

## Contract

| facet | contract |
| --- | --- |
| Proves | null, booleans, finite numbers, strings, arrays, string-key objects; anything else needs a [`Field`](values.md) |
| Reads | `from_utf8`, `from_bytes`, `from_reader` with their `_all` and `from_lines_*` siblings; objects become a name-sorted record, arrays a `Sequence`, numbers the narrowest exact family |
| Writes | `into_utf8`, `into_bytes`, `into_writer` with their `_all` siblings; compact, deterministic record order, string keys only |
| Bindings | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Exact types | `_with_field` / `field=` / `{ field }` recovers `D128`, `Bytes`, temporals from strings |
| One document | `from_utf8`, `from_bytes`, `from_reader` in; `into_utf8`, `into_bytes`, `into_writer` out |
| Streams | `_all` takes whitespace-separated values; `from_lines_*` and `Format::JsonLines` take one per non-empty line |
| Lazy | `load_all` pulls a path or readable; `loads_all` decodes held content; iterators fuse after the first error |
| Handle | `read_scalar` / `write_scalar` derive JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quotes.json.gz` needs no argument |
| Arrow | `read_arrow` and `write_arrow` are the one bridge to rows, in Rust and Python only; [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Limits | nullable `max_depth` / `maxDepth`, input bytes, decoded nodes, document count; omitted means core defaults |

## Use

Loads return only types the JSON grammar proves; dumps interoperate.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::{from_json_scalar, into_json_scalar, Scalar};

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;
    let encoded = json::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    assert_eq!(encoded, r#"{"quantity":100,"symbol":"AAPL"}"#);

    // The crate-root inferring entry points answer the same value, from text or bytes.
    assert_eq!(from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?, value);
    assert_eq!(into_json_scalar(&value)?, encoded);
    assert_eq!(from_json_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)
    encoded = json.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {"quantity": 100, "symbol": "AAPL"}
    assert encoded == b'{"quantity":100,"symbol":"AAPL"}'
    assert json.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    assert.deepEqual(json.loads(encoded), natural)
    assert.ok(json.loads(encoded, { scalar: true }).equals(value))
    ```

## Pages

| Page | Owns |
| --- | --- |
| [Read](read.md) | a document as a native scalar, from content, a location or a handle; JSON Lines; then rows as Arrow batches |
| [Write](write.md) | a scalar as a document, to bytes, a location or a handle; JSON Lines; formatting; then Arrow batches |
| [Values](values.md) | what a bare parse answers, and what a declared `Field` changes |

Every scheme answers the same two surfaces. JSON answers native scalars in all three bindings, and Arrow rows in Rust and Python only - JavaScript binds neither `read_arrow` nor `write_arrow`. The format-agnostic facade, the `Format` and `Limits` vocabulary, and the shared `Formatting` rules are on [Structured documents](../structured.md).

## Inferring entry point

`from_json_scalar`, `from_json_scalar_with_field`, and `into_json_scalar` are the [inferring entry points](../structured.md#raw-document-codecs) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

## Edges

- text naming an existing file -> parsed as JSON, never read; a bare file name is invalid syntax.
- depth above the hard nesting ceiling -> refused, whatever `max_depth` asks.
- placeholder options -> refused; only [YAML](../yaml/index.md) and [TOML](../toml/index.md) substitute, see [Placeholders](../placeholders.md).
- `read_arrow_reader`, `overwrite_arrow_reader` on a `.json` name -> refused; JSON is not a record encoding.
- streamed Arrow batches -> [Text records](../text/index.md); keyed merge is refused there since a line has no row identity.
- `_with_limits` / `_with_formatting` -> explicit form only; the inferring entry point has neither.

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
