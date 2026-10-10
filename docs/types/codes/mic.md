# MIC

ISO 10383's four-character market identifier code, and `XXXX`, the market that states none.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `mic`, `MicType`/`MicField`, the `Mic` value and `Scalar::Mic` |
| Validates | US-ASCII, no NUL, at most four bytes; the ISO registry is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A fifth byte or a byte past `0x7F`, naming the width: `at most 4 bytes` |

## DataType

`mic` is the one spelling, and FIX's own name for the same registry, `Exchange`, resolves to it as a logical name.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::mic(), DataType::Mic);
    assert_eq!(DataType::from_str("mic")?, DataType::Mic);
    assert_eq!(DataType::Mic.to_string(), "mic");
    assert_eq!(DataType::Mic.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Mic.code_name(), Some("mic"));
    assert_eq!(DataType::Mic.code_width(), Some(4));
    assert_eq!(DataType::Mic.fixed_byte_width(), None);
    // FIX calls the ISO 10383 code an `Exchange`; one registry, one datatype.
    assert_eq!(DataType::from_logical_name("Exchange")?, DataType::Mic);
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

`MicField` is the typed marker; Python and JavaScript name the factory `mic`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MicField};

    let venue = MicField::unit("venue", false);
    assert_eq!(venue.dtype(), &DataType::Mic);
    assert_eq!(venue.to_field(), Field::new("venue", DataType::Mic, false));

    // The leaf is the datatype's: four bytes of text are not a market.
    let plain = Field::new("venue", DataType::fixed_ascii(4)?, false);
    assert!(MicField::try_from_field(plain).is_err());
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
    use yggdryl::{DataType, Mic, Scalar};

    let paris = DataType::mic().scalar("XPAR")?;
    assert_eq!(paris, Scalar::Mic(Mic::new("XPAR")?));
    assert_eq!(paris.as_str(), Some("XPAR"));
    assert_eq!(paris.kind(), "mic");

    // Shorter than the width is a value, not a padded one.
    assert_eq!(DataType::mic().scalar("BX")?.as_str(), Some("BX"));
    let refused = Mic::new("XPARIS").unwrap_err().to_string();
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

    let venue = Field::new("venue", DataType::Mic, false);
    let arrow = venue.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.mic");
    assert_eq!(Field::from_arrow_field(&arrow)?, venue);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    venue = Field("venue", "mic")
    arrow_field = venue.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.mic"
    assert Field.from_arrow(arrow_field) == venue

    # The column holds the text itself: nothing is padded, so nothing is
    # trimmed back.
    stored = Serie.from_arrow_array(pa.array(["XPAR", "BX"]), venue, safe=False)
    assert stored.as_py() == ["XPAR", "BX"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['XPAR', 'BX']), fields.mic('venue'))
    assert.deepEqual([...stored.intoArrowArray()], ['XPAR', 'BX'])
    ```

## `XXXX` states no market

ISO 10383 publishes `XXXX` for "no market", so it is what a merge takes the other side over: `Mic::none()` is that value and `is_none()` asks for it, the [rank](index.md#rank) zero any stated market outranks whichever leads ([`merge_with`](index.md#the-code-family-value)). The registry this crate lists is deliberately partial, so it is no rank. Rust only.

```rust
use yggdryl::{CodeValue, Mic};

assert_eq!(Mic::none().as_str(), "XXXX");
assert!(Mic::none().is_none() && Mic::none().rank() == 0);
assert_eq!(Mic::none().merge_with(&Mic::new("XPAR")?).as_str(), "XPAR");
assert_eq!(Mic::new("XPAR")?.merge_with(&Mic::none()).as_str(), "XPAR");
// Two stated markets are one rank: this one stands.
assert_eq!(Mic::new("XPAR")?.merge_with(&Mic::new("XLON")?).as_str(), "XPAR");
```

## A venue's short code is no ISO MIC

`Mic::new` stays permissive, because a stored column may hold the short code a venue wrote - `S`, `TW`. `Mic::is_iso` is the stricter question a reading asks when it takes the first market a message names: exactly four upper-case ASCII letters or digits. The FIX [market ladder](../../fix/capture.md#the-crates-own-columns) reads by it. Rust only.

```rust
use yggdryl::Mic;

