# ISIN

ISO 6166's international securities identification number: twelve characters, held by their shape and ranked by whether the check digit closes them and an agency numbers under the prefix.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `isin`, `IsinType`/`IsinField`, the `Isin` value and `Scalar::Isin` |
| Validates | The shape: twelve ASCII bytes, a two-letter prefix, nine alphanumerics, one check digit; lower case folds at the value door. Whether the digit closes the number and whether an agency numbers under the prefix are its [rank](index.md#rank), never a gate |
| Lazy | Nothing - the whole check runs on the stack, over bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling of the wrong length or shape; the empty text, so there is no default value |

A spelling whose check digit does not close it is a typo or a mask - a masked line's `XX0000000001` - and a typo typed as a security joins to the wrong one. So it is held as a value of a lower rank rather than refused: a [merge](index.md#rank) replaces it with a real number whichever was stated first, and a derivation that needs the number to be real - the national identifier it embeds, the country of issue - asks `Isin::is_closed` first.

## DataType

`isin` is the one spelling, `DataType::isin()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::isin(), DataType::Isin);
    assert_eq!(DataType::from_str("isin")?, DataType::Isin);
    assert_eq!(DataType::Isin.to_string(), "isin");
    assert_eq!(DataType::Isin.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Isin.code_name(), Some("isin"));
    assert_eq!(DataType::Isin.code_width(), Some(12));
    assert_eq!(DataType::Isin.fixed_byte_width(), None);
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

`IsinField` is the typed marker; Python and JavaScript name the factory `isin`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, IsinField};

    let sid = IsinField::unit("sid", true);
    assert_eq!(sid.dtype(), &DataType::Isin);
    assert_eq!(sid.to_field(), Field::new("sid", DataType::Isin, true));
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

The value is the canonical spelling: upper case, of the number's shape. Lower case is read as the upper case it spells, because the check digit expands a letter by its alphabet position and case does not change that. A check digit that does not close is admitted as stated, at a lower [rank](#the-check-digit-and-the-rank).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Isin, Scalar};

    let apple = DataType::isin().scalar("us0378331005")?;
    assert_eq!(apple, Scalar::Isin(Isin::new("US0378331005")?));
    assert_eq!(apple.as_str(), Some("US0378331005"));
    assert_eq!(apple.kind(), "isin");

    // One digit off is a typo: a value of a lower rank, never a refusal.
    let typo = DataType::isin().scalar("US0378331006")?;
    assert_eq!(typo.as_str(), Some("US0378331006"));
    // The shape is the refusal.
    assert!(DataType::isin().scalar("US037833100").is_err());
    assert_ne!(apple, Scalar::from("US0378331005"));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    apple = DataType("isin").scalar("us0378331005")
    assert apple.as_py() == "US0378331005"
    assert apple.kind == "isin"

    # One digit off is a typo: a value of a lower rank, never a refusal.
    assert DataType("isin").scalar("US0378331006").as_py() == "US0378331006"
    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected twelve characters"):
        DataType("isin").scalar("US037833100")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const isin = new DataType('isin')
    assert.equal(isin.scalar('us0378331005').asJs(), 'US0378331005')
    // One digit off is a typo: a value of a lower rank, never a refusal.
    assert.equal(isin.scalar('US0378331006').asJs(), 'US0378331006')
    // The shape is the refusal.
    assert.throws(() => isin.scalar('US037833100'), /expected twelve characters/)
    ```

## Arrow storage

`Utf8` under `yggdryl.isin` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let sid = Field::new("sid", DataType::Isin, false);
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
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.isin"
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

<a id="the-check-digit"></a>

## The check digit and the rank

Two letters of prefix - the numbering agency's country, or an international prefix such as `XS` - then nine alphanumerics of national number, then the Luhn digit of the eleven before them, each letter first expanded to the two digits of its alphabet position. `closing_digit` computes it, and `prefix`, `nsin` and `check_digit` read a built value apart.

Two readings say how real a number is, and its [rank](index.md#rank) counts them. `Isin::is_closed` answers whether the digit closes the number, and `is_listed_prefix` whether an agency numbers under the prefix: an ISO 3166 country `StringEnum::COUNTRIES` lists, or one of the agency prefixes `EU`, `EZ`, `XA`, `XB`, `XC`, `XD`, `XF`, `XK`, `XS` and `XT`, the prefix of a referential instrument such as a digital token; `ZZ` - ISO 6166's prefix for a derivative no agency has numbered yet - and the user-assigned `XX` are listed nowhere. `rank_of` reads the rank off the text alone: two for a real number (`MAX_RANK`), one for a typo under a listed prefix or a closing `ZZ` number, zero for a masked one and for text that is not the upper-case shape. `is_canonical` is the strict question about the spelling - upper case and the shape - and says nothing of the digit. `Isin::NONE` is `XX0000000000`, the number stated as none: `Isin::none()` builds it, `is_none()` asks, and it is the lowest rank there is, so any stated number replaces it. Rust only; the other bindings reach the shape through the value door above.

```rust
use yggdryl::{CodeValue, Isin};

let apple = Isin::new("US0378331005")?;
assert_eq!(apple.prefix(), "US");
assert_eq!(apple.nsin(), "037833100");
assert_eq!(apple.check_digit(), 5);
assert_eq!(Isin::closing_digit("US037833100"), Some(5));

// Closing and listing are readings, and the rank counts them.
assert!(Isin::is_closed("US0378331005") && Isin::is_listed_prefix("US0378331005"));
assert_eq!(apple.rank(), <Isin as CodeValue>::MAX_RANK);
assert!(apple.is_real());
let typo = Isin::new("US0378331006")?;
assert!(!Isin::is_closed(typo.as_str()));
assert_eq!(typo.rank(), 1);
assert_eq!(Isin::rank_of("ZZ0000000008"), 1);
assert_eq!(Isin::rank_of("XX0000000001"), 0);

// The canonical spelling is upper case and the shape, whatever the digit.
assert!(Isin::is_canonical("US0378331006"));
assert!(!Isin::is_canonical("us0378331005"));

// The number stated as none is the lowest rank there is.
assert_eq!(Isin::NONE, "XX0000000000");
assert!(Isin::none().is_none());
assert_eq!(Isin::none().rank(), 0);
assert_eq!(Isin::none().merge_with(&typo), typo);

// The shape is the refusal, and it says why.
let refused = Isin::new("US037833100A").unwrap_err().to_string();
assert!(refused.contains("expected a closing check digit"), "{refused}");
```

## A minted number

An instrument no agency numbers - an FX pair, a forward, an option, a strategy - still gets an ISIN where a reader joins on `isin`: `Isin::minted(digest)` writes the prefix `QY` - a user-assigned code ISO 3166 leaves to private use, listed nowhere - then nine base-36 digits of the digest's low 46 bits and the digit that closes them, and `Isin::is_minted(text, digest)` asks whether a number is that mint of that digest, byte for byte, computing nothing; `Isin::MINTED_PREFIX` is the prefix. A minted number closes under a prefix no agency numbers under, so it ranks one - below every agency's number, which replaces it whichever leads. The [instrument](../../graph/instrument.md#the-minted-number) mints from the XXH3-128 of its cross code, so the number is a function of the code alone. Rust only; Python reaches it as `Instruments.mint(crosscode)`.

```rust
use yggdryl::{CodeValue, Isin};

let digest = yggdryl::xxhash::xxh128(b"IF:EUR/USD");
let minted = Isin::minted(digest);
assert_eq!(minted.as_str(), "QYLTVIRYHNX5");
assert!(minted.as_str().starts_with(Isin::MINTED_PREFIX));
assert!(Isin::is_minted(minted.as_str(), digest));
assert!(!Isin::is_minted("QY0000000000", digest), "another system's number of the shape");
assert!(Isin::is_closed(minted.as_str()) && !Isin::is_listed_prefix(minted.as_str()));
assert_eq!(minted.rank(), 1);
```

## A column holds the canonical spelling

A scalar read folds the case; a column's bytes are what every reader digests, so an Arrow cast is held to the canonical spelling - upper case, the shape - and answers null under the default `safe` for a lower-case spelling. A typo is a spelling of the shape and lands as the value it is, as the value door answers it. Strict names the row and the column.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let sid = Field::new("sid", DataType::Isin, true);
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
    # Safe: the refused cell is null and the rest of the column stands; a typo lands.
    source = pa.array(["US0378331005", "us0378331005", "US0378331006"])
    assert Serie.from_arrow_array(source, sid).as_py() == ["US0378331005", None, "US0378331006"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const source = utf8(['US0378331005', 'us0378331005', 'US0378331006'])
    const stored = Serie.fromArrowArray(source, fields.isin('sid'))
    assert.deepEqual([...stored.intoArrowArray()], ['US0378331005', null, 'US0378331006'])
    ```

`try_cast(sid as isin)` in an [expression](../../expression/terms.md) is that safe cast: a spelling of the shape answers, closing or not, and anything else is null. `cast(sid as isin)` is the strict one and refuses the column.

## Edges

- `expected twelve characters`, `expected a two-letter prefix`, `expected nine alphanumerics after the prefix`, `expected a closing check digit` - the four refusals, each naming `isin` and the spelling it saw; a digit that does not close the number is a rank, never a refusal.
- A thirteenth byte -> `at most 12 bytes`, the refusal any code of that width gives.
- No default value: the empty text names no security, so an empty text cell entering the column is null, as it is for a UUID ([Cast](../cast.md#empty-text)).
- No vocabulary: `StringEnum::from_logical_name("isin")` answers an enum of no members, and no Python code class declares it.
- A `ZZ` number - ISO 6166's placeholder for a derivative no agency has numbered yet - closes but is listed nowhere: rank one, which a real number replaces on a [`merge_with`](index.md#rank) whichever leads, as a real number replaces a typo and a typo a masked number. Two numbers of one rank are two statements, and the leading one stands.
- A lifecycle may learn a missing matching identifier or CFI attribute only under a real ISIN (`is_real`: closing under a listed prefix) - or, for an instrument no agency numbers, its cross code - in its own [graph walk](../../graph/event.md#lifecycle-walk), into the [instruments](../../graph/instrument.md) it fills from; that association is not a codec parser, a global mapper, or a replacement for a stated fact.
- `SecurityIDSource(22)` and the crate tag `isincode(65022)` carry the normalized column in a [FIX capture](index.md#fix-message-definitions): a view of the message's `isin` [security identifier](../../graph/identifier.md).
- The prefix is the numbering agency's, which includes international prefixes no [country](country.md) names, so it is read as text rather than as that code.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi::coded code::datatypes code::securities cusip::securities figi::securities isin:: sedol::securities
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
