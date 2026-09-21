# Reading JSON

One JSON or JSON Lines document in - as a native [`Scalar`](../../types/scalar.md) first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Content in | `from_utf8`, `from_bytes`, `from_reader`; Python and JavaScript `loads` - a string is content, never a location |
| Location in | Python `loads` takes a `PathLike`; JavaScript `load` takes a `file:` URL, a descriptor or a stream; Rust reads a location through a handle |
| Returns | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Many | `_all` takes whitespace-separated values; `from_lines_*` and `loads_all` / `loadsAll` take one per non-empty line |
| Lazy | `load_all` pulls a path or readable; `loads_all` decodes held content; `from_reader_iter` and every binding iterator fuse after the first error |
| Handle | `read_scalar` derives JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `read_arrow(options)`; an array document holds the rows, any other document is one row; JSON Lines reads every line, then one batch |
| Field | declares the root the rows land under; without one the rows name the root their contents prove |
| Held | a document has no frame to read a prefix of, so `.json` holds the rows its one frame encloses |
| Bindings | all three for the scalar surface; `read_arrow` is Rust and Python only |

## Use

The smallest read: content in, one value out. `cls=Scalar` / `{ scalar: true }` asks for the lossless core value instead of the natural one.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::Scalar;

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    // Bytes read the same document as text does.
    assert_eq!(json::from_bytes(br#"{"symbol":"AAPL","quantity":100}"#)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)

    assert natural == {"quantity": 100, "symbol": "AAPL"}
    assert value.kind == "struct"
    assert value.as_py() == natural
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })

    assert.deepEqual(natural, { quantity: 100, symbol: 'AAPL' })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    ```

## A document from a location

Rust reads any `Read`; Python takes a `PathLike` where a `str` would have been content, and JavaScript takes a `file:` URL, a file descriptor or a stream for the same reason.

=== "Rust"

    ```rust
    use std::io::Cursor;

    use yggdryl::json;
    use yggdryl::Scalar;

    let value = json::from_reader(Cursor::new(br#"{"id":1}"#.to_vec()))?;

    assert_eq!(value, Scalar::from_struct([("id", Scalar::from(1))])?);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.text import json

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.json"
    source.write_bytes(b'{"symbol":"AAPL","quantity":100}')

    # A `str` is content; a `PathLike` is the location.
    assert json.loads(source) == {"quantity": 100, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { json } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-json-'))
    const source = path.join(root, 'quote.json')
    fs.writeFileSync(source, '{"symbol":"AAPL","quantity":100}')

    // A string is content, so a location is spelled as a `file:` URL.
    assert.deepEqual(json.load(pathToFileURL(source)), { quantity: 100, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quote.json.gz` reads without arguments: the coding comes off before the parser sees a byte.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///quote.json")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    handle.write_all_bytes(br#"{"symbol":"AAPL","quantity":100}"#)?;

    let value = handle.read_scalar(None)?;
    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.json.gz"
    source.write_bytes(gzip.compress(b'{"symbol":"AAPL","quantity":100}'))

    assert IOBase(source).read_scalar() == {"quantity": 100, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-json-'))
    const source = path.join(root, 'quote.json.gz')
    fs.writeFileSync(source, zlib.gzipSync('{"symbol":"AAPL","quantity":100}'))

    assert.deepEqual(new IOBase(source).readScalar(), { quantity: 100, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Many documents and JSON Lines

`from_lines_*` and `loads_all` / `loadsAll` read one document per non-empty line; Rust's `_all` family also reads whitespace-separated values from one input. Python `load_all` and JavaScript `loadAll` are the lazy halves, pulling a path or a readable rather than decoding held content.

=== "Rust"

    ```rust
    use yggdryl::json;

    let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;

    assert_eq!(rows.len(), 2);
    assert_eq!(rows, json::from_lines_bytes(b"{\"id\":1}\n{\"id\":2}\n")?);
    ```

=== "Python"

    ```python
    from yggdryl.text import json

    rows = list(json.loads_all('{"id":1}\n{"id":2}\n'))

    assert rows == [{"id": 1}, {"id": 2}]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const rows = json.loadsAll('{"id":1}\n{"id":2}\n')

    assert.deepEqual(rows, [{ id: 1 }, { id: 2 }])
    ```

## Rows as Arrow batches

`read_arrow` is the one bridge from a document to a batch. An array document holds the rows, any other document is one row, and a `.jsonl` name reads every line before answering one batch.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.json")?.media_type());
    handle.write_all_bytes(br#"[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]"#)?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
    source.write_bytes(b'[{"id":1,"symbol":"AAPL"},{"id":2,"symbol":"MSFT"}]')

    # The document's own shape holds the rows; the root is the one they prove.
    value = IOBase(source).read_arrow()
    assert value.shape == "batch"
    assert value.as_py() == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": "MSFT"},
    ]
    ```

A declared [`Field`](values.md) names the root the rows land under and types the document's natural strings on the way in.

## Edges

- duplicate object names -> rejected.
- invalid UTF-8, or trailing data after one document -> error at the boundary, with the byte offset.
- malformed JSON Lines row -> error at its offset in the original input, preceding lines included.
- a streamed iterator -> fused after the first error.
- `quote.json.gz` -> the coding comes off before the parser.
- a document that is not an array -> one row.
- a row shape no `Field` proves -> inferred from what the document proves, which is the rule every schemaless read follows.
- `read_arrow_reader` on a `.json` name -> refused; [JSON is not a record encoding](index.md#contract).
- JavaScript -> `read_arrow` is not bound; use [`json.loads`](#use) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text json::
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo bench -p yggdryl --bench text -- codec/json
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/json
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    ```
