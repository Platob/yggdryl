# Decimal

One exact base-10 family: four parameterized backing widths - a precision, a scale, and a coefficient that is an integer whatever it is written as - and two fixed leaves, [`decimal`](#decimal) and [`bigdecimal`](#bigdecimal), whose scale is eighteen already.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `decimal32`, `decimal64`, `decimal128`, `decimal256`, the `Decimal32`..`Decimal256` values, the `i256`/`u256` pair they compute in, and the two fixed leaves `decimal` and `bigdecimal` with their values `Decimal` and `BigDecimal` |
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
| `numeric(p,s)` (parser) | same as `decimal(p,s)`; `bignumeric` is `decimal256`; bare `numeric` is SQL's `decimal128(38,0)` | |
| bare `decimal`, `bigdecimal` (parser) | the fixed leaves [`decimal`](#decimal) and [`bigdecimal`](#bigdecimal), never a width | 38, 76 |

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

A decimal value is a coefficient and a scale - `D32`, `D64`, `D128`, `D256` - and the number it names is `coefficient * 10^-scale`; the [fixed leaves](#decimal) hold their units alone. `as_decimal` is the cross-width reader that widens the coefficient to 256 bits; `as_d128` and `as_d256` read one width. Equality, order and hashing normalize first, so one number written at two scales is one value.

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
| [`decimal`](#decimal) | `Decimal128(38, 18)` under `yggdryl.decimal` | 16 |
| [`bigdecimal`](#bigdecimal) | `Decimal256(76, 18)` under `yggdryl.bigdecimal` | 32 |

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

## Decimal { #decimal }

`decimal` is the family's fixed leaf: `decimal128(38, 18)` preapplied, one datatype, field and scalar of its own, and the value [a market](../../graph/index.md#market) holds its prices and quantities as.

### Contract

| Aspect | Rule |
| --- | --- |
| Value | `yggdryl::Decimal`: one `i128` of units at scale eighteen, bounded to thirty-eight digits, `MIN` to `MAX`; add, subtract and compare are the integer's, and multiply and divide widen to 256 bits for the one product |
| Arithmetic | `checked_add`, `checked_sub`, `checked_mul`, `checked_div` answer `None` past thirty-eight digits, the last also for a divisor of nothing; `checked_mul` and `checked_div` keep eighteen fractional digits and truncate the rest toward zero; the operators refuse an overflow as the integers' do |
| Text | `Decimal::parse` (and `FromStr`) is lenient and one pass: surrounding whitespace, an empty text as zero, a sign, grouping with `,` `_` `'` or a space ahead of the point, a leading or trailing point, an exponent, and digits past the eighteenth fractional one truncated toward zero; it refuses only text stating no number and a value past thirty-eight digits. The value door - `DataType::Decimal.scalar`, `Field::scalar` - is strict: a nineteenth fractional digit is refused, never truncated |
| Width | `widened()` is the lossless way into [`BigDecimal`](#bigdecimal); `DataType::DECIMAL` is the `decimal128(38, 18)` storage the leaf rides and the spelling a FIX dictionary types a price with, a parameterized width and not the leaf |
| Identity | `DataTypeId::Decimal`, `0x2d`, inside the decimal family's [range](../scalar.md#families): `is_decimal` answers it and `is_parameterized` does not |

### DataType

Bare `decimal` is the leaf; a parenthesis names the parameterized family, so `decimal(38, 18)` is the narrowest width holding it, and SQL's bare `numeric` keeps its own meaning.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Decimal};

    let leaf: DataType = "decimal".parse()?;
    assert_eq!(leaf, DataType::Decimal);
    assert_eq!(leaf.to_string(), "decimal");
    assert_eq!(leaf.id(), DataTypeId::Decimal);
    assert_eq!(DataTypeId::Decimal.as_u8(), 0x2d);
    assert_eq!(leaf.kind(), DataTypeKind::Decimal);
    assert!(!leaf.id().is_parameterized());
    assert_eq!(Decimal::dtype(), leaf);

    // A parenthesis is the parameterized family; `numeric` is SQL's.
    assert_eq!("decimal(38,18)".parse::<DataType>()?, DataType::decimal128(38, 18)?);
    assert_eq!("numeric".parse::<DataType>()?, DataType::decimal128(38, 0)?);

    // `DataType::DECIMAL` is the storage the leaf rides, not the leaf.
    assert_eq!(DataType::DECIMAL, DataType::decimal128(38, 18)?);
    assert_ne!(DataType::DECIMAL, DataType::Decimal);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    leaf = DataType("decimal")
    assert leaf.id == "decimal"
    assert str(leaf) == "decimal"
    assert leaf.kind == "decimal"

    # A parenthesis is the parameterized family.
    assert DataType("decimal(38,18)") == DataType("decimal128(38,18)")
    assert DataType("decimal(38,18)") != leaf
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const leaf = DataType.from('decimal')
    assert.equal(leaf.id, 'decimal')
    assert.equal(leaf.toString(), 'decimal')
    assert.equal(leaf.kind, 'decimal')

    // A parenthesis is the parameterized family.
    assert.equal(DataType.from('decimal(38,18)').toString(), 'decimal128(38,18)')
    ```

### Field

`DecimalField` carries the payload `DecimalType::Decimal`, whose precision and scale are the leaf's. Python's `yggdryl.decimal(name)` is the leaf, typed `DecimalField`, and `yggdryl.decimal(name, precision, scale)` the width, typed `DecimalWidthField`; JavaScript's `fields.decimal(name)` and `fields.decimal(name, precision, scale)` split the same way.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DecimalField, DecimalType, Field};

    let px = DecimalField::new("px", DecimalType::Decimal, false);
    assert_eq!(px.dtype(), &DataType::Decimal);
    assert_eq!((DecimalType::Decimal.precision(), DecimalType::Decimal.scale()), (38, 18));

    let root: Field = px.into_field();
    assert!(DecimalField::from_field(&root).is_some());
    ```

=== "Python"

    ```python
    import yggdryl

    px = yggdryl.decimal("px", nullable=False)
    assert str(px.dtype) == "decimal"
    assert not px.nullable

    # A precision names the width instead.
    assert str(yggdryl.decimal("amount", 38, 4).dtype) == "decimal128(38,4)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const px = fields.decimal('px', { nullable: false })
    assert.equal(px.dtype.toString(), 'decimal')
    assert.equal(px.nullable, false)

    // A precision names the width instead.
    assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
    ```

### Scalar

`Scalar::Decimal` holds the value; its wire tag is `decimal`, carrying the units. It equals, orders and hashes as the `d128` twin naming the same number, and digests alike, so a value is one value whichever leaf holds it. `as_decimal` reads it as its units at scale eighteen, and `Display` trims the zeros behind the point. Arithmetic over a leaf keeps the leaf: the other operand meets it at scale eighteen, a `bigdecimal` on either side answers a `bigdecimal`, and a remainder is refused because a fixed scale states none.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Decimal, Scalar};

    let px: Decimal = "82.5".parse()?;
    let qty = Decimal::from_int(1_000);
    assert_eq!((px * qty).to_string(), "82500");
    assert_eq!((px / Decimal::from_int(4)).to_string(), "20.625");
    assert_eq!(px + Decimal::from_int(1), "83.5".parse::<Decimal>()?);
    // Eighteen digits kept, the rest truncated toward zero.
    assert_eq!((Decimal::ONE / Decimal::from_int(3)).to_string(), "0.333333333333333333");
    assert_eq!(px.units(), 82_500_000_000_000_000_000);
    assert_eq!(Decimal::MAX.checked_add(Decimal::ONE), None);

    // The text door reads leniently and truncates.
    assert_eq!(Decimal::parse(" 1,250.50 ")?.to_string(), "1250.5");
    assert_eq!(Decimal::parse("")?, Decimal::ZERO);
    assert_eq!(Decimal::parse("2.5e3")?.to_string(), "2500");
    assert_eq!(Decimal::parse("0.1234567890123456789")?.to_string(), "0.123456789012345678");
    assert!(Decimal::parse("1.2.3").is_err() && Decimal::parse("NaN").is_err());

    // The scalar is the leaf's own, and one number whichever leaf holds it.
    let value = Scalar::from(px);
    assert_eq!(value, Scalar::Decimal(px));
    assert_eq!(value.kind(), "decimal");
    assert_eq!(value, Scalar::d128(825, 1));
    assert_eq!(value.as_decimal().map(|(_, scale)| scale), Some(18));
    assert_eq!(Decimal::from_scalar(&Scalar::d128(825, 1)), Some(px));

    // The value door restates and is strict.
    assert_eq!(DataType::Decimal.scalar(Scalar::d128(825, 1))?, value);
    assert!(DataType::Decimal.scalar(Scalar::d128(1, 19)).is_err(), "a nineteenth digit");

    // Arithmetic keeps the leaf; a remainder is refused.
    assert_eq!(value.checked_mul(&Scalar::from(1_000_i64))?, Scalar::Decimal(Decimal::from_int(82_500)));
    assert!(value.checked_rem(&Scalar::from(2_i64)).is_err());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    import pytest

    from yggdryl import Field, Scalar

    px = Field("px", "decimal").scalar(Decimal("82.5"))
    assert px.kind == "decimal"
    assert px.as_py() == Decimal("82.5")
    assert px == Scalar.decimal(825, 1)

    # The value door refuses a nineteenth fractional digit.
    with pytest.raises(ValueError):
        Field("px", "decimal").scalar(Decimal("0.1234567890123456789"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const px = DataType.from('decimal').scalar('82.5')
    assert.equal(px.id, 'decimal')
    assert.ok(px.equals(Scalar.decimal(825n, 1)))

    // The value door refuses a nineteenth fractional digit.
    assert.throws(() => DataType.from('decimal').scalar(Scalar.decimal(1n, 19)))
    ```

### Arrow storage

A `decimal` column is `Decimal128(38, 18)` under the `yggdryl.decimal` extension name with an empty document, and a field carrying the name imports back as the leaf. Bare `Decimal128(38, 18)` storage imports as the parameterized width, and the cast between the two is the identity; `yggdryl.decimal` over any other storage imports as that storage. Where a format has no extension - Avro, Iceberg, Spark, Polars, pandas - the leaf crosses as their `decimal(38, 18)` and the name stays behind.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, DataType, Decimal, Field, Scalar, Serie};

    let px = Field::new("px", DataType::Decimal, false);
    let arrow = px.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &arrow_schema::DataType::Decimal128(38, 18));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.decimal");
    assert_eq!(Field::from_arrow_field(&arrow)?, px);

    // Bare storage is the width, and casts onto the leaf by identity.
    assert_eq!(
        DataType::from_arrow_datatype(&arrow_schema::DataType::Decimal128(38, 18))?,
        DataType::DECIMAL,
    );
    let value = Scalar::Decimal("82.5".parse::<Decimal>()?);
    let stored = Serie::from_scalars(Field::new("px", DataType::DECIMAL, false), [value.clone()])?;
    assert_eq!(stored.cast(&px, ArrowCastOptions::new())?.scalar(0)?, value);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    px = Field("px", "decimal", nullable=False)
    arrow = px.into_arrow()
    assert arrow.type == pa.decimal128(38, 18)
    assert arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.decimal"
    assert Field.from_arrow(arrow) == px

    # Bare storage is the width.
    bare = Field.from_arrow(pa.field("px", pa.decimal128(38, 18), nullable=False))
    assert str(bare.dtype) == "decimal128(38,18)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, Serie, fields } = require('yggdryl')

    const px = fields.decimal('px', { nullable: false })
    const batch = Serie.fromScalars(px, [Scalar.decimal(825n, 1)]).intoArrowBatch()
    const column = batch.schema.fields[0]
    assert.equal(column.type.precision, 38)
    assert.equal(column.type.scale, 18)
    assert.equal(column.metadata.get('ARROW:extension:name'), 'yggdryl.decimal')
    ```

## BigDecimal { #bigdecimal }

`bigdecimal` is the wide twin of [`decimal`](#decimal): the same scale, so every `Decimal` widens into it losslessly, and seventy-six digits for the notional a book of them sums to.

### Contract

| Aspect | Rule |
| --- | --- |
| Value | `yggdryl::BigDecimal`: one 256-bit integer of units at scale eighteen, bounded to seventy-six digits - `decimal256(76, 18)` preapplied; add, subtract and compare are the integer's, and multiply and divide run through a 512-bit product so the one truncation is the scale's |
| Arithmetic | the four checked operations answer `None` past seventy-six digits, `checked_div` also for a divisor of nothing; the operators refuse an overflow |
| Text | `BigDecimal::parse` reads exactly as `Decimal::parse` does, with seventy-six digits to fill; the value door is as strict as the narrow leaf's |
| Width | `Decimal::widened()` and `From<Decimal>` come in losslessly; `narrowed()` answers the `Decimal` within thirty-eight digits and `None` past them |
| Identity | `DataTypeId::BigDecimal`, `0x2e`, inside the decimal family's range |

### DataType

=== "Rust"

    ```rust
    use yggdryl::{BigDecimal, DataType, DataTypeId, DataTypeKind};

    let wide: DataType = "bigdecimal".parse()?;
    assert_eq!(wide, DataType::BigDecimal);
    assert_eq!(wide.to_string(), "bigdecimal");
    assert_eq!(DataTypeId::BigDecimal.as_u8(), 0x2e);
    assert!(DataTypeKind::Decimal.contains(wide.id()));
    assert_eq!(BigDecimal::dtype(), wide);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    wide = DataType("bigdecimal")
    assert wide.id == "bigdecimal"
    assert str(wide) == "bigdecimal"
    assert wide.kind == "decimal"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const wide = DataType.from('bigdecimal')
    assert.equal(wide.id, 'bigdecimal')
    assert.equal(wide.toString(), 'bigdecimal')
    assert.equal(wide.kind, 'decimal')
    ```

### Field

`DecimalField` with the payload `DecimalType::BigDecimal`; Python's `yggdryl.bigdecimal(name)` answers a `BigDecimalField`, JavaScript's is `fields.bigdecimal(name)`.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DecimalField, DecimalType};

    let notional = DecimalField::new("notional", DecimalType::BigDecimal, false);
    assert_eq!(notional.dtype(), &DataType::BigDecimal);
    assert_eq!((DecimalType::BigDecimal.precision(), DecimalType::BigDecimal.scale()), (76, 18));
    ```

=== "Python"

    ```python
    import yggdryl

    notional = yggdryl.bigdecimal("notional", nullable=False)
    assert str(notional.dtype) == "bigdecimal"
    assert not notional.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const notional = fields.bigdecimal('notional', { nullable: false })
    assert.equal(notional.dtype.toString(), 'bigdecimal')
    assert.equal(notional.nullable, false)
    ```

### Scalar

`Scalar::BigDecimal` holds the value under the wire tag `bigdecimal`, and, like the narrow leaf, equals, orders, hashes and digests as the `d256` or `d128` twin naming the same number.

=== "Rust"

    ```rust
    use yggdryl::{BigDecimal, Decimal, Scalar};

    let notional: BigDecimal = "123456789012345678901234567890.5".parse()?;
    let px = Decimal::from_int(3).widened();
    assert_eq!((notional * px).to_string(), "370370367037037036703703703671.5");
    assert_eq!((notional / px).to_string(), "41152263004115226300411522630.166666666666666666");
    assert_eq!(px.narrowed(), Some(Decimal::from_int(3)));
    assert_eq!(notional.narrowed(), None, "forty-eight digits do not narrow");
    assert_eq!(BigDecimal::MAX.checked_add(BigDecimal::ONE), None);

    let value = Scalar::from(notional);
    assert_eq!(value, Scalar::BigDecimal(notional));
    assert_eq!(value.kind(), "bigdecimal");
    // A narrow leaf meeting a wide one answers the wide one.
    let product = Scalar::from(Decimal::from_int(2)).checked_mul(&Scalar::from(px))?;
    assert!(matches!(product, Scalar::BigDecimal(_)));
    assert_eq!(product, Scalar::from(BigDecimal::from_int(6)));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import DataType

    notional = DataType("bigdecimal").scalar(Decimal("123456789012345678901234567890.5"))
    assert notional.kind == "bigdecimal"
    assert notional.as_py() == Decimal("123456789012345678901234567890.5")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const notional = DataType.from('bigdecimal').scalar('1.5')
    assert.equal(notional.id, 'bigdecimal')
    assert.ok(notional.equals(Scalar.decimal(15n, 1)))
    ```

### Arrow storage

A `bigdecimal` column is `Decimal256(76, 18)` under `yggdryl.bigdecimal`, bare `Decimal256(76, 18)` importing as the parameterized width. A cast from `decimal` widens every value; one back narrows under the cast's [`safe`](../cast.md) rule. Avro, Iceberg, Spark, Polars and pandas state at most thirty-eight digits, so a `bigdecimal` column is refused by each of them by name rather than narrowed.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, BigDecimal, DataType, Decimal, Field, Scalar, Serie};

    let notional = Field::new("notional", DataType::BigDecimal, false);
    let arrow = notional.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &arrow_schema::DataType::Decimal256(76, 18));
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.bigdecimal");
    assert_eq!(Field::from_arrow_field(&arrow)?, notional);

    // A decimal column widens, value for value.
    let px: Decimal = "82.5".parse()?;
    let narrow = Serie::from_scalars(Field::new("notional", DataType::Decimal, false), [Scalar::Decimal(px)])?;
    let widened = narrow.cast(&notional, ArrowCastOptions::new())?;
    assert_eq!(widened.scalar(0)?, Scalar::BigDecimal(px.widened()));
    assert_eq!(BigDecimal::from(px), px.widened());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    notional = Field("notional", "bigdecimal", nullable=False)
    arrow = notional.into_arrow()
    assert arrow.type == pa.decimal256(76, 18)
    assert arrow.metadata[b"ARROW:extension:name"] == b"yggdryl.bigdecimal"
    assert Field.from_arrow(arrow) == notional
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, Serie, fields } = require('yggdryl')

    const notional = fields.bigdecimal('notional', { nullable: false })
    const batch = Serie.fromScalars(notional, [Scalar.decimal(15n, 1)]).intoArrowBatch()
    const column = batch.schema.fields[0]
    assert.equal(column.type.precision, 76)
    assert.equal(column.type.scale, 18)
    assert.equal(column.metadata.get('ARROW:extension:name'), 'yggdryl.bigdecimal')
    ```

## Edges

- `decimal(2, 3)` -> refused, `positive scale cannot exceed precision: 3 > 2`; a negative scale is accepted (`decimal256(39,-4)`).
- `decimal(0, 0)` -> `precision must be between 1 and 9: 0`; `decimal128(39, 0)` -> `precision must be between 1 and 38: 39`; `decimal(77, 0)` -> the `1 and 76` bound of `decimal256`.
- Python `DataType.decimal(True, 0)` -> `TypeError`; `DataType.decimal("18.0", 2)` -> `ValueError`; `DataType.decimal(256, 0)` -> `OverflowError`; `__index__` objects and base-10 strings are accepted.
- An inexact quotient -> refused, not rounded; a quotient whose exact scale would exceed the width's maximum is the same refusal.
- [Merged](../field.md#merging-two-schemas) widening -> the widest backing either side declared: `decimal128(10,2)` beside `int16` stays `decimal128(10,2)`. Narrowing takes the backing the merged precision needs.
- A decimal beside a float -> refused; an exact number and an approximate one have no meeting point that is not a re-encoding.
- `Decimal::MAX.checked_add(Decimal::ONE)` and `BigDecimal::MAX.checked_add(BigDecimal::ONE)` -> `None`; the operator form refuses as the integers' do.
- `Decimal::parse("0.1234567890123456789")` -> truncated to eighteen digits; `DataType::Decimal.scalar` of the same number -> refused, because the value door restates exactly or not at all.
- A `decimal` or `bigdecimal` value under `%` -> refused: a fixed scale states no remainder. Beside a float -> refused, as every exact decimal is.
- `BigDecimal::narrowed()` past thirty-eight digits -> `None`; a `bigdecimal` column cast onto `decimal` narrows under the cast's `safe` rule.
- `yggdryl.decimal` or `yggdryl.bigdecimal` over another storage, or with a nonempty document -> imports as that storage, a foreign field wearing the name.
- A `bigdecimal` column into Avro, Iceberg, Spark, Polars or pandas -> refused by name; a `decimal` column crosses as their `decimal(38, 18)`.
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
    node --test --test-name-pattern="defaulted temporal and decimal overloads share exact option handling|the fixed decimal leaves" node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
