# MIC

ISO 10383's four-character market identifier code, and `XXXX`, the market that states none.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `mic`, `MicCodeType`/`MicCodeField`, the `MicCode` value and `Scalar::MicCode` |
| Validates | US-ASCII, no NUL, at most four bytes; the ISO registry is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A fifth byte or a byte past `0x7F`, naming the width: `at most 4 bytes` |

## DataType

`mic` is the one spelling, and FIX's own name for the same registry, `Exchange`, resolves to it as a logical name.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::mic(), DataType::MicCode);
    assert_eq!(DataType::from_str("mic")?, DataType::MicCode);
    assert_eq!(DataType::MicCode.to_string(), "mic");
    assert_eq!(DataType::MicCode.kind(), DataTypeKind::Code);
    assert_eq!(DataType::MicCode.code_name(), Some("mic"));
    assert_eq!(DataType::MicCode.code_width(), Some(4));
    assert_eq!(DataType::MicCode.fixed_byte_width(), None);
    // FIX calls the ISO 10383 code an `Exchange`; one registry, one datatype.
    assert_eq!(DataType::from_logical_name("Exchange")?, DataType::MicCode);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    mic = DataType("mic")
    assert mic.id == "mic"
    assert mic.kind == "code"
    assert mic.code_width == 4
    assert mic.fixed_byte_width is None
    assert DataType.from_logical_name("Exchange") == mic
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const mic = new DataType('mic')
    assert.equal(mic.id, 'mic')
    assert.equal(mic.kind, 'code')
    assert.equal(mic.codeWidth, 4)
    assert.equal(mic.fixedByteWidth, null)
    assert.ok(DataType.fromLogicalName('Exchange').equals(mic))
    ```

## Field

`MicCodeField` is the typed marker; Python and JavaScript name the factory `mic`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MicCodeField};

    let venue = MicCodeField::unit("venue", false);
    assert_eq!(venue.dtype(), &DataType::MicCode);
    assert_eq!(venue.to_field(), Field::new("venue", DataType::MicCode, false));

    // The leaf is the datatype's: four bytes of text are not a market.
    let plain = Field::new("venue", DataType::fixed_ascii(4)?, false);
    assert!(MicCodeField::try_from_field(plain).is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    venue = yggdryl.mic("venue", nullable=False)
    assert isinstance(venue, Field)
    assert str(venue.dtype) == "mic"
    assert not venue.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const venue = fields.mic('venue', { nullable: false })
    assert.equal(venue.dtype.toString(), 'mic')
    assert.equal(venue.nullable, false)
    ```

## Scalar

The value is the identifier, under the market's identity. A value shorter than the width stores as itself: there is no slot to fill.

