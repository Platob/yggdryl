# Arrow

A capture already in Arrow is read where it sits: `FixCodec::parse_text_arrow_reader` takes the batches a text reader answers - one row per line, the frame in the payload column - through the same [codec](capture.md#a-reader-is-the-whole-parse-surface) a captured line goes through, and returns the crate's one batch reader, [one row per message](#one-row-per-message) - so a day of session log reaches Parquet or Iceberg with nothing here knowing what either is. Its twin, `lifecycle_arrow_reader`, [chains](lifecycle.md) batches of FIX rows the way `lifecycle` chains a stream of messages. Both compose two converters any stage composes the same way - `messages`, rows to messages, and `arrow_reader`, messages to rows - and `write_arrow_reader` writes the rows back to the wire. The market twin, `book_arrow_reader`, expands sorted FIX messages into typed operations, folds them into books and streams the canonical nested book schema without exposing an intermediate collection; `market_data` is the [sorted door](#fix-market-books) that orders a capture's operations for that fold.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::parse_text_arrow_reader`, `lifecycle_arrow_reader`, `messages`, `arrow_reader`, `book_arrow_reader`, `market_data`, `market_arrow_reader`, `market_data_arrow_reader`, `write_arrow_reader`, `FixCodec::DEFAULT_BATCH_BYTE_SIZE`, `DEFAULT_PAYLOAD_COLUMN`, `SOH` |
| Returns | `BatchReader`, the one type every encoding in the crate returns; Python gets a `pyarrow.RecordBatchReader`, JavaScript a `BatchReader` |
| Schema | answered before the first row is read, from the source's schema and the [dictionary](registry.md) alone, never from the data; `lifecycle_arrow_reader` answers the schema it read, `arrow_reader` the one it was given, `book_arrow_reader` the graph's lifted [`MarketData::field()`](../graph/market-data.md#arrow), one row per book, its `marketdatakind` `BOOK`, and `market_arrow_reader` and `market_data_arrow_reader` the same field, one row per operation. `fix_schema(&registry, "fix")` answers the [fixed row](capture.md#the-columns-are-the-folded-names): 150 columns |
| Order | the element, event, market and operation columns every generated schema opens with lead the row, the source's own columns follow them, and the rest of the [fixed columns](capture.md#the-columns-are-the-folded-names) close it |
| Clash | a carried column whose folded name a FIX column takes is dropped in front and lands in that column, never renamed and never duplicated |
| Rows | one row per message, never one per line: a line carrying two frames is two rows, a message the parse [split another off](message.md#a-parse-splits-what-a-message-reports) is followed by its row, a JSON document is one row holding an `unknown` message with no entries, a payload that would not parse is one row holding an empty message, and a line carrying no message at all is no row - [what a line carries](decode.md) is the codec's rule; a row's carried source columns repeat over every message it answers |
| Batches | closed by estimated landed bytes against the codec's `batch_byte_size`, `DEFAULT_BATCH_BYTE_SIZE` (128 MiB), or by rows against `batch_row_size`, `DEFAULT_BATCH_ROW_SIZE` (32,768), whichever it reaches first, unless pinned: several small input batches accumulate into one, one larger than the target splits by rows in proportion, and a batch always holds at least one row |
| Pins | on the codec, for the whole run: `with_payload_column`, `with_capture_names`, `with_separator`, `with_null_values`, `try_with_direction`, `try_with_default_sending_time`, `with_batch_byte_size`, `with_batch_row_size`, `with_threads`, `with_include_msgtypes`, `with_exclude_msgtypes`, `with_snapshot_ns`, `with_sorted_lifecycle`, `with_official_time_delay_ms`, `with_dedup_window_ms`, `with_market_metadata`; no dialect pin, because the registry is one namespace, and no version pin, because a version is what a line said |
| Stages | a call, never a flag: `FixCodec::lifecycle` composes over `messages` and `arrow_reader`, and `lifecycle_arrow_reader` is the walk composed for you; `book_arrow_reader` composes `FixMarketIterator`, `BookIterator` - under its `filter`, where given - and `MarketData::arrow_reader`, one `BOOK` row per book, but does not infer or apply lifecycle enrichment; `market_data` admits what `book_arrow_reader` admits and the executions besides, and sorts the operations, `market_arrow_reader` writes them and `market_data_arrow_reader` reads them from FIX rows, none of them running the lifecycle either; Rust-only `FixDedup` drops an adjacent republication, while lifecycle keeps finite-capture history and yields each identity once within its `dedup_window_ms` in [Lifecycle](lifecycle.md) |
| Doors | capture Arrow parsing pools its whole input by batch, then merges rows in source order under the output byte and row closing targets; `arrow_reader` and `book_arrow_reader` stream; `lifecycle` collects a finite capture to sort its history before it emits chained messages, and `market_data` one to sort its operations; only a source's own failure is an error item |
| Per row | `beginstring` and `msgdirection` are parameters read from the row; a `sourceurl` column is neither a parameter nor a fill - it is [the capture's own](message.md#a-row-is-a-message-again) and is restated onto the output row from the source row; any other column named after a field - `msgpluginid` among them - fills it where the message did not state it; a capture `timestamp` is carried context, never a FIX clock |
| Errors | typed I/O and schema failures, a schema that makes no FIX row first among them, refused before a row is read; what the data refuses is left out with a [warning](capture.md#warnings) and the rest read - a source batch of another schema than the first, a `state` code no member takes, a message the row will not hold; a source's own failure is yielded after the completed prefix, in source order under pooled parsing, and fuses the reader; dropping a pooled reader joins dispatched work |
| Lazy | one worker parses with no pool; capture Arrow parsing pools batches and the row ranges a large one is cut into, while line, message and write doors retain bounded 64-row chunks, two per worker; lifecycle retains finite-capture history |
| Wire | `write_arrow_reader` rebuilds every semantic message from its projected columns, its residual `fixentries` - its `0:<key>` entries, the keys no dictionary resolved that an identifier map holds, included - and the keys its `metadata` holds that no dictionary resolved; a batch without that residual column is refused before a row is read |
| Bindings | Rust; Python (`FixCodec.parse_text_arrow_reader`, `lifecycle_arrow_reader`, `messages`, `arrow_reader`, `book_arrow_reader`, `market_data`, `market_arrow_reader`, `market_data_arrow_reader`, `write_arrow_reader`, and the `market_metadata=` keyword); JavaScript (`parseTextArrowReader`, `lifecycleArrowReader`, `messages`, `arrowReader`, `bookArrowReader`, `marketData`, `marketArrowReader`, `marketDataArrowReader`, `writeArrowReader`, and `{ marketMetadata }`); `FixDedup` is Rust-only |

## Use

One column of frames in, batches out, the capture's own columns carried right after the shared columns every row opens with.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar, Serie, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);

    // A capture shaped the way a log reader shapes one: where the line was
    // read from, which line it was, and the frame itself.
    let capture = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?)
    .required_field("line");
    let row = Scalar::from_sequence([
        Scalar::from("file:///capture.log"),
        Scalar::from(7_i64),
        Scalar::from("recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|"),
    ]);
    let batch = Serie::from_scalars(capture, [row])?.into_arrow_batch()?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

    let read = FixCodec::new(registry)
        .with_threads(4)
        .parse_text_arrow_reader(source)?;

    // The schema is answered before a row is read: the shared element, event,
    // market and operation columns lead it, the capture follows them, the
    // tags follow it, and the arrival record closes it.
    let columns: Vec<String> = read
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().clone())
        .collect();
    assert_eq!(columns[0], "curruuid");
    let at = columns.iter().position(|name| name == "url").expect("the capture's url");
    assert_eq!(columns[at - 1], "partyids");
    assert_eq!(&columns[at..at + 3], ["url", "rownum", "body"]);
    assert_eq!(columns.last().map(String::as_str), Some("fixentries"));

    let rows: usize = read.map(|batch| batch.expect("a batch").num_rows()).sum();
    assert_eq!(rows, 1, "one ordinary frame per input row");
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    import pyarrow as pa

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())

    # A capture shaped the way a log reader shapes one: where the line was
    # read from, which line it was, the clock and the session its header
    # stated, and the frame itself.
    clocks = [
        datetime(2026, 8, 21, 10, 30, 0, 415655, tzinfo=timezone.utc),
        datetime(2026, 8, 21, 10, 30, 1, 2000, tzinfo=timezone.utc),
    ]
    capture = pa.table(
        {
            "url": ["file:///capture.log"] * 2,
            "rownum": pa.array([7, 8], pa.int64()),
            "timestamp": pa.array(clocks, pa.timestamp("us", "UTC")),
            "bridgesessionid": pa.array(["0123abcd", None], pa.utf8()),
            "body": pa.array(
                [
                    "recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|",
                    "8=FIX.4.4|35=D|11=ORDER-2|55=MSFT|10=0|",
                ],
                pa.string(),
            ),
        }
    )

    codec = FixCodec(registry, threads=4)
    read = codec.parse_text_arrow_reader(capture.to_reader())

    # The schema is answered before a row is read: the shared element, event,
    # market and operation columns lead it, the capture follows them, less the
    # column named after a FIX column, the message's columns follow, and the
    # arrival record closes it.
    columns = [field.name for field in read.schema]
    assert columns[0] == "curruuid"
    at = columns.index("url")
    assert columns[at - 1] == "partyids"
    assert columns[at : at + 5] == ["url", "rownum", "timestamp", "bridgesessionid", "body"]
    assert columns[-1] == "fixentries"

    held = read.read_all()
    assert held.num_rows == 2, "one ordinary frame per input row"
    assert held.column("symbol").to_pylist() == ["AAPL", "MSFT"]
    assert held.column("url")[0].as_py() == "file:///capture.log"
    # The verb in front of the frame beats the direction the codec defaults
    # to; both are codes of tag 385's set.
    assert held.column("msgdirection").to_pylist() == ["R", "S"]
    # The capture clock rides along as context and never dates the message;
    # a capture named after a field fills it - where the row stated one.
    assert held.column("timestamp").cast(pa.timestamp("us", "UTC")).to_pylist() == clocks
    assert held.column("currunix").null_count == 0
    assert held.column("bridgesessionid").to_pylist() == ["0123abcd", None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))

    // A capture shaped the way a log reader shapes one: where the line was
    // read from, which line it was, and the frame itself.
    const capture = new arrow.Table({
      url: arrow.vectorFromArray(['file:///capture.log'], new arrow.Utf8()),
      rownum: arrow.vectorFromArray([7n], new arrow.Int64()),
      body: arrow.vectorFromArray(
        ['recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|'],
        new arrow.Utf8(),
      ),
    })

    const read = new fix.FixCodec(registry, { threads: 4 }).parseTextArrowReader(BatchReader.from(capture))

    // The schema is answered before a row is read: the shared element, event,
    // market and operation columns lead it, the capture follows them, the
    // tags follow it, and the arrival record closes it.
    const columns = Array.from({ length: read.field.fieldLen }, (_, index) => read.field.fieldAt(index).name)
    assert.equal(columns[0], 'curruuid')
    const at = columns.indexOf('url')
    assert.equal(columns[at - 1], 'partyids')
    assert.deepEqual(columns.slice(at, at + 3), ['url', 'rownum', 'body'])
    assert.equal(columns[columns.length - 1], 'fixentries')

    const held = read.intoTable()
    assert.equal(held.numRows, 1, 'one ordinary frame per input row')
    assert.equal(held.getChild('symbol').get(0), 'AAPL')
    assert.equal(held.getChild('url').get(0), 'file:///capture.log')
    ```

## FIX market books

`FixCodec::book_arrow_reader(messages, snapshot_millis, filter)` is the centralized Rust path from semantic messages to Arrow books. The composed reader admits what a book states - orders, quotes and `W`/`X` into its sides, and every execution among its deltas, the kinds [`MarketDataKind::is_recorded`](../types/enum/marketdatakind.md#sided-kinds-and-batches) admits - and ignores every other record before it is expanded: administration, requests, an acknowledgement of an execution, trades and batches. A fill moves a book through its order's or quote's report, which the parse [split off](message.md#a-parse-splits-what-a-message-reports) the execution, so the execution stands among the deltas of its instant and moves nothing, and a book message's trade entries (`269=2`) are recorded as the executions they are by the same rule; a venue's acknowledgement, cancel, reject, expiry or replace is its order's report and moves the entry it names. A quote is one entry, resting on each leg it states. Ignored records do not advance book time. `FixMarketIterator` lazily turns each admitted sorted message into `MarketData` values - one direct `OrderEvent` or `QuoteEvent`, or the operations and the `SnapshotEvent` of a `W` / `X` market-data group - while retaining at most that message's expansion; an operation dated before one it already yielded is yielded all the same, with a warning, and the book leaves it out. [`BookIterator`](../graph/book.md#book-fold) applies those operations atomically by effective timestamp, one book per [book key](../graph/book.md#books-by-key) - the instrument's ISIN, whatever its rank, else its ticker, else `XX0000000000`, the number that states none - and [`MarketData::arrow_reader`](../graph/market-data.md#arrow) writes each emitted book as one row of `MarketData::field()`, its `marketdatakind` `BOOK`, closing batches under the codec's row and byte limits. `filter`, where given, is an expression [`Filter`](../expression/filters.md) over that row, bound once: it narrows what the books fold and never admits a kind the rule above prunes, and one naming a column the row does not carry, or answering anything but a boolean, is refused when the reader is built. Nothing a message states fails the door: what cannot stand is [warned about](capture.md#warnings) and left out or defaulted, as [the message](message.md#market-data) and [the book fold](../graph/book.md#book-fold) say; only a source failure is an error item, once after the completed book prefix, and the reader fuses. The call deliberately does not run [`lifecycle`](lifecycle.md): pass enriched messages when predecessor state is required.

Every booked input is a delta. A book row is emitted where its instant applied a delta, or at a snapshot tick where the book holds a live entry - a snapshot emptying a book is emitted too, empty and complete - and states the deltas its instant applied, in the order applied, each a row of the same market data shape, beside its best tradable bid and ask as its [`bidpx`, `bidqty`, `askpx` and `askqty`](../graph/market.md#bid-and-ask). Only a snapshot tick emits a complete book, nesting its live entries and each side's [price levels](../graph/book.md#limits), best first: every grid tick a positive `snapshot_millis` crosses, and a snapshot input - a `W` full refresh, an empty `W`, inputs stating `snapunix`. Every other book is a delta book: its `alive`, `bidlimits` and `asklimits` cells are null, its `prevuuid` names the book it follows - none for a book key's first book - and [`with_previous`](../graph/book.md#book-fold) over the complete book before it rebuilds it whole. With `snapshot_millis = 0` and no `W`, every book is a delta book. A book row's `executions` cell is null. Python exposes the same path as `book_arrow_reader(messages, snapshot_millis=0, filter=None) -> pyarrow.RecordBatchReader` and JavaScript as `bookArrowReader(messages, snapshotMillis = 0, filter = undefined) -> BatchReader`, `filter` a `Filter`, a `Term` or the text of a predicate; both read back as typed books by `graph.MarketData.from_arrow_reader` / `graph.MarketData.fromArrowReader`. Neither binding reimplements the split, operation conversion, matching, book summaries or Arrow encoding.

`FixCodec::market_data(messages)` is the sorted door. It collects a finite capture, admits what `book_arrow_reader` admits and the executions besides - a trade as the executions its parse split off - which stay market data of their own, expands each admitted message into its leaves as [`into_market_data`](message.md#market-data) does - each carrying its message's unmapped fields where the codec's `market_metadata` says so - and stably sorts the operations by the instant each stands at, the snapshot instant a walk stated else the event's own, which is the key `FixMarketIterator` and [`BookIterator`](../graph/book.md#book-fold) check. Sorting the operations rather than the messages is what places an entry whose own clock stands before an earlier message's, so the answer never regresses. Nothing a message states fails the capture: a message its intake refused for what it states is left out with a warning. A source failure ends the capture where it happens - yielded after the operations of every message before it, no later message read - and the iterator is fused. `market_arrow_reader(messages)` writes those operations as `MarketData::field()` rows under the codec's row and byte bounds, every row of the capture read before a source failure and then that failure, and `market_data_arrow_reader(source)` is its twin over batches of FIX rows, refusing a schema that makes no root before a row is read. The twin reads each row as its own message, its market facts derived from what the row states, so over rows no walk wrote it answers the leaves `market_arrow_reader` answers for their messages. Neither runs [`lifecycle`](lifecycle.md): `codec.market_data(codec.lifecycle(messages))` - or `market_arrow_reader` over the same walk - is the sorted handoff when the walk's enrichment is wanted, and a walked capture reaches it as messages, never as the rows `lifecycle_arrow_reader` writes: what the walk settles from a message's predecessors - `prevpx`, `prevqty`, a side or ticker it carries forward, an execution instant - is no cell of the row, and the twin reads a walked row without it.

```text
FixCodec::market_data(&self, messages) -> impl FusedIterator<Item = Result<MarketData>> + Send + 'static
FixCodec::market_arrow_reader(&self, messages) -> Result<BatchReader>
FixCodec::market_data_arrow_reader(&self, source: BatchReader) -> Result<BatchReader>
FixCodec::book_arrow_reader(&self, messages, snapshot_millis: u64, filter: Option<&Filter>) -> Result<BatchReader>
```

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{BookIterator, MarketData, MarketKind};
    use yggdryl::local::LocalFolder;
    use yggdryl::{Filter, FixCodec, FixMsg, FixRegistry, Side, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));

    // The update arrives before the snapshot it follows.
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    ];
    let capture: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // The sorted door sorts every operation by the instant it stands at; the
    // trade entry is market data of its own.
    let operations: Vec<MarketData> =
        codec.market_data(capture.clone()).collect::<yggdryl::Result<_>>()?;
    let kinds: Vec<MarketKind> = operations.iter().map(MarketData::kind).collect();
    assert_eq!(
        kinds,
        [MarketKind::QuoteEvent, MarketKind::QuoteEvent, MarketKind::QuoteEvent, MarketKind::ExecutionEvent]
    );
    // The book walk prunes the trade: the snapshot is a complete book, the
    // bid's change a delta book over it.
    let books = BookIterator::new(operations.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(books.len(), 2);
    assert!(books[0].is_complete() && !books[1].is_complete());
    assert_eq!(books[1].deltas().len(), 1);
    assert_eq!(books[1].best_price(Side::Buy).map(|price| price.to_string()).as_deref(), Some("101"));

    // The book door does not sort: the same capture, out of order, leaves the
    // snapshot out with a warning - dated before the book it would fold into -
    // and no batch is an error.
    let mut lenient = codec.book_arrow_reader(capture.clone(), 0, None)?;
    assert!(lenient.all(|batch| batch.is_ok()));

    let rows = |reader: yggdryl::arrow::BatchReader| -> usize {
        reader.map(|batch| batch.expect("a batch").num_rows()).sum()
    };
    // A filter narrows what the books fold and never admits an execution.
    let executions: Filter = "marketdatakind = 'EXEC'".parse()?;
    assert_eq!(rows(codec.book_arrow_reader(capture.clone(), 0, Some(&executions))?), 0);

    // The same operations as rows, from the messages or from their FIX rows.
    assert_eq!(rows(codec.market_arrow_reader(capture.clone())?), 4);
    let fixed = codec.arrow_reader(fix_schema(&registry, "fix")?, capture)?;
    assert_eq!(rows(codec.market_data_arrow_reader(fixed)?), 4);
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl import Side, graph
    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)

    # The update arrives before the snapshot it follows.
    lines = [
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    ]
    capture = list(codec.parse_lines(lines))

    # The sorted door sorts every operation by the instant it stands at; the
    # trade entry is market data of its own.
    operations = list(codec.market_data(capture))
    assert [value.kind for value in operations] == [
        "quote_event",
        "quote_event",
        "quote_event",
        "execution_event",
    ]
    # The book walk prunes the trade: the snapshot is a complete book, the
    # bid's change a delta book over it.
    books = list(graph.BookIterator(operations))
    assert len(books) == 2
    assert books[0].is_complete and not books[1].is_complete
    best = books[1].best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal(101)

    # A filter narrows what the books fold and never admits an execution.
    assert codec.book_arrow_reader(capture, 0, "marketdatakind = 'EXEC'").read_all().num_rows == 0

    # The same operations as rows, from the messages or from their FIX rows.
    assert codec.market_arrow_reader(capture).read_all().num_rows == 4
    fixed = codec.arrow_reader(fix_schema(registry, "fix"), capture)
    assert codec.market_data_arrow_reader(fixed).read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, graph } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)

    // The update arrives before the snapshot it follows.
    const lines = [
      '8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|',
      '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
    ].map((line) => Buffer.from(line))
    const capture = [...codec.parseLines(lines)]

    // The sorted door sorts every operation by the instant it stands at; the
    // trade entry is market data of its own.
    const operations = [...codec.marketData(capture)]
    assert.deepEqual(
      operations.map((value) => value.kind),
      ['quote_event', 'quote_event', 'quote_event', 'execution_event'],
    )
    // The book walk prunes the trade: the snapshot is a complete book, the
    // bid's change a delta book over it.
    const books = [...new graph.BookIterator(operations)]
    assert.equal(books.length, 2)
    assert.ok(books[0].isComplete && !books[1].isComplete)
    assert.equal(books[1].bestPrice('BUYS'), '101')

    // A filter narrows what the books fold and never admits an execution.
    assert.equal(codec.bookArrowReader(capture, 0, "marketdatakind = 'EXEC'").intoTable().numRows, 0)

    // The same operations as rows, from the messages or from their FIX rows.
    assert.equal(codec.marketArrowReader(capture).intoTable().numRows, 4)
    const fixed = codec.arrowReader(fix.schema(registry, 'fix'), capture)
    assert.equal(codec.marketDataArrowReader(fixed).intoTable().numRows, 4)
    ```

## Serie faces

Every Arrow door has a serie face in Rust and Python: `parse_text_serie`, `lifecycle_serie`, `market_data_serie` and `messages_serie` take any `SerieSource` - a `Serie`, a `ChunkedSerie`, the `SerieReader` a handle's `read_serie` answers - and `serie_reader`, `book_serie` and `market_serie` take messages; each answers a `SerieReader` under its Arrow door's root. A face is a redirect: the source crosses as the batches it already is under an identity plan, so its answer handed to another face, or to a table's `append_serie` or `overwrite_serie`, is the door's own reader again and casts, copies and lands no row. Python answers the native `SerieReader`, so a capture read with `read_serie` reaches a table off the GIL. Every Python door that waits on the parse - these, the Arrow doors, `format_arrow_reader`, `write_arrow_reader`, and a handle's write of the `pyarrow` reader one answers - releases the GIL: the parse spreads over worker threads, and a worker that warns takes the GIL to reach `logging`. `lifecycle_arrow_reader` and `lifecycle_serie` drop a `SORT:by` their source declares, since the walk answers in its own order and may date a message again.

## The source's columns follow the shared ones

Where a line was read from is what a monitor orders and joins on, so the source's own columns stand right after the element, event, market and operation columns every generated schema opens with, and the message's own columns follow, exactly as they do for a [capture read line by line](capture.md#a-captures-own-columns-follow-the-shared-ones). Each emitted message receives the source row's carried values. A line carrying two frames therefore repeats the same URL, row number and timestamp. Which source columns survive a FIX column's claim on a name is decided once from the schema.

## A pin is on the codec, a stage is a call

What holds for a whole run is pinned on the codec once, and each pin is the per-run form of an argument the [byte readers](capture.md#a-reader-is-the-whole-parse-surface) read per call. There is no second options struct: the codec that reads a line is the codec that reads a batch.

| Pin | Builder | Default | Says |
| --- | --- | --- | --- |
| `payload_column` | `with_payload_column` | `body` (`DEFAULT_PAYLOAD_COLUMN`) | which column carries the frames: `utf8`, as the [text reader emits its rows](../media/text.md), and `binary` accepted too on intake, for a capture another producer landed as bytes |
| `separator` | `with_separator` | `SOH` (`0x01`) | the separator a re-emitted line is written with, which is what `write_arrow_reader` writes; reading takes none, because a line already said which byte separated its fields |
| `null_values` | `with_null_values` | the crate's spellings | what means "nothing was sent" |
| `direction` | `try_with_direction` | the set's `Send` code, `S` | the code of tag 385's set a line that states none of its own takes on the batch door - no `msgdirection` column stating one, and no [rule of tag 385's `FIX:directions`](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) matching the prose in front of its payload; any spelling of a code of the set, resolved once, and `None` or `""` pins nothing |
| `batch_byte_size` | `with_batch_byte_size` | `DEFAULT_BATCH_BYTE_SIZE`, 128 MiB | the estimated landed bytes one output batch targets |
| `batch_row_size` | `with_batch_row_size` | `DEFAULT_BATCH_ROW_SIZE`, 32,768 | the rows one output batch targets; a batch closes on whichever bound it reaches first |
| `threads` | `with_threads` | available CPUs | workers for parsing and row conversion. Capture Arrow parse doors hand each worker one job, at most one ahead: an input batch, or one of the near-equal row ranges a batch of more than twice `PARALLEL_JOB_ROWS` (256) rows is cut into without a copy - four per worker, none under that many rows - so one large batch keeps every worker busy and a worker holds at most part of one input batch. They merge ordered output using the closing targets `batch_byte_size` and `batch_row_size`; early reader drop joins dispatched work. Line, message and write doors instead retain 64-row chunks, at most two per worker. One uses no pool, zero reads as one, and `lifecycle` remains one ordered finite-capture walk. |
| `exclude_msgtypes` | `with_exclude_msgtypes` | `DEFAULT_REFUSED_MSGTYPES`: `Heartbeat`, `TestRequest`, the untyped row | the types [no row is built for](decode.md#a-type-nobody-asked-for-is-never-built) |
| `include_msgtypes` | `with_include_msgtypes` | empty, which reads every type the refusals leave | the types read, naming any clearing the default refusals |
| `capture_names` | `with_capture_names` | none | what a run's row-header captures are called, in the order a line answers them, so [`parse_text_line`](capture.md#a-reader-is-the-whole-parse-surface) reads a capture by position rather than by name |
| `default_sending_time` | `try_with_default_sending_time` | none, one UTC-now read per undated message | the [`SendingTime(52)`](capture.md#every-message-is-dated) a message stating none is dated by where neither a row cell nor the `currunix` of the line it was read out of states one; an exact nanosecond UTC instant, else refused; pin it for a reproducible read |
| `market_metadata` | `with_market_metadata` | on | whether a leaf the codec builds - `market_data`, `market_arrow_reader`, `market_data_arrow_reader`, `book_arrow_reader` - carries in its metadata what its message states that no typed column reads and no identifier map of the leaf holds, and lifts into its `identifiers` the keys ending with an identifier its message declares, by [the one rule](message.md#what-a-leafs-metadata-holds); the map and what it lifts feed the leaf's identity, so turning it off - no metadata, nothing lifted - moves every leaf whose message states such a field, and `FixMsg::market_data` always carries it |
| `official_time_delay_ms` | `with_official_time_delay_ms` | `DEFAULT_OFFICIAL_TIME_DELAY_MS`, one second | how far from `SendingTime(52)` an [official clock](capture.md#the-official-clock-dates-the-message) may stand and still date the message: the `TransactTime(60)` the message states, else the `TrdRegTimestamp(769)` its `TrdRegTimestampType(770)` says is about the event or a hop, nearest the sending clock; the sending clock dates the message where none stands that near, and a nonpositive delay admits only a clock equal to it |
| `dedup_window_ms` | `with_dedup_window_ms` | `DEFAULT_DEDUP_WINDOW_MS`, one minute | how long, in milliseconds of event time, [`lifecycle`](lifecycle.md#an-identity-is-yielded-once) remembers an identity it yielded so it yields that identity once; a grid view is exempt, and a nonpositive window remembers none |

What happens to a message on its way into a row is a stage, and a stage is a call over the stream rather than a flag on the reader: [`lifecycle_arrow_reader`](#chained-where-it-sits) chains batches, `lifecycle` [chains](lifecycle.md#in-a-batch-read) a finite capture, and Rust-only `FixDedup` drops an adjacent republication. Restating and filling are parse behavior. Python spells `snapshot_ns` as an exact integer nanosecond keyword; JavaScript spells `snapshotNs` as a `bigint`; absent, `null`, zero and negative values disable snapshots. The [sorted lifecycle](lifecycle.md#a-sorted-source-is-walked-one-hour-at-a-time) pin is Python's `sorted_lifecycle=True` and JavaScript's `{ sortedLifecycle: true }`, off when absent. Python spells the dating delay `official_time_delay_ms` and JavaScript `officialTimeDelayMs`, each a whole number of milliseconds, and absence takes the core's one second. The lifecycle's window is Python's `dedup_window_ms` and JavaScript's `dedupWindowMs`, a whole number of milliseconds: absence takes the core's one minute, `None` / `null` remembers nothing, and `with_dedup_window_ms` / `withDedupWindowMs` sets another on a codec in hand. The metadata switch is Python's `market_metadata=True` and JavaScript's `{ marketMetadata }`, each read back by a getter of its name.

Lines to batches, with a lifecycle stage that sorts the finite capture: the walk names an order and its fill report as one chain, the execution the parse split off the report is a chain of its own, and every row carries its `crossuuid`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::local::LocalFolder;
    use yggdryl::{ArrowCastOptions, FixCodec, FixRegistry, Serie, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;

    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ];
    let read = codec.arrow_reader(schema, codec.lifecycle(codec.parse_lines(lines)))?;

    let batch = read.into_iter().next().expect("one batch")?;
    assert_eq!(batch.num_rows(), 3, "the order, the report and its execution");
    let chain = batch.column_by_name("crossuuid").expect("the chain column");
    assert_eq!(chain.null_count(), 0, "every message names its chain");
    let held = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())?;
    let chains = held.child("crossuuid").expect("the chain column");
    assert_eq!(chains.scalar(0)?, chains.scalar(1)?, "one order, one chain");
    assert_ne!(chains.scalar(2)?, chains.scalar(0)?, "the execution is its own");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)
    schema = fix_schema(registry, "fix")

    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ]
    read = codec.arrow_reader(schema, codec.lifecycle(codec.parse_lines(lines)))

    held = read.read_all()
    assert held.num_rows == 3, "the order, the report and its execution"
    chains = held.column("crossuuid").to_pylist()
    assert None not in chains, "every message names its chain"
    assert chains[0] == chains[1], "one order, one chain"
    assert chains[2] != chains[0], "the execution is its own"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)
    const schema = fix.schema(registry, 'fix')

    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|',
    ].map((line) => Buffer.from(line))
    const read = codec.arrowReader(schema, codec.lifecycle(codec.parseLines(lines)))

    const held = read.intoTable()
    assert.equal(held.numRows, 3, 'the order, the report and its execution')
    const chain = held.getChild('crossuuid')
    assert.equal(chain.nullCount, 0, 'every message names its chain')
    assert.deepEqual(chain.get(0), chain.get(1), 'one order, one chain')
    assert.notDeepEqual(chain.get(2), chain.get(0), 'the execution is its own')
    ```

## A column is the caller speaking per row

One column carries the frames; two more supply, per row, arguments the byte readers already take per call, and every other column is offered to the message by name. A separator is not among them: which byte separated a frame's fields is what the line itself said, so no row states it. Nor is a capture's `timestamp`: it is context carried in front, and the message's own [clocks](capture.md#every-message-is-dated) settle `currunix`.

| Column | Supplies |
| --- | --- |
| the payload column, named by the codec | the frame parsed |
| `beginstring` | the version the row was read at |
| `msgdirection` | the direction, stated: a code of tag 385's set, under any spelling |
| `currunix` | when the row's line was written - the text reader's `mtime` - as the carrier's precise recording instant: written to `recdunix` unless the message or another row cell states `recdunix` directly, and the [sending clock](capture.md#every-message-is-dated) of a message stating no `SendingTime(52)` that no `sendingtime` cell dates, never a fact of the message; the epoch is silence, because a line nothing dated reads as the epoch |
| `sourceurl` | nothing: [the capture's own column](message.md#a-row-is-a-message-again) is what a reader said about the line, so it fills no message fact and is written straight into its own column of the row instead |
| any other column named after a field | that field, where the message did not state it - a `sendingtime` column among them, which outranks the line's `currunix` and the codec's default sending time |
| one of the other fourteen [event columns](../graph/market-data.md#columns) | nothing: they are the carrier's own facts - the [text line](../media/text.md) each row is, as the reader stated it - so `curruuid` is each message's one source and the rest fill no message fact, a line's `prevunix` - the instant of the line the read cut before it - among them |

A column is the caller speaking per row and a pin is the caller speaking per run, so a column outranks the pin and both outrank what the frame infers: a row whose `beginstring` says `FIX.4.2` is read at 4.2 whatever the codec was pinned to, and its values translate through the code spellings 4.2 declares. A column absent, null or empty is silence, never an instruction and never an error.

A fill is named the way a key is: a column whose folded name resolves in the registry's one namespace - the canonical fold, then an alias fold, so a `MsgSessionId` capture reaches the crate's own `msgsessionid` and a `msgpluginid` column the crate's `msgpluginid`. It is row-only: never an entry, so it is not in `fixentries`, not re-emitted by `write_arrow_reader` and not in the arrival digest; a value the field cannot hold fills nothing rather than a null; a column named by a tag's digits fills nothing, because a name is what reaches a field; and a name reaching one of [the capture's own columns](message.md#a-row-is-a-message-again) fills nothing either, because no message holds a *fact* for one: the cell is carried instead, and stated again at its own column. Which columns fill is decided once, from the schema and the dictionary, rather than per row.

`beginstring` and `msgdirection` are FIX columns' own names, so they are not carried in front: the row's `beginstring` and `version` columns say what a `beginstring` column decided. A record carrying only a payload column behaves exactly as the byte reader behaves, which is what makes this an entry point rather than a second contract.

### A bridge log names what it fills

`yggdryl::ULBRIDGE_ROWHEADER` is the [row header](../media/text.md) a ULBridge log writes in front of every line - a clock, a thread bracket, the plugin that wrote the line and its level. Its clock is `mtime`, so the header dates each line it matches; the other four captures are each named for what they fill, and the thread and the level are matched but not captured, so the header leads a row with no column of its own. Rust names the constant; the regex is the same text, ending in one space, in any binding's `rowheader`. A row header capture named `seqnum` or `crosscode`, in any case, is refused because the line derives those facts from its row number and the identifier it was read under. A capture named for another capture-fed [event column](../graph/market-data.md#columns) - `state`, `prevuuid`, or an optional event instant - feeds the line's own fact but no message field: the line states its identity as the message's source, which is all it says about the message. A capture named `execunix` is no event fact - when an element last executed is a [market fact](../graph/market.md#contract), and a line is no market element - so it is an ordinary text column of the line's batch.

```text
^(?P<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:[.,]\d{3}(?:_\d{3})?)?) \[[1-9]\d*(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] \[(?P<msgpluginid>[^\]]+)\] \([A-Z]+\) 
```

| Capture | Typed as | In a batch read |
| --- | --- | --- |
| `mtime` | `datetime64(ns, UTC)`, under the text options' `timezone` | no column: consumed into the line's `currunix`, which is its messages' `recdunix` and the sending clock of any that states no `SendingTime(52)`; a point or a comma opens the fraction - three digits, or the grouped microseconds `.524_315` - and a clock may state none |
| `msgsessionid` | utf8, nullable | the session instance the bridge handled the line on; fills `msgsessionid` (65043) rather than leading the row, and never over a reading the message stated. Not the counterparty session a bridge row spells `SESSIONID` for: two connections to one counterparty are two instances |
| `msgctxid` | utf8, nullable | fills `msgctxid` (65042) |
| `msgseqnum` | int64, nullable | fills `MsgSeqNum(34)` where the frame did not carry it |
| `msgpluginid` | utf8 | fills `msgpluginid` (65040), the plugin that logged the line, and selects nothing |

The session instance, the context and the sequence number are optional as a whole, so a line carrying only its thread still frames and leaves them null rather than failing the row.

Editing a row header changes how many events a walk answers, which is worth saying plainly because nothing about it looks like a lifecycle change. A line the expression does not match yields no captures at all rather than failing the row: it keeps its body, is dated by its object's modification time rather than a clock of its own, and reaches the walk with no session instance, no message context and no sequence. Those three with the message type are what build `msgsesseventid`, and that is the key two observations of one session event are [merged](lifecycle.md#a-twin-is-not-a-successor) on - so a line the header misses is a line the walk cannot fold, and one unchanged capture read under a narrower header answers *more* events, not fewer. Every message still parses and `currhashcode` still agrees, because the capture's session context is provenance and is excluded from the content code by name; the missed lines' `currunix` and `curruuid` move, because their object's modification time dates them instead of their own clock. `ULBRIDGE_ROWHEADER`'s own clock reads both fractions the bridge writes, three digits and the grouped microseconds it spells `23:59:46.524_315`, behind a point or a comma, and a clock stating none; it admitted only three digits until 0.1.10, and so could not read the last fifteen lines of the capture shipped beside it. Assert the count of lines a header matched beside the count of messages parsed; the two diverge silently otherwise.

A capture that names a field fills it, so the registry's one namespace is what lands it and nothing translates in between; a capture that names none is the capture's own column and fills nothing - this header has none. The clock is `mtime`, which is [consumed into `currunix`](../media/text.md) rather than carried beside it and is read at `datetime64(ns, UTC)` whatever its own syntax suggests, so every line the header matches is dated by the clock written in front of it rather than by its file's modification time - and a line's `currunix` is its messages' `recdunix` and the sending clock of any that states no `SendingTime(52)`. Until 0.1.17 the clock was captured as `timestamp`, which dated nothing, so every line of a read took its file's one modification time. The bridge writes these in camel case - `msgCtxId`, `seqNum` - and they used to be captured that way, with a table mapping `seqnum` onto tag 34; naming the captures for the fields retires that table. The session instance, context and plugin are [capture facts](message.md#typed-tags), held by `FixCapture` rather than the content row. A complete nonempty message type, session instance and context with a present message sequence join to `msgsesseventid` (65044), a fourth capture fact and a column of the fixed row of its own: the four values joined by `:` as stated, `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>`, with the sequence in its canonical `u64` spelling. Thus message type `8`, session `e7256476`, context `9effef3e6a` and sequence `1094` spell `8:e7256476:9effef3e6a:1094`, and any absent or empty text part leaves it null. It is capture provenance and is excluded from the FIX content UUID. A cross code instead comes from an explicit nonempty value or the message's ordered FIX identifiers, as [the lifecycle](lifecycle.md#a-chain-is-named-by-its-cross-code) defines. Where the line was read from is not among them at all - that is [the capture's own column](message.md#a-row-is-a-message-again), which a message carries and never states, because the same message read from a second copy of one day's log is the same message.

The plugin is a fill and nothing more: it lands in the crate's own `msgpluginid` column by name, like any capture named after a field, and selects no dictionary and no version - the registry is one namespace, and which dictionaries a field belongs to is the field's own `FIX:branches`, which no read consults.

=== "Rust"

    ```rust
    use yggdryl::text::TextOptions;
    use yggdryl::{DataType, ULBRIDGE_ROWHEADER};

    let options = TextOptions::new().try_with_rowheader(ULBRIDGE_ROWHEADER)?;
    let captures = options.source_field()?;
    let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
    // The clock dates each line, so it leads no column of its own.
    assert!(names.ends_with(&["msgsessionid", "msgctxid", "msgseqnum", "msgpluginid"]));
    assert!(!names.contains(&"mtime"));
    // Typed from the pattern before a byte is read.
    assert_eq!(captures.field("msgseqnum")?.dtype(), &DataType::Int64);
    ```

## One row per message

A source row is read for every message it carries, so a capture answers one row per message and never one per line. One ordinary frame is one row, and a line carrying two frames is two, each re-emitting only its own bytes. A line carrying a [JSON document](capture.md#a-json-document-is-one-message-stating-nothing) is one row whatever the document names - a bulk or wildcard answer as much as a single one - holding an `unknown` message with no entries and no `msgtype`, its carried columns beside it. A line carrying no message at all emits zero rows: a bridge's own prose is a line and not a row. The one refusal that still yields a row is a payload that was there and would not parse: it holds an empty message, so malformed syntax never fails a batch; a stated clock that does not read - `SendingTime(52)` or `TransactTime(60)` - is left unstated beside an anomaly and a [warning](capture.md#warnings), and the message is dated as one stating none is. What a line carries is the codec's rule, stated in [decode](decode.md); a caller wanting one row per *line* reads the capture with the [text reader](../media/text.md), which answers every line whether or not a message is in it. Join a parsed capture by its carried source identifier rather than assuming row positions still align. `parse_text_line` is the same reading of one line, and answers the iterator when expansion is wanted.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{ArrowCastOptions, DataType, FixCodec, FixRegistry, Scalar, Serie, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);

    // Row 7 carries two frames; row 8 is a sentence, which is no row at all.
    let body = "8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|";
    let silent = "heartbeat emitted seq=7";
    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?).required_field("capture");
    let rows = [
        Scalar::from_sequence([Scalar::from(7_i64), Scalar::from(body)]),
        Scalar::from_sequence([Scalar::from(8_i64), Scalar::from(silent)]),
    ];
    let batch = Serie::from_scalars(field, rows)?.into_arrow_batch()?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
    let reader = FixCodec::new(registry).parse_text_arrow_reader(source)?;
    let mut count = 0;
    for batch in reader {
        let values = Serie::from_arrow_batch(None, &batch?, ArrowCastOptions::new())?;
        // Each row keeps the source row's own columns, read by name.
        let rownum = values.child("rownum").expect("the capture's row number");
        let text = values.child("body").expect("the capture's body");
        for row in 0..values.len() {
            assert_eq!(rownum.scalar(row)?, Scalar::from(7_i64));
            assert_eq!(text.scalar(row)?, Scalar::from(body));
            count += 1;
        }
    }
    assert_eq!(count, 2);
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pyarrow as pa
    from yggdryl.fix import FixCodec, FixRegistry

    # Row 7 carries two frames; row 8 is a sentence, which is no row at all.
    body = "8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|"
    silent = "heartbeat emitted seq=7"
    source = pa.table({"rownum": pa.array([7, 8], pa.int64()), "body": pa.array([body, silent], pa.string())})
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    result = FixCodec(registry).parse_text_arrow_reader(source.to_reader()).read_all()
    assert result.num_rows == 2
    # Each row keeps the source row's own columns, and reads its own frame.
    assert result.column("rownum").to_pylist() == [7, 7]
    assert result.column("body").to_pylist() == [body, body]
    assert result.column("clordid").to_pylist() == ["A", "B"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, fix } = require('yggdryl')

    // Row 7 carries two frames; row 8 is a sentence, which is no row at all.
    const body = '8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|'
    const silent = 'heartbeat emitted seq=7'
    const source = new arrow.Table({
      rownum: arrow.vectorFromArray([7n, 8n], new arrow.Int64()),
      body: arrow.vectorFromArray([body, silent], new arrow.Utf8()),
    })
    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const result = new fix.FixCodec(registry).parseTextArrowReader(BatchReader.from(source)).intoTable()
    assert.equal(result.numRows, 2)
    // Each row keeps the source row's own columns, and reads its own frame.
    assert.deepEqual([...result.getChild('rownum')], [7n, 7n])
    assert.deepEqual([...result.getChild('body')], [body, body])
    assert.deepEqual([...result.getChild('clordid')], ['A', 'B'])
    ```

## Rows are messages again, and messages rows

`messages` reads a stream of batches back as semantic messages, each row through [`FixMsg::from_row`](message.md#a-row-is-a-message-again) under the source schema: projected columns, residual `fixentries` - `0:<key>` entries included - and the keys `metadata` holds that no dictionary resolved rebuild the message without parsing. Its recorded identity cells remain stated while market getters refill from the reconstructed content. `arrow_reader` is the other direction: a stream of messages into batches under a schema, each through `FixMsg::into_row`, closed on the bytes each row lands as. The two invert each other at the canonical row: `messages` reads each row's own cells into the message it makes, carried and never content, and `into_row` states them again at their columns, so a `messages` -> stage -> `arrow_reader` composition keeps them whatever order the stage answers in - `lifecycle_arrow_reader` and `format_arrow_reader` are that composition spelled once. It is what lets a stage run over a capture already landed in Arrow; the example ends [back on the wire](#back-to-the-wire). One thread holds one source batch at a time. Several threads retain bounded 64-row chunks, at most two per worker, which can span input batches and still answer messages in source order.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');
    let schema = fix_schema(&registry, "fix")?;

    let lines = ["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|", "8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // Into batches, and back: the same canonical row and content identity.
    let again: Vec<FixMsg> = codec
        .messages(codec.arrow_reader(schema.clone(), parsed.clone())?)
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(again.len(), parsed.len());
    for (held, message) in again.iter().zip(&parsed) {
        assert_eq!(held.into_row(&schema)?, message.into_row(&schema)?);
    }

    // And out to the wire: one line per row, rebuilt from the message's
    // own facts and its entries. Residual entries lead, then projected facts
    // reconstruct in schema order, so this canonical wire is explicit.
    let mut written = Vec::new();
    assert_eq!(codec.write_arrow_reader(codec.arrow_reader(schema, again)?, &mut written)?, 2);
    assert_eq!(
        String::from_utf8(written)?,
        "8=FIX.4.4|35=D|11=ORDER-1|9999=x|54=1|59=0|55=AAPL|10=0|\n8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|381=637.5|59=0|10=0|\n",
    );
    ```

=== "Python"

    ```python
    import io
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry, separator=ord("|"))
    schema = fix_schema(registry, "fix")

    lines = [b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|", b"8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"]
    parsed = list(codec.parse_lines(lines))

    # Into batches, and back: the same canonical row and content identity.
    again = list(codec.messages(codec.arrow_reader(schema, parsed)))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        assert held.currhashcode == message.currhashcode
        assert held.into_row(schema) == message.into_row(schema)

    # And out to the wire: one line per row, rebuilt from the message's own
    # facts and its entries. Residual entries lead, then projected facts
    # reconstruct in schema order, so this canonical wire is explicit.
    sink = io.BytesIO()
    assert codec.write_arrow_reader(codec.arrow_reader(schema, again), sink) == 2
    assert sink.getvalue().decode().splitlines() == [
        "8=FIX.4.4|35=D|11=ORDER-1|9999=x|54=1|59=0|55=AAPL|10=0|",
        "8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|381=637.5|59=0|10=0|",
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry, { separator: 124 })
    const schema = fix.schema(registry, 'fix')

    const lines = ['8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|', '8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|']
    const parsed = [...codec.parseLines(lines.map((line) => Buffer.from(line)))]

    // Into batches, and back: the same canonical row and content identity.
    const again = [...codec.messages(codec.arrowReader(schema, parsed))]
    assert.equal(again.length, parsed.length)
    again.forEach((held, at) => {
      assert.equal(held.currhashcode, parsed[at].currhashcode)
      assert.ok(held.intoRow(schema).equals(parsed[at].intoRow(schema)))
    })

    // And out to the wire: anything with write(chunk) is a sink - a stream,
    // a socket, an array - one line per row, rebuilt from the message's own
    // facts and its entries. Residual entries lead, then projected facts
    // reconstruct in schema order, so this canonical wire is explicit.
    const chunks = []
    assert.equal(codec.writeArrowReader(codec.arrowReader(schema, again), { write: (chunk) => chunks.push(Buffer.from(chunk)) }), 2)
    assert.deepEqual(Buffer.concat(chunks).toString().split('\n').slice(0, 2), [
      '8=FIX.4.4|35=D|11=ORDER-1|9999=x|54=1|59=0|55=AAPL|10=0|',
      '8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|381=637.5|59=0|10=0|',
    ])
    ```

## Back to the wire

`write_arrow_reader` streams a batch back out as lines, one per row and so [one per message](#one-row-per-message). It first rebuilds each semantic message from its projected columns, its residual `fixentries` - `0:<key>` entries included - and the unresolved keys of its `metadata`, then calls [`into_bytes`](encode.md) with the codec's `separator`, `SOH` unless pinned, and a newline. The output is canonical message wire rather than a promise to reproduce original arrival order. The count of lines is answered; the [round trip above](#rows-are-messages-again-and-messages-rows) ends there. A batch carrying no `fixentries` column cannot be written and says so before a row is read. One batch is pulled, its rows written, and it is dropped; no buffer bigger than a row is held.

## Chained where it sits

`lifecycle_arrow_reader` is [`lifecycle`](lifecycle.md) over batches: each row becomes its message through [`FixMsg::from_row`](message.md#a-row-is-a-message-again), the walk states it as the one after the live message of its chain, and it is written back under the **same** schema, so a carried column returns to its place and the content is carried through untouched. Nothing is parsed again.

A carried column returns to its place because the message carries it: a message holds no *fact* about the reading it arrived through, so the door reads each row's own cells into the message as [what it carries](message.md#a-row-is-a-message-again), provenance and never content, and `into_row` states them again at their own columns. The pairing is by message and never by position - the walk answers messages in their own order, which a capture's lines are routinely not in, so a row's `body` would otherwise come back attached to a different message's row. The batches it answers close on the bytes each row lands as, against the codec's `batch_byte_size`; the walk itself collects its source, because a capture's lines are written in the order a bridge logged them and two messages of one chain routinely arrive out of their own order.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::local::LocalFolder;
    use yggdryl::{ArrowCastOptions, DataType, FixCodec, FixRegistry, Scalar, Serie, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(registry);

    // An order and the fill that answers it, on two lines of one capture.
    let capture = DataType::from(StructType::from_fields([DataType::utf8().required_field("body")])?).required_field("line");
    let lines = [
        "8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        "8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ];
    let batch = Serie::from_scalars(
        capture,
        lines.map(|line| Scalar::from_sequence([Scalar::from(line)])),
    )?
    .into_arrow_batch()?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

    let read = codec.parse_text_arrow_reader(source)?;
    let chained = codec.lifecycle_arrow_reader(read)?;
    let held = chained.into_iter().next().expect("one batch")?;

    // The same schema, and one chain: the fill report follows the order,
    // names it and shares its identity; the execution split off it is the
    // third row, a chain of its own, standing after the report at the
    // report's instant - the report, a second after the order, is first at
    // its own: place zero.
    assert_eq!(held.num_rows(), 3);
    let cross = held.column_by_name("crossuuid").expect("the chain column");
    assert_eq!(cross.null_count(), 0);
    let rows = Serie::from_arrow_batch(None, &held, ArrowCastOptions::new())?;
    let column = |name: &str| rows.child(name).expect("a column");
    assert_eq!(column("crossuuid").scalar(0)?, column("crossuuid").scalar(1)?);
    assert!(!column("prevuuid").is_null(1)?);
    assert_eq!(column("seqnum").scalar(1)?, Scalar::from(0_u64));
    assert_eq!(column("seqnum").scalar(2)?, Scalar::from(1_u64));
    // The content is what each line stated, carried through untouched.
    assert_eq!(column("ordstatus").scalar(1)?.as_str(), Some("2"));
    ```

## Edges

- Ordinary unframed text produces no row, and an empty payload none either; a payload that was there and would not parse produces one row holding an empty message. Output counts follow message expansion.
- A carried column whose folded name a FIX column takes is dropped in front rather than renamed - two columns of one name is not a schema - and what it stated lands in that FIX column.
- A `msgdirection` column is the row's stated direction, read as a parameter - any spelling of a code of tag 385's set, stored as the code - and it outranks the reading of the line and the codec's pin; the FIX column carries it and no second column repeats it.
- A `timestamp` column is carried context: it leads the row as a column of its own and never dates the message - the message's [own clocks](capture.md#the-official-clock-dates-the-message) do - and it is a fact about the capture, so it is outside the code the message's content digests to.
- A fill never overrides what the frame stated: a `msgseqnum` capture beside a frame carrying `34=` leaves that field to the frame.
- A fill is row-only: never an entry, never in `fixentries`, never re-emitted by `write_arrow_reader`, never in the arrival digest.
- `messages` reads a row carrying the settled values - `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `crosscode`, `curruuid`, `crossuuid`, `seqnum` - as a replayable message; a row leaving one of them null is left out with a [warning](capture.md#warnings), since the fixed row declares them required.
- `messages` on a row whose `securityids`, `identifiers` or `partyids` cell holds a key that reads as none, a value its type refuses or two spellings of one key with two values, or whose `fixentries` holds a key naming another field than its tag (`55:securityid`) -> that row left out with a [warning](capture.md#warnings), because [`FixMsg::from_row`](message.md#a-row-is-a-message-again) refuses it: a row's identifier map is its word, never replaced by what its fields state.
- A batch closes on the bytes each row lands as - the leaves of every column the row fills and a per-row width - so a source batch of any size splits by what its messages land as, and a line answering two messages is charged twice, once per row.
- A `batch_byte_size` of `0` or `1` is a batch a row: the target is where a batch closes, never a bound a row must fit under.
- `parse_text_arrow_reader` on a source with no column named as the payload column, or one holding neither text nor bytes under it -> refused before a row is read, naming the column. `parse_text_line` has no column to name: a line's body is a typed field, so a line whose body is empty answers no message at all and nothing else is refusable.
- `messages` on a source whose schema makes no root field -> the one error item, before a row is read, which fuses the stream; a later batch of another schema, and a row that is not a FIX row, are left out with a [warning](capture.md#warnings) and the rest read; a source failure is yielded after the messages before it and fuses the stream.
- `arrow_reader` under a schema the message cannot fill whole -> that message alone is left out with a [warning](capture.md#warnings) and the rest of its batch reads - a required column the message leaves empty; an item its stream refused is left out the same way, and a source failure yields the completed prefix, then the error, and fuses the reader. A payload the codec cannot read is an empty message through `parse_text_arrow_reader` and left out with a warning by `parse_lines`.
- `arrow_reader` over messages carrying no entries - built by hand, or read back from rows holding only the fixed columns - charges each the leaves of its row, so a projection without the arrival record is bounded by the same target.
- `write_arrow_reader` on a batch without `fixentries` -> refused before a row is read; a row whose message held no pairs -> an empty line, still counted.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single row.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix batch::a_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix
    cargo test -p yggdryl --test fix dataset::
    cargo test -p yggdryl --test fix dataset::ulbridge_dataset_allocation_profile_is_sequential_and_staged -- --exact --nocapture --test-threads=1
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_fix.py
    python scripts/check_docs_examples.py --lang python
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    python scripts/check_docs_examples.py --lang javascript
    ```

## Performance

Two measurements, kept apart: what a stage allocates, which is exact and pinned,
and how long it takes, which is a release run on a named machine.

### Allocations per stage

`a_real_line_costs_the_same_at_every_stage_every_time` in `rust/tests/allocations.rs`
pins what three real lines of `rust/tests/fix/ulbridge.log` cost at each stage of
the pipeline on the committed dictionary, once and sixty-four times over, so a
stage that grows with a cache or leaks a plan fails its linearity before its
number: a bridge row of named keys (`bridge_pipe`, line 1), a frame spelled with
`|` (`frame_pipe`, line 72) and the `35=UL` frame packing a group inside a group
(`frame_packed`, line 111). Every stage runs over what the one before it made,
set up outside the count - the clone a stage takes by value is the caller's -
and the landing is on a root whose projection is warm, because a root is a fact
of the schema and the first landing under it fills every level's cache. Debug
build, `--all-targets`, one thread; the numbers are the pins, and they move only
with the sentence in the pin that says why.

| stage, per message | `bridge_pipe` | `frame_pipe` | `frame_packed` |
| --- | ---: | ---: | ---: |
| `parse`, the codec over the body | 556 | 223 | 1,022 |
| `into_row`, a fresh clone against the fixed schema | 88 | 64 | 250 |
| `landing`, the row as a one-row `Serie` under the warm root | 1,502 | 1,482 | 1,520 |
| `batch`, the `Serie` built into a `RecordBatch` | 210 | 210 | 210 |
| `digest`, the arrival record's hash | 1 | 1 | 1 |
| `lifecycle`, the walk over one message | 10 | 10 | 10 |

The `fix_allocations` target reports the same dimension over the whole capture,
one copy on one thread, per message: `fix/allocations` counts requests and
`fix/allocated_bytes` the bytes they asked for, each over the stages
`fix/ulbridge` times - `text`, `parse`, `parse_lifecycle`, `into_row`,
`from_row`, `into_bytes`, `batch`, `digest` and the `step/*` cases - through
the counting allocator the test binaries share. A count is exact, so Criterion
prints one number under its `time` label and `p = 0.00` where a count moved;
`--quick` is the smoke, and the release configuration adds nothing a count needs.

```bash
cargo test -p yggdryl --test allocations a_real_line
cargo bench -p yggdryl --bench fix_allocations -- --quick
```

`ulbridge_dataset_allocation_profile_is_sequential_and_staged` in
`rust/tests/fix/ulbridge.rs` is the staged profile over the whole capture: it
prints first and second passes for text framing, codec parsing, fixed-row
materialization, typed row holders and Arrow output - requested bytes are
cumulative requests, never peak memory - and asserts that a second pass of the
codec, the row and the batch door retains nothing: what a stage keeps after its
first pass is a cache filling, and what it keeps after the second would be a
plan table growing per message.

### Timings

`fix/pipeline`, the whole path a desk takes over a bridge's own log: `rust/tests/fix/ulbridge.log`, a second of a ULBridge's capture beside every shape a bridge writes - a Jolokia exchange whose answer is a JSON document the codec does not read, FIXML behind a verb, frames spelled with `^A` and `<SOH>`, a `35=UL` frame packing a group inside a group, bridge rows of a hundred named keys, a statistics line, an empty body, the bridge's sixteen handed-over lines and the fifteen of a cancel/reject flow - repeated 64 times: 9,216 lines, 6,080 messages, 13.9 MB. Every stage runs over the same corpus on its own, so a figure is per line of a real capture rather than of one shape, and a row is one per message rather than one per line ([decode](decode.md)). The current smoke covers six pool cases: one, two and four workers for each of `parse_text_arrow_reader` and `parse_arrow_messages`; it claims no current speed or throughput. The historical release run used thin LTO, one codegen unit, one Linux x86_64 container, Intel Xeon @ 2.10 GHz, 4 cores, 15 GiB, no other build running, load average 1.1 when the run ended; rustc 1.94.1; the shipped dictionary alone; `cargo bench -p yggdryl --bench fix -j 2 -- 'fix/pipeline/(text_read|parse_text_arrow_reader|parse_lines|parse_text_lines_msgpluginid|into_row|arrow_reader|lifecycle|digest)$' --sample-size 10`, ten samples a case. That release run predates the message becoming a typed market event over a content row, which moved the fill into the parse and the chain onto the graph's one walk, so every figure below is historical and due regeneration.

| stage | estimate | throughput | per line, row or message |
| --- | --- | --- | --- |
| `text_read`, the row header framed, each line numbered, classified and read for its direction | 99 ms | 140.5 MB/s | 10.7 us |
| `parse_text_arrow_reader`, the whole path into fixed rows | 2.12 s | 6.6 MB/s | 229.9 us |
| `parse_lines`, the codec alone over the framed bodies | 1.19 s | 11.7 MB/s | 129.0 us |
| `parse_text_lines_msgpluginid`, the line reader with each row naming its plugin | 1.18 s | 11.8 MB/s | 128.4 us |

The release estimates above are historical. In that run, the text stage was a small fraction of the whole and the codec about half; the rest was the row landing in Arrow.

What remains is attributed rather than argued, by the `text_scan` group of the text benchmark and the `fix/line` group of this one, which measure the scan and the codec shape by shape. On the codec path the builder is 60-90% of every shape; in front of it, a bridge row of a hundred pairs is read into a tree of counted ranges of its page, and each pair then crosses one more stage on its way to the builder. That is the cost of ranges: every key and value a message records is a range of the line it came from, so a data field re-slices to its stated length and a frame re-emits byte for byte without a copy, and it is paid on every pair whether or not a reader ever asks for the range. It goes only with a reader that builds the codec's pairs from the scanner's spans without the tree between them, which is a change to what an entry is and not to how fast it is read.

What a message costs after it is built, each pass over fresh clones of the 6,080 messages, so every number is the pass over a message the stream just built:

| pass | estimate | per message |
| --- | --- | --- |
| `into_row`, the message read against the fixed schema | 350 ms | 57.5 us |
| `arrow_reader`, the rows landed in batches | 439 ms | 72.2 us |
| `lifecycle`, the stamp that joins a message to its order's life | 311 ms | 51.1 us |
| `digest`, the arrival record's hash | 21 ms | 3.5 us |

A row pays `into_row` and its share of the batch; it pays for the walk only when the caller composes that [stage](#a-pin-is-on-the-codec-a-stage-is-a-call), and for the fill inside the parse that built it. Reading a message against the fixed schema is a lookup per column, most of them misses answered by a name table the message builds on its first projection; the batch is the rows canonicalized and built into one `RecordBatch`, of which the residual map is the one column rendering JSON - a group or a component it holds is written as its JSON once per row. The fill inside a parse is every child resolved against the dictionary once, the specification's retirements of its tags applied from the crate's table, a currency pair read off the symbol through the registry's memo, then the crate's [native derivations](capture.md#what-a-message-implied-is-filled-in) read off the message by tag, with no expression tree and no working row, landing everything derived in one rebuild. The walk is a chain lookup, one statement of the predecessor's identity, instant and place, and the identity settled again. The digest is a hash over the arrival record and nothing else.

`decoded_lifecycle` is the walk a bridge capture pays: the messages of the decoded line stream, each carrying its row header's session, context, sequence and recording clock, walked with no parse in front. Most of a bridge's lines are one session event observed again at another hop, so the walk folds every observation of an event into one reference before it chains anything, and a fold merges content - both rows unpacked, merged, sorted and repacked. An observation whose row, lifted facts and text equal a content the reference already merged adds nothing to that union, so it folds its event facts, anomalies and provenance alone; `a_content_merged_once_folds_nothing_more_when_delivered_again` in `rust/tests/fix/ulbridge.rs` pins that a repeated content moves nothing. The redating moves the clock alone: a frame stating no `SendingTime(52)` is dated by its `TransactTime(60)`, and only what the clock moves is settled again - a report's own execution instant and the identity - while the market, the maps and the digest stand, because a parsed message is settled already; `a_redated_message_settles_its_clock_as_the_whole_pass_does` in `rust/tests/fix/enrich.rs` holds that to the whole pass over every message of the bridge's capture. Release build, thin LTO, one codegen unit, one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB, rustc 1.94.1, 6,016 messages, both sides measured back to back:

| pass | before | after | 0.1.17 |
| --- | --- | --- | --- |
| `lifecycle`, the walk over the 6,080 parsed messages | 447 ms | 114 ms | 102 ms |
| `lifecycle_same_shape`, one execution report logged a thousand times | 67.0 ms | 9.2 ms | 8.1 ms |
| `lifecycle_snapshots/16x60`, the walk with a one-minute grid | 12.9 ms | 11.5 ms | 10.0 ms |
| `decoded_lifecycle`, the walk over the decoded capture | 1.121 s | 683 ms | 696 ms |
| `decoded_lifecycle_sorted`, the same walk one hour at a time | 1.483 s | 1.086 s | 663 ms |
| `decoded_lines_lifecycle`, the decoded capture parsed and walked | 3.609 s | 3.251 s | 3.129 s |

The before column is the walk that settled every redated message whole; the before and after figures are each the mean of two ten-sample runs of each side, alternated. The 0.1.17 column is one ten-sample run of the released tree, which also places every message among the messages of its instant - the parse by order, the walk by content - and dates each line of the capture by its own row header, so the decoded passes walk a capture spread over its own clock rather than stamped with one file time; placing costs nothing the run-to-run noise does not exceed. The [identity window](lifecycle.md#an-identity-is-yielded-once) is inside the after column: `decoded_lifecycle_undeduplicated`, the same walk remembering nothing, measured 686 ms and 868 ms in the two runs against 670 ms and 697 ms remembering a minute, so remembering costs nothing the run-to-run noise does not exceed.

```bash
cargo bench -p yggdryl --bench fix -- 'fix/pipeline/(lifecycle|lifecycle_same_shape|decoded_lifecycle|decoded_lifecycle_undeduplicated|decoded_lifecycle_sorted|decoded_lines_lifecycle)$|fix/pipeline/lifecycle_snapshots/' --sample-size 10
```

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/pipeline
```
