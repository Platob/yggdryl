# Numeric

This page owns the boolean, integer, floating and decimal datatypes and their width selectors; the calendar and clock datatypes, their units and their zones are on [Temporal](temporal.md).

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `null`, `boolean`, `int8`..`int64`, `uint8`..`uint64`, `float16`/`float32`/`float64`, `decimal32`..`decimal256` |
| Selectors | `decimal(p, s)` picks a physical width; the exact constructors (`decimal32`..`decimal256`) stay available |
| Validates | Once at construction: precision `1..=max` of the width, positive scale `<= precision`; a bad parameter never becomes a value |
| Errors | Rust `Error::InvalidDataType { kind, reason }`; Python `ValueError` (`TypeError`/`OverflowError` for bad argument types); JavaScript throws |
| Bindings | Python `DataType.decimal(precision, scale=0)`; JavaScript `fields.decimal*(name, precision, scale=0)`; no `DataType.decimal` in JavaScript |

## Use

The selector validates once and picks the physical width at construction.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::decimal(38, 4)?, DataType::decimal128(38, 4)?);
    assert_eq!(DataType::decimal(39, 4)?, DataType::decimal256(39, 4)?);

    // Out-of-range parameters are refused.
    assert!(DataType::decimal(2, 3).is_err());
    assert!(DataType::decimal128(39, 0).is_err());
    assert!(DataType::fixed_binary(0).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal(39, 4) == DataType("decimal256(39,4)")

    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, 3)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
    assert.equal(fields.decimal('wide', 39, 4).dtype.toString(), 'decimal256(39,4)')

    assert.throws(() => fields.decimal('bad', 2, 3), /positive scale cannot exceed precision/)
    ```

## Decimal18

`Decimal18` is `decimal128(38, 18)` preapplied: one `i128` of units at eighteen fractional digits, bounded to thirty-eight digits, so a price and a quantity add, multiply and compare as integers do and land in a `decimal128(38, 18)` column exactly. `DataType::DECIMAL` is that datatype, and `Decimal` is the decimal family's value enum over the four widths ([Scalar](scalar.md#families)), not this value. Multiplication and division widen to 256 bits and truncate the rest toward zero; a result past the precision is an overflow the checked operations answer as `None` and the operators refuse as the integers' do. Text reads leniently - whitespace, an empty text as nothing, grouping with `,` `_` `'` or a space, a leading or trailing point, an exponent, extra fractional digits truncated - and refuses only what states no number or a value past the precision. Rust only: a market element's price and quantity are held as it.

```rust
use yggdryl::Decimal18;
use yggdryl::{DataType, Scalar};

let px: Decimal18 = "82.5".parse()?;
let qty = Decimal18::from_int(1_000);
assert_eq!((px * qty).to_string(), "82500");
assert_eq!((px / Decimal18::from_int(4)).to_string(), "20.625");
assert_eq!(Decimal18::parse(" 1,250.50 ")?.to_string(), "1250.5");
assert_eq!(Decimal18::parse("")?, Decimal18::ZERO);
assert_eq!(Decimal18::dtype(), DataType::DECIMAL);
assert_eq!(Scalar::from(px).as_d128(), Some((82_500_000_000_000_000_000, 18)));
assert_eq!(Decimal18::from_scalar(&Scalar::d128(825, 1)), Some(px));
assert_eq!(Decimal18::MAX.checked_add(Decimal18::ONE), None);
```

## Selectors

| Call | Result |
| --- | --- |
| `decimal(1..=9, s)` | `decimal32(p,s)` |
| `decimal(10..=18, s)` | `decimal64(p,s)` |
| `decimal(19..=38, s)` | `decimal128(p,s)` |
| `decimal(39..=76, s)` | `decimal256(p,s)` |
| `numeric(p,s)` (parser) | same as `decimal(p,s)`; `bignumeric` is `decimal256` |

The clock selector `time(unit)`, the unit and zone vocabulary and the temporal spellings are on [Temporal](temporal.md).

## Typed markers

Rust only. Each marker `M` has a `FieldOf<M>` alias (`Int8Field`, `DecimalField`, ...) described on [Field](field.md); a family with leaves carries its payload enum instead, `DecimalType` here and the five [temporal](temporal.md) payloads `DateType`, `TimeType`, `DateTimeType`, `DurationType`, `IntervalType`.

| Family (`yggdryl::<family>`) | Markers |
| --- | --- |
| `boolean` | `NullType`, `BooleanType` |
| `integer` | `Int8Type`, `Int16Type`, `Int32Type`, `Int64Type`, `UInt8Type`, `UInt16Type`, `UInt32Type`, `UInt64Type` |
| `floating` | `Float16Type`, `Float32Type`, `Float64Type` |
| `decimal` | `DecimalType`, the payload of `DecimalField` |

## Edges

- `decimal(2, 3)` -> refused, `positive scale cannot exceed precision: 3 > 2`; a negative scale is accepted (`decimal256(39,-4)`).
- `decimal(0, 0)` -> `precision must be between 1 and 9: 0`; `decimal128(39, 0)` -> `precision must be between 1 and 38: 39`; `decimal(77, 0)` -> the `1 and 76` bound of `decimal256`.
- Python `DataType.decimal(True, 0)` -> `TypeError`; `DataType.decimal("18.0", 2)` -> `ValueError`; `DataType.decimal(256, 0)` -> `OverflowError`; `__index__` objects and base-10 strings are accepted.
- `fixed_size_binary(0)` -> refused, `at least one byte, got 0`; the byte layouts live on [Strings & bytes](text.md).
- [Merged](field.md) widening -> the widest backing either side declared: `decimal128(10,2)` beside `int16` stays `decimal128(10,2)`. Narrowing takes the backing the merged precision needs.
- A decimal beside a float -> refused; an exact number and an approximate one have no meeting point that is not a re-encoding.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::floating field::integer field::floating field::decimal field::scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- decimal::tests arithmetic::tests
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^decimal/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "decimal"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="temporal and decimal" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
