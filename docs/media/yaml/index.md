# YAML

YAML documents and document streams as one scheme over the shared [`Scalar`](../../types/scalar.md) codec.

## Contract

| Aspect | Contract |
| --- | --- |
| Owns | `yggdryl::yaml`; `yggdryl.yaml`; `yaml` from `yggdryl` |
| Proves | null, boolean, integer, float, string, sequence, mapping, the standard `!!binary` tag; anything else needs a [`Field`](values.md) |
| Records | string keys: sorted `Record`; other keys: insertion-ordered `Mapping` |
| Bindings | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural objects |
| One document | `from_utf8`, `from_bytes`, `from_reader` in; `into_utf8`, `into_bytes`, `into_writer` out; `loads` / `dumps` in both bindings |
| Streams | `*_all` and `into_writer_all`; `loads_all` / `dumps_all`, `loadsAll` / `dumpAll` - one value per `---` separated document |
| Lazy | `from_reader_iter[_with_field]`, Python `load_all`, JavaScript `loadAll`; fused after the first error |
| Handle | `read_scalar` / `write_scalar`, and the generic `from_io` / `into_io` facade, derive YAML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quotes.yaml.gz` needs no argument |
| Arrow | `read_arrow` and `write_arrow` are the one bridge to rows, one document per row; [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Formatting | two-space block style by default; `Formatting::indented(n)` / `indent=n` / `{ indent: n }` keeps block style at that width; `Formatting::compact()` / `indent=None` / `{ indent: null }` is flow |
| Placeholders | quoted `{{ }}` substitution, off by default, string values only |
| Limits | `max_input_bytes` / `maxInputBytes`, depth, decoded nodes, documents; held input and streams alike; omitted means core defaults |
| Errors | invalid UTF-8, duplicate keys, malformed syntax, exhaustion, Field conversion: YAML plus byte offset |

## Use

Rust returns the shared `Scalar`; Python and JavaScript project it into native objects through the same codec.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::{from_yaml_scalar, into_yaml_scalar, Scalar};

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;
    let encoded = yaml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    assert_eq!(encoded, "quantity: 2\nsymbol: AAPL\n");

    // The crate-root inferring entry points answer the same value, from text or bytes.
    assert_eq!(from_yaml_scalar("symbol: AAPL\nquantity: 2\n")?, value);
    assert_eq!(into_yaml_scalar(&value)?, encoded);
    assert_eq!(from_yaml_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import yaml

    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)
    encoded = yaml.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {"quantity": 2, "symbol": "AAPL"}
    assert encoded == b"quantity: 2\nsymbol: AAPL\n"
    assert yaml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')
    assert.deepEqual(yaml.loads(encoded), natural)
    assert.ok(yaml.loads(encoded, { scalar: true }).equals(value))
    ```

## Pages

| Page | Owns |
| --- | --- |
| [Read](read.md) | a document as a native scalar, from content, a location or a handle; multi-document loads; then rows as Arrow batches |
| [Write](write.md) | a scalar as a document, to bytes, a location or a handle; multi-document dumps; block and flow layout; then Arrow batches |
| [Values](values.md) | what a bare parse answers, and what a declared `Field` changes |

Every scheme answers the same two surfaces - native scalars and Arrow rows. YAML answers native scalars in all three bindings, and Arrow rows in Rust and Python only - JavaScript binds neither `read_arrow` nor `write_arrow`. The format-agnostic facade, the `Format` and `Limits` vocabulary, and the shared `Formatting` rules are on [Structured documents](../structured.md).

## One inferring entry point

`yggdryl::from_yaml_scalar`, `from_yaml_scalar_with_field`, and `into_yaml_scalar` are YAML's crate-root [inferring entry points](../structured.md#raw-document-codecs) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

## Placeholders

Substitution is opt-in, runs after parsing, and touches string values only, never keys or structure; the contract, the syntax, the security note, and the measured overhead are on [Placeholders](../placeholders.md).

## Edges

- unquoted `port: {{ PORT }}` -> a flow mapping, not a placeholder; braces the grammar reads structurally are parsed before substitution runs, so quote the placeholder.
- placeholder options on [JSON](../json/index.md) -> refused; only YAML and [TOML](../toml/index.md) substitute.
- depth above `yaml::MAX_PARSER_DEPTH` -> refused, whatever a caller's limit asks; `[` and `{` flow syntax is bounded first, by `yaml::MAX_FLOW_DEPTH`.
- `_with_limits` / `_with_formatting` -> explicit form only; the inferring entry point has neither.
- streamed Arrow batches with overwrite, append, and a schema -> [Text records](../text/index.md); keyed merge is refused there, since a line has no row identity.

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
    npm run --prefix node bench:text
    ```
