# CUSIP

The nine-character North American securities identifier: six of issuer, two of issue, and the check digit that closes them - held by its shape, ranked by whether the digit closes.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `cusip`, `CusipType`/`CusipField`, the `Cusip` value and `Scalar::Cusip` |
| Validates | The shape: nine ASCII bytes, eight alphanumerics and one check digit; lower case folds at the value door. Whether the digit closes the eight by modulus-10 double-add-double is its [rank](index.md#rank), never a gate |
| Lazy | Nothing - the whole check runs on the stack, over bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling of the wrong length or shape; the empty text, so there is no default value |

## DataType

`cusip` is the one spelling, `DataType::cusip()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::cusip(), DataType::Cusip);
    assert_eq!(DataType::from_str("cusip")?, DataType::Cusip);
    assert_eq!(DataType::Cusip.to_string(), "cusip");
    assert_eq!(DataType::Cusip.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Cusip.code_name(), Some("cusip"));
    assert_eq!(DataType::Cusip.code_width(), Some(9));
    assert_eq!(DataType::Cusip.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    cusip = DataType("cusip")
    assert cusip.id == "cusip"
    assert cusip.kind == "code"
    assert cusip.code_width == 9
    assert cusip.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const cusip = new DataType('cusip')
    assert.equal(cusip.id, 'cusip')
    assert.equal(cusip.kind, 'code')
    assert.equal(cusip.codeWidth, 9)
    assert.equal(cusip.fixedByteWidth, null)
    ```

## Field

`CusipField` is the typed marker; Python and JavaScript name the factory `cusip`.

=== "Rust"

    ```rust
    use yggdryl::{CusipField, DataType, Field};

    let sid = CusipField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::Cusip);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::Cusip, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sid = yggdryl.cusip("sid")
    assert isinstance(sid, Field)
    assert str(sid.dtype) == "cusip"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const sid = fields.cusip('sid')
    assert.equal(sid.dtype.toString(), 'cusip')
    ```

## Scalar

The value is the canonical spelling: upper case, closed by its check digit. Lower case is read as the upper case it spells, because the check digit reads a letter by its alphabet position.

=== "Rust"

    ```rust
    use yggdryl::{Cusip, DataType, Scalar};

    let apple = DataType::cusip().scalar("037833100")?;
    assert_eq!(apple, Scalar::Cusip(Cusip::new("037833100")?));
    assert_eq!(apple.as_str(), Some("037833100"));
    assert_eq!(apple.kind(), "cusip");
    assert_eq!(DataType::cusip().scalar("38259p508")?.as_str(), Some("38259P508"));

    // One digit off is a typo: a value of rank zero, never a refusal.
    assert_eq!(DataType::cusip().scalar("037833101")?.as_str(), Some("037833101"));
    // The shape is the refusal.
    assert!(DataType::cusip().scalar("03783310").is_err());
    assert_ne!(apple, Scalar::from("037833100"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    cusip = DataType("cusip")
    assert cusip.scalar("037833100").as_py() == "037833100"
    assert cusip.scalar("38259p508").as_py() == "38259P508"
    assert cusip.scalar("037833100").kind == "cusip"
    assert cusip.scalar("037833100") != DataType("utf8").scalar("037833100")

    # One digit off is a typo: a value of rank zero, never a refusal.
    assert cusip.scalar("037833101").as_py() == "037833101"
    with pytest.raises(ValueError, match="expected nine characters"):
        cusip.scalar("03783310")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const cusip = new DataType('cusip')
    assert.equal(cusip.scalar('38259p508').asJs(), '38259P508')
    // One digit off is a typo: a value of rank zero, never a refusal.
    assert.equal(cusip.scalar('037833101').asJs(), '037833101')
    assert.throws(() => cusip.scalar('03783310'), /expected nine characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.cusip` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::Cusip, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.cusip");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "cusip")
    arrow_field = sid.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.cusip"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    // A column holds the canonical spelling: a typo is one and lands, and a
    // lower-case spelling is null.
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['037833100', '037833101', '38259p508']), fields.cusip('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['037833100', '037833101', null])
    ```

## The check digit

Each of the eight leading characters reads as a digit or as ten plus its alphabet position; every second value is doubled, the digits of every value are summed, and the digit closes that sum to a multiple of ten. `Cusip::issuer` and `issue` read the six and two characters before the digit, and `closing_digit` computes it. `is_closed` answers whether the digit closes an upper-case identifier - the [rank](index.md#rank) a value answers, one where it closes, zero where it does not - and `is_canonical` whether text is the canonical spelling, upper case and the shape, whatever the digit. Rust only.

```rust
use yggdryl::{CodeValue, Cusip};

let apple = Cusip::new("037833100")?;
assert_eq!(apple.issuer(), "037833");
assert_eq!(apple.issue(), "10");
assert_eq!(apple.check_digit(), 0);

// A letter reads as ten plus its alphabet position.
assert_eq!(Cusip::new("38259P508")?.check_digit(), 8);
assert_eq!(Cusip::closing_digit("38259P50"), Some(8));

// The readings answer without building a value.
assert!(Cusip::is_closed("38259P508"));
assert!(!Cusip::is_closed("037833101"));
assert!(Cusip::is_canonical("037833101"));
assert!(!Cusip::is_canonical("38259p508"));

// A typo is a value of rank zero, which a closing identifier replaces.
let typo = Cusip::new("037833101")?;
assert_eq!(typo.rank(), 0);
assert_eq!(typo.merge_with(&apple), apple);
```

## A column holds the canonical spelling

A scalar read folds the case; a column's bytes are what every reader digests, so an Arrow cast is held to the canonical spelling - upper case, the shape - and answers null under the default `safe` for a lower-case spelling, while a typo is a spelling of the shape and lands as the value it is. Strict names the row and the column, and `try_cast(sid as cusip)` in an [expression](../../expression/terms.md) is that safe cast.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let sid = Field::new("sid", DataType::Cusip, true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["38259P508", "38259p508"]));
    let strict = ArrowCastOptions::new().with_safe(false);
    let refused = Serie::from_arrow_array(Some(&sid), source, strict)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("canonical spelling"), "{refused}");
    assert!(refused.contains("row 1 of column sid"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    sid = Field("sid", "cusip")
    stored = Serie.from_arrow_array(pa.array(["38259P508", "38259p508", "037833101"]), sid)
    assert stored.as_py() == ["38259P508", None, "037833101"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['38259P508', '38259p508']), fields.cusip('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['38259P508', null])
    ```

## Edges

- `expected nine characters`, `expected eight alphanumerics before the check digit`, `expected a closing check digit` - the three refusals, each naming `cusip` and the spelling it saw; a digit that does not close the identifier is a rank, never a refusal.
- A tenth byte -> `at most 9 bytes`, the refusal any code of that width gives.
- No default value: the empty text names no security, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("cusip")` answers an enum of no members, and no Python code class declares it.
- [`merge_with`](index.md#the-code-family-value) takes an identifier that closes over one that does not, whichever leads; two of one rank keep this one.
- A CUSIP and a [SEDOL](sedol.md) of the same bytes are two values, and neither is the string that spells it.
- No crate column: in a [FIX capture](index.md#fix-message-definitions) a CUSIP is one [security identifier](../../graph/identifier.md) of type `cusip` - `SecurityID(48)` under source `1`, or a `SecAltIDGrp(454)` occurrence, each under the base key `cusip` - read as `get_securityids().get(&IdType::Cusip)`, and derived from a US or CA ISIN that closes where the message states none, from `derived`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- code::securities cusip::securities figi::securities sedol::securities
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
