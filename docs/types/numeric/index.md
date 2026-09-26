# Numeric

Boolean, integer, floating and decimal: the four families whose values are numbers, the widths each one is spelled at, and the selectors they share.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `null`, `boolean`, `int8`..`int64`, `uint8`..`uint64`, `float16`/`float32`/`float64`, `decimal32`..`decimal256`, and the fixed `decimal` and `bigdecimal` |
| Validates | Once at construction, and only a decimal has something to check: precision `1..=max` of the width, positive scale `<= precision`; a bad parameter never becomes a value |
| Lazy | Nothing - a number has no children, no registry and no deferred parse |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field and shared by its clones |
| Refuses | A magnitude the declared width cannot hold, a precision or scale outside the width, and a decimal merged with a float |
| Errors | Rust `Error::InvalidDataType { kind, reason }` at construction and `Error::InvalidRecord` at the value door; Python `ValueError` (`TypeError`/`OverflowError` for bad argument types); JavaScript throws |
| Bindings | Python `DataType.decimal(precision, scale=0)` is the one decimal constructor; JavaScript builds decimals through `fields.decimal*(name, precision, scale=0)` and has no `DataType.decimal` |

## Pages

| page | owns |
| --- | --- |
| [Integer](integer.md) | The eight Arrow widths, signed and unsigned, their markers, their `Scalar` variants and readers, and the 128-bit values a wide computation lands in |
| [Floating](floating.md) | `float16`, `float32`, `float64`: bit-preserving, totally ordered values, and the NaN and zero rules that follow |
| [Decimal](decimal.md) | One decimal family over four backing widths, precision and scale, the exact `i256`/`u256` arithmetic, and the two fixed leaves at scale eighteen, `decimal` and `bigdecimal` |
| [Boolean](boolean.md) | `boolean` and the `null` datatype beside it: two parameter-free variants with no payload to carry |

## The kind a number answers

