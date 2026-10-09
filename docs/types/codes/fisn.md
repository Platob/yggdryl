# FISN

ISO 18774's Financial Instrument Short Name: at most thirty-five characters, an issuer's short name and the instrument's description either side of the first `/` - held by its shape, every well-shaped value of one rank.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `fisn`, `FisnType`/`FisnField`, the `Fisn` value and `Scalar::Fisn` |
| Validates | The shape: at most thirty-five printable ASCII bytes, delimiter included, holding a `/` with text on both sides of the first one; lower case folds at the value door. Every well-shaped value ranks the same: there is no check character |
| Lazy | Nothing past the value: a short name of at most twenty-three bytes stays inline, and a longer one is one shared allocation, as a [Bbg](bbg.md) or a [RIC](ric.md) is |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A thirty-sixth byte, a byte past `0x7F` - a Latin-1 letter included - a control character, no `/`, nothing before or after the first `/`; the empty text, so there is no default value |

A FISN is a name a person reads, not a key: two instruments may share one, and nothing about it closes. It sits beside the [ISIN](isin.md) and the [CFI](cfi.md) in an instrument's reference data. An [identifier](../../graph/identifier.md#per-type-value-checks) of type `fisn` - also spelled `fisncode` and `financialinstrumentshortname` - is a security identifier held to this code's shape: FIX gives it no `SecurityIDSource(22)` code, and a [FIX message](../../fix/message.md#typed-tags) states its `FinancialInstrumentShortName(2737)` in `securityids` under the base `fisn` key.

## DataType

`fisn` is the one spelling, `DataType::fisn()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::fisn(), DataType::Fisn);
    assert_eq!(DataType::from_str("fisn")?, DataType::Fisn);
    assert_eq!(DataType::Fisn.to_string(), "fisn");
    assert_eq!(DataType::Fisn.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Fisn.code_name(), Some("fisn"));
    assert_eq!(DataType::Fisn.code_width(), Some(35));
    assert_eq!(DataType::Fisn.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    fisn = DataType("fisn")
    assert (fisn.id, fisn.code_width, fisn.kind) == ("fisn", 35, "code")
    assert fisn.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const fisn = DataType.fromString('fisn')
    assert.equal(fisn.id, 'fisn')
    assert.equal(fisn.kind, 'code')
    assert.equal(fisn.codeWidth, 35)
    assert.equal(fisn.fixedByteWidth, null)
    ```

## Field

`FisnField` is the typed marker; Python and JavaScript name the factory `fisn`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FisnField, Field};

    let name = FisnField::unit("fisn", false);
    assert_eq!(name.dtype(), &DataType::Fisn);
    assert_eq!(name.to_field(), Field::new("fisn", DataType::Fisn, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    assert yggdryl.fisn("fisn", nullable=False).dtype == DataType("fisn")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    assert.equal(fields.fisn('fisn', { nullable: false }).dtype.id, 'fisn')
    ```

## Scalar

The value is the canonical spelling: upper case, every other byte as stated. Lower case input is normalized once at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Fisn, Scalar};

    let name = Fisn::new("acme corp/sh")?;
    assert_eq!(name.as_str(), "ACME CORP/SH");
    assert_eq!(DataType::fisn().scalar("ACME CORP/SH")?, Scalar::Fisn(name));
    assert_eq!(DataType::fisn().scalar("ACME CORP/SH")?.kind(), "fisn");
    // The shape is the refusal.
    assert!(Fisn::new("ACME CORP SH").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    fisn = DataType("fisn")
    assert fisn.scalar("acme corp/sh").as_py() == "ACME CORP/SH"
    assert fisn.scalar("acme corp/sh").kind == "fisn"

    # The shape is the refusal.
    with pytest.raises(ValueError, match="expected a '/' between the issuer"):
        fisn.scalar("ACME CORP SH")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const fisn = DataType.fromString('fisn')
    assert.equal(fisn.scalar('acme corp/sh').asJs(), 'ACME CORP/SH')
    // The shape is the refusal.
    assert.throws(() => fisn.scalar('ACME CORP SH'), /expected a '\/' between the issuer/)
    ```

## Arrow storage

`Utf8` under `yggdryl.fisn` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let name = Field::new("fisn", DataType::Fisn, false);
    let arrow = name.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.fisn");
    assert_eq!(Field::from_arrow_field(&arrow)?, name);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    name = Field("fisn", "fisn")
    arrow_field = name.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.fisn"
    assert Field.from_arrow(arrow_field) == name
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['ACME CORP/SH']), fields.fisn('fisn'))
    assert.deepEqual([...stored.intoArrowArray()], ['ACME CORP/SH'])
    ```

## Issuer, delimiter, description

A short name is the issuer's name, a `/`, then the instrument's description, thirty-five characters in all with the delimiter counted. The separator is the first `/`: a description may hold another (`AMORT PN W/P/C`), and dots, ampersands, parentheses, hyphens and colons besides, so `issuer` is everything before the first `/` and `description` everything after it. The ANNA guidelines bound the issuer at fifteen characters, leaving the description at least nineteen, save a collective investment vehicle's or an OTC derivative's, whose issuer may run longer - so fifteen is not a shape rule here, and only the thirty-five-byte whole is. The standard writes upper case alone, so lower case folds; spacing is kept as stated. ISO 18774 draws its characters from ISO/IEC 8859-1, while a code here stores US-ASCII: a short name holding a Latin-1 letter is refused rather than transliterated. There is no check character, so every well-shaped value holds the one [rank](index.md#rank) and [`merge_with`](index.md#the-code-family-value) keeps the value that leads. `is_canonical` answers whether text is the canonical spelling, upper case and the shape. Rust only.

```rust
use yggdryl::{CodeValue, Fisn};

