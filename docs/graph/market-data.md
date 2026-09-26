# Market data

`MarketData` is one value over every leaf, and the lifted `marketdata` Arrow row any of them crosses a boundary as; the column enums name that row's facts, `MarketView` reads it through named plans.

## MarketData

| Key | Rule |
| --- | --- |
| Variants | `Order`, `Quote`, `Execution`, `BookSide`, `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent`, `BookEvent` (boxed) and `SnapshotEvent`, in `graph::market_data` |
| `kind()` | the `MarketKind`, in `graph::kind`: `as_str` = `order`, `quote`, `execution`, `book_side`, `order_event`, `quote_event`, `execution_event`, `trade_event`, `book_event`, `snapshot_event`; `read` = same (any case); `is_event` = one of the six dated leaves, echoed by the value's `is_event()` |
| Conversions | `From<leaf>` builds one from every leaf; `TryFrom<MarketData>` takes it back, else `InvalidRecord` at `$.kind` (names expected/found); `as_order()` ... `as_snapshot_event()` borrow a variant; `book()` = the [control](order.md#book-control) of an operation event or snapshot |
| Traits | `Element`/`Market` delegate to the leaf held, so a resolved boundary reads it generically |
| Order, following, merging | `is_after` orders two dated values by instant, none if either undated; `with_previous`/`merge_with` are the leaf's own for one variant; an operation event also follows another kind via shared facts, keeping its own kind (an execution follows its filled order); a merge never crosses variants |
| Bindings | Python `graph.MarketData(leaf)`, `MarketData.kinds`, `kind`, `is_event`, `as_order_event()` and the rest, `into_leaf()`; JavaScript `new graph.MarketData(leaf)`, `MarketData.kinds()`, `asOrderEvent()`, `intoLeaf()`; `enums.MARKET_KINDS`/`enums.marketKinds` list the spellings |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, MarketData, MarketKind, OrderEvent, QuoteEvent};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.finalize();

    let value = MarketData::from(order.clone());
    assert_eq!(value.kind(), MarketKind::OrderEvent);
    assert_eq!(value.kind().as_str(), "order_event");
    assert_eq!(MarketKind::read("ORDER_EVENT"), Some(MarketKind::OrderEvent));
    assert!(value.is_event());
    assert_eq!(value.as_order_event(), Some(&order));
    assert_eq!(value.get_curruuid(), order.get_curruuid());
    assert!(QuoteEvent::try_from(value.clone()).is_err(), "another kind is refused at $.kind");
    assert_eq!(OrderEvent::try_from(value)?, order);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    value = graph.MarketData(order)
    assert value.kind == "order_event"
    assert "order_event" in graph.MarketData.kinds
    assert value.is_event
    assert value.as_order_event() == order
    assert value.curruuid == order.curruuid
    assert value.as_quote_event() is None, "another kind is none of this value"
    assert value.into_leaf() == order
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    const value = new graph.MarketData(order)
    assert.equal(value.kind, 'order_event')
    assert.ok(graph.MarketData.kinds().includes('order_event'))
    assert.equal(value.isEvent, true)
    assert.ok(value.asOrderEvent().equals(order))
    assert.equal(value.curruuid, order.curruuid)
    assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
    assert.ok(value.intoLeaf() instanceof graph.OrderEvent)
    ```

## Columns

Three enums - `EventColumn` (`graph::column`), `MarketColumn` (`graph::market_column`), `OperationColumn` (`graph::operation_column`) - name every fact the traits answer, one column each, so a [text line](../media/index.md#plain-text)'s batch, a [FIX row](../fix/capture.md#the-crates-own-columns) and a [chained message](../fix/lifecycle.md#a-chain-carries-its-creation-and-its-history) join without mapping.

| Enum | Columns, in `ALL` order |
| --- | --- |
| `EventColumn` (16) | `currunix`, `creaunix`, `execunix`, `recdunix`, `exprtime`, `prevunix`, `snapunix` (nanosecond UTC clocks); `curruuid`, `crossuuid` ([`uuid`](../types/uuid.md)), `crosscode` (`utf8`), `currhashcode`, `crosshashcode` (`uint64`), `prevuuid`, `seqnum` (`uint64`), `srcuuids` (`serie<uuid>`, item `srcuuid`); `state` ([`state`](../types/codes/state.md#the-rank-leads)) |
| `MarketColumn` (19) | `price`, `currency`, `quantity`, `unit`, `side`, `securityids`, `cficode`, `miccode`, `lastpx`, `lastqty`, `avgpx`, `cumqty`, `leavesqty`, `prevpx`, `prevqty`, `spotrate`, `forwardpoints`, `ticker`, `metadata`: numbers are [`decimal`](../types/numeric/decimal.md#decimal), each [code](../types/codes/index.md) its own leaf (`ccy`, `unit`, `side`, `cfi`, `mic`), a sorted `map<utf8, utf8>` for identifiers/metadata, `utf8` for the ticker |
| `OperationColumn` (8) | `marketoperationid` (`int32`), `tif` (`timeinforce`), `tradable` (`boolean`), `accountids`, `userids`, `altids` (each `IdMap::dtype()`, sorted `map<utf8, utf8>`), `bid`, `ask` (each `struct<price, spotrate, forwardpoints, currency, quantity, unit>`, `OperationColumn::LANE_FIELDS`) |
| Book control (5) | `BookRef` as a row: `mdupdateaction` (`utf8`, action's `as_str`), `bookscope` (`utf8`), `mdentrypositionno` (`uint32`), `mdentrypx`, `mdentrysize` (`decimal`); all null on a non-market-data-entry operation |

| Key | Rule |
| --- | --- |
| Verbs | each enum answers `ALL`, `name`, `display`, `datatype`, `nullable`, `field`, `fields`, `of_name` (any case), `fact` (what an element states, nothing if none), `record` (states a cell back: null clears it, unreadable leaves it unchanged); `EventColumn` also `description`, each taking its trait: `Event`/`Market`/`Operation` |
| Nullability | never null: `currunix`, `curruuid`, `crossuuid`, `currhashcode`, `crosshashcode`, `currency`, `unit`, `side`; every other column is null where nothing is stated (empty code/serie/map, zero place, absent instant); `state` also admits null - no neutral member for an empty cell |
| Order | `ALL` is the canonical order `fields()` and event-native schemas use; a FIX row holds the same columns through the crate's own fields, each at its datatype, in protocol-oriented time/identity bands rather than reordered around `ALL` |
| Round trip | a line read back from its batch, a message from its row, restate all sixteen event columns - identities and clocks survive |
| Bindings | Python `enums.EVENT_COLUMNS`, `MARKET_COLUMNS`, `OPERATION_COLUMNS`; JavaScript `enums.eventColumns`, `marketColumns`, `operationColumns`; column verbs are Rust-only |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, EventColumn, Market, MarketColumn, MarketData, OperationColumn, OrderEvent};
    use yggdryl::Side;

    assert_eq!((EventColumn::ALL.len(), MarketColumn::ALL.len(), OperationColumn::ALL.len()), (16, 19, 8));
    assert_eq!((EventColumn::ALL[0].name(), EventColumn::ALL[15].name()), ("currunix", "state"));
    assert!(!EventColumn::CurrUuid.nullable() && !MarketColumn::Side.nullable());

    // The lifted row states them in that order, behind its `kind`.
    let field = MarketData::field()?;
    let names: Vec<&str> = field.fields()[1..44].iter().map(|child| child.name()).collect();
    let listed: Vec<&str> = EventColumn::ALL.map(EventColumn::name).into_iter()
        .chain(MarketColumn::ALL.map(MarketColumn::name))
        .chain(OperationColumn::ALL.map(OperationColumn::name))
        .collect();
    assert_eq!(names, listed);

    // What an element states under a column, and the same fact stated back.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.finalize();
    let code = EventColumn::of_name("CROSSCODE").and_then(|column| column.fact(&order)).expect("a code");
    let side = MarketColumn::Side.fact(&order).expect("a side");
    let mut again = OrderEvent::at(1_700_000_000_000_000_000);
    EventColumn::CrossCode.record(&mut again, &code);
    MarketColumn::Side.record(&mut again, &side);
    assert_eq!((again.get_crosscode(), again.get_side()), ("O-1001", Side::Buy));
    // Nothing stated is a null.
    assert_eq!(EventColumn::PrevUuid.fact(&order), None);
    assert_eq!(MarketColumn::Price.fact(&order), None);
    ```

