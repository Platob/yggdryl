# CUSIP

The nine-character North American securities identifier: six of issuer, two of issue, and the check digit that closes them.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `cusip`, `CusipCodeType`/`CusipCodeField`, the `CusipCode` value and `Scalar::CusipCode` |
| Validates | Nine ASCII bytes: eight alphanumerics and one digit closing them by modulus-10 double-add-double; lower case folds at the value door |
| Lazy | Nothing - the whole check runs on the stack, over bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling of the wrong length or shape, and a check digit that does not close the identifier; the empty text, so there is no default value |

## DataType

`cusip` is the one spelling, `DataType::cusip()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::cusip(), DataType::CusipCode);
    assert_eq!(DataType::from_str("cusip")?, DataType::CusipCode);
    assert_eq!(DataType::CusipCode.to_string(), "cusip");
    assert_eq!(DataType::CusipCode.kind(), DataTypeKind::Code);
    assert_eq!(DataType::CusipCode.code_name(), Some("cusip"));
    assert_eq!(DataType::CusipCode.code_width(), Some(9));
    assert_eq!(DataType::CusipCode.fixed_byte_width(), None);
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

`CusipCodeField` is the typed marker; Python and JavaScript name the factory `cusip`.

=== "Rust"

    ```rust
    use yggdryl::{CusipCodeField, DataType, Field};

    let sid = CusipCodeField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::CusipCode);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::CusipCode, true));
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
    use yggdryl::{CusipCode, DataType, Scalar};

    let apple = DataType::cusip().scalar("037833100")?;
    assert_eq!(apple, Scalar::CusipCode(CusipCode::new("037833100")?));
    assert_eq!(apple.as_str(), Some("037833100"));
    assert_eq!(apple.kind(), "cusip");
    assert_eq!(DataType::cusip().scalar("38259p508")?.as_str(), Some("38259P508"));

    // One digit off is a typo, not a security.
    assert!(DataType::cusip().scalar("037833101").is_err());
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

    with pytest.raises(ValueError, match="check digit does not close"):
        cusip.scalar("037833101")
    with pytest.raises(ValueError, match="expected nine characters"):
        cusip.scalar("03783310")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const cusip = new DataType('cusip')
    assert.equal(cusip.scalar('38259p508').asJs(), '38259P508')
    assert.throws(() => cusip.scalar('037833101'), /check digit does not close/)
    assert.throws(() => cusip.scalar('03783310'), /expected nine characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.cusip` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::CusipCode, false);
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
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.cusip"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A column holds the canonical spelling, so a cast lets in an identifier
    // the check digit closes and answers null for a typo.
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual(
      [...fields.cusip('sid').castArrowArray(utf8(['037833100', '037833101']))],
      ['037833100', null],
    )
    ```

## The check digit

Each of the eight leading characters reads as a digit or as ten plus its alphabet position; every second value is doubled, the digits of every value are summed, and the digit closes that sum to a multiple of ten. `CusipCode::issuer` and `issue` read the six and two characters before the digit; `is_valid`, `is_canonical` and `closing_digit` answer the rule without building a value. Rust only.

```rust
use yggdryl::CusipCode;

let apple = CusipCode::new("037833100")?;
assert_eq!(apple.issuer(), "037833");
assert_eq!(apple.issue(), "10");
assert_eq!(apple.check_digit(), 0);

// A letter reads as ten plus its alphabet position.
assert_eq!(CusipCode::new("38259P508")?.check_digit(), 8);
assert_eq!(CusipCode::closing_digit("38259P50"), Some(8));

// The rule answers without building a value, in either case.
assert!(CusipCode::is_valid("38259p508"));
assert!(CusipCode::is_canonical("38259P508"));
assert!(!CusipCode::is_canonical("38259p508"));
assert!(!CusipCode::is_valid("037833101"));
```

## A column holds the canonical spelling

A scalar read folds the case; a column's bytes are what every reader digests, so an Arrow cast is held to the canonical spelling and answers null under the default `safe` for a typo or a lower-case spelling alike. Strict names the row and the column, and `try_cast(sid as cusip)` in an [expression](../../expression/terms.md) is that safe cast.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::FieldValue as _;
    use yggdryl::{ArrowCastOptions, DataType, Field};

    let sid = Field::new("sid", DataType::CusipCode, true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["38259P508", "38259p508"]));
    let refused = sid
        .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("canonical spelling"), "{refused}");
    assert!(refused.contains("row 1 of column sid"), "{refused}");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "cusip")
    stored = sid.cast_arrow_array(pa.array(["38259P508", "38259p508", "037833101"]))
    assert stored.to_pylist() == ["38259P508", None, None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual(
      [...fields.cusip('sid').castArrowArray(utf8(['38259P508', '38259p508']))],
      ['38259P508', null],
    )
    ```

## Edges

- `expected nine characters`, `expected eight alphanumerics before the check digit`, `expected a closing check digit`, `the check digit does not close the identifier` - the four refusals, each naming `cusip` and the spelling it saw.
- A tenth byte -> `at most 9 bytes`, the refusal any code of that width gives.
- No default value: the empty text names no security, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("cusip")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about an identifier, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- A CUSIP and a [SEDOL](sedol.md) of the same bytes are two values, and neither is the string that spells it.
- The crate tag `cusipcode(65057)` carries the normalized column in a [FIX capture](index.md#fix-message-definitions).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::securities
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/types/datatype.test.js
    ```
