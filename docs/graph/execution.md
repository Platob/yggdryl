# Execution

`Execution` is an execution with no instant, `ExecutionEvent` one at an instant: one fill, complete in itself, against the order it fills.

## Contract

| Key | Rule |
| --- | --- |
| Types | `Execution = OperationElement<ExecutionKind>`, `ExecutionEvent = OperationEvent<ExecutionKind>`: the [operation leaf contract](order.md#contract), filed under `EXEC` |
| `is_execution` | always true, whatever the state; the [walk](event.md#lifecycle-walk) fills an absent [`execunix`](market.md#contract) with the event's instant, and following keeps the later execution clock |
| A chain of its own | the walk joins an execution to no chain by a name or a base code: it follows only under its own cross code, and a chain matches within one market data kind, so a fill never restates, follows or ends its order under a shared code |
| What traded | `lastpx`, `lastqty`, `avgpx`, `cumqty`, `leavesqty`; never `price` or `quantity`, which are what an element states ([Market](market.md#contract)): an execution's quantity is its own, and neither its `leavesqty` nor its order's ever becomes it. Its state says what it reports, so its order quantities fill one another as a working order's do ([by state](market.md#order-quantities-by-state)) |
| From FIX | an execution report, an order's or a quote's report and each side of a trade that report a fill split off an execution message at the parse: `EXEC`, state `FILLED`, an identity of its own, chained under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`, else `<report code>|Execution=<digest>`, on its side (`8:1:E-1`), its source's identity among its sources; the report keeps its own state (`PARTIALLY_FILLED`) and is its order's report (`ORDR`, `QUOT` where it names a `QuoteID`); an execution report of no fill - a new, a cancel, a reject, an expiry - is its order's (or quote's) leaf alone, so a venue's cancel takes the entry off its book, and only an `ExecutionAcknowledgement` (`BN`) or a `DontKnowTrade` (`Q`) is no leaf at all; an `ExecutionEvent` read off a FIX message reads `FILLED` unless a book message deleted it ([FIX](../fix/message.md#market-data)) |
| Following an order | a leaf's own `with_previous` follows its own kind only; through [`MarketData`](market-data.md#marketdata) an execution follows the order it fills and keeps its kind, taking what [an operation follows](operation.md#following-and-merging) |
| In a trade | a [`TradeEvent`](trade.md) is made of executions at the trade's instant, each on the side it states - `UKNW` included, a fill nobody said the side of |
| In a book | never: [`MarketDataKind::is_booked`](../types/enum/marketdatakind.md) prunes `EXEC` before a book walk routes it, and `add_operations` drops it, so an instant only an execution reached yields no book - a fill moves its book through its order's or quote's own report ([Book](book.md#book-fold)) |

## Example

The fill of an Apple order a quarter second after it was placed.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::{ExecutionEvent, Market, MarketData, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, State};
    use yggdryl_market::{IdKey, IdType, Identifier, Side};
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let mut order = OrderEvent::at(T);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.set_price(Some("189.50".parse()?), true);
    order.set_quantity(Some(Decimal::from_int(100)), true);
    order.set_currency(Ccy::new("USD")?, true);
    order.insert_identifier(Identifier::new(IdKey::base(IdType::OrderId), "O-1001")?)?;
    order.finalize();

    let mut fill = ExecutionEvent::at(T + 250_000_000);
    fill.set_crosscode("O-1001".to_owned());
    fill.set_side(Side::Buy, true);
    fill.set_state(State::Filled);
    fill.set_lastpx(Some("189.50".parse()?), true);
    fill.set_lastqty(Some(Decimal::from_int(100)), true);
    fill.set_cumqty(Some(Decimal::from_int(100)), true);
    fill.set_leavesqty(Some(Decimal::from_int(0)), true);
    fill.insert_identifier(Identifier::new(IdKey::base(IdType::ExecId), "X-1")?)?;
    fill.finalize();
    assert!(fill.is_execution());
    // What traded is never a price or a quantity the execution states: its
    // `leavesqty` is what its order has left, never its own quantity.
    assert_eq!((fill.get_price(), fill.get_quantity()), (None, None));

    // It follows the order it fills, and stays an execution.
    let followed = MarketData::from(fill)
        .with_previous(&MarketData::from(order.clone()))
        .expect("a fill follows its order");
    let fill = followed.as_execution_event().expect("still an execution");
    assert_eq!((fill.get_prevuuid(), fill.get_seqnum()), (Some(order.get_uuid()), 0));
    assert_eq!(fill.get_crosscode(), "8:1:O-1001");
    assert_eq!(fill.get_execunix(), Some(T + 250_000_000));
    assert_eq!(fill.get_prevpx(), Some("189.50".parse()?));
    assert_eq!(fill.get_currency().as_str(), "USD");
    assert_eq!(fill.get_identifiers().to_string(), "[execid=X-1, orderid=O-1001]");
    assert_eq!(fill.get_state(), &State::Filled);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Identifier, State, graph

    T = 1_700_000_000_000_000_000
    order = graph.OrderEvent(
        T,
        crosscode="O-1001",
        side="BUYS",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        identifiers=[Identifier("orderid", "O-1001")],
    )
    fill = graph.ExecutionEvent(
        T + 250_000_000,
        crosscode="O-1001",
        side="BUYS",
        state="FILLED",
        lastpx=Decimal("189.50"),
        lastqty=100,
        cumqty=100,
        leavesqty=0,
        identifiers=[Identifier("execid", "X-1")],
    )
    assert fill.is_execution
    # What traded is never a price or a quantity the execution states: its
    # `leavesqty` is what its order has left, never its own quantity.
    assert fill.price is None and fill.quantity is None

    # It follows the order it fills, and stays an execution.
    followed = graph.MarketData(fill).with_previous(graph.MarketData(order))
    assert followed is not None and followed.kind == "execution_event"
    fill = followed.as_execution_event()
    assert fill is not None
    assert (fill.prevuuid, fill.seqnum) == (order.uuid, 0)
    assert fill.crosscode == "8:1:O-1001"
    assert fill.execunix == T + 250_000_000
    assert fill.prevpx is not None and fill.prevpx.as_py() == Decimal("189.50")
    assert fill.currency.as_py() == "USD"
    assert str(fill.identifiers) == "[execid=X-1, orderid=O-1001]"
    assert fill.state is State.FILLED
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = new graph.OrderEvent(T, {
      crosscode: 'O-1001',
      side: 'BUYS',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      identifiers: [new Identifier('orderid', 'O-1001')],
    })
    let fill = new graph.ExecutionEvent(T + 250_000_000n, {
      crosscode: 'O-1001',
      side: 'BUYS',
      state: 'FILLED',
      lastpx: '189.50',
      lastqty: 100,
      cumqty: 100,
      leavesqty: 0,
      identifiers: [new Identifier('execid', 'X-1')],
    })
    assert.equal(fill.isExecution, true)
    // What traded is never a price or a quantity the execution states: its
    // `leavesqty` is what its order has left, never its own quantity.
    assert.equal(fill.price, null)
    assert.equal(fill.quantity, null)

    // It follows the order it fills, and stays an execution.
    const followed = new graph.MarketData(fill).withPrevious(new graph.MarketData(order))
    assert.equal(followed.kind, 'execution_event')
    fill = followed.asExecutionEvent()
    assert.equal(fill.prevuuid, order.uuid)
    assert.equal(fill.seqnum, 0)
    assert.equal(fill.crosscode, '8:1:O-1001')
    assert.equal(fill.execunix, T + 250_000_000n)
    assert.equal(fill.prevpx, '189.5')
    assert.equal(fill.currency, 'USD')
    assert.equal(fill.identifiers.toString(), '[execid=X-1, orderid=O-1001]')
    assert.equal(fill.state, 'FILLED')
    ```
