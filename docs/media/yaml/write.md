# Writing YAML

One native [`Scalar`](../../types/scalar.md) out - as a YAML document or a document stream first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Value out | `into_utf8`, `into_bytes`, `into_writer`; Python `dumps` / `dump`, JavaScript `dumps` / `dump` |
| Location out | a destination string is a path in both bindings - the asymmetry a source string does not have; a writable stream is written to and left open |
| Order | a `Record` is written in name order, so repeated dumps are byte-identical; a `Mapping` keeps insertion order. A JavaScript object is a `Record`; a Python `dict` is a `Mapping`, so it writes in its own order and a read sorts it back |
| Many | `into_bytes_all`, `into_utf8_all`, `into_writer_all`; Python `dumps_all` / `dump_all`, JavaScript `dumpAll` - documents `---` separated |
| Layout | two-space block style by default; `Formatting::indented(n)` / `indent=n` / `{ indent: n }` keeps block style at that width, `Formatting::compact()` / `indent=None` / `{ indent: null }` is flow style |
| Quoting | the writer quotes a scalar whose plain spelling would change its type or the structure around it |
| Handle | `write_scalar` derives YAML and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `write_arrow(value, mode)` writes one document per row, `---` separated |
| Mode | overwrite only: a document set is written whole |
| Held | nothing but the batch being encoded: the frame is per document, so rows stream out |
| Bindings | all three for the scalar surface; `write_arrow` is Rust and Python only |

## Use

The smallest write: one value in, block-style bytes out. Nothing in the call names a layout, because two-space block style is the default. A `Record` writes in name order whatever order it was built in, which is why a JavaScript object does; a Python `dict` is a `Mapping` and keeps its own.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::Scalar;

    let value = Scalar::from_struct([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;

    assert_eq!(yaml::into_utf8(&value)?, "quantity: 2\nsymbol: AAPL\n");
    assert_eq!(yaml::into_bytes(&value)?, b"quantity: 2\nsymbol: AAPL\n");

    // A `Record` writes in name order, whatever order it was built in.
    let built = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("quantity", Scalar::from(2_i64)),
    ])?;
    assert_eq!(yaml::into_utf8(&built)?, "quantity: 2\nsymbol: AAPL\n");
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    value = {"quantity": 2, "symbol": "AAPL"}

    assert yaml.dumps(value) == b"quantity: 2\nsymbol: AAPL\n"
    assert yaml.dump(value, utf8=True) == "quantity: 2\nsymbol: AAPL\n"

    # A `dict` is a `Mapping`, so it writes in its own order; the read sorts it back.
    assert yaml.dumps({"symbol": "AAPL", "quantity": 2}) == b"symbol: AAPL\nquantity: 2\n"
    assert yaml.loads(b"symbol: AAPL\nquantity: 2\n") == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = { quantity: 2, symbol: 'AAPL' }
    const encoded = yaml.dumps(value)

    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')

    // An object is a `Record`, so it writes in name order however it was built.
    assert.equal(
      yaml.dumps({ symbol: 'AAPL', quantity: 2 }).toString(),
      'quantity: 2\nsymbol: AAPL\n',
    )
    ```

## A document to a location

Rust writes to any `Write`; Python and JavaScript take a path or a writable destination, and a caller's stream is written to and left open.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::Scalar;

    let value = Scalar::from_struct([("id", Scalar::from(1_i64))])?;
    let mut destination = Vec::new();
    yaml::into_writer(&value, &mut destination)?;

    assert_eq!(destination, b"id: 1\n");
    ```

