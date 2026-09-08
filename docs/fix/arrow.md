# Arrow

A capture already in Arrow is read where it sits: `FixBatchReader::from_column` takes one payload column through the same [reader](capture.md#a-reader-is-the-whole-parse-surface) a captured line goes through, and returns the crate's one batch reader - so a day of session log reaches Parquet or Iceberg with nothing here knowing what either is.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixBatchReader::from_rows` and `::from_column`, `FixOptions`, `FixMsg::from_record`, `classify_arrow_array`, `write_fix`, `DEFAULT_PAYLOAD_COLUMN`, `SOH` |
| Returns | `BatchReader`, the one type every encoding in the crate returns; Python gets a `pyarrow.RecordBatchReader` |
| Schema | answered before the first row is read, from the options and the [dictionary](registry.md) alone, never from the data |
| Order | the source's own columns lead the row, the [fixed columns](capture.md#the-columns-are-the-tags) follow |
| Clash | a carried column whose name a FIX column takes is dropped, never renamed and never duplicated |
| Rows | a row in is a row out, so a batch joins back to its source by position; `dedup` is the one exception |
| Refuses | nothing a row's content can do; the `Result` is for I/O and for options that do not make a root field |
| Per row | `branch`, `beginstring`, `sep` and `direction` are parameters read from the row, and are still carried into it |
| Lazy | one batch is pulled, its rows built, and it is dropped; the source is never concatenated |
| Classify | `classify_arrow_array` builds no message and resolves nothing against a dictionary |
| Bindings | Rust and Python (`parse_arrow_reader`, `classify_arrow_array`); no JavaScript binding |

## Use

One column of frames in, batches out, the capture's own columns still in front of them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixBatchReader, FixOptions, FixRegistry, Scalar};

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
    let schema = batch.schema();

    let read = FixBatchReader::from_column(
        registry,
        yggdryl::arrow::batch_reader(schema, [batch]),
        "body",
        FixOptions::new(),
    )?;

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
    assert_eq!(rows, 1, "a row in is a row out");
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pyarrow as pa

    from yggdryl.fix import FixRegistry, parse_arrow_reader

    registry = FixRegistry.from_handle(Path("config/fix").resolve())

    # A capture shaped the way a log reader shapes one: where the line was
    # read from, which line it was, and the frame itself.
    capture = pa.table(
        {
            "url": ["file:///capture.log"] * 2,
            "rownum": pa.array([7, 8], pa.int64()),
            "body": pa.array(
                [
                    b"recv 8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|",
                    b"8=FIX.4.4|35=D|11=ORDER-2|55=MSFT|10=0|",
                ],
                pa.binary(),
            ),
        }
    )

    read = parse_arrow_reader(capture, registry, "body", version="FIX.4.4")

    # The schema is answered before a row is read: the capture leads it, the
    # tags follow, and the two lists close it.
    columns = [field.name for field in read.schema]
    assert columns[:3] == ["url", "rownum", "body"]
    assert columns[-2:] == ["nofixentries", "nounmappedfixentries"]

    held = read.read_all()
    assert held.num_rows == 2, "a row in is a row out"
    assert held.column("55").to_pylist() == ["AAPL", "MSFT"]
    assert held.column("url")[0].as_py() == "file:///capture.log"
    # The verb in front of the frame beats the option that named a default.
    assert held.column("385").to_pylist() == [b"RECV", b"SENT"]
    ```

## The source's columns lead the row

Where a line was read from is what a monitor orders and joins on, so the source's own columns lead the row and the fixed columns follow, exactly as they do for a [capture read line by line](capture.md#a-captures-own-columns-lead-the-row). Because the row counts match, carrying them is a slice rather than a join; which of them survive a FIX column's claim on a name is decided once, from the schema, rather than per row.

## The options are the reader's arguments, per stream

`FixOptions` says how a line becomes a message, and each parse option is the per-stream form of an argument the [byte readers](capture.md#a-reader-is-the-whole-parse-surface) take per call. The rest of the struct is the shared [record options](../media/options.md) every encoding takes: the batch bounds, the publication cadence, the declared root.

| Option | Builder | Default | Says |
| --- | --- | --- | --- |
| `payload_column` | `with_payload_column` | `body` (`DEFAULT_PAYLOAD_COLUMN`) | which column carries the bytes; `from_column`'s own argument sets it |
| `separator` | `with_separator` | `SOH` (`0x01`) | the separator a numeric frame is written with |
| `branch` | `with_branch` | none | the dialect, so no row infers one |
| `version` | `with_version` | none | the version values are translated at, never what a column is called; unpinned, each row answers for itself |
| `null_values` | `with_null_values` | the crate's spellings | what means "nothing was sent" |
| `direction` | `with_direction` | `SENT` | the direction a line that states none of its own took — no verb in front of its payload, and no [document saying which half it is](registry.md#a-direction-is-the-verb-in-front-of-the-payload) |
| `dedup` | `with_dedup` | `false` | whether an adjacent republication is dropped |

`name` names the root, and is `fix` unless it is set.

Python spells them as keywords on `parse_arrow_reader`, under the same names, with the payload column as the third positional argument and `separator` as the byte's integer value. A `registry` of `None` links the [process-wide default](registry.md#one-default-registry-per-process), and `direction` takes `"sent"`, `"recv"` or `"unknown"`.

## A column is the caller speaking per row

One column carries the bytes; five more supply, per row, arguments the byte readers already take per call.

| Column | Supplies |
| --- | --- |
| the payload column, named by the options | the bytes parsed |
| `branch` | the dialect |
| `beginstring` | the source version |
| `sep` | the separator |
| `direction` | the direction, stated |

A column is the caller speaking per row and an option is the caller speaking per stream, so a column outranks the option and both outrank what the frame infers: a row whose `beginstring` says `FIX.4.2` is read at 4.2 whatever the stream was pinned to, and its values translate through the code spellings 4.2 declares. A column absent, null or empty is silence, never an instruction and never an error.

Each one is still carried into the row, because a monitor needs to see the value it supplied rather than infer that it was used. A record carrying only a payload column behaves exactly as the byte reader behaves, which is what makes this an entry point rather than a second contract.

## A row in is a row out

The output row count equals the input row count. That is what lets a parsed capture be joined back to its source by position, and it is why nothing a row's *content* can do is an error: a frame the reader refuses is a message with nothing in it, a row with no pairs is a row with no entries, and the `Result` is for I/O.

`dedup` is the one exception, and it is off by default for exactly that reason: with it on, a row whose digest equals the one before it is dropped, and the batch no longer aligns with its source. The comparison is with the row before only, so a republication one row later is kept.

## What a column says about itself

`classify_arrow_array` is the column form of the three readings the byte readers already make per line: what the record is, the raw `MsgType` its frame spells, and which way it moved. Three `Utf8` arrays the length of the input, from one shallow scan - no message is built and nothing is resolved against a dictionary, which is what makes it cheap enough to run over every line of a capture before deciding what to parse. The readings themselves, and what each answer means, are [one page over](registry.md#classifying-a-captured-line).

The classifying stage and the parsing one therefore cannot disagree: they are the same scan, asked at a different width.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::cast::AsArray;
    use arrow_array::{Array, ArrayRef, BinaryArray};
    use yggdryl::classify_arrow_array;
    use yggdryl::types::MsgDirection;

    let lines: Vec<&[u8]> = vec![
        b"recv 8=FIX.4.4\x0135=8\x01150=F\x0110=0\x01",
        b"toBridge #MSGTYPE=D|#SYMBOL=TTF|",
        b"20260821 10:30:00 nothing framed here",
        br#"{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":200}"#,
    ];
    let column: ArrayRef = Arc::new(BinaryArray::from(lines));

    let (mimetype, msgtype, direction) =
        classify_arrow_array(&column, Some(MsgDirection::SENT))?;
    let mimetype = mimetype.as_string::<i32>();
    let msgtype = msgtype.as_string::<i32>();
    let direction = direction.as_string::<i32>();

    assert_eq!(mimetype.value(0), "text/fix");
    assert_eq!(mimetype.value(1), "text/ullink");
    assert_eq!(mimetype.value(2), "application/octet-stream");
    assert_eq!(mimetype.value(3), "text/ulconfig");
    // A bridge writes its own type in front of the frame it relays.
    assert_eq!(msgtype.value(0), "8");
    assert_eq!(msgtype.value(1), "D");
    assert!(msgtype.is_null(2));
    // A configuration document declares what the MBean it names is.
    assert_eq!(msgtype.value(3), "Bridge");
    // The default names what a line stating nothing took; a marked line and a
    // document that says which half it is both keep their own.
    assert_eq!(direction.value(0), MsgDirection::RECV);
    assert_eq!(direction.value(1), MsgDirection::SENT);
    assert_eq!(direction.value(3), MsgDirection::RECV);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl.fix import classify_arrow_array

    lines = pa.array(
        [
            b"recv 8=FIX.4.4\x0135=8\x01150=F\x0110=0\x01",
            b"toBridge #MSGTYPE=D|#SYMBOL=TTF|",
            b"20260821 10:30:00 nothing framed here",
            b'{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"status":200}',
        ],
        pa.binary(),
    )

    mimetype, msgtype, direction = classify_arrow_array(lines, "sent")

    assert mimetype.to_pylist() == [
        "text/fix",
        "text/ullink",
        "application/octet-stream",
        "text/ulconfig",
    ]
    # A bridge writes its own type in front of the frame it relays, and a
    # configuration document declares what the MBean it names is.
    assert msgtype.to_pylist() == ["8", "D", None, "Bridge"]
    # The default names what a line stating nothing took; a marked line and a
    # document that says which half it is both keep their own.
    assert direction.to_pylist() == ["RECV", "SENT", "SENT", "RECV"]
    ```

## Back to the wire

`write_fix` streams a batch back out as frames, rebuilt from each row's `nofixentries` and never from its columns: the fixed columns are a *reading* of the message, so a frame rebuilt from them would be one nobody sent. The walk is pre-order, so a group's members follow the counter that heads them, exactly as they arrived. A batch carrying no `nofixentries` column cannot be written and says so plainly. One batch is pulled, its rows written, and it is dropped; no buffer bigger than a row is held. Rust only.

## Edges

- A frame the reader cannot read is a row with nothing in it, so the count still matches the capture's.
- A carried column whose name a FIX column takes is dropped rather than renamed: two columns of one name is not a schema.
- A `direction` column is read as a parameter *and* carried, so a row shows both the value supplied and the direction read, in tag `385`.
- `classify_arrow_array` takes `Binary`, `LargeBinary`, `Utf8` and `LargeUtf8`; another column type is refused naming what it got.
- A null payload row classifies as `application/octet-stream`, with no message type and the default direction.
- Python takes one `pyarrow.Array`; a `ChunkedArray` is refused, and `combine_chunks()` hands over the one array it holds.
- Python's `direction="unknown"` is the third answer: null, rather than a side the capture never stated.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single row.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix batch::the_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python scripts/check_docs_examples.py --lang python
    ```
