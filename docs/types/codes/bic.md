# BIC

ISO 9362's Business Identifier Code: eight or eleven characters - a party prefix, a country, a location and an optional branch - held by its shape as stated, ranked by whether its country is one.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `bic`, `BicType`/`BicField`, the `Bic` value and `Scalar::Bic` |
| Validates | The shape: eight or eleven ASCII bytes - four letters or digits, two letters, two letters or digits, and an optional three letters or digits; lower case folds at the value door. Whether the country is one ISO 3166 lists is its [rank](index.md#rank), never a gate |
| Lazy | Nothing - the accepted eleven bytes stay inline, so the constructor, the clone and the shared field allocate nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A length other than eight or eleven, a digit in the country, punctuation anywhere; the empty text, so there is no default value |

A BIC names a party - a bank, an institution, one of its branches - never an instrument, and it carries no check character.

## DataType

`bic` is the one spelling, `DataType::bic()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::bic(), DataType::Bic);
    assert_eq!(DataType::from_str("bic")?, DataType::Bic);
    assert_eq!(DataType::Bic.to_string(), "bic");
    assert_eq!(DataType::Bic.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Bic.code_name(), Some("bic"));
    assert_eq!(DataType::Bic.code_width(), Some(11));
    assert_eq!(DataType::Bic.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    bic = DataType("bic")
    assert (bic.id, bic.code_width, bic.kind) == ("bic", 11, "code")
    assert bic.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bic = DataType.fromString('bic')
    assert.equal(bic.id, 'bic')
    assert.equal(bic.kind, 'code')
    assert.equal(bic.codeWidth, 11)
    assert.equal(bic.fixedByteWidth, null)
    ```

## Field

`BicField` is the typed marker; Python and JavaScript name the factory `bic`.

=== "Rust"

    ```rust
    use yggdryl::{BicField, DataType, Field};

    let party = BicField::unit("bic", false);
    assert_eq!(party.dtype(), &DataType::Bic);
    assert_eq!(party.to_field(), Field::new("bic", DataType::Bic, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.bic("bic", nullable=False).dtype == DataType("bic")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.bic('bic', { nullable: false }).dtype.id, 'bic')
    ```

## Scalar

The value is the canonical spelling: upper case, at the length it was stated. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{Bic, DataType, Scalar};

    let office = Bic::new("deutdeff500")?;
    assert_eq!(office.as_str(), "DEUTDEFF500");
    assert_eq!(DataType::bic().scalar("DEUTDEFF500")?, Scalar::Bic(office));
    assert_eq!(DataType::bic().scalar("DEUTDEFF500")?.kind(), "bic");
    // Eight characters are kept as eight: no branch is invented.
    assert_eq!(Bic::new("DEUTDEFF")?.as_str(), "DEUTDEFF");
    // The shape is the refusal.
    assert!(Bic::new("DEUT1EFF").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    bic = DataType("bic")
    assert bic.scalar("deutdeff500").as_py() == "DEUTDEFF500"
    assert bic.scalar("deutdeff500").kind == "bic"

    # Eight characters are kept as eight: no branch is invented.
    assert bic.scalar("DEUTDEFF").as_py() == "DEUTDEFF"
    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected a two-letter country code"):
        bic.scalar("DEUT1EFF")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bic = DataType.fromString('bic')
    assert.equal(bic.scalar('deutdeff500').asJs(), 'DEUTDEFF500')
    // Eight characters are kept as eight: no branch is invented.
    assert.equal(bic.scalar('DEUTDEFF').asJs(), 'DEUTDEFF')
    // The shape is the refusal.
    assert.throws(() => bic.scalar('DEUT1EFF'), /expected a two-letter country code/)
    ```

## Arrow storage

`Utf8` under `yggdryl.bic` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let party = Field::new("bic", DataType::Bic, false);
    let arrow = party.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.bic");
    assert_eq!(Field::from_arrow_field(&arrow)?, party);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    party = Field("bic", "bic")
    arrow_field = party.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.bic"
    assert Field.from_arrow(arrow_field) == party
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['DEUTDEFF', 'DEUTDEFF500']), fields.bic('bic'))
    assert.deepEqual([...stored.intoArrowArray()], ['DEUTDEFF', 'DEUTDEFF500'])
    ```

## The four parts, and the branch

A BIC reads in four parts: the business party prefix, four letters or digits (`party_prefix`); the country, two letters (`country`); the business party suffix naming the location, two letters or digits (`location`); and, on eleven characters, the branch, three letters or digits (`branch`, `None` on eight). ISO 9362:2009 wrote the prefix in letters alone; ISO 9362:2014 opened it to digits, so `1234DEFF` is the shape. Eight characters and eleven ending `XXX` both name the party's primary office (`is_primary_office`), and the two are kept as stated rather than folded into one: `DEUTDEFF` and `DEUTDEFFXXX` are two values, as they are two spellings on the wire. There is no check character, so the [rank](index.md#rank) is the country: one where ISO 3166 lists it or where it is `XK`, which SWIFT assigns for Kosovo and ISO 3166 does not, zero for any other - an unlisted country is a value of a lower rank, never a refusal. `is_canonical` answers whether text is the canonical spelling, upper case and the shape. Rust only.

```rust
use yggdryl::{Bic, CodeValue};

let office = Bic::new("DEUTDEFF500")?;
assert_eq!(office.party_prefix(), "DEUT");
assert_eq!(office.country(), "DE");
assert_eq!(office.location(), "FF");
assert_eq!(office.branch(), Some("500"));
assert!(!office.is_primary_office());
assert!(office.is_real());

// Eight characters, or the branch `XXX`, name the primary office; the two
// spellings stay two values.
let primary = Bic::new("DEUTDEFF")?;
assert_eq!(primary.branch(), None);
assert!(primary.is_primary_office());
assert!(Bic::new("DEUTDEFFXXX")?.is_primary_office());
assert_ne!(primary, Bic::new("DEUTDEFFXXX")?);

// Digits in the party prefix are the shape since ISO 9362:2014.
assert_eq!(Bic::new("1234DEFF")?.party_prefix(), "1234");
assert!(Bic::is_canonical("DEUTDEFF500"));
assert!(!Bic::is_canonical("deutdeff500"));

// An unlisted country is a value of rank zero, which a listed one replaces.
let unlisted = Bic::new("DEUTZZFF")?;
assert_eq!(unlisted.rank(), 0);
assert_eq!(unlisted.merge_with(&primary), primary);

// Nine characters, a digit in the country and punctuation are not the shape.
for refused in ["DEUTDEFF5", "DEUT1EFF", "DEU-DEFF"] {
    assert!(Bic::new(refused).is_err(), "{refused}");
}
```

## Edges

- `expected eight or eleven characters`, `expected a four-character party prefix of letters or digits`, `expected a two-letter country code`, `expected a two-character location of letters or digits`, `expected a three-character branch of letters or digits` - the five refusals, each naming `bic` and the spelling it saw; an unlisted country is a rank, never a refusal.
- A twelfth byte -> `at most 11 bytes`, the refusal any code of that width gives.
- No folding: `DEUTDEFF` and `DEUTDEFFXXX` are two values, equal neither under `==` nor in a join key, and a cast keeps each as it was written.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md) and [FIGI](figi.md) are: a lower-case spelling is null under the default `safe`.
- Serde reads a BIC through the same door: a document holding a spelling that is not the shape is refused rather than deserialized, and lower case is folded.
- No default value: the empty text names no party, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- [`merge_with`](index.md#the-code-family-value) takes a code whose country is listed over one whose country is not, whichever leads; two of one rank keep this one.
- A party id stated under the `bic` source - `PartyIDSource(447)` `B`, `AcctIDSource(660)` `1` - is held to a BIC's shape, upper-cased, whatever its role: a value of another shape is no [identifier](../../graph/identifier.md), refused on its key (`a value under the bic source is a BIC: ...`), and a FIX party it refuses stays on the wire as an anomaly of its `partyid`, `rootpartyid` or `account`; its rank is the lower of its role's and the BIC's own, so a BIC of a listed country replaces one of an unlisted country under the same key and as the role's answer, whichever was stated first ([Under a source](../../graph/identifier.md#under-a-source)).

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- bic::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "reference_data_code or keep_what_their_standards_leave_open"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="reference-data codes" node/tests/datatype.test.js
    ```
