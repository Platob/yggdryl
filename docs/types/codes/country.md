# Country

ISO 3166-1 alpha-2, the two-letter country code: the narrowest of the seventeen, and the one a securities identifier opens with.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `country`, `CountryType`/`CountryField`, the `Country` value and `Scalar::Country` |
| Validates | US-ASCII, no NUL, at most two bytes; the ISO listing is the code's [rank](index.md#rank) and a declared vocabulary, never a gate |
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
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.country"
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

The width is the narrowest of the seventeen, and every path reads it from the datatype: a third byte is refused at the value door, and `ascii_packed` pads into two bytes rather than three.

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

`StringEnum::COUNTRIES` ships with the package under the logical name `country`, and Python declares it over the width as `yggdryl.enums.COUNTRY`, over the `yggdryl.enums.Country` base a caller subclasses for a vocabulary of its own. A declared vocabulary, never a gate ([packed integers](index.md#packed-integers-and-the-declared-vocabulary)). `Country::is_listed` answers whether ISO 3166-1 currently assigns a value: it is the [rank](index.md#rank) - one listed, zero for the user-assigned `XX` or a code the registry does not know - which a merge reads, so a listed country replaces an unlisted one whichever leads. Rust, and JavaScript's `isListed` getter on a `Country`.

=== "Rust"

    ```rust
    use yggdryl::{CodeValue, Country, StringEnum};

    let countries = StringEnum::from_logical_name("country")?;
    assert_eq!(countries.len(), StringEnum::COUNTRIES.len());
    assert_eq!(countries.get("US"), Some("US"));

    // The listing is the rank a merge reads, never a gate.
    let swiss = Country::new("CH")?;
    assert!(swiss.is_listed() && swiss.is_real());
    let masked = Country::new("XX")?;
    assert!(!masked.is_listed());
    assert_eq!(masked.rank(), 0);
    assert_eq!(masked.merge_with(&swiss), swiss);
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

## Currency

`Country::currency` answers the legal tender ISO 4217 list one gives a country - Python's `Country.currency` property, a `Ccy` member, JavaScript's `currency` getter, the code as text. One currency per country, a fund code never: where list one names two tenders the generator's override table settles which - the country's own for `BT`, `HT`, `LS`, `NA`, `PA` and `VE`, the US dollar for `EC`, `SV` and `TL`. None for a code list one gives no currency: the user-assigned `XX` and `ZZ`, an agency prefix (`XS`, `EU`), a country with no universal currency (`AQ`). The table is `rust/src/country/tables.rs`, generated by `python scripts/generate_country_currency.py` and never edited by hand. It is what an [instrument's listing](../../graph/instrument.md#derived-facts) defaults its currency to, through the country its market is in ([MIC](mic.md#operating-mic-and-country)).

=== "Rust"

    ```rust
    use yggdryl::{Ccy, Country};

    assert_eq!(Country::new("US")?.currency(), Some(Ccy::new("USD")?));
    assert_eq!(Country::new("CH")?.currency(), Some(Ccy::new("CHF")?));
    // Two tenders in list one, one answer: Ecuador's is the US dollar.
    assert_eq!(Country::new("EC")?.currency(), Some(Ccy::new("USD")?));
    // The user-assigned code and an agency prefix name no currency.
    assert_eq!(Country::new("XX")?.currency(), None);
    assert_eq!(Country::new("XS")?.currency(), None);
    ```

=== "Python"

    ```python
    from yggdryl.enums import Ccy, Country

    assert Country.from_str("US").currency == Ccy.from_str("USD")
    assert str(Country.from_str("CH").currency) == "CHF"
    # Two tenders in list one, one answer: Ecuador's is the US dollar.
    assert Country.from_str("EC").currency == Ccy.from_str("USD")
    # The user-assigned code and an agency prefix name no currency.
    assert Country.from_str("XX").currency is None
    assert Country.from_str("XS").currency is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Country } = require('yggdryl')

    assert.equal(new Country('US').currency, 'USD')
    assert.equal(new Country('CH').currency, 'CHF')
    // Two tenders in list one, one answer: Ecuador's is the US dollar.
    assert.equal(new Country('EC').currency, 'USD')
    // The user-assigned code and an agency prefix name no currency.
    assert.equal(new Country('XX').currency, null)
    assert.equal(new Country('XS').currency, null)
    ```

## Edges

- `at most 2 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- A listed country replaces an unlisted one - the user-assigned `XX`, a code the registry does not know - on a [merge](index.md#the-code-family-value), whichever leads; two of one rank keep this one.
- `country` beside [`ccy`](ccy.md) merges to `sized_ascii(8)` widening and `sized_ascii(2)` narrowing - the bounded text both fit.
- The two letters an [ISIN](isin.md) opens with are the numbering agency's prefix, which includes international prefixes such as `XS` that no country names; `Isin::prefix` reads them as text rather than as this code.
- The default value is the empty text, answered as a `country` scalar ([Cast](../cast.md#empty-text)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi::coded code::datatypes country:: string::listings
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
