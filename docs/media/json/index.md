# JSON

JSON and JSON Lines over the shared `Scalar` codec.

## Contract

| facet | contract |
| --- | --- |
| Proves | null, booleans, finite numbers, strings, arrays, string-key objects; anything else needs a `Field` |
| Loads | objects to name-sorted `Record`, arrays to `Sequence`, numbers to the narrowest exact family |
| Dumps | compact; deterministic `Record` order; `Mapping` keeps insertion order, string keys only |
| Bindings | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Exact types | `_with_field` / `field=` / `{ field }` recovers `D128`, `Bytes`, temporals from strings |
| One document | `from_utf8`, `from_bytes`, `from_reader` in; `into_utf8`, `into_bytes`, `into_writer` out |
| Streams | `_all` takes whitespace-separated values; `from_lines_*` and `JsonLines` take one per non-empty line |
| Lazy | `load_all` pulls a path or readable; `loads_all` decodes held content; iterators fuse after the first error |
| Limits | nullable `max_depth` / `maxDepth`, input bytes, decoded nodes, document count; omitted means core defaults |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | `loads` / `dumps`, documents and streams, formatting, `read_scalar` / `write_scalar` over a handle |
| [Arrow](arrow.md) | the document as Arrow rows: one array of objects, or one object per line |

The format-agnostic facade, limits, and `Format` vocabulary are on [Structured documents](../structured.md).

## Use

Loads return only types the JSON grammar proves; dumps interoperate.

=== "Rust"

    ```rust
    use yggdryl::text::json;
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

    assert value.kind == "record"
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
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    assert.deepEqual(json.loads(encoded), natural)
    assert.ok(json.loads(encoded, { scalar: true }).equals(value))
    ```

## Inferring entry point

`from_json_scalar`, `from_json_scalar_with_field`, and `into_json_scalar` are the [inferring entry points](../structured.md#raw-document-codecs) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

## Natural values and exact Fields

Other native values use interoperable spellings, without a private marker envelope:

| native value | natural JSON |
| --- | --- |
| `D128`, `D256` | scale-preserving string |
| `Bytes`, `Geospatial` | base64 string |
| date, time, `DateTime64`, duration | ISO string when representable |
| non-finite float | error |
| Mapping with non-string keys | error |

A schemaless reader sees strings; pass a native [`Field`](../../types/field.md) to recover exact types. A [string](../../types/text.md) Field puts its layout, charset and width on the value it reads and checks its bound, naming the bytes it counted; a byte Field reads base64 and holds the payload to its width or maximum the same way.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::json;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    assert_eq!(json::from_utf8_with_field(r#""12.50""#, &amount)?, Scalar::d128(1_250, 2));

    let symbol = Field::new("symbol", DataType::fixed_ascii(4)?, false);
    let held = json::from_utf8_with_field(r#""AAPL""#, &symbol)?;

    assert_eq!(held.dtype()?, DataType::fixed_ascii(4)?);
    assert_eq!(held.as_str(), Some("AAPL"));
    let refused = json::from_utf8_with_field(r#""AAPLE""#, &symbol).unwrap_err().to_string();
    assert!(refused.contains("expected at most 4 bytes of us-ascii, got 5"), "{refused}");

    let key = Field::new("key", DataType::fixed_size_binary(2)?, false);
    assert_eq!(json::from_utf8_with_field(r#""AP8=""#, &key)?, Scalar::from(&[0_u8, 0xFF][..]));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import DataType, Field, Scalar
    from yggdryl.text import json

    amount = Field("amount", "decimal128(8, 2)", nullable=False)
    exact = json.loads('"12.50"', field=amount, cls=Scalar)
    assert exact.kind == "d128"
    assert exact.unscaled == 1_250
    assert json.loads('"12.50"', field=amount) == Decimal("12.50")

    symbol = Field("symbol", "fixed_ascii(4)", nullable=False)
    held = json.loads('"AAPL"', field=symbol, cls=Scalar)

    assert held.dtype == DataType("fixed_ascii(4)")
    assert held.as_str() == "AAPL"
    assert json.loads('"AAPL"', field=symbol) == "AAPL"
    try:
        json.loads('"AAPLE"', field=symbol)
    except ValueError as error:
        assert "expected at most 4 bytes of us-ascii, got 5" in str(error), error
    else:
        raise AssertionError("five bytes do not fit a four-byte string")

    key = Field("key", "fixed_size_binary(2)", nullable=False)
    assert json.loads('"AP8="', field=key) == b"\x00\xff"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, json } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = json.loads('"12.50"', { field: amount })
    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.unscaled, 1250n)
    assert.equal(decoded.scale, 2)

    const symbol = new Field('symbol', 'fixed_ascii(4)', false)
    const held = json.loads('"AAPL"', { field: symbol, scalar: true })

    assert.ok(held.dtype.equals(DataType.from('fixed_ascii(4)')))
    assert.equal(held.asStr(), 'AAPL')
    assert.equal(json.loads('"AAPL"', { field: symbol }), 'AAPL')
    assert.throws(
      () => json.loads('"AAPLE"', { field: symbol }),
      /expected at most 4 bytes of us-ascii, got 5/,
    )

    const key = new Field('key', 'fixed_size_binary(2)', false)
    assert.deepEqual(json.loads('"AP8="', { field: key }), Buffer.from([0, 255]))
    ```

A Struct Field yields one ordered row `Sequence` in Rust, a dictionary or object elsewhere; Python `cls=SomeDataclass` materializes it.

## Edges

- duplicate object names -> rejected.
- text naming an existing file -> parsed as JSON, never read; a bare file name is invalid syntax.
- invalid UTF-8, trailing data after one document, or a failed `Field` conversion -> error at the boundary, with the byte offset.
- malformed JSON Lines row -> error at its offset in the original input, preceding lines included.
- depth above the hard nesting ceiling -> refused, whatever `max_depth` asks.
- placeholder options -> refused; only [YAML](../yaml/index.md) and [TOML](../toml/index.md) substitute, see [Placeholders](../placeholders.md).
- streamed Arrow batches -> [Text records](../text/index.md); keyed merge is refused there since a line has no row identity.

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
