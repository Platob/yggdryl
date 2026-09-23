# Decimal

One exact base-10 datatype over four backing widths: a precision, a scale, and a coefficient that is an integer whatever it is written as.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `decimal32`, `decimal64`, `decimal128`, `decimal256`, the `Decimal32`..`Decimal256` values, the `i256`/`u256` pair they compute in, and `Decimal18` |
| Validates | Once at construction: precision `1..=max` of the width (9, 18, 38, 76), a positive scale `<= precision`; a bad parameter never becomes a value |
| Lazy | Nothing - a decimal holds a coefficient and a scale and resolves neither later |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | A precision or scale outside the width, an inexact quotient, an overflow of the target width, and a merge with a float |
| Normalizes | Equality, order and hashing strip trailing zeros first, so `10.50` and `10.5` are one value at one digest |

## DataType

`decimal(p, s)` is one rule with the width chosen for the caller: the most compact backing integer that holds the precision. The four exact constructors stay available and each states its own maximum. Precision and scale are what the column *means*; the width is only how wide the backing integer is, so a reader asks the payload and never branches on the width.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, DecimalType};

    let amount = DataType::decimal(38, 4)?;
    assert_eq!(amount, DataType::decimal128(38, 4)?);
    assert_eq!(amount.to_string(), "decimal128(38,4)");
    assert_eq!(amount.id(), DataTypeId::Decimal128);
    assert_eq!(amount.kind(), DataTypeKind::Decimal);

    // The payload answers what the column means, whichever width holds it.
    let held = DecimalType::try_from(&amount)?;
    assert_eq!(held.precision(), 38);
    assert_eq!(held.scale(), 4);
    assert_eq!(held.maximum(), 38);
    assert_eq!(
        DecimalType::narrowest(9, 2),
        DecimalType::Decimal32 { precision: 9, scale: 2 },
    );
    assert_eq!(DataType::decimal_of(DecimalType::narrowest(9, 2))?, DataType::decimal32(9, 2)?);

    // A bad parameter never becomes a value.
    assert!(DataType::decimal32(10, 0).is_err());
    assert!(DataType::decimal(2, 3).is_err());
    assert!(DataType::decimal(77, 0).is_err());

    // A negative scale is a multiplier rather than fractional digits.
    assert_eq!(DataType::decimal256(39, -4)?.to_string(), "decimal256(39,-4)");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    amount = DataType.decimal(38, 4)
    assert amount == DataType("decimal128(38,4)")
    assert str(amount) == "decimal128(38,4)"
    assert amount.id == "decimal128"
    assert amount.kind == "decimal"
    assert amount.is_decimal and amount.is_parameterized

    assert DataType("numeric(18,4)") == DataType("decimal64(18,4)")
    assert DataType("bignumeric(39,4)") == DataType("decimal256(39,4)")

    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, 3)
    with pytest.raises(ValueError, match="precision must be between 1 and 76"):
        DataType.decimal(77, 0)

    # A negative scale is a multiplier rather than fractional digits.
    assert str(DataType.decimal(39, -4)) == "decimal256(39,-4)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // There is no `DataType.decimal` here: a decimal is declared on the field.
    assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
    assert.equal(fields.decimal32('small', 9, 2).dtype.toString(), 'decimal32(9,2)')
    assert.equal(fields.decimal256('wide', 39, 4).dtype.toString(), 'decimal256(39,4)')

    assert.equal(DataType.from('numeric(18,4)').toString(), 'decimal64(18,4)')
    assert.equal(DataType.from('bignumeric(39,4)').toString(), 'decimal256(39,4)')

    assert.throws(() => fields.decimal('bad', 2, 3), /positive scale cannot exceed precision/)
    ```

| Call | Result | Maximum precision |
| --- | --- | ---: |
| `decimal(1..=9, s)` | `decimal32(p,s)` | 9 |
| `decimal(10..=18, s)` | `decimal64(p,s)` | 18 |
| `decimal(19..=38, s)` | `decimal128(p,s)` | 38 |
| `decimal(39..=76, s)` | `decimal256(p,s)` | 76 |
| `numeric(p,s)` (parser) | same as `decimal(p,s)`; `bignumeric` is `decimal256` | |

## Field

`DecimalField` is `FieldOf<DecimalType>`: the family has a parameter, so the field is built with `new(name, payload, nullable)` carrying that payload rather than with `unit`.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DecimalField, DecimalType, Field, Int64Field};

    let amount = DecimalField::new(
        "amount",
        DecimalType::Decimal128 { precision: 38, scale: 4 },
        false,
    );
    assert_eq!(amount.name(), "amount");
    assert_eq!(amount.dtype(), &DataType::decimal128(38, 4)?);
    assert!(!amount.is_nullable());

    let root: Field = amount.into_field();
    assert!(DecimalField::from_field(&root).is_some());
    assert!(Int64Field::from_field(&root).is_none());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    amount = yggdryl.decimal("amount", 38, 4, nullable=False)
    small = yggdryl.decimal32("small", 9, 2)

    assert isinstance(amount, Field)
    assert str(amount.dtype) == "decimal128(38,4)"
    assert not amount.nullable
    assert str(small.dtype) == "decimal32(9,2)"
    assert str(yggdryl.decimal64("mid", 18, 6).dtype) == "decimal64(18,6)"
    assert str(yggdryl.decimal256("wide", 39, 4).dtype) == "decimal256(39,4)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const amount = fields.decimal('amount', 38, 4, { nullable: false })
    assert.ok(amount instanceof Field)
    assert.equal(amount.dtype.toString(), 'decimal128(38,4)')
    assert.equal(amount.nullable, false)
    assert.equal(fields.decimal64('mid', 18, 6).dtype.toString(), 'decimal64(18,6)')
    ```

