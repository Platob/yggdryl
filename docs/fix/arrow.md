# Arrow

A capture already in Arrow is read where it sits: `FixCodec::parse_text_arrow_reader` takes the batches a text reader answers - one row per line, the frame in the payload column - through the same [codec](capture.md#a-reader-is-the-whole-parse-surface) a captured line goes through, and returns the crate's one batch reader, [one row per message](#one-row-per-message) - so a day of session log reaches Parquet or Iceberg with nothing here knowing what either is. Its twin, `lifecycle_arrow_reader`, [chains](lifecycle.md) batches of FIX rows the way `lifecycle` chains a stream of messages. Both compose two converters any stage composes the same way - `messages`, rows to messages, and `arrow_reader`, messages to rows - and `write_arrow_reader` writes the rows back to the wire.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::parse_text_arrow_reader`, `lifecycle_arrow_reader`, `messages`, `arrow_reader`, `write_arrow_reader`, `FixCodec::DEFAULT_BATCH_BYTE_SIZE`, `DEFAULT_PAYLOAD_COLUMN`, `SOH` |
| Returns | `BatchReader`, the one type every encoding in the crate returns; Python gets a `pyarrow.RecordBatchReader`, JavaScript a `BatchReader` |
| Schema | answered before the first row is read, from the source's schema and the [dictionary](registry.md) alone, never from the data; `lifecycle_arrow_reader` answers the schema it read, `arrow_reader` the one it was given. `fix_schema(&registry, "fix")` answers the fixed row: 122 columns over 118 tags |
| Order | the source's own columns lead the row, the [fixed columns](capture.md#the-columns-are-the-folded-names) follow |
| Clash | a carried column whose folded name a FIX column takes is dropped in front and lands in that column, never renamed and never duplicated |
| Rows | one row per message, never one per line: a line carrying two frames is two rows, a JSON document is one row holding an `unknown` message with no entries, a payload that would not parse is one row holding an empty message, and a line carrying no message at all is no row - [what a line carries](decode.md) is the codec's rule; a row's carried source columns repeat over every message it answers |
| Batches | closed by estimated landed bytes against the codec's `batch_byte_size`, `DEFAULT_BATCH_BYTE_SIZE` (128 MiB), or by rows against `batch_row_size`, `DEFAULT_BATCH_ROW_SIZE` (32,768), whichever it reaches first, unless pinned: several small input batches accumulate into one, one larger than the target splits by rows in proportion, and a batch always holds at least one row |
| Pins | on the codec, for the whole run: `with_payload_column`, `with_capture_names`, `with_separator`, `with_null_values`, `try_with_direction`, `try_with_default_sending_time`, `with_batch_byte_size`, `with_batch_row_size`, `with_threads`, `with_include_msgtypes`, `with_exclude_msgtypes`, `with_snapshot_ns`; no dialect pin, because the registry is one namespace, and no version pin, because a version is what a line said |
| Stages | a call, never a flag: `FixCodec::lifecycle` composes over `messages` and `arrow_reader`, and `lifecycle_arrow_reader` is the walk composed for you; Rust-only `FixDedup` drops an adjacent republication, while lifecycle keeps finite-capture history in [Lifecycle](lifecycle.md) |
| Doors | capture Arrow parsing pools its whole input by batch, then merges rows in source order under the output byte and row closing targets; `arrow_reader` streams; `lifecycle` collects a finite capture to sort its history before it emits chained messages; an error item keeps its type |
| Per row | `beginstring` and `msgdirection` are parameters read from the row; a `sourceurl` column is neither a parameter nor a fill - it is [the capture's own](message.md#a-row-is-a-message-again) and is restated onto the output row from the source row; any other column named after a field - `msgpluginid` among them - fills it where the message did not state it; a capture `timestamp` is carried context, never a FIX clock |
| Errors | typed I/O, schema and parsing failures; a source batch of another schema than the first is a conflict; pooled parsing reports errors in source order, and dropping its reader joins dispatched work; `arrow_reader` yields its completed prefix, then an item's error, then fuses |
| Lazy | one worker parses with no pool; capture Arrow parsing pools batches, while line, message and write doors retain bounded 64-row chunks, two per worker; lifecycle retains finite-capture history |
| Wire | `write_arrow_reader` rebuilds every line from `fixentries` and never from the columns; a batch without that column is refused before a row is read |
| Bindings | Rust; Python (`FixCodec.parse_text_arrow_reader`, `lifecycle_arrow_reader`, `messages`, `arrow_reader`, `write_arrow_reader`); JavaScript (`parseTextArrowReader`, `lifecycleArrowReader`, `messages`, `arrowReader`, `writeArrowReader`); `FixDedup` is Rust-only |

## Use

One column of frames in, batches out, the capture's own columns still in front of them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    // A capture shaped the way a log reader shapes one: where the line was
    // read from, which line it was, and the frame itself.
    let capture = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?)
    .required_field("line");
    let values = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("file:///capture.log"),
        Scalar::from(7_i64),
        Scalar::from("recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|"),
    ])]);
    let batch = yggdryl::arrow::batch_from_value(&capture, &values)?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

    let read = FixCodec::new(registry)
        .with_threads(4)
        .parse_text_arrow_reader(source)?;

    // The schema is answered before a row is read: the capture leads it, the
    // tags follow, and the arrival record closes it.
    let columns: Vec<String> = read
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().clone())
        .collect();
    assert_eq!(&columns[..3], ["url", "rownum", "body"]);
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

    # The schema is answered before a row is read: the capture leads it, less
    # the column named after a FIX column, the fixed columns follow, and the
    # arrival record closes it.
    columns = [field.name for field in read.schema]
    assert columns[:5] == ["url", "rownum", "timestamp", "bridgesessionid", "body"]
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

    // The schema is answered before a row is read: the capture leads it, the
    // tags follow, and the arrival record closes it.
    assert.equal(read.field.fieldAt(0).name, 'url')
    assert.equal(read.field.fieldAt(2).name, 'body')
    assert.equal(read.field.fieldAt(read.field.fieldLen - 1).name, 'fixentries')

    const held = read.intoTable()
    assert.equal(held.numRows, 1, 'one ordinary frame per input row')
    assert.equal(held.getChild('symbol').get(0), 'AAPL')
    assert.equal(held.getChild('url').get(0), 'file:///capture.log')
    ```

## The source's columns lead the row

Where a line was read from is what a monitor orders and joins on, so the source's own columns lead the row and the fixed columns follow, exactly as they do for a [capture read line by line](capture.md#a-captures-own-columns-lead-the-row). Each emitted message receives the source row's carried values. A line carrying two frames therefore repeats the same URL, row number and timestamp. Which source columns survive a FIX column's claim on a name is decided once from the schema.

## A pin is on the codec, a stage is a call

What holds for a whole run is pinned on the codec once, and each pin is the per-run form of an argument the [byte readers](capture.md#a-reader-is-the-whole-parse-surface) read per call. There is no second options struct: the codec that reads a line is the codec that reads a batch.

| Pin | Builder | Default | Says |
| --- | --- | --- | --- |
| `payload_column` | `with_payload_column` | `body` (`DEFAULT_PAYLOAD_COLUMN`) | which column carries the frames: `utf8`, as the [text reader emits its rows](../media/text/index.md#row-schema), and `binary` accepted too on intake, for a capture another producer landed as bytes |
| `separator` | `with_separator` | `SOH` (`0x01`) | the separator a re-emitted line is written with, which is what `write_arrow_reader` writes; reading takes none, because a line already said which byte separated its fields |
| `null_values` | `with_null_values` | the crate's spellings | what means "nothing was sent" |
| `direction` | `try_with_direction` | the set's `Send` code, `S` | the code of tag 385's set a line that states none of its own takes on the batch door - no `msgdirection` column stating one, and no [rule of tag 385's `FIX:directions`](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) matching the prose in front of its payload; any spelling of a code of the set, resolved once, and `None` or `""` pins nothing |
| `batch_byte_size` | `with_batch_byte_size` | `DEFAULT_BATCH_BYTE_SIZE`, 128 MiB | the estimated landed bytes one output batch targets |
| `batch_row_size` | `with_batch_row_size` | `DEFAULT_BATCH_ROW_SIZE`, 32,768 | the rows one output batch targets; a batch closes on whichever bound it reaches first |
| `threads` | `with_threads` | available CPUs | workers for parsing and row conversion. Capture Arrow parse doors pool the whole input by batch, capped at this count; the concurrency bound is input batch count, never bytes. They merge ordered output using the closing targets `batch_byte_size` and `batch_row_size`; early reader drop joins dispatched work. Line, message and write doors instead retain 64-row chunks, at most two per worker. One uses no pool, zero reads as one, and `lifecycle` remains one ordered finite-capture walk. |
| `exclude_msgtypes` | `with_exclude_msgtypes` | `DEFAULT_REFUSED_MSGTYPES`: `Heartbeat`, `TestRequest`, the untyped row | the types [no row is built for](decode.md#a-type-nobody-asked-for-is-never-built) |
| `include_msgtypes` | `with_include_msgtypes` | empty, which reads every type the refusals leave | the types read, naming any clearing the default refusals |
| `capture_names` | `with_capture_names` | none | what a run's row-header captures are called, in the order a line answers them, so [`parse_text_line`](capture.md#a-reader-is-the-whole-parse-surface) reads a capture by position rather than by name |
| `default_sending_time` | `try_with_default_sending_time` | none, one UTC-now read per undated message | the [`SendingTime(52)`](capture.md#every-message-is-dated) a message stating none, on a row stating none, is dated by; an exact nanosecond UTC instant, else refused; pin it for a reproducible read |

What happens to a message on its way into a row is a stage, and a stage is a call over the stream rather than a flag on the reader: [`lifecycle_arrow_reader`](#chained-where-it-sits) chains batches, `lifecycle` [chains](lifecycle.md#in-a-batch-read) a finite capture, and Rust-only `FixDedup` drops an adjacent republication. Restating and filling are parse behavior. Python spells `snapshot_ns` as an exact integer nanosecond keyword; JavaScript spells `snapshotNs` as a `bigint`; absent, `null`, zero and negative values disable snapshots.

Lines to batches, with a lifecycle stage that sorts the finite capture: the walk names an order and its fill as one chain, and every row carries its `crossuuid`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;

    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ];
    let read = codec.arrow_reader(schema, codec.lifecycle(codec.parse_lines(lines)))?;

    let batch = read.into_iter().next().expect("one batch")?;
    assert_eq!(batch.num_rows(), 2);
    let chain = batch.column_by_name("crossuuid").expect("the chain column");
    assert_eq!(chain.null_count(), 0, "every message names its chain");
    let held = yggdryl::arrow::batch_to_value(&batch)?;
    let at = batch.schema().index_of("crossuuid")?;
    let chains: Vec<_> = held.as_sequence().expect("rows").iter().map(|row| row.get(at).cloned()).collect();
    assert_eq!(chains[0], chains[1], "one order, one chain");
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
    assert held.num_rows == 2
    chains = held.column("crossuuid").to_pylist()
    assert None not in chains, "every message names its chain"
    assert chains[0] == chains[1], "one order, one chain"
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
    assert.equal(held.numRows, 2)
    const chain = held.getChild('crossuuid')
    assert.equal(chain.nullCount, 0, 'every message names its chain')
    assert.deepEqual(chain.get(0), chain.get(1), 'one order, one chain')
    ```

