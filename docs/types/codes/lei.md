# LEI

ISO 17442's Legal Entity Identifier: twenty characters, eighteen letters or digits and the two check digits that close them - held by its shape, ranked by whether the digits close.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `lei`, `LeiType`/`LeiField`, the `Lei` value and `Scalar::Lei` |
| Validates | The shape: twenty ASCII bytes, eighteen letters or digits, then two decimal check digits; lower case folds at the value door. Whether the digits close the identifier is its [rank](index.md#rank), never a gate |
| Lazy | Nothing - the accepted twenty bytes stay inline, so the constructor, the clone and the shared field allocate nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A length other than twenty, punctuation among the eighteen leading characters, a letter in the check digits; the empty text, so there is no default value |

An LEI names a legal entity, not an instrument. `SecurityIDSource(22)=T` is its FIX security source, and an [identifier](../../graph/identifier.md#per-type-value-checks) of type `lei` is held to this code's shape and ranked by its check.

## DataType

`lei` is the one spelling, `DataType::lei()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::lei(), DataType::Lei);
    assert_eq!(DataType::from_str("lei")?, DataType::Lei);
    assert_eq!(DataType::Lei.to_string(), "lei");
    assert_eq!(DataType::Lei.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Lei.code_name(), Some("lei"));
    assert_eq!(DataType::Lei.code_width(), Some(20));
    assert_eq!(DataType::Lei.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    lei = DataType("lei")
    assert (lei.id, lei.code_width, lei.kind) == ("lei", 20, "code")
    assert lei.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const lei = DataType.fromString('lei')
    assert.equal(lei.id, 'lei')
    assert.equal(lei.kind, 'code')
    assert.equal(lei.codeWidth, 20)
    assert.equal(lei.fixedByteWidth, null)
    ```

## Field

`LeiField` is the typed marker; Python and JavaScript name the factory `lei`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, LeiField};

    let issuer = LeiField::unit("lei", false);
    assert_eq!(issuer.dtype(), &DataType::Lei);
    assert_eq!(issuer.to_field(), Field::new("lei", DataType::Lei, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.lei("lei", nullable=False).dtype == DataType("lei")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.lei('lei', { nullable: false }).dtype.id, 'lei')
    ```

## Scalar

The value is the canonical spelling: upper case, the check digits as stated. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Lei, Scalar};

    let apple = Lei::new("hwupkr0mpou8fgxbt394")?;
    assert_eq!(apple.as_str(), "HWUPKR0MPOU8FGXBT394");
    assert_eq!(DataType::lei().scalar("HWUPKR0MPOU8FGXBT394")?, Scalar::Lei(apple));
    assert_eq!(DataType::lei().scalar("HWUPKR0MPOU8FGXBT394")?.kind(), "lei");
    // One digit off is a typo: a value of rank zero, never a refusal.
    assert_eq!(Lei::new("HWUPKR0MPOU8FGXBT395")?.as_str(), "HWUPKR0MPOU8FGXBT395");
    // The shape is the refusal.
    assert!(Lei::new("HWUPKR0MPOU8FGXBT3A4").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    lei = DataType("lei")
    assert lei.scalar("hwupkr0mpou8fgxbt394").as_py() == "HWUPKR0MPOU8FGXBT394"
    assert lei.scalar("hwupkr0mpou8fgxbt394").kind == "lei"

    # One digit off is a typo: a value of rank zero, never a refusal.
    assert lei.scalar("HWUPKR0MPOU8FGXBT395").as_py() == "HWUPKR0MPOU8FGXBT395"
    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected two closing check digits"):
        lei.scalar("HWUPKR0MPOU8FGXBT3A4")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const lei = DataType.fromString('lei')
    assert.equal(lei.scalar('hwupkr0mpou8fgxbt394').asJs(), 'HWUPKR0MPOU8FGXBT394')
    // One digit off is a typo: a value of rank zero, never a refusal.
    assert.equal(lei.scalar('HWUPKR0MPOU8FGXBT395').asJs(), 'HWUPKR0MPOU8FGXBT395')
    // The shape is the refusal.
    assert.throws(() => lei.scalar('HWUPKR0MPOU8FGXBT3A4'), /expected two closing check digits/)
    ```

## Arrow storage

`Utf8` under `yggdryl.lei` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let issuer = Field::new("lei", DataType::Lei, false);
    let arrow = issuer.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.lei");
    assert_eq!(Field::from_arrow_field(&arrow)?, issuer);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    issuer = Field("lei", "lei")
    arrow_field = issuer.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.lei"
    assert Field.from_arrow(arrow_field) == issuer
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['HWUPKR0MPOU8FGXBT394']), fields.lei('lei'))
    assert.deepEqual([...stored.intoArrowArray()], ['HWUPKR0MPOU8FGXBT394'])
    ```

## The shape and the check digits

Eighteen letters or digits, then two decimal digits. The first four characters are the prefix of the Local Operating Unit that issued the code (`prefix`), and the fourteen after them are the entity's own. ISO 17442:2012 reserved the fifth and sixth characters as `00`; ISO 17442-1:2020 made them part of the entity's code, so `HWUPKR0MPOU8FGXBT394` - Apple Inc.'s LEI, `KR` in those places - is the shape, and a reader holding the 2012 rule would refuse it. The check is ISO/IEC 7064 MOD 97-10: the twenty characters read as one decimal number, a digit as itself and a letter as its two digits from `A` at ten to `Z` at thirty-five, leave a remainder of one modulo 97. `closing_digits` computes the two digits that close eighteen leading characters, `is_closed` answers whether they close an upper-case identifier - the [rank](index.md#rank) a value answers, one where they close, zero where they do not - `check_digits` reads the two as stated, and `is_canonical` whether text is the canonical spelling, upper case and the shape, whatever the digits. Rust only.

```rust
use yggdryl::{CodeValue, IdType, Lei};