## Scalar

A decimal value is a coefficient and a scale - `D32`, `D64`, `D128`, `D256` - and the number it names is `coefficient * 10^-scale`. `as_decimal` is the cross-width reader that widens the coefficient to 256 bits; `as_d128` and `as_d256` read one width. Equality, order and hashing normalize first, so one number written at two scales is one value.

=== "Rust"

    ```rust
    use yggdryl::{Decimal128, Scalar, i256};

    let price = Scalar::d128(1_050, 2);
    assert_eq!(price.kind(), "d128");
    assert!(price.is_decimal());
    assert_eq!(price.as_d128(), Some((1_050, 2)));
    assert_eq!(price.as_decimal(), Some((i256::from_i128(1_050), 2)));
    assert_eq!(price.into_decimal_utf8().as_deref(), Some("10.50"));

    // One number at two scales is one value, and reads at any scale asked for.
    assert_eq!(price, Scalar::d128(105, 1));
    assert_eq!(price.decimal_unscaled_at(4), Some(105_000));

    // A caller with only a coefficient gets the narrowest width holding it.
    assert_eq!(Scalar::from_decimal(i256::from_i128(1_250), 2), Scalar::d128(1_250, 2));
    assert_eq!(Decimal128::new(1_050, 2).coefficient(), 1_050);
    assert_eq!(Decimal128::new(1_050, 2).scale(), 2);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Scalar

    price = Scalar.decimal(1050, 2)
    assert price.kind == "d128"
    assert price.unscaled == 1050
    assert price.scale == 2
    assert price.as_py() == Decimal("10.50")
    assert str(price.dtype) == "decimal128(4,2)"

    # One number at two scales is one value.
    assert price == Scalar.decimal(105, 1)

    # A coefficient too wide for 128 bits takes the 256-bit width.
    assert Scalar.decimal("12345678901234567890123456789012345678901", 2).kind == "d256"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    const price = Scalar.decimal(1250n, 2)
    assert.equal(price.kind, 'd128')
    assert.equal(price.unscaled, 1250n)
    assert.equal(price.scale, 2)
    assert.equal(price.family, 'decimal')
    assert.equal(price.dtype.toString(), 'decimal128(4,2)')

    // One number at two scales is one value.
    assert.ok(price.equals(Scalar.decimal(125n, 1)))
    ```

## Arrow storage

Each width is Arrow's own decimal, carrying the same precision and scale, and imports back as itself. Projection re-checks the parameters, so a directly built leaf cannot escape through it.

