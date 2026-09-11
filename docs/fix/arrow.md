# Arrow

A capture already in Arrow is read where it sits: `FixCodec::parse_text_arrow_reader` takes the batches a text reader answers - one row per line, the frame in the payload column - through the same [codec](capture.md#a-reader-is-the-whole-parse-surface) a captured line goes through, and returns the crate's one batch reader - so a day of session log reaches Parquet or Iceberg with nothing here knowing what either is. Its twin, `enrich_messages_arrow_reader`, fills batches of FIX rows the way `enrich_messages` fills a stream of messages. Both compose two converters any stage composes the same way - `messages`, rows to messages, and `arrow_reader`, messages to rows - and `write_arrow_reader` writes the rows back to the wire.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::parse_text_arrow_reader`, `enrich_messages_arrow_reader`, `messages`, `arrow_reader`, `write_arrow_reader`, `FixCodec::DEFAULT_BATCH_BYTE_SIZE`, `DEFAULT_PAYLOAD_COLUMN`, `SOH` |
| Returns | `BatchReader`, the one type every encoding in the crate returns; Python gets a `pyarrow.RecordBatchReader`, JavaScript a `BatchReader` |
| Schema | answered before the first row is read, from the source's schema and the [dictionary](registry.md) alone, never from the data; `enrich_messages_arrow_reader` answers the schema it read, `arrow_reader` the one it was given |
| Order | the source's own columns lead the row, the [fixed columns](capture.md#the-columns-are-the-folded-names) follow |
| Clash | a carried column whose folded name a FIX column takes is dropped in front and lands in that column, never renamed and never duplicated |
| Rows | one output row per emitted message; bulk and wildcard bodies expand, with carried source columns repeated; a line the reader refuses is a row holding an empty message, so a row in is a row out |
| Batches | closed by raw bytes against the codec's `batch_byte_size`, `DEFAULT_BATCH_BYTE_SIZE` (128 MiB) unless pinned: several small input batches accumulate into one, one larger than the target splits by rows in proportion, and a batch always holds at least one row |
| Pins | on the codec, for the whole run: `with_payload_column`, `with_capture_names`, `with_separator`, `with_branch`, `with_version`, `with_null_values`, `with_direction`, `with_batch_byte_size` |
| Stages | a call, never a flag: `enrich_messages_arrow_reader`, `FixCodec::lifecycle`, `FixMsg::into_latest` and `FixDedup` compose over `messages` and `arrow_reader` |
| Per row | `branch`, `beginstring`, `direction` and `timestamp` are parameters read from the row; any other column named after a field fills it where the message did not state it |
| Errors | typed I/O, schema and parsing failures; a source batch of another schema than the first is a conflict; malformed bulk input reports its location and stops the stream |
| Lazy | one source batch held at a time; configuration cursors consumed incrementally under the output batch bound |
| Wire | `write_arrow_reader` rebuilds every line from `nofixentries` and never from the columns; a batch without that column is refused before a row is read |
| Bindings | Rust; Python (`FixCodec.parse_text_arrow_reader`, `enrich_messages_arrow_reader`, `messages`, `arrow_reader`, `write_arrow_reader`); JavaScript (`parseTextArrowReader`, `enrichMessagesArrowReader`, `messages`, `arrowReader`, `writeArrowReader`); `FixDedup` is Rust-only |

## Use

One column of frames in, batches out, the capture's own columns still in front of them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    // A capture shaped the way a log reader shapes one: where the line was
    // read from, which line it was, and the frame itself.
    let capture = DataType::from_fields([
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Binary.required_field("body"),
    ])?
    .required_field("line");
    let values = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("file:///capture.log"),
        Scalar::from(7_i64),
        Scalar::from(b"recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|".to_vec()),
    ])]);
    let batch = yggdryl::arrow::batch_from_value(&capture, &values)?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);

    let read = FixCodec::new(registry).parse_text_arrow_reader(source)?;

    // The schema is answered before a row is read: the capture leads it, the
    // tags follow, and the two lists close it.
    let columns: Vec<String> = read
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().clone())
        .collect();
    assert_eq!(&columns[..3], ["url", "rownum", "body"]);
    assert_eq!(&columns[columns.len() - 2..], ["nofixentries", "nounmappedfixentries"]);

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
            "senderSessionId": pa.array(["0123abcd", None], pa.utf8()),
            "body": pa.array(
                [
                    b"recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|",
                    b"8=FIX.4.4|35=D|11=ORDER-2|55=MSFT|10=0|",
                ],
                pa.binary(),
            ),
        }
    )

    codec = FixCodec(registry, version="FIX.4.4")
    read = codec.parse_text_arrow_reader(capture.to_reader())

    # The schema is answered before a row is read: the capture leads it, less
    # the two columns named after FIX columns, the fixed columns follow, and
    # the two lists close it.
    columns = [field.name for field in read.schema]
    assert columns[:3] == ["url", "rownum", "body"]
    assert columns[-2:] == ["nofixentries", "nounmappedfixentries"]

    held = read.read_all()
    assert held.num_rows == 2, "one ordinary frame per input row"
    assert held.column("symbol").to_pylist() == ["AAPL", "MSFT"]
    assert held.column("url")[0].as_py() == "file:///capture.log"
    # The verb in front of the frame beats the direction the codec defaults to.
    assert held.column("msgdirection").to_pylist() == ["RECV", "SENT"]
    # The row's clock stamps the message, and a capture named after a field
    # fills it - where the row stated one.
    assert held.column("timestamp").cast(pa.timestamp("us", "UTC")).to_pylist() == clocks
    assert held.column("sendersessionid").to_pylist() == ["0123abcd", None]
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
        [Buffer.from('recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|')],
        new arrow.Binary(),
      ),
    })

    const read = new fix.FixCodec(registry).parseTextArrowReader(BatchReader.from(capture))

    // The schema is answered before a row is read: the capture leads it, the
    // tags follow, and the two lists close it.
    assert.equal(read.field.fieldAt(0).name, 'url')
    assert.equal(read.field.fieldAt(2).name, 'body')
    assert.equal(read.field.fieldAt(read.field.fieldLen - 2).name, 'nofixentries')

    const held = read.intoTable()
    assert.equal(held.numRows, 1, 'one ordinary frame per input row')
    assert.equal(held.getChild('symbol').get(0), 'AAPL')
    assert.equal(held.getChild('url').get(0), 'file:///capture.log')
    ```

## The source's columns lead the row

Where a line was read from is what a monitor orders and joins on, so the source's own columns lead the row and the fixed columns follow, exactly as they do for a [capture read line by line](capture.md#a-captures-own-columns-lead-the-row). Each emitted message receives the source row's carried values. A bulk body can therefore repeat the same URL, row number and timestamp. Which source columns survive a FIX column's claim on a name is decided once from the schema.

## A pin is on the codec, a stage is a call

What holds for a whole run is pinned on the codec once, and each pin is the per-run form of an argument the [byte readers](capture.md#a-reader-is-the-whole-parse-surface) read per call. There is no second options struct: the codec that reads a line is the codec that reads a batch.

| Pin | Builder | Default | Says |
| --- | --- | --- | --- |
| `payload_column` | `with_payload_column` | `body` (`DEFAULT_PAYLOAD_COLUMN`) | which column carries the bytes |
| `separator` | `with_separator` | `SOH` (`0x01`) | the separator a re-emitted line is written with, which is what `write_arrow_reader` writes; reading takes none, because a line already said which byte separated its fields |
| `branch` | `with_branch` | none | the dialect, so no row infers one |
| `version` | `with_version` | none | the version values are translated at, never what a column is called; unpinned, each row answers for itself |
| `null_values` | `with_null_values` | the crate's spellings | what means "nothing was sent" |
| `direction` | `with_direction` | `SENT` | the direction a line that states none of its own took - no verb in front of its payload, and no [document saying which half it is](registry.md#a-direction-is-the-verb-in-front-of-the-payload) |
| `batch_byte_size` | `with_batch_byte_size` | `DEFAULT_BATCH_BYTE_SIZE`, 128 MiB | the raw bytes one output batch targets |
| `capture_names` | `with_capture_names` | none | what a run's row-header captures are called, in the order a line answers them, so [`parse_text_line`](capture.md#a-reader-is-the-whole-parse-surface) reads a capture by position rather than by name |

What happens to a message on its way into a row is a stage, and a stage is a call over the stream rather than a flag on the reader: [`enrich_messages_arrow_reader`](#filled-where-it-sits) fills batches, `lifecycle` [stamps](lifecycle.md#in-a-batch-read) a stream, `into_latest` [restates](message.md#restated-at-the-dictionarys-newest-version) a message and `FixDedup` drops an adjacent republication - each composed as `arrow_reader(schema, stage(messages(reader)))`, so the order stages run in is the order they are written in and nothing runs unasked. Python spells the pins as keywords on `FixCodec(registry, *, branch, version, separator, payload_column, capture_names, null_values, direction, batch_byte_size)`, JavaScript as the options object of `new fix.FixCodec(registry, { ... })` in camelCase; `separator` is the byte's integer value, and `direction` takes `"sent"`, `"recv"` or `"unknown"`.

Messages to batches, with one stage between them: the lifecycle stamps four messages of one order's life, and every row carries the chain.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;

    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.000|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let stamped: Vec<FixMsg> = codec.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    let read = codec.arrow_reader(schema, stamped.into_iter().map(Ok))?;

    let batch = read.into_iter().next().expect("one batch")?;
    assert_eq!(batch.num_rows(), 4);
    let chain = batch.column_by_name("persistentid").expect("the chain column");
    assert_eq!(chain.null_count(), 0, "every message of the life names its chain");
    let held = yggdryl::arrow::batch_to_value(&batch)?;
    let at = batch.schema().index_of("persistentid")?;
    let chains: Vec<_> = held.as_sequence().expect("rows").iter().map(|row| row.get(at).cloned()).collect();
    assert!(chains.iter().all(|held| held == &chains[0]), "one order, one chain");
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
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.000|10=0|",
    ]
    read = codec.arrow_reader(schema, codec.lifecycle(codec.parse_lines(lines)))

    held = read.read_all()
    assert held.num_rows == 4
    chains = held.column("persistentid").to_pylist()
    assert None not in chains, "every message of the life names its chain"
    assert len(set(chains)) == 1, "one order, one chain"
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
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|',
      '8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.000|10=0|',
    ].map((line) => Buffer.from(line))
    const read = codec.arrowReader(schema, codec.lifecycle([...codec.parseLines(lines)]))

    const held = read.intoTable()
    assert.equal(held.numRows, 4)
    const chain = held.getChild('persistentid')
    assert.equal(chain.nullCount, 0, 'every message of the life names its chain')
    const first = Buffer.from(chain.get(0))
    assert.ok([...chain].every((held) => Buffer.from(held).equals(first)), 'one order, one chain')
    ```

