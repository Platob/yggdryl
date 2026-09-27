# Operation

`Operation: Market` adds three facts: how long the operation stands, whether it can trade, and the names it goes by.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Operation` (and `FOLLOWED_ALTIDS`), in `graph::market`; Rust-only - operation [leaves](index.md#leaves) answer it in Python/JavaScript |
| `tif`, `tradable` | `get_`/`set_` each: `tif` how long it stands ([`TimeInForce::from_spelling("day")`](../types/codes/timeinforce.md) stores code `0`); `tradable` (`Option<bool>`) whether the instrument can trade where a status says, `None` if the market said nothing either way |
| `altids` | the [alternate identifiers](#alternate-identifiers) |
| Category | an operation states none of its own: its leaf's [`marketdatakind`](market-data.md#marketdata) - `ORDR`, `QUOT`, `EXEC` - is its category |
| `is_followed_altid(key)` | provided: whether an operation that follows another carries the identifier under `key` - [`FOLLOWED_ALTIDS`](#following-and-merging) unless the holder's own dictionary says otherwise (a FIX message reads its registry's `FIX:idmap` follow flags) |
| `digest_operation` | provided (`Self: Element`): continues [`digest_market`](market.md#contract) with the time in force, whether it trades, and the alternate identifiers (key order) |
| `merging_operation` | where `Self: Element`, no clocks: `self` leads |
| Provided on events | where `Self: Event`: `digest_operation_event`, `following_operation`, `merging_operation_event` ([below](#following-and-merging)) |

## Alternate identifiers

`altids` is an [`IdMap`](../fix/message.md#the-identifier-maps): the names the operation goes by - `ORDERID`, `CLORDID`, `EXECID`, `MDENTRYID`, ... - under upper-cased ASCII keys, key-ordered, each value trimmed.

| Verb | Rule |
| --- | --- |
| `set_altids(IdMap)` | replaces the map whole; `IdMap::new()` unsays it |
| `insert_altid(key, value)` | fills only an absent key (folded to upper case); `false` for a held one, and a null-like value (`null`, `none`, `n/a`, empty) adds nothing |
| `remove_altid(key)` | removes one key; returns whether one was held |
| A view | a [FIX message](../fix/message.md#the-identifier-maps) writes through, refusing unknown keys; a plain holder always answers `Ok` |
| In a walk | a name a live element goes by is how an element arriving under no live identity finds its chain, on its own side ([walk](event.md#lifecycle-walk)); a book resolves `MDENTRYID`/`MDENTRYREFID` the [same way](book.md#entries) |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_operation` | [`following_market`](market.md#following-and-merging), then time in force/tradability if unstated, and only the order's own alternate identifiers - `FOLLOWED_ALTIDS`: `ORDERID`, `SECONDARYORDERID`, `PARENTORDERID`, `PARENTCLORDID`, `OMSDEALERPARENTORDERID`, `EXCHANGECLIENTORDERID`, `TRANSVERSALKEY`, or what a holder's own dictionary flags instead - never an execution's/quote's |
| The side | this operation's where it states one, the chain's where it states `UNKNOWN` - in following and in restating - and with it the chain's side-prefixed [cross code](market.md#sides-and-cross-codes) |
| Restating | a market operation event's [`restating`](event.md#restating) also takes the time in force, tradability and followed alternate identifiers |
| `merging_operation_event` | [`merging_market_event`](market.md#following-and-merging), then the time in force (the better), tradability (the reference's if stated), the alternate identifiers (union, reference-led) |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Example

A replacement order, following the one it replaces.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, Operation, OrderEvent, FOLLOWED_ALTIDS};
    use yggdryl::{Side, TimeInForce};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.set_side(Side::Buy);
    placed.insert_altid("clordid", "C-1")?;
    placed.insert_altid("ORDERID", "O-1001")?;
    placed.set_tif(TimeInForce::from_spelling("day"));
    placed.set_tradable(Some(true));
    placed.finalize();
    // Keys fold to upper case, and an insert fills an absent key only.
    assert_eq!(placed.get_altids().get("CLORDID"), Some("C-1"));
    assert!(!placed.insert_altid("orderid", "O-9999")?);
    assert!(!placed.insert_altid("SECONDARYORDERID", "n/a")?, "a null-like value adds nothing");
    assert_eq!(placed.get_tif().map(TimeInForce::as_str), Some("0"));

    // The replacement states its own client id and nothing else.
    let mut replacing = OrderEvent::at(T + 1_000_000_000);
    replacing.set_crosscode("O-1001".to_owned());
    replacing.insert_altid("CLORDID", "C-2")?;
    replacing.finalize();
    let replaced = replacing.with_previous(&placed).expect("a later event follows");
    assert_eq!(replaced.get_tif(), placed.get_tif());
    assert_eq!(replaced.get_tradable(), Some(true));
    assert_eq!(replaced.get_altids().get("ORDERID"), Some("O-1001"));
    assert_eq!(replaced.get_altids().get("CLORDID"), Some("C-2"));
    // Stating no side, it stands on the chain's, under the chain's code.
    assert_eq!((replaced.get_side(), replaced.get_crosscode()), (Side::Buy, "BUY:O-1001"));
    assert!(FOLLOWED_ALTIDS.contains(&"ORDERID") && !FOLLOWED_ALTIDS.contains(&"CLORDID"));
    ```

=== "Python"

    ```python
    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000
    placed = graph.OrderEvent(
        T,
        crosscode="O-1001",
        side="BUY",
        altids={"CLORDID": "C-1", "orderid": "O-1001"},
        tif="0",
        tradable=True,
    )
    # Keys fold to upper case.
    assert placed.altids == {"CLORDID": "C-1", "ORDERID": "O-1001"}

    # The replacement states its own client id and nothing else.
    replaced = graph.OrderEvent(T + 1_000_000_000, crosscode="O-1001", altids={"CLORDID": "C-2"}).with_previous(placed)
    assert replaced is not None
    assert (replaced.tif, replaced.tradable) == ("0", True)
    assert replaced.altids == {"CLORDID": "C-2", "ORDERID": "O-1001"}
    # Stating no side, it stands on the chain's, under the chain's code.
    assert (replaced.side, replaced.crosscode) == (Side.BUY, "BUY:O-1001")
    assert "ORDERID" in graph.FOLLOWED_ALTIDS and "CLORDID" not in graph.FOLLOWED_ALTIDS
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const placed = new graph.OrderEvent(T, {
      crosscode: 'O-1001',
      side: 'BUY',
      altids: { CLORDID: 'C-1', orderid: 'O-1001' },
      tif: '0',
      tradable: true,
    })
    // Keys fold to upper case.
    assert.deepEqual(placed.altids, { CLORDID: 'C-1', ORDERID: 'O-1001' })

    // The replacement states its own client id and nothing else.
    const replaced = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001', altids: { CLORDID: 'C-2' } })
      .withPrevious(placed)
    assert.equal(replaced.tif, '0')
    assert.equal(replaced.tradable, true)
    assert.deepEqual(replaced.altids, { CLORDID: 'C-2', ORDERID: 'O-1001' })
    // Stating no side, it stands on the chain's, under the chain's code.
    assert.equal(replaced.side, 'BUY')
    assert.equal(replaced.crosscode, 'BUY:O-1001')
    assert.ok(graph.FOLLOWED_ALTIDS.includes('ORDERID') && !graph.FOLLOWED_ALTIDS.includes('CLORDID'))
    ```

## Edges

- `tradable` is what a status said: `None` states nothing, and a [book level](book.md#limits) reads an entry stating nothing as one that trades.
- A key or value that is empty, not ASCII, or too wide - a key past 32 bytes once upper-cased, a value past 64 once trimmed - is refused by `IdMap`, located on the key: `insert_altid` answers the error, and a binding refuses the fact by name.
- A following operation never carries an execution's or a quote's own identifiers (`EXECID`, `QUOTEID`): each names that statement alone.