let name = Fisn::new("ACME CORP/AMORT PN W/P/C")?;
assert_eq!(name.issuer(), "ACME CORP");
assert_eq!(name.description(), "AMORT PN W/P/C");
assert!(name.is_real());
assert!(Fisn::is_canonical("ACME CORP/SH"));
assert!(!Fisn::is_canonical("acme corp/sh"));

// The issuer may run past fifteen characters; only the whole is bounded.
let long = Fisn::new("ACME CORPORATION HOLDINGS/SH")?;
assert_eq!(long.issuer().len(), 25);
assert!(Fisn::new("ACME CORPORATION INCORPORATED/ORD SH").is_err(), "thirty-six bytes");

// No `/`, nothing before or after it, a control character and a Latin-1
// letter are not the shape.
for refused in ["ACME CORP SH", "/SH", "ACME CORP/", "ACME\tCORP/SH", "SOCIÉTÉ/SH"] {
    assert!(Fisn::new(refused).is_err(), "{refused}");
}
```

## In the instrument registry

An [`IsinRegistry`](../../graph/isin-registry.md) row holds an instrument's short name in its `fisn` column, typed `fisn`, right after `ticker`: an instrument fact, held alike on every [listing](../../graph/isin-registry.md#listings) of its ISIN - one row per market - and filled into an element on any market. A lifecycle learns it where a message states one - `FinancialInstrumentShortName(2737)`, or a `fisn` security identifier - and fills it into an element stating none as a `derived` identifier; the [seed](../../graph/isin-registry.md#seed) states it where FIRDS spells one. A merge replaces a held name by one that differs, as every column.

=== "Rust"

    ```rust
    use yggdryl_market::IsinRegistry;
    yggdryl_market::install()?;

    let registry = IsinRegistry::seeded();
    let name = registry.get("US0378331005").and_then(|row| row.fisn()).expect("seeded");
    assert_eq!(name.as_str(), "APPLE INC/SH SH");
    assert_eq!((name.issuer(), name.description()), ("APPLE INC", "SH SH"));
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    registry = IsinRegistry.seeded()
    row = registry.get("US0378331005")
    assert row is not None and row["fisn"] == "APPLE INC/SH SH"
    field = IsinRegistry.field()
    assert field.index_of("fisn") == field.index_of("ticker") + 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const registry = IsinRegistry.seeded()
    assert.equal(registry.get('US0378331005').fisn, 'APPLE INC/SH SH')
    const field = IsinRegistry.field()
    assert.equal(field.indexOf('fisn'), field.indexOf('ticker') + 1)
    ```

## Similarity

`Fisn::similarity` scores how alike two short names are, from `0` to `1`: one less the Levenshtein distance between their bytes over the longer one's length - both already upper case - symmetric, `1` for two equal names, and allocating nothing. It is what the instrument registry's [economic match](../../graph/isin-registry.md#by-short-name) weighs, at or above its `economic_threshold` (`0.85` unless set). Python and JavaScript hold no `Fisn` value, so the score crosses there as a `resolve` answer's `similarity`.

=== "Rust"

    ```rust
    use yggdryl::Fisn;

    let apple = Fisn::new("APPLE INC/SH")?;
    let dotted = Fisn::new("APPLE INC./SH")?;
    assert_eq!(apple.similarity(&apple), 1.0);
    assert!((apple.similarity(&dotted) - 12.0 / 13.0).abs() < 1e-12, "one insertion in thirteen");
    assert_eq!(apple.similarity(&dotted), dotted.similarity(&apple));
    assert_eq!(apple.similarity(&Fisn::new("APPLE INC/SH USD")?), 0.75);
    ```

=== "Python"

    ```python
    import math

    from yggdryl import Identifier, IsinRegistry, graph

    registry = IsinRegistry()
    registry.merge({"isin": "US0378331005", "miccode": "XNAS", "fisn": "APPLE INC./SH"})
    order = graph.OrderEvent(1, securityids=[Identifier("fisn", "APPLE INC/SH")], currency="USD")
    answer = registry.resolve(order)
    assert answer.tier == "economic" and answer.similarity is not None
    assert math.isclose(answer.similarity, 12 / 13), "one insertion in thirteen"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, IsinRegistry, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'US0378331005', miccode: 'XNAS', fisn: 'APPLE INC./SH' })
    const order = new graph.OrderEvent(1n, { securityids: [new Identifier('fisn', 'APPLE INC/SH')], currency: 'USD' })
    const answer = registry.resolve(order)
    assert.equal(answer.tier, 'economic')
    assert.ok(Math.abs(answer.similarity - 12 / 13) < 1e-12, 'one insertion in thirteen')
    ```

## Edges

- `expected a '/' between the issuer and the instrument description`, `expected an issuer name before the '/'`, `expected an instrument description after the '/'`, `expected printable characters` - the four shape refusals, each naming `fisn` and the spelling it saw.
- A thirty-sixth byte -> `at most 35 bytes`, and a byte past `0x7F` -> `a non-ASCII byte` at its position: the refusals any code of that width gives, before the shape is read.
- A Latin-1 letter ISO 18774 allows (`SOCIÉTÉ`) -> refused: a code stores US-ASCII, so a short name that needs one is held as [`utf8`](../text/string.md) text, not as a `fisn`.
- Spacing is the caller's: a doubled blank or a blank before the `/` is kept, and two spellings that differ only there are two values.
- Lower case folds when a scalar is constructed; an Arrow cast is held to the canonical spelling, exactly as [ISIN](isin.md) and [FIGI](figi.md) are: a lower-case spelling is null under the default `safe`.
- Serde reads a short name through the same door: a document holding a spelling that is not the shape is refused rather than deserialized, and lower case is folded.
- No default value: the empty text names no instrument, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)).
- No packed integer: thirty-five bytes are past the sixteen `ascii_packed` fills, so neither `ascii_packed` nor a `StringEnum` takes a `fisn` column.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- fisn::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "reference_data_code or keep_what_their_standards_leave_open"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="reference-data codes" node/tests/datatype.test.js
    ```
