# DTI

ISO 24165's Digital Token Identifier: nine characters of a thirty-symbol alphabet, an eight-character base and the check character that closes it - held by its shape, ranked by whether the character closes.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `dti`, `DtiType`/`DtiField`, the `Dti` value and `Scalar::Dti` |
| Validates | The shape: nine ASCII bytes of `0123456789BCDFGHJKLMNPQRSTVWXZ`, the first not `0`; lower case folds at the value door. Whether the check character closes the base is its [rank](index.md#rank), never a gate |
| Lazy | Nothing - the accepted nine bytes stay inline, so the constructor, the clone and the shared field allocate nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A length other than nine, a vowel or `Y`, punctuation, a leading `0`; the empty text, so there is no default value |

A DTI names a digital token. `SecurityIDSource(22)=Y` is its FIX security source, and an [identifier](../../graph/identifier.md#per-type-value-checks) of type `dti` is held to this code's shape and ranked by its check.

## DataType

`dti` is the one spelling, `DataType::dti()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::dti(), DataType::Dti);
    assert_eq!(DataType::from_str("dti")?, DataType::Dti);
    assert_eq!(DataType::Dti.to_string(), "dti");
    assert_eq!(DataType::Dti.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Dti.code_name(), Some("dti"));
    assert_eq!(DataType::Dti.code_width(), Some(9));
    assert_eq!(DataType::Dti.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    dti = DataType("dti")
    assert (dti.id, dti.code_width, dti.kind) == ("dti", 9, "code")
    assert dti.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dti = DataType.fromString('dti')
    assert.equal(dti.id, 'dti')
    assert.equal(dti.kind, 'code')
    assert.equal(dti.codeWidth, 9)
    assert.equal(dti.fixedByteWidth, null)
    ```

## Field

`DtiField` is the typed marker; Python and JavaScript name the factory `dti`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DtiField, Field};

    let token = DtiField::unit("dti", false);
    assert_eq!(token.dtype(), &DataType::Dti);
    assert_eq!(token.to_field(), Field::new("dti", DataType::Dti, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.dti("dti", nullable=False).dtype == DataType("dti")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.dti('dti', { nullable: false }).dtype.id, 'dti')
    ```

## Scalar

The value is the canonical spelling: upper case, the check character as stated. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Dti, Scalar};

    let token = Dti::new("x9j9k872s")?;
    assert_eq!(token.as_str(), "X9J9K872S");
    assert_eq!(DataType::dti().scalar("X9J9K872S")?, Scalar::Dti(token));
    assert_eq!(DataType::dti().scalar("X9J9K872S")?.kind(), "dti");
    // One character off is a typo: a value of rank zero, never a refusal.
    assert_eq!(Dti::new("X9J9K872T")?.as_str(), "X9J9K872T");
    // The shape is the refusal.
    assert!(Dti::new("A9J9K872S").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    dti = DataType("dti")
    assert dti.scalar("x9j9k872s").as_py() == "X9J9K872S"
    assert dti.scalar("x9j9k872s").kind == "dti"

    # One character off is a typo: a value of rank zero, never a refusal.
    assert dti.scalar("X9J9K872T").as_py() == "X9J9K872T"
    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected digits or consonants other than Y"):
        dti.scalar("A9J9K872S")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const dti = DataType.fromString('dti')
    assert.equal(dti.scalar('x9j9k872s').asJs(), 'X9J9K872S')
    // One character off is a typo: a value of rank zero, never a refusal.
    assert.equal(dti.scalar('X9J9K872T').asJs(), 'X9J9K872T')
    // The shape is the refusal.
    assert.throws(() => dti.scalar('A9J9K872S'), /expected digits or consonants other than Y/)
    ```

## Arrow storage

`Utf8` under `yggdryl.dti` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let token = Field::new("dti", DataType::Dti, false);
    let arrow = token.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.dti");
    assert_eq!(Field::from_arrow_field(&arrow)?, token);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    token = Field("dti", "dti")
    arrow_field = token.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.dti"
    assert Field.from_arrow(arrow_field) == token
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['X9J9K872S']), fields.dti('dti'))
    assert.deepEqual([...stored.intoArrowArray()], ['X9J9K872S'])
    ```

## The alphabet and the check character

A DTI is written in thirty symbols - the ten digits and the consonants but `Y`, `0123456789BCDFGHJKLMNPQRSTVWXZ` - so no word can be spelled in one, and a symbol's value is its place in that list. The first eight characters are the base, which never opens with `0`, and the ninth is the check character under ISO/IEC 7064 hybrid MOD 31,30: from `p = 30`, each base symbol of value `v` sets `s = (p + v) mod 30` - thirty where that is zero - and `p = 2s mod 31`; the check symbol is the one whose value is `(31 - p) mod 30`. `closing_character` computes it, `is_closed` answers whether it closes an upper-case identifier - the [rank](index.md#rank) a value answers, one where it closes, zero where it does not - `check_character` reads the ninth as stated, and `is_canonical` whether text is the canonical spelling, upper case and the shape, whatever the check says. The algorithm is the rule and nothing else is: the crate keeps no table of exceptions, so a code the registry assigned whose stored check character the algorithm does not give ranks zero, exactly as a typo does, and a merge takes any code that closes over it. Rust only.

```rust
use yggdryl::{CodeValue, Dti, IdType};

let token = Dti::new("X9J9K872S")?;
assert_eq!(token.check_character(), 'S');
assert_eq!(Dti::closing_character("X9J9K872"), Some('S'));
assert!(Dti::is_closed("X9J9K872S"));
assert!(Dti::is_canonical("X9J9K872S"));
assert!(!Dti::is_canonical("x9j9k872s"));
assert!(token.is_real());

// A check character the algorithm does not give is a value of rank zero,
// which a closing one replaces.
let typo = Dti::new("X9J9K872T")?;
assert!(!Dti::is_closed(typo.as_str()));
assert_eq!(typo.rank(), 0);
assert_eq!(typo.merge_with(&token), token);

// An identifier of type `dti` ranks the same, its case folded.
assert_eq!(IdType::from_fix_security_source('Y'), Some(IdType::Dti));
assert_eq!(IdType::Dti.rank("x9j9k872s"), 1);

// A vowel, `Y`, a leading `0` and eight characters are not the shape.
for refused in ["A9J9K872S", "Y9J9K872S", "09J9K872S", "X9J9K872"] {
    assert!(Dti::new(refused).is_err(), "{refused}");
}
```

## Edges

- `expected nine characters`, `expected a first character other than 0`, `expected digits or consonants other than Y` - the three refusals, each naming `dti` and the spelling it saw; a check character that does not close the base is a rank, never a refusal.
- A tenth byte -> `at most 9 bytes`, the refusal any code of that width gives.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md) and [FIGI](figi.md) are: a lower-case spelling is null under the default `safe`, and a typo lands as the value it is.
- Serde reads a DTI through the same door: a document holding a spelling that is not the shape is refused rather than deserialized, and lower case is folded.
- No default value: the empty text names no token, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- [`merge_with`](index.md#the-code-family-value) takes an identifier that closes over one that does not, whichever leads; two of one rank keep this one.
- An [`IsinRegistry`](../../graph/isin-registry.md) column of type `dti` declares this datatype; a store whose column is `utf8` reads each cell through this code's rule.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- dti::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "reference_data_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="reference-data codes" node/tests/datatype.test.js
    ```
