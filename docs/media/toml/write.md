# Writing TOML

One native [`Scalar`](../../types/scalar.md) out - as a TOML document first, then as Arrow rows.

## Contract

| Key | Value |
| --- | --- |
| Value out | `into_utf8`, `into_bytes`, `into_writer`; Python `dumps` / `dump`, JavaScript `dumps` / `dump` |
| Location out | a destination string is a path in both bindings - the asymmetry a source string does not have; a writable stream is written to and left open |
| Root | exactly one string-key record; names are sorted, keys are quoted, and repeated writes are byte-identical |
| Preflight | `validate_for_write` checks the natural projection before a destination opens, so nothing partial is left behind |
| One document | Rust `into_*_all` takes exactly one value and refuses zero or two; the bindings expose no `dump_all` |
| Layout | `Formatting::indented(n)` / `indent=n` / `{ indent: n }` lays array items out vertically; objects stay inline tables and only whitespace changes |
| Handle | `write_scalar` derives TOML and any outer [coding](../../coding/index.md) from the handle's `MediaType` |
| Arrow | `write_arrow(value, mode)` writes one array of tables under the root's name, each row an inline table |
| Mode | overwrite only: a document is one frame around its rows, so it is written whole |
| Held | the rows the one frame encloses |
| Bindings | all three for the scalar surface; `write_arrow` is Rust and Python only |

## Use

The smallest write: one record in, one document out. Nothing in the call names a layout, because names are sorted and keys are quoted by default.

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::Scalar;

    let value = Scalar::from_struct([
        ("count", Scalar::from(3_i64)),
        ("title", Scalar::from("yggdryl")),
    ])?;

    assert_eq!(toml::into_utf8(&value)?, "\"count\" = 3\n\"title\" = \"yggdryl\"\n");
    assert_eq!(toml::from_utf8(&toml::into_utf8(&value)?)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import toml

    value = {"count": 3, "title": "yggdryl"}

    assert toml.dumps(value) == b'"count" = 3\n"title" = "yggdryl"\n'
    assert toml.dump(value, utf8=True) == '"count" = 3\n"title" = "yggdryl"\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const encoded = toml.dumps({ count: 3, title: 'yggdryl' })

    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '"count" = 3\n"title" = "yggdryl"\n')
    ```

## A document to a location

Rust writes to any `Write`; Python and JavaScript take a path or a writable destination, and a caller's stream is written to and left open.

=== "Rust"

    ```rust
    use yggdryl::toml;

    let value = toml::from_utf8("id = 1\n")?;
    let mut destination = Vec::new();
    toml::into_writer(&value, &mut destination)?;

    assert_eq!(toml::from_bytes(&destination)?, value);
    ```

=== "Python"

    ```python
    import io
    import pathlib
    import tempfile

    from yggdryl import toml

    target = pathlib.Path(tempfile.mkdtemp()) / "quote.toml"
    toml.dump({"id": 1}, target)
    assert target.read_bytes() == b'"id" = 1\n'

    destination = io.BytesIO()
    toml.dump({"id": 1}, destination)
    assert toml.loads(destination.getvalue()) == {"id": 1}
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
    const target = path.join(root, 'value.toml')

    // A destination string is a path, where a source string would be content.
    toml.dump({ id: 1 }, target)

    assert.deepEqual(toml.load(pathToFileURL(target)), { id: 1 })
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Through a handle

