# Reading YAML

One YAML document, or a whole stream of them, in - as a native [`Scalar`](../../types/scalar.md) first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Content in | `from_utf8`, `from_bytes`, `from_reader`; Python and JavaScript `loads` - a string is content, never a location |
| Location in | Python `loads` takes a `PathLike`; JavaScript `load` takes a `file:` URL, a descriptor or a stream; Rust reads a location through a handle |
| Returns | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Many | `from_utf8_all`, `from_bytes_all`, `from_reader_all`; Python `loads_all`, JavaScript `loadsAll` - one value per `---` separated document |
| Lazy | `from_reader_iter[_with_field]` and Python `load_all` pull a `PathLike` or a readable and yield as they go; JavaScript `loadAll` answers an array for a `file:` URL or a descriptor, and an async iterable only for a stream; every iterator fuses after the first error |
| Handle | `read_scalar` derives YAML and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `read_arrow(options)` reads every document in the stream, then answers one batch |
| Held | the documents that batch is built from: a document has no frame to read a prefix of, so a read holds the rows it collected |
| Field | declares the root the rows land under; without one the rows name the root their contents prove |
| Bindings | all three for the scalar surface; `read_arrow` is Rust and Python only |

## Use

The smallest read: content in, one value out. `cls=Scalar` / `{ scalar: true }` asks for the lossless core value instead of the natural one.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::Scalar;

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    // Bytes read the same document as text does.
    assert_eq!(yaml::from_bytes(b"symbol: AAPL\nquantity: 2\n")?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import yaml

    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)

    assert natural == {"quantity": 2, "symbol": "AAPL"}
    assert value.kind == "struct"
    assert value.as_py() == natural
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })

    assert.deepEqual(natural, { quantity: 2, symbol: 'AAPL' })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    ```

## A document from a location

Rust reads any `Read`; Python takes a `PathLike` where a `str` would have been content, and JavaScript takes a `file:` URL, a file descriptor or a stream for the same reason.

=== "Rust"

    ```rust
    use std::io::Cursor;

    use yggdryl::yaml;
    use yggdryl::Scalar;

    let value = yaml::from_reader(Cursor::new(b"id: 1\n".to_vec()))?;

    assert_eq!(value, Scalar::from_struct([("id", Scalar::from(1_i64))])?);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.text import yaml

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.yaml"
    source.write_bytes(b"symbol: AAPL\nquantity: 2\n")

    # A `str` is content; a `PathLike` is the location.
    assert yaml.loads(source) == {"quantity": 2, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { yaml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-yaml-'))
    const source = path.join(root, 'quote.yaml')
    fs.writeFileSync(source, 'symbol: AAPL\nquantity: 2\n')

    // A string is content, so a location is spelled as a `file:` URL.
    assert.deepEqual(yaml.load(pathToFileURL(source)), { quantity: 2, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Many documents in one stream

`---` separates documents, and the `_all` family reads every one of them. Python `load_all` is the lazy half, pulling a `PathLike` or a readable rather than decoding held content. JavaScript `loadAll` reads a location - a `file:` URL or a descriptor - and answers an array; only a stream makes it an async iterable, because a JavaScript string is content here as everywhere. A one-document form given a second document is an error, so a stream is read through these.

=== "Rust"

    ```rust
    use yggdryl::yaml;

    let documents = yaml::from_utf8_all("id: 1\n---\nid: 2\n")?;

    assert_eq!(documents.len(), 2);
    assert_eq!(yaml::from_bytes_all(b"id: 1\n---\nid: 2\n")?, documents);
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import yaml

    # `load_all` pulls the readable lazily; `loads_all` decodes held content.
    documents = list(yaml.load_all(io.BytesIO(b"id: 1\n---\nid: 2\n")))

    assert documents == [{"id": 1}, {"id": 2}]
    assert list(yaml.loads_all("id: 1\n---\nid: 2\n")) == documents
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { yaml } = require('yggdryl')

    const documents = yaml.loadsAll('id: 1\n---\nid: 2\n')

    assert.deepEqual(documents, [{ id: 1 }, { id: 2 }])

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-yaml-'))
    const source = path.join(root, 'quotes.yaml')
    fs.writeFileSync(source, 'id: 1\n---\nid: 2\n')

    // `loadAll` reads the location; the same string would have been content.
    assert.deepEqual(yaml.loadAll(pathToFileURL(source)), documents)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives YAML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quote.yaml.gz` reads without arguments: the coding comes off before the parser sees a byte.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///quote.yaml")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    handle.write_all_bytes(b"symbol: AAPL\nquantity: 2\n")?;

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

    source = pathlib.Path(tempfile.mkdtemp()) / "quote.yaml.gz"
    source.write_bytes(gzip.compress(b"symbol: AAPL\nquantity: 2\n"))

    assert IOBase(source).read_scalar() == {"quantity": 2, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-yaml-'))
    const source = path.join(root, 'quote.yaml.gz')
    fs.writeFileSync(source, zlib.gzipSync('symbol: AAPL\nquantity: 2\n'))

    assert.deepEqual(new IOBase(source).readScalar(), { quantity: 2, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

`read_arrow` is the one bridge from a document stream to a batch: every document in the stream is one row, and the read answers one batch. A single document that is a sequence holds its items as the rows; any other single document is one row.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.yaml")?.media_type());
    handle.write_all_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")?;

    // The document's own shape holds the rows; the root is the one they prove.
    let value = handle.read_arrow(None)?;
    assert_eq!((value.row_size(), value.column_size()), (Some(2), 2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "quotes.yaml"
    source.write_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")

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

- a second document through a one-document form -> error; use the stream forms.
- stream failure -> the error names the document start and the failing byte offset; the iterator is then exhausted.
- text naming an existing file -> that plain string scalar, not the file's content; inference is deterministic and a source string is always content.
- invalid UTF-8, or a duplicate key -> error naming YAML and the byte offset, cumulative across documents.
- `quote.yaml.gz` -> the coding comes off before the parser; Python takes a `PathLike`, JavaScript paths, descriptors, file URLs and streams.
- a [placeholder](../placeholders.md) -> substituted before the rows are typed, so the resolved string becomes the exact value.
- a row shape no `Field` proves -> inferred from what the documents prove, which is the rule every schemaless read follows.
- `read_arrow_reader` on a `.yaml` name -> refused; [YAML is not a record encoding](index.md#contract).
- JavaScript -> `read_arrow` is not bound; use [`yaml.loads`](#use) and build the batch with Arrow JS.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text yaml::
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo bench -p yggdryl --bench text -- codec/yaml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/yaml
    python/.venv/bin/python -m pytest python/tests/arrow/test_arrow_scalar.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="yaml" node/tests/text/codec.test.js
    ```
