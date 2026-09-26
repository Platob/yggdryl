# Bbg

A Bloomberg identifier: a ticker, a market and a yellow key with spaces between them - `AAPL US Equity` - and the one code here with no shape to check.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `bbg`, `BbgType`/`BbgField`, the `Bbg` value and `Scalar::Bbg` |
| Validates | US-ASCII, no NUL, nonempty, at most thirty-two bytes - and nothing more, because refusing a spelling nobody published would be a guess |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | The empty text, naming `bbg`; a thirty-third byte or a byte past `0x7F`, naming the width |

Thirty-two is a bound rather than a shape: a ticker, a market and a yellow key have no fixed length between them, and no standard closes a terminal identifier. It still fits one compact-string allocation. [FIGI](figi.md) is a separate checked identity, never a Bloomberg fallback, and a [RIC](ric.md) is LSEG's ticker-like code, a type of its own.

## DataType

`bbg` is the one spelling, `DataType::bbg()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::bbg(), DataType::Bbg);
    assert_eq!(DataType::from_str("bbg")?, DataType::Bbg);
    assert_eq!(DataType::Bbg.to_string(), "bbg");
    assert_eq!(DataType::Bbg.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Bbg.code_name(), Some("bbg"));
    assert_eq!(DataType::Bbg.code_width(), Some(32));
    assert_eq!(DataType::Bbg.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    bbg = DataType("bbg")
    assert bbg.id == "bbg"
    assert bbg.kind == "code"
    assert bbg.code_width == 32
    assert bbg.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const bbg = new DataType('bbg')
    assert.equal(bbg.id, 'bbg')
    assert.equal(bbg.kind, 'code')
    assert.equal(bbg.codeWidth, 32)
    assert.equal(bbg.fixedByteWidth, null)
    ```

## Field

`BbgField` is the typed marker; Python and JavaScript name the factory `bbg`.

=== "Rust"

    ```rust
    use yggdryl::{BbgField, DataType, Field};

    let sid = BbgField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::Bbg);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::Bbg, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sid = yggdryl.bbg("sid")
    assert isinstance(sid, Field)
    assert str(sid.dtype) == "bbg"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const sid = fields.bbg('sid')
    assert.equal(sid.dtype.toString(), 'bbg')
    ```

## Scalar

The value is the published spelling, kept exactly: case is preserved, and the spaces between the parts are part of the identifier.

=== "Rust"

    ```rust
    use yggdryl::{Bbg, DataType, Scalar};

    let apple = DataType::bbg().scalar("AAPL US Equity")?;
    assert_eq!(apple, Scalar::Bbg(Bbg::new("AAPL US Equity")?));
    assert_eq!(apple.as_str(), Some("AAPL US Equity"));
    assert_eq!(apple.kind(), "bbg");
    // Case is the terminal's, so it is preserved rather than folded.
    assert_ne!(
        Bbg::new("AAPL US Equity")?,
        Bbg::new("AAPL US EQUITY")?
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    apple = DataType("bbg").scalar("AAPL US Equity")
    assert apple.as_py() == "AAPL US Equity"
    assert apple.kind == "bbg"
    # Case is the terminal's, so it is preserved rather than folded.
    assert DataType("bbg").scalar("AAPL US EQUITY") != apple
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const apple = new DataType('bbg').scalar('AAPL US Equity')
    assert.equal(apple.asJs(), 'AAPL US Equity')
    assert.equal(apple.kind, 'bbg')
    ```

## Arrow storage

`Utf8` under `yggdryl.bbg` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::Bbg, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.bbg");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "bbg")
    arrow_field = sid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.bbg"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['AAPL US Equity']), fields.bbg('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['AAPL US Equity'])
    ```

## Canonical is the published spelling

There is no check digit and no grammar to hold a value to, so canonical means what it means for every code: nonempty ASCII that fits the thirty-two bytes, with no trailing padding, and the published case retained. `Bbg::is_canonical` answers that rule without building a value. Rust only.

```rust
use yggdryl::Bbg;

assert!(Bbg::is_canonical("AAPL US Equity"));
assert!(Bbg::is_canonical("BBG000B9XRY4"));
// The empty text names no security, and neither does a value past the width.
assert!(!Bbg::is_canonical(""));
assert!(!Bbg::is_canonical("AAPL US Equity with far too much said about it"));
```

## The empty text names no security

An identifier has no neutral member, so the empty text is refused at the value door rather than stored - and the datatype's canonical default refuses with it, which is what makes an empty cell entering the column null rather than a member nobody issued.

=== "Rust"

    ```rust
    use yggdryl::{Bbg, DataType};

    let refused = Bbg::new("").unwrap_err().to_string();
    assert!(refused.contains("bbg"), "{refused}");
    assert!(DataType::Bbg.default_value().is_err());
    // An empty cell is therefore null, as it is for a UUID.
    assert!(DataType::Bbg.scalar("")?.is_null());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    with pytest.raises(ValueError, match="bbg"):
        DataType("bbg").default_scalar()
    # An empty cell is therefore null, as it is for a UUID.
    assert DataType("bbg").scalar("").is_null()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.throws(() => new DataType('bbg').defaultJSValue(), /bbg/)
    assert.equal(new DataType('bbg').scalar('').kind, 'null')
    ```

## Edges

- The empty text -> refused naming `bbg`; `default_value` refuses too, so an empty text cell entering the column is null, as it is for a UUID ([Cast](../cast.md#empty-text)).
- A thirty-third byte or a byte past `0x7F` -> refused naming the width, and the row in a cast.
- No vocabulary: `StringEnum::from_logical_name("bbg")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about it, so [`merge_with`](index.md#the-code-family-value) keeps this identifier.
- A `bbg` and a [`ric`](ric.md) of the same bytes are two values: the identity leads, then the text.
- FIX names the company rather than the type: `SecurityIDSource(22)=A` is the source key `BLOOMBERG` in a [FIX capture](index.md#fix-message-definitions), and the crate tag for its normalized column is `bloombergcode(65059)`; source `S` lifts a valid [FIGI](figi.md) instead.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- bbg::value bbg::datatype code::datatypes code::securities securityid::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
