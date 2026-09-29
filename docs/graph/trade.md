# Trade

`TradeEvent` is a composite trade: one operation event whose executions are the sided fills it is made of.

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::graph::TradeEvent` in `graph::trade`: `Element`, `Event`, `Market` and `Operation` over its root, filed under `TRAD` |
| Construction | `TradeEvent::from_parts(&root, executions)`, the one door, from any `Event + Operation` root and a `Vec<ExecutionEvent>`; a `TRAD` row decodes only through it. A trade is not [sided](market.md#sides-and-cross-codes): it keeps the root's cross code as given, the base code of a sided root, whatever side it states |
| Refusals | `InvalidRecord` at `$.executions` when there is none, and at `$.executions[i]` for a child at another instant (`.currunix`), naming another ticker (`.ticker`) or repeating a cross code (`.crosscode`); a child may state any side, `UNKNOWN` included, because a fill nobody sided is still a fill |
| Canonical | children are finalized and sorted by side, cross code and identity, so input order never changes the trade; `executions()` answers that order, each child's cross code carrying its [side](market.md#sides-and-cross-codes) |
| Root | the highest child place, the earliest creation and recording instants, the latest execution instant; its digest feeds the execution count and each child's `curruuid` - never a child's `currhashcode` or content |
| `is_execution` | always true |
| `set_currunix` | rebases the root and every child atomically and re-finalizes them; each child keeps its `execunix` |
| Following, merging | only under the same root cross code: children combine by execution cross code, then rebase to the resulting instant |
| In a book | a [book](book.md#books) takes the bounds from the root and lists the children in `executions()`; neither enters depth |
| From FIX | a trade capture report (`AE`) is no leaf: [`FixMsg::market_data`](../fix/message.md#market-data) answers none for it, and a capture's market data and books skip it; what it reports are the executions its parse splits off - one execution message per `NoSides(552)` occurrence, `EXEC` and `FILLED`, of side `UNKNOWN` beside a [warning](../fix/capture.md#warnings) where the occurrence states no side or one no side reads - so each fill is stated once |
| Bindings | Python `graph.TradeEvent.from_parts(root, executions)`, JavaScript `graph.TradeEvent.fromParts(root, executions)`; the built trade's `executions` is a property in both, a list (Python) or an array (JavaScript) of `ExecutionEvent` in the canonical order |

## Example

Apple shares crossed between a buyer and a seller at 189.50.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, ExecutionEvent, Market, OrderEvent, TradeEvent};
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let fill = |code: &str, side: Side| -> yggdryl::Result<ExecutionEvent> {
        let mut execution = ExecutionEvent::at(T);
        execution.set_crosscode(code.to_owned());
        execution.set_side(side);
        execution.set_lastpx(Some("189.50".parse()?));
        execution.set_lastqty(Some(Decimal::from_int(100)));
        Ok(execution)
    };
    let mut root = OrderEvent::at(T);
    root.set_crosscode("T-1".to_owned());
    root.set_ticker(Some("AAPL".into()));

    let trade = TradeEvent::from_parts(&root, vec![fill("E-SELL", Side::Sell)?, fill("E-BUY", Side::Buy)?])?;
    let codes: Vec<&str> = trade.executions().iter().map(Element::get_crosscode).collect();
    assert_eq!(codes, ["BUY:E-BUY", "SELL:E-SELL"]);
    assert!(trade.is_execution());
    // Input order cannot change the trade.
    let again = TradeEvent::from_parts(&root, vec![fill("E-BUY", Side::Buy)?, fill("E-SELL", Side::Sell)?])?;
    assert_eq!(again.get_curruuid(), trade.get_curruuid());

    // An execution is required, each at the trade's instant.
    assert!(TradeEvent::from_parts(&root, Vec::new()).is_err());
    let mut late = fill("E-LATE", Side::Buy)?;
    late.set_currunix(T + 1);
    let error = TradeEvent::from_parts(&root, vec![late]).unwrap_err();
    assert!(error.to_string().contains("$.executions[0].currunix"), "{error}");

    // Any side stands: a fill nobody sided is still a fill.
    let unsided = TradeEvent::from_parts(&root, vec![fill("E-NONE", Side::Unknown)?])?;
    assert_eq!(unsided.executions()[0].get_side(), Side::Unknown);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def fill(code: str, side: str, unix: int = T) -> graph.ExecutionEvent:
        return graph.ExecutionEvent(unix, crosscode=code, side=side, lastpx=Decimal("189.50"), lastqty=100)

    root = graph.OrderEvent(T, crosscode="T-1", ticker="AAPL")
    trade = graph.TradeEvent.from_parts(root, [fill("E-SELL", "SELL"), fill("E-BUY", "BUY")])
    assert [execution.crosscode for execution in trade.executions] == ["BUY:E-BUY", "SELL:E-SELL"]
    assert trade.is_execution
    # Input order cannot change the trade.
    again = graph.TradeEvent.from_parts(root, [fill("E-BUY", "BUY"), fill("E-SELL", "SELL")])
    assert again.curruuid == trade.curruuid

    # An execution is required, each at the trade's instant.
    for executions, where in (
        ([], "$.executions:"),
        ([fill("E-LATE", "BUY", T + 1)], "$.executions[0].currunix"),
    ):
        try:
            graph.TradeEvent.from_parts(root, executions)
        except ValueError as error:
            assert where in str(error), error
        else:
            raise AssertionError("an invalid trade was built")

    # Any side stands: a fill nobody sided is still a fill.
    unsided = graph.TradeEvent.from_parts(root, [fill("E-NONE", "UNKNOWN")])
    assert unsided.executions[0].side is Side.UNKNOWN
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const fill = (code, side, unix = T) => new graph.ExecutionEvent(unix, {
      crosscode: code, side, lastpx: '189.50', lastqty: 100,
    })
    const root = new graph.OrderEvent(T, { crosscode: 'T-1', ticker: 'AAPL' })

    const trade = graph.TradeEvent.fromParts(root, [fill('E-SELL', 'SELL'), fill('E-BUY', 'BUY')])
    assert.deepEqual(trade.executions.map((execution) => execution.crosscode), ['BUY:E-BUY', 'SELL:E-SELL'])
    assert.equal(trade.isExecution, true)
    // Input order cannot change the trade.
    const again = graph.TradeEvent.fromParts(root, [fill('E-BUY', 'BUY'), fill('E-SELL', 'SELL')])
    assert.equal(again.curruuid, trade.curruuid)

    // An execution is required, each at the trade's instant.
    assert.throws(() => graph.TradeEvent.fromParts(root, []), /at least one execution/)
    assert.throws(() => graph.TradeEvent.fromParts(root, [fill('E-LATE', 'BUY', T + 1n)]), /\$\.executions\[0\]\.currunix/)

    // Any side stands: a fill nobody sided is still a fill.
    const unsided = graph.TradeEvent.fromParts(root, [fill('E-NONE', 'UNKNOWN')])
    assert.equal(unsided.executions[0].side, 'UNKNOWN')
    ```