=== "Python"

    ```python
    from yggdryl import enums, graph

    assert (len(enums.EVENT_COLUMNS), len(enums.MARKET_COLUMNS), len(enums.OPERATION_COLUMNS)) == (16, 19, 8)
    assert (enums.EVENT_COLUMNS[0], enums.EVENT_COLUMNS[-1]) == ("currunix", "state")

    # The lifted row states them in that order, behind its `kind`.
    names = [child.name for child in graph.MarketData.field()]
    assert names[1:44] == [*enums.EVENT_COLUMNS, *enums.MARKET_COLUMNS, *enums.OPERATION_COLUMNS]
    assert names[44:49] == ["mdupdateaction", "bookscope", "mdentrypositionno", "mdentrypx", "mdentrysize"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { enums, graph } = require('yggdryl')

    assert.deepEqual([enums.eventColumns.length, enums.marketColumns.length, enums.operationColumns.length], [16, 19, 8])
    assert.deepEqual([enums.eventColumns[0], enums.eventColumns[15]], ['currunix', 'state'])

    // The lifted row states them in that order, behind its `kind`.
    const field = graph.MarketData.field()
    const names = Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at).name)
    assert.deepEqual(names.slice(1, 44), [...enums.eventColumns, ...enums.marketColumns, ...enums.operationColumns])
    assert.deepEqual(names.slice(44, 49), ['mdupdateaction', 'bookscope', 'mdentrypositionno', 'mdentrypx', 'mdentrysize'])
    ```

