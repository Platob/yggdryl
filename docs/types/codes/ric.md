# RIC

LSEG's [Refinitiv Identification Code](https://en.wikipedia.org/wiki/Refinitiv_Identification_Code), first the Reuters Instrument Code: a ticker, a period and the mnemonic of the exchange it trades on - `VOD.L`, `IBM.N` - held as the one token it is, its case kept.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `ric`, `RicType`/`RicField`, the `Ric` value and `Scalar::Ric` |
| Validates | US-ASCII of at most thirty-two bytes once the NUL padding a fixed slot wrote is trimmed, nonempty, and one token: every byte printable, no space and no control byte. Case is kept |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A space or a control byte, naming `ric`, the byte and its position; a thirty-third byte, a NUL inside the code or a byte past `0x7F`, naming the width; the empty text, naming `ric`, so there is no default value |

No registry publishes the set and no check digit closes a code, so the rule is its shape. A ticker, a period and an exchange mnemonic is the common form - `VOD.L` in London, `IBM.N` on the New York Stock Exchange, `0005.HK` in Hong Kong - and the others name no exchange: an index opens on a period (`.SPX`), a chain carries `#` (`0#.FTSE`), a currency quote closes on `=` (`EUR=`), a continuation future has no period at all (`ESc1`). Case is part of the code: `ESc1` is not `ESC1`, and an exchange mnemonic tells `b` from `B`. Thirty-two bytes is a bound rather than a shape, as it is for a [Bbg](bbg.md) identifier.

## DataType

`ric` is the one spelling, `DataType::ric()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::ric(), DataType::Ric);
    assert_eq!(DataType::from_str("ric")?, DataType::Ric);
    assert_eq!(DataType::Ric.to_string(), "ric");
    assert_eq!(DataType::Ric.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Ric.code_name(), Some("ric"));
    assert_eq!(DataType::Ric.code_width(), Some(32));
    assert_eq!(DataType::Ric.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    ric = DataType("ric")
    assert (ric.id, ric.kind, ric.code_width) == ("ric", "code", 32)
    assert ric.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const ric = new DataType('ric')
    assert.equal(ric.id, 'ric')
    assert.equal(ric.kind, 'code')
    assert.equal(ric.codeWidth, 32)
    assert.equal(ric.fixedByteWidth, null)
    ```

## Field