assert!(Mic::is_iso("XSWX"));
assert!(!Mic::is_iso("S"));
assert!(!Mic::is_iso("xswx"));
// Permissive where a column already holds what a venue wrote.
assert_eq!(Mic::new("TW")?.as_str(), "TW");
```

## The ISO 10383 registry

`StringEnum::MICS` ships with the package, reached by either logical name - `mic` or FIX's `Exchange` - because they name one thing. Python declares it over the width as `yggdryl.enums.MIC`, over the `yggdryl.enums.Mic` base a caller subclasses for a vocabulary of its own. A declared vocabulary, never a gate: a market the registry has not published yet is stored rather than refused.

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

## Operating MIC and country

ISO 10383 states, for every code it ever assigned, the operating MIC it trades under and the country it is in. `Mic::operating` answers the operating MIC - itself for an operating MIC, its market's for a segment (`XNGS` under `XNAS`), none for a code the registry never assigned, a venue's own short code among them; an expired code answers too, because a capture names venues that have since closed. `Mic::is_segment` says whether the code is a segment of another market, and `Mic::country` the country the registry places it in - none for a code it never assigned or one it places in no single country: `XOFF` and `XXXX` carry the registry's own `ZZ`, which is no country. Python reads them as properties of a `yggdryl.enums.Mic` member (`operating`, `is_segment`, `country`), answering a `Mic` and a `Country` member; JavaScript as getters of a `Mic` (`operating`, `isSegment`, `country`), answering text. The table is `rust/src/mic/tables.rs`, generated by `python scripts/generate_mic_table.py` from every row of ISO 10383, whatever its status; `StringEnum::MICS`, the declared vocabulary above, is a constant and unchanged by it. A market's country leads to its [currency](country.md#currency), which is what an [instrument's listing](../../graph/instrument.md#derived-facts) on it defaults its currency to.

=== "Rust"

    ```rust
    use yggdryl::{Ccy, Country, Mic};

    let segment = Mic::new("XNGS")?;
    assert!(segment.is_segment());
    assert_eq!(segment.operating(), Some(Mic::new("XNAS")?));
    assert_eq!(segment.country(), Some(Country::new("US")?));
    assert_eq!(Mic::new("XNAS")?.operating(), Some(Mic::new("XNAS")?), "an operating MIC is its own");

    // A market leads to the currency of its country.
    let london = Mic::new("XLON")?.country();
    assert_eq!(london.and_then(|country| country.currency()), Some(Ccy::new("GBP")?));

    // Off-exchange is in no single country; an unassigned code answers nothing.
    let off = Mic::new("XOFF")?;
    assert_eq!((off.operating(), off.country()), (Some(off), None));
    let unassigned = Mic::new("QQQQ")?;
    assert!(unassigned.operating().is_none() && !unassigned.is_segment() && unassigned.country().is_none());
    ```

=== "Python"

    ```python
    from yggdryl.enums import Ccy, Country, Mic

    segment = Mic.from_str("XNGS")
    assert segment.is_segment
    assert segment.operating == Mic.from_str("XNAS")
    assert segment.country == Country.from_str("US")
    assert Mic.from_str("XNAS").operating == Mic.from_str("XNAS"), "an operating MIC is its own"

    # A market leads to the currency of its country.
    london = Mic.from_str("XLON").country
    assert london is not None and london.currency == Ccy.from_str("GBP")

    # Off-exchange is in no single country; an unassigned code answers nothing.
    assert Mic.from_str("XOFF").operating == Mic.from_str("XOFF")
    assert Mic.from_str("XOFF").country is None
    unassigned = Mic.from_str("QQQQ")
    assert unassigned.operating is None and not unassigned.is_segment and unassigned.country is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Country, Mic } = require('yggdryl')

    const segment = new Mic('XNGS')
    assert.ok(segment.isSegment)
    assert.equal(segment.operating, 'XNAS')
    assert.equal(segment.country, 'US')
    assert.equal(new Mic('XNAS').operating, 'XNAS', 'an operating MIC is its own')

    // A market leads to the currency of its country.
    assert.equal(new Country(new Mic('XLON').country).currency, 'GBP')

    // Off-exchange is in no single country; an unassigned code answers nothing.
    assert.equal(new Mic('XOFF').operating, 'XOFF')
    assert.equal(new Mic('XOFF').country, null)
    const unassigned = new Mic('QQQQ')
    assert.deepEqual([unassigned.operating, unassigned.isSegment, unassigned.country], [null, false, null])
    ```

## A dxFeed exchange code names a market only inside its feed

`Mic::from_dxfeed_exchange_code(feed, code)` resolves a regional code only under the `DxFeedExchangeFeed` table that gives it meaning. The feed is mandatory: `Q` is `XNAS` under CTA/UTP and Nasdaq Basic but `XNDQ` under US Options. Aggregate Cboe codes and CME source codes are refused because they name no single MIC; custom OPOL values are a separate namespace, not inferred as MICs. The mapping follows [dxFeed's published tables](https://kb.dxfeed.com/en/data-model/reference-data/exchange-codes.html), with CTA/UTP `H` corrected to ISO's current `EPRL` for MIAX Pearl Equities rather than the page's nonexistent `MRPL`, and NYSE BQT `A` corrected to `XASE` for NYSE American rather than `XNYS` ([ISO 10383 registry](https://www.iso20022.org/market-identifier-codes)). Rust only.

```rust
use yggdryl::{DxFeedExchangeFeed, Mic};

