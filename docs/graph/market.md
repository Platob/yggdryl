# Market

`Market` states nineteen facts about the instrument, the side, the price and quantity, and what has traded.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Market` (`graph::market`), no supertrait; Rust-only - [leaves](index.md#leaves) answer it in Python/JavaScript |
| Price, quantity, numbers | `get_price`/`get_quantity` + `set_`: what the element states, exact as [`Decimal`](../types/numeric/decimal.md#decimal), `None` if none - never last-executed, never a default; likewise `lastpx`/`lastqty` (last executed price/quantity), `avgpx`, `cumqty`, `leavesqty`, `prevpx`/`prevqty` (prior step's settlement), `spotrate`/`forwardpoints` (FX parts) |
| Currency, unit, side | `get_currency`/`set_currency`: [`Ccy::none()`](../types/codes/ccy.md) if unstated; `get_unit`/`set_unit`: [`Unit::none()`](../types/codes/unit.md) if unstated; `get_side`/`set_side`: the [side](../types/codes/side.md) by value, one byte, `Side::Unknown` if none |
| Instrument | the [security identifiers](#security-identifiers); `get_cficode`/`get_miccode` + setters; `get_ticker`/`set_ticker`: an informal name, apart from the codes |
| Metadata | `get_metadata`/`set_metadata`: a `BTreeMap<SmolStr, SmolStr>` of source facts no typed column reads, keyed by name/[path](../types/paths.md); never identifier/typed; `None`/empty alike; a FIX leaf's = [`FixMsg::market_operations`](../fix/message.md#market-operations)'s |
| `fill_market` | provided, idempotent, called by `finalize` pre-digest: never invents price/quantity/`cumqty`/`leavesqty`; [derives](#security-identifiers) the national id a canonical ISIN embeds |
| `digest_market` | provided (`Self: Element`): extends [`Element::digest`](element.md#contract) - price, currency, quantity, unit, side, security ids (key/code), classification, market, last-trade/avg/progress/FX (if stated), ticker, metadata (key order); excludes `prevpx`/`prevqty`, like the predecessor's instant/identity |
| Provided on events | where `Self: Event`: `digest_market_event`, `following_market`, `merging_market_event` ([below](#following-and-merging)) |

## Security identifiers

| Verb | Rule |
| --- | --- |
| `get_securityids` | one validated code per source key, sorted; `get("ISIN")` (also `"4"` or `"isin"`) answers `Option<&str>` |
| `set_securityids` | replaces the whole set (all then stated); `SecurityIds::default()` unsays them |
| `insert_securityid` | fills an absent key or replaces a derived one (then stated) - never a stated one; `true` if it did |
| `derive_securityid` | fills only an absent key from implication, not statement; never reaches the holder's backing store |
| `remove_securityid` | removes one key; removing the ISIN also revokes everything derived under it |
| Derived | an ISIN's embedded national number - CUSIP (`US`/`CA`), SEDOL (`GB`/`IE`/`GG`/`JE`/`IM`, behind `00`), WKN (`DE`, behind `000`), Valor (`CH`/`LI`) ([`securityid::embedded`](../types/codes/isin.md)) - plus lifecycle-learned entries; all hang on the ISIN, so removing/replacing it revokes them |
| Provenance | derivation provenance isn't content: not carried in a row, and doesn't affect equality |
| Refusals | `SecurityId::new` validates by key; `SecType::read` refuses `TICKER` (use `set_ticker`); a [FIX message](../fix/message.md) refuses keys no field of its dictionary states |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_market` | [`Event::following`](event.md#following); `prevpx`/`prevqty`, currency, unit, side (also if stated as none), ticker, each lacked security identifier (none if ISINs differ - another instrument), classification, market - all from the predecessor where this event says nothing; always leads, even with the timed link unchanged |
| Restating | a market event's [`restating`](event.md#restating) also takes the market's place: `prevpx`/`prevqty`, and what the chain is about where this reading said nothing |
| `merging_market` | where `Self: Element`, no clocks: `self` leads |
| `merging_market_event` | [`Event::merging`](event.md#merging)'s reference leads: price/quantity/unit stand; optional numbers/ticker/metadata = reference's if stated else other's (metadata unioned, reference-led); currency/side/classification/market = the better; security ids: reference's replace by key, other's fill gaps - unless ISINs differ, then the reference's stand whole (two instruments never mixed) |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Examples

### The facts of an order

=== "Rust"

    ```rust
    use smol_str::SmolStr;
    use yggdryl::graph::{Element, Market, Metadata, OrderEvent};
    use yggdryl::{Ccy, Cfi, Decimal, Mic, SecType, SecurityId, Side};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.set_price(Some("189.50".parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.set_currency(Ccy::new("USD")?);
    order.set_ticker(Some("AAPL".into()));
    order.set_miccode(Some(Mic::new("XNAS")?));
    order.set_cficode(Some(Cfi::new("ESVUFR")?));
    order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
    order.set_metadata(Some(Metadata::from([(SmolStr::new("ordtype"), SmolStr::new("2"))])));
    order.finalize();

    assert_eq!(order.get_price(), Some("189.5".parse()?));
    assert_eq!(order.get_currency().as_str(), "USD");
    assert_eq!(order.get_side(), Side::Buy);
    assert!(order.get_unit().is_none(), "unstated");
    // A price is what the element states, never a last executed price.
    assert_eq!(order.get_lastpx(), None);
    // The ISIN is stated under any spelling of its key, and carries a CUSIP.
    assert_eq!(order.get_securityids().get("4"), Some("US0378331005"));
    assert_eq!(order.get_securityids().get("cusip"), Some("037833100"));
    assert_eq!(order.get_metadata().get("ordtype").map(SmolStr::as_str), Some("2"));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        side="BUY",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        ticker="AAPL",
        miccode="XNAS",
        cficode="ESVUFR",
        securityids={"ISIN": "US0378331005"},
        metadata={"ordtype": "2"},
    )

    assert order.price is not None and order.price.as_py() == Decimal("189.50")
    assert order.currency.as_py() == "USD"
    assert order.side.as_py() == "BUY"
    assert order.unit == "", "unstated"
    # A price is what the element states, never a last executed price.
    assert order.lastpx is None
    # The ISIN carries a CUSIP.
    assert order.securityids == {"CUSIP": "037833100", "ISIN": "US0378331005"}
    assert order.miccode is not None and order.miccode.as_py() == "XNAS"
    assert order.metadata == {"ordtype": "2"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      side: 'BUY',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      ticker: 'AAPL',
      miccode: 'XNAS',
      cficode: 'ESVUFR',
      securityids: { ISIN: 'US0378331005' },
      metadata: { ordtype: '2' },
    })

    assert.equal(order.price, '189.5')
    assert.equal(order.currency, 'USD')
    assert.equal(order.side, 'BUY')
    assert.equal(order.unit, '', 'unstated')
    // A price is what the element states, never a last executed price.
    assert.equal(order.lastpx, null)
    // The ISIN carries a CUSIP.
    assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
    assert.equal(order.miccode, 'XNAS')
    assert.deepEqual(order.metadata, { ordtype: '2' })
    ```

### The identifier verbs

Rust-only: a binding states identifiers when it builds a leaf.

```rust
use yggdryl::graph::{Element, Market, OrderEvent};
use yggdryl::{SecType, SecurityId};

let id = |key: &str, code: &str| -> yggdryl::Result<SecurityId> {
    SecurityId::new(SecType::read(key)?, code)
};
let keys = |order: &OrderEvent| -> Vec<String> {
    order.get_securityids().iter().map(ToString::to_string).collect()
};
let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.insert_securityid(id("ISIN", "US0378331005")?)?;
order.finalize();
assert_eq!(keys(&order), ["CUSIP:037833100", "ISIN:US0378331005"]);

// A derivation fills an absent key only, and a stated identifier stands.
assert!(!order.derive_securityid(id("CUSIP", "594918104")?));
assert!(!order.insert_securityid(id("ISIN", "US5949181045")?)?);

// Everything derived hangs on the ISIN: removing it takes the CUSIP back.
let mut unlisted = order.clone();
assert!(unlisted.remove_securityid(&SecType::read("ISIN")?)?);
assert!(unlisted.get_securityids().is_empty());

// A stated identifier replaces a derived one, and then outlives the ISIN.
assert!(order.insert_securityid(id("CUSIP", "037833100")?)?);
assert!(order.remove_securityid(&SecType::read("ISIN")?)?);
assert_eq!(keys(&order), ["CUSIP:037833100"]);
```

### Identifiers along a chain

An amendment to an Apple order, and one naming Microsoft's ISIN instead.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, OrderEvent};
    use yggdryl::{SecType, SecurityId, SecurityIds};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, stated: &[(&str, &str)]| -> yggdryl::Result<OrderEvent> {
        let mut ids = SecurityIds::default();
        for (key, code) in stated {
            ids.insert(SecurityId::new(SecType::read(key)?, code)?);
        }
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        order.set_securityids(ids)?;
        order.finalize();
        Ok(order)
    };
    let keys = |order: &OrderEvent| -> Vec<String> {
        order.get_securityids().iter().map(ToString::to_string).collect()
    };
    let placed = order(T, &[("ISIN", "US0378331005"), ("FIGI", "BBG000B9XRY4")])?;
    assert_eq!(keys(&placed), ["CUSIP:037833100", "FIGI:BBG000B9XRY4", "ISIN:US0378331005"]);

    let amended = order(T + 1, &[])?.with_previous(&placed).expect("a later event follows");
    assert_eq!(keys(&amended), keys(&placed));
    let other = order(T + 1, &[("ISIN", "US5949181045")])?.with_previous(&placed).expect("it follows");
    assert_eq!(keys(&other), ["CUSIP:594918104", "ISIN:US5949181045"]);

    // Two statements naming different ISINs never mix: the leading
    // statement - here the later recording - keeps its identifiers whole.
    let mut restated = placed.clone();
    restated.set_recdunix(Some(T + 5));
    restated.set_securityids(other.get_securityids().clone())?;
    let merged = placed.clone().merge_with(&restated).expect("the later recording leads");
    assert_eq!(keys(&merged), ["CUSIP:594918104", "ISIN:US5949181045"]);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, securityids: dict[str, str]) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode="O-1001", securityids=securityids)

    placed = order(T, {"FIGI": "BBG000B9XRY4", "ISIN": "US0378331005"})
    assert placed.securityids == {"CUSIP": "037833100", "FIGI": "BBG000B9XRY4", "ISIN": "US0378331005"}

    amended = order(T + 1, {}).with_previous(placed)
    assert amended is not None and amended.securityids == placed.securityids
    other = order(T + 1, {"ISIN": "US5949181045"}).with_previous(placed)
    assert other is not None and other.securityids == {"CUSIP": "594918104", "ISIN": "US5949181045"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, securityids) => new graph.OrderEvent(unix, { crosscode: 'O-1001', securityids })

    const placed = order(T, { FIGI: 'BBG000B9XRY4', ISIN: 'US0378331005' })
    assert.deepEqual(placed.securityids, { CUSIP: '037833100', FIGI: 'BBG000B9XRY4', ISIN: 'US0378331005' })

    const amended = order(T + 1n, {}).withPrevious(placed)
    assert.deepEqual(amended.securityids, placed.securityids)
    const other = order(T + 1n, { ISIN: 'US5949181045' }).withPrevious(placed)
    assert.deepEqual(other.securityids, { CUSIP: '594918104', ISIN: 'US5949181045' })
    ```
