# JSON scalars

One JSON or JSON Lines document in and out of the shared [`Scalar`](../../types/scalar.md), held or streamed.

## Contract

| Key | Value |
| --- | --- |
| One document | `from_utf8`, `from_bytes`, `from_reader` in; `into_utf8`, `into_bytes`, `into_writer` out |
| Many | `_all` takes whitespace-separated values; `from_lines_*` and `JsonLines` take one per non-empty line |
| Lazy | `load_all` pulls a path or readable; `loads_all` decodes held content; iterators fuse after the first error |
| Returns | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Handle | `read_scalar` / `write_scalar` derive the format and any outer coding from the handle's `MediaType` |
| Layout | compact by default; `indent=n` spaces each nesting level, and neither changes the parsed value |

## Use

Reader iterators yield one `Result<Scalar>` at a time and writers stream to `Write`; Python and JavaScript keep `loads` / `dumps` and leave caller streams open.

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

Rust `Formatting::indented(n)` adds layout and `Formatting::compact()` removes it; neither changes the parsed value. JSON is compact by default, and `indent=n` / `{ indent: n }` indents each nesting level by `n` spaces; the [shared Formatting](../structured.md#formatting) example pins the exact bytes of all three formats.

## `IOBase` and content coding

[Structured-text I/O](../../holder/iobase/values.md) derives JSON and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quotes.json.gz` reads, writes, and publishes without arguments. Python accepts `PathLike`; JavaScript accepts path strings, file descriptors, file URLs, and streams.

## Edges

- trailing data after one document -> error at the boundary, with the byte offset.
- malformed JSON Lines row -> error at its offset in the original input, preceding lines included.
- a streamed iterator -> fused after the first error.
- `quotes.json.gz` -> the coding comes off before the parser and goes back on after the writer.

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
