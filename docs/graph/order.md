# Order

`Order` is an order with no instant, `OrderEvent` one at an instant. The [quote](quote.md) and [execution](execution.md) leaves share this contract.

## Contract

| Key | Rule |
| --- | --- |
| Types | `OperationElement<K>` and `OperationEvent<K>` in `graph::operation`, `K` one of the sealed `OrderKind`, `QuoteKind`, `ExecutionKind`; the kind is `K::KIND`, a [`MarketKind`](market-data.md#marketdata) filed under its [`marketdatakind`](../types/enum/marketdatakind.md) - `ORDR`, `QUOT`, `EXEC` - and `is_execution()` reads the kind, never the state |
| Construction | `Order::new()` states nothing, `OrderEvent::at(unix)` only an instant; `at` and `into_element` move between the two, `into_element` dropping the event clocks and the book control and keeping `execunix`, a [market fact](market.md#contract) |
| Order facts | what it ordered is [`ordqty`](operation.md#order-quantity), FIX's `OrderQty(38)`, and how it is priced its `marketdatatype` - `ORDLIMIT`, `ORDMKT`, `ORDPEGGED` - a [`MarketDataType`](../types/enum/marketdatatype.md) a FIX order reads off `OrdType(40)`; both are columns of the [`marketdata` row](schemas.md#the-marketdata-row), and there is no `ordtype` fact |
| From another event | `OperationEvent::<K>::from(&event)` copies every fact the event states, its identities included, with no book control and no finalize |
| `finalize` | digests the leaf's `marketdatakind` code, so one entry as an order and as a quote are two operations; an element takes UUIDv8 over its code, an event also digests its [book scope](#book-control) and takes its [event identity](event.md#identity) |
| From FIX | one message is one leaf: an order message is an `OrderEvent`; an execution report of no fill - a new, a cancel, a reject, an expiry - is its order's report - `ORDR`, its own state - and one that reports a fill is split at the parse into that report and the [execution](execution.md#contract) it reports; an order batch - a list, a mass order, a cross, a mass cancel report (`ORDB`) - is split at the parse into one order message per entry ([FIX](../fix/message.md#a-parse-splits-what-a-message-reports)) |
| Following and merging | as [`Operation`](operation.md#following-and-merging) states; an element's `with_previous` takes the predecessor's cross code, market and operation facts with no timed link |
| Bindings | Python `graph.Order(**facts)`, `graph.OrderEvent(transunix, book=..., **facts)`; JavaScript `new graph.Order(facts)`, `new graph.OrderEvent(transunix, facts)` with `book` among the facts; `kind` the `MarketKind` spelling, `marketdatakind` the member (Python) or its name (JavaScript) |

## Book control

A market-data entry carries its book control beside its facts.

| Key | Rule |
| --- | --- |
| On the event | `book()`, `set_book(Option<BookRef>)`, `with_book(BookRef)`; `action()` and `scope()` read through it; `is_full_snapshot()` is `action() == Some(MdUpdateAction::Snapshot)`, which every FIX `W` carries |
| `BookRef` | `action`, `scope`, `position` (`MDEntryPositionNo(290)`), `entry_px`, `entry_size`, each optional; `is_stated()` when any is set; a control stating nothing is no control |
| Row facts | the scope, the action and the position are columns of the [`marketdata` row](market-data.md#arrow) - `bookscope` (`utf8`), `bookaction` (`utf8`, the `MDUpdateAction(279)` code or `snapshot`) and `bookposition` (`uint32`, `MDEntryPositionNo(290)`) - which a book's `delta` replays by; a `bookaction` no `MdUpdateAction` spelling reads is refused at that column |
| Identity | only the `scope` feeds the digest: the action and the position steer the [book](book.md#entries) and move no identity, and the price and size the entry stated are walk-time alone - no column, gone once the book placed the entry |
| Entry identifiers | not in the control: an entry's own and referenced identifiers are the `mdentryid` and `mdentryrefid` identifiers (`graph::book::ENTRY_ID`, `ENTRY_REF_ID`, both `IdType`s) |
| `MdUpdateAction` | FIX `MDUpdateAction(279)` - `New` (0), `Change`, `Delete`, `DeleteThru`, `DeleteFrom`, `Overlay` - plus `Snapshot` (6); `as_str` answers the code or `snapshot`, `read` the code, the folded name or `SNAPSHOT` (else `None`); `is_range_delete` for 3 and 4, `is_partial` for 1 and 5 |

How a book applies each action is the [book's](book.md#entries).

## Example

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookRef, Market, MarketKind, MdUpdateAction, Order, OrderEvent, QuoteEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::{MarketDataKind, Side};
    use yggdryl::Uuid;
    yggdryl_market::install()?;

    // An undated order: its identity is its content, UUIDv8 over its code.
    let mut order = Order::new();
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.set_price(Some("189.50".parse()?), true);
    order.finalize();
    assert_eq!(order.kind(), MarketKind::Order);
    assert_eq!(order.kind().marketdatakind(), MarketDataKind::Order);
    assert_eq!(order.get_uuid(), Uuid::from_v8(u128::from(order.get_hashcode())));

    // Dated, it is an order event, finalized: UUIDv7.
    let event: OrderEvent = order.clone().at(1_700_000_000_000_000_000);
    assert_eq!(event.get_uuid(), event.time_uuid()?);
    assert!(!event.is_execution(), "the kind decides, whatever the state");

    // The same facts as a quote are another operation.
    let mut quote = QuoteEvent::from(&event);
    quote.finalize();
    assert_ne!(quote.get_uuid(), event.get_uuid());

    // A market-data entry carries its book control; its scope digests into it.
    let mut entry = quote.clone().with_book(BookRef {
        action: MdUpdateAction::read("new"),
        scope: Some("AAPL.XNAS".into()),
        position: Some(1),
        ..BookRef::default()
    });
    entry.finalize();
    assert_eq!((entry.action().map(MdUpdateAction::as_str), entry.scope()), (Some("0"), "AAPL.XNAS"));
    assert_ne!(entry.get_uuid(), quote.get_uuid());
    // The action and the position steer the book: they move no identity.
    let mut placed = quote.clone().with_book(BookRef { action: Some(MdUpdateAction::New), position: Some(1), ..BookRef::default() });
    placed.finalize();
    assert_eq!(placed.get_uuid(), quote.get_uuid());

    // Undated again, the clocks and the control stay behind.
    let mut element = entry.into_element();
    element.finalize();
    let mut plain = quote.into_element();
    plain.finalize();
    assert_eq!(element, plain);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import MarketDataKind, graph

    # An undated order: its identity is its content.
    order = graph.Order(crosscode="O-1001", side="BUYS", price=Decimal("189.50"))
    assert order.kind == "order"
    assert order.marketdatakind is MarketDataKind.ORDR

    # Dated, it is an order event.
    event = order.at(1_700_000_000_000_000_000)
    assert isinstance(event, graph.OrderEvent)
    assert event.transunix == 1_700_000_000_000_000_000
    assert not event.is_execution, "the kind decides, whatever the state"

    # The same facts as a quote are another operation.
    quote = graph.QuoteEvent(event.transunix, crosscode="O-1001", side="BUYS", price=Decimal("189.50"))
    assert quote.uuid != event.uuid

    # A market-data entry carries its book control; its scope digests into it.
    entry = quote.with_book(graph.BookRef(action="new", scope="AAPL.XNAS", position=1))
    assert (entry.action, entry.scope, entry.book.position) == ("0", "AAPL.XNAS", 1)
    assert entry.uuid != quote.uuid
    # The action and the position steer the book: they move no identity.
    assert quote.with_book(graph.BookRef(action="new", position=1)).uuid == quote.uuid

    # Undated again, the clocks and the control stay behind.
    assert entry.into_element() == quote.into_element()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    // An undated order: its identity is its content.
    const order = new graph.Order({ crosscode: 'O-1001', side: 'BUYS', price: '189.50' })
    assert.equal(order.kind, 'order')
    assert.equal(order.marketdatakind, 'ORDR')

    // Dated, it is an order event.
    const event = order.at(1_700_000_000_000_000_000n)
    assert.ok(event instanceof graph.OrderEvent)
    assert.equal(event.transunix, 1_700_000_000_000_000_000n)
    assert.equal(event.isExecution, false, 'the kind decides, whatever the state')

    // The same facts as a quote are another operation.
    const quote = new graph.QuoteEvent(event.transunix, { crosscode: 'O-1001', side: 'BUYS', price: '189.50' })
    assert.notEqual(quote.uuid, event.uuid)

    // A market-data entry carries its book control; its scope digests into it.
    const entry = quote.withBook(new graph.BookRef({ action: 'new', scope: 'AAPL.XNAS', position: 1 }))
    assert.equal(entry.action, '0')
    assert.equal(entry.scope, 'AAPL.XNAS')
    assert.equal(entry.book.position, 1)
    assert.notEqual(entry.uuid, quote.uuid)
    // The action and the position steer the book: they move no identity.
    assert.equal(quote.withBook(new graph.BookRef({ action: 'new', position: 1 })).uuid, quote.uuid)

    // Undated again, the clocks and the control stay behind.
    assert.ok(entry.intoElement().equals(quote.intoElement()))
    ```

## Edges

- `set_book` and `From<&E>` do not finalize: a caller finalizes once the facts are in.
- An undated leaf states no order: `is_after` is always false, so a sort keeps arrival order.
- A book folds only dated operations and refuses an undated `Order` by its [kind](book.md#edges).