## A column is the caller speaking per row

One column carries the bytes; four more supply, per row, arguments the byte readers already take per call, and every other column is offered to the message by name. A separator is not among them: which byte separated a frame's fields is what the line itself said, so no row states it.

| Column | Supplies |
| --- | --- |
| the payload column, named by the codec | the bytes parsed |
| `branch` | the dialect |
| `beginstring` | the source version |
| `direction` | the direction, stated |
| `timestamp` | the row's own clock, which [stamps the message](capture.md#every-message-is-dated-and-versioned) ahead of any clock the frame carries |
| any other column named after a field | that field, where the message did not state it |

A column is the caller speaking per row and a pin is the caller speaking per run, so a column outranks the pin and both outrank what the frame infers: a row whose `beginstring` says `FIX.4.2` is read at 4.2 whatever the codec was pinned to, and its values translate through the code spellings 4.2 declares. A column absent, null or empty is silence, never an instruction and never an error.

A fill is named the way a key is: a column whose folded name resolves in the message's branch, then the standard one, then any dictionary the registry holds - so a `senderSessionId` capture reaches the crate's own `sendersessionid` - and last through the bridge's own spellings of standard fields, `seqNum` reaching `MsgSeqNum(34)`. It is row-only: never an entry, so it is not in `nofixentries`, not re-emitted by `write_arrow_reader` and not in `msghash`; a value the field cannot hold fills nothing rather than a null; and a column named by a tag's digits fills nothing, because a name is what reaches a field. Which columns fill is decided once, from the schema and the dictionary, rather than per row.

