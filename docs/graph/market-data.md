# Market data

`MarketData` is one value over every leaf, and the lifted `marketdata` Arrow row any of them crosses a boundary as; the column enums name that row's facts, `MarketView` reads it through named plans.

## MarketData

| Key | Rule |
| --- | --- |
| Variants | `Order`, `Quote`, `Execution`, `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent`, `BookEvent` (boxed), `SnapshotEvent`, and `Fix` - a boxed [`FixMsg`](../fix/message.md) held whole ([below](#a-fix-message-held-whole)) - in `graph::market_data` |
| `kind()` | the `MarketKind`, in `graph::kind`: `as_str` = `order`, `quote`, `execution`, `order_event`, `quote_event`, `execution_event`, `trade_event`, `book_event`, `snapshot_event`, `fix`; `read` = same (any case); `is_event` = one of the six dated leaves or a FIX message, echoed by the value's `is_event()` |
| `marketdatakind()` | the [`MarketDataKind`](../types/enum/marketdatakind.md) the leaf stands under, `MarketKind::marketdatakind`: an order `ORDR` (`10`), a quote `QUOT` (`14`), an execution `EXEC` (`8`), a trade `TRAD` (`21`), a book and a snapshot control `BOOK` (`3`), a FIX message the category its dictionary files it under (`FixMsg::msgcat`); the first market column of the row, and what an operation's digest feeds, so the same facts as an order and as a quote are two operations |
| Conversions | `From<leaf>` builds one from every leaf; `TryFrom<MarketData>` takes it back, else `InvalidRecord` at `$.kind` (names expected/found); `as_order()` ... `as_snapshot_event()` and `as_fix()` borrow a variant; `book()` = the [control](order.md#book-control) of an operation event or snapshot |
| Traits | `Element`/`Market` delegate to the leaf held, so a resolved boundary reads it generically |
| Order, following, merging | `is_after` orders two dated values by instant, none if either undated; `with_previous`/`merge_with` are the leaf's own for one variant; an operation event also follows another kind via shared facts, keeping its own kind (an execution follows its filled order); a merge never crosses variants |
| Bindings | Python `graph.MarketData(leaf)`, `MarketData.kinds`, `kind`, `marketdatakind` (the `yggdryl.MarketDataKind` member), `is_event`, `as_order_event()` and the rest, `as_fix()`, `into_leaf()`, a `FixMsg` taken and answered like any leaf; JavaScript `new graph.MarketData(leaf)`, `MarketData.kinds()`, `marketdatakind` (the member's name, `'ORDR'`), `isEvent`, `asOrderEvent()`, `asFix()`, `intoLeaf()`; `enums.MARKET_KINDS`/`enums.marketKinds` list the spellings |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, MarketData, MarketKind, OrderEvent, QuoteEvent};
    use yggdryl::MarketDataKind;

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.finalize();

    let value = MarketData::from(order.clone());
    assert_eq!(value.kind(), MarketKind::OrderEvent);
    assert_eq!(value.kind().as_str(), "order_event");
    assert_eq!(MarketKind::read("ORDER_EVENT"), Some(MarketKind::OrderEvent));
    assert_eq!(value.marketdatakind(), MarketDataKind::Order);
    assert_eq!(value.marketdatakind().code(), 10);
    assert!(value.is_event());
    assert_eq!(value.as_order_event(), Some(&order));
    assert_eq!(value.get_curruuid(), order.get_curruuid());
    assert!(QuoteEvent::try_from(value.clone()).is_err(), "another kind is refused at $.kind");
    assert_eq!(OrderEvent::try_from(value)?, order);
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataKind, graph

    order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    value = graph.MarketData(order)
    assert value.kind == "order_event"
    assert "order_event" in graph.MarketData.kinds
    assert value.marketdatakind is MarketDataKind.ORDR and value.marketdatakind == 10
    assert value.is_event
    assert value.as_order_event() == order
    assert value.curruuid == order.curruuid
    assert value.as_quote_event() is None, "another kind is none of this value"
    assert value.into_leaf() == order
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MarketDataKind, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    const value = new graph.MarketData(order)
    assert.equal(value.kind, 'order_event')
    assert.ok(graph.MarketData.kinds().includes('order_event'))
    assert.equal(value.marketdatakind, 'ORDR')
    assert.equal(MarketDataKind[value.marketdatakind], 10)
    assert.equal(value.isEvent, true)
    assert.ok(value.asOrderEvent().equals(order))
    assert.equal(value.curruuid, order.curruuid)
    assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
    assert.ok(value.intoLeaf() instanceof graph.OrderEvent)
    ```

### A FIX message held whole

`MarketData::from(message)` holds a [`FixMsg`](../fix/message.md) as it is - its fields, its capture and every fact its dictionary reads - rather than the leaves it reports. It answers `Element`, `Event` and `Market` as the message does, its kind is `fix`, its `marketdatakind` the message's `msgcat`, and it walks and merges as itself: an event walk follows it by its own identity, and a merge meets only another FIX message. Where a leaf is the only thing that can stand, it is split there: the Arrow writer and the [book fold](book.md#book-fold) take the leaves `FixMsg::into_market_data` answers - one per entry of a `W` or `X`, nothing for a trade or a batch, whose parse already split them off - so a row is always a leaf. `FixMsg::into_market_leaf` is the one leaf a message is, refused where it is not exactly one; `TryFrom<MarketData> for FixMsg` takes the message back.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, MarketData, MarketKind};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let message = codec
        .parse_line(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|")?
        .next()
        .expect("one frame")?;

    // Held whole: the message's own kind, category and identity.
    let value = MarketData::from(message.clone());
    assert_eq!(value.kind(), MarketKind::Fix);
    assert_eq!(value.kind().as_str(), "fix");
    assert_eq!(value.marketdatakind(), MarketDataKind::Book);
    assert!(value.is_event());
    assert_eq!(value.get_curruuid(), message.get_curruuid());
    assert!(value.as_fix().is_some() && value.as_quote_event().is_none());

    // Split where it is written: one row per book entry, each a quote.
    let rows = MarketData::arrow_reader([value.clone()], None, None)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 2);
    assert_eq!(FixMsg::try_from(value)?.msgcat(), MarketDataKind::Book);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import MarketDataKind, graph
    from yggdryl.fix import FixCodec, FixMsg, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    message = codec.parse_fix_line(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|"
    )

    # Held whole: the message's own kind, category and identity.
    value = graph.MarketData(message)
    assert value.kind == "fix" and "fix" in graph.MarketData.kinds
    assert value.marketdatakind is MarketDataKind.BOOK
    assert value.is_event
    assert value.curruuid == message.curruuid
    assert value.as_fix() == message and value.as_quote_event() is None
    assert isinstance(value.into_leaf(), FixMsg)

    # Split where it is written: one row per book entry, each a quote.
    table = graph.MarketData.arrow_reader([value]).read_all()
    assert table.num_rows == 2
    assert table.column("marketdatakind").to_pylist() == [int(MarketDataKind.QUOT)] * 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { MarketDataKind, fix, graph } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const message = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|'),
    )

    // Held whole: the message's own kind and category.
    const value = new graph.MarketData(message)
    assert.equal(value.kind, 'fix')
    assert.ok(graph.MarketData.kinds().includes('fix'))
    assert.equal(value.marketdatakind, 'BOOK')
    assert.ok(value.asFix() instanceof fix.FixMsg)
    assert.equal(value.asQuoteEvent(), null)

    // Split where it is written: one row per book entry, each a quote.
    const table = graph.MarketData.arrowReader([value]).intoTable()
    assert.equal(table.numRows, 2)
    assert.deepEqual([...table.getChild('marketdatakind')], [MarketDataKind.QUOT, MarketDataKind.QUOT])
    ```

## Columns

Four enums - `ElementColumn` (`graph::element_column`), `EventColumn` (`graph::column`), `MarketColumn` (`graph::market_column`), `OperationColumn` (`graph::operation_column`) - name every fact the traits answer, one column each, so a [text line](../media/index.md#plain-text)'s batch, a [FIX row](../fix/capture.md#the-crates-own-columns) and a [chained message](../fix/lifecycle.md#a-chain-carries-its-creation-and-its-history) join without mapping.

| Enum | Columns, in `ALL` order |
| --- | --- |
| `ElementColumn` (6) | `curruuid`, `crossuuid` ([`uuid`](../types/uuid.md)), `crosscode` (`utf8`), `currhashcode`, `crosshashcode` (`uint64`), `srcuuids` (`serie<uuid>`, item `srcuuid`) |
| `EventColumn` (9) | `currunix`, `creaunix`, `recdunix`, `exprunix`, `prevunix`, `snapunix` (nanosecond UTC clocks); `prevuuid` ([`uuid`](../types/uuid.md)), `seqnum` (`uint64`); `state` ([`state`](../types/enum/state.md#the-code-is-the-rank)) |
| `MarketColumn` (34) | `marketdatakind`, `marketdatatype`, `price`, `stoppx`, `currency`, `quantity`, `displayqty`, `hiddenqty`, `unit`, `side`, `securityids`, `isincode`, `cficode`, `miccode`, `execunix`, `lastpx`, `lastqty`, `avgpx`, `cumqty`, `leavesqty`, `cxlqty`, `prevpx`, `prevqty`, `spotrate`, `forwardpoints`, `bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy`, `fxrates`, `ticker`, `metadata`: `marketdatakind` and `marketdatatype` the [`marketdatakind`](../types/enum/marketdatakind.md) and [`marketdatatype`](../types/enum/marketdatatype.md) enums, `quantity` `Quantity(53)` or, where none is stated, what is left to work, `execunix` a nanosecond UTC clock like the event's, numbers are [`decimal`](../types/numeric/decimal.md#decimal), each [code](../types/codes/index.md) its own leaf (`ccy` for the three currencies, `unit`, `isin`, `cfi`, `mic`), `side` the [`side`](../types/enum/side.md) enum, `securityids` a sorted `map<utf8, utf8>` of [identifiers](identifier.md#arrow), the key's text to its value (`Identifiers::dtype()`), a sorted `map<utf8, utf8>` for the metadata, a sorted `map<ccy, decimal>` for the rates, `utf8` for the ticker |
| `OperationColumn` (5) | `ordqty` (`decimal`, the quantity ordered, `OrderQty(38)`), `timeinforce` (`timeinforce`), `tradable` (`boolean`), `identifiers`, `partyids` (each `Identifiers::dtype()`, a sorted `map<utf8, utf8>` from the key's text - `src:type`, the type alone for the base source - to the value) |

| Key | Rule |
| --- | --- |
| Verbs | each enum answers `ALL`, `name`, `display`, `datatype`, `nullable`, `field`, `fields`, `of_name` (any case), `fact` (what an element states, nothing if none), `record` (states a cell back: null clears it, unreadable leaves it unchanged); `ElementColumn` and `EventColumn` also `description`, each taking its trait: `Element`/`Event`/`Market`/`Operation` |
| Nullability | never null: `currunix`, `curruuid`, `crossuuid`, `currhashcode`, `crosshashcode`, `marketdatakind`, `marketdatatype`, `currency`, `unit`, `side` - a type stated as none is `UNKN`, and a side stated as none is the cell `UNKN` (code `0`); every other column is null where nothing is stated (empty code/serie/map, zero place, absent instant); `state` also admits null - no neutral member for an empty cell |
| `execunix` | when the element last executed: a market fact, so an undated leaf states it too and a text line, which is an event and no market element, states none ([Market](market.md#contract)) |
| `isincode` | a projection of `securityids`: `fact` is the value of its `isin` identifier, and `record` fills an absent `isin` base key and ignores a disagreeing one - the strict door is the [row reader](#arrow) |
| Order | `ALL` is the canonical order `fields()` and event-native schemas use; a FIX row holds the same columns through the crate's own fields, each at its datatype, in protocol-oriented time/identity bands rather than reordered around `ALL` |
| Round trip | a line read back from its batch, a message from its row, restate all six element and nine event columns - identities and clocks survive |
| Bindings | Python `enums.ELEMENT_COLUMNS`, `EVENT_COLUMNS`, `MARKET_COLUMNS`, `OPERATION_COLUMNS`; JavaScript `enums.elementColumns`, `eventColumns`, `marketColumns`, `operationColumns`; column verbs are Rust-only. The whole row is listed on [Row schemas](schemas.md#the-marketdata-row) |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, ElementColumn, EventColumn, Market, MarketColumn, MarketData, OperationColumn, OrderEvent};
    use yggdryl::{Scalar, Side};

    assert_eq!(
        (ElementColumn::ALL.len(), EventColumn::ALL.len(), MarketColumn::ALL.len(), OperationColumn::ALL.len()),
        (6, 9, 34, 5)
    );
    assert_eq!((EventColumn::ALL[0].name(), EventColumn::ALL[8].name()), ("currunix", "state"));
    assert_eq!((MarketColumn::ALL[0].name(), MarketColumn::ALL[1].name()), ("marketdatakind", "marketdatatype"));
    assert!(!ElementColumn::CurrUuid.nullable() && !MarketColumn::Side.nullable());
    // When an element last executed is a market column, never an event's.
    assert_eq!(MarketColumn::of_name("EXECUNIX"), Some(MarketColumn::ExecUnix));
    assert_eq!(EventColumn::of_name("execunix"), None);

    // The lifted row states them in that order.
    let field = MarketData::field()?;
    let names: Vec<&str> = field.fields()[..54].iter().map(|child| child.name()).collect();
    let listed: Vec<&str> = ElementColumn::ALL.map(ElementColumn::name).into_iter()
        .chain(EventColumn::ALL.map(EventColumn::name))
        .chain(MarketColumn::ALL.map(MarketColumn::name))
        .chain(OperationColumn::ALL.map(OperationColumn::name))
        .collect();
    assert_eq!(names, listed);

    // What an element states under a column, and the same fact stated back.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.finalize();
    let code = ElementColumn::of_name("CROSSCODE").and_then(|column| column.fact(&order)).expect("a code");
    let side = MarketColumn::Side.fact(&order).expect("a side");
    let mut again = OrderEvent::at(1_700_000_000_000_000_000);
    ElementColumn::CrossCode.record(&mut again, &code);
    MarketColumn::Side.record(&mut again, &side);
    assert_eq!((again.get_crosscode(), again.get_side()), ("10:1:O-1001", Side::Buy));
    // Nothing stated is a null, except a side: stated as none, it is UNKN.
    assert_eq!(EventColumn::PrevUuid.fact(&order), None);
    assert_eq!(MarketColumn::Price.fact(&order), None);
    assert_eq!(MarketColumn::Side.fact(&OrderEvent::at(1)), Some(Scalar::from(Side::Unknown)));
    ```

=== "Python"

    ```python
    from yggdryl import enums, graph

    counts = [len(enums.ELEMENT_COLUMNS), len(enums.EVENT_COLUMNS), len(enums.MARKET_COLUMNS), len(enums.OPERATION_COLUMNS)]
    assert counts == [6, 9, 34, 5]
    assert (enums.EVENT_COLUMNS[0], enums.EVENT_COLUMNS[-1]) == ("currunix", "state")
    assert enums.MARKET_COLUMNS[:2] == ("marketdatakind", "marketdatatype")
    assert enums.OPERATION_COLUMNS == ("ordqty", "timeinforce", "tradable", "identifiers", "partyids")
    # When an element last executed is a market column, never an event's.
    assert "execunix" in enums.MARKET_COLUMNS and "execunix" not in enums.EVENT_COLUMNS

    # The lifted row states them in that order.
    names = [child.name for child in graph.MarketData.field()]
    assert names[:54] == [
        *enums.ELEMENT_COLUMNS, *enums.EVENT_COLUMNS, *enums.MARKET_COLUMNS, *enums.OPERATION_COLUMNS
    ]
    assert names[54:] == ["bookscope", "alive", "deltas", "executions", "bidlimits", "asklimits"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { enums, graph } = require('yggdryl')

    const lists = [enums.elementColumns, enums.eventColumns, enums.marketColumns, enums.operationColumns]
    assert.deepEqual(lists.map((list) => list.length), [6, 9, 34, 5])
    assert.deepEqual([enums.eventColumns[0], enums.eventColumns[8]], ['currunix', 'state'])
    assert.deepEqual(enums.marketColumns.slice(0, 2), ['marketdatakind', 'marketdatatype'])
    assert.deepEqual([...enums.operationColumns], ['ordqty', 'timeinforce', 'tradable', 'identifiers', 'partyids'])
    // When an element last executed is a market column, never an event's.
    assert.ok(enums.marketColumns.includes('execunix') && !enums.eventColumns.includes('execunix'))

    // The lifted row states them in that order.
    const field = graph.MarketData.field()
    const names = Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at).name)
    assert.deepEqual(names.slice(0, 54), lists.flat())
    assert.deepEqual(names.slice(54), ['bookscope', 'alive', 'deltas', 'executions', 'bidlimits', 'asklimits'])
    ```

## Arrow

| Key | Rule |
| --- | --- |
| `MarketData::field()` | in `graph::arrow`: the required `marketdata` struct, 60 columns - `[0..6]` `ElementColumn::ALL` and `[6..15]` `EventColumn::ALL`, nullable here (an undated leaf has no clock); `[15..49]` `MarketColumn::ALL`, opening with the required `marketdatakind` and `marketdatatype`; `[49..54]` `OperationColumn::ALL`; `[54]` `bookscope` (`utf8`), the one book-control fact a row states; `[55..60]` the nullable nested columns `alive`, `deltas`, `executions`, `bidlimits`, `asklimits`. Every column is listed on [Row schemas](schemas.md#the-marketdata-row) |
| Operation rows | the item of `alive`, `deltas` and `executions`: the root's first 55 columns, nothing nested |
| Nested columns | a book's `alive` is every live order and quote of both sides - the bid side's best first, then the ask side's - and its `deltas` what it applied since the book before; `executions` a trade's or a book's; `bidlimits`, `asklimits` a book's price levels, one [`Limit`](book.md#limits) each, best first and the unpriced last, an empty side an empty list; a leaf leaves null each list it does not hold |
| Rows | every fact is its own typed column, null where the leaf states none; a book states its best tradable bid and ask in `bidpx`/`bidqty`/`bidccy` and `askpx`/`askqty`/`askccy`; `isincode` is the `isin` of `securityids`; every decimal is the registered [`decimal`](../types/numeric/decimal.md#decimal) |
| `arrow_reader(values, batch_row_size, batch_byte_size)` | streams `IntoIterator` of `MarketData`/`Result<MarketData>` into bounded `BatchReader` batches, lazily - a [FIX message held whole](#a-fix-message-held-whole) written as the leaves it splits into - column by column, no per-row `Scalar`; no row bound = shared default, zero = one row; a byte bound closes a nonempty batch once reached; values write only as their canonical self - stale derived facts refused at their row; a source/refusal error follows the completed prefix, fuses the reader |
| `from_arrow_reader(batches)` | one value per row, tolerant of shape: root columns resolved by name once per stream (any case, subset, order); an unnamed column ignored; a castable column cast via one plan compiled before the first batch; two columns naming one fact refused before a row is read |
| Leaves | a row's `marketdatakind` and `currunix` name its leaf - [below](#the-leaf-a-row-names); each batch lands once as one record [`Serie`](../types/serie.md), so no cell decodes twice; a refused value is named by row and path - `$[0].alive[0].miccode` - before any leaf is rebuilt |
| Canonical rows | a trade rebuilds only via `TradeEvent::from_parts`, a book from its `alive`, `deltas` and `executions` directly, never by replaying deltas; every stated identity must match the rebuilt leaf's (null `curruuid`/`crossuuid`/`currhashcode`/`crosshashcode` refused, absent = nothing); every other stated fact - the stored `crosscode`, a book's `bidlimits`/`asklimits` and bid/ask included - must match the leaf, else refused with `expected the value derived from the row ...` (a dated order stating no side and the code `O-1001` is refused at `$[0].crosscode`: expected `10:0:O-1001`); a reader failure or refused row returns once, fuses the iterator |
| `isincode`, `fxrates` | a stated `isincode` fills an absent `isin` base key and must equal the `securityids` answer (`expected the securityids isin "US0378331005", got ...`); `securityids`, `identifiers` and `partyids` are read raw and closed, and refuse a key that reads as none, a value `Identifier::new` refuses and two spellings of one key with two values, located at the row's column and key (`$[0].identifiers['fix:']`); `fxrates` refuses a null key or rate and a target stated twice |
| Bindings | Python `graph.MarketData.field()`, `arrow_reader(items, batch_row_size=None, batch_byte_size=None)` - a `pyarrow.RecordBatchReader` - and `from_arrow_reader(source)`; JavaScript `graph.MarketData.field()`, `arrowReader(items, batchRowSize, batchByteSize)` - a `BatchReader` - and `fromArrowReader(reader)`, any source `BatchReader.from` accepts |

### The leaf a row names

| `marketdatakind` | `currunix` null | `currunix` stated |
| --- | --- | --- |
| `ORDR` | `Order` | `OrderEvent` |
| `QUOT` | `Quote` | `QuoteEvent` |
| `EXEC` | `Execution` | `ExecutionEvent` |
| `TRAD` | refused at `$[i].marketdatakind`: `expected a dated TRAD row, got currunix null` | `TradeEvent`; a null `executions` is refused at `$[i].executions` |
| `BOOK` | refused at `$[i].marketdatakind`: `expected a dated BOOK row, got currunix null` | `BookEvent` where `alive` is non-null - an empty list included - and `SnapshotEvent` where it is null, or empty beside the `curruuid` a snapshot control derives, which no book shares: a table may store a null list as an empty one, as PyIceberg does; a batch with no `alive` column is refused at `$[i].alive`: `expected the alive column that tells a book_event from a snapshot_event, got none` |
| another member, or null | refused at `$[i].marketdatakind`: `expected ORDR, QUOT, EXEC, TRAD or BOOK, got ACCT` (`got null`) | the same |

A nested row names its leaf the same way: `alive` and `deltas` items must be `ORDR` or `QUOT` (`expected ORDR or QUOT on a book, got EXEC`), and an `executions` item `EXEC` or null.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{BookEvent, Element, Event, MarketData, MarketKind, Order, OrderEvent, SnapshotEvent};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.finalize();
    // A value is written only as its canonical self: finalized.
    let mut undated = Order::new();
    undated.finalize();
    let values = vec![
        MarketData::from(undated),
        MarketData::from(order.clone()),
        MarketData::from(BookEvent::new(1_700_000_001_000_000_000, "AAPL")),
        MarketData::from(SnapshotEvent::snapshot(&order, None)),
    ];

    // Written column by column, read back value for value.
    let batches: Vec<RecordBatch> =
        MarketData::arrow_reader(values.clone(), None, None)?.collect::<Result<_, _>>()?;
    let read: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
            .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read, values);
    // An empty book states its entries - none - and a control states none.
    let kinds: Vec<MarketKind> = read.iter().map(MarketData::kind).collect();
    assert_eq!(kinds, [MarketKind::Order, MarketKind::OrderEvent, MarketKind::BookEvent, MarketKind::SnapshotEvent]);

    // The row: 6 element, 9 event, 34 market and 5 operation columns,
    // the book scope, then the five nested columns.
    let field = MarketData::field()?;
    assert_eq!(field.field_len(), 6 + 9 + 34 + 5 + 1 + 5);
    let nested: Vec<&str> = field.fields()[55..].iter().map(|child| child.name()).collect();
    assert_eq!(nested, ["alive", "deltas", "executions", "bidlimits", "asklimits"]);

    // A foreign shape: a few columns in another order, one it does not name.
    let written = MarketData::arrow_reader([MarketData::from(order.clone())], None, None)?
        .next()
        .expect("one batch")?;
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();
    for name in ["crosscode", "currunix"] {
        let index = written.schema().index_of(name)?;
        fields.push(written.schema().field(index).clone());
        columns.push(written.column(index).clone());
    }
    fields.push(Field::new("MarketDataKind", DataType::Int32, true));
    columns.push(Arc::new(Int32Array::from(vec![10])));
    fields.push(Field::new("msgtype", DataType::Utf8, true));
    columns.push(Arc::new(StringArray::from(vec!["D"])));
    let foreign = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)?;
    let lifted: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(foreign.schema(), [foreign]))?
            .collect::<yggdryl::Result<_>>()?;
    let event = lifted[0].as_order_event().expect("an order event");
    assert_eq!((event.get_crosscode(), event.get_currunix()), ("10:0:O-1001", 1_700_000_000_000_000_000));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import MarketDataKind, graph

    order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    values = [
        graph.Order(),
        order,
        graph.BookEvent(1_700_000_001_000_000_000, "AAPL"),
        graph.SnapshotEvent.snapshot(order),
    ]

    # Written column by column, read back value for value.
    reader = graph.MarketData.arrow_reader(values)
    assert isinstance(reader, pa.RecordBatchReader)
    read = list(graph.MarketData.from_arrow_reader(reader))
    assert read == [graph.MarketData(value) for value in values]
    # An empty book states its entries - none - and a control states none.
    assert [value.kind for value in read] == ["order", "order_event", "book_event", "snapshot_event"]
    assert [value.marketdatakind for value in read] == [MarketDataKind.ORDR] * 2 + [MarketDataKind.BOOK] * 2

    # The row: 6 element, 9 event, 34 market and 5 operation columns,
    # the book scope, then the five nested columns.
    names = [child.name for child in graph.MarketData.field()]
    assert len(names) == 6 + 9 + 34 + 5 + 1 + 5
    assert names[55:] == ["alive", "deltas", "executions", "bidlimits", "asklimits"]

    # A foreign shape: a few columns in another order, one it does not name.
    foreign = pa.table(
        {
            "crosscode": ["10:0:O-1001"],
            "currunix": pa.array([1_700_000_000_000_000_000], pa.int64()),
            "MarketDataKind": pa.array([int(MarketDataKind.ORDR)], pa.int32()),
            "msgtype": ["D"],
        }
    )
    [lifted] = graph.MarketData.from_arrow_reader(foreign)
    event = lifted.as_order_event()
    assert event is not None and (event.crosscode, event.currunix) == ("10:0:O-1001", 1_700_000_000_000_000_000)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, MarketDataKind, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    const values = [
      new graph.Order(),
      order,
      new graph.BookEvent(1_700_000_001_000_000_000n, 'AAPL'),
      graph.SnapshotEvent.snapshot(order),
    ]

    // Written column by column, read back value for value.
    const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
    assert.equal(read.length, 4)
    read.forEach((value, at) => assert.ok(value.intoLeaf().equals(values[at])))
    // An empty book states its entries - none - and a control states none.
    assert.deepEqual(read.map((value) => value.kind), ['order', 'order_event', 'book_event', 'snapshot_event'])
    assert.deepEqual(read.map((value) => value.marketdatakind), ['ORDR', 'ORDR', 'BOOK', 'BOOK'])

    // The row: 6 element, 9 event, 34 market and 5 operation columns,
    // the book scope, then the five nested columns.
    const field = graph.MarketData.field()
    assert.equal(field.fieldLen, 6 + 9 + 34 + 5 + 1 + 5)
    assert.deepEqual([55, 56, 57, 58, 59].map((at) => field.fieldAt(at).name), [
      'alive', 'deltas', 'executions', 'bidlimits', 'asklimits',
    ])

    // A foreign shape: a few columns in another order, one it does not name.
    const foreign = new arrow.Table({
      crosscode: arrow.vectorFromArray(['10:0:O-1001'], new arrow.Utf8()),
      currunix: arrow.vectorFromArray([1_700_000_000_000_000_000n], new arrow.Int64()),
      MarketDataKind: arrow.vectorFromArray([MarketDataKind.ORDR], new arrow.Int32()),
      msgtype: arrow.vectorFromArray(['D'], new arrow.Utf8()),
    })
    const [lifted] = graph.MarketData.fromArrowReader(BatchReader.from(foreign))
    const event = lifted.asOrderEvent()
    assert.equal(event.crosscode, '10:0:O-1001')
    assert.equal(event.currunix, 1_700_000_000_000_000_000n)
    ```

## Views

`MarketView`, in `graph::view`, names six readings of a `marketdata` stream: one structurally-built [`Plan`](../expression/plans.md) whose text reads back as the same plan.

```text
MarketData::plan(view: &MarketView, lifts: &[FieldPath]) -> Result<Plan>
MarketData::apply_view(view: &MarketView, lifts: &[FieldPath], reader: BatchReader) -> Result<BatchReader>
```

| View | Plan | Rows and columns |
| --- | --- | --- |
| `orders`, `quotes`, `executions` | `select * exclude (alive, deltas, executions, bidlimits, asklimits) where marketdatakind = 'ORDR'`, `'QUOT'` and `'EXEC'` likewise | the category's leaves, undated and dated: the 55 flat columns |
| `trades` | `select * exclude (...), unnest(executions) as execution where marketdatakind = 'TRAD'` | one row per execution, in the trade's held order: the trade's flat columns, then `execution.<column>` per operation-row column |
| `books` | `select * exclude (executions) where marketdatakind = 'BOOK' and alive is not null` | one row per book - a snapshot control states no `alive` - its entries, deltas and levels kept nested; over a table that stores a null list as an empty one a snapshot control's row is kept too, holding no entry, since only the reader can tell it by its identity |
| `lifecycle` | `select * exclude (...) where crosscode = '<crosscode>' order by currunix` | every leaf of one chain in event order, tied instants by arrival; bounded to the one chain the `where` kept (the ordering collects); a chain is named by its stored code - `10:1:O-1001` for an order to buy |

| Key | Rule |
| --- | --- |
| Spellings | `MarketView::ALL` lists them, `as_str`/`Display` write them, `MarketView::read(text, crosscode)` reads one (any case): `lifecycle` requires `crosscode`, every other view refuses one - `InvalidRecord` at `$.view`/`$.crosscode` |
| `apply_view` | exactly the plan's `apply_arrow_reader`, bound once against the reader's schema; composite leaves are laid flat by [`unnest`](../expression/grammar.md#unnest); the literal `'ORDR'` coerces to the `marketdatakind` member where the plan binds |
| Lifts | a [`FieldPath`](../types/paths.md) appended to the view's projections, turning a nested fact into a column: `identifiers['clordid'] as clordid` reads the value the `identifiers` map holds under that key - the stored text, `src:type` or the type alone for the base source, lower case; a key the map lacks, `'FIX:CLORDID'` included, reads null - and the market row's `isincode` column carries the ISIN; a path naming a column the root lacks, or a name a kept column already has, refuses where the plan binds |
| Bindings | Python `MarketData.plan(view, lifts=(), *, crosscode=None)`, `MarketData.apply_view(view, source, lifts=(), *, crosscode=None)` (a lift: `FieldPath` text or object), spellings `enums.MARKET_VIEWS`; JavaScript `graph.MarketData.plan(view, lifts, crosscode)`, `graph.MarketData.applyView(view, reader, lifts, crosscode)`, spellings `enums.marketViews` |

=== "Rust"

    ```rust
    use arrow_array::RecordBatch;
    use yggdryl::graph::{Element, ExecutionEvent, Market, MarketData, MarketView, Operation, OrderEvent, TradeEvent};
    use yggdryl::{FieldPath, IdKey, IdType, Identifier, Plan, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut order = OrderEvent::at(T);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.insert_identifier(Identifier::new(IdKey::base(IdType::ClOrdId), "C-1")?)?;
    order.finalize();
    let fill = |code: &str, side: Side| {
        let mut execution = ExecutionEvent::at(T + 1_000_000_000);
        execution.set_crosscode(code.to_owned());
        execution.set_side(side, true);
        execution
    };
    let mut root = OrderEvent::at(T + 1_000_000_000);
    root.set_crosscode("T-1".to_owned());
    let trade = TradeEvent::from_parts(&root, vec![fill("E-1", Side::Buy), fill("E-2", Side::Sell)])?;
    let stream = || {
        MarketData::arrow_reader([MarketData::from(order.clone()), MarketData::from(trade.clone())], None, None)
    };

    // The orders, one identifier lifted out of the map by its key into a column.
    let lifts: Vec<FieldPath> = vec!["identifiers['clordid'] as clordid".parse()?];
    let orders = MarketData::apply_view(&MarketView::Orders, &lifts, stream()?)?;
    let schema = orders.schema();
    assert_eq!(schema.fields().last().map(|column| column.name().as_str()), Some("clordid"));
    assert!(schema.index_of("executions").is_err(), "a flat view drops the nested columns");
    let held: Vec<RecordBatch> = orders.collect::<Result<_, _>>()?;
    assert_eq!(held.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);

    // The trades, one row per execution beside the trade's own columns.
    let trades = MarketData::apply_view(&MarketView::Trades, &[], stream()?)?;
    assert!(trades.schema().index_of("execution.crosscode").is_ok());
    let held: Vec<RecordBatch> = trades.collect::<Result<_, _>>()?;
    assert_eq!(held.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);

    // A view is a plan, and its text reads back as the same plan.
    let plan = MarketData::plan(&MarketView::read("Trades", None)?, &[])?;
    assert_eq!(plan.to_string().parse::<Plan>()?, plan);
    assert!(plan.to_string().ends_with("where marketdatakind = 'TRAD'"));
    assert!(MarketView::read("lifecycle", None).is_err(), "a lifecycle needs its crosscode");
    // An order's chain is named by its stored code, `10:1:O-1001`.
    let lifecycle = MarketView::read("lifecycle", Some(order.get_crosscode()))?;
    let chain = MarketData::apply_view(&lifecycle, &[], stream()?)?.collect::<Result<Vec<RecordBatch>, _>>()?;
    assert_eq!(chain.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Plan, enums, graph

    T = 1_700_000_000_000_000_000
    order = graph.OrderEvent(
        T, crosscode="O-1001", side="BUYS", identifiers=[Identifier("clordid", "C-1")]
    )
    root = graph.OrderEvent(T + 1_000_000_000, crosscode="T-1")
    trade = graph.TradeEvent.from_parts(
        root,
        [
            graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-1", side="BUYS"),
            graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-2", side="SELL"),
        ],
    )

    def stream():
        return graph.MarketData.arrow_reader([order, trade])

    # The orders, one identifier lifted out of the map by its key into a column.
    orders = graph.MarketData.apply_view("orders", stream(), ["identifiers['clordid'] as clordid"]).read_all()
    assert orders.schema.names[-1] == "clordid"
    assert "executions" not in orders.schema.names
    assert orders.column("clordid").to_pylist() == ["C-1"]

    # The trades, one row per execution beside the trade's own columns.
    trades = graph.MarketData.apply_view("trades", stream()).read_all()
    assert trades.num_rows == 2
    assert sorted(trades.column("execution.crosscode").to_pylist()) == ["8:1:E-1", "8:2:E-2"]

    # A view is a plan, and its text reads back as the same plan.
    plan = graph.MarketData.plan("trades")
    assert Plan(str(plan)) == plan
    assert str(plan).endswith("where marketdatakind = 'TRAD'")
    assert "lifecycle" in enums.MARKET_VIEWS
    # An order's chain is named by its stored code, `10:1:O-1001`.
    chain = graph.MarketData.apply_view("lifecycle", stream(), crosscode="10:1:O-1001").read_all()
    assert chain.column("crosscode").to_pylist() == ["10:1:O-1001"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Plan, enums, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = new graph.OrderEvent(T, {
      crosscode: 'O-1001', side: 'BUYS', identifiers: [new Identifier('clordid', 'C-1')],
    })
    const root = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'T-1' })
    const trade = graph.TradeEvent.fromParts(root, [
      new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-1', side: 'BUYS' }),
      new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-2', side: 'SELL' }),
    ])
    const stream = () => graph.MarketData.arrowReader([order, trade])

    // The orders, one identifier lifted out of the map by its key into a column.
    const orders = graph.MarketData.applyView('orders', stream(), ["identifiers['clordid'] as clordid"]).intoTable()
    const names = orders.schema.fields.map((column) => column.name)
    assert.equal(names[names.length - 1], 'clordid')
    assert.ok(!names.includes('executions'))
    assert.deepEqual([...orders.getChild('clordid')], ['C-1'])

    // The trades, one row per execution beside the trade's own columns.
    const trades = graph.MarketData.applyView('trades', stream()).intoTable()
    assert.equal(trades.numRows, 2)
    assert.deepEqual([...trades.getChild('execution.crosscode')].sort(), ['8:1:E-1', '8:2:E-2'])

    // A view is a plan, and its text reads back as the same plan.
    const plan = graph.MarketData.plan('trades')
    assert.ok(new Plan(plan.toString()).equals(plan))
    assert.ok(plan.toString().endsWith("where marketdatakind = 'TRAD'"))
    assert.ok(enums.marketViews.includes('lifecycle'))
    // An order's chain is named by its stored code, `10:1:O-1001`.
    const chain = graph.MarketData.applyView('lifecycle', stream(), undefined, '10:1:O-1001').intoTable()
    assert.deepEqual([...chain.getChild('crosscode')], ['10:1:O-1001'])
    ```

The [book display](serve.md) applies no view: each of its readings is [one filtered read](serve.md#contract) of a table's `BOOK` rows.

## Edges

- A view's lift naming a column the root does not hold is refused where the plan binds; a key the identifier map lacks reads null, and so does a key not spelled as stored - the lookup compares exactly.
- A row whose `marketdatakind` is `TRAD` decodes only through `TradeEvent::from_parts`; a book row only as the entries it states, never by replaying its deltas.
- A flat view's `BOOK` rows are a report, not a round trip: the view drops `alive`, which tells a book from a snapshot control, and the reader refuses such a row by naming the missing column.
