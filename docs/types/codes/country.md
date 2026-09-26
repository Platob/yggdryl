# Country

ISO 3166-1 alpha-2, the two-letter country code: the narrowest of the fourteen, and the one a securities identifier opens with.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `country`, `CountryType`/`CountryField`, the `Country` value and `Scalar::Country` |
| Validates | US-ASCII, no NUL, at most two bytes; the ISO listing is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A third byte or a byte past `0x7F`, naming the width: `at most 2 bytes` |

## DataType

`country` is the one spelling, `DataType::country()` the constructor, `DataTypeId::Country` the id.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::country(), DataType::Country);
    assert_eq!(DataType::from_str("country")?, DataType::Country);
    assert_eq!(DataType::Country.to_string(), "country");
    assert_eq!(DataType::Country.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Country.code_name(), Some("country"));
    assert_eq!(DataType::Country.code_width(), Some(2));
    assert_eq!(DataType::Country.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    country = DataType("country")
    assert country.id == "country"
    assert country.kind == "code"
    assert country.code_width == 2
    assert country.fixed_byte_width is None
    assert DataType.from_logical_name("Country") == country
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const country = new DataType('country')
    assert.equal(country.id, 'country')
    assert.equal(country.kind, 'code')
    assert.equal(country.codeWidth, 2)
    assert.equal(country.fixedByteWidth, null)
    ```

## Field

`CountryField` is the typed marker; Python and JavaScript name the factory `country`.

=== "Rust"

    ```rust
    use yggdryl::{CountryField, DataType, Field};

    let iso = CountryField::unit("iso", true);
    assert_eq!(iso.dtype(), &DataType::Country);
    assert!(iso.is_nullable());
    assert_eq!(iso.to_field(), Field::new("iso", DataType::Country, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    iso = yggdryl.country("iso")
    assert isinstance(iso, Field)
    assert str(iso.dtype) == "country"
    assert iso.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const iso = fields.country('iso')
    assert.equal(iso.dtype.toString(), 'country')
    assert.equal(iso.nullable, true)
    ```

## Scalar

The value is the two letters, under the country's identity.

=== "Rust"

    ```rust
    use yggdryl::{Country, DataType, Scalar};

    let france = DataType::country().scalar("FR")?;
    assert_eq!(france, Scalar::Country(Country::new("FR")?));
    assert_eq!(france.as_str(), Some("FR"));
    assert_eq!(france.kind(), "country");

    // A country and a currency of alike bytes are two values.
    assert_ne!(DataType::Country.scalar("US")?, DataType::Ccy.scalar("US")?);
    let refused = Country::new("FRA").unwrap_err().to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    france = DataType("country").scalar("FR")
    assert france.as_py() == "FR"
    assert france.kind == "country"
    assert DataType("country").scalar("US") != DataType("ccy").scalar("US")

    with pytest.raises(ValueError, match="at most 2 bytes"):
        DataType("country").scalar("FRA")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const france = new DataType('country').scalar('FR')
    assert.equal(france.asJs(), 'FR')
    assert.equal(france.kind, 'country')
    assert.throws(() => new DataType('country').scalar('FRA'), /at most 2 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.country` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let iso = Field::new("iso", DataType::Country, false);
    let arrow = iso.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.country");
    assert_eq!(Field::from_arrow_field(&arrow)?, iso);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    iso = Field("iso", "country")
    arrow_field = iso.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.country"
    assert Field.from_arrow(arrow_field) == iso
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['FR', 'US']), fields.country('iso'))
    assert.deepEqual([...stored.intoArrowArray()], ['FR', 'US'])
    ```

## Two bytes is the whole width

The width is the narrowest of the fourteen, and every path reads it from the datatype: a third byte is refused at the value door, and `ascii_packed` pads into two bytes rather than three.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::Country.ascii_packed(b"FR")?, 0x4652);
    assert_eq!(DataType::Country.ascii_value(0x4652)?, "FR");
    assert!(DataType::Country.ascii_packed(b"USD").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    assert DataType("country").ascii_packed("FR") == 0x4652
    assert DataType("country").ascii_value(0x4652) == "FR"
    with pytest.raises(ValueError, match="at most 2 bytes"):
        DataType("country").ascii_packed("USD")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(new DataType('country').asciiPacked('FR'), 0x4652n)
    assert.equal(new DataType('country').asciiValue(0x4652n), 'FR')
    assert.throws(() => new DataType('country').asciiPacked('USD'), /at most 2 bytes/)
    ```

## The ISO 3166 listing

`StringEnum::COUNTRIES` ships with the package under the logical name `country`, and Python declares it over the width as `yggdryl.enums.COUNTRY`, over the `yggdryl.enums.Country` base a caller subclasses for a vocabulary of its own. A declared vocabulary, never a gate ([packed integers](index.md#packed-integers-and-the-declared-vocabulary)).

=== "Rust"

    ```rust
    use yggdryl::StringEnum;

    let countries = StringEnum::from_logical_name("country")?;
    assert_eq!(countries.len(), StringEnum::COUNTRIES.len());
    assert_eq!(countries.get("US"), Some("US"));
    ```

=== "Python"

    ```python
    from yggdryl import StringEnum
    from yggdryl.enums import COUNTRY

    countries = StringEnum.from_logical_name("country")
    assert len(countries) == len(StringEnum.prebuilt()["country"])
    assert countries.get("US") == "US"

    # A member is its value's own storage bytes, read big-endian.
    assert int(COUNTRY.US) == 0x5553
    assert str(COUNTRY.US) == "US"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { StringEnum } = require('yggdryl')

    const countries = StringEnum.fromLogicalName('country')
    assert.equal(countries.length, StringEnum.prebuilt().country.length)
    assert.equal(countries.get('US'), 'US')
    ```

## Edges

- `at most 2 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- A country has no value stating none, so nothing is taken over on a [merge](index.md#the-code-family-value): this one stands.
- `country` beside [`ccy`](ccy.md) merges to `sized_ascii(3)` widening and `sized_ascii(2)` narrowing - the bounded text both fit.
- The two letters an [ISIN](isin.md) opens with are the numbering agency's prefix, which includes international prefixes such as `XS` that no country names; `Isin::prefix` reads them as text rather than as this code.
- The default value is the empty text, answered as a `country` scalar ([Cast](../cast.md#empty-text)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi::coded code::datatypes state::coded string::listings timeinforce::coded
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