=== "Python"

    ```python
    import io
    import pathlib
    import tempfile

    from yggdryl import yaml

    target = pathlib.Path(tempfile.mkdtemp()) / "quote.yaml"
    yaml.dump({"quantity": 2, "symbol": "AAPL"}, target)
    assert target.read_bytes() == b"quantity: 2\nsymbol: AAPL\n"

    destination = io.BytesIO()
    yaml.dump({"id": 1}, destination)
    assert destination.getvalue() == b"id: 1\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { yaml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-yaml-'))
    const target = path.join(root, 'quote.yaml')

    // A destination string is a path, where a source string would be content.
    yaml.dump({ quantity: 2, symbol: 'AAPL' }, target)
    assert.equal(fs.readFileSync(target).toString(), 'quantity: 2\nsymbol: AAPL\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Many documents in one dump

The `_all` family writes a document set, `---` between documents, in the order the values arrive; the stream reads back as the values that went in.

=== "Rust"

    ```rust
    use yggdryl::yaml;

    let documents = yaml::from_utf8_all("id: 1\n---\nid: 2\n")?;
    let mut destination = Vec::new();
    yaml::into_writer_all(&documents, &mut destination)?;

    assert_eq!(destination, b"id: 1\n---\nid: 2\n");
    assert_eq!(yaml::from_bytes_all(&destination)?, documents);
    ```

=== "Python"

    ```python
    import io

    from yggdryl import yaml

    documents = [{"id": 1}, {"id": 2}]
    destination = io.BytesIO()
    yaml.dump_all(documents, destination)

    assert destination.getvalue() == b"id: 1\n---\nid: 2\n"
    assert yaml.dumps_all(documents) == b"id: 1\n---\nid: 2\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const documents = [{ id: 1 }, { id: 2 }]
    const encoded = yaml.dumpAll(documents)

    assert.equal(encoded.toString(), 'id: 1\n---\nid: 2\n')
    assert.deepEqual(yaml.loadsAll(encoded), documents)
    ```

## Block and flow layout

Layout changes bytes, never meaning. Two-space block style is the default, an explicit width keeps block style at that width, and `indent=None` / `{ indent: null }` selects flow style - `{a: 1, b: 2}` on one line, which is valid YAML and round-trips. The [shared `Formatting`](../structured.md#formatting) rules own the vocabulary and pin the exact bytes of all three formats.

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives YAML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `trade.yaml.gz` publishes without arguments: the coding goes back on after the writer.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///trade.yaml.gz")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_struct([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;
    handle.write_scalar(&value)?;

    // The bytes on the handle are gzip, not YAML.
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

    target = pathlib.Path(tempfile.mkdtemp()) / "trade.yaml.gz"
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

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-yaml-'))
    const handle = new IOBase(path.join(root, 'trade.yaml.gz'))
    handle.writeScalar({ quantity: 2, symbol: 'AAPL' })

    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual(handle.readScalar(), { quantity: 2, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

`write_arrow` is the one bridge from a batch to a document stream: one document per row, `---` separated. The frame is per document, so nothing but the batch being encoded is ever held.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.yaml")?.media_type());
    handle.write_all_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")?;
    let value = handle.read_arrow(None)?;

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert!(String::from_utf8(handle.read_all_bytes()?)?.contains("---"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "quotes.yaml"
    target.write_bytes(b"id: 1\nsymbol: AAPL\n---\nid: 2\nsymbol: MSFT\n")
    handle = IOBase(target)
    value = handle.read_arrow()

    # Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value.into_arrow_table())
    assert b"---" in handle.read_bytes()
    ```

## Edges

- an unsupported natural shape, or a failed Field conversion -> error, never silent coercion.
- `append` or `merge` -> refused naming the mode; the document set is written whole.
- `trade.yaml.gz` -> the coding goes back on after the writer.
- `indent` above sixteen spaces -> clamped by the core formatter.
- a resolved [placeholder](../placeholders.md) -> an ordinary value; dumps write it and never reintroduce the placeholder.
- `overwrite_arrow_reader` on a `.yaml` name -> refused; [YAML is not a record encoding](index.md#contract).
- JavaScript -> `write_arrow` is not bound; build the rows with Arrow JS and write them with [`yaml.dumps`](#use).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test yaml
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo bench -p yggdryl --bench text -- codec/yaml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/yaml/test_init.py
    python/.venv/bin/python -m pytest python/tests/test_arrow.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="yaml" node/tests/text/codec.test.js
    ```
