# ISIN

ISO 6166's international securities identification number: twelve characters, closed by its own check digit.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `isin`, `IsinCodeType`/`IsinCodeField`, the `IsinCode` value and `Scalar::IsinCode` |
| Validates | Twelve ASCII bytes: a two-letter prefix, nine alphanumerics, one digit that closes the eleven before it; lower case folds at the value door |
| Lazy | Nothing - the whole check runs on the stack, over bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling of the wrong length or shape, and a check digit that does not close the number; the empty text, so there is no default value |

A spelling whose check digit does not close it is not that identifier - it is a typo, and a typo typed as a security joins to the wrong one.

## DataType

`isin` is the one spelling, `DataType::isin()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::isin(), DataType::IsinCode);
    assert_eq!(DataType::from_str("isin")?, DataType::IsinCode);
    assert_eq!(DataType::IsinCode.to_string(), "isin");
    assert_eq!(DataType::IsinCode.kind(), DataTypeKind::Code);
    assert_eq!(DataType::IsinCode.code_name(), Some("isin"));
    assert_eq!(DataType::IsinCode.code_width(), Some(12));
    assert_eq!(DataType::IsinCode.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    isin = DataType("isin")
    assert isin.id == "isin"
    assert isin.kind == "code"
    assert isin.code_width == 12
    assert isin.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const isin = new DataType('isin')
    assert.equal(isin.id, 'isin')
    assert.equal(isin.kind, 'code')
    assert.equal(isin.codeWidth, 12)
    assert.equal(isin.fixedByteWidth, null)
    ```

## Field

`IsinCodeField` is the typed marker; Python and JavaScript name the factory `isin`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, IsinCodeField};

    let sid = IsinCodeField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::IsinCode);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::IsinCode, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sid = yggdryl.isin("sid")
    assert isinstance(sid, Field)
    assert str(sid.dtype) == "isin"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const sid = fields.isin('sid')
    assert.equal(sid.dtype.toString(), 'isin')
    ```

## Scalar

The value is the canonical spelling: upper case, closed by its check digit. Lower case is read as the upper case it spells, because the check digit expands a letter by its alphabet position and case does not change that.

=== "Rust"

    ```rust
    use yggdryl::{DataType, IsinCode, Scalar};

    let apple = DataType::isin().scalar("us0378331005")?;
    assert_eq!(apple, Scalar::IsinCode(IsinCode::new("US0378331005")?));
    assert_eq!(apple.as_str(), Some("US0378331005"));
    assert_eq!(apple.kind(), "isin");

    // One digit off is a typo, not a security.
    assert!(DataType::isin().scalar("US0378331006").is_err());
    assert_ne!(apple, Scalar::from("US0378331005"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    apple = DataType("isin").scalar("us0378331005")
    assert apple.as_py() == "US0378331005"
    assert apple.kind == "isin"

    with pytest.raises(ValueError, match="check digit does not close"):
        DataType("isin").scalar("US0378331006")
    with pytest.raises(ValueError, match="expected twelve characters"):
        DataType("isin").scalar("US037833100")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const isin = new DataType('isin')
    assert.equal(isin.scalar('us0378331005').asJs(), 'US0378331005')
    assert.throws(() => isin.scalar('US0378331006'), /check digit does not close/)
    assert.throws(() => isin.scalar('US037833100'), /expected twelve characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.isin` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::IsinCode, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.isin");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "isin")
    arrow_field = sid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.isin"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['US0378331005']), fields.isin('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['US0378331005'])
    ```

## The check digit

Two letters of prefix - the numbering agency's country, or an international prefix such as `XS` - then nine alphanumerics of national number, then the Luhn digit of the eleven before them, each letter first expanded to the two digits of its alphabet position. `IsinCode::is_valid`, `is_canonical` and `closing_digit` answer the rule without building a value, and `prefix`, `nsin` and `check_digit` read a built one apart. Rust only; the other bindings reach the same rule through the value door above.

```rust
use yggdryl::IsinCode;

let apple = IsinCode::new("US0378331005")?;
assert_eq!(apple.prefix(), "US");
assert_eq!(apple.nsin(), "037833100");
assert_eq!(apple.check_digit(), 5);

// The rule answers without building a value, in either case.
assert!(IsinCode::is_valid("us0378331005"));
assert!(IsinCode::is_canonical("US0378331005"));
assert!(!IsinCode::is_canonical("us0378331005"));
assert_eq!(IsinCode::closing_digit("US037833100"), Some(5));

// A typo is refused, and the refusal says why.
let refused = IsinCode::new("US0378331006").unwrap_err().to_string();
assert!(refused.contains("the check digit does not close the number"), "{refused}");
```

## A column holds the canonical spelling

A scalar read folds the case; a column's bytes are what every reader digests, so an Arrow cast is held to the canonical spelling and answers null under the default `safe` for anything else - a typo or a lower-case spelling alike. Strict names the row and the column.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let sid = Field::new("sid", DataType::IsinCode, true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["US0378331005", "us0378331005"]));
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

    sid = Field("sid", "isin")
    # Safe: the refused cell is null and the rest of the column stands.
    source = pa.array(["US0378331005", "us0378331005", "US0378331006"])
    assert Serie.from_arrow_array(source, sid).as_py() == ["US0378331005", None, None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const source = utf8(['US0378331005', 'us0378331005', 'US0378331006'])
    const stored = Serie.fromArrowArray(source, fields.isin('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['US0378331005', null, null])
    ```

`try_cast(sid as isin)` in an [expression](../../expression/terms.md) is that safe cast: an identifier the check digit closes answers, anything else is null. `cast(sid as isin)` is the strict one and refuses the column.

## Edges

- `expected twelve characters`, `expected a two-letter prefix`, `expected nine alphanumerics after the prefix`, `expected a closing check digit`, `the check digit does not close the number` - the five refusals, each naming `isin` and the spelling it saw.
- A thirteenth byte -> `at most 12 bytes`, the refusal any code of that width gives.
- No default value: the empty text names no security, so an empty text cell entering the column is null, as it is for a UUID ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("isin")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about an identifier, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- A lifecycle may learn a missing matching identifier or CFI attribute only under an already-valid ISIN in its own [graph walk](../../graph.md); that association registry is not a codec parser, a global mapper, or a replacement for a stated fact.
- `SecurityIDSource(22)` and the crate tag `isincode(65055)` carry the normalized column in a [FIX capture](index.md#fix-message-definitions).
- The prefix is the numbering agency's, which includes international prefixes no [country](country.md) names, so it is read as text rather than as that code.

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
