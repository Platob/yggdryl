# YAML

Owns the YAML codec: natural types, exact Fields, documents and streams, formatting, quoted placeholders, and limits.

## Contract

| Aspect | Contract |
| --- | --- |
| Owns | `yggdryl::yaml`; `yggdryl.text.yaml`; `yaml` from `yggdryl` |
| Natural types | null, boolean, integer, float, string, sequence, mapping, standard `!!binary` |
| Records | string keys: sorted `Record`; other keys: insertion-ordered `Mapping` |
| Exact tree | `cls=Scalar` (Python), `{ scalar: true }` (JavaScript); omitted: natural objects |
| One document | `from_utf8`, `from_bytes`, `from_reader`, `loads`, `dumps` |
| Stream | `*_all`, `into_writer_all`, `loads_all` / `dumps_all`, `loadsAll` / `dumpAll` |
| Lazy | `from_reader_iter[_with_field]`, `load_all`, `loadAll`; fused after the first error |
| Formatting | two-space block default; `Formatting::indented(n)` / `indent=n`; `Formatting::compact()` / `indent=None` / `{ indent: null }` is flow |
| Limits | `max_input_bytes` / `maxInputBytes`, depth, decoded nodes, documents; held input and streams alike; omitted means core defaults |
| Errors | invalid UTF-8, duplicate keys, malformed syntax, exhaustion, Field conversion: YAML plus byte offset |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | `loads` / `dumps`, document streams, block or flow layout, `read_scalar` / `write_scalar` over a handle |
| [Arrow](arrow.md) | the document stream as Arrow rows, one document per row |

The format-agnostic facade, limits, and `Format` vocabulary are on [Structured documents](../structured.md).

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
    from yggdryl.text import yaml

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

## One inferring entry point

`yggdryl::from_yaml_scalar`, `from_yaml_scalar_with_field`, and `into_yaml_scalar` are YAML's crate-root [inferring entry points](../structured.md#raw-document-codecs) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`; the [Use](#use) example shows them answering what the explicit form answers. The bindings' `loads` and `dumps` are that entry.

## Natural values and exact Fields

Schemaless reads keep only syntax-proven types; unknown custom tags read by their natural shape, never as private runtime classes.

| native value | natural YAML |
| --- | --- |
| `D128`, `D256` | quoted scale-preserving string |
| `Bytes`, `Geospatial` | standard `!!binary` base64 |
| date, time, `DateTime64`, duration | ISO scalar when representable |
| `F16`, `F32`, `F64` | YAML float, including non-finite values |

`!!binary` is a YAML standard tag, not a private marker envelope; no private tag is written.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::yaml;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = yaml::from_utf8_with_field("'12.50'\n", &amount)?;

    assert_eq!(decoded, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field
    from yggdryl.text import yaml

    amount = Field("amount", "decimal128(8, 2)", nullable=False)

    assert yaml.loads("'12.50'\n", field=amount) == Decimal("12.50")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, yaml } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = yaml.loads("'12.50'\n", { field: amount })

    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.scale, 2)
    ```

A Struct Field resolves record names into its child order: a row `Sequence` in Rust, a dictionary or object in Python and JavaScript.

## Placeholders

Substitution is opt-in, runs after parsing, and touches string values only, never keys or structure; a quoted `"{{ PORT }}"` becomes the variable's own typed value. Quote placeholders, since unquoted braces are YAML flow-mapping syntax: `port: {{ PORT }}` parses as a mapping before substitution runs. The YAML example, syntax, security, and measured overhead live on [Placeholders](../placeholders.md).

## Edges

- A second document through a one-document form -> error; use the stream forms.
- Stream failure -> the error names the document start and the failing byte offset; the iterator is then exhausted.
- Quoted `"AP8="` -> a string unless a [`Field`](../../types/field.md) declares it binary.
- Malformed exact value or missing required Struct child -> Field conversion error.
- Text naming an existing file -> that plain string scalar, not the file's content.
- Unquoted `port: {{ PORT }}` -> a flow mapping, not a placeholder.
- Placeholder sources -> the supplied mapping wins; environment lookup is a separate switch, off by default.
- Placeholder under a Field -> interpretation runs after substitution, so the resolved string becomes the exact typed value.
- `.yaml.gz` handle -> `from_io` / `into_io` infer YAML and the outer coding; Python takes `PathLike`, JavaScript paths, descriptors, file URLs, streams.
- Line-record media -> use [Text records](../text/index.md) for Arrow batches with overwrite and append; keyed merge is refused.

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
