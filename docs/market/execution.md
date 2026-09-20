# Execution

`market::Execution` is one fill: the price and quantity that traded, the execution's own identifier, and the order it belongs to - read out of the one execution report that states it, which is why its door is the narrow one.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/execution.rs`: the `Execution` trait, the `ExecutionData` holder, the row and the `Product` answers; the FIX reading is `FixCodec::executions` beside the [codec](../fix/index.md) |
| Chain | the order's, and it is taken rather than guessed: a fill is read out of the same report that restates its order, so the [statements](iterator.md) stream walks that order first and the fill takes the chain's `crosscode` and `crossuuid` from it, whichever identifier the report spelled. A fill therefore joins the [order table](order.md) on `crossuuid` by construction. The names it goes by are its own - `execid`, `execrefid`, `secondaryexecid` - and the order's, never the match's, which two orders' fills share |
| Facts | `px` and `qty` the price and quantity that traded, `LastPx(31)` and `LastQty(32)`; the side, the currency and the unit; the ticker and the six codes; `miccode` the market it traded on, `LastMkt(30)`. Exactly what the traits name, so it holds a [`MarketEventData`](../graph.md) and nothing beside |
| Reads | `notional()` what the fill was worth, `is_partial()` whether it left its order open and `completes()` whether it finished it - each read off the order's status after the fill, which is what an execution report states |
| State | what the report stated, `OrdStatus(39)` else `ExecType(150)` |
| Doors | `FixCodec::executions(messages)`: one per execution report (`35=8`) stating a quantity that traded; an acknowledgement, a cancel and a reject fill nothing and yield none; `executions_arrow_reader(source)` under `ExecutionData::field()` |
| Writes back | `FixMsg::from_execution(&codec, &execution)` states an execution report, `35=8`, `150=F`, exactly: `ExecID(17)` from the name the execution goes by, refused where it has none; `OrderID(37)` and `ClOrdID(11)` where known; the order's status as far as a fill can say it; the instrument, `LastMkt(30)`, the side, `LastPx(31)`, `LastQty(32)`, the currency and unit; the instant as `TransactTime(60)` and `SendingTime(52)` |
| Bindings | Rust `Execution`/`ExecutionData`; Python `yggdryl.market.Execution`, `FixCodec.executions`, `executions_arrow_reader`, `FixMsg.from_execution`; JavaScript `market.Execution`, `executions`, `executionsArrowReader`, `FixMsg.fromExecution` |

## Use