One [`DataTypeKind`](../datatype.md#identity-and-family) per family, and the kind is what family-uniform behaviour dispatches on; the exact variant stays available as the id.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    assert_eq!(DataType::Int32.kind(), DataTypeKind::Integer);
    assert_eq!(DataType::Float64.kind(), DataTypeKind::Floating);
    assert_eq!(DataType::decimal(38, 4)?.kind(), DataTypeKind::Decimal);
    assert_eq!(DataType::Boolean.kind(), DataTypeKind::Boolean);

    // A boolean is logic rather than arithmetic, so it is not a number.
    assert!(DataTypeKind::Integer.is_numeric());
    assert!(DataTypeKind::Decimal.is_numeric());
    assert!(!DataTypeKind::Boolean.is_numeric());

    // The id is the exact variant, and it knows the sign.
    assert_eq!(DataType::UInt8.id(), DataTypeId::UInt8);
    assert!(DataTypeId::Int32.is_signed_integer());
    assert!(DataTypeId::UInt8.is_unsigned_integer());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert DataType("int32").kind == "integer"
    assert DataType("float64").kind == "floating"
    assert DataType.decimal(38, 4).kind == "decimal"
    assert DataType("boolean").kind == "boolean"

    # A boolean is logic rather than arithmetic, so it is not a number.
    assert DataType("int32").is_numeric
    assert not DataType("boolean").is_numeric

    assert DataType("uint8").id == "uint8"
    assert DataType("int32").is_signed_integer
    assert DataType("uint8").is_unsigned_integer
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('int32').kind, 'integer')
    assert.equal(DataType.from('float64').kind, 'floating')
    assert.equal(DataType.from('decimal(38,4)').kind, 'decimal')
    assert.equal(DataType.from('boolean').kind, 'boolean')

    assert.equal(DataType.from('uint8').id, 'uint8')
    assert.equal(DataType.from('decimal(38,4)').id, 'decimal128')
    ```

## The family a value is in

A family is not a type: it is the range of identifiers its `DataTypeKind` owns, so the value answers which one it is in, `DataTypeKind::contains` checks an identifier against it, and the value itself is the leaf its variant holds - `Scalar::Int32(Int32)`, never an enum over the widths ([Scalar](../scalar.md#families)). The ranges are Rust only; every language reads `family` off the value.

=== "Rust"

    ```rust
    use yggdryl::{DataTypeKind, Scalar};

    assert_eq!(Scalar::from(7_i32).family(), DataTypeKind::Integer);
    assert_eq!(Scalar::from(1.5_f64).family(), DataTypeKind::Floating);
    assert_eq!(Scalar::d128(1_250, 2).family(), DataTypeKind::Decimal);
    assert_eq!(Scalar::from(true).family(), DataTypeKind::Boolean);

    // Membership is the identifier's range, whichever width holds the value.
    assert!(DataTypeKind::Integer.contains(Scalar::from(7_u8).id()));
    assert!(Scalar::d128(1_250, 2).is_number() && !Scalar::d128(1_250, 2).is_integer());

    // The kind is the width, the id the datatype it proves.
    assert_eq!(Scalar::from(7_i32).kind(), "i32");
    assert_eq!(Scalar::from(1.5_f64).kind(), "f64");
    assert_eq!(Scalar::d128(1_250, 2).kind(), "d128");
    assert_eq!(Scalar::from(true).kind(), "boolean");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    assert DataType("int32").scalar(7).family == "integer"
    assert DataType("float32").scalar(1.5).family == "floating"
    assert Scalar.decimal(1250, 2).family == "decimal"
    assert DataType("boolean").scalar(True).family == "boolean"

    # The kind is the width, the id is the datatype it proves.
    assert DataType("int32").scalar(7).kind == "i32"
    assert DataType("int32").scalar(7).id == "int32"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    assert.equal(DataType.from('int32').scalar(7).family, 'integer')
    assert.equal(DataType.from('float32').scalar(1.5).family, 'floating')
    assert.equal(Scalar.decimal(1250n, 2).family, 'decimal')
    assert.equal(DataType.from('boolean').scalar(true).family, 'boolean')

    assert.equal(DataType.from('int32').scalar(7).kind, 'i32')
    assert.equal(DataType.from('int32').scalar(7).id, 'int32')
    ```

## Selectors

Only the decimal family has a width to pick, and `decimal(p, s)` picks the most compact one that holds the precision. The exact constructors stay available, and each one states its own maximum.

| Call | Result |
| --- | --- |
| `decimal(1..=9, s)` | `decimal32(p,s)` |
| `decimal(10..=18, s)` | `decimal64(p,s)` |
| `decimal(19..=38, s)` | `decimal128(p,s)` |
| `decimal(39..=76, s)` | `decimal256(p,s)` |
| `numeric(p,s)` (parser) | same as `decimal(p,s)`; `bignumeric` is `decimal256` |

The clock selector `time(unit)`, the unit and zone vocabulary and the temporal spellings are on [Temporal](../temporal/index.md).

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::decimal(38, 4)?, DataType::decimal128(38, 4)?);
    assert_eq!(DataType::decimal(39, 4)?, DataType::decimal256(39, 4)?);

    // Out-of-range parameters are refused.
    assert!(DataType::decimal(2, 3).is_err());
    assert!(DataType::decimal128(39, 0).is_err());
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

## Typed markers

Rust only. Each payload `D` has a `FieldOf<D>` alias - `Int8Field`, `DecimalField`, ... - described on [Field](../field.md#the-field-leaves); a family with a parameter carries its payload enum instead, `DecimalType` here and the five [temporal](../temporal/index.md) payloads `DateType`, `TimeType`, `DateTimeType`, `DurationType`, `IntervalType`.

| Family (`yggdryl::<family>`) | Markers |
| --- | --- |
| `boolean` | `NullType`, `BooleanType` |
| `integer` | `Int8Type`, `Int16Type`, `Int32Type`, `Int64Type`, `UInt8Type`, `UInt16Type`, `UInt32Type`, `UInt64Type` |
| `floating` | `Float16Type`, `Float32Type`, `Float64Type` |
| `decimal` | `DecimalType`, the payload of `DecimalField` |

## Widening the four families share

[`merge_with`](../field.md#merging-two-schemas) is the one promotion table, and for these families it meets by width: two integers at the wider one, an integer beside a decimal at the widest backing either side declared, an integer beside a float at the float. An exact number and an approximate one have no meeting point that is not a re-encoding, so a decimal beside a float is refused.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::Int16.merge_with(&DataType::Int64, true)?, DataType::Int64);
    assert_eq!(DataType::Int32.merge_with(&DataType::Float32, true)?, DataType::Float32);
    assert_eq!(
        DataType::decimal128(10, 2)?.merge_with(&DataType::Int16, true)?,
        DataType::decimal128(10, 2)?,
    );

    // Narrowing meets at the tightest type naming both.
    assert_eq!(DataType::Int16.merge_with(&DataType::Int64, false)?, DataType::Int16);

    // An exact number and an approximate one do not meet.
    assert!(DataType::decimal128(10, 2)?.merge_with(&DataType::Float64, true).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert DataType("int16").merge_with(DataType("int64")) == DataType("int64")
    assert DataType("int32").merge_with(DataType("float32")) == DataType("float32")
    assert DataType("decimal128(10,2)").merge_with(DataType("int16")) == DataType(
        "decimal128(10,2)"
    )

    try:
        DataType("decimal128(10,2)").merge_with(DataType("float64"))
    except ValueError:
        pass
    else:
        raise AssertionError("an exact number and an approximate one do not meet")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('int16').mergeWith('int64').toString(), 'int64')
    assert.equal(DataType.from('int32').mergeWith('float32').toString(), 'float32')
    assert.equal(
      DataType.from('decimal128(10,2)').mergeWith('int16').toString(),
      'decimal128(10,2)',
    )

    assert.throws(() => DataType.from('decimal128(10,2)').mergeWith('float64'))
    ```

