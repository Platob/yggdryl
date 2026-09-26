# Execution

`Execution` is an execution with no instant, `ExecutionEvent` one at an instant: what traded, against the order it fills.

## Contract

| Key | Rule |
| --- | --- |
| Types | `Execution = OperationElement<ExecutionKind>`, `ExecutionEvent = OperationEvent<ExecutionKind>`: the [operation leaf contract](order.md#contract), digested as `execution` |
| `is_execution` | always true, whatever the state; the [walk](event.md#lifecycle-walk) fills an absent `execunix` with the event's instant, and following keeps the later execution clock |
| What traded | `lastpx`, `lastqty`, `avgpx`, `cumqty`, `leavesqty`; never `price` or `quantity`, which are what an element states ([Market](market.md#contract)) |
| Following an order | a leaf's own `with_previous` follows its own kind only; through [`MarketData`](market-data.md#marketdata) an execution follows the order it fills and keeps its kind |
| In a trade | a [`TradeEvent`](trade.md) is made of executions, each on a bid or ask side at the trade's instant |
| In a book | a [book](book.md#books) lists an execution at its instant and never changes resting depth by it |

## Example

The fill of an Apple order a quarter second after it was placed.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, ExecutionEvent, Market, MarketData, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, Side, State};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut order = OrderEvent::at(T);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.set_price(Some("189.50".parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.set_currency(Ccy::new("USD")?);
    order.insert_altid("ORDERID", "O-1001")?;
    order.finalize();

    let mut fill = ExecutionEvent::at(T + 250_000_000);
    fill.set_crosscode("O-1001".to_owned());
    fill.set_side(Side::Buy);
    fill.set_state(State::from_spelling("Filled").expect("a shipped state"));
    fill.set_lastpx(Some("189.50".parse()?));
    fill.set_lastqty(Some(Decimal::from_int(100)));
    fill.set_cumqty(Some(Decimal::from_int(100)));
    fill.set_leavesqty(Some(Decimal::from_int(0)));
    fill.insert_altid("EXECID", "X-1")?;
    fill.finalize();
    assert!(fill.is_execution());
    // What traded is never a price the execution states.
    assert_eq!((fill.get_price(), fill.get_quantity()), (None, None));

    // It follows the order it fills, and stays an execution.
    let followed = MarketData::from(fill)
        .with_previous(&MarketData::from(order.clone()))
        .expect("a fill follows its order");
    let fill = followed.as_execution_event().expect("still an execution");
    assert_eq!((fill.get_prevuuid(), fill.get_seqnum()), (Some(order.get_curruuid()), 1));
    assert_eq!(fill.get_execunix(), Some(T + 250_000_000));
    assert_eq!(fill.get_prevpx(), Some("189.50".parse()?));
    assert_eq!(fill.get_currency().as_str(), "USD");
    assert_eq!(fill.get_altids().get("ORDERID"), Some("O-1001"));
    assert_eq!(fill.get_altids().get("EXECID"), Some("X-1"));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    order = graph.OrderEvent(
        T,
        crosscode="O-1001",
        side="BUY",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        altids={"ORDERID": "O-1001"},
    )
    fill = graph.ExecutionEvent(
        T + 250_000_000,
        crosscode="O-1001",
        side="BUY",
        state="FILLED",
        lastpx=Decimal("189.50"),
        lastqty=100,
        cumqty=100,
        leavesqty=0,
        altids={"EXECID": "X-1"},
    )
    assert fill.is_execution
    # What traded is never a price the execution states.
    assert (fill.price, fill.quantity) == (None, None)

    # It follows the order it fills, and stays an execution.
    followed = graph.MarketData(fill).with_previous(graph.MarketData(order))
    assert followed is not None and followed.kind == "execution_event"
    fill = followed.as_execution_event()
    assert fill is not None
    assert (fill.prevuuid, fill.seqnum) == (order.curruuid, 1)
    assert fill.execunix == T + 250_000_000
    assert fill.prevpx is not None and fill.prevpx.as_py() == Decimal("189.50")
    assert fill.currency.as_py() == "USD"
    assert fill.altids == {"EXECID": "X-1", "ORDERID": "O-1001"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = new graph.OrderEvent(T, {
      crosscode: 'O-1001', side: 'BUY', price: '189.50', quantity: 100, currency: 'USD', altids: { ORDERID: 'O-1001' },
    })
    let fill = new graph.ExecutionEvent(T + 250_000_000n, {
      crosscode: 'O-1001',
      side: 'BUY',
      state: 'FILLED',
      lastpx: '189.50',
      lastqty: 100,
      cumqty: 100,
      leavesqty: 0,
      altids: { EXECID: 'X-1' },
    })
    assert.equal(fill.isExecution, true)
    // What traded is never a price the execution states.
    assert.equal(fill.price, null)
    assert.equal(fill.quantity, null)

    // It follows the order it fills, and stays an execution.
    const followed = new graph.MarketData(fill).withPrevious(new graph.MarketData(order))
    assert.equal(followed.kind, 'execution_event')
    fill = followed.asExecutionEvent()
    assert.equal(fill.prevuuid, order.curruuid)
    assert.equal(fill.seqnum, 1)
    assert.equal(fill.execunix, T + 250_000_000n)
    assert.equal(fill.prevpx, '189.5')
    assert.equal(fill.currency, 'USD')
    assert.deepEqual(fill.altids, { EXECID: 'X-1', ORDERID: 'O-1001' })
    ```