## A column is the caller speaking per row

One column carries the frames; two more supply, per row, arguments the byte readers already take per call, and every other column is offered to the message by name. A separator is not among them: which byte separated a frame's fields is what the line itself said, so no row states it. Nor is a capture's `timestamp`: it is context carried in front, and the message's own [clocks](capture.md#every-message-is-dated) settle `currunix`.

| Column | Supplies |
| --- | --- |
| the payload column, named by the codec | the frame parsed |
| `beginstring` | the version the row was read at |
| `msgdirection` | the direction, stated: a code of tag 385's set, under any spelling |
| `sourceurl` | nothing: [the capture's own column](message.md#a-row-is-a-message-again) is what a reader said about the line, so it fills no message fact and is written straight into its own column of the row instead |
| any other column named after a field | that field, where the message did not state it - a `sendingtime` column among them, which outranks the codec's default sending time |
| one of the sixteen [event columns](../graph.md#columns) | nothing: they are the carrier's own facts - the [text line](../media/text/index.md#row-schema) each row is, as the reader stated it - so `curruuid` is each message's one source and the rest fill no message fact |

A column is the caller speaking per row and a pin is the caller speaking per run, so a column outranks the pin and both outrank what the frame infers: a row whose `beginstring` says `FIX.4.2` is read at 4.2 whatever the codec was pinned to, and its values translate through the code spellings 4.2 declares. A column absent, null or empty is silence, never an instruction and never an error.

A fill is named the way a key is: a column whose folded name resolves in the registry's one namespace - the canonical fold, then an alias fold, so a `MsgSessionId` capture reaches the crate's own `msgsessionid` and a `msgpluginid` column the crate's `msgpluginid`. It is row-only: never an entry, so it is not in `fixentries`, not re-emitted by `write_arrow_reader` and not in the arrival digest; a value the field cannot hold fills nothing rather than a null; a column named by a tag's digits fills nothing, because a name is what reaches a field; and a name reaching one of [the capture's own columns](message.md#a-row-is-a-message-again) fills nothing either, because no message holds a *fact* for one: the cell is carried instead, and stated again at its own column. Which columns fill is decided once, from the schema and the dictionary, rather than per row.

`beginstring` and `msgdirection` are FIX columns' own names, so they are not carried in front: the row's `beginstring` and `version` columns say what a `beginstring` column decided. A record carrying only a payload column behaves exactly as the byte reader behaves, which is what makes this an entry point rather than a second contract.

### A bridge log names what it fills

`yggdryl::ULBRIDGE_ROWHEADER` is the [row header](../media/text/index.md#row-schema) a ULBridge log writes in front of every line - a clock, a thread bracket, the plugin that wrote the line and its level - with every capture named for what it fills. Rust names the constant; the regex is the same text, ending in one space, in any binding's `rowheader`. A capture named for one of the sixteen [event columns](../graph.md#columns) the line is stated in - `seqnum`, `state`, `crosscode`, a lifecycle instant - fills nothing on either door: it is the line's own fact, and what a line says about a message is its identity, stated as the message's source.

```text
^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<msgthreadid>[1-9]\d*)(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] \[(?P<msgpluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) 
```

| Capture | Typed as | In a batch read |
| --- | --- | --- |
| `timestamp` | datetime | the capture's own column, leading the row as context; the message's clocks stay its own |
| `msgthreadid` | int64 | the capture's own column, leading the row |
| `msgsessionid` | utf8, nullable | the session instance the bridge handled the line on; fills `msgsessionid` (65032) rather than leading the row, and never over a reading the message stated. Not the counterparty session a bridge row spells `SESSIONID` for: two connections to one counterparty are two instances |
| `msgctxid` | utf8, nullable | fills `msgctxid` (65008) |
| `msgseqnum` | int64, nullable | fills `MsgSeqNum(34)` where the frame did not carry it |
| `msgpluginid` | utf8 | fills `msgpluginid` (65009), the plugin that logged the line, and selects nothing |
| `level` | utf8 | the capture's own column |

The session instance, the context and the sequence number are optional as a whole, so a line carrying only its thread still frames and leaves them null rather than failing the row.

Every capture is named for the field it fills, so the registry's one namespace is what lands it and nothing translates in between. The bridge writes these in camel case - `msgCtxId`, `seqNum` - and they used to be captured that way, with a table mapping `seqnum` onto tag 34; naming the captures for the fields retires that table. The session instance, the context and the plugin are the [capture's own facts](message.md#typed-tags), held by `FixCapture` rather than in the content row, so they are outside the code the message's content digests to: what a bridge wrote around one message is not that message; where the row header brackets both a session instance and a context, the two name the chain through the [cross code](lifecycle.md#a-chain-is-named-by-its-cross-code), which is the chain's identity and not the message's. Where the line was read from is not among them at all - that is [the capture's own column](message.md#a-row-is-a-message-again), which a message carries and never states, because the same message read from a second copy of one day's log is the same message.

The plugin is a fill and nothing more: it lands in the crate's own `msgpluginid` column by name, like any capture named after a field, and selects no dictionary and no version - the registry is one namespace, and which dictionaries a field belongs to is the field's own `FIX:branches`, which no read consults.

=== "Rust"

    ```rust
    use yggdryl::text::TextOptions;
    use yggdryl::{DataType, ULBRIDGE_ROWHEADER};

    let options = TextOptions::new().try_with_rowheader(ULBRIDGE_ROWHEADER)?;
    let captures = options.source_field()?;
    let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
    assert!(names.ends_with(&["timestamp", "msgthreadid", "msgsessionid", "msgctxid", "msgseqnum", "msgpluginid", "level"]));
    // Typed from the pattern before a byte is read.
    assert_eq!(captures.field("msgseqnum")?.dtype(), &DataType::Int64);
    ```

## One row per message

A source row is read for every message it carries, so a capture answers one row per message and never one per line. One ordinary frame is one row, and a line carrying two frames is two, each re-emitting only its own bytes. A line carrying a [JSON document](capture.md#a-json-document-is-one-message-stating-nothing) is one row whatever the document names - a bulk or wildcard answer as much as a single one - holding an `unknown` message with no entries and no `msgtype`, its carried columns beside it. A line carrying no message at all emits zero rows: a bridge's own prose is a line and not a row. The one refusal that still yields a row is a payload that was there and would not parse: it holds an empty message, so malformed syntax never fails a batch; a stated clock that does not read - `SendingTime(52)` and `TransactTime(60)`, the two the registry types as instants, and `currunix` and `creaunix`, the two the identity is settled against - is a located error item. What a line carries is the codec's rule, stated in [decode](decode.md); a caller wanting one row per *line* reads the capture with the [text reader](../media/text/index.md#row-schema), which answers every line whether or not a message is in it. Join a parsed capture by its carried source identifier rather than assuming row positions still align. `parse_text_line` is the same reading of one line, and answers the iterator when expansion is wanted.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    // Row 7 carries two frames; row 8 is a sentence, which is no row at all.
    let body = "8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|";
    let silent = "heartbeat emitted seq=7";
    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?).required_field("capture");
    let rows = Scalar::from_sequence([
        Scalar::from_sequence([Scalar::from(7_i64), Scalar::from(body)]),
        Scalar::from_sequence([Scalar::from(8_i64), Scalar::from(silent)]),
    ]);
    let batch = yggdryl::arrow::batch_from_value(&field, &rows)?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
    let reader = FixCodec::new(registry).parse_text_arrow_reader(source)?;
    let mut count = 0;
    for batch in reader {
        let values = yggdryl::arrow::batch_to_value(&batch?)?;
        for row in values.as_sequence().expect("rows") {
            // Each row keeps the source row's own columns in front.
            assert_eq!(row.get(0), Some(&Scalar::from(7_i64)));
            assert_eq!(row.get(1), Some(&Scalar::from(body)));
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

`messages` reads a stream of batches back as the messages that made them, each row through [`FixMsg::from_row`](message.md#a-row-is-a-message-again) under the schema read off the source - its entries rebuilt from `fixentries`, so the message re-emits its line, digests, restates and stamps exactly as the parsed one did - at the cost of the values the row already holds, and no parse. `arrow_reader` is the other direction: a stream of messages into batches under a schema, each through `FixMsg::into_row`, closed on the bytes each row lands as. The two invert each other over everything a message holds, [the capture's own columns](message.md#a-row-is-a-message-again) included: `messages` reads each row's own cells into the message it makes, carried and never content, and `into_row` states them again at their columns, so a `messages` -> stage -> `arrow_reader` composition keeps them whatever order the stage answers in - `lifecycle_arrow_reader` and `format_arrow_reader` are that composition spelled once. It is what lets a stage run over a capture already landed in Arrow; the example ends [back on the wire](#back-to-the-wire). Where the codec reads on [several threads](#a-pin-is-on-the-codec-a-stage-is-a-call), a batch's rows are read a chunk at a time, each row's message made on some thread and answered in row order.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');
    let schema = fix_schema(&registry, "fix")?;

    let lines = ["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|", "8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // Into batches, and back: the same arrival record, the same digest, and
    // the same row again.
    let again: Vec<FixMsg> = codec
        .messages(codec.arrow_reader(schema.clone(), parsed.clone())?)
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(again.len(), parsed.len());
    for (held, message) in again.iter().zip(&parsed) {
        assert_eq!(held.entries(), message.entries());
        assert_eq!(held.digest(), message.digest());
        assert_eq!(held.into_row(&schema)?, message.into_row(&schema)?);
    }

    // And out to the wire: one line per row, rebuilt from the message's
    // own facts and its entries - what arrived, and what the dictionary
    // derived from it.
    let mut written = Vec::new();
    assert_eq!(codec.write_arrow_reader(codec.arrow_reader(schema, again)?, &mut written)?, 2);
    assert_eq!(
        String::from_utf8(written)?,
        "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|59=0|10=0|\n8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|59=0|381=637.5|10=0|\n",
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

    # Into batches, and back: the same arrival record, the same digest, and
    # the same row again.
    again = list(codec.messages(codec.arrow_reader(schema, parsed)))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        assert held.entries() == message.entries()
        assert held.digest() == message.digest()
        assert held.into_row(schema) == message.into_row(schema)

    # And out to the wire: one line per row, rebuilt from the message's own
    # facts and its entries - what arrived, and what the dictionary derived
    # from it.
    sink = io.BytesIO()
    assert codec.write_arrow_reader(codec.arrow_reader(schema, again), sink) == 2
    assert sink.getvalue().decode().splitlines() == [
        "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|59=0|10=0|",
        "8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|59=0|381=637.5|10=0|",
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

    // Into batches, and back: the same arrival record, the same digest, and
    // the same row again.
    const again = [...codec.messages(codec.arrowReader(schema, parsed))]
    assert.equal(again.length, parsed.length)
    again.forEach((held, at) => {
      assert.deepEqual(held.entries(), parsed[at].entries())
      assert.deepEqual(held.digest(), parsed[at].digest())
      assert.ok(held.intoRow(schema).equals(parsed[at].intoRow(schema)))
    })

    // And out to the wire: anything with write(chunk) is a sink - a stream,
    // a socket, an array - one line per row, rebuilt from the message's own
    // facts and its entries.
    const chunks = []
    assert.equal(codec.writeArrowReader(codec.arrowReader(schema, again), { write: (chunk) => chunks.push(Buffer.from(chunk)) }), 2)
    assert.deepEqual(Buffer.concat(chunks).toString().split('\n').slice(0, 2), [
      '8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|59=0|10=0|',
      '8=FIX.4.4|35=8|17=E1|31=12.75|32=50|37=O9|59=0|381=637.5|10=0|',
    ])
    ```

## Back to the wire

`write_arrow_reader` streams a batch back out as lines, one per row and so [one per message](#one-row-per-message) - a source line that held two frames comes back as two lines, each the bytes its own frame arrived as - rebuilt from each row's `fixentries` and never from its columns: the fixed columns are a *reading* of the message, so a frame rebuilt from them would be one nobody sent. Each line is [`into_bytes`](encode.md) with the codec's `separator`, `SOH` unless pinned, then a newline, and the count of lines is answered; the [round trip above](#rows-are-messages-again-and-messages-rows) ends there. The walk is pre-order, so a group's members follow the counter that heads them, exactly as they arrived. A batch carrying no `fixentries` column cannot be written and says so before a row is read. One batch is pulled, its rows written, and it is dropped; no buffer bigger than a row is held.

## Chained where it sits

`lifecycle_arrow_reader` is [`lifecycle`](lifecycle.md) over batches: each row becomes its message through [`FixMsg::from_row`](message.md#a-row-is-a-message-again), the walk states it as the one after the live message of its chain, and it is written back under the **same** schema, so a carried column returns to its place and the content is carried through untouched. Nothing is parsed again.

A carried column returns to its place because the message carries it: a message holds no *fact* about the reading it arrived through, so the door reads each row's own cells into the message as [what it carries](message.md#a-row-is-a-message-again), provenance and never content, and `into_row` states them again at their own columns. The pairing is by message and never by position - the walk answers messages in their own order, which a capture's lines are routinely not in, so a row's `body` would otherwise come back attached to a different message's row. The batches it answers close on the bytes each row lands as, against the codec's `batch_byte_size`; the walk itself collects its source, because a capture's lines are written in the order a bridge logged them and two messages of one chain routinely arrive out of their own order.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar, StructType};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(registry);

    // An order and the fill that answers it, on two lines of one capture.
    let capture = DataType::from(StructType::from_fields([DataType::utf8().required_field("body")])?).required_field("line");
    let lines = [
        "8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        "8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ];
    let batch = yggdryl::arrow::batch_from_value(
        &capture,
        &Scalar::from_sequence(lines.map(|line| Scalar::from_sequence([Scalar::from(line)]))),
    )?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

    let read = codec.parse_text_arrow_reader(source)?;
    let chained = codec.lifecycle_arrow_reader(read)?;
    let held = chained.into_iter().next().expect("one batch")?;

    // The same schema, and one chain: the fill follows the order, names it
    // and shares its identity.
    assert_eq!(held.num_rows(), 2);
    let cross = held.column_by_name("crossuuid").expect("the chain column");
    assert_eq!(cross.null_count(), 0);
    let rows = yggdryl::arrow::batch_to_value(&held)?;
    let rows = rows.as_sequence().expect("rows");
    let at = |name: &str| held.schema().index_of(name).expect("a column");
    assert_eq!(rows[0].get(at("crossuuid")), rows[1].get(at("crossuuid")));
    assert_eq!(rows[1].get(at("seqnum")), Some(&Scalar::from(1_u64)));
    assert!(rows[1].get(at("prevuuid")).is_some_and(|held| !held.is_null()));
    // The content is what each line stated, carried through untouched.
    assert_eq!(rows[1].get(at("ordstatus")).and_then(Scalar::as_str), Some("2"));
    ```

## Edges

- Ordinary unframed text produces no row, and an empty payload none either; a payload that was there and would not parse produces one row holding an empty message. Output counts follow message expansion.
- A carried column whose folded name a FIX column takes is dropped in front rather than renamed - two columns of one name is not a schema - and what it stated lands in that FIX column.
- A `msgdirection` column is the row's stated direction, read as a parameter - any spelling of a code of tag 385's set, stored as the code - and it outranks the reading of the line and the codec's pin; the FIX column carries it and no second column repeats it.
- A `timestamp` column is carried context: it leads the row as a column of its own and never dates the message - `SendingTime` does - and it is a fact about the capture, so it is outside the code the message's content digests to.
- A fill never overrides what the frame stated: a `msgseqnum` capture beside a frame carrying `34=` leaves that field to the frame.
- A fill is row-only: never an entry, never in `fixentries`, never re-emitted by `write_arrow_reader`, never in the arrival digest.
- `messages` reads a row carrying the settled values - `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid` - as a replayable message; a row leaving one of them null is the schema's refusal, since the fixed row declares them required.
- A batch closes on the bytes each row lands as - the leaves of every column the row fills and a per-row width - so a source batch of any size splits by what its messages land as, and a line answering two messages is charged twice, once per row.
- A `batch_byte_size` of `0` or `1` is a batch a row: the target is where a batch closes, never a bound a row must fit under.
- `parse_text_arrow_reader` on a source with no column named as the payload column, or one holding neither text nor bytes under it -> refused before a row is read, naming the column. `parse_text_line` has no column to name: a line's body is a typed field, so a line whose body is empty answers no message at all and nothing else is refusable.
- `messages` on a source whose schema makes no root field -> one error item; a later batch of another schema -> a conflict item; a row that is not a FIX row -> an error item; each fuses the stream.
- `arrow_reader` under a schema the message cannot fill whole -> the row's own refusal, at that row; an `Err` item in its stream - a line `parse_lines` refused - yields the completed prefix, then the error, and fuses the reader, so a capture that must survive a payload the codec cannot read goes through `parse_text_arrow_reader`, where such a payload is an empty message and not an error.
- `arrow_reader` over messages carrying no entries - built by hand, or read back from rows holding only the fixed columns - charges each the leaves of its row, so a projection without the arrival record is bounded by the same target.
- `write_arrow_reader` on a batch without `fixentries` -> refused before a row is read; a row whose message held no pairs -> an empty line, still counted.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single row.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix batch::the_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix
    cargo test -p yggdryl --test fix dataset::
    cargo test -p yggdryl --test fix dataset::ulbridge_dataset_allocation_profile_is_sequential_and_staged -- --exact --nocapture --test-threads=1
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python scripts/check_docs_examples.py --lang python
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/fix/*.test.js"
    python scripts/check_docs_examples.py --lang javascript
    ```

## Performance

The staged Rust integration profile above counts allocations over `ulbridge.log`
for text framing, codec parsing, fixed-row materialization, typed row holders,
and Arrow output. It reports first and second passes separately; requested bytes
are cumulative allocation requests, not peak memory. Row and arrival-entry
materialization write into their final shared storage without temporary
vectors. Timing benchmarks remain separate from these allocation counts.

`fix/pipeline`, the whole path a desk takes over a bridge's own log: `rust/tests/fix/ulbridge.log`, a second of a ULBridge's capture beside every shape a bridge writes - a Jolokia exchange whose answer is a JSON document the codec does not read, FIXML behind a verb, frames spelled with `^A` and `<SOH>`, a `35=UL` frame packing a group inside a group, bridge rows of a hundred named keys, a statistics line, an empty body, the bridge's sixteen handed-over lines and the fifteen of a cancel/reject flow - repeated 64 times: 9,216 lines, 6,080 messages, 13.9 MB. Every stage runs over the same corpus on its own, so a figure is per line of a real capture rather than of one shape, and a row is one per message rather than one per line ([decode](decode.md)). The current smoke covers six pool cases: one, two and four workers for each of `parse_text_arrow_reader` and `parse_arrow_messages`; it claims no current speed or throughput. The historical release run used thin LTO, one codegen unit, one Linux x86_64 container, Intel Xeon @ 2.10 GHz, 4 cores, 15 GiB, no other build running, load average 1.1 when the run ended; rustc 1.94.1; the shipped dictionary alone; `cargo bench -p yggdryl --bench fix -j 2 -- 'fix/pipeline/(text_read|parse_text_arrow_reader|parse_lines|parse_text_lines_msgpluginid|into_row|arrow_reader|lifecycle|digest)$' --sample-size 10`, ten samples a case. That release run predates the message becoming a typed market event over a content row, which moved the fill into the parse and the chain onto the graph's one walk, so every figure below is historical and due regeneration.

| stage | estimate | throughput | per line, row or message |
| --- | --- | --- | --- |
| `text_read`, the row header framed, each line numbered, classified and read for its direction | 99 ms | 140.5 MB/s | 10.7 us |
| `parse_text_arrow_reader`, the whole path into fixed rows | 2.12 s | 6.6 MB/s | 229.9 us |
| `parse_lines`, the codec alone over the framed bodies | 1.19 s | 11.7 MB/s | 129.0 us |
| `parse_text_lines_msgpluginid`, the line reader with each row naming its plugin | 1.18 s | 11.8 MB/s | 128.4 us |

The release estimates above are historical. A current debug counting-allocator
profile over the ULBridge fixture measures requests and requested bytes, not
CPU time or throughput:

| Warm stage | Original baseline | Current |
| --- | ---: | ---: |
| `FieldRecord::into_arrow_batch` requests / bytes | 805,081 / 127,257,638 | 528,256 / 81,233,104 |
| `FixMsg::into_row` requests / bytes | 10,470 / 3,732,676 | 8,239 / 2,462,660 |

In that historical release run, the text stage was a small fraction of the whole and the codec about half; the rest was the row landing in Arrow.

What remains is attributed rather than argued, by the `text_scan` group of the text benchmark and the `fix/line` group of this one, which measure the scan and the codec shape by shape. On the codec path the builder is 60-90% of every shape; in front of it, a bridge row of a hundred pairs is read into a tree of counted ranges of its page, and each pair then crosses one more stage on its way to the builder. That is the cost of ranges: every key and value a message records is a range of the line it came from, so a data field re-slices to its stated length and a frame re-emits byte for byte without a copy, and it is paid on every pair whether or not a reader ever asks for the range. It goes only with a reader that builds the codec's pairs from the scanner's spans without the tree between them, which is a change to what an entry is and not to how fast it is read.

What a message costs after it is built, each pass over fresh clones of the 6,080 messages, so every number is the pass over a message the stream just built:

| pass | estimate | per message |
| --- | --- | --- |
| `into_row`, the message read against the fixed schema | 350 ms | 57.5 us |
| `arrow_reader`, the rows landed in batches | 439 ms | 72.2 us |
| `lifecycle`, the stamp that joins a message to its order's life | 311 ms | 51.1 us |
| `digest`, the arrival record's hash | 21 ms | 3.5 us |

A row pays `into_row` and its share of the batch; it pays for the walk only when the caller composes that [stage](#a-pin-is-on-the-codec-a-stage-is-a-call), and for the fill inside the parse that built it. Reading a message against the fixed schema is a lookup per column, most of them misses answered by a name table the message builds on its first projection, and a FIX column a message implied rather than stated one evaluation of the dictionary's own compiled derivation over the columns it reads; the batch is the rows canonicalized and built into one `RecordBatch`, of which the arrival record is the one nested column. The fill inside a parse is every child resolved against the dictionary once, the specification's retirements of its tags applied from the crate's table and the rules a registry states of its own read borrowed, then the registry's [derivations](registry.md#a-field-carries-how-it-is-derived) - compiled and bound once per registry, gathered into one working row per message by tag, swept to a fixpoint, most answers null on a message that stated everything, and one rebuild landing what derived; the walk is a chain lookup, one statement of the predecessor's identity, instant and place, and the identity settled again. The digest is a hash over the arrival record and nothing else.

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/pipeline
```