[Structured-text I/O](../../holder/iobase/values.md) derives TOML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `trade.toml.gz` publishes without arguments: the coding goes back on after the writer.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar, Url};

    let media = Url::from_str("file:///trade.toml.gz")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_struct([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;
    handle.write_scalar(&value)?;

    // The bytes on the handle are gzip, not TOML.
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

    target = pathlib.Path(tempfile.mkdtemp()) / "trade.toml.gz"
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

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-'))
    const handle = new IOBase(path.join(root, 'trade.toml.gz'))
    handle.writeScalar({ quantity: 2, symbol: 'AAPL' })

    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual(handle.readScalar(), { quantity: 2, symbol: 'AAPL' })

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Refused before a destination opens

`validate_for_write` walks the natural projection first, so a value TOML cannot spell never reaches a file: null, a scalar or sequence root, a non-string key, or an integer outside `i64` is an error at the boundary rather than a truncated document.

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::Scalar;

    // A root that is not a table, and a null leaf, are both refused.
    assert!(toml::into_bytes(&Scalar::from(1)).is_err());
    assert!(toml::into_bytes(&Scalar::from_struct([("missing", Scalar::Null)])?).is_err());

    // The check is available on its own, before a destination is opened.
    let refused = toml::validate_for_write(&Scalar::from_struct([("missing", Scalar::Null)])?)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("null"), "{refused}");
    ```

=== "Python"

    ```python
    from yggdryl import toml

    for value in (None, 42, [1, 2], {1: "one"}):
        try:
            toml.dumps(value)
        except ValueError as error:
            assert "root must be a record" in str(error) or "keys must be strings" in str(error)
        else:
            raise AssertionError(f"{value!r} is not a TOML document root")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    for (const root of [null, 'scalar root', [1, 2], Buffer.from([1, 2])]) {
      assert.throws(() => toml.dumps(root), /root must be a record/i)
    }
    assert.throws(() => toml.dumps({ missing: undefined }), /cannot represent null/i)
    assert.throws(() => toml.dumps({ bigint: 2n ** 100n }), /exceeds i64/i)
    ```

## Formatting

`Formatting::indented(n)` lays array items out vertically and `Formatting::compact()` adds no layout; objects stay inline tables and only whitespace changes. The [shared `Formatting`](../structured.md#formatting) rules own the vocabulary and pin the exact bytes of all three formats.

## Rows as Arrow batches

`write_arrow` is the one bridge from a batch to a document. TOML has no top-level sequence, so a write puts the rows back under the root's name, each row an inline table, and the whole document is written at once.

Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{IOBase, IOMedia, IOMode, Url};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///quotes.toml")?.media_type());
    handle.write_all_bytes(
        b"[[row]]\nid = 1\nsymbol = \"AAPL\"\n\n[[row]]\nid = 2\nsymbol = \"MSFT\"\n",
    )?;
    let value = handle.read_arrow(None)?;

    // Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value, IOMode::Overwrite, None)?;
    assert!(String::from_utf8(handle.read_all_bytes()?)?.starts_with("\"row\" = ["));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "quotes.toml"
    target.write_bytes(b'[[row]]\nid = 1\nsymbol = "AAPL"\n\n[[row]]\nid = 2\nsymbol = "MSFT"\n')
    handle = IOBase(target)
    value = handle.read_arrow()

    # Written back, the rows are framed the way this format frames them.
    handle.write_arrow(value.into_arrow_table())
    assert handle.read_bytes().startswith(b'"row" = [')
    ```

## Edges

- null, a non-record root, a non-string key, or an integer outside `i64` -> `validate_for_write` error, no partial destination.
- more than one value to `into_writer_all`, or none -> refused; TOML carries exactly one document.
- `append` or `merge` -> refused naming the mode; a document is written whole.
- `trade.toml.gz` -> the coding goes back on after the writer.
- a user key spelled like a private marker -> written as ordinary data.
- a resolved [placeholder](../placeholders.md) -> an ordinary value; a dump never reintroduces the `{{ }}` spelling.
- `overwrite_arrow_reader` on a `.toml` name -> refused; [TOML is not a record encoding](index.md#contract).
- JavaScript -> `write_arrow` is not bound; build the rows with Arrow JS and write them with [`toml.dumps`](#use).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test toml -- mod_
    cargo test --features "parquet iceberg" -p yggdryl --test media -- structured::
    cargo bench -p yggdryl --bench text -- codec/toml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/toml/test_init.py
    python/.venv/bin/python -m pytest python/tests/test_arrow.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    ```
