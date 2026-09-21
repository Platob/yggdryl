# Reading TOML

One TOML document in - as a native [`Scalar`](../../types/scalar.md) first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Content in | `from_utf8`, `from_bytes`, `from_reader`; Python and JavaScript `loads` - a string is content, never a location |
| Location in | Python `loads` takes a `PathLike`; JavaScript `load` takes a descriptor or a `file:` URL, and answers a promise for a stream - a string stays content; Rust reads a location through a handle |
| Returns | Rust `Scalar`, always a string-key `Record`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural mappings |
| One document | exactly one; `from_*_all` answers a one-element collection and the bindings expose no `loads_all` |
| Lazy | `from_reader_iter` yields that one document and fuses after an error; `Reader::byte_offset` reports the bytes pulled |
| Limits | `_with_limits` / `max_input_bytes=` / `{ maxInputBytes }` bound input bytes, depth, decoded nodes and documents while reading |
| Handle | `read_scalar` derives TOML and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `read_arrow(options)`; TOML has no top-level sequence, so the rows are the array of tables stored under the root's name |
| Field | declares the root, which is also the table name the rows are stored under |
| Held | a document has no frame to read a prefix of, so the rows the one frame encloses are held |
| Bindings | all three for the scalar surface; `read_arrow` is Rust and Python only |

## Use

The smallest read: content in, one record out. `cls=Scalar` / `{ scalar: true }` asks for the lossless core value instead of the natural one.

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::Scalar;

    let source = "title = \"yggdryl\"\ncount = 3\n";
    let value = toml::from_utf8(source)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_str),
        Some("yggdryl")
    );
    // Bytes read the same document as text does.
    assert_eq!(toml::from_bytes(source.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import toml

    source = 'title = "yggdryl"\ncount = 3\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)

    assert natural == {"count": 3, "title": "yggdryl"}
    assert value.kind == "struct"
    assert value.as_py() == natural
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n'
    const natural = toml.loads(source)
    const value = toml.loads(source, { scalar: true })

    assert.deepEqual(natural, { count: 3, title: 'yggdryl' })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    ```

## A document from a location

Rust reads any `Read`; Python takes a `PathLike` where a `str` would have been content, and JavaScript takes a file descriptor or a `file:` URL for the same reason - a bare string is parsed as content, not opened.

=== "Rust"

    ```rust
    use std::io::Cursor;

    use yggdryl::toml;
    use yggdryl::Scalar;

    let value = toml::from_reader(Cursor::new(b"id = 1\n".to_vec()))?;

    assert_eq!(value, Scalar::from_struct([("id", Scalar::from(1))])?);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.text import toml

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.toml"
    source.write_bytes(b'symbol = "AAPL"\nquantity = 100\n')

    # A `str` is content; a `PathLike` is the location.
    assert toml.loads(source) == {"quantity": 100, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { toml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-'))
    const source = path.join(root, 'quote.toml')
    fs.writeFileSync(source, 'symbol = "AAPL"\nquantity = 100\n')

    // A string is content, so a location is spelled as a `file:` URL.
    assert.deepEqual(toml.load(pathToFileURL(source)), { quantity: 100, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives TOML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quote.toml.gz` reads without arguments: the coding comes off before the parser sees a byte.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///quote.toml")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    handle.write_all_bytes(b"symbol = \"AAPL\"\nquantity = 100\n")?;

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

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.toml.gz"
    source.write_bytes(gzip.compress(b'symbol = "AAPL"\nquantity = 100\n'))

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

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-'))
    const source = path.join(root, 'quote.toml.gz')
    fs.writeFileSync(source, zlib.gzipSync('symbol = "AAPL"\nquantity = 100\n'))

    assert.deepEqual(new IOBase(source).readScalar(), { quantity: 100, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Exactly one document

TOML carries one document, so the `_all` family exists only to answer the same shape the streaming formats do: a one-element collection. The bindings drop it rather than spell a stream TOML has not got.

=== "Rust"

    ```rust
    use yggdryl::toml;

    let documents = toml::from_utf8_all("id = 1\n")?;

    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0], toml::from_utf8("id = 1\n")?);
    ```

=== "Python"

    ```python
    from yggdryl.text import toml

    # A single-document format binds no collection reader.
    for name in ("load_all", "loads_all"):
        assert not hasattr(toml, name)
    assert toml.loads("id = 1\n") == {"id": 1}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    // A single-document format binds no collection reader.
    for (const name of ['loadAll', 'loadsAll']) {
      assert.equal(toml[name], undefined)
    }
    assert.deepEqual(toml.loads('id = 1\n'), { id: 1 })
    ```

## Rows as Arrow batches

`read_arrow` is the one bridge from a document to a batch. TOML has no top-level sequence, so the rows are the array of tables stored under the root's name; a root naming no array of tables is one row.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.toml")?.media_type());
    handle.write_all_bytes(
        b"[[row]]\nid = 1\nsymbol = \"AAPL\"\n\n[[row]]\nid = 2\nsymbol = \"MSFT\"\n",
    )?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.toml"
    source.write_bytes(b'[[row]]\nid = 1\nsymbol = "AAPL"\n\n[[row]]\nid = 2\nsymbol = "MSFT"\n')
    handle = IOBase(source)

    # The document's own shape holds the rows; the root is the one they prove.
    value = handle.read_arrow()
    assert value.shape == "batch"
    assert value.as_py() == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": "MSFT"},
    ]
    ```

A declared [`Field`](values.md) names the root the rows are stored under and types the document's natural strings on the way in.

## Edges

- more than one document -> refused; TOML carries exactly one.
- a root that is not a string-key record -> refused; TOML has no other top-level shape.
- empty or comment-only document -> empty `Record`.
- text naming an existing file -> parsed as TOML content, never read; a bare file name fails as the bare word it is.
- invalid UTF-8, a duplicate key, or trailing data -> error naming TOML and the byte offset.
- `quotes.toml.gz` -> the coding comes off before the parser.
- a root naming no array of tables -> the document is one row.
- a [placeholder](../placeholders.md) -> substituted before the rows are typed, so the resolved string becomes the exact value.
- `read_arrow_reader` on a `.toml` name -> refused; [TOML is not a record encoding](index.md#contract).
- JavaScript -> `read_arrow` is not bound; use [`toml.loads`](#use) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text toml::
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo test --features "parquet iceberg" -p yggdryl --lib toml::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/toml
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/toml.test.js
    ```