| datatype | Arrow storage | bytes |
| --- | --- | ---: |
| `decimal32(p,s)` | `Decimal32(p, s)` | 4 |
| `decimal64(p,s)` | `Decimal64(p, s)` | 8 |
| `decimal128(p,s)` | `Decimal128(p, s)` | 16 |
| `decimal256(p,s)` | `Decimal256(p, s)` | 32 |

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(
        DataType::decimal128(38, 4)?.into_arrow_datatype()?,
        arrow_schema::DataType::Decimal128(38, 4),
    );
    assert_eq!(
        DataType::from_arrow_datatype(&arrow_schema::DataType::Decimal32(9, 2))?,
        DataType::decimal32(9, 2)?,
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    assert DataType("decimal128(38,4)").into_arrow() == pa.decimal128(38, 4)
    assert DataType.from_arrow(pa.decimal32(9, 2)) == DataType("decimal32(9,2)")

    assert DataType("decimal128(38,4)").fixed_byte_width == 16
    assert DataType("decimal256(40,2)").fixed_byte_width == 32
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('decimal128(38,4)').fixedByteWidth, 16)
    assert.equal(DataType.from('decimal256(40,2)').fixedByteWidth, 32)
    assert.equal(DataType.fromArrow({ toString: () => 'decimal(9,2)' }).toString(), 'decimal32(9,2)')
    ```

## Exact arithmetic

Addition, subtraction and remainder meet at the wider scale; multiplication adds the scales; division answers only when the quotient is exact, and refuses rather than rounding. Every step runs in `i256`, and a result past the target width's precision is an overflow.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert_eq!(
        Scalar::d128(1_050, 2).checked_add(&Scalar::d128(1, 0))?,
        Scalar::d128(1_150, 2),
    );
    assert_eq!(
        Scalar::d128(825, 1).checked_mul(&Scalar::d128(1_000, 0))?,
        Scalar::d128(825_000, 1),
    );
    assert_eq!(Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0))?, Scalar::d128(5, 1));

    // An inexact quotient is refused rather than rounded, and zero is refused.
    assert!(Scalar::d128(1, 0).checked_div(&Scalar::d128(3, 0)).is_err());
    assert!(Scalar::d128(1, 0).checked_div(&Scalar::d128(0, 0)).is_err());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    import pytest

    from yggdryl import Scalar

    assert (Scalar.decimal(1050, 2) + Scalar.decimal(1, 0)).as_py() == Decimal("11.50")
    assert (Scalar.decimal(825, 1) * Scalar.decimal(1000, 0)).as_py() == Decimal("82500.0")
    assert Scalar.decimal(1, 0) / Scalar.decimal(2, 0) == Scalar.decimal(5, 1)

    with pytest.raises(ArithmeticError):
        Scalar.decimal(1, 0) / Scalar.decimal(3, 0)
    with pytest.raises(ZeroDivisionError):
        Scalar.decimal(1, 0) / Scalar.decimal(0, 0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.ok(Scalar.decimal(1050n, 2).add(Scalar.decimal(1n, 0)).equals(Scalar.decimal(1150n, 2)))
    assert.ok(Scalar.decimal(1n).divide(Scalar.decimal(2n)).equals(Scalar.decimal(5n, 1)))

    assert.throws(() => Scalar.decimal(1n).divide(Scalar.decimal(3n)))
    assert.throws(() => Scalar.decimal(1n).divide(Scalar.decimal(0n)), /division by zero/)
    ```

## Casts

