# CFI

ISO 10962's six-character classification of financial instruments: a category, a group, and four attribute positions.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `cfi`, `CfiType`/`CfiField`, the `Cfi` value and `Scalar::Cfi`, and the `CFI_CATEGORIES` grid with `CfiCategory`/`CfiGroup` |
| Validates | At the value door: US-ASCII, no NUL, at most six bytes. The category grid is a *reading*, answered by `is_classified`, not a gate the value door applies |
| Lazy | Nothing - classification and merging walk the six bytes on the stack |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A seventh byte or a byte past `0x7F`, naming the width: `at most 6 bytes` |

Six bytes is what the code is, not the eight some other width would pad it to. The grid is generated from the ISO 10962:2021 code list published by SIX Financial Information, the standard's Maintenance Agency (`cfi-20210507-current`), which the 2021 edition publishes as a free external list.

## DataType

`cfi` is the one spelling, `DataType::cfi()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::cfi(), DataType::Cfi);
    assert_eq!(DataType::from_str("cfi")?, DataType::Cfi);
    assert_eq!(DataType::Cfi.to_string(), "cfi");
    assert_eq!(DataType::Cfi.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Cfi.code_name(), Some("cfi"));
    assert_eq!(DataType::Cfi.code_width(), Some(6));
    assert_eq!(DataType::Cfi.fixed_byte_width(), None);
    // Six bytes with an identity is not six bytes of text.
    assert_ne!(DataType::Cfi, DataType::fixed_ascii(6)?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    cfi = DataType("cfi")
    assert (cfi.id, cfi.code_width, cfi.kind) == ("cfi", 6, "code")
    assert cfi.fixed_byte_width is None
    assert cfi != DataType.fixed_ascii(6)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const cfi = new DataType('cfi')
    assert.equal(cfi.id, 'cfi')
    assert.equal(cfi.kind, 'code')
    assert.equal(cfi.codeWidth, 6)
    assert.ok(!cfi.equals(DataType.fixedAscii(6)))
    ```

## Field

`CfiField` is the typed marker; Python and JavaScript name the factory `cfi`.

=== "Rust"

    ```rust
    use yggdryl::{CfiField, DataType, Field};

    let classification = CfiField::unit("classification", true);
    assert_eq!(classification.dtype(), &DataType::Cfi);
    assert_eq!(
        classification.to_field(),
        Field::new("classification", DataType::Cfi, true)
    );
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    classification = yggdryl.cfi("classification")
    assert isinstance(classification, Field)
    assert str(classification.dtype) == "cfi"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const classification = fields.cfi('classification')
    assert.equal(classification.dtype.toString(), 'cfi')
    ```

## Scalar

