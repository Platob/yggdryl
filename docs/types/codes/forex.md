# Forex

An ISO 4217 currency pair - the base currency, a solidus and the quote currency, `EUR/USD` - as one registered code: the code of a foreign exchange instrument, held to the seven bytes of its one stored spelling.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `forex`, `ForexType`/`ForexField`, the `Forex` value and `Scalar::Forex`; the Rust-only symbol reader `FxSymbol` and its `FxTenor` |
| Validates | Two ISO 4217 currencies the crate lists (`StringEnum::CURRENCIES`), distinct, neither `XXX` (no currency) nor `XTS` (the testing code), spelled `CCY/CCY`, `CCYCCY` or with `-`, `.` or `_` between the legs, in any case, with ASCII whitespace or trailing NUL padding around it; the value is the one canonical spelling `CCY/CCY` |
| Lazy | Nothing - two legs on the stack and two binary searches |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | Anything else, naming `forex`, the rule and the text: an inner space, a tenor, one currency twice, a leg no listing names; the empty text, so there is no default value |

The base is the currency one unit of which is priced, the quote the currency it is priced in: `EUR/USD` is how many dollars one euro buys. The precious metals `XAU`, `XAG`, `XPT` and `XPD` are currencies to ISO 4217 and legs to this code. Anything past the pair - a tenor, a yellow key, a RIC's `=` - makes a symbol rather than a code, and [`FxSymbol::from_symbol`](#a-symbol-names-a-pair-and-a-tenor) reads those.

## DataType