## Casts the four families share

One reading serves a row and a column: text into any of the four, any of the four into text, and a number into another number by the value it spells. Same-width pairs can instead carry the bytes under the number, which is [Cast](../cast.md#reading-the-bits)'s `representation`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
    let strict = ArrowCastOptions::new().with_safe(false);
    let id = Field::new("id", DataType::Int64, false);
    let px = Field::new("px", DataType::Float64, false);

    let ids = Serie::from_arrow_array(Some(&id), Arc::clone(&text), strict)?;
    assert_eq!(ids.as_int64().expect("an int64 column").values(), &[1, 2]);

    let prices = Serie::from_arrow_array(Some(&px), text, strict)?;
    assert_eq!(prices.as_float64().expect("a float64 column").values(), &[1.0, 2.0]);

    // A column in hand converts the same way: a number by the value it spells.
    let widened = ids.cast(&px, strict)?;
    assert_eq!(widened.as_float64().expect("a float64 column").values(), &[1.0, 2.0]);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field, Serie

    text = pa.array(["1", "2"])

    ids = Serie.from_arrow_array(text, Field("id", "int64"))
    assert ids.into_arrow_array().equals(pa.array([1, 2], type=pa.int64()))
    prices = Serie.from_arrow_array(text, Field("px", "float64"))
    assert prices.into_arrow_array().equals(pa.array([1.0, 2.0], type=pa.float64()))
    flags = Serie.from_arrow_array(pa.array(["true", "no"]), Field("ok", "boolean"))
    assert flags.into_arrow_array().equals(pa.array([True, False]))

    # A column in hand converts the same way: a number by the value it spells.
    assert ids.cast(Field("px", "float64")).as_py() == [1.0, 2.0]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const text = arrow.vectorFromArray(['1', '2'], new arrow.Utf8())

    const ids = Serie.fromArrowArray(text, fields.int64('id'))
    assert.deepEqual([...ids.intoArrowArray()], [1n, 2n])
    const prices = Serie.fromArrowArray(text, fields.float64('px'))
    assert.deepEqual([...prices.intoArrowArray()], [1, 2])

    // A column in hand converts the same way: a number by the value it spells.
    assert.deepEqual([...ids.cast(fields.float64('px')).intoArrowArray()], [1, 2])
    ```

## Edges

- `decimal(2, 3)` -> refused, `positive scale cannot exceed precision: 3 > 2`; a negative scale is accepted (`decimal256(39,-4)`).
- `decimal(0, 0)` -> `precision must be between 1 and 9: 0`; `decimal128(39, 0)` -> `precision must be between 1 and 38: 39`; `decimal(77, 0)` -> the `1 and 76` bound of `decimal256`.
- Python `DataType.decimal(True, 0)` -> `TypeError`; `DataType.decimal("18.0", 2)` -> `ValueError`; `DataType.decimal(256, 0)` -> `OverflowError`; `__index__` objects and base-10 strings are accepted.
- [Merged](../field.md#merging-two-schemas) widening -> the widest backing either side declared: `decimal128(10,2)` beside `int16` stays `decimal128(10,2)`. Narrowing takes the backing the merged precision needs.
- A decimal beside a float -> refused; an exact number and an approximate one have no meeting point that is not a re-encoding.
- `null` is a kind of its own, not a numeric one: it is on [Boolean](boolean.md), beside the other datatype that carries no payload.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- boolean decimal::fields decimal::selection floating integer
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- decimal::internal::reading arithmetic
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^decimal/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
