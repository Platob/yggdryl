# Operation

`Operation: Market` adds five facts: the quantity it ordered, how long the operation stands, whether it can trade, the identifiers it goes by and the party ids it names.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Operation`, in `graph::market`; Rust-only - operation [leaves](index.md#leaves) answer it in Python/JavaScript |
| `ordqty` | `get_ordqty`/`set_ordqty`: the quantity the operation ordered, FIX's `OrderQty(38)`, never the quantity the element is about ([`get_quantity`](market.md#contract)); it, `get_cumqty`, `get_leavesqty` and `get_cxlqty` fill one another by the state the operation reached - fresh, all of it left; working, any two of ordered, traded and left give the third; filled, nothing left and all of it traded; canceled, done for the day or expired, nothing left and the untraded rest canceled ([Market](market.md#setting-fill-or-overwrite)) |
| `timeinforce`, `tradable` | `get_`/`set_` each: `timeinforce` how long it stands, an `Option<TimeInForce>` [enum member](../types/enum/timeinforce.md) - `set_timeinforce(TimeInForce::from_spelling("day"), true)` stores `DAY`, code `1`; Python answers the `yggdryl.TimeInForce` member and JavaScript its name, and either takes a member, its code, a name, FIX's name or the wire value; `tradable` (`Option<bool>`) whether the instrument can trade where a status says, `None` if the market said nothing either way |
| `identifiers` | the operation's own [identifiers](#identifiers), an [`Identifiers`](identifier.md) map keyed `src:type` |
| `partyids` | the [party ids](#party-identifiers), an [`Identifiers`](identifier.md) map keyed `src:type` |
| Category | an operation states none of its own: its leaf's [`marketdatakind`](market-data.md#marketdata) - `ORDR`, `QUOT`, `EXEC` - is its category |
| `is_followed_identifier(id)` | provided: whether an operation that follows another carries `id`, one of the chain's identifiers, where it states none of its source and type - every type but `mdentryrefid`, unless the holder's own dictionary says otherwise (a FIX message reads its registry's [`FIX:idmap` follow flags](../fix/registry.md#a-field-names-a-message-by-its-identifiers), each followed type's [parents](identifier.md#parentage) travelling with it) |
| `parents_of(base)`, `parent_of(kind)` | provided: the parent types of one of the operation's identifier types, nearest first, and the base a parent type belongs to with its place among the base's parents - [`IdType::parents` and `parent_of`](identifier.md#parentage) unless the holder's own dictionary says otherwise (a FIX message reads its registry's [`FIX:parents`](../fix/registry.md#parents-of-an-identifier)) |
| `digest_operation` | provided (`Self: Element`): continues [`digest_market`](market.md#contract) with the quantity ordered, the time in force, whether it trades, and the identifiers and the party ids (each fed as source, type and value under the labels `identifiers` and `partyids`, in key order, the parents among them) |
| `merging_operation` | where `Self: Element`, no clocks: `self` leads |
| Provided on events | where `Self: Event`: `digest_operation_event`, `following_operation`, `merging_operation_event` ([below](#following-and-merging)) |

## Order quantity

`ordqty` is what the operation ordered and `quantity` what it is about: stating the one never states the other. Working, what is left is what was ordered less what traded, so the one fact a message leaves out is filled from the two it states, and `quantity` - where nothing states it - is what is left to work. The whole rule, state by state, is [Market](market.md#setting-fill-or-overwrite)'s.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
    use yggdryl::{Decimal, State};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_state(State::PartiallyFilled);
    order.set_ordqty(Some(Decimal::from_int(100)), true);
    order.set_cumqty(Some(Decimal::from_int(40)), true);
    order.finalize();
    assert_eq!(order.get_leavesqty(), Some(Decimal::from_int(60)));
    assert_eq!(order.get_quantity(), Some(Decimal::from_int(60)));
    assert_eq!(order.get_ordqty(), Some(Decimal::from_int(100)));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000, crosscode="O-1001", state="PARTIALLY_FILLED", ordqty="100", cumqty="40"
    )
    assert order.leavesqty.as_py() == Decimal(60)
    assert order.quantity.as_py() == Decimal(60)
    assert order.ordqty.as_py() == Decimal(100)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      state: 'PARTIALLY_FILLED',
      ordqty: '100',
      cumqty: '40',
    })
    assert.equal(order.leavesqty, '60')
    assert.equal(order.quantity, '60')
    assert.equal(order.ordqty, '100')
    ```

