# Natural values and exact Fields

What a bare JSON parse answers, and what a declared [`Field`](../../types/field.md) changes about it.

## Contract

| Key | Value |
| --- | --- |
| Proves | null, booleans, finite numbers, strings, arrays, string-key objects; anything else needs a `Field` |
| Loads | objects to a name-sorted record, arrays to a `Sequence`, numbers to the narrowest exact family |
| Natural out | a value with no JSON syntax of its own travels as a scale-preserving string, base64, or ISO text |
| Exact in | `from_*_with_field` / `field=` / `{ field }` types the natural value, orders records, and validates |
| Order | parse, then `Field` interpretation; JSON substitutes no [placeholders](../placeholders.md) |
| Root | a Struct `Field` yields one ordered row `Sequence` in Rust, a dictionary or object elsewhere |
| Refuses | duplicate object names, non-finite floats, non-string mapping keys, a failed `Field` conversion |

## Use

A schemaless read answers only what the grammar proves, so an exact decimal spelled as text stays a string until a `Field` names it.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::Scalar;

    let value = json::from_utf8(r#"{"amount":"12.50","quantity":100}"#)?;

    assert_eq!(
        value.get_key_str("amount").and_then(Scalar::as_str),
        Some("12.50")
    );
    assert_eq!(json::into_utf8(&value)?, r#"{"amount":"12.50","quantity":100}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import json

    value = json.loads('{"amount":"12.50","quantity":100}', cls=Scalar)

    assert value["amount"].kind == "string"
    assert value["quantity"].kind == "u64"
    assert json.loads('{"amount":"12.50","quantity":100}') == {
        "amount": "12.50",
        "quantity": 100,
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = json.loads('{"amount":"12.50","quantity":100}', { scalar: true })

    assert.equal(value.get('amount').kind, 'string')
    assert.equal(value.get('quantity').kind, 'u64')
    assert.deepEqual(json.loads('{"amount":"12.50","quantity":100}'), {
      amount: '12.50',
      quantity: 100,
    })
    ```

## Natural spellings

Other native values use interoperable spellings, without a private marker envelope:

| native value | natural JSON |
| --- | --- |
| `D128`, `D256` | scale-preserving string |
| `Bytes`, `Geospatial` | base64 string |
| date, time, `DateTime64`, duration | ISO string when representable |
| non-finite float | error |
| Mapping with non-string keys | error |

## Exact Fields

A schemaless reader sees strings; pass a native [`Field`](../../types/field.md) to recover exact types. A string Field puts its leaf - charset, shape and width - on the value it reads and checks its bound, naming the bytes it counted; a byte Field reads base64 and holds the payload to its width or maximum the same way.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::json;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    assert_eq!(json::from_utf8_with_field(r#""12.50""#, &amount)?, Scalar::d128(1_250, 2));

    let symbol = Field::new("symbol", DataType::fixed_ascii(4)?, false);
    let held = json::from_utf8_with_field(r#""AAPL""#, &symbol)?;

    assert_eq!(held.dtype()?, DataType::fixed_ascii(4)?);
    assert_eq!(held.as_str(), Some("AAPL"));
    let refused = json::from_utf8_with_field(r#""AAPLE""#, &symbol).unwrap_err().to_string();
    assert!(refused.contains("expected at most 4 bytes of us-ascii, got 5"), "{refused}");

    let key = Field::new("key", DataType::fixed_binary(2)?, false);
    assert_eq!(json::from_utf8_with_field(r#""AP8=""#, &key)?, Scalar::from(&[0_u8, 0xFF][..]));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import DataType, Field, Scalar
    from yggdryl import json

    amount = Field("amount", "decimal128(8, 2)", nullable=False)
    exact = json.loads('"12.50"', field=amount, cls=Scalar)
    assert exact.kind == "d128"
    assert exact.unscaled == 1_250
    assert json.loads('"12.50"', field=amount) == Decimal("12.50")

    symbol = Field("symbol", "fixed_ascii(4)", nullable=False)
    held = json.loads('"AAPL"', field=symbol, cls=Scalar)

    assert held.dtype == DataType("fixed_ascii(4)")
    assert held.as_str() == "AAPL"
    assert json.loads('"AAPL"', field=symbol) == "AAPL"
    try:
        json.loads('"AAPLE"', field=symbol)
    except ValueError as error:
        assert "expected at most 4 bytes of us-ascii, got 5" in str(error), error
    else:
        raise AssertionError("five bytes do not fit a four-byte string")

    key = Field("key", "fixed_size_binary(2)", nullable=False)
    assert json.loads('"AP8="', field=key) == b"\x00\xff"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, json } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = json.loads('"12.50"', { field: amount })
    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.unscaled, 1250n)
    assert.equal(decoded.scale, 2)

    const symbol = new Field('symbol', 'fixed_ascii(4)', false)
    const held = json.loads('"AAPL"', { field: symbol, scalar: true })

    assert.ok(held.dtype.equals(DataType.from('fixed_ascii(4)')))
    assert.equal(held.asStr(), 'AAPL')
    assert.equal(json.loads('"AAPL"', { field: symbol }), 'AAPL')
    assert.throws(
      () => json.loads('"AAPLE"', { field: symbol }),
      /expected at most 4 bytes of us-ascii, got 5/,
    )

    const key = new Field('key', 'fixed_size_binary(2)', false)
    assert.deepEqual(json.loads('"AP8="', { field: key }), Buffer.from([0, 255]))
    ```

## The row root

A Struct Field yields one ordered row `Sequence` in Rust, a dictionary or object elsewhere; Python `cls=SomeDataclass` materializes it. The same field is what an Arrow [read](read.md#rows-as-arrow-batches) declares the rows land under, so one declaration types a document and its batch alike.

## Edges

- duplicate object names -> rejected, before any Field is consulted.
- a failed `Field` conversion -> error at the boundary, with the byte offset.
- a non-finite float, or a Mapping with non-string keys -> error on the way out, never silent coercion.
- empty or positional rows without a `Field` -> ambiguous; an explicit `Field` is required.
- placeholder options -> refused; only [YAML](../yaml/index.md) and [TOML](../toml/index.md) substitute, see [Placeholders](../placeholders.md).
- a time of day or duration carrying a zone -> refused; both must be naive, as on [Structured documents](../structured.md#edges).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test json
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/json/test_init.py
    python/.venv/bin/python -m pytest python/tests/text/test_codec.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    ```
