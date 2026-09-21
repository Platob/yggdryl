# Currency

ISO 4217's three-letter currency code, and `XXX`, the currency that states none.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `currency`, `CurrencyType`/`CurrencyField`, the `Currency` value and `Scalar::Currency` |
| Validates | US-ASCII, no NUL, at most three bytes; the ISO listing is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A fourth byte or a byte past `0x7F`, naming the width: `at most 3 bytes` |

## DataType

`currency` is the one spelling, `DataType::currency()` the constructor, `DataTypeId::Currency` the id. The width bounds a value rather than laying one out, so `fixed_byte_width` is none and `fixed_ascii(3)` is a different datatype.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::currency(), DataType::Currency);
    assert_eq!(DataType::from_str("currency")?, DataType::Currency);
    assert_eq!(DataType::Currency.to_string(), "currency");
    assert_eq!(DataType::Currency.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Currency.code_name(), Some("currency"));
    assert_eq!(DataType::Currency.code_width(), Some(3));
    assert_eq!(DataType::Currency.fixed_byte_width(), None);
    assert_ne!(DataType::Currency, DataType::fixed_ascii(3)?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    currency = DataType("currency")
    assert currency.id == "currency"
    assert currency.kind == "code"
    assert currency.code_width == 3
    assert currency.fixed_byte_width is None
    # The logical name folds, and the datatype it names is this one.
    assert DataType.from_logical_name("Currency") == currency
    assert DataType(" CURRENCY ") == currency
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const currency = new DataType('currency')
    assert.equal(currency.id, 'currency')
    assert.equal(currency.kind, 'code')
    assert.equal(currency.codeWidth, 3)
    assert.equal(currency.fixedByteWidth, null)
    assert.ok(DataType.from(' CURRENCY ').equals(currency))
    ```

## Field

`CurrencyField` is the typed marker - a parameter-free leaf, so `unit(name, nullable)` is the whole constructor. Python and JavaScript name the factory `currency`.

=== "Rust"

    ```rust
    use yggdryl::{CurrencyField, DataType, Field};

    let ccy = CurrencyField::unit("ccy", false);
    assert_eq!(ccy.name(), "ccy");
    assert_eq!(ccy.dtype(), &DataType::Currency);
    assert!(!ccy.is_nullable());
    assert_eq!(ccy.to_field(), Field::new("ccy", DataType::Currency, false));

    // The leaf is the datatype's: a width of the same size is not a code.
    let plain = Field::new("ccy", DataType::fixed_ascii(3)?, false);
    assert!(CurrencyField::try_from_field(plain).is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    ccy = yggdryl.currency("ccy", nullable=False)
    assert isinstance(ccy, Field)
    assert ccy.name == "ccy"
    assert str(ccy.dtype) == "currency"
    assert not ccy.nullable
    assert ccy == Field("ccy", "currency", nullable=False)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const ccy = fields.currency('ccy', { nullable: false })
    assert.ok(ccy instanceof Field)
    assert.equal(ccy.name, 'ccy')
    assert.equal(ccy.dtype.toString(), 'currency')
    assert.equal(ccy.nullable, false)
    ```

## Scalar

The value is the text, under the currency's identity. `Currency::new` is the Rust door, `DataType::scalar` the one every binding shares; `as_str` reads it back.

=== "Rust"

    ```rust
    use yggdryl::{Currency, DataType, Scalar};

    let usd = DataType::currency().scalar("USD")?;
    assert_eq!(usd, Scalar::Currency(Currency::new("USD")?));
    assert_eq!(usd.as_str(), Some("USD"));
    assert_eq!(usd.kind(), "currency");

    // A code and the text that spells it are two values.
    assert_ne!(usd, Scalar::from("USD"));
    // The fourth byte is refused, naming the width.
    let refused = Currency::new("USDX").unwrap_err().to_string();
    assert!(refused.contains("at most 3 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    usd = DataType("currency").scalar("USD")
    assert usd.as_py() == "USD"
    assert usd.kind == "currency"
    assert usd != DataType("utf8").scalar("USD")

    with pytest.raises(ValueError, match="at most 3 bytes"):
        DataType("currency").scalar("USDX")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const usd = new DataType('currency').scalar('USD')
    assert.equal(usd.asJs(), 'USD')
    assert.equal(usd.kind, 'currency')
    assert.throws(() => new DataType('currency').scalar('USDX'), /at most 3 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.currency`: the storage is the text, the name beside it is the identity ([Codes](index.md#arrow-storage)). The same text under the string family's name is a bounded string, and under no name at all is plain text.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let ccy = Field::new("ccy", DataType::Currency, false);
    let arrow = ccy.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.currency");
    assert_eq!(Field::from_arrow_field(&arrow)?, ccy);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    ccy = Field("ccy", "currency")
    arrow_field = ccy.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.currency",
        b"ARROW:extension:metadata": b"",
    }
    assert Field.from_arrow(arrow_field) == ccy

    # The same text under the string family's name is a bounded string, and
    # under no name at all is plain text.
    bounded = Field("ccy", DataType("ascii(3)"))
    assert Field.from_arrow(bounded.into_arrow()) == bounded
    assert Field.from_arrow(pa.field("ccy", pa.string())) == Field("ccy", "utf8")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // The column holds the text itself: nothing is padded, so nothing is
    // trimmed back.
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual([...fields.currency('ccy').castArrowArray(utf8(['USD', 'EUR']))], ['USD', 'EUR'])
    ```

## `XXX` states no currency

ISO 4217 publishes `XXX` for "no currency", so it is what a merge takes the other side over: `Currency::none()` is that value, and [`merge_with`](index.md#the-code-family-value) folds two statements of one price's currency into the better one. Rust only.

```rust
use yggdryl::{CodeValue, Currency};

assert_eq!(Currency::none().as_str(), "XXX");
assert_eq!(Currency::none().merge_with(&Currency::new("USD")?).as_str(), "USD");
// Anything stated stands, whatever the other side says.
assert_eq!(Currency::new("USD")?.merge_with(&Currency::new("EUR")?).as_str(), "USD");
```

## The ISO 4217 listing

The listing ships with the package as `StringEnum::CURRENCIES`, reached by the logical name `currency`. It is a declared vocabulary, never a gate: a column typed `currency` holds any three-byte ASCII value, and the enum is what a field *declares* its values are drawn from ([packed integers](index.md#packed-integers-and-the-declared-vocabulary)).

=== "Rust"

    ```rust
    use yggdryl::StringEnum;

    let currencies = StringEnum::from_logical_name("currency")?;
    assert_eq!(currencies.len(), StringEnum::CURRENCIES.len());
    assert_eq!(currencies.get("USD"), Some("USD"));
    ```

=== "Python"

    ```python
    from yggdryl import StringEnum

    currencies = StringEnum.from_logical_name("currency")
    assert len(currencies) == len(StringEnum.prebuilt()["currency"])
    assert currencies.get("USD") == "USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { StringEnum } = require('yggdryl')

    const currencies = StringEnum.fromLogicalName('currency')
    assert.equal(currencies.length, StringEnum.prebuilt().currency.length)
    assert.equal(currencies.get('USD'), 'USD')
    ```

Python declares a vocabulary over the width as well: `yggdryl.enums.Currency` is the registered base, whose members are their own storage bytes read big-endian.

=== "Python"

    ```python
    from yggdryl.enums import Currency

    assert int(Currency.USD) == 0x555344
    assert str(Currency.USD) == "USD"
    ```

## Edges

- `at most 3 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- `ascii_packed("USD")` is the same integer a `fixed_ascii(3)` packs, `0x555344`; the padding belongs to the packing, never to the column.
- The default value is the empty text, answered as a `currency` scalar, and an empty text cell entering the column reads as it ([Cast](../cast.md#empty-text)).
- `currency` beside [`country`](country.md) merges to `sized_ascii(3)` widening and `sized_ascii(2)` narrowing - the bounded text both fit, never one code holding the other's values.
- A value no ISO listing names is stored: the listing is a vocabulary, and [Side](side.md) and [State](state.md) are the two codes that gate instead.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::coded vocabulary::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "registered_code"
    python/.venv/bin/python -m pytest python/tests/test_enums.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/types/datatype.test.js
    ```