An order acknowledged and filled twice: the acknowledgement fills nothing, and each fill is one execution in the order's chain.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Execution, ExecutionData, Product};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:33.100|10=0|",
    ];
    let fills: Vec<ExecutionData> = reader.executions(reader.parse_lines(lines)).collect::<yggdryl::Result<_>>()?;
    let [first, second] = fills.as_slice() else { panic!("two fills") };
    assert_eq!((first.get_px(), first.get_qty()), ("10.5".parse()?, Decimal18::from_int(40)));
    assert_eq!((second.get_px(), second.get_qty()), ("10.6".parse()?, Decimal18::from_int(60)));
    assert_eq!(first.get_miccode().map(yggdryl::Mic::as_str), Some("XNAS"));
    // What the stated facts imply.
    assert_eq!(first.notional(), Some(Decimal18::from_int(420)));
    assert!(first.is_partial() && !first.completes(), "the order stayed open after it");
    assert!(second.completes(), "the fill that finished the order");
    // The execution's own name, in the order's chain.
    assert_eq!(first.get_identifiers()["execid"], "E1");
    assert_eq!(first.get_crosscode(), "O1");
    assert_eq!(second.get_crossuuid(), first.get_crossuuid());
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_seqnum(), 1);
    // The fill states an execution report, and reads back as the fill.
    let message = FixMsg::from_execution(&reader, first)?;
    assert_eq!(message.header().msgtype(), "8");
    assert_eq!(message.by_tag(150)?.as_str(), Some("F"));
    assert_eq!(message.by_tag(39)?.as_str(), Some("1"), "still open");
    assert_eq!(message.get_lastqty(), Some(first.get_qty()));
    let field = ExecutionData::field()?;
    assert_eq!(field.fields()[16].name(), "px");
    assert_eq!(ExecutionData::from_row(&field, &first.into_row()?)?.get_curruuid(), first.get_curruuid());
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry
    from yggdryl.market import Execution

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:33.100|10=0|",
    ]
    first, second = reader.executions(reader.parse_lines(lines))
    assert (first.px.as_py(), first.qty.as_py()) == (Decimal("10.5"), Decimal("40"))
    assert (second.px.as_py(), second.qty.as_py()) == (Decimal("10.6"), Decimal("60"))
    assert first.miccode.as_py() == "XNAS"
    # What the stated facts imply.
    assert first.notional.as_py() == Decimal("420")
    assert first.is_partial and not first.completes
    assert second.completes
    # The execution's own name, in the order's chain.
    assert first.identifiers["execid"] == "E1"
    assert first.crosscode == "O1"
    assert second.crossuuid == first.crossuuid
    assert second.prevuuid == first.curruuid
    assert second.seqnum == 1
    # The fill states an execution report, and reads back as the fill.
    message = FixMsg.from_execution(reader, first)
    assert message.header().msgtype == "8"
    assert message.by_tag(150).as_py() == "F"
    assert message.by_tag(39).as_py() == "1"  # still open
    assert message.lastqty == first.qty
    field = Execution.field()
    assert field.field_at(16).name == "px"
    assert Execution.from_row(field, first.into_row()).curruuid == first.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:31.100|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|52=20260102-10:15:33.100|10=0|',
    ].map((line) => Buffer.from(line))
    const [first, second] = [...reader.executions(reader.parseLines(lines))]
    assert.deepEqual([first.px, first.qty], ['10.5', '40'])
    assert.deepEqual([second.px, second.qty], ['10.6', '60'])
    assert.equal(first.miccode, 'XNAS')
    // What the stated facts imply.
    assert.equal(first.notional, '420')
    assert.ok(first.isPartial && !first.completes)
    assert.equal(second.completes, true)
    // The execution's own name, in the order's chain.
    assert.equal(first.identifiers.execid, 'E1')
    assert.equal(first.crosscode, 'O1')
    assert.equal(second.crossuuid, first.crossuuid)
    assert.equal(second.prevuuid, first.curruuid)
    assert.equal(second.seqnum, 1)
    // The fill states an execution report, and reads back as the fill.
    const message = fix.FixMsg.fromExecution(reader, first)
    assert.equal(message.header().msgtype, '8')
    assert.equal(message.byTag(150).asJs(), 'F')
    assert.equal(message.byTag(39).asJs(), '1') // still open
    assert.equal(message.lastqty, first.qty)
    const field = market.Execution.field()
    assert.equal(field.fieldAt(16).name, 'px')
    assert.equal(market.Execution.fromRow(field, first.intoRow()).curruuid, first.curruuid)
    ```

## Row schema

`ExecutionData::field()` opens with the sixteen [event columns](../graph.md#columns), then the fill's own.

| column | datatype | value |
| --- | --- | --- |
| `px` | `decimal128(38, 18)` | nullable; the price that traded, `LastPx(31)` |
| `qty` | `decimal128(38, 18)` | nullable; the quantity that traded, `LastQty(32)` |
| `side` | `side` | required; `UNKNOWN` where none is stated |
| `currency` | `currency` | required; `XXX` where none is stated |
| `unit` | `utf8` | nullable; the unit the quantity is counted in |
| `symbolticker` | `utf8` | nullable; the ticker the instrument is known by |
| `isincode`, `cusipcode`, `sedolcode`, `bloombergcode` | `isin`, `cusip`, `sedol`, `bloomberg` | nullable; the instrument under each identifier the market named it by |
| `cficode` | `cfi` | nullable; the instrument's classification |
| `miccode` | `mic` | nullable; the market it traded on, `LastMkt(30)` |

The order's running totals after the fill - `cumqty`, `leavesqty`, `avgpx` - are the [order](order.md)'s facts, on the order statement the same report makes; the two join on `srcuuids`. An execution's table joins the [message table](../fix/capture.md#the-crates-own-columns) on `srcuuids` to `curruuid`, the order table on `crossuuid`, and its own table on `prevuuid` and `parentuuids`.

## Edges

- A report stating no `LastQty(32)`, or one of zero, is no execution: the door skips it.
- `from_execution` refuses an execution that goes by no `execid`, naming it: an execution report states one.
- The match a fill belongs to, `TrdMatchID(880)`, is the [trade](trade.md)'s name and not the fill's: two orders' fills share it, and a name shared across chains would join them as one.
- A fill's chain is its order's because the [statements](iterator.md) stream hands it the walked order statement's chain; reading fills out of a stream that carries no order statements would leave each fill naming only what its own report spelled.

## Commands

```bash
cargo test -p yggdryl --test market execution::
cargo test -p yggdryl --doc market::execution
cargo bench -p yggdryl --bench market -- "market/doors/executions|market/rows/execution"
```
