# TOML scalars

One TOML document - always a string-key record - in and out of the shared [`Scalar`](../../types/scalar.md).

## Contract

| Key | Value |
| --- | --- |
| One document | `from_utf8`, `from_bytes`, `from_reader` in; `into_utf8`, `into_bytes`, `into_writer` out - exactly one, always a string-key record |
| Binding | `loads` takes content, paths, descriptors, file URLs, and readers; `dump` returns bytes or text, or writes directly |
| Returns | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Handle | `read_scalar` / `write_scalar` derive the format and any outer coding from the handle's `MediaType` |
| Layout | indentation for nested readability only; objects stay inline tables |

## Use

Rust `from_utf8`, `from_bytes`, `from_reader` decode one document; `into_utf8`, `into_bytes`, `into_writer` encode one. Binding `loads` takes content, paths, descriptors, file URLs, and readers; `dump` returns bytes or text or writes directly.

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

    from yggdryl.text import toml

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
    toml.dump({ id: 1 }, target)

    assert.deepEqual(toml.load(pathToFileURL(target)), { id: 1 })
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Formatting

`Formatting::indented(n)` lays array items out vertically and `Formatting::compact()` adds no layout; objects stay inline tables and only whitespace changes. The [shared Formatting](../structured.md#formatting) example pins the exact bytes of all three formats.

## `IOBase` and content coding

[Structured-text I/O](../../holder/iobase/values.md) derives the format and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so ``quotes.toml.gz`` reads, writes, and publishes without arguments. Python accepts `PathLike`; JavaScript accepts path strings, file descriptors, file URLs, and streams.

## Edges

- more than one document -> refused; TOML carries exactly one.
- a root that is not a string-key record -> refused; TOML has no other top-level shape.
- `quotes.toml.gz` -> the coding comes off before the parser and goes back on after the writer.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text toml::
    cargo test --features "parquet iceberg" -p yggdryl --lib toml::
    cargo bench -p yggdryl --bench text -- codec/toml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/toml
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/toml.test.js
    npm run --prefix node bench:text
    ```