`direction` is still carried into the row, because a monitor needs to see the value it supplied rather than infer that it was used. `beginstring` and `timestamp` are FIX columns' own names, so they are not carried in front: the row's `beginstring` and `version` columns say what a `beginstring` column decided, and its `timestamp` column holds what a `timestamp` column stated. A record carrying only a payload column behaves exactly as the byte reader behaves, which is what makes this an entry point rather than a second contract.

### A bridge log names what it fills

`yggdryl::ULBRIDGE_ROWHEADER` is the [row header](../media/text.md#row-schema) a ULBridge log writes in front of every line - a clock, a thread bracket, the plugin that wrote the line and its level - with every capture named for what it fills. Rust names the constant; the regex is the same text, ending in one space, in any binding's `rowheader`.

```text
^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<threadId>[1-9]\d*)(?:-(?P<senderSessionId>[0-9a-f]{8}):(?P<msgCtxId>[0-9a-f]{10}):(?P<seqNum>\d+))?\] \[(?P<pluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) 
```

| Capture | Typed as | In a batch read |
| --- | --- | --- |
| `timestamp` | datetime | the row's clock, so the message's `timestamp` |
| `threadId` | int64 | the capture's own column, leading the row |
| `senderSessionId` | utf8, nullable | the session instance the bridge handled the line on; folds onto `sendersessionid` (65007), so it fills that column rather than leading the row, and never over a reading the message stated |
| `msgCtxId` | utf8, nullable | fills `msgctxid` (65008) |
| `seqNum` | int64, nullable | fills `msgseqnum` (34) where the frame did not carry it; carried in front too, since no FIX column is named `seqnum` |
| `pluginid` | utf8 | fills `pluginid` (65009), the plugin that logged the line; where its text names a dialect the dictionary declares, it is also the branch the row is read under |
| `level` | utf8 | the capture's own column |

The session uid, the context and the sequence number are optional as a whole, so a line carrying only its thread still frames and leaves them null rather than failing the row.

The plugin is read twice over, from one cell: it fills the crate's own `pluginid` column by name, like any capture named after a field, and where its text is the name or an alias of a dialect the dictionary declares it is the branch the row is read under, outranking the codec's own pin. A plugin no branch is named after leaves that pin standing. The two session names are only ever what the line itself spells, through the `ULFROMSESSIONNAME` and `ULTOSESSIONNAME` aliases they answer to.

=== "Rust"

    ```rust
    use yggdryl::media::text::TextOptions;
    use yggdryl::{DataType, ULBRIDGE_ROWHEADER};

    let options = TextOptions::new().try_with_rowheader(ULBRIDGE_ROWHEADER)?;
    let captures = options.source_field()?;
    let names: Vec<&str> = captures.fields().iter().map(yggdryl::Field::name).collect();
    assert!(names.ends_with(&["timestamp", "threadId", "senderSessionId", "msgCtxId", "seqNum", "pluginid", "level"]));
    // Typed from the pattern before a byte is read.
    assert_eq!(captures.field("seqNum")?.dtype(), &DataType::Int64);
    ```

## One row per message

Ordinary frames produce one row each. Bulk configuration arrays emit every response, and wildcard responses emit every selected MBean, each repeating its source row's carried columns. Empty bulk and wildcard answers emit zero rows. Join a parsed capture by its carried source identifier rather than assuming row positions still align. `parse_text_line` is the same reading of one line, and answers the iterator when expansion is wanted.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar};

    let body = br#"[{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]"#;
    let field = DataType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::Binary.required_field("body"),
    ])?.required_field("capture");
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(7_i64), Scalar::from(body.to_vec()),
    ])]);
    let batch = yggdryl::arrow::batch_from_value(&field, &rows)?;
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
    let registry = Arc::new(FixRegistry::new().with_ulbridge_fields()?);
    let reader = FixCodec::new(registry).parse_text_arrow_reader(source)?;
    let mut count = 0;
    for batch in reader {
        let values = yggdryl::arrow::batch_to_value(&batch?)?;
        for row in values.as_sequence().expect("rows") {
            assert_eq!(row.get(0), Some(&Scalar::from(7_i64)));
            count += 1;
        }
    }
    assert_eq!(count, 2);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.fix import FixCodec, FixRegistry

    body = b'[{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]'
    source = pa.table({"rownum": pa.array([7], pa.int64()), "body": pa.array([body], pa.binary())})
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    result = FixCodec(registry).parse_text_arrow_reader(source.to_reader()).read_all()
    assert result.num_rows == 2
    assert result.column("rownum").to_pylist() == [7, 7]
    assert result.column("body").to_pylist() == [body, body]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, fix } = require('yggdryl')

    const body = Buffer.from('[{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]')
    const source = new arrow.Table({
      rownum: arrow.vectorFromArray([7n], new arrow.Int64()),
      body: arrow.vectorFromArray([body], new arrow.Binary()),
    })
    const registry = new fix.FixRegistry()
    registry.withUlbridgeFields()
    const result = new fix.FixCodec(registry).parseTextArrowReader(BatchReader.from(source)).intoTable()
    assert.equal(result.numRows, 2)
    assert.deepEqual([...result.getChild('rownum')], [7n, 7n])
    assert.ok([...result.getChild('body')].every((held) => Buffer.from(held).equals(body)))
    ```

## Rows are messages again, and messages rows

`messages` reads a stream of batches back as the messages that made them, each row through [`FixMsg::from_row`](message.md#a-row-is-a-message-again) under the schema read off the source - its entries rebuilt from `nofixentries`, so the message re-emits its line, digests, restates and stamps exactly as the parsed one did - at the cost of the values the row already holds, and no parse. `arrow_reader` is the other direction: a stream of messages into batches under a schema, each through `FixMsg::into_row`, closed by the raw bytes of each message's arrival record. The two invert each other, which is what lets a stage run over a capture already landed in Arrow.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;

    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|",
        b"8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // Into batches, and back: the same arrival record, the same wire, the
    // same digest, and the same row again.
    let reader = codec.arrow_reader(schema.clone(), parsed.clone().into_iter().map(Ok))?;
    let again: Vec<FixMsg> = codec.messages(reader).collect::<yggdryl::Result<_>>()?;
    assert_eq!(again.len(), parsed.len());
    for (held, message) in again.iter().zip(&parsed) {
        assert_eq!(held.entries(), message.entries());
        assert_eq!(held.into_bytes(b'|'), message.into_bytes(b'|'));
        assert_eq!(held.digest(), message.digest());
        assert_eq!(held.by_tag(35)?, message.by_tag(35)?);
        assert_eq!(held.into_row(&schema)?, message.into_row(&schema)?);
    }
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)
    schema = fix_schema(registry, "fix")

    lines = [
        b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|",
        b"8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|",
    ]
    parsed = list(codec.parse_lines(lines))

    # Into batches, and back: the same arrival record, the same wire, the
    # same digest, and the same row again.
    reader = codec.arrow_reader(schema, parsed)
    again = list(codec.messages(reader))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        assert held.entries() == message.entries()
        assert held.into_bytes(ord("|")) == message.into_bytes(ord("|"))
        assert held.digest() == message.digest()
        assert held.by_tag(35) == message.by_tag(35)
        assert held.into_row(schema) == message.into_row(schema)
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
      '8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|',
      '8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|',
    ].map((line) => Buffer.from(line))
    const parsed = [...codec.parseLines(lines)]

    // Into batches, and back: the same wire, the same digest, and the same
    // row again.
    const reader = codec.arrowReader(schema, parsed)
    const again = [...codec.messages(reader)]
    assert.equal(again.length, parsed.length)
    again.forEach((held, at) => {
      const message = parsed[at]
      assert.deepEqual(held.intoBytes(124), message.intoBytes(124))
      assert.deepEqual(held.digest(), message.digest())
      assert.ok(held.byTag(35).equals(message.byTag(35)))
      assert.ok(held.intoRow(schema).equals(message.intoRow(schema)))
    })
    ```