=== "Rust"

    ```rust
    use yggdryl::{DataType, MicCode, Scalar};

    let paris = DataType::mic().scalar("XPAR")?;
    assert_eq!(paris, Scalar::MicCode(MicCode::new("XPAR")?));
    assert_eq!(paris.as_str(), Some("XPAR"));
    assert_eq!(paris.kind(), "mic");

    // Shorter than the width is a value, not a padded one.
    assert_eq!(DataType::mic().scalar("BX")?.as_str(), Some("BX"));
    let refused = MicCode::new("XPARIS").unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    paris = DataType("mic").scalar("XPAR")
    assert paris.as_py() == "XPAR"
    assert paris.kind == "mic"
    assert DataType("mic").scalar("BX").as_py() == "BX"

    with pytest.raises(ValueError, match="at most 4 bytes"):
        DataType("mic").scalar("XPARIS")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const paris = new DataType('mic').scalar('XPAR')
    assert.equal(paris.asJs(), 'XPAR')
    assert.equal(paris.kind, 'mic')
    assert.equal(new DataType('mic').scalar('BX').asJs(), 'BX')
    assert.throws(() => new DataType('mic').scalar('XPARIS'), /at most 4 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.mic` ([Codes](index.md#arrow-storage)). A fixed-width source column is still a spelling the cast reads, with the padding that column's slot wrote trimmed back off.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let venue = Field::new("venue", DataType::MicCode, false);
    let arrow = venue.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow_field(&arrow)?, venue);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    venue = Field("venue", "mic")
    arrow_field = venue.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.mic"
    assert Field.from_arrow(arrow_field) == venue

    # The column holds the text itself: nothing is padded, so nothing is
    # trimmed back.
    stored = venue.cast_arrow_array(pa.array(["XPAR", "BX"]), safe=False)
    assert stored.to_pylist() == ["XPAR", "BX"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual([...fields.mic('venue').castArrowArray(utf8(['XPAR', 'BX']))], ['XPAR', 'BX'])
    ```

## `XXXX` states no market

ISO 10383 publishes `XXXX` for "no market", so it is what a merge takes the other side over ([`merge_with`](index.md#the-code-family-value)). Rust only.

```rust
use yggdryl::{CodeValue, MicCode};

assert_eq!(MicCode::none().as_str(), "XXXX");
assert_eq!(MicCode::none().merge_with(&MicCode::new("XPAR")?).as_str(), "XPAR");
// Anything stated stands.
assert_eq!(MicCode::new("XPAR")?.merge_with(&MicCode::new("XLON")?).as_str(), "XPAR");
```

## The ISO 10383 registry

`StringEnum::MICS` ships with the package, reached by either logical name - `mic` or FIX's `Exchange` - because they name one thing. Python declares it over the width as `yggdryl.enums.MIC`. A declared vocabulary, never a gate: a market the registry has not published yet is stored rather than refused.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StringEnum};

    let venues = StringEnum::from_logical_name("mic")?;
    assert_eq!(venues.len(), StringEnum::MICS.len());
    assert_eq!(StringEnum::from_logical_name("Exchange")?.len(), StringEnum::MICS.len());

    // The listing is a vocabulary, never a gate on the value.
    assert_eq!(DataType::mic().scalar("ZZZZ")?.as_str(), Some("ZZZZ"));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, StringEnum
    from yggdryl.enums import MIC

    venues = StringEnum.from_logical_name("mic")
    assert len(venues) == len(StringEnum.prebuilt()["mic"])
    assert len(StringEnum.from_logical_name("Exchange")) == len(venues)

    # A member is its value's own storage bytes, read big-endian.
    assert int(MIC.XPAR) == DataType("mic").ascii_packed("XPAR")
    # The listing is a vocabulary, never a gate on the value.
    assert DataType("mic").scalar("ZZZZ").as_py() == "ZZZZ"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, StringEnum } = require('yggdryl')

    const venues = StringEnum.fromLogicalName('mic')
    assert.equal(venues.length, StringEnum.prebuilt().mic.length)
    assert.equal(StringEnum.fromLogicalName('Exchange').length, venues.length)
    assert.equal(new DataType('mic').scalar('ZZZZ').asJs(), 'ZZZZ')
    ```

## A dxFeed exchange code names a market only inside its feed

`MicCode::from_dxfeed_exchange_code(feed, code)` resolves a regional code only under the `DxFeedExchangeFeed` table that gives it meaning. The feed is mandatory: `Q` is `XNAS` under CTA/UTP and Nasdaq Basic but `XNDQ` under US Options. Aggregate Cboe codes and CME source codes are refused because they name no single MIC; custom OPOL values are a separate namespace, not inferred as MICs. The mapping follows [dxFeed's published tables](https://kb.dxfeed.com/en/data-model/reference-data/exchange-codes.html), with CTA/UTP `H` corrected to ISO's current `EPRL` for MIAX Pearl Equities rather than the page's nonexistent `MRPL`, and NYSE BQT `A` corrected to `XASE` for NYSE American rather than `XNYS` ([ISO 10383 registry](https://www.iso20022.org/market-identifier-codes)). Rust only.

```rust
use yggdryl::{DxFeedExchangeFeed, MicCode};

// A dxFeed regional exchange code has meaning only inside its feed.
assert_eq!(
    MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::CtaUtp, "Q")?.as_str(),
    "XNAS",
);
assert_eq!(
    MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::UsOptions, "Q")?.as_str(),
    "XNDQ",
);
// C is an aggregate under Cboe, not a market that could be stored as a MIC.
assert!(MicCode::from_dxfeed_exchange_code(DxFeedExchangeFeed::Cboe, "C").is_err());
```

## Edges

- `at most 4 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`. The refusal names the code's own width, not the next ASCII one up.
- A fixed-width source column casts in: the NUL padding its slot wrote is the slot's, never the value's, so it is trimmed.
- The default value is the empty text, answered as a `mic` scalar ([Cast](../cast.md#empty-text)).
- A dictionary-encoded `mic` keeps its identity: a low-cardinality venue column is exactly the one a writer dictionary-encodes, and the extension name rides the field.

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