// A dxFeed regional exchange code has meaning only inside its feed.
assert_eq!(
    Mic::from_dxfeed_exchange_code(DxFeedExchangeFeed::CtaUtp, "Q")?.as_str(),
    "XNAS",
);
assert_eq!(
    Mic::from_dxfeed_exchange_code(DxFeedExchangeFeed::UsOptions, "Q")?.as_str(),
    "XNDQ",
);
// C is an aggregate under Cboe, not a market that could be stored as a MIC.
assert!(Mic::from_dxfeed_exchange_code(DxFeedExchangeFeed::Cboe, "C").is_err());
```

## A Reuters exchange mnemonic names one market

`Mic::from_reuters_exchange_code(code)` resolves a Reuters exchange mnemonic - the suffix of a [RIC](ric.md), `Ric::exchange_code`, and the value [FIX 4.2's Appendix C](https://www.onixs.biz/fix-dictionary/4.2/app_c.html) gives `LastMkt(30)`, `ExDestination(100)` and `SecurityExchange(207)` - into its ISO 10383 MIC. The mnemonic is case-sensitive: `B` is Boston and `b` Belfox, `D` Dusseldorf and `d` Eurex Germany. A market that closed resolves to the MIC that carries it on (Pacific `P` to `ARCX`); a row naming a segment, a scheme or no market (`TH`, `0`, `11`) and a closed market ISO never carried on are refused. The FIX market ladder reads each of those three tags as an ISO MIC, else as a mnemonic, so an order routed with `100=TW` names `XTAI`. Rust only.

```rust
use yggdryl::Mic;

assert_eq!(Mic::from_reuters_exchange_code("L")?.as_str(), "XLON");
assert_eq!(Mic::from_reuters_exchange_code("TW")?.as_str(), "XTAI");
// Case is part of the mnemonic.
assert_eq!(Mic::from_reuters_exchange_code("b")?.as_str(), "XBRD");
// Third Market is a scheme, not a market with a MIC.
assert!(Mic::from_reuters_exchange_code("TH").is_err());
```

## Edges

- `at most 4 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`. The refusal names the code's own width, not the next ASCII one up.
- A fixed-width source column casts in: the NUL padding its slot wrote is the slot's, never the value's, so it is trimmed.
- The default value is the empty text, answered as a `mic` scalar ([Cast](../cast.md#empty-text)).
- A dictionary-encoded `mic` keeps its identity: a low-cardinality venue column is exactly the one a writer dictionary-encodes, and the extension name rides the field.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi::coded code::datatypes string::listings
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
