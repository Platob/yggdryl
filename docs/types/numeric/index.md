# Numeric

Boolean, integer, floating and decimal: the four families whose values are numbers, the widths each one is spelled at, and the selectors they share.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `null`, `boolean`, `int8`..`int64`, `uint8`..`uint64`, `float16`/`float32`/`float64`, `decimal32`..`decimal256` |
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
| [Decimal](decimal.md) | One decimal datatype over four backing widths, precision and scale, the exact `i256`/`u256` arithmetic, and `Decimal18` |
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

## Family values

Three of the four families have several leaves, so each is one `FamilyValue` enum over them - `Integer`, `Floating` and `Decimal`. A boolean has one leaf, so the leaf *is* the family and there is no enum. The enums are Rust only; the full contract is on [Scalar](../scalar.md#families).

```rust
use yggdryl::{Decimal, DataTypeKind, FamilyValue, Floating, Int32, Integer, Scalar};

// The variant is the leaf, spelled as the scalar spells it.
let held = Integer::from(Int32::new(7));
assert_eq!(Integer::KIND, DataTypeKind::Integer);
assert_eq!(Scalar::from(7_i32).as_integer(), Some(held));
assert!(matches!(Scalar::from(1.5_f64).as_floating(), Some(Floating::Float64(_))));

// The decimal family narrows through its own enum, because `as_decimal`
// is the coefficient-and-scale reader.
assert!(matches!(Decimal::from_scalar(&Scalar::d128(1_250, 2)), Some(Decimal::Decimal128(_))));
assert_eq!(Decimal::from_scalar(&Scalar::from(3_i64)), None);

// A boolean has one leaf, so there is nothing to narrow to.
assert_eq!(Scalar::from(true).as_bool(), Some(true));
```

In every language the value itself answers which family it belongs to.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

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

    use arrow_array::{Array, ArrayRef, StringArray};
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
    let strict = ArrowCastOptions::new().with_safe(false);

    let ids = Field::new("id", DataType::Int64, false).cast_arrow_array(Arc::clone(&text), strict)?;
    assert_eq!(ids.data_type(), &arrow_schema::DataType::Int64);

    let prices = Field::new("px", DataType::Float64, false).cast_arrow_array(text, strict)?;
    assert_eq!(prices.data_type(), &arrow_schema::DataType::Float64);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    text = pa.array(["1", "2"])

    assert Field("id", "int64").cast_arrow_array(text).equals(pa.array([1, 2], type=pa.int64()))
    assert Field("px", "float64").cast_arrow_array(text).equals(
        pa.array([1.0, 2.0], type=pa.float64())
    )
    assert Field("ok", "boolean").cast_arrow_array(pa.array(["true", "no"])).equals(
        pa.array([True, False])
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const text = arrow.vectorFromArray(['1', '2'], new arrow.Utf8())

    assert.deepEqual([...fields.int64('id').castArrowArray(text)], [1n, 2n])
    assert.deepEqual([...fields.float64('px').castArrowArray(text)], [1, 2])
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
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::floating field::integer field::floating field::decimal field::scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- decimal::tests arithmetic::tests
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^decimal/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_factories.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
