# YAML

Owns the YAML codec: natural types, exact Fields, documents and streams, formatting, quoted placeholders, and limits.

## Contract

| Aspect | Contract |
| --- | --- |
| Owns | `yggdryl::text::yaml`; `yggdryl.text.yaml`; `yaml` from `yggdryl` |
| Natural types | null, boolean, integer, float, string, sequence, mapping, standard `!!binary` |
| Records | string keys: sorted `Record`; other keys: insertion-ordered `Mapping` |
| Exact tree | `cls=Scalar` (Python), `{ scalar: true }` (JavaScript); omitted: natural objects |
| One document | `from_utf8`, `from_bytes`, `from_reader`, `loads`, `dumps` |
| Stream | `*_all`, `into_writer_all`, `loads_all` / `dumps_all`, `loadsAll` / `dumpAll` |
| Lazy | `from_reader_iter[_with_field]`, `load_all`, `loadAll`; fused after the first error |
| Formatting | two-space block default; `Formatting::indented(n)` / `indent=n`; `Formatting::compact()` / `indent=None` / `{ indent: null }` is flow |
| Limits | `max_input_bytes` / `maxInputBytes`, depth, decoded nodes, documents; held input and streams alike; omitted means core defaults |
| Errors | invalid UTF-8, duplicate keys, malformed syntax, exhaustion, Field conversion: YAML plus byte offset |

## Use

Rust returns the shared `Scalar`; Python and JavaScript project it into native objects through the same codec.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::yaml;

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(yaml::into_utf8(&value)?, "quantity: 2\nsymbol: AAPL\n");
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import yaml

    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {"quantity": 2, "symbol": "AAPL"}
    assert yaml.dumps(value) == b"quantity: 2\nsymbol: AAPL\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(yaml.loads(encoded), natural)
    ```

## One inferring entry point

`yggdryl::from_yaml_scalar`, `from_yaml_scalar_with_field`, and `into_yaml_scalar` are YAML's crate-root [inferring entry points](index.md) over `from_bytes`, `from_bytes_with_field`, and `into_utf8`.

=== "Rust"

    ```rust
    use yggdryl::{from_yaml_scalar, into_yaml_scalar};

    let value = from_yaml_scalar("symbol: AAPL\nquantity: 2\n")?;
    let encoded = into_yaml_scalar(&value)?;

    assert_eq!(encoded, "quantity: 2\nsymbol: AAPL\n");
    assert_eq!(from_yaml_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import yaml

    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)
    encoded = yaml.dumps(value)

    assert encoded == b"quantity: 2\nsymbol: AAPL\n"
    assert yaml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')
    assert.ok(yaml.loads(encoded, { scalar: true }).equals(value))
    ```

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
    use yggdryl::text::yaml;

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

## Documents and streams

Python `load_all` and JavaScript `loadAll` keep readable streams lazy.

=== "Rust"

    ```rust
    use yggdryl::text::yaml;

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

Layout changes bytes, never meaning; the writer quotes scalars whose plain spelling would change type or structure. Deterministic Record order makes repeated dumps byte-identical.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::yaml;

    let value = Scalar::from_record([("id", Scalar::from(1_i64))])?;
    let flow =
        yaml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_eq!(flow, "{id: 1}\n");
    assert_eq!(yaml::from_utf8(&flow)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    value = {"child": {"id": 1}}
    laid_out = yaml.dumps(value, indent=4)
    flow = yaml.dumps(value, indent=None)

    assert b"\n    id:" in laid_out
    assert flow.startswith(b"{")
    assert yaml.loads(laid_out) == yaml.loads(flow) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = { child: { id: 1 } }
    const laidOut = yaml.dumps(value, { indent: 4 })
    const flow = yaml.dumps(value, { indent: null })

    assert.ok(laidOut.includes(Buffer.from('\n    id:')))
    assert.equal(flow[0], '{'.charCodeAt(0))
    assert.deepEqual(yaml.loads(laidOut), yaml.loads(flow))
    ```

## Placeholders

Substitution is opt-in, runs after parsing, and touches string values only, never keys or structure. Quote placeholders, since unquoted braces are YAML flow-mapping syntax.

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new().with_variable("PORT", Scalar::from(8080_i64)),
    );
    let value = yggdryl::text::from_utf8_with(
        "port: \"{{ PORT }}\"\n",
        Format::Yaml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(8080_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    options = {"placeholders": {"PORT": 8080}}

    assert yaml.loads('port: "{{ PORT }}"\n', **options) == {"port": 8080}
    assert isinstance(yaml.loads("port: {{ PORT }}\n", **options)["port"], dict)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const options = { placeholders: { PORT: 8080 } }

    assert.deepEqual(yaml.loads('port: "{{ PORT }}"\n', options), { port: 8080 })
    ```

Syntax, security, and measured overhead live on [Placeholders](placeholders.md).

## Edges

- A second document through a one-document form -> error; use the stream forms.
- Stream failure -> the error names the document start and the failing byte offset; the iterator is then exhausted.
- Quoted `"AP8="` -> a string unless a [`Field`](../types/field.md) declares it binary.
- Malformed exact value or missing required Struct child -> Field conversion error.
- Text naming an existing file -> that plain string scalar, not the file's content.
- Unquoted `port: {{ PORT }}` -> a flow mapping, not a placeholder.
- Placeholder sources -> the supplied mapping wins; environment lookup is a separate switch, off by default.
- Placeholder under a Field -> interpretation runs after substitution, so the resolved string becomes the exact typed value.
- `.yaml.gz` handle -> `from_io` / `into_io` infer YAML and the outer coding; Python takes `PathLike`, JavaScript paths, descriptors, file URLs, streams.
- Line-record media -> use [Text records](../media/text.md) for Arrow batches with overwrite and append; keyed merge is refused.

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
