# ELF

ISO 20275's Entity Legal Form code: four letters or digits naming one legal form in GLEIF's code list - held by its shape, every well-shaped value of one rank.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `elf`, `ElfType`/`ElfField`, the `Elf` value and `Scalar::Elf` |
| Validates | The shape: four ASCII letters or digits; lower case folds at the value door. Every well-shaped value ranks the same: there is no check character, and whether the code list assigns a code is not a question the value answers |
| Lazy | Nothing - the accepted four bytes stay inline, so the constructor, the clone and the shared field allocate nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A length other than four, punctuation; the empty text, so there is no default value |

An ELF code names the legal form an entity is registered under, so it sits beside an [LEI](lei.md) in reference data rather than identifying anything itself.

## DataType

`elf` is the one spelling, `DataType::elf()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::elf(), DataType::Elf);
    assert_eq!(DataType::from_str("elf")?, DataType::Elf);
    assert_eq!(DataType::Elf.to_string(), "elf");
    assert_eq!(DataType::Elf.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Elf.code_name(), Some("elf"));
    assert_eq!(DataType::Elf.code_width(), Some(4));
    assert_eq!(DataType::Elf.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    elf = DataType("elf")
    assert (elf.id, elf.code_width, elf.kind) == ("elf", 4, "code")
    assert elf.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const elf = DataType.fromString('elf')
    assert.equal(elf.id, 'elf')
    assert.equal(elf.kind, 'code')
    assert.equal(elf.codeWidth, 4)
    assert.equal(elf.fixedByteWidth, null)
    ```

## Field

`ElfField` is the typed marker; Python and JavaScript name the factory `elf`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, ElfField, Field};

    let form = ElfField::unit("elf", false);
    assert_eq!(form.dtype(), &DataType::Elf);
    assert_eq!(form.to_field(), Field::new("elf", DataType::Elf, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.elf("elf", nullable=False).dtype == DataType("elf")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.elf('elf', { nullable: false }).dtype.id, 'elf')
    ```

## Scalar

The value is the canonical spelling: upper case. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Elf, Scalar};

    let form = Elf::new("2hbr")?;
    assert_eq!(form.as_str(), "2HBR");
    assert_eq!(DataType::elf().scalar("8888")?, Scalar::Elf(Elf::new("8888")?));
    assert_eq!(DataType::elf().scalar("8888")?.kind(), "elf");
    // The shape is the refusal.
    assert!(Elf::new("2HB").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    elf = DataType("elf")
    assert elf.scalar("2hbr").as_py() == "2HBR"
    assert elf.scalar("8888").kind == "elf"

    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected four characters"):
        elf.scalar("2HB")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const elf = DataType.fromString('elf')
    assert.equal(elf.scalar('2hbr').asJs(), '2HBR')
    assert.equal(elf.scalar('8888').kind, 'elf')
    // The shape is the refusal.
    assert.throws(() => elf.scalar('2HB'), /expected four characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.elf` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let form = Field::new("elf", DataType::Elf, false);
    let arrow = form.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.elf");
    assert_eq!(Field::from_arrow_field(&arrow)?, form);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    form = Field("elf", "elf")
    arrow_field = form.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.elf"
    assert Field.from_arrow(arrow_field) == form
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['8888']), fields.elf('elf'))
    assert.deepEqual([...stored.intoArrowArray()], ['8888'])
    ```

## A legal form, not a check

Four letters or digits, and nothing more to read: ISO 20275 gives the code no check character, and GLEIF publishes the list of assigned codes separately and revises it, so a value states a legal form without proving one. Every well-shaped value therefore holds the one [rank](index.md#rank) - `is_real` answers `true` for each, and no spelling stands as a placeholder for "none applies" - and [`merge_with`](index.md#the-code-family-value) keeps the value that leads. `is_canonical` answers whether text is the canonical spelling, four upper-case letters or digits. Four bytes are within what `ascii_packed` fills, so a code packs to a 32-bit integer and a `StringEnum` may name a vocabulary of legal forms over an `elf` column. Rust only.

```rust
use yggdryl::{CodeValue, DataType, Elf};

let form = Elf::new("8888")?;
assert!(form.is_real());
assert_eq!(<Elf as CodeValue>::MAX_RANK, 1);
assert!(Elf::is_canonical("2HBR"));
assert!(!Elf::is_canonical("2hbr"));

// Two well-shaped codes are of one rank: the one that leads stands.
assert_eq!(form.clone().merge_with(&Elf::new("2HBR")?), form);

// Four bytes pack, big-endian, into one 32-bit integer.
assert_eq!(DataType::Elf.ascii_packed(b"8888")?, 0x3838_3838);

// Three characters, five, and punctuation are not the shape.
for refused in ["2HB", "2HBR5", "2H-R"] {
    assert!(Elf::new(refused).is_err(), "{refused}");
}
```

## Edges

- `expected four characters`, `expected four letters or digits` - the two refusals, each naming `elf` and the spelling it saw; a fifth byte -> `at most 4 bytes`, the refusal any code of that width gives.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md) and [FIGI](figi.md) are: a lower-case spelling is null under the default `safe`.
- Serde reads an ELF code through the same door: a document holding a spelling that is not the shape is refused rather than deserialized, and lower case is folded.
- No default value: the empty text names no legal form, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No shipped vocabulary: `StringEnum::from_logical_name("elf")` answers an enum of no members, because the code list is GLEIF's to revise, not the package's to freeze.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- elf::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "reference_data_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="reference-data codes" node/tests/datatype.test.js
    ```