`RicField` is the typed marker; Python and JavaScript name the factory `ric`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, RicField};

    let rid = RicField::unit("rid", true);
    assert_eq!(rid.dtype(), &DataType::Ric);
    assert_eq!(rid.to_field(), Field::new("rid", DataType::Ric, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    rid = yggdryl.ric("rid")
    assert isinstance(rid, Field)
    assert rid.dtype == DataType("ric")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const rid = fields.ric('rid')
    assert.equal(rid.dtype.toString(), 'ric')
    ```

## Scalar

The value is the code as written: case is kept rather than folded, and a space splits the token, so it is refused.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Ric, Scalar};

    let vodafone = DataType::ric().scalar("VOD.L")?;
    assert_eq!(vodafone, Scalar::Ric(Ric::new("VOD.L")?));
    assert_eq!(vodafone.as_str(), Some("VOD.L"));
    assert_eq!(vodafone.kind(), "ric");
    // Case is part of the code, so it is kept rather than folded.
    assert_eq!(Ric::new("ESc1")?.as_str(), "ESc1");
    assert_ne!(Ric::new("ESc1")?, Ric::new("ESC1")?);
    // A space splits the token, and the refusal says where.
    let refused = Ric::new("VOD L").unwrap_err().to_string();
    assert!(refused.contains("printable ASCII, got 0x20 at 3"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    ric = DataType("ric")
    vodafone = ric.scalar("VOD.L")
    assert vodafone.as_py() == "VOD.L"
    assert vodafone.kind == "ric"
    # Case is part of the code, so it is kept rather than folded.
    assert ric.scalar("ESc1").as_py() == "ESc1"
    assert ric.scalar("ESc1") != ric.scalar("ESC1")

    with pytest.raises(ValueError, match="printable ASCII"):
        ric.scalar("VOD L")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const ric = new DataType('ric')
    const vodafone = ric.scalar('VOD.L')
    assert.equal(vodafone.asJs(), 'VOD.L')
    assert.equal(vodafone.kind, 'ric')
    assert.equal(ric.scalar('ESc1').asJs(), 'ESc1')
    assert.throws(() => ric.scalar('VOD L'), /printable ASCII/)
    ```

## Arrow storage

`Utf8` under `yggdryl.ric` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let rid = Field::new("rid", DataType::Ric, false);
    let arrow = rid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ric");
    assert_eq!(Field::from_arrow_field(&arrow)?, rid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    rid = Field("rid", "ric")
    arrow_field = rid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.ric"
    assert Field.from_arrow(arrow_field) == rid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['VOD.L', 'ESc1']), fields.ric('rid'))
    assert.deepEqual([...stored.intoArrowArray()], ['VOD.L', 'ESc1'])
    ```

## The exchange mnemonic

`Ric::exchange_code` is the text after the last period, when a ticker stands before it and something follows it, and `None` for a code that names no exchange: an index opens on its period, a chain carries `#`, and a currency quote or a continuation has no period at all. The mnemonic is the one FIX 4.2's Appendix C lists, which [`Mic::from_reuters_exchange_code`](mic.md#a-reuters-exchange-mnemonic-names-one-market) resolves into the market's MIC. `Ric::is_canonical` answers the rule without building a value. Rust only.

```rust
use yggdryl::{Mic, Ric};

let hsbc = Ric::new("0005.HK")?;
assert_eq!(hsbc.exchange_code(), Some("HK"));
assert_eq!(Mic::from_reuters_exchange_code("HK")?.as_str(), "XHKG");
assert_eq!(Ric::new("VOD.L")?.exchange_code(), Some("L"));

// An index, a chain, a currency quote and a continuation name no exchange.
for code in [".SPX", "0#.FTSE", "EUR=", "ESc1"] {
    assert_eq!(Ric::new(code)?.exchange_code(), None, "{code}");
}

// Canonical is the code as stored: nonempty, one token, no padding.
assert!(Ric::is_canonical("VOD.L"));
assert!(!Ric::is_canonical("VOD.L\0"));
assert!(!Ric::is_canonical("VOD L"));
assert!(!Ric::is_canonical(""));
```

## The empty text names no instrument

A code has no neutral member here, so the empty text is refused at the value door rather than stored - and the datatype's canonical default refuses with it, which is what makes an empty cell entering the column null rather than a code nobody issued.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Ric};

    let refused = Ric::new("").unwrap_err().to_string();
    assert!(refused.contains("ric"), "{refused}");
    assert!(DataType::Ric.default_value().is_err());
    // An empty cell is therefore null, as it is for a UUID.
    assert!(DataType::Ric.scalar("")?.is_null());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    with pytest.raises(ValueError, match="invalid ric datatype"):
        DataType("ric").default_scalar()
    # An empty cell is therefore null, as it is for a UUID.
    assert DataType("ric").scalar("").is_null()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.throws(() => new DataType('ric').defaultJSValue(), /Refinitiv Identification Code/)
    assert.equal(new DataType('ric').scalar('').kind, 'null')
    ```

## A column holds the canonical spelling

An Arrow cast into the column is held to the canonical spelling: under the default `safe` a split token is null and so is an empty cell, and the rest of the column stands; strict names the row and the column.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let rid = Field::new("rid", DataType::Ric, true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["VOD.L", "VOD L"]));
    let strict = ArrowCastOptions::new().with_safe(false);
    let refused = Serie::from_arrow_array(Some(&rid), source, strict)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("canonical spelling"), "{refused}");
    assert!(refused.contains("row 1 of column rid"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    rid = Field("rid", "ric")
    # Safe: the refused cell and the empty one are null, the rest stands.
    source = pa.array(["VOD.L", "VOD L", ""])
    stored = Serie.from_arrow_array(source, rid).into_arrow_array()
    assert stored.to_pylist() == ["VOD.L", None, None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['VOD.L', 'VOD L', '']), fields.ric('rid'))
    assert.deepEqual([...stored.intoArrowArray()], ['VOD.L', null, null])
    ```

## Edges

- `expected a Refinitiv Identification Code, got ""` and `expected one token of printable ASCII, got 0x20 at 3 in "VOD L"` - the two refusals of the code's own rule, each naming `ric`.
- A thirty-third byte, a NUL inside the code or a byte past `0x7F` -> the refusal any code of that width gives, `at most 32 bytes`. Only trailing NUL padding is trimmed; a trailing space is a space, and refused.
- No default value: the empty text names no instrument, so an empty text cell entering the column is null, as it is for a UUID ([Cast](../cast.md#empty-text)), and the column's nullability is what may refuse it.
- Serde reads a RIC through the same door: a document holding a split token is refused rather than deserialized.
- No vocabulary: `StringEnum::from_logical_name("ric")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about an identifier, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- A `ric` and a [`bbg`](bbg.md) of the same bytes are two values: the identity leads, then the text.
- `SecurityIDSource(22)=5` is the `RIC` source key in a [FIX capture](index.md#fix-message-definitions), and its code is validated by `Ric::new`: a code with an inner space, which a source with no rule of its own would hold, is refused. A RIC is one more security identifier under its own key, `ids.get("RIC")`, and no crate column of its own.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- ric::value:: ric::datatype:: code::datatypes securityid::a_ric_source
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code or a_ric_is"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js node/tests/fields.test.js
    ```