## Filled where it sits

`enrich_messages_arrow_reader` is [`enrich_messages`](capture.md#what-a-message-implied-is-filled-in) over batches: each row becomes its message, is filled with what it implies, and is written back under the same schema, so a carried column returns to its place and the arrival record is carried through untouched. Nothing is parsed again. The batches it answers close on the raw bytes of each message's arrival record, against the codec's `batch_byte_size`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::Array;
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(registry);

    // A report stating what was done and what was left: the rest is implied.
    let report = b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let capture = DataType::from_fields([DataType::Binary.required_field("body")])?.required_field("line");
    let source = || -> yggdryl::Result<_> {
        let batch = yggdryl::arrow::batch_from_value(
            &capture,
            &Scalar::from_sequence([Scalar::from_sequence([Scalar::from(report.to_vec())])]),
        )?;
        Ok(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
    };

    let bare = codec.parse_text_arrow_reader(source()?)?.next().expect("one batch")?;
    let filled = codec
        .enrich_messages_arrow_reader(codec.parse_text_arrow_reader(source()?)?)?
        .next()
        .expect("one batch")?;

    // The same schema, the carried column still leading it.
    assert_eq!(filled.schema(), bare.schema());
    let at = bare.schema().index_of("leavesqty")?;
    assert!(bare.column(at).is_null(0), "unsaid, and left unsaid");
    let row = |batch: &arrow_array::RecordBatch| -> yggdryl::Result<Vec<Scalar>> {
        let rows = yggdryl::arrow::batch_to_value(batch)?;
        Ok(rows.as_sequence().expect("rows")[0].as_sequence().expect("columns").to_vec())
    };
    let (before, after) = (row(&bare)?, row(&filled)?);
    assert_eq!(after[at], Scalar::from(60.0_f64), "OrderQty less CumQty");
    assert_eq!(after[bare.schema().index_of("avgpx")?], Scalar::from(10.5_f64), "one fill, so its price");
    // The arrival record is what arrived either way.
    let entries = bare.schema().index_of("nofixentries")?;
    assert_eq!(after[entries], before[entries]);
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pyarrow as pa

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)

    # A report stating what was done and what was left: the rest is implied.
    report = b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|"
    capture = pa.table({"body": pa.array([report], pa.binary())})

    bare = codec.parse_text_arrow_reader(capture.to_reader()).read_all()
    filled = codec.enrich_messages_arrow_reader(
        codec.parse_text_arrow_reader(capture.to_reader())
    ).read_all()

    # The same schema, the carried column still leading it.
    assert filled.schema == bare.schema
    assert bare.column("leavesqty").to_pylist() == [None], "unsaid, and left unsaid"
    assert filled.column("leavesqty").to_pylist() == [60.0], "OrderQty less CumQty"
    assert filled.column("avgpx").to_pylist() == [10.5], "one fill, so its price"
    # The arrival record is what arrived either way.
    assert filled.column("nofixentries").to_pylist() == bare.column("nofixentries").to_pylist()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)

    // A report stating what was done and what was left: the rest is implied.
    const report = Buffer.from('8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|')
    const capture = () => BatchReader.from(new arrow.Table({
      body: arrow.vectorFromArray([report], new arrow.Binary()),
    }))

    const bare = codec.parseTextArrowReader(capture()).intoTable()
    const filled = codec.enrichMessagesArrowReader(codec.parseTextArrowReader(capture())).intoTable()

    // The same schema, the carried column still leading it.
    assert.deepEqual(
      filled.schema.fields.map((field) => field.name),
      bare.schema.fields.map((field) => field.name),
    )
    assert.equal(bare.getChild('leavesqty').get(0), null, 'unsaid, and left unsaid')
    assert.equal(filled.getChild('leavesqty').get(0), 60, 'OrderQty less CumQty')
    assert.equal(filled.getChild('avgpx').get(0), 10.5, 'one fill, so its price')
    ```

## Back to the wire

`write_arrow_reader` streams a batch back out as lines, one per row, rebuilt from each row's `nofixentries` and never from its columns: the fixed columns are a *reading* of the message, so a frame rebuilt from them would be one nobody sent. Each line is [`into_bytes`](encode.md) with the codec's `separator`, `SOH` unless pinned, then a newline, and the count of lines is answered. The walk is pre-order, so a group's members follow the counter that heads them, exactly as they arrived. A batch carrying no `nofixentries` column cannot be written and says so before a row is read. One batch is pulled, its rows written, and it is dropped; no buffer bigger than a row is held.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');
    let schema = fix_schema(&registry, "fix")?;

    let lines = ["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|10=0|", "8=FIX.4.4|35=8|17=E1|37=O9|32=50|10=0|"];
    let reader = codec.arrow_reader(schema, codec.parse_lines(lines))?;

    let mut written = Vec::new();
    let count = codec.write_arrow_reader(reader, &mut written)?;
    assert_eq!(count, 2);
    assert_eq!(String::from_utf8(written)?, format!("{}\n{}\n", lines[0], lines[1]));
    ```