The value is the six characters, under the classification's identity.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, DataType, Scalar};

    let share = DataType::cfi().scalar("ESVUFR")?;
    assert_eq!(share, Scalar::Cfi(Cfi::new("ESVUFR")?));
    assert_eq!(share.as_str(), Some("ESVUFR"));
    assert_eq!(share.kind(), "cfi");
    assert_ne!(share, Scalar::from("ESVUFR"));

    // The value door holds the width; the grid is read separately.
    let refused = Cfi::new("ESVUFRX").unwrap_err().to_string();
    assert!(refused.contains("at most 6 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    share = DataType("cfi").scalar("ESVUFR")
    assert share.as_py() == "ESVUFR"
    assert share.kind == "cfi"

    with pytest.raises(ValueError, match="at most 6 bytes"):
        DataType("cfi").scalar("ESVUFRX")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const share = new DataType('cfi').scalar('ESVUFR')
    assert.equal(share.asJs(), 'ESVUFR')
    assert.equal(share.kind, 'cfi')
    assert.throws(() => new DataType('cfi').scalar('ESVUFRX'), /at most 6 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.cfi` ([Codes](index.md#arrow-storage)). The column stores the six characters it is and nothing beside them.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let classification = Field::new("classification", DataType::Cfi, false);
    let arrow = classification.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.cfi");
    assert_eq!(Field::from_arrow_field(&arrow)?, classification);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    classification = Field("classification", "cfi")
    arrow_field = classification.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.cfi"
    assert Field.from_arrow(arrow_field) == classification
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['ESVUFR']), fields.cfi('cfi'))
    assert.deepEqual([...stored.intoArrowArray()], ['ESVUFR'])
    ```

## The category grid

Position 1 is the category, position 2 the group inside it, and positions 3 to 6 the four attributes that group defines. `X` means "not applicable or unknown" and reads at any attribute position; it is never a category and never a group, so `XXXXXX` classifies nothing. An empty attribute set means the position does not apply to that group, and only `X` reads there.

| Letter | Category |
| --- | --- |
| `E` | Equities |
| `C` | CIVs |
| `D` | Debt instruments |
| `R` | Entitlement (rights) |
| `O` | Listed options |
| `F` | Futures |
| `S` | Swaps |
| `H` | Non-listed and complex listed options |
| `I` | Spot |
| `J` | Forwards |
| `K` | Strategies |
| `L` | Financing |
| `T` | Referential instruments |
| `M` | Others (miscellaneous) |

`Cfi::is_classified` answers the grid, `category_of` and `CfiCategory::group` walk it, and `Cfi::coarse` builds a code from a category and a group a caller inferred with every attribute unknown - falling back to the category's own Others group rather than to `X`, because `X` is not a group. Rust only.

```rust
use yggdryl::Cfi;

assert_eq!(Cfi::LENGTH, 6);
assert_eq!(Cfi::UNKNOWN, 'X');
assert!(Cfi::is_classified("ESVUFR"));
assert!(Cfi::is_classified("ESXXXX"));
// `X` is not a category and not a group.
assert!(!Cfi::is_classified("XXXXXX"));
assert!(!Cfi::is_classified("EXXXXX"));
// `Z` is no voting right a common share has.
assert!(!Cfi::is_classified("ESZUFR"));
// Position 5 is not applicable to a hedge fund, so only `X` reads there.
assert!(Cfi::is_classified("CHAXXX"));
assert!(!Cfi::is_classified("CHAAXX"));

let equities = Cfi::category_of('E').expect("a category");
assert_eq!(equities.name(), "Equities");
assert_eq!(equities.group('S').expect("a group").name(), "Common/ordinary shares");

// A category and a group with every attribute unknown.
assert_eq!(Cfi::coarse('E', Some('S')).as_deref(), Some("ESXXXX"));
// The category's own Others group is how the standard says "kind unspecified".
assert_eq!(Cfi::coarse('E', None).as_deref(), Some("EMXXXX"));
assert_eq!(Cfi::coarse('X', None), None);
```

A classified code that says nothing past its category and group - `ESXXXX` - is coarse. `Cfi::is_detailed` answers whether one of positions 3 to 6 is stated, which is what a market reading that keeps only detailed codes asks. Rust only.

```rust
use yggdryl::Cfi;

assert!(Cfi::is_detailed("ESVUFR"));
assert!(Cfi::is_detailed("ESVXXX"));
assert!(!Cfi::is_detailed("ESXXXX"));
assert!(!Cfi::is_detailed("XXXXXX"));
```

## Two statements of one instrument

`Cfi::merged` folds two classifications position by position: within one `(category, group)` a stated attribute fills an unknown one, and two different stated attributes answer `X`, because ambiguity answers nothing. Two different categories or groups are two subjects rather than one disagreement, so they answer nothing at all. [`merge_with`](index.md#the-code-family-value) is that reading as the `CodeValue` leaf contract states it - the merged code where the two describe one instrument, this code as it is where they do not. Rust only.

```rust
use yggdryl::{Cfi, CodeValue};

// What one statement left unsaid, the other says.
assert_eq!(Cfi::merged("ESXXXX", "ESVUFR").as_deref(), Some("ESVUFR"));
// Each fills the other's gaps.
assert_eq!(Cfi::merged("ESVXXX", "ESXUFR").as_deref(), Some("ESVUFR"));
// A disagreement inside one instrument is unknown, not a winner.
assert_eq!(Cfi::merged("ESVUFR", "ESNUFR").as_deref(), Some("ESXUFR"));
// Two different instruments are not one.
assert_eq!(Cfi::merged("ESVUFR", "DBFNFB"), None);
assert_eq!(Cfi::merged("ESVUFR", "EPVNFR"), None);

// The leaf contract reads the same rule, keeping this code where they do not.
assert_eq!(Cfi::new("ESVXXX")?.merge_with(&Cfi::new("ESXUFR")?).as_str(), "ESVUFR");
assert_eq!(Cfi::new("ESVUFR")?.merge_with(&Cfi::new("DBFNFB")?).as_str(), "ESVUFR");
```

## Edges

- `at most 6 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- The grid is a reading, not a gate: a six-byte value the grid does not classify is stored, and `is_classified` is what asks. That is why `CFICode(461)` from a dialect this build does not know still lands in a column.
- The default value is the empty text, answered as a `cfi` scalar ([Cast](../cast.md#empty-text)).
- `Cfi::merged` answers `None` for anything that is not two well-formed codes; `merge_with` turns that `None` back into this code.
- Python declares the vocabulary over the width as `yggdryl.enums.CFI`, over the `yggdryl.enums.Cfi` base a caller subclasses for a vocabulary of its own; `StringEnum::from_logical_name("cfi")` answers an enum of no members, because the grid is a rule rather than a listing.
- `CFICode(461)` is the standard classification field, so no crate tag 65056 exists ([FIX message definitions](index.md#fix-message-definitions)).
- A lifecycle may learn a missing CFI attribute only under an already-valid [ISIN](isin.md) in its own [graph walk](../../graph/index.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi::coded code::datatypes state::coded timeinforce::coded
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
