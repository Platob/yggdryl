# Operation

`Operation: Market` adds eight facts: category, duration, tradability, the names it goes by, and its two lanes.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Operation` (`Lane`, `FOLLOWED_ALTIDS`), in `graph::market`; Rust-only - operation [leaves](index.md#leaves) answer it in Python/JavaScript |
| `marketoperationid`, `tif`, `tradable` | `get_`/`set_` each: `marketoperationid` (`Option<i32>`) the stable numeric category; `tif` how long it stands ([`TimeInForce::from_spelling("day")`](../types/codes/timeinforce.md) stores code `0`); `tradable` (`Option<bool>`) `None` if the market said nothing either way |
| Maps, Lanes | `accountids`/`userids`/`altids`: the [identifier maps](#identifier-maps); `bid`/`ask`: the [lanes](#lanes) |
| `fill_operation` | provided, run by `finalize` after [`fill_market`](market.md#contract): a quote with one lane and no side/price/last-trade/average of its own takes that lane's side (`BUY`=bid, `SELL`=ask; two lanes or none name nothing); price/quantity/currency/unit/FX default to its side's lane if unstated; then `fill_lanes` |
| `fill_lanes` | provided: fills the lane the side implies (`Side::is_bid`/`is_ask`) from stated facts only - absent price/quantity/currency or a `None` unit fill nothing; never overwrites an already-stated slot; nothing for a side taking neither lane |
| `digest_operation` | provided (`Self: Element`): continues [`digest_market`](market.md#contract) with the category, time in force, whether it trades, the three maps (key order), each lane |
| `merging_operation` | where `Self: Element`, no clocks: `self` leads |
| Provided on events | where `Self: Event`: `digest_operation_event`, `following_operation`, `merging_operation_event` ([below](#following-and-merging)) |

## Identifier maps

Three [`IdMap`](../fix/message.md#the-identifier-maps)s, upper-cased ASCII keys/values, key-ordered: `accountids` (who for), `userids` (who by), `altids` (names it goes by - `ORDERID`, `CLORDID`, `MDENTRYID`, ...)

| Verb | Rule |
| --- | --- |
| `set_<map>(IdMap)` | replaces the map whole; `IdMap::new()` unsays it |
| `insert_accountid`/`insert_userid`/`insert_altid` `(key, value)` | fills only an absent key (folded to upper case); `false` for a held one |
| `remove_accountid`/`remove_userid`/`remove_altid` `(key)` | removes one key; returns whether one was held |
| A view | a [FIX message](../fix/message.md#the-identifier-maps) writes through, refusing unknown keys; a plain holder always answers `Ok` |

## Lanes

`get_bid`/`set_bid`, `get_ask`/`set_ask`: what a party pays, and is paid. `Lane { price, spotrate, forwardpoints, currency: Option<Ccy>, quantity, unit: Option<Unit> }` states each slot or `None`; `is_stated` says if any is; `stated()` answers `Some` only then; stating nothing sets `None`.

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_operation` | [`following_market`](market.md#following-and-merging), then time in force/tradability if unstated, every lacked account/user key, and only the order's own alternate identifiers - `FOLLOWED_ALTIDS`: `ORDERID`, `SECONDARYORDERID`, `PARENTORDERID`, `PARENTCLORDID`, `OMSDEALERPARENTORDERID`, `EXCHANGECLIENTORDERID`, `TRANSVERSALKEY`, or what a holder's own dictionary flags instead - never an execution's/quote's, never a lane |
| The side | the chain's, only if the operation quotes no lane of its own (one lane names its side, two name none) - both in following and restating |
| Restating | a market operation event's [`restating`](event.md#restating) also takes the time in force, tradability, accounts, users and followed alternate identifiers |
| `merging_operation_event` | [`merging_market_event`](market.md#following-and-merging), then category, time in force, tradability, the three maps (union, reference-led), lanes slot by slot |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Examples

### Lanes

A buy order, and a quote naming only its ask lane.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Lane, Market, Operation, OrderEvent, QuoteEvent};
    use yggdryl::{Ccy, Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut order = OrderEvent::at(T);
    order.set_side(Side::Buy);
    order.set_price(Some("189.50".parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.finalize();
    let bid = order.get_bid().expect("a buy quotes the bid");
    assert_eq!((bid.price, bid.quantity), (Some("189.5".parse()?), Some(Decimal::from_int(100))));
    assert_eq!(order.get_ask(), None);

    let mut offer = QuoteEvent::at(T);
    offer.set_ask(Some(Lane {
        price: Some("189.52".parse()?),
        quantity: Some(Decimal::from_int(100)),
        currency: Some(Ccy::new("USD")?),
        ..Lane::default()
    }));
    offer.finalize();
    assert_eq!(offer.get_side(), Side::Sell);
    assert_eq!(offer.get_price(), Some("189.52".parse()?));
    assert_eq!(offer.get_currency().as_str(), "USD");

    // A lane stating nothing is no lane.
    offer.set_bid(Some(Lane::default()));
    assert_eq!(offer.get_bid(), None);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    order = graph.OrderEvent(T, side="BUY", price=Decimal("189.50"), quantity=100)
    assert order.bid is not None
    assert order.bid.price == order.price and order.bid.quantity == order.quantity
    assert order.ask is None

    offer = graph.QuoteEvent(T, ask=graph.Lane(price=Decimal("189.52"), quantity=100, currency="USD"))
    assert offer.side.as_py() == "SELL"
    assert offer.price is not None and offer.price.as_py() == Decimal("189.52")
    assert offer.currency.as_py() == "USD"

    # A lane stating nothing is no lane.
    assert not graph.Lane().is_stated()
    assert graph.QuoteEvent(T, bid=graph.Lane()).bid is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = new graph.OrderEvent(T, { side: 'BUY', price: '189.50', quantity: 100 })
    assert.equal(order.bid.price, '189.5')
    assert.equal(order.bid.quantity, '100')
    assert.equal(order.ask, null)

    const offer = new graph.QuoteEvent(T, { ask: new graph.Lane({ price: '189.52', quantity: 100, currency: 'USD' }) })
    assert.equal(offer.side, 'SELL')
    assert.equal(offer.price, '189.52')
    assert.equal(offer.currency, 'USD')

    // A lane stating nothing is no lane.
    assert.equal(new graph.Lane().isStated(), false)
    assert.equal(new graph.QuoteEvent(T, { bid: new graph.Lane() }).bid, null)
    ```

### The names an order goes by

A replacement order, following the one it replaces.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Operation, OrderEvent, FOLLOWED_ALTIDS};
    use yggdryl::TimeInForce;

    const T: i64 = 1_700_000_000_000_000_000;
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.insert_accountid("account", "ACC-7")?;
    placed.insert_userid("TRADER", "jdoe")?;
    placed.insert_altid("CLORDID", "C-1")?;
    placed.insert_altid("ORDERID", "O-1001")?;
    placed.set_tif(TimeInForce::from_spelling("day"));
    placed.finalize();
    // Keys fold to upper case, and an insert fills an absent key only.
    assert_eq!(placed.get_accountids().get("ACCOUNT"), Some("ACC-7"));
    assert!(!placed.insert_altid("orderid", "O-9999")?);
    assert_eq!(placed.get_tif().map(TimeInForce::as_str), Some("0"));

    let mut replacing = OrderEvent::at(T + 1_000_000_000);
    replacing.set_crosscode("O-1001".to_owned());
    replacing.insert_altid("CLORDID", "C-2")?;
    replacing.finalize();
    let replaced = replacing.with_previous(&placed).expect("a later event follows");
    assert_eq!(replaced.get_accountids().get("ACCOUNT"), Some("ACC-7"));
    assert_eq!(replaced.get_userids().get("TRADER"), Some("jdoe"));
    assert_eq!(replaced.get_tif(), placed.get_tif());
    assert_eq!(replaced.get_altids().get("ORDERID"), Some("O-1001"));
    assert_eq!(replaced.get_altids().get("CLORDID"), Some("C-2"));
    assert!(FOLLOWED_ALTIDS.contains(&"ORDERID") && !FOLLOWED_ALTIDS.contains(&"CLORDID"));
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    placed = graph.OrderEvent(
        T,
        crosscode="O-1001",
        accountids={"account": "ACC-7"},
        userids={"TRADER": "jdoe"},
        altids={"CLORDID": "C-1", "ORDERID": "O-1001"},
        tif="0",
    )
    # Keys fold to upper case.
    assert placed.accountids == {"ACCOUNT": "ACC-7"}

    replaced = graph.OrderEvent(T + 1_000_000_000, crosscode="O-1001", altids={"CLORDID": "C-2"}).with_previous(placed)
    assert replaced is not None
    assert (replaced.accountids, replaced.userids, replaced.tif) == ({"ACCOUNT": "ACC-7"}, {"TRADER": "jdoe"}, "0")
    assert replaced.altids == {"CLORDID": "C-2", "ORDERID": "O-1001"}
    assert "ORDERID" in graph.FOLLOWED_ALTIDS and "CLORDID" not in graph.FOLLOWED_ALTIDS
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const placed = new graph.OrderEvent(T, {
      crosscode: 'O-1001',
      accountids: { account: 'ACC-7' },
      userids: { TRADER: 'jdoe' },
      altids: { CLORDID: 'C-1', ORDERID: 'O-1001' },
      tif: '0',
    })
    // Keys fold to upper case.
    assert.deepEqual(placed.accountids, { ACCOUNT: 'ACC-7' })

    const replaced = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001', altids: { CLORDID: 'C-2' } })
      .withPrevious(placed)
    assert.deepEqual(replaced.accountids, { ACCOUNT: 'ACC-7' })
    assert.deepEqual(replaced.userids, { TRADER: 'jdoe' })
    assert.equal(replaced.tif, '0')
    assert.deepEqual(replaced.altids, { CLORDID: 'C-2', ORDERID: 'O-1001' })
    assert.ok(graph.FOLLOWED_ALTIDS.includes('ORDERID') && !graph.FOLLOWED_ALTIDS.includes('CLORDID'))
    ```

## Edges

- A side taking no lane (cross, `OPPOSITE`, `UNKNOWN`) fills none via `fill_lanes`; `set_bid(Some(Lane::default()))` states nothing, landing as `None`.
- A quote stating its own price, last trade or average is about that; a lane beside it is context, naming no side.