let apple = Lei::new("HWUPKR0MPOU8FGXBT394")?;
assert_eq!(apple.prefix(), "HWUP");
assert_eq!(apple.check_digits(), 94);
assert_eq!(Lei::closing_digits("HWUPKR0MPOU8FGXBT3"), Some(94));
assert!(Lei::is_closed("HWUPKR0MPOU8FGXBT394"));
assert!(Lei::is_canonical("HWUPKR0MPOU8FGXBT394"));
assert!(!Lei::is_canonical("hwupkr0mpou8fgxbt394"));
assert!(apple.is_real());

// A typo is the shape: a value of rank zero, which a closing one replaces.
let typo = Lei::new("HWUPKR0MPOU8FGXBT395")?;
assert!(!Lei::is_closed(typo.as_str()));
assert_eq!(typo.rank(), 0);
assert_eq!(typo.merge_with(&apple), apple);

// An identifier of type `lei` ranks the same, its case folded.
assert_eq!(IdType::from_fix_security_source('T'), Some(IdType::Lei));
assert_eq!(IdType::Lei.rank("hwupkr0mpou8fgxbt394"), 1);
assert_eq!(IdType::Lei.rank("HWUPKR0MPOU8FGXBT395"), 0);

// Nineteen characters, punctuation in the body and a letter for a check
// digit are not the shape.
for refused in ["HWUPKR0MPOU8FGXBT39", "HWUPKR0MPOU8FGX-T394", "HWUPKR0MPOU8FGXBT3A4"] {
    assert!(Lei::new(refused).is_err(), "{refused}");
}
```

## Edges

- `expected twenty characters`, `expected eighteen letters or digits before the check digits`, `expected two closing check digits` - the three refusals, each naming `lei` and the spelling it saw; digits that do not close the identifier are a rank, never a refusal.
- A twenty-first byte -> `at most 20 bytes`, the refusal any code of that width gives.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md) and [FIGI](figi.md) are: a lower-case spelling is null under the default `safe`, and a typo lands as the value it is.
- Serde reads an LEI through the same door: a document holding a spelling that is not the shape is refused rather than deserialized, and lower case is folded.
- No default value: the empty text names no entity, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No packed integer: twenty bytes are past the sixteen `ascii_packed` fills, so neither `ascii_packed` nor a `StringEnum` takes an `lei` column.
- [`merge_with`](index.md#the-code-family-value) takes an identifier that closes over one that does not, whichever leads; two of one rank keep this one.
- An [`IsinRegistry`](../../graph/isin-registry.md) column of type `lei` declares this datatype; a store whose column is `utf8` reads each cell through this code's rule.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- lei::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "reference_data_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="reference-data codes" node/tests/datatype.test.js
    ```