Text reads into a decimal without passing through a float, and an integer converts into one by rescaling its coefficient; the declared precision and scale are the target, and `safe` and `nullability` decide what a failure becomes, on [Cast](../cast.md).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["10.5", "-0.25"]));
    let amount = Field::new("amount", DataType::decimal128(38, 4)?, false);
    let strict = ArrowCastOptions::new().with_safe(false);
    let amounts = Serie::from_arrow_array(Some(&amount), text, strict)?;

    // The coefficients, at the declared scale of four.
    let coefficients = amounts.as_decimal128().expect("a decimal128 column").values();
    assert_eq!(coefficients, &[105_000, -2_500]);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    import pyarrow as pa

    from yggdryl import Field, Serie

    amount = Field("amount", "decimal128(38,4)")
    amounts = Serie.from_arrow_array(pa.array(["10.5", "-0.25"]), amount)
    assert amounts.as_py() == [Decimal("10.5000"), Decimal("-0.2500")]

    # An integer column rescales into the declared scale.
    counts = pa.array([1, 2], type=pa.int32())
    counted = Serie.from_arrow_array(counts, Field("amount", "decimal64(18,2)"))
    assert counted.as_py() == [Decimal("1.00"), Decimal("2.00")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const text = arrow.vectorFromArray(['10.5', '-0.25'], new arrow.Utf8())
    const amounts = Serie.fromArrowArray(text, fields.decimal('amount', 38, 4))
    assert.equal(amounts.length, 2)
    assert.equal(amounts.field.dtype.toString(), 'decimal128(38,4)')
    // Arrow JS renders the coefficient, at the declared scale of four.
    assert.equal(String(amounts.intoArrowArray().get(0)), '105000')
    ```

## The 256-bit pair

`u256` is the unsigned magnitude every wide computation runs in and `i256` is the signed two's-complement value a `decimal256` coefficient is stored as. Both keep four 64-bit words least-significant first, so a little-endian buffer is the same 32 bytes in either direction and Arrow conversion copies rather than re-encodes. Rust only: a decimal crosses a binding as a coefficient and a scale.

```rust
use yggdryl::{i256, u256};

let widest: u256 =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935".parse()?;
assert_eq!(widest, u256::MAX);
assert_eq!(u256::from_u128(10).checked_mul(u256::from_u128(10)), Some(u256::from_u128(100)));

// The signed half is the coefficient a `decimal256` stores.
assert_eq!(i256::from_i128(7).as_i128(), Some(7));
assert!(i256::from_i128(-1).is_negative());
assert_eq!(i256::from_i128(-5).unsigned_abs(), u256::from_u128(5));
assert_eq!(i256::from_le_bytes(i256::from_i128(9).into_le_bytes()), i256::from_i128(9));
```

## Decimal18

`Decimal18` is `decimal128(38, 18)` preapplied: one `i128` of units at eighteen fractional digits, bounded to thirty-eight digits, so a price and a quantity add, multiply and compare as integers do and land in a `decimal128(38, 18)` column exactly. `DataType::DECIMAL` is that datatype, and `Decimal` is the decimal family's value enum over the four widths ([Scalar](../scalar.md#families)), not this value. Multiplication and division widen to 256 bits and truncate the rest toward zero; a result past the precision is an overflow the checked operations answer as `None` and the operators refuse as the integers' do. Text reads leniently - whitespace, an empty text as nothing, grouping with `,` `_` `'` or a space, a leading or trailing point, an exponent, extra fractional digits truncated - and refuses only what states no number or a value past the precision. Rust only: a market element's price and quantity are held as it.

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

## Edges

- `decimal(2, 3)` -> refused, `positive scale cannot exceed precision: 3 > 2`; a negative scale is accepted (`decimal256(39,-4)`).
- `decimal(0, 0)` -> `precision must be between 1 and 9: 0`; `decimal128(39, 0)` -> `precision must be between 1 and 38: 39`; `decimal(77, 0)` -> the `1 and 76` bound of `decimal256`.
- Python `DataType.decimal(True, 0)` -> `TypeError`; `DataType.decimal("18.0", 2)` -> `ValueError`; `DataType.decimal(256, 0)` -> `OverflowError`; `__index__` objects and base-10 strings are accepted.
- An inexact quotient -> refused, not rounded; a quotient whose exact scale would exceed the width's maximum is the same refusal.
- [Merged](../field.md#merging-two-schemas) widening -> the widest backing either side declared: `decimal128(10,2)` beside `int16` stays `decimal128(10,2)`. Narrowing takes the backing the merged precision needs.
- A decimal beside a float -> refused; an exact number and an approximate one have no meeting point that is not a re-encoding.
- `Decimal18::MAX.checked_add(Decimal18::ONE)` -> `None`; the operator form refuses as the integers' do.
- Python has no `DataType.decimal128`: `DataType.decimal(precision, scale)` is the one constructor, and the exact widths are field factories (`yggdryl.decimal128`). JavaScript has no `DataType.decimal` at all, only `fields.decimal*`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- compatibility decimal::exact::comparison decimal::exact::family decimal::exact::fixed decimal::exact::representation decimal::exact::restating decimal::fields decimal::selection int256 merge::lattice parser::families regex::fractions temporal::datatypes variant::encoding wkb::exactness
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test value -- canonical::value::readings
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- decimal::internal::reading arithmetic
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^decimal/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test_scalar.py -k "decimal"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="defaulted temporal and decimal overloads share exact option handling" node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
