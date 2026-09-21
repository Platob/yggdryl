# Natural values and exact Fields

What a bare TOML parse answers, and what a declared [`Field`](../../types/field.md) changes about it.

## Contract

| Key | Value |
| --- | --- |
| Proves | strings, `i64`, `f64`, booleans, arrays, tables, and the four date/time forms; anything else needs a `Field` |
| Loads | a table or inline table to a name-sorted `Record`, an array to a `Sequence`, each date/time form to its exact temporal |
| Natural out | a value with no TOML syntax of its own travels as a quoted scale-preserving string, base64, or ISO text - never a private marker envelope |
| Exact in | `from_*_with_field` / `field=` / `{ field }` types the natural value, orders the root table, and validates |
| Order | parse, then [placeholder](../placeholders.md) substitution, then `Field` interpretation |
| Root | a Struct `Field` yields one ordered row `Sequence` in Rust; the bindings restore field names, and Python may add `cls=YourDataclass` |
| Refuses | null, a non-record root, a non-string key, an integer outside `i64`, a failed `Field` conversion |

## Use

A schemaless read answers only what the grammar proves, so an exact decimal spelled as a quoted string stays a string until a `Field` names it.

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::Scalar;

    let value = toml::from_utf8("amount = '12.50'\nquantity = 100\n")?;

    assert_eq!(
        value.get_key_str("amount").and_then(Scalar::as_str),
        Some("12.50")
    );
    assert_eq!(
        toml::into_utf8(&value)?,
        "\"amount\" = \"12.50\"\n\"quantity\" = 100\n"
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import toml

    source = "amount = '12.50'\nquantity = 100\n"
    value = toml.loads(source, cls=Scalar)

    assert value["amount"].kind == "string"
    assert value["quantity"].kind == "i64"
    assert toml.loads(source) == {"amount": "12.50", "quantity": 100}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const source = "amount = '12.50'\nquantity = 100\n"
    const value = toml.loads(source, { scalar: true })

    assert.equal(value.get('amount').kind, 'string')
    assert.equal(value.get('quantity').kind, 'i64')
    assert.deepEqual(toml.loads(source), { amount: '12.50', quantity: 100 })
    ```

## Natural spellings

Unspellable Scalars are rejected before a destination opens, never encoded into a side format.

| input or native value | natural TOML behavior |
| --- | --- |
| table / inline table | sorted `Record` |
| date | `Date32` |
| local time | `Time32` or `Time64` |
| local or offset date-time | `DateTime64` with explicit timezone |
| `D128`, `D256` | quoted scale-preserving string |
| bytes / geospatial | quoted base64 string |
| duration | quoted ISO duration |
| null | error |
| integer outside `i64` | error |

## Exact Fields

A Struct Field yields a row `Sequence` in Rust; the bindings restore field names, and Python may add `cls=YourDataclass`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, StructType};
    use yggdryl::toml;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let row = Field::new(
        "row",
        DataType::from(StructType::from_fields([amount])?),
        false,
    );
    let decoded = toml::from_utf8_with_field("amount = '12.50'\n", &row)?;

    assert_eq!(decoded.as_sequence().unwrap()[0], Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    import yggdryl

    from yggdryl import Field
    from yggdryl import toml

    row = yggdryl.struct(
        "row",
        [Field("amount", "decimal128(8, 2)", nullable=False)],
        nullable=False,
    )

    assert toml.loads("amount = '12.50'\n", field=row) == {
        "amount": Decimal("12.50")
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, toml } = require('yggdryl')

    const row = fields.struct(
      'row',
      [fields.decimal128('amount', 8, 2, { nullable: false })],
      { nullable: false },
    )
    const decoded = toml.loads("amount = '12.50'\n", { field: row })

    assert.equal(decoded.amount.kind, 'd128')
    assert.equal(decoded.amount.unscaled, 1250n)
    ```

The same field is what an Arrow [read](read.md#rows-as-arrow-batches) declares the rows land under, so one declaration types a document and its batch alike.

## Dates and times

TOML is the one structured format whose grammar proves temporals, so the four forms decode without a `Field` and write back as the syntax they were read from. Every native temporal carries a `TimeUnit` and a non-null [`Timezone`](../../types/temporal/timezone.md).

| value | TOML behavior |
| --- | --- |
| offset date-time | `DateTime64` instant; a local date-time uses `Timezone::NAIVE` |
| date, local time | zone-free |
| outside the grammar, or a named zone | ISO string or count, never a rewritten offset |
| bindings | closest lossless native temporal; `Scalar` keeps the rest |

=== "Rust"

    ```rust
    use yggdryl::toml;

    let source = "at = 1979-05-27T07:32:00Z\nlocal = 1979-05-27T07:32:00\n\
                  on = 1979-05-27\nclock = 07:32:00\n";
    let value = toml::from_utf8(source)?;
    let record = value.as_struct().unwrap();

    assert!(record["at"].as_datetime64().is_some());
    assert!(record["on"].as_date32().is_some());
    assert!(record["clock"].as_time32().is_some());
    // Each writes back as the syntax it was read from.
    assert_eq!(toml::from_utf8(&toml::into_utf8(&value)?)?, value);
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import toml

    decoded = toml.loads(
        b"offset = 1979-05-27T07:32:00Z\n"
        b"local_datetime = 1979-05-27T07:32:00\n"
        b"local_date = 1979-05-27\n"
        b"local_time = 07:32:00\n"
    )

    assert decoded == {
        "offset": dt.datetime(1979, 5, 27, 7, 32, tzinfo=dt.timezone.utc),
        "local_datetime": dt.datetime(1979, 5, 27, 7, 32),
        "local_date": dt.date(1979, 5, 27),
        "local_time": dt.time(7, 32),
    }
    # Each one writes back as the same TOML syntax it was read from.
    assert toml.dumps(decoded).split(b"\n")[:4] == [
        b'"local_date" = 1979-05-27',
        b'"local_datetime" = 1979-05-27T07:32:00',
        b'"local_time" = 07:32:00',
        b'"offset" = 1979-05-27T07:32:00Z',
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, toml } = require('yggdryl')

    const source =
      'offset = 1979-05-27T07:32:00Z\nlocal = 1979-05-27T07:32:00\ndate = 1979-05-27\ntime = 07:32:00\n'
    const decoded = toml.loads(source)

    assert.ok(decoded.offset instanceof Date)
    assert.equal(decoded.offset.toISOString(), '1979-05-27T07:32:00.000Z')
    assert.ok(decoded.local.equals(new DataType('datetime64(s)').scalar(296638320n)))
    assert.ok(decoded.date.equals(new DataType('date32').scalar(3433)))
    assert.ok(decoded.time.equals(new DataType('time32(s)').scalar(27120)))

    // Each one writes back as the same TOML syntax it was read from.
    assert.equal(
      toml.dumps(decoded).toString('utf8'),
      '"date" = 1979-05-27\n"local" = 1979-05-27T07:32:00\n"offset" = 1979-05-27T07:32:00Z\n"time" = 07:32:00\n',
    )
    ```

## Edges

- a time of day or duration carrying a zone -> refused; both must be naive, as on [Structured documents](../structured.md#edges).
- a duration -> a quoted ISO string on the way out, read back under a duration `Field`.
- `D128` and `D256` -> quoted scale-preserving strings; the scale survives the round trip only under a decimal `Field`.
- an integer outside `i64` -> error on the way out; TOML integers are `i64`.
- a failed `Field` conversion -> error at the boundary, with the byte offset.
- a duplicate key -> rejected, before any Field is consulted.
- a user key spelled like a private marker -> ordinary data; nothing is reserved.
- a quoted [placeholder](../placeholders.md) -> substituted before the Field runs, so the resolved string becomes the exact value.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test toml -- mod_
    cargo test --features "iceberg internals parquet" -p yggdryl --test toml -- wire
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/toml/test_init.py
    python/.venv/bin/python -m pytest python/tests/text/test_codec.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text/codec.test.js
    ```
