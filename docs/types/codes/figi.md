# FIGI

ANSI X9.145's Financial Instrument Global Identifier: twelve characters, a consonant prefix, `G`, and the check digit that closes them.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `figi`, `FIGICodeType`/`FIGICodeField`, the `FIGICode` value and `Scalar::FIGICode` |
| Validates | Twelve ASCII bytes: two consonants that are not a reserved prefix, `G`, eight consonants or digits, one decimal check digit; lower case folds at the value door |
| Lazy | Nothing - the accepted twelve bytes stay inline, so the constructor, the clone and the shared field allocate nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A bad prefix, a reserved prefix, a third character that is not `G`, a vowel or punctuation in the body, and a check digit that does not close it; the empty text, so there is no default value |

A FIGI is a checked identity of its own, never a [Bloomberg](bloomberg.md) fallback: `SecurityIDSource(22)=S` lifts a valid FIGI, and source `A` remains Bloomberg.

## DataType

`figi` is the one spelling, `DataType::figi()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::figi(), DataType::FIGICode);
    assert_eq!(DataType::from_str("figi")?, DataType::FIGICode);
    assert_eq!(DataType::FIGICode.to_string(), "figi");
    assert_eq!(DataType::FIGICode.kind(), DataTypeKind::Code);
    assert_eq!(DataType::FIGICode.code_name(), Some("figi"));
    assert_eq!(DataType::FIGICode.code_width(), Some(12));
    assert_eq!(DataType::FIGICode.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    figi = DataType("figi")
    assert (figi.id, figi.code_width, figi.kind) == ("figi", 12, "code")
    assert figi.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const figi = DataType.fromString('figi')
    assert.equal(figi.id, 'figi')
    assert.equal(figi.kind, 'code')
    assert.equal(figi.codeWidth, 12)
    assert.equal(figi.fixedByteWidth, null)
    ```

## Field

`FIGICodeField` is the typed marker; Python and JavaScript name the factory `figi`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, FIGICodeField};

    let sid = FIGICodeField::unit("figi", false);
    assert_eq!(sid.dtype(), &DataType::FIGICode);
    assert_eq!(sid.to_field(), Field::new("figi", DataType::FIGICode, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.figi("figi", nullable=False).dtype == DataType("figi")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.figi('figi', { nullable: false }).dtype.id, 'figi')
    ```

## Scalar

The value is the canonical spelling: upper case, closed by its check digit. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FIGICode, Scalar};

    let figi = FIGICode::new("bbg000blnq16")?;
    assert_eq!(figi.as_str(), "BBG000BLNQ16");
    assert_eq!(DataType::figi().scalar("BBG000BLNQ16")?, Scalar::FIGICode(figi));
    assert_eq!(DataType::figi().scalar("BBG000BLNQ16")?.kind(), "figi");
    assert!(FIGICode::new("BBG000BLNQ17").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    figi = DataType("figi")
    assert figi.scalar("bbg000blnq16").as_py() == "BBG000BLNQ16"
    assert figi.scalar("bbg000blnq16").kind == "figi"

    with pytest.raises(ValueError, match="check digit"):
        figi.scalar("BBG000BLNQ17")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const figi = DataType.fromString('figi')
    assert.equal(figi.scalar('bbg000blnq16').asJs(), 'BBG000BLNQ16')
    assert.throws(() => figi.scalar('BBG000BLNQ17'), /FIGI|check/i)
    ```

## Arrow storage

`Utf8` under `yggdryl.figi` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("figi", DataType::FIGICode, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.figi");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("figi", "figi")
    arrow_field = sid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.figi"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual(
      [...fields.figi('figi').castArrowArray(utf8(['BBG000BLNQ16']))],
      ['BBG000BLNQ16'],
    )
    ```

## The shape and the check digit

Two consonants, then `G`, then eight consonants or digits, then one decimal digit. The prefix may not be one of the seven the standard reserves - `BS`, `BM`, `GG`, `GB`, `GH`, `KY`, `VG` - because those are country codes a reader would misread. The check digit reads each of the eleven leading characters as a value from zero to thirty-five, doubles the ones at odd positions, sums the decimal digits of every value and closes the sum to a multiple of ten. `FIGICode::is_valid`, `is_canonical` and `closing_digit` answer the rule without building a value. Rust only.

```rust
use yggdryl::FIGICode;

let figi = FIGICode::new("BBG000BLNQ16")?;
assert_eq!(figi.check_digit(), 6);
assert!(FIGICode::is_valid("bbg000blnq16"));
assert!(FIGICode::is_canonical("BBG000BLNQ16"));
assert!(!FIGICode::is_canonical("bbg000blnq16"));
// A permitted consonant prefix that is not one of the seven reserved.
assert!(FIGICode::is_valid("BCG000000005"));

// A vowel in the prefix, a reserved prefix, no `G`, a letter for the check
// digit, the wrong length, and a check digit that does not close it.
for refused in [
    "BAG000BLNQ16",
    "BSG000BLNQ16",
    "BBX000BLNQ16",
    "BBG000BLNQ1A",
    "BBG000BLNQ1",
    "BBG000BLNQ17",
] {
    assert!(FIGICode::new(refused).is_err(), "{refused}");
}
```

## Edges

- `expected twelve characters`, `expected a two-consonant prefix`, `expected a non-reserved prefix`, `expected G as the third character`, `expected eight consonants or digits after the prefix and G`, `expected a closing check digit`, `the check digit does not close the identifier` - the seven refusals, each naming `figi` and the spelling it saw.
- A thirteenth byte -> `at most 12 bytes`, the refusal any code of that width gives.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md), [CUSIP](cusip.md) and [SEDOL](sedol.md) are, and answers null under the default `safe`.
- Serde reads a FIGI through the same door: a document holding a spelling the check digit does not close is refused rather than deserialized.
- No default value: the empty text names no security, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("figi")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about an identifier, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- The crate tag `figicode(65061)` carries the normalized column in a [FIX capture](index.md#fix-message-definitions); `SecurityIDSource(22)=S` and `SecurityAltIDSource(456)=S` lift a valid FIGI.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- code::securities cusip_code::securities figi_code::securities sedol_code::securities
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
