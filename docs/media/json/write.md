# Writing JSON

One native [`Scalar`](../../types/scalar.md) out - as a JSON or JSON Lines document first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Value out | `into_utf8`, `into_bytes`, `into_writer`; Python `dumps` / `dump`, JavaScript `dumps` / `dump` |
| Location out | a destination string is a path in both bindings - the asymmetry a source string does not have; a writable stream is written to and left open |
| Order | compact; a `Scalar` record and a JavaScript object are written in name order, a Python `Mapping` in its own; string keys only |
| Many | `into_bytes_all`, `into_utf8_all`, `into_writer_all` and `dump_all` / `dumpAll` write one compact document per line |
| Layout | `Formatting::indented(n)` / `indent=n` / `{ indent: n }` spaces each nesting level and changes bytes, never meaning |
| Handle | `write_scalar` derives JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `write_arrow(value, mode, options)`; JSON frames every row in one array, JSON Lines writes one document per line |
| Mode | overwrite only: a document is one frame around its rows, so it is written whole |
| Held | `.json` holds the rows it frames; `.jsonl` streams, holding no more than the batch being encoded |
| Bindings | all three for the scalar surface; `write_arrow` is Rust and Python only |

## Use

The smallest write: one value in, compact bytes out. Nothing in the call names a layout, because compact is the default, and nothing names an order: a record is written in name order, a Python `Mapping` in its own.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::Scalar;

    let value = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("quantity", Scalar::from(100_i64)),
    ])?;

    // A record is written in name order, whatever order it was built in.
    assert_eq!(json::into_utf8(&value)?, r#"{"quantity":100,"symbol":"AAPL"}"#);
    assert_eq!(json::into_bytes(&value)?, br#"{"quantity":100,"symbol":"AAPL"}"#);
    ```

=== "Python"

    ```python
    from yggdryl import json

    value = {"symbol": "AAPL", "quantity": 100}

    # A `Mapping` is written in its own order, not sorted.
    assert json.dumps(value) == b'{"symbol":"AAPL","quantity":100}'
    assert json.dump(value, utf8=True) == '{"symbol":"AAPL","quantity":100}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = { symbol: 'AAPL', quantity: 100 }
    const encoded = json.dumps(value)

    // An object is written in name order here, where a Python `Mapping` is not.
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    ```

## A document to a location

Rust writes to any `Write`; Python and JavaScript take a path or a writable destination, and a caller's stream is written to and left open.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::Scalar;

    let value = Scalar::from_struct([("id", Scalar::from(1_i64))])?;
    let mut destination = Vec::new();
    json::into_writer(&value, &mut destination)?;

    assert_eq!(destination, br#"{"id":1}"#);
    ```

=== "Python"

    ```python
    import io
    import pathlib
    import tempfile

    from yggdryl import json

    target = pathlib.Path(tempfile.mkdtemp()) / "quote.json"
    json.dump({"quantity": 100, "symbol": "AAPL"}, target)
    assert target.read_bytes() == b'{"quantity":100,"symbol":"AAPL"}'

    destination = io.BytesIO()
    json.dump({"id": 1}, destination)
    assert destination.getvalue() == b'{"id":1}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { json } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-json-'))
    const target = path.join(root, 'quote.json')

    // A destination string is a path, where a source string would be content.
    json.dump({ quantity: 100, symbol: 'AAPL' }, target)
    assert.equal(fs.readFileSync(target).toString(), '{"quantity":100,"symbol":"AAPL"}')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `trade.json.gz` publishes without arguments: the coding goes back on after the writer.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///trade.json.gz")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_struct([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;
    handle.write_scalar(&value)?;

    // The bytes on the handle are gzip, not JSON.
    assert_eq!(&handle.read_all_bytes()?[..2], &[0x1F, 0x8B]);
    assert_eq!(
        handle.read_scalar(None)?.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "trade.json.gz"
    handle = IOBase(target)
    handle.write_scalar({"quantity": 2, "symbol": "AAPL"})

    assert target.read_bytes()[:2] == b"\x1f\x8b"
    assert handle.read_scalar() == {"quantity": 2, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-json-'))
    const handle = new IOBase(path.join(root, 'trade.json.gz'))
    handle.writeScalar({ quantity: 2, symbol: 'AAPL' })

    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual(handle.readScalar(), { quantity: 2, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## JSON Lines

One compact document per line, in the order the values arrive. Rust streams to any `Write`, Python `dump_all` to a destination, JavaScript `dumpAll` to a `Buffer` or a destination.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::Scalar;

    let rows = [
        Scalar::from_struct([("id", Scalar::from(1_i64))])?,
        Scalar::from_struct([("id", Scalar::from(2_i64))])?,
    ];
    let mut destination = Vec::new();
    json::into_writer_all(&rows, &mut destination)?;

    assert_eq!(destination, b"{\"id\":1}\n{\"id\":2}\n");
    ```

=== "Python"

    ```python
    import io

    from yggdryl import json

    destination = io.BytesIO()
    json.dump_all([{"id": 1}, {"id": 2}], destination)

    assert destination.getvalue() == b'{"id":1}\n{"id":2}\n'
    assert json.dumps_all([{"id": 1}, {"id": 2}]) == b'{"id":1}\n{"id":2}\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const rows = [{ id: 1 }, { id: 2 }]
    const encoded = json.dumpAll(rows)

    assert.equal(encoded.toString(), '{"id":1}\n{"id":2}\n')
    assert.deepEqual(json.loadsAll(encoded), rows)
    ```

## Formatting

JSON is compact by default and `indent=n` spaces each nesting level; the [shared `Formatting`](../structured.md#formatting) rules own the vocabulary and pin the exact bytes of all three formats. An indented document no longer occupies one line-delimited slot, so JSON Lines stays compact.

## Rows as Arrow batches

`write_arrow` is the one bridge from a batch to a document. A `.json` name frames every row in one array and holds them; a `.jsonl` name writes one document per line and holds no more than the batch being encoded.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.json")?.media_type());
    handle.write_all_bytes(br#"[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]"#)?;
    let value = handle.read_arrow(None)?;

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert_eq!(&handle.read_all_bytes()?[..2], b"[{");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
    target.write_bytes(b'[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]')
    handle = IOBase(target)
    value = handle.read_arrow()

    # Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value.into_arrow_table())
    assert handle.read_bytes().startswith(b"[{")
    ```

## Edges

- a non-finite float, or a `Mapping` with non-string keys -> error, never silent coercion.
- a Python `Mapping` built out of name order -> written in that order; the same object in JavaScript -> name-sorted, as a `Scalar` record is.
- `append` or `merge` -> refused naming the mode; a document is written whole.
- `trade.json.gz` -> the coding goes back on after the writer.
- an indented JSON Lines stream -> no longer one document per line, which is why compact is the default there.
- `overwrite_arrow_reader` on a `.json` name -> refused; [JSON is not a record encoding](index.md#contract).
- JavaScript -> `write_arrow` is not bound; build the rows with Arrow JS and write them with [`json.dumps`](#use).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test json
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo bench -p yggdryl --bench text -- codec/json
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/json/test_init.py
    python/.venv/bin/python -m pytest python/tests/test_arrow.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    ```