=== "Python"

    ```python
    import io
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry, separator=ord("|"))
    schema = fix_schema(registry, "fix")

    lines = [b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|10=0|", b"8=FIX.4.4|35=8|17=E1|37=O9|32=50|10=0|"]
    reader = codec.arrow_reader(schema, codec.parse_lines(lines))

    sink = io.BytesIO()
    assert codec.write_arrow_reader(reader, sink) == 2
    assert sink.getvalue() == b"".join(line + b"\n" for line in lines)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry, { separator: 124 })
    const schema = fix.schema(registry, 'fix')

    const lines = ['8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|10=0|', '8=FIX.4.4|35=8|17=E1|37=O9|32=50|10=0|']
    const reader = codec.arrowReader(schema, [...codec.parseLines(lines.map((line) => Buffer.from(line)))])

    // Anything with write(chunk) is a sink: a stream, a socket, an array.
    const chunks = []
    assert.equal(codec.writeArrowReader(reader, { write: (chunk) => chunks.push(Buffer.from(chunk)) }), 2)
    assert.equal(Buffer.concat(chunks).toString(), lines.map((line) => `${line}\n`).join(''))
    ```

## Edges

- Ordinary unframed text produces a row holding an empty message; bulk parsing errors propagate, and output counts follow message expansion.
- A carried column whose folded name a FIX column takes is dropped in front rather than renamed - two columns of one name is not a schema - and what it stated lands in that FIX column.
- A `direction` column is read as a parameter *and* carried, so a row shows both the value supplied and the direction read, in `msgdirection` (385); the two are different names, so both are present.
- A `timestamp` column is the row's clock: it stamps the message, and a row stating none leaves the message to its own clocks, else the epoch.
- A fill never overrides what the frame stated: a `seqNum` capture beside a frame carrying `34=` leaves `msgseqnum` to the frame.
- A fill is row-only: never an entry, never in `nofixentries`, never re-emitted by `write_arrow_reader`, never in `msghash`.
- A batch's bytes are read once from the payload column's offsets and spread evenly over its rows, so a large batch splits into equal row counts; a bulk configuration row's whole charge rides on its first message.
- A `batch_byte_size` of `0` or `1` is a batch a row: the target is where a batch closes, never a bound a row must fit under.
- `parse_text_arrow_reader` on a source with no column named as the payload column, or one holding neither text nor bytes under it -> refused before a row is read, naming the column. `parse_text_line` has no column to name: a line's body is a typed field, so a line carrying no bytes is a row holding an empty message and nothing else is refusable.
- `messages` on a source whose schema makes no root field -> one error item; a later batch of another schema -> a conflict item; a row that is not a FIX row -> an error item; each fuses the stream.
- `arrow_reader` under a schema the message cannot fill whole -> the row's own refusal, at that row; an `Err` item in its stream - a line `parse_lines` refused - yields the completed prefix, then the error, and fuses the reader, so a capture wanting every line as a row reads through `parse_text_arrow_reader`.
- `arrow_reader` over messages carrying no arrival record - built by hand, or read back from rows holding only lifted columns - charges each the leaves of its row, so `enrich_messages_arrow_reader` over a lifted-only projection is bounded by the same target.
- `write_arrow_reader` on a batch without `nofixentries` -> refused before a row is read; a row whose message held no pairs -> an empty line, still counted.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single row.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix batch::the_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix
    cargo test -p yggdryl --test fix dataset::
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