`forex` is the one spelling, `DataType::forex()` the constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::forex(), DataType::Forex);
    assert_eq!(DataType::from_str("forex")?, DataType::Forex);
    assert_eq!(DataType::Forex.to_string(), "forex");
    assert_eq!(DataType::Forex.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Forex.code_name(), Some("forex"));
    assert_eq!(DataType::Forex.code_width(), Some(7));
    assert_eq!(DataType::Forex.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    forex = DataType("forex")
    assert (forex.id, forex.kind, forex.code_width) == ("forex", "code", 7)
    assert forex.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const forex = new DataType('forex')
    assert.equal(forex.id, 'forex')
    assert.equal(forex.kind, 'code')
    assert.equal(forex.codeWidth, 7)
    assert.equal(forex.fixedByteWidth, null)
    ```

## Field

`ForexField` is the typed marker; Python and JavaScript name the factory `forex`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, ForexField};

    let pair = ForexField::unit("pair", true);
    assert_eq!(pair.dtype(), &DataType::Forex);
    assert_eq!(pair.to_field(), Field::new("pair", DataType::Forex, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    pair = yggdryl.forex("pair")
    assert isinstance(pair, Field)
    assert pair.dtype == DataType("forex")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const pair = fields.forex('pair')
    assert.equal(pair.dtype.toString(), 'forex')
    ```

## Scalar

The value is the canonical pair, however a feed spelled it, so a column holds one spelling and a value compares by it. `Forex::base`, `quote` and `is_metal` read the legs, and `Forex::is_canonical` answers the rule without building a value; the four are Rust only.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Forex, Scalar};

    let pair = DataType::forex().scalar("eurusd")?;
    assert_eq!(pair, Scalar::Forex(Forex::new("EUR/USD")?));
    assert_eq!(pair.as_str(), Some("EUR/USD"));
    assert_eq!(pair.kind(), "forex");
    // Every spelling a feed writes lands as the one stored pair.
    for spelling in ["EUR/USD", "EUR-USD", "eur.usd", "EUR_USD", " EURUSD "] {
        assert_eq!(DataType::forex().scalar(spelling)?, pair, "{spelling}");
    }

    // The legs, and a metal as one of them.
    let cable = Forex::new("GBP/JPY")?;
    assert_eq!((cable.base().as_str(), cable.quote().as_str()), ("GBP", "JPY"));
    assert!(Forex::new("XAU/USD")?.is_metal());
    assert!(Forex::is_canonical("EUR/USD") && !Forex::is_canonical("EURUSD"));

    // One currency twice, and a tenor, name no pair; the refusal names the rule.
    let refused = Forex::new("EUR/EUR").unwrap_err().to_string();
    assert!(refused.contains("CCY/CCY"), "{refused}");
    assert!(Forex::new("EUR/USD 1M").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    forex = DataType("forex")
    pair = forex.scalar("eurusd")
    assert pair.as_py() == "EUR/USD"
    assert pair.kind == "forex"
    assert forex.scalar("EUR-USD") == pair

    with pytest.raises(ValueError, match="CCY/CCY"):
        forex.scalar("EUR/EUR")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const forex = new DataType('forex')
    const pair = forex.scalar('eurusd')
    assert.equal(pair.asJs(), 'EUR/USD')
    assert.equal(pair.kind, 'forex')
    assert.throws(() => forex.scalar('EUR/EUR'), /CCY\/CCY/)
    ```

## Arrow storage

`Utf8` under `yggdryl.forex` ([Codes](index.md#arrow-storage)). A cast into the column lands every accepted spelling as the canonical pair; under the default `safe` a cell naming no pair is null and the rest of the column stands, and a column already holding the canonical spelling is shared rather than copied.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let pair = Field::new("pair", DataType::Forex, false);
    let arrow = pair.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.forex");
    assert_eq!(Field::from_arrow_field(&arrow)?, pair);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    pair = Field("pair", "forex")
    arrow_field = pair.into_arrow()
    assert arrow_field.type.storage_type == pa.string()
    assert arrow_field.type.extension_name == "yggdryl.forex"
    assert Field.from_arrow(arrow_field) == pair

    # Safe: every spelling lands as the pair, and a cell naming none is null.
    source = pa.array(["eurusd", "XAU_USD", "EUR/EUR"])
    stored = Serie.from_arrow_array(source, pair).into_arrow_array()
    assert stored.to_pylist() == ["EUR/USD", "XAU/USD", None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['EURUSD', 'eur-usd', 'EUR/EUR']), fields.forex('pair'))
    assert.deepEqual([...stored.intoArrowArray()], ['EUR/USD', 'EUR/USD', null])
    ```

## A symbol names a pair and a tenor

`FxSymbol::from_symbol` is the one detector of a pair spelled with something around it - `Symbol(55)` and every other place a venue writes one - and answers the pair, the `FxTenor` the symbol states and the FIX `SettlType(63)` that tenor spells, or `None` for anything that is not one pair. The pair region runs from the trimmed text's start to the first space or `=` and reads as `Forex::new` reads; after it, one suffix in any case:

| After the pair | `tenor` | `settltype` |
| --- | --- | --- |
| nothing | `Unstated` | none |
| `=` or `=R`, a spot RIC | `Spot` | none |
| `SPOT` or `SP` | `Spot` | `0` |
| `TOD` | `Spot` | `1` |
| `TOM` | `Spot` | `2` |
| `SN` | `Forward` | `C` |
| `SW` | `Forward` | `W1` |
| `BROKEN` | `Forward` | `B` |
| one to three digits then `D`, `W`, `M` or `Y` - `1M`, `12M`, `2Y` | `Forward` | the unit then the count: `M1`, `M12`, `Y2` |
| `CURNCY` (Bloomberg's yellow key), `ON`, `TN` | `Unstated` | none |
| anything else, or anything but `R` after `=` | no pair | |

A single currency's RIC (`EUR=`), a tenor glued to one leg (`EUR1M=`) and a pair of one currency name no pair. A [FIX capture](../../fix/capture.md#what-a-message-implied-is-filled-in) reads `Symbol(55)` through it where a message states no other class, and fills what the pair implies. `FxSymbol` and `FxTenor` are Rust only.

```rust
use yggdryl::{FxSymbol, FxTenor};

let forward = FxSymbol::from_symbol("EUR-USD 1M").expect("a pair");
assert_eq!(forward.forex.as_str(), "EUR/USD");
assert_eq!(forward.tenor, FxTenor::Forward);
assert_eq!(forward.settltype.as_deref(), Some("M1"));

let ric = FxSymbol::from_symbol("EURGBP=R").expect("a pair");
assert_eq!(ric.forex.as_str(), "EUR/GBP");
assert_eq!((ric.tenor, ric.settltype), (FxTenor::Spot, None));

let spot = FxSymbol::from_symbol("GBPUSD TOM").expect("a pair");
assert_eq!(spot.settltype.as_deref(), Some("2"));
assert_eq!(FxSymbol::from_symbol("EUR/USD Curncy").map(|held| held.tenor), Some(FxTenor::Unstated));

// A single currency's RIC, a suffix nothing names and a stranger name no pair.
for symbol in ["EUR=", "EUR1M=", "EUR/USD XYZ", "EURUSD 1Q", "AAPL"] {
    assert!(FxSymbol::from_symbol(symbol).is_none(), "{symbol}");
}
```

## The `forex` security identifier { #the-forex-security-identifier }

FIX gives a currency pair no `SecurityIDSource(22)` code, so the identifier type is the crate's own: `forex`, read by `forexcode`, `ccypair` and `currencypair` too, its code validated by `Forex::new` and stored as the canonical pair. It is one more [security identifier](../../graph/market.md#security-identifiers) of a market element, an [`Identifier`](../../graph/identifier.md) of its `securityids`. In a FIX capture the `forexcode` crate column is a view of that type: a pair a row states is stated, and one detected off `Symbol(55)` is derived, from `derived`, so a stated identifier replaces it.

=== "Rust"

    ```rust
    use yggdryl::{IdSource, IdType, Identifier};

    let key = "ccypair".parse::<IdType>()?;
    assert_eq!(key, IdType::Forex);
    assert_eq!(key.as_str(), "forex");
    assert_eq!(key.fix_security_source(), None);

    let pair = Identifier::new(IdSource::Base, IdType::Forex, "eurusd")?;
    assert_eq!(pair.value(), "EUR/USD");
    assert_eq!(pair.to_string(), "base:forex=EUR/USD");
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="FX-1",
        securityids=[Identifier("base", "forex", "eurusd")],
    )
    assert str(order.securityids) == "[base:forex=EUR/USD]"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'FX-1',
      securityids: [new Identifier('base', 'forex', 'eurusd')],
    })
    assert.equal(order.securityids.toString(), '[base:forex=EUR/USD]')
    ```

## The empty text names no pair

A pair has no neutral member, so the empty text is refused at the value door rather than stored - and the datatype's canonical default refuses with it, which is what makes an empty cell entering the column null rather than a pair nobody quotes.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Forex};

    assert!(Forex::new("").is_err());
    assert!(DataType::Forex.default_value().is_err());
    // An empty cell is therefore null, as it is for a UUID.
    assert!(DataType::Forex.scalar("")?.is_null());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    with pytest.raises(ValueError, match="CCY/CCY"):
        DataType("forex").default_scalar()
    # An empty cell is therefore null, as it is for a UUID.
    assert DataType("forex").scalar("").is_null()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.throws(() => new DataType('forex').defaultJSValue(), /CCY\/CCY/)
    assert.equal(new DataType('forex').scalar('').kind, 'null')
    ```

## Edges

- `expected a pair of two distinct ISO 4217 currencies, CCY/CCY, got "EUR/EUR"` - the one refusal of the code's own rule, naming `forex` and the text as trimmed.
- Only trailing NUL padding and the ASCII whitespace around the pair are trimmed; an inner space is a symbol's, and refused.
- No default value: the empty text names no pair, so an empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and the column's nullability is what may refuse it.
- Serde reads a pair through the same door: `eurusd` in a document lands as `EUR/USD`, and one currency twice is refused rather than deserialized.
- In an expression a constant coerces into the operand it meets, so any spelling of the pair reads as the pair: `pair = 'eurusd'`, `pair in ('EUR-USD', 'xau_usd')` and `pair = forex 'GBPUSD'` compare pairs.
- The value stream and a digest feed the canonical text under the code's own identifier: `eurusd` and `EUR/USD` hash alike, and a `forex` and a [`bbg`](bbg.md) of the same text are two values.
- `ascii_packed` pads a pair into seven bytes, exactly as `fixed_ascii(7)` does.
- Nothing partial about a pair, so [`merge_with`](index.md#the-code-family-value) keeps this one.
- A digital-asset ticker is a [`ccy`](ccy.md#digital-asset-tickers) but no leg: both legs are ISO 4217 codes the crate lists, so `BTC/USDT` is no `forex`.
- No vocabulary: no listing ships under `forex`, and no Python code class declares one.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- forex:: securityid::forex
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "typed_field_factory"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code|typed field factories" node/tests/datatype.test.js node/tests/fields.test.js
    ```