## Identifiers

`identifiers` is an [`Identifiers`](identifier.md) map: the names the operation goes by - `orderid`, `clordid`, `execid`, `mdentryid`, ... - one value per key `src:type`, the base key spelled as its type alone and holding the type's answer - what a FIX field states, a named source filling or replacing it where it is empty or ranks below it ([The base key](identifier.md#the-base-key)) - with the [parents](identifier.md#parentage) a chain gave it: a replacement's `clordid` names the one it replaced as its `origclordid`, and an `orderid` that changed names the value before it as `parentorderid` and its chain's first as `origorderid`.

| Verb | Rule |
| --- | --- |
| `get_identifiers` | the map, in key order; `get(&IdType::ClOrdId)` the base key's value - the type's answer ([Lookups](identifier.md#contract)) - `get_from(&IdKey::new(src, IdType::OrderId))` one source's |
| `set_identifiers(ids, overwrite)` | with `overwrite`, replaces the map whole, `Identifiers::new()` unsaying it; without, `Identifiers::merge(..., false)` replaces lower-ranked values, keeps equal-ranked held values, and retains other keys |
| `insert_identifier(id)` | `Identifiers::insert`: fills an absent key or replaces a lower-ranked value; a named source fills or replaces its type's base key where it is empty or ranks below it; `false` when a value of equal or higher rank stands - another source of one type is another identifier |
| `remove_identifier(&key)` | removes what an `IdKey` holds - a named source's key that identifier alone, the base key every key of its type; returns whether one was held |
| A FIX message | its sets are logical facts read off its fields, the wire kept as sent: a caller's write is the message's word, held as stated - no field moves and no settle restates it ([FIX](../fix/message.md#the-identifier-maps)); a plain holder always answers `Ok` |
| In a walk | a name a live element goes by - and a parent identifier's value, under the parent's own type - is how an element arriving under no live identity finds its chain, on its own side and within its own market data kind ([walk](event.md#lifecycle-walk)); a book resolves `mdentryid`/`mdentryrefid` the [same way](book.md#entries) |

## Party identifiers

`partyids` is an [`Identifiers`](identifier.md) map too: the accounts, traders, firms and users the operation names, each a role - the type - from the source that issued it, which fills the role's base key: `proprietary:executingtrader=T-1` beside `executingtrader=T-1`, `account=ACCT-7`.

| Verb | Rule |
| --- | --- |
| `get_partyids` | the map; `get(&IdType::ExecutingTrader)`, `get(&IdType::Account)` |
| `set_partyids(partyids, overwrite)` | with `overwrite`, replaces the map whole; without, `Identifiers::merge(..., false)` replaces lower-ranked values, keeps equal-ranked held values, and retains other keys |
| `insert_partyid(partyid)` | inserts through `Identifiers::insert`: fills an absent key or replaces a lower-ranked value; `false` when an equal- or higher-ranked value stands |
| `remove_partyid(&key)` | removes what an `IdKey` holds, the base key every key of its role; returns whether one was held |
| A FIX message | each `Parties(453)` or `RootParties(1116)` occurrence typed by its role's name from its source's name, `Account(1)` an `account` from its `AcctIDSource(660)`'s - the [naming rule](identifier.md#where-identifiers-come-from); the groups and `Account(1)` stay on the wire as sent, and a caller's write is the message's word |
| Column | `partyids`: a sorted `map<utf8, utf8>` from the key's text to the value, null where empty ([Market data](market-data.md#columns)); the five operation columns - `ordqty`, `timeinforce`, `tradable`, `identifiers`, `partyids` - close every generated row's shared columns ([Row schemas](schemas.md#the-marketdata-row)) |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_operation` | [`following_market`](market.md#following-and-merging), then time in force/tradability if unstated, the party ids, and every identifier of the chain it lacks but `mdentryrefid`, each carried as it is; each base identifier it states takes the [parents](identifier.md#parentage) its chain gave it - its own values stand; a holder's own dictionary may flag fewer |
| The side | an order's or an execution's: this operation's where it states one, the chain's where it states `UKNW` - in following and in restating - and with it the chain's [cross code](market.md#sides-and-cross-codes), restated under that side; a quote's side is its own tag, and it takes the [legs](market.md#a-quotes-two-legs) of its chain it states nothing of |
| Party ids | a source and role this statement names none for is the chain's, every role - the parties an operation is booked to stay with its chain |
| Restating | a market operation event's [`restating`](event.md#restating) also takes the time in force, tradability, followed identifiers and the party ids, as following does |
| `merging_operation_event` | [`merging_market_event`](market.md#following-and-merging), then the time in force (the better), tradability (the reference's if stated), the identifiers and the party ids (each a union by source and type, reference-led) |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Example

A replacement order following the one it replaces, then an acknowledgment that states no identifier. The replacement's `clordid` names the one it replaced as its `origclordid`.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, Operation, OrderEvent};
    use yggdryl::{IdKey, IdSource, IdType, Identifier, Side, TimeInForce};

    const T: i64 = 1_700_000_000_000_000_000;
    let fix = |kind: IdType, value: &str| Identifier::new(IdKey::base(kind), value);
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.set_side(Side::Buy, true);
    placed.insert_identifier(fix(IdType::ClOrdId, "C-1")?)?;
    placed.insert_identifier(fix(IdType::OrderId, "O-1001")?)?;
    placed.insert_identifier(fix(IdType::MdEntryRefId, "R-1")?)?;
    placed.insert_partyid(Identifier::new(IdKey::new(IdSource::Proprietary, IdType::ClientId), "ACC-1")?)?;
    placed.set_timeinforce(TimeInForce::from_spelling("day"), true);
    placed.set_tradable(Some(true), true);
    placed.finalize();
    // An equal-ranked statement under the same key leaves the held value.
    assert_eq!(placed.get_identifiers().get(&IdType::ClOrdId), Some("C-1"));
    assert_eq!(placed.get_partyids().to_string(), "[clientid=ACC-1, proprietary:clientid=ACC-1]");
    assert!(!placed.insert_identifier(fix(IdType::OrderId, "O-9999")?)?);
    assert!(Identifier::new(IdKey::base(IdType::SecondaryOrderId), "n/a").is_err(), "a null-like value is no identifier");
    assert_eq!(placed.get_timeinforce(), Some(&TimeInForce::Day));
    assert_eq!(placed.get_timeinforce().map(|tif| tif.as_str()), Some("DAY"));

    // The replacement states its own client and execution ids.
    let mut replacing = OrderEvent::at(T + 1_000_000_000);
    replacing.set_crosscode("O-1001".to_owned());
    replacing.insert_identifier(fix(IdType::ClOrdId, "C-2")?)?;
    replacing.insert_identifier(fix(IdType::ExecId, "X-2")?)?;
    replacing.finalize();
    let replaced = replacing.with_previous(&placed).expect("a later event follows");
    assert_eq!(replaced.get_timeinforce(), placed.get_timeinforce());
    assert_eq!(replaced.get_tradable(), Some(true));
    // Its own identifiers stand and the chain's order id is taken - never the
    // entry reference - and its client order id names the one it replaced as
    // its parent.
    let identifiers: Vec<String> = replaced.get_identifiers().iter().map(ToString::to_string).collect();
    assert_eq!(identifiers, ["clordid=C-2", "execid=X-2", "orderid=O-1001", "origclordid=C-1"]);
    assert_eq!(replaced.get_identifiers().get(&IdType::OrigClOrdId), Some("C-1"));
    assert_eq!(replaced.get_partyids(), placed.get_partyids(), "the chain's party");
    // Stating no side, it stands on the chain's, under the chain's code.
    assert_eq!((replaced.get_side(), replaced.get_crosscode()), (Side::Buy, "10:1:O-1001"));

    // A statement naming no identifier takes the chain's.
    let mut ack = OrderEvent::at(T + 2_000_000_000);
    ack.set_crosscode("O-1001".to_owned());
    ack.finalize();
    let ack = ack.with_previous(&replaced).expect("a later event follows");
    assert_eq!(ack.get_identifiers(), replaced.get_identifiers());
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Side, TimeInForce, graph

    T = 1_700_000_000_000_000_000
    placed = graph.OrderEvent(
        T,
        crosscode="O-1001",
        side="BUYS",
        identifiers=[
            Identifier("clordid", "C-1"),
            Identifier("mdentryrefid", "R-1"),
            Identifier("orderid", "O-1001"),
        ],
        partyids=[Identifier("proprietary:clientid", "ACC-1")],
        timeinforce="day",
        tradable=True,
    )
    # Sources and types are lower-case words.
    assert [str(id) for id in placed.identifiers] == ["clordid=C-1", "mdentryrefid=R-1", "orderid=O-1001"]
    assert str(placed.partyids) == "[clientid=ACC-1, proprietary:clientid=ACC-1]"

    # The replacement states its own client and execution ids.
    replaced = graph.OrderEvent(
        T + 1_000_000_000,
        crosscode="O-1001",
        identifiers=[Identifier("clordid", "C-2"), Identifier("execid", "X-2")],
    ).with_previous(placed)
    assert replaced is not None
    assert (replaced.timeinforce, replaced.tradable) == (TimeInForce.DAY, True)
    # Its own identifiers stand and the chain's order id is taken - never the
    # entry reference - and its client order id names the one it replaced as
    # its parent.
    assert [str(id) for id in replaced.identifiers] == [
        "clordid=C-2", "execid=X-2", "orderid=O-1001", "origclordid=C-1",
    ]
    assert replaced.identifiers.get_from("origclordid") == "C-1"
    assert replaced.partyids == placed.partyids, "the chain's party"
    # Stating no side, it stands on the chain's, under the chain's code.
    assert (replaced.side, replaced.crosscode) == (Side.BUYS, "10:1:O-1001")

    # A statement naming no identifier takes the chain's.
    ack = graph.OrderEvent(T + 2_000_000_000, crosscode="O-1001").with_previous(replaced)
    assert ack is not None and ack.identifiers == replaced.identifiers
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const placed = new graph.OrderEvent(T, {
      crosscode: 'O-1001',
      side: 'BUYS',
      identifiers: [
        new Identifier('clordid', 'C-1'),
        new Identifier('mdentryrefid', 'R-1'),
        new Identifier('orderid', 'O-1001'),
      ],
      partyids: [new Identifier('proprietary:clientid', 'ACC-1')],
      timeinforce: 'day',
      tradable: true,
    })
    // Sources and types are lower-case words.
    assert.equal(placed.identifiers.toString(), '[clordid=C-1, mdentryrefid=R-1, orderid=O-1001]')
    assert.equal(placed.partyids.toString(), '[clientid=ACC-1, proprietary:clientid=ACC-1]')

    // The replacement states its own client and execution ids.
    const replaced = new graph.OrderEvent(T + 1_000_000_000n, {
      crosscode: 'O-1001',
      identifiers: [new Identifier('clordid', 'C-2'), new Identifier('execid', 'X-2')],
    }).withPrevious(placed)
    assert.equal(replaced.timeinforce, 'DAY')
    assert.equal(replaced.tradable, true)
    // Its own identifiers stand and the chain's order id is taken - never the
    // entry reference - and its client order id names the one it replaced as
    // its parent.
    assert.equal(
      replaced.identifiers.toString(),
      '[clordid=C-2, execid=X-2, orderid=O-1001, origclordid=C-1]',
    )
    assert.equal(replaced.identifiers.getFrom('origclordid'), 'C-1')
    assert.ok(replaced.partyids.equals(placed.partyids), "the chain's party")
    // Stating no side, it stands on the chain's, under the chain's code.
    assert.equal(replaced.side, 'BUYS')
    assert.equal(replaced.crosscode, '10:1:O-1001')

    // A statement naming no identifier takes the chain's.
    const ack = new graph.OrderEvent(T + 2_000_000_000n, { crosscode: 'O-1001' }).withPrevious(replaced)
    assert.ok(ack.identifiers.equals(replaced.identifiers))
    ```

## Edges

- `tradable` is what a status said: `None` states nothing, and a [book level](book.md#limits) reads an entry stating nothing as one that trades.
- A type or source that is no word, or a value that is empty, null-like, past 64 bytes once trimmed or one its type refuses, is refused by [`Identifier::new`](identifier.md#contract) before any verb sees it, and a binding refuses the fact by name.
- `mdentryrefid` is the one identifier type a follower never takes: a book entry's [reference to its predecessor](book.md#entries) names one step, not the chain. A FIX message follows only the types its dictionary's `FIX:idmap` entries flag, and the parents of each.