`fix/pipeline`, the whole path a desk takes over a bridge's own log: `rust/tests/fix/ulbridge.log`, a second of a ULBridge's capture beside every shape a bridge writes - a Jolokia exchange whose answer is a configuration document, FIXML behind a verb, frames spelled with `^A` and `<SOH>`, a `35=UL` frame packing a group inside a group, bridge rows of a hundred named keys, a statistics line, an empty body - repeated 64 times: 7,232 lines, 7,296 rows (7,232 messages; the empty body is a row and not a message), 11.0 MB. Every stage runs over the same corpus on its own, so a figure is per line of a real capture rather than of one shape. Release build, one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.94.1 release (thin LTO, one codegen unit); the codec pinned to the bridge's dialect.

| stage | estimate | throughput | per line, row or message |
| --- | --- | --- | --- |
| `text_read`, the row header framed, each line numbered, classified and read for its direction | 160 ms | 68.6 MB/s | 22.2 us |
| `parse_text_arrow_reader`, the whole path into fixed rows | 1.82 s | 6.1 MB/s | 249 us |
| `parse_lines`, the codec alone over the framed bodies | 859 ms | 12.8 MB/s | 119 us |

The text stage is under a tenth of the whole and the codec just under a half; the rest is the row landing in Arrow.

These three are slower than they were before a frame decided where a value ends and an entry became a range of its line, measured on one container against the commit those changes began from: `text_read` by 16%, `parse_text_arrow_reader` by 9%, `parse_lines` by 30%. `text_read` runs no FIX code at all, so its share is the scanner's: `LineSeparator::for_line` ranks four candidates by position where it used to stop at the first that answered, and ranking means asking each one where it first separated a field. That reading is the one [`inspect`, `classify` and `entry_spans` now share](../media/text.md), which is what stopped them being three answers to one question - and what it costs is stated here rather than left for a reader to discover. What a message costs after it is built, each pass over fresh clones of the 7,232 messages, so every number is the pass over a message the stream just built:

