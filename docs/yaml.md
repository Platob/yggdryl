# YAML

YAML supports one document or a document stream, using ordinary YAML without
private Yggdryl tags.

## Text media and Arrow batches

A YAML document stream carries raw structured values, not line-record media.
For streamed Arrow batches with explicit overwrite and append behavior, use
[Text media](text.md#text-media-and-arrow-batches). Text deliberately refuses
keyed merge because a line has no stable row identity.

## Raw shared-Scalar access

Rust returns the shared `Scalar`; Python and JavaScript project it into native
objects through the same codec.

=== "Rust"

    ```rust
    use yggdryl::{yaml, Scalar};

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(yaml::into_utf8(&value)?, "quantity: 2\nsymbol: AAPL\n");
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, yaml

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

Mappings with string names become sorted `Record` values. YAML mappings with
other keys remain `Mapping` and preserve insertion order. Duplicate keys are
rejected. Python `cls=Scalar` and JavaScript `{ scalar: true }` return the exact
native tree; omitting the selector keeps natural language objects.

## Natural values and exact Fields

Schemaless reads preserve only types proven by YAML syntax: null, boolean,
integer, float, string, sequence, mapping, and standard binary. Unknown custom
tags do not create private runtime classes; the tagged scalar or collection is
read by its natural shape.

Exact native values dump as interoperable YAML:

| native value | natural YAML |
| --- | --- |
| `D128`, `D256` | quoted scale-preserving string |
| `Bytes`, `Geospatial` | standard `!!binary` base64 |
| date, time, `DateTime64`, duration | ISO scalar when representable |
| `F16`, `F32`, `F64` | YAML float, including non-finite values |

`!!binary` is a YAML standard tag, not a private marker envelope. No private
tag is written. A plain quoted `"AP8="` therefore stays a string unless a
[`Field`](field.md) declares it binary.

=== "Rust"

    ```rust
    use yggdryl::{yaml, DataType, Field, Scalar};

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = yaml::from_utf8_with_field("'12.50'\n", &amount)?;

    assert_eq!(decoded, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, yaml

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

A Struct Field resolves record names into its child order and returns a row
`Sequence` in Rust; Python and JavaScript restore a dictionary or object at
their boundary. Malformed exact values and missing required children fail.

## Documents and streams

Rust `from_utf8`, `from_bytes`, and `from_reader` require exactly one YAML
document. Their `_all` forms return every document, and
`from_reader_iter[_with_field]` yields lazily and fuses after the first error.
`into_writer_all` emits a YAML document stream.

Python and JavaScript use `loads` / `dumps` for one document and
`loads_all` / `dump_all` for a stream. Python `load_all` and JavaScript
`loadAll` keep readable streams lazy.

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

    from yggdryl import yaml

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

Each error carries both the document start and the failing byte offset in the
whole source. After failure, the iterator is exhausted.

## Formatting

YAML defaults to two-space block style. `Formatting::indented(n)` changes the
block width; `Formatting::compact()` selects flow style. Layout changes bytes,
never meaning.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{yaml, Scalar};

    let value = Scalar::from_record([("id", Scalar::I64(1))])?;
    let flow =
        yaml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_eq!(flow, "{id: 1}\n");
    assert_eq!(yaml::from_utf8(&flow)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import yaml

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

The writer quotes scalars when their plain spelling would change type or
structure. Deterministic Record order makes repeated dumps byte-identical.

## Placeholders

Placeholder substitution is opt-in and runs after YAML parsing. It changes
string values only, never keys or document structure. Quote placeholders:
unquoted braces are YAML flow-mapping syntax.

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new().with_variable("PORT", Scalar::I64(8080)),
    );
    let value = yggdryl::text::from_utf8_with(
        "port: \"{{ PORT }}\"\n",
        Format::Yaml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("port"), Some(&Scalar::I64(8080)));
    ```

=== "Python"

    ```python
    from yggdryl import yaml

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

The supplied mapping wins over the environment. Environment lookup is a
separate switch and remains off by default. Field interpretation runs after
substitution, so a resolved natural string can become an exact typed value.
See [structured text](text.md#jinja-style-placeholders) for syntax, security,
and measured overhead.

## Limits and errors

Nullable binding options expose the same byte, depth, decoded-node, and
document limits as Rust (`max_input_bytes` / `maxInputBytes`, and the matching
names for the other three). They apply equally to held input and streams;
omission uses the core defaults. Invalid UTF-8, duplicate keys, malformed
syntax, exhaustion, and Field conversion report YAML plus a byte offset.

## `IOBase` and content coding

Generic `from_io` / `into_io` infer YAML and outer compression from the
handle's media type, so `.yaml.gz` is one transparent operation. Python accepts
`PathLike` sources and destinations; JavaScript accepts paths, file
descriptors, file URLs, and streams.
