# SEDOL

The London Stock Exchange's seven-character securities identifier: six alphanumerics and the weighted check digit that closes them.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `sedol`, `SedolType`/`SedolField`, the `Sedol` value and `Scalar::Sedol` |
| Validates | Seven ASCII bytes: six alphanumerics weighted `1, 3, 1, 7, 3, 9` and one digit closing the weighted sum to a multiple of ten; lower case folds at the value door |
| Lazy | Nothing - the whole check runs on the stack, over bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling of the wrong length or shape, and a check digit that does not close the identifier; the empty text, so there is no default value |

## DataType

`sedol` is the one spelling, `DataType::sedol()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::sedol(), DataType::Sedol);
    assert_eq!(DataType::from_str("sedol")?, DataType::Sedol);
    assert_eq!(DataType::Sedol.to_string(), "sedol");
    assert_eq!(DataType::Sedol.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Sedol.code_name(), Some("sedol"));
    assert_eq!(DataType::Sedol.code_width(), Some(7));
    assert_eq!(DataType::Sedol.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    sedol = DataType("sedol")
    assert sedol.id == "sedol"
    assert sedol.kind == "code"
    assert sedol.code_width == 7
    assert sedol.fixed_byte_width is None
    assert DataType.from_logical_name("sedol") == sedol
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const sedol = new DataType('sedol')
    assert.equal(sedol.id, 'sedol')
    assert.equal(sedol.kind, 'code')
    assert.equal(sedol.codeWidth, 7)
    assert.equal(DataType.fromLogicalName('sedol').toString(), 'sedol')
    ```

## Field

`SedolField` is the typed marker; Python and JavaScript name the factory `sedol`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, SedolField};

    let sid = SedolField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::Sedol);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::Sedol, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sid = yggdryl.sedol("sid")
    assert isinstance(sid, Field)
    assert str(sid.dtype) == "sedol"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const sid = fields.sedol('sid')
    assert.equal(sid.dtype.toString(), 'sedol')
    ```

## Scalar

The value is the canonical spelling: upper case, closed by its check digit.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Sedol};

    let shell = DataType::sedol().scalar("b0ybkj7")?;
    assert_eq!(shell, Scalar::Sedol(Sedol::new("B0YBKJ7")?));
    assert_eq!(shell.as_str(), Some("B0YBKJ7"));
    assert_eq!(shell.kind(), "sedol");

    // One digit off is a typo, not a security.
    assert!(DataType::sedol().scalar("B0YBKJ8").is_err());
    assert_ne!(shell, Scalar::from("B0YBKJ7"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    sedol = DataType("sedol")
    assert sedol.scalar("b0ybkj7").as_py() == "B0YBKJ7"
    assert sedol.scalar("b0ybkj7").kind == "sedol"

    with pytest.raises(ValueError, match="check digit does not close"):
        sedol.scalar("B0YBKJ8")
    with pytest.raises(ValueError, match="expected seven characters"):
        sedol.scalar("B0YBKJ")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const sedol = new DataType('sedol')
    assert.equal(sedol.scalar('b0ybkj7').asJs(), 'B0YBKJ7')
    assert.throws(() => sedol.scalar('B0YBKJ8'), /check digit does not close/)
    assert.throws(() => sedol.scalar('B0YBKJ'), /expected seven characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.sedol` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::Sedol, false);
    let arrow = sid.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.sedol");
    assert_eq!(Field::from_arrow_field(&arrow)?, sid);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    sid = Field("sid", "sedol")
    arrow_field = sid.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.sedol"
    assert Field.from_arrow(arrow_field) == sid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    // A column holds the canonical spelling, so a cast lets in an identifier
    // the check digit closes and answers null for a lower-case spelling.
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['B0YBKJ7', 'b0ybkj7']), fields.sedol('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['B0YBKJ7', null])
    ```

## The check digit

Each of the six leading characters reads as a digit or as ten plus its alphabet position, weighted `1, 3, 1, 7, 3, 9` in turn; the digit is what closes the weighted sum to a multiple of ten. `Sedol::is_valid`, `is_canonical` and `closing_digit` answer the rule without building a value. Rust only.

```rust
use yggdryl::Sedol;

let shell = Sedol::new("B0YBKJ7")?;
assert_eq!(shell.check_digit(), 7);
assert_eq!(Sedol::closing_digit("B0YBKJ"), Some(7));
assert_eq!(Sedol::new("0263494")?.check_digit(), 4);

// The rule answers without building a value, in either case.
assert!(Sedol::is_valid("b0ybkj7"));
assert!(Sedol::is_canonical("B0YBKJ7"));
assert!(!Sedol::is_canonical("b0ybkj7"));
assert!(!Sedol::is_valid("B0YBKJ8"));
```

## A column holds the canonical spelling

A scalar read folds the case; a column's bytes are what every reader digests, so an Arrow cast is held to the canonical spelling and answers null under the default `safe` for a typo or a lower-case spelling alike. Strict names the row and the column, and `try_cast(sid as sedol)` in an [expression](../../expression/terms.md) is that safe cast.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let sid = Field::new("sid", DataType::Sedol, true);
    let source: ArrayRef = Arc::new(StringArray::from(vec!["B0YBKJ7", "b0ybkj7"]));
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

    sid = Field("sid", "sedol")
    stored = Serie.from_arrow_array(pa.array(["B0YBKJ7", "b0ybkj7", "B0YBKJ8"]), sid)
    assert stored.as_py() == ["B0YBKJ7", None, None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['B0YBKJ7', 'B0YBKJ8']), fields.sedol('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['B0YBKJ7', null])
    ```

## Edges

- `expected seven characters`, `expected six alphanumerics before the check digit`, `expected a closing check digit`, `the check digit does not close the identifier` - the four refusals, each naming `sedol` and the spelling it saw.
- An eighth byte -> `at most 7 bytes`, the refusal any code of that width gives.
- No default value: the empty text names no security, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("sedol")` answers an enum of no members, and no Python code class declares it.
- Nothing partial about an identifier, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- No crate column: in a [FIX capture](index.md#fix-message-definitions) a SEDOL is one security identifier under key `SEDOL` - `SecurityID(48)` under source `2`, or a `secaltids` occurrence - read as `get_securityids().get("SEDOL")`, and derived from a GB, IE, GG, JE or IM ISIN where the message states none; the crate tag `sedolcode(65058)` is retired and never reused.

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
