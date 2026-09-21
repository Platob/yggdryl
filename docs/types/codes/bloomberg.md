# Bloomberg

A Bloomberg identifier: a ticker, a market and a yellow key with spaces between them - `AAPL US Equity` - and the one code here with no shape to check.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `bloomberg`, `BloombergCodeType`/`BloombergCodeField`, the `BloombergCode` value and `Scalar::BloombergCode` |
| Validates | US-ASCII, no NUL, nonempty, at most thirty-two bytes - and nothing more, because refusing a spelling nobody published would be a guess |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | The empty text, naming the identifier; a thirty-third byte or a byte past `0x7F`, naming the width |

Thirty-two is a bound rather than a shape: a ticker, a market and a yellow key have no fixed length between them, and no standard closes a terminal identifier. It still fits one compact-string allocation. [FIGI](figi.md) is a separate checked identity, never a Bloomberg fallback.

## DataType

`bloomberg` is the one spelling; the datatype is the variant, and there is no shorthand constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("bloomberg")?, DataType::BloombergCode);
    assert_eq!(DataType::BloombergCode.to_string(), "bloomberg");
    assert_eq!(DataType::BloombergCode.kind(), DataTypeKind::Code);
    assert_eq!(DataType::BloombergCode.code_name(), Some("bloomberg"));
    assert_eq!(DataType::BloombergCode.code_width(), Some(32));
    assert_eq!(DataType::BloombergCode.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    bloomberg = DataType("bloomberg")
    assert bloomberg.id == "bloomberg"
    assert bloomberg.kind == "code"
    assert bloomberg.code_width == 32
    assert bloomberg.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bloomberg = new DataType('bloomberg')
    assert.equal(bloomberg.id, 'bloomberg')
    assert.equal(bloomberg.kind, 'code')
    assert.equal(bloomberg.codeWidth, 32)
    assert.equal(bloomberg.fixedByteWidth, null)
    ```

## Field

`BloombergCodeField` is the typed marker; Python and JavaScript name the factory `bloomberg`.

=== "Rust"

    ```rust
    use yggdryl::{BloombergCodeField, DataType, Field};

    let sid = BloombergCodeField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::BloombergCode);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::BloombergCode, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sid = yggdryl.bloomberg("sid")
    assert isinstance(sid, Field)
    assert str(sid.dtype) == "bloomberg"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const sid = fields.bloomberg('sid')
    assert.equal(sid.dtype.toString(), 'bloomberg')
    ```

## Scalar

The value is the published spelling, kept exactly: case is preserved, and the spaces between the parts are part of the identifier.

=== "Rust"

    ```rust
    use yggdryl::{BloombergCode, DataType, Scalar};

    let apple = DataType::BloombergCode.scalar("AAPL US Equity")?;
    assert_eq!(apple, Scalar::BloombergCode(BloombergCode::new("AAPL US Equity")?));
    assert_eq!(apple.as_str(), Some("AAPL US Equity"));
    assert_eq!(apple.kind(), "bloomberg");
    // Case is the terminal's, so it is preserved rather than folded.
    assert_ne!(
        BloombergCode::new("AAPL US Equity")?,
        BloombergCode::new("AAPL US EQUITY")?
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    apple = DataType("bloomberg").scalar("AAPL US Equity")
    assert apple.as_py() == "AAPL US Equity"
    assert apple.kind == "bloomberg"
    # Case is the terminal's, so it is preserved rather than folded.
    assert DataType("bloomberg").scalar("AAPL US EQUITY") != apple
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const apple = new DataType('bloomberg').scalar('AAPL US Equity')
    assert.equal(apple.asJs(), 'AAPL US Equity')
    assert.equal(apple.kind, 'bloomberg')
    ```

## Arrow storage

`Utf8` under `yggdryl.bloomberg` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::BloombergCode, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.bloomberg");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "bloomberg")
    arrow_field = sid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.bloomberg"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual(
      [...fields.bloomberg('sid').castArrowArray(utf8(['AAPL US Equity']))],
      ['AAPL US Equity'],
    )
    ```

## Canonical is the published spelling

There is no check digit and no grammar to hold a value to, so canonical means what it means for every code: nonempty ASCII that fits the thirty-two bytes, with no trailing padding, and the published case retained. `BloombergCode::is_canonical` answers that rule without building a value. Rust only.

```rust
use yggdryl::BloombergCode;

assert!(BloombergCode::is_canonical("AAPL US Equity"));
assert!(BloombergCode::is_canonical("BBG000B9XRY4"));
// The empty text names no security, and neither does a value past the width.
assert!(!BloombergCode::is_canonical(""));
assert!(!BloombergCode::is_canonical("AAPL US Equity with far too much said about it"));
```

## The empty text names no security

An identifier has no neutral member, so the empty text is refused at the value door rather than stored - and the datatype's canonical default refuses with it, which is what makes an empty cell entering the column null rather than a member nobody issued.

=== "Rust"

    ```rust
    use yggdryl::{BloombergCode, DataType};

    let refused = BloombergCode::new("").unwrap_err().to_string();
    assert!(refused.contains("bloomberg"), "{refused}");
    assert!(DataType::BloombergCode.default_value().is_err());
    // An empty cell is therefore null, as it is for a UUID.
    assert!(DataType::BloombergCode.scalar("")?.is_null());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    with pytest.raises(ValueError, match="bloomberg"):
        DataType("bloomberg").default_scalar()
    # An empty cell is therefore null, as it is for a UUID.
    assert DataType("bloomberg").scalar("").is_null()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.throws(() => new DataType('bloomberg').defaultJSValue(), /bloomberg/)
    assert.equal(new DataType('bloomberg').scalar('').kind, 'null')
    ```

## Edges

- The empty text -> refused naming the identifier; `default_value` refuses too, so an empty text cell entering the column is null, as it is for a UUID ([Cast](../cast.md#empty-text)).
- A thirty-third byte or a byte past `0x7F` -> refused naming the width, and the row in a cast.
- No vocabulary: `StringEnum::from_logical_name("bloomberg")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about it, so [`merge_with`](index.md#the-code-family-value) keeps this identifier.
- `SecurityIDSource(22)=A` remains Bloomberg in a [FIX capture](index.md#fix-message-definitions); source `S` lifts a valid [FIGI](figi.md) instead. The crate tag for the normalized column is `bloombergcode(65059)`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi_code::coded code::datatypes code::securities cusip_code::securities figi_code::securities sedol_code::securities state::coded timeinforce::coded
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
