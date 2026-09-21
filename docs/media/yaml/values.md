# YAML values

What a bare YAML parse proves, and what a declared [`Field`](../../types/field.md) changes.

## Contract

| Key | Value |
| --- | --- |
| Proves | null, boolean, integer, float, string, sequence, mapping, and the standard `!!binary` tag |
| Records | string keys become a name-sorted `Record`; any other key kind an insertion-ordered `Mapping` |
| Custom tags | read by their natural shape, never as a private runtime class; no private tag is ever written |
| Exact types | `from_utf8_with_field` / `field=` / `{ field }` recovers `D128`, `Bytes`, `Geospatial` and the temporals from their natural spellings |
| Struct root | a Struct `Field` resolves record names into its child order: a row `Sequence` in Rust, a dictionary or object in the bindings |
| Order | parse, then [placeholder](../placeholders.md) substitution, then Field interpretation |
| Refuses | a malformed exact value, or a missing required Struct child: a Field conversion error naming YAML and the byte offset |

## Use

A schemaless read keeps only what the syntax proves: `~` is null, `true` a boolean, `1.5` a float, and a flow sequence a sequence.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::Scalar;

    let value = yaml::from_utf8("flag: true\nname: ~\nratio: 1.5\ntags: [a, b]\n")?;

    assert_eq!(value.get_key_str("flag"), Some(&Scalar::from(true)));
    assert_eq!(value.get_key_str("ratio"), Some(&Scalar::from(1.5_f64)));
    assert_eq!(value.get_key_str("tags").map(Scalar::len), Some(2));
    assert!(value.get_key_str("name").is_some_and(Scalar::is_null));
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    value = yaml.loads("flag: true\nname: ~\nratio: 1.5\ntags: [a, b]\n")

    assert value == {"flag": True, "name": None, "ratio": 1.5, "tags": ["a", "b"]}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = yaml.loads('flag: true\nname: ~\nratio: 1.5\ntags: [a, b]\n')

    assert.deepEqual(value, { flag: true, name: null, ratio: 1.5, tags: ['a', 'b'] })
    ```

## Natural values and exact Fields

Schemaless reads keep only syntax-proven types; a value with no native YAML syntax is written as the ordinary spelling below, and read back as that exact type only under a `Field`.

| native value | natural YAML |
| --- | --- |
| `D128`, `D256` | quoted scale-preserving string |
| `Bytes`, `Geospatial` | standard `!!binary` base64 |
| date, time, `DateTime64`, duration | ISO scalar when representable |
| `F16`, `F32`, `F64` | YAML float, including non-finite values |

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
    from yggdryl import yaml

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

## Standard `!!binary`

`!!binary` is a YAML standard tag, not a private marker envelope, so bytes round-trip through it and another YAML reader sees ordinary base64.

=== "Rust"

    ```rust
    use yggdryl::yaml;

    let value = yaml::from_utf8("payload: !!binary AP8=\n")?;

    assert_eq!(yaml::into_utf8(&value)?, "payload: !!binary \"AP8=\"\n");
    ```

=== "Python"

    ```python
    from yggdryl import yaml

    value = yaml.loads("payload: !!binary AP8=\n")

    assert value == {"payload": b"\x00\xff"}
    assert yaml.dumps(value) == b'payload: !!binary "AP8="\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = yaml.loads('payload: !!binary AP8=\n')

    assert.ok(Buffer.isBuffer(value.payload))
    assert.deepEqual([...value.payload], [0, 255])
    assert.equal(yaml.dumps(value).toString(), 'payload: !!binary "AP8="\n')
    ```

## A Struct Field names the row

A Struct `Field` resolves record names into its child order, whatever order the document wrote them in: the core answers a row `Sequence`, and the bindings project it back into a dictionary or an object under the field's own names.

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::{Field, Scalar};

    let row =
        Field::from_str("row: struct<symbol: utf8 not null, quantity: int32 not null> not null")?;
    let value = yaml::from_utf8_with_field("quantity: 2\nsymbol: AAPL\n", &row)?;

    // The core answers the ordered row the field names, not the document's order.
    assert_eq!(value.len(), 2);
    assert_eq!(value[0], Scalar::from("AAPL"));
    ```

=== "Python"

    ```python
    from yggdryl import Field, Scalar
    from yggdryl import yaml

    row = Field.from_str("row: struct<symbol: utf8 not null, quantity: int32 not null> not null")
    document = "quantity: 2\nsymbol: AAPL\n"

    assert yaml.loads(document, field=row) == {"quantity": 2, "symbol": "AAPL"}
    # The core value is the ordered row the field names, not the document's order.
    assert yaml.loads(document, field=row, cls=Scalar).as_py() == ["AAPL", 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, yaml } = require('yggdryl')

    const row = new Field('row', 'struct<symbol: utf8 not null, quantity: int32 not null>', false)
    const document = 'quantity: 2\nsymbol: AAPL\n'

    assert.deepEqual(yaml.loads(document, { field: row }), { quantity: 2, symbol: 'AAPL' })
    // The core value is the ordered row the field names, not the document's order.
    assert.deepEqual(yaml.loads(document, { field: row, scalar: true }).asJs(), ['AAPL', 2])
    ```

## Edges

- quoted `'AP8='` with no `Field` -> a string; base64 is a spelling, and only a `Field` declaring it binary makes it `Bytes`.
- a malformed exact value, or a missing required Struct child -> Field conversion error, never a silent coercion.
- a custom tag the reader does not know -> its natural shape, never a private runtime class.
- a [placeholder](../placeholders.md) -> substituted before the Field runs, so the resolved string becomes the exact typed value.
- empty or positional rows without a `Field` -> ambiguous; an explicit `Field` is required.
- a non-string key -> an insertion-ordered `Mapping`, not a sorted `Record`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text yaml::
    cargo test --features "parquet iceberg" -p yggdryl --test text value::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/yaml
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="yaml" node/tests/text/codec.test.js
    ```
