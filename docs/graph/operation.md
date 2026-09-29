# Operation

`Operation: Market` adds four facts: how long the operation stands, whether it can trade, the names it goes by and the accounts it names.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Operation`, in `graph::market`; Rust-only - operation [leaves](index.md#leaves) answer it in Python/JavaScript |
| `tif`, `tradable` | `get_`/`set_` each: `tif` how long it stands ([`TimeInForce::from_spelling("day")`](../types/codes/timeinforce.md) stores code `0`); `tradable` (`Option<bool>`) whether the instrument can trade where a status says, `None` if the market said nothing either way |
| `altids` | the [alternate identifiers](#alternate-identifiers) |
| `accountids` | the [account identifiers](#account-identifiers) |
| Category | an operation states none of its own: its leaf's [`marketdatakind`](market-data.md#marketdata) - `ORDR`, `QUOT`, `EXEC` - is its category |
| `is_followed_altid(key)` | provided: whether an operation that follows another carries the identifier under `key` where it states none - every key but `MDENTRYREFID`, unless the holder's own dictionary says otherwise (a FIX message reads its registry's [`FIX:idmap` follow flags](../fix/registry.md#a-field-names-a-message-by-its-identifiers)) |
| `digest_operation` | provided (`Self: Element`): continues [`digest_market`](market.md#contract) with the time in force, whether it trades, and the alternate and account identifiers (key order) |
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
| In a walk | a name a live element goes by is how an element arriving under no live identity finds its chain, on its own side and within its own market data kind ([walk](event.md#lifecycle-walk)); a book resolves `MDENTRYID`/`MDENTRYREFID` the [same way](book.md#entries) |

## Account identifiers

`accountids` is an [`IdMap`](../fix/message.md#the-identifier-maps) too: the accounts and parties the operation names, one identifier per party role - `CUSTOMERACCOUNT`, `EXECUTINGTRADER`, `CLIENTID` - under upper-cased ASCII keys, key-ordered, each value trimmed.

| Verb | Rule |
| --- | --- |
| `set_accountids(IdMap)` | replaces the map whole; `IdMap::new()` unsays it |
| `insert_accountid(key, value)` | fills only an absent role (folded to upper case); `false` for a held one, and a null-like value adds nothing |
| `remove_accountid(key)` | removes one role; returns whether one was held |
| A view | a [FIX message](../fix/message.md#accounts-and-regulatory-trade-identifiers)'s accounts are its parties, each `PartyID` under its role's name, and its `Account(1)`, under `ACCOUNT`: they are read-only there - a change is refused by name, a no-op answers `Ok` - and a plain holder always answers `Ok` |
| Column | `accountids`: a sorted `map<utf8, utf8>`, nullable - null or empty states none ([Market data](market-data.md#columns)) |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_operation` | [`following_market`](market.md#following-and-merging), then time in force/tradability if unstated, the accounts, and every alternate identifier of the chain it lacks but `MDENTRYREFID` - its own values stand; a holder's own dictionary may flag fewer |
| The side | this operation's where it states one, the chain's where it states `UNKN` - in following and in restating - and with it the chain's [cross code](market.md#sides-and-cross-codes), under that side for an order, a quote or an execution |
| Accounts | a role this statement names none for is the chain's, every role - the accounts an operation is booked to stay with its chain; a FIX message's accounts are its fields' (its parties and `Account(1)`), so it takes none |
| Restating | a market operation event's [`restating`](event.md#restating) also takes the time in force, tradability, followed alternate identifiers and the accounts, as following does |
| `merging_operation_event` | [`merging_market_event`](market.md#following-and-merging), then the time in force (the better), tradability (the reference's if stated), the alternate identifiers and the accounts (each a union, reference-led) |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Example

A replacement order following the one it replaces, then an acknowledgment that states no identifier.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, Operation, OrderEvent};
    use yggdryl::{Side, TimeInForce};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.set_side(Side::Buy);
    placed.insert_altid("clordid", "C-1")?;
    placed.insert_altid("ORDERID", "O-1001")?;
    placed.insert_altid("MDENTRYREFID", "R-1")?;
    placed.insert_accountid("clientid", "ACC-1")?;
    placed.set_tif(TimeInForce::from_spelling("day"));
    placed.set_tradable(Some(true));
    placed.finalize();
    // Keys fold to upper case, and an insert fills an absent key only.
    assert_eq!(placed.get_altids().get("CLORDID"), Some("C-1"));
    assert_eq!(placed.get_accountids().get("CLIENTID"), Some("ACC-1"));
    assert!(!placed.insert_altid("orderid", "O-9999")?);
    assert!(!placed.insert_altid("SECONDARYORDERID", "n/a")?, "a null-like value adds nothing");
    assert_eq!(placed.get_tif().map(TimeInForce::as_str), Some("0"));

    // The replacement states its own client and execution ids.
    let mut replacing = OrderEvent::at(T + 1_000_000_000);
    replacing.set_crosscode("O-1001".to_owned());
    replacing.insert_altid("CLORDID", "C-2")?;
    replacing.insert_altid("EXECID", "X-2")?;
    replacing.finalize();
    let replaced = replacing.with_previous(&placed).expect("a later event follows");
    assert_eq!(replaced.get_tif(), placed.get_tif());
    assert_eq!(replaced.get_tradable(), Some(true));
    // Its own words stand and the chain's order id is taken - never the entry reference.
    assert_eq!(replaced.get_altids().get("CLORDID"), Some("C-2"));
    assert_eq!(replaced.get_altids().get("EXECID"), Some("X-2"));
    assert_eq!(replaced.get_altids().get("ORDERID"), Some("O-1001"));
    assert_eq!(replaced.get_altids().get("MDENTRYREFID"), None);
    assert_eq!(replaced.get_accountids().get("CLIENTID"), Some("ACC-1"), "the chain's account");
    // Stating no side, it stands on the chain's, under the chain's code.
    assert_eq!((replaced.get_side(), replaced.get_crosscode()), (Side::Buy, "BUYS:O-1001"));

    // A statement naming no identifier takes the chain's client id as well.
    let mut ack = OrderEvent::at(T + 2_000_000_000);
    ack.set_crosscode("O-1001".to_owned());
    ack.finalize();
    let ack = ack.with_previous(&replaced).expect("a later event follows");
    assert_eq!(ack.get_altids(), replaced.get_altids());
    ```

=== "Python"

    ```python
    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000
    placed = graph.OrderEvent(
        T,
        crosscode="O-1001",
        side="BUYS",
        altids={"CLORDID": "C-1", "MDENTRYREFID": "R-1", "orderid": "O-1001"},
        accountids={"clientid": "ACC-1"},
        tif="0",
        tradable=True,
    )
    # Keys fold to upper case.
    assert placed.altids == {"CLORDID": "C-1", "MDENTRYREFID": "R-1", "ORDERID": "O-1001"}
    assert placed.accountids == {"CLIENTID": "ACC-1"}

    # The replacement states its own client and execution ids.
    replaced = graph.OrderEvent(
        T + 1_000_000_000, crosscode="O-1001", altids={"CLORDID": "C-2", "EXECID": "X-2"}
    ).with_previous(placed)
    assert replaced is not None
    assert (replaced.tif, replaced.tradable) == ("0", True)
    # Its own words stand and the chain's order id is taken - never the entry reference.
    assert replaced.altids == {"CLORDID": "C-2", "EXECID": "X-2", "ORDERID": "O-1001"}
    assert replaced.accountids == {"CLIENTID": "ACC-1"}, "the chain's account"
    # Stating no side, it stands on the chain's, under the chain's code.
    assert (replaced.side, replaced.crosscode) == (Side.BUYS, "BUYS:O-1001")

    # A statement naming no identifier takes the chain's client id as well.
    ack = graph.OrderEvent(T + 2_000_000_000, crosscode="O-1001").with_previous(replaced)
    assert ack is not None and ack.altids == replaced.altids
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const placed = new graph.OrderEvent(T, {
      crosscode: 'O-1001',
      side: 'BUYS',
      altids: { CLORDID: 'C-1', MDENTRYREFID: 'R-1', orderid: 'O-1001' },
      accountids: { clientid: 'ACC-1' },
      tif: '0',
      tradable: true,
    })
    // Keys fold to upper case.
    assert.deepEqual(placed.altids, { CLORDID: 'C-1', MDENTRYREFID: 'R-1', ORDERID: 'O-1001' })
    assert.deepEqual(placed.accountids, { CLIENTID: 'ACC-1' })

    // The replacement states its own client and execution ids.
    const replaced = new graph.OrderEvent(T + 1_000_000_000n, {
      crosscode: 'O-1001',
      altids: { CLORDID: 'C-2', EXECID: 'X-2' },
    }).withPrevious(placed)
    assert.equal(replaced.tif, '0')
    assert.equal(replaced.tradable, true)
    // Its own words stand and the chain's order id is taken - never the entry reference.
    assert.deepEqual(replaced.altids, { CLORDID: 'C-2', EXECID: 'X-2', ORDERID: 'O-1001' })
    assert.deepEqual(replaced.accountids, { CLIENTID: 'ACC-1' })
    // Stating no side, it stands on the chain's, under the chain's code.
    assert.equal(replaced.side, 'BUYS')
    assert.equal(replaced.crosscode, 'BUYS:O-1001')

    // A statement naming no identifier takes the chain's client id as well.
    const ack = new graph.OrderEvent(T + 2_000_000_000n, { crosscode: 'O-1001' }).withPrevious(replaced)
    assert.deepEqual(ack.altids, replaced.altids)
    ```

## Edges

- `tradable` is what a status said: `None` states nothing, and a [book level](book.md#limits) reads an entry stating nothing as one that trades.
- A key or value that is empty, not ASCII, or too wide - a key past 32 bytes once upper-cased, a value past 64 once trimmed - is refused by `IdMap`, located on the key: `insert_altid` and `insert_accountid` answer the error, and a binding refuses the fact by name.
- `MDENTRYREFID` is the one identifier a follower never takes: a book entry's [reference to its predecessor](book.md#entries) names one step, not the chain. A FIX message follows an identifier through the field that states it, so only where its dictionary flags one.