## Arrow

| Key | Rule |
| --- | --- |
| `MarketData::field()` | in `graph::arrow`: required `marketdata` struct, 59 columns - required `kind` (`MarketKind` spelling); `EventColumn::ALL` (nullable here: an undated leaf has no clock); `MarketColumn::ALL`; `OperationColumn::ALL`; the five book-control columns; three derived facts - `spread` (`decimal`, as [`BookEvent`'s](book.md#limits)), `crossed`/`locked` (`boolean`); then the nullable nested columns |
| Nested columns | `executions`: operation-row serie a trade/book fills; `bidside`/`askside`: six element facts + `MarketColumn::ALL` + `live`/`deltas` operation-row series + `limits` ([`Limit::field()`](book.md#limits) serie); `snapshotpartitions`: `snapshotpartition` structs (nullable `symbol`, required `scope`), absent after an ordinary update; side's own `live`/`deltas`/`limits`. Operation row = `kind`+16+19+8+5 = 49; side = 6+19+3 = 28 |
| Rows | every fact is its own typed column, null where absent; a trade flattens into a book's `executions`; a book's `bid`/`ask` lanes state its two bests (`price`/`quantity`: side's best price/aggregate quantity, null if none) - same facts as a side row's own, so no `bestpx`/`bestqty` repeats them; book-added columns nullable in every role, `limits` empty for an empty side; every decimal is the registered [`decimal`](../types/numeric/decimal.md#decimal) |
| `arrow_reader(values, batch_row_size, batch_byte_size)` | streams `IntoIterator` of `MarketData`/`Result<MarketData>` into bounded `BatchReader` batches, lazily, column by column, no per-row `Scalar`; no row bound = shared default, zero = one row; a byte bound closes a nonempty batch once reached; values write only as their canonical self - stale derived facts refused at their row; a source/refusal error follows the completed prefix, fuses the reader |
| `from_arrow_reader(batches)` | one value per row, tolerant of shape: root columns resolved by name once per stream (any case, subset, order); an unnamed column ignored; a castable column cast via one plan compiled before the first batch; two columns naming one fact refused before a row is read |
| Decoding | a row's `kind` names its leaf; a missing/unknown kind, or a required fact - `currunix` for a dated leaf, a trade's `executions`, a book's two sides - is refused at the first row needing it; each batch lands once as one record [`Serie`](../types/serie.md), so no cell decodes twice; a refused value is named by row and path - `$[0].bidside.live[0].miccode` - before any leaf is rebuilt |
| Canonical rows | a trade rebuilds only via `TradeEvent::from_parts`, a book's live depth/deltas directly, never by replaying deltas; every stated identity must match the rebuilt leaf's (null `curruuid`/`crossuuid`/`currhashcode`/`crosshashcode` refused, absent = nothing); every other stated fact - lanes, its three facts, a side's limits - must match the leaf, else refused with `differs`; a reader failure or refused row returns once, fuses the iterator |
| Bindings | Python `graph.MarketData.field()`, `arrow_reader(items, batch_row_size=None, batch_byte_size=None)` - a `pyarrow.RecordBatchReader` - and `from_arrow_reader(source)`; JavaScript `graph.MarketData.field()`, `arrowReader(items, batchRowSize, batchByteSize)` - a `BatchReader` - and `fromArrowReader(reader)`, any source `BatchReader.from` accepts |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{BookEvent, Element, Event, MarketData, Order, OrderEvent};

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
    ];

    // Written column by column, read back value for value.
    let batches: Vec<RecordBatch> =
        MarketData::arrow_reader(values.clone(), None, None)?.collect::<Result<_, _>>()?;
    let read: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
            .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read, values);

    // The row: kind, 16 event, 19 market, 8 operation and 5 book-control
    // columns, the three book facts, then the seven nested columns.
    let field = MarketData::field()?;
    assert_eq!(field.field_len(), 1 + 16 + 19 + 8 + 5 + 3 + 7);
    let facts: Vec<&str> = field.fields()[49..52].iter().map(|child| child.name()).collect();
    assert_eq!(facts, ["spread", "crossed", "locked"]);

    // A foreign shape: a few columns in another order, one it does not name.
    let written = MarketData::arrow_reader([MarketData::from(order.clone())], None, None)?
        .next()
        .expect("one batch")?;
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();
    for name in ["crosscode", "currunix", "kind"] {
        let index = written.schema().index_of(name)?;
        fields.push(written.schema().field(index).clone());
        columns.push(written.column(index).clone());
    }
    fields.push(Field::new("msgtype", DataType::Utf8, true));
    columns.push(Arc::new(StringArray::from(vec!["D"])));
    let foreign = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)?;
    let lifted: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(foreign.schema(), [foreign]))?
            .collect::<yggdryl::Result<_>>()?;
    let event = lifted[0].as_order_event().expect("an order event");
    assert_eq!((event.get_crosscode(), event.get_currunix()), ("O-1001", 1_700_000_000_000_000_000));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import graph

    order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    values = [graph.Order(), order, graph.BookEvent(1_700_000_001_000_000_000, "AAPL")]

    # Written column by column, read back value for value.
    reader = graph.MarketData.arrow_reader(values)
    assert isinstance(reader, pa.RecordBatchReader)
    read = list(graph.MarketData.from_arrow_reader(reader))
    assert read == [graph.MarketData(value) for value in values]

    # The row: kind, 16 event, 19 market, 8 operation and 5 book-control
    # columns, the three book facts, then the seven nested columns.
    names = [child.name for child in graph.MarketData.field()]
    assert len(names) == 1 + 16 + 19 + 8 + 5 + 3 + 7
    assert names[49:52] == ["spread", "crossed", "locked"]

    # A foreign shape: a few columns in another order, one it does not name.
    foreign = pa.table(
        {
            "crosscode": ["O-1001"],
            "currunix": pa.array([1_700_000_000_000_000_000], pa.int64()),
            "kind": ["order_event"],
            "msgtype": ["D"],
        }
    )
    [lifted] = graph.MarketData.from_arrow_reader(foreign)
    event = lifted.as_order_event()
    assert event is not None and (event.crosscode, event.currunix) == ("O-1001", 1_700_000_000_000_000_000)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    const values = [new graph.Order(), order, new graph.BookEvent(1_700_000_001_000_000_000n, 'AAPL')]

    // Written column by column, read back value for value.
    const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
    assert.equal(read.length, 3)
    read.forEach((value, at) => assert.ok(value.intoLeaf().equals(values[at])))

    // The row: kind, 16 event, 19 market, 8 operation and 5 book-control
    // columns, the three book facts, then the seven nested columns.
    const field = graph.MarketData.field()
    assert.equal(field.fieldLen, 1 + 16 + 19 + 8 + 5 + 3 + 7)
    assert.deepEqual([49, 50, 51].map((at) => field.fieldAt(at).name), ['spread', 'crossed', 'locked'])

    // A foreign shape: a few columns in another order, one it does not name.
    const foreign = new arrow.Table({
      crosscode: arrow.vectorFromArray(['O-1001'], new arrow.Utf8()),
      currunix: arrow.vectorFromArray([1_700_000_000_000_000_000n], new arrow.Int64()),
      kind: arrow.vectorFromArray(['order_event'], new arrow.Utf8()),
      msgtype: arrow.vectorFromArray(['D'], new arrow.Utf8()),
    })
    const [lifted] = graph.MarketData.fromArrowReader(BatchReader.from(foreign))
    const event = lifted.asOrderEvent()
    assert.equal(event.crosscode, 'O-1001')
    assert.equal(event.currunix, 1_700_000_000_000_000_000n)
    ```

## Views

`MarketView`, in `graph::view`, names seven readings of a `marketdata` stream: one structurally-built [`Plan`](../expression/plans.md) whose text reads back as the same plan.

```text
MarketData::plan(view: &MarketView, lifts: &[FieldPath]) -> Result<Plan>
MarketData::apply_view(view: &MarketView, lifts: &[FieldPath], reader: BatchReader) -> Result<BatchReader>
```

| View | Plan | Rows and columns |
| --- | --- | --- |
| `orders`, `quotes`, `executions` | `select * exclude (executions, bidside, askside, snapshotpartitions, live, deltas, limits) where kind in ('order', 'order_event')`, the quote and execution kinds likewise | the kind's leaves: `kind` + 16 event + 19 market + 8 operation + 5 book-control columns, `spread`, `crossed`, `locked` |
| `trades` | `select * exclude (...), unnest(executions) as execution where kind = 'trade_event'` | one row per execution, trade's held order: trade's flat columns, then `execution.<column>` per operation-row column |
| `book_sides` | `select currunix, snapunix, unnest([bidside, askside]) as side where kind = 'book_event'` | two rows per book (bid, ask): `currunix`, `snapunix`, `side.<column>` - 6 element + 19 market columns, `side.live`/`deltas`/`limits` |
| `books` | `select * exclude (executions, live, deltas, limits) where kind = 'book_event'` | one row per book, its sides and partitions kept nested |
| `lifecycle` | `select * exclude (...) where crosscode = '<crosscode>' order by currunix` | every leaf of one chain in event order, tied instants by arrival; bounded to the one chain the `where` kept (the ordering collects) |

| Key | Rule |
| --- | --- |
| Spellings | `MarketView::ALL` lists them, `as_str`/`Display` write them, `MarketView::read(text, crosscode)` reads one (any case): `lifecycle` requires `crosscode`, every other view refuses one - `InvalidRecord` at `$.view`/`$.crosscode` |
| `apply_view` | exactly the plan's `apply_arrow_reader`, bound once against the reader's schema; composite leaves are laid flat by [`unnest`](../expression/grammar.md#unnest) |
| Lifts | a [`FieldPath`](../types/paths.md) appended to the view's projections, turning a nested fact into a column: `securityids['ISIN'] as isin` matches the stored key exactly (upper-case; `'isin'` finds nothing), null if absent; a path naming a column the root lacks, or a name a kept column already has, refuses where the plan binds |
| Bindings | Python `MarketData.plan(view, lifts=(), *, crosscode=None)`, `MarketData.apply_view(view, source, lifts=(), *, crosscode=None)` (a lift: `FieldPath` text or object), spellings `enums.MARKET_VIEWS`; JavaScript `graph.MarketData.plan(view, lifts, crosscode)`, `graph.MarketData.applyView(view, reader, lifts, crosscode)`, spellings `enums.marketViews` |

=== "Rust"

    ```rust
    use arrow_array::RecordBatch;
    use yggdryl::graph::{Element, ExecutionEvent, Market, MarketData, MarketView, OrderEvent, TradeEvent};
    use yggdryl::{FieldPath, Plan, SecType, SecurityId, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut order = OrderEvent::at(T);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
    order.finalize();
    let fill = |code: &str, side: Side| {
        let mut execution = ExecutionEvent::at(T + 1_000_000_000);
        execution.set_crosscode(code.to_owned());
        execution.set_side(side);
        execution
    };
    let mut root = OrderEvent::at(T + 1_000_000_000);
    root.set_crosscode("T-1".to_owned());
    let trade = TradeEvent::from_parts(&root, vec![fill("E-1", Side::Buy), fill("E-2", Side::Sell)])?;
    let stream = || {
        MarketData::arrow_reader([MarketData::from(order.clone()), MarketData::from(trade.clone())], None, None)
    };

    // The orders, the ISIN lifted out of the identifiers into a column.
    let lifts: Vec<FieldPath> = vec!["securityids['ISIN'] as isin".parse()?];
    let orders = MarketData::apply_view(&MarketView::Orders, &lifts, stream()?)?;
    let schema = orders.schema();
    assert_eq!(schema.fields().last().map(|column| column.name().as_str()), Some("isin"));
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
    assert!(MarketView::read("lifecycle", None).is_err(), "a lifecycle needs its crosscode");
    let lifecycle = MarketView::read("lifecycle", Some("O-1001"))?;
    let chain = MarketData::apply_view(&lifecycle, &[], stream()?)?.collect::<Result<Vec<RecordBatch>, _>>()?;
    assert_eq!(chain.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import Plan, enums, graph

    T = 1_700_000_000_000_000_000
    order = graph.OrderEvent(T, crosscode="O-1001", side="BUY", securityids={"ISIN": "US0378331005"})
    root = graph.OrderEvent(T + 1_000_000_000, crosscode="T-1")
    trade = graph.TradeEvent.from_parts(
        root,
        [
            graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-1", side="BUY"),
            graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-2", side="SELL"),
        ],
    )

    def stream():
        return graph.MarketData.arrow_reader([order, trade])

    # The orders, the ISIN lifted out of the identifiers into a column.
    orders = graph.MarketData.apply_view("orders", stream(), ["securityids['ISIN'] as isin"]).read_all()
    assert orders.schema.names[-1] == "isin"
    assert "executions" not in orders.schema.names
    assert orders.column("isin").to_pylist() == ["US0378331005"]

    # The trades, one row per execution beside the trade's own columns.
    trades = graph.MarketData.apply_view("trades", stream()).read_all()
    assert trades.num_rows == 2
    assert sorted(trades.column("execution.crosscode").to_pylist()) == ["E-1", "E-2"]

    # A view is a plan, and its text reads back as the same plan.
    plan = graph.MarketData.plan("trades")
    assert Plan(str(plan)) == plan
    assert "lifecycle" in enums.MARKET_VIEWS
    chain = graph.MarketData.apply_view("lifecycle", stream(), crosscode="O-1001").read_all()
    assert chain.column("crosscode").to_pylist() == ["O-1001"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Plan, enums, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = new graph.OrderEvent(T, {
      crosscode: 'O-1001', side: 'BUY', securityids: { ISIN: 'US0378331005' },
    })
    const root = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'T-1' })
    const trade = graph.TradeEvent.fromParts(root, [
      new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-1', side: 'BUY' }),
      new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-2', side: 'SELL' }),
    ])
    const stream = () => graph.MarketData.arrowReader([order, trade])

    // The orders, the ISIN lifted out of the identifiers into a column.
    const orders = graph.MarketData.applyView('orders', stream(), ["securityids['ISIN'] as isin"]).intoTable()
    const names = orders.schema.fields.map((column) => column.name)
    assert.equal(names[names.length - 1], 'isin')
    assert.ok(!names.includes('executions'))
    assert.deepEqual([...orders.getChild('isin')], ['US0378331005'])

    // The trades, one row per execution beside the trade's own columns.
    const trades = graph.MarketData.applyView('trades', stream()).intoTable()
    assert.equal(trades.numRows, 2)
    assert.ok(trades.schema.fields.some((column) => column.name === 'execution.crosscode'))

    // A view is a plan, and its text reads back as the same plan.
    const plan = graph.MarketData.plan('trades')
    assert.ok(new Plan(plan.toString()).equals(plan))
    assert.ok(enums.marketViews.includes('lifecycle'))
    const chain = graph.MarketData.applyView('lifecycle', stream(), undefined, 'O-1001').intoTable()
    assert.deepEqual([...chain.getChild('crosscode')], ['O-1001'])
    ```

## Edges

- A view's lift naming a column the root does not hold is refused where the plan binds; a key the identifiers lack reads null, and so does any key not spelled in its canonical upper case - the key is matched exactly.
- A row whose `kind` is `trade_event` decodes only through `TradeEvent::from_parts`; a book row only as the sides it states, never by replaying its deltas.
