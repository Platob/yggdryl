# YAML scalars

One YAML document, or a stream of them, in and out of the shared [`Scalar`](../../types/scalar.md).

## Contract

| Key | Value |
| --- | --- |
| One document | `from_utf8`, `from_bytes`, `from_reader`, `loads`, `dumps` |
| Many | `*_all`, `into_writer_all`, `loads_all` / `dumps_all`, `loadsAll` / `dumpAll` |
| Lazy | `from_reader_iter[_with_field]`, `load_all`, `loadAll`; fused after the first error |
| Returns | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| Handle | `read_scalar` / `write_scalar` derive the format and any outer coding from the handle's `MediaType` |
| Layout | two-space block by default; `indent=None` / `{ indent: null }` is flow style |

## Use

Python `load_all` and JavaScript `loadAll` keep readable streams lazy.

=== "Rust"

    ```rust
    use yggdryl::yaml;

    let documents = yaml::from_utf8_all("id: 1\n---\nid: 2\n")?;
    let mut destination = Vec::new();
    yaml::into_writer_all(&documents, &mut destination)?;

    assert_eq!(documents.len(), 2);
    assert_eq!(yaml::from_bytes_all(&destination)?, documents);
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import yaml

    documents = list(yaml.load_all(io.BytesIO(b"id: 1\n---\nid: 2\n")))

    assert documents == [{"id": 1}, {"id": 2}]
    assert yaml.dumps_all(documents) == b"id: 1\n---\nid: 2\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const documents = yaml.loadsAll('id: 1\n---\nid: 2\n')
    const encoded = yaml.dumpAll(documents)

    assert.deepEqual(documents, [{ id: 1 }, { id: 2 }])
    assert.deepEqual(yaml.loadsAll(encoded), documents)
    ```

## Formatting

Layout changes bytes, never meaning; the writer quotes scalars whose plain spelling would change type or structure, and deterministic `Record` order makes repeated dumps byte-identical. Two-space block style is the default and `indent=None` / `{ indent: null }` selects flow style; the [shared Formatting](../structured.md#formatting) example pins the exact bytes of all three formats.

## `IOBase` and content coding

[Structured-text I/O](../../holder/iobase/values.md) derives the format and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so ``quotes.yaml.gz`` reads, writes, and publishes without arguments. Python accepts `PathLike`; JavaScript accepts path strings, file descriptors, file URLs, and streams.

## Edges

- a second document through a one-document form -> error; use the stream forms.
- stream failure -> the error names the document start and the failing byte offset; the iterator is then exhausted.
- `quotes.yaml.gz` -> the coding comes off before the parser and goes back on after the writer.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text yaml::
    cargo bench -p yggdryl --bench text -- codec/yaml
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/yaml
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    node --test --test-name-pattern="yaml" node/tests/text/codec.test.js
    npm run --prefix node bench:text
    ```
