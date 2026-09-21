# TOML

One natural record document over the shared [`Scalar`](../../types/scalar.md) codec.

## Contract

| facet | contract |
| --- | --- |
| Root | exactly one table, read and written as a name-sorted `Record`; repeated writes are byte-identical |
| Proves | strings, `i64`, `f64`, booleans, arrays, tables, and the four date/time forms; anything else needs a [`Field`](values.md) |
| Lacks | null, non-string keys, a scalar root, several documents, streams, private markers |
| Reads | `from_utf8`, `from_bytes`, `from_reader` with their `_with_field`, `_with_limits` and `_all` siblings |
| Writes | `into_utf8`, `into_bytes`, `into_writer` with `_with_formatting`; `validate_for_write` refuses before a destination opens |
| Bindings | Rust `Scalar`; Python `cls=Scalar` / JavaScript `{ scalar: true }` return it, else natural mappings |
| Exact types | `_with_field` / `field=` / `{ field }` recovers `D128`, `D256`, `Bytes`, durations and string temporals from quoted strings |
| Handle | `read_scalar` / `write_scalar` derive TOML and any outer [coding](../../coding/index.md) from the handle's `MediaType`, so `quotes.toml.gz` needs no argument |
| Arrow | `read_arrow` and `write_arrow` are the one bridge to rows; [`RecordOptions`](../options.md) names no structured format, so `read_arrow_reader` and the three write intents refuse the name |
| Placeholders | opt-in `{{ }}` substitution inside quoted strings, at any table depth |
| Limits | nullable input bytes, depth, decoded nodes and document count; snake_case in Python and camelCase in JavaScript, core defaults when omitted |
| Errors | name TOML and a byte offset |

## Use

Rust returns `Scalar`; bindings redirect native mappings through the same codec.

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::{from_toml_scalar, into_toml_scalar, Scalar};

    let source = "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n";
    let value = toml::from_utf8(source)?;
    let encoded = toml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_str),
        Some("yggdryl")
    );
    assert_eq!(toml::from_utf8(&encoded)?, value);

    // The crate-root inferring entry points answer the same Record, from text or bytes.
    assert_eq!(from_toml_scalar(source)?, value);
    assert_eq!(from_toml_scalar(into_toml_scalar(&value)?.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)
    encoded = toml.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {
        "count": 3,
        "owner": {"name": "Ada"},
        "title": "yggdryl",
    }
    assert toml.loads(encoded) == natural
    assert toml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const natural = toml.loads(source)
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(toml.loads(encoded), natural)
    assert.ok(toml.loads(encoded, { scalar: true }).equals(value))
    ```

## Pages

| Page | Owns |
| --- | --- |
| [Read](read.md) | a document as a native scalar, from content, a location or a handle; then rows as Arrow batches |
| [Write](write.md) | a scalar as a document, to bytes, a location or a handle; formatting; then Arrow batches |
| [Values](values.md) | what a bare parse answers, what a declared `Field` changes, and the four date/time forms |

Every scheme answers the same two surfaces. TOML answers native scalars in all three bindings, and Arrow rows in Rust and Python only - JavaScript binds neither `read_arrow` nor `write_arrow`. The format-agnostic facade, the `Format` and `Limits` vocabulary, and the shared `Formatting` rules are on [Structured documents](../structured.md).

## Inferring entry point

`from_toml_scalar`, `from_toml_scalar_with_field`, and `into_toml_scalar` are TOML's [inferring entry points](../structured.md#raw-document-codecs) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`, answering the root `Record`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

## Placeholders

TOML substitutes opt-in Jinja-style `{{ }}` variables inside quoted strings, at any table depth, after parsing and before Field interpretation: [Placeholders](../placeholders.md) owns the contract and shows a TOML table taking a string and an integer variable.

## Edges

- empty or comment-only document -> empty `Record`.
- null, non-record root, non-string key, `i64` overflow, excessive depth -> `validate_for_write` error, no partial destination.
- an existing file name as text -> parsed as TOML; fails as a bare word.
- Rust `_all` forms -> exactly one value; the bindings expose no `loads_all` or `dump_all`, and JavaScript's `loadStream` / `dumpStream` carry that one document, never several.
- a user key spelled like a private marker -> ordinary data.
- placeholder mapping -> wins over the process environment, which is never read unless enabled.
- streamed Arrow batches -> [Text records](../text/index.md), which refuse keyed merge.

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
