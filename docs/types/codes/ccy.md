# Ccy

ISO 4217's three-letter currency code, and `XXX`, the currency that states none.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `ccy`, `CcyType`/`CcyField`, the `Ccy` value and `Scalar::Ccy` |
| Validates | US-ASCII, no NUL, at most three bytes; the ISO listing is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A fourth byte or a byte past `0x7F`, naming the width: `at most 3 bytes` |

## DataType

`ccy` is the one spelling, `DataType::ccy()` the constructor, `DataTypeId::Ccy` the id. The width bounds a value rather than laying one out, so `fixed_byte_width` is none and `fixed_ascii(3)` is a different datatype.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::ccy(), DataType::Ccy);
    assert_eq!(DataType::from_str("ccy")?, DataType::Ccy);
    assert_eq!(DataType::Ccy.to_string(), "ccy");
    assert_eq!(DataType::Ccy.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Ccy.code_name(), Some("ccy"));
    assert_eq!(DataType::Ccy.code_width(), Some(3));
    assert_eq!(DataType::Ccy.fixed_byte_width(), None);
    assert_ne!(DataType::Ccy, DataType::fixed_ascii(3)?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    currency = DataType("ccy")
    assert currency.id == "ccy"
    assert currency.kind == "code"
    assert currency.code_width == 3
    assert currency.fixed_byte_width is None
    # The logical name folds, and the datatype it names is this one.
    assert DataType.from_logical_name("Ccy") == currency
    assert DataType(" CCY ") == currency
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const currency = new DataType('ccy')
    assert.equal(currency.id, 'ccy')
    assert.equal(currency.kind, 'code')
    assert.equal(currency.codeWidth, 3)
    assert.equal(currency.fixedByteWidth, null)
    assert.ok(DataType.from(' CCY ').equals(currency))
    ```

## Field

`CcyField` is the typed marker - a parameter-free leaf, so `unit(name, nullable)` is the whole constructor. Python and JavaScript name the factory `ccy`.

=== "Rust"

    ```rust
    use yggdryl::{CcyField, DataType, Field};

    let ccy = CcyField::unit("ccy", false);
    assert_eq!(ccy.name(), "ccy");
    assert_eq!(ccy.dtype(), &DataType::Ccy);
    assert!(!ccy.is_nullable());
    assert_eq!(ccy.to_field(), Field::new("ccy", DataType::Ccy, false));

    // The leaf is the datatype's: a width of the same size is not a code.
    let plain = Field::new("ccy", DataType::fixed_ascii(3)?, false);
    assert!(CcyField::try_from_field(plain).is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    ccy = yggdryl.ccy("ccy", nullable=False)
    assert isinstance(ccy, Field)
    assert ccy.name == "ccy"
    assert str(ccy.dtype) == "ccy"
    assert not ccy.nullable
    assert ccy == Field("ccy", "ccy", nullable=False)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const ccy = fields.ccy('ccy', { nullable: false })
    assert.ok(ccy instanceof Field)
    assert.equal(ccy.name, 'ccy')
    assert.equal(ccy.dtype.toString(), 'ccy')
    assert.equal(ccy.nullable, false)
    ```

## Scalar

The value is the text, under the currency's identity. `Ccy::new` is the Rust door, `DataType::scalar` the one every binding shares; `as_str` reads it back.

=== "Rust"

    ```rust
    use yggdryl::{Ccy, DataType, Scalar};

    let usd = DataType::ccy().scalar("USD")?;
    assert_eq!(usd, Scalar::Ccy(Ccy::new("USD")?));
    assert_eq!(usd.as_str(), Some("USD"));
    assert_eq!(usd.kind(), "ccy");

    // A code and the text that spells it are two values.
    assert_ne!(usd, Scalar::from("USD"));
    // The fourth byte is refused, naming the width.
    let refused = Ccy::new("USDX").unwrap_err().to_string();
    assert!(refused.contains("at most 3 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    usd = DataType("ccy").scalar("USD")
    assert usd.as_py() == "USD"
    assert usd.kind == "ccy"
    assert usd != DataType("utf8").scalar("USD")

    with pytest.raises(ValueError, match="at most 3 bytes"):
        DataType("ccy").scalar("USDX")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const usd = new DataType('ccy').scalar('USD')
    assert.equal(usd.asJs(), 'USD')
    assert.equal(usd.kind, 'ccy')
    assert.throws(() => new DataType('ccy').scalar('USDX'), /at most 3 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.ccy`: the storage is the text, the name beside it is the identity ([Codes](index.md#arrow-storage)). The same text under the string family's name is a bounded string, and under no name at all is plain text.

The retired `yggdryl.currency` name is refused, including over dictionary storage; it is never read as `utf8` or as `ccy`.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let ccy = Field::new("ccy", DataType::Ccy, false);
    let arrow = ccy.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ccy");
    assert_eq!(Field::from_arrow_field(&arrow)?, ccy);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    ccy = Field("ccy", "ccy")
    arrow_field = ccy.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata == {
        b"ARROW:extension:name": b"yggdryl.ccy",
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
    const { Serie, fields } = require('yggdryl')

    // The column holds the text itself: nothing is padded, so nothing is
    // trimmed back.
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['USD', 'EUR']), fields.ccy('ccy'))
    assert.deepEqual([...stored.intoArrowArray()], ['USD', 'EUR'])
    ```

## `XXX` states no currency

ISO 4217 publishes `XXX` for "no currency", so it is what a merge takes the other side over: `Ccy::none()` is that value, and [`merge_with`](index.md#the-code-family-value) folds two statements of one price's currency into the better one. Rust only.

```rust
use yggdryl::{CodeValue, Ccy};

assert_eq!(Ccy::none().as_str(), "XXX");
assert_eq!(Ccy::none().merge_with(&Ccy::new("USD")?).as_str(), "USD");
// Anything stated stands, whatever the other side says.
assert_eq!(Ccy::new("USD")?.merge_with(&Ccy::new("EUR")?).as_str(), "USD");
```

## The ISO 4217 listing

The listing ships with the package as `StringEnum::CURRENCIES`, reached by the logical name `ccy`. It is a declared vocabulary, never a gate: a column typed `ccy` holds any three-byte ASCII value, and the enum is what a field *declares* its values are drawn from ([packed integers](index.md#packed-integers-and-the-declared-vocabulary)).

=== "Rust"

    ```rust
    use yggdryl::StringEnum;

    let currencies = StringEnum::from_logical_name("ccy")?;
    assert_eq!(currencies.len(), StringEnum::CURRENCIES.len());
    assert_eq!(currencies.get("USD"), Some("USD"));
    ```

=== "Python"

    ```python
    from yggdryl import StringEnum

    currencies = StringEnum.from_logical_name("ccy")
    assert len(currencies) == len(StringEnum.prebuilt()["ccy"])
    assert currencies.get("USD") == "USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { StringEnum } = require('yggdryl')

    const currencies = StringEnum.fromLogicalName('ccy')
    assert.equal(currencies.length, StringEnum.prebuilt().ccy.length)
    assert.equal(currencies.get('USD'), 'USD')
    ```

Python declares a vocabulary over the width as well: `yggdryl.enums.Ccy` is the registered base, whose members are their own storage bytes read big-endian.

=== "Python"

    ```python
    from yggdryl.enums import Ccy

    assert int(Ccy.USD) == 0x555344
    assert str(Ccy.USD) == "USD"
    ```

## Edges

- `at most 3 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- `ascii_packed("USD")` is the same integer a `fixed_ascii(3)` packs, `0x555344`; the padding belongs to the packing, never to the column.
- The default value is the empty text, answered as a `ccy` scalar, and an empty text cell entering the column reads as it ([Cast](../cast.md#empty-text)).
- `ccy` beside [`country`](country.md) merges to `sized_ascii(3)` widening and `sized_ascii(2)` narrowing - the bounded text both fit, never one code holding the other's values.
- A value no ISO listing names is stored: the listing is a vocabulary, and [Side](side.md) and [State](state.md) are the two codes that gate instead.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi_code::coded code::datatypes state::coded string::listings timeinforce::coded
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    python/.venv/bin/python -m pytest python/tests/enums/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