| pass | estimate | per message |
| --- | --- | --- |
| `into_row`, the message read against the fixed schema | 220 ms | 30.5 us |
| `arrow_reader`, the rows landed in batches | 458 ms | 63.3 us |
| `enrich_messages`, the rules that fill what a message implies | 516 ms | 71.4 us |
| `into_latest`, the row restated at the dictionary's newest version | 551 ms | 76.2 us |
| `lifecycle`, the stamp that joins a message to its order's life | 166 ms | 22.9 us |
| `digest`, the message's identity | 28.8 ms | 3.98 us |

A row pays `into_row` and its share of the batch; it pays for enrichment, the restatement and the lifecycle only when the caller composes that [stage](#a-pin-is-on-the-codec-a-stage-is-a-call). Reading a message against the fixed schema is a lookup per column, most of them misses answered by a name table the message builds on its first projection; the batch is the rows canonicalized and built into one `RecordBatch`, of which the arrival record is the one nested column. Enrichment is the rule table walked once, most of it lookups that answer nothing on a message that stated everything, and each answer a write into the row; the restatement is every child resolved against the dictionary once and the rules its fields carry read borrowed; the lifecycle is two digests, a chain lookup and one write of three stamps. The digest is a hash over the arrival record and nothing else.

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/pipeline
```

