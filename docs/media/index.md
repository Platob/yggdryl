# Media

A handle's name picks the encoding, the compression and the charset; the read and write calls never change.

## Overview

Each medium has a page of its own - what declares it, how it reads, how it writes, then what only it has - and every record encoding answers the calls on this page through [`IOMedia`](../holder/index.md#records), with no format argument anywhere.

| Medium | Declared by | Build |
| --- | --- | --- |
| [Arrow IPC](ipc.md) | `application/vnd.apache.arrow.stream`, `.arrows` | default |
| [Parquet](parquet.md) | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature |
| [Avro](avro.md) | `application/avro`, `.avro` | default |
| [Plain text](text.md) | `text/plain`, `.txt`, `.log` | default |
| [CSV](csv.md) | `text/csv`, `text/tab-separated-values`, `.csv`, `.tsv` | default |
| [JSON](json.md) | `application/json`, `application/x-ndjson`, `.json`, `.jsonl` | default |
| [YAML](yaml.md) | `application/yaml`, `.yaml` | default |
| [TOML](toml.md) | `application/toml`, `.toml` | default |
| [XML](xml.md) | `application/xml`, `.xml` | default |
| [XML for Analysis](xmla.md) | `application/xmla+xml`, `.xmla`; the provider serves catalogs over HTTP | default; the provider's route `http` feature |
| [Excel](excel.md) | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | default |
| [Iceberg](iceberg.md) | a table folder | `iceberg` feature |
| [HTTP messages](http.md) | `message/http`, `.http` | `http` feature |
| [Compression](compression.md) | `.gz`, `.zz`, `.zst` suffix | default |
| [Charsets](charsets.md) | `;charset=` parameter | default |

## Read

A read returns an [`arrow::BatchReader`](../arrow/readers.md); only the current batch is alive. Rows come back as native values through Python `read_records` and JavaScript `readRecords`; Rust has no `read_records` and reads rows through `read_serie`, a `SerieReader` of one record `Serie` per batch - the [write example](#write) reads its rows back each way. `read_serie` with no options reads under the handle's own, in every language: `read_serie()` in Python, `readSerie()` in JavaScript.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS"), None])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // Batches arrive one at a time; only the current one is alive.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&options)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(
        pa.record_batch({"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=schema)
    )

    # A pyarrow.RecordBatchReader over the Arrow C Stream; batches arrive one at a time.
    rows = sum(part.num_rows for part in handle.read_arrow_reader())
    assert rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
          venue: arrow.vectorFromArray(['XNAS', 'XNYS', null], new arrow.Utf8()),
        }),
      ),
    )

    // Batches arrive one at a time; only the current one is alive.
    let rows = 0
    for (const batch of handle.readArrowReader()) {
      rows += batch.numRows
    }
    assert.equal(rows, 3)
    ```

A folder, a location ending in `/` and a glob read as the one table their leaves hold, through the same calls. A folder finds the encoding beneath it; a pattern's suffix names it, so `lake/**/*.parquet` reads as Parquet and only the `.parquet` leaves it matches are read - a marker such as `_SUCCESS` or a note beside the data is not - and `logs/*.log.gz` reads as plain text, each leaf taking off its own coding. Private names - `.venv`, `.config` and anything else starting with a dot - are left out of every walk by default, with the tree beneath them.

## Write

Every write states its intent: `overwrite_*` replaces the stored rows, `append_*` keeps them and adds its own after them, and `merge_*` updates the rows whose key matches - the options' `merge_by`, else the destination's own ([an Iceberg table's](iceberg.md#the-merge-key)) - and appends the rest. `overwrite_records`, `append_records` and `merge_records` write native rows; the `*_arrow_reader` and `*_arrow_batch` twins - and `*_arrow_table` in the bindings - write Arrow batches, streamed and never collected; and `overwrite_serie`, `append_serie` and `merge_serie` - `write_serie` with the mode named - write a held `Serie`, a `ChunkedSerie` or a `SerieReader` as the batches it already is ([Writing a serie to a handle](../types/serie.md#writing-a-serie-to-a-handle)), absent options being the handle's own. Every one of them answers an `IOResult` - the rows it read, wrote and skipped ([Write results](../holder/index.md#write-results)).

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

    struct Quote(i64, &'static str);

    impl From<Quote> for Scalar {
        fn from(row: Quote) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
        }
    }

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
    ])?)
    .required_field("row");
    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?.with_field(field.clone());

    handle.overwrite_records([Quote(1, "XNAS"), Quote(2, "XNYS")], &options)?;
    handle.append_records([Quote(3, "XLON")], &options)?;
    handle.merge_records([Quote(2, "XPAR")], &options.clone().with_merge_by(["id"])?)?;

    // Rust reads Arrow as a stream of record columns, one per batch, and a
    // record column lends each child column by name.
    let mut venues = Vec::new();
    for records in handle.read_serie(Some(&options.clone().with_field(field.clone())))? {
        let venue = records?.child("venue").cloned().expect("a venue column");
        for row in 0..venue.len() {
            venues.push(venue.scalar(row)?);
        }
    }

    assert_eq!(venues.len(), 3);
    assert!(venues.contains(&Scalar::from("XPAR")) && !venues.contains(&Scalar::from("XNYS")));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, scalar

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # Mappings are rows, and a non-empty source infers the field from them.
    handle.overwrite_records([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": "XNYS"}])
    handle.append_records([{"id": 3, "venue": "XLON"}])

    merging = handle.record_options()
    merging.merge_by = ["id"]
    handle.merge_records([{"id": 2, "venue": "XPAR"}], options=merging)

    # Plain dictionaries back, one row at a time.
    rows = list(handle.read_records())
    assert len(rows) == 3
    assert {row["venue"] for row in rows} == {"XNAS", "XPAR", "XLON"}

    # Instances of a record class are rows of the field their class declares,
    # and each crosses the core's value contract on its way to the column.
    @scalar(frozen=True)
    class Trade:
        id: int
        venue: str

    classes = IOBase(pathlib.Path(tempfile.mkdtemp()) / "classes.arrows")
    classes.overwrite_records([Trade(1, "XNAS"), Trade(2, "XNYS")])
    assert list(classes.read_records(Trade)) == [Trade(1, "XNAS"), Trade(2, "XNYS")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))

    // Plain objects are rows; the writers widen them into the stream.
    handle.overwriteRecords([{ id: 1n, venue: 'XNAS' }, { id: 2n, venue: 'XNYS' }])
    handle.appendRecords([{ id: 3n, venue: 'XLON' }])
    handle.mergeRecords(
      [{ id: 2n, venue: 'XPAR' }],
      handle.recordOptions().withMergeBy(['id']),
    )

    const rows = [...handle.readRecords()]
    assert.equal(rows.length, 3)
    assert.deepEqual(
      new Set(rows.map((row) => row.venue)),
      new Set(['XNAS', 'XPAR', 'XLON']),
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Options

One `RecordOptions` drives every encoding: the root `field`, `select`, `filter`, `batch_row_size`, `merge_by`, `safe`, `level`, the row bounds `row_offset`, `max_row_size` and `max_byte_size` ([Limits](../holder/index.md#limits)), the publication cadence `commit_batch_num` ([Commit cadence](../holder/index.md#commit-cadence)) and the write's `num_threads`, plus the settings one encoding owns. A JSON, YAML, TOML or XML document is one value rather than a stream of batches, so it has no `RecordOptions` of its own: it reads through `read_serie` or `read_scalar` and writes, whole, through `overwrite_serie` or `write_scalar`, taking of a record encoding's options only the declared `field`. The declared `field` and the field a write completes onto are both declarations and cast by [one rule](../types/cast.md#required-columns): a nullable column takes a value it cannot convert as null while `safe` (the default) holds, and a not-null column refuses that value, a null and a missing column by name rather than storing its canonical default. A `TRANSFORM:`, `PARTITION:` or `DIGEST:` declaration on either field is metadata the cast carries, never a column a read or write fills: a derived or holder column the rows do not carry lands null where nullable and is refused by path where required, and the caller fills it first through the field's `transform` or `digest` view ([Applying a schema](../types/field.md#applying-a-schema)).

| Write setting | Unset | Set |
| --- | --- | --- |
| `commit_batch_num` | the destination's own cadence: a leaf, a plain folder and an Iceberg table publish once, when the source ends - the table holding every partition's rows under the process [spill bound](../types/serie.md#spilling-to-disk) until then, so an overwrite of any length is one atomic snapshot | a publication every `N` whole batches, then the remainder; `0` is refused before the source is pulled ([Commit cadence](../holder/index.md#commit-cadence)) |
| `merge_by` | on a merge, the destination's own key: an Iceberg table's identity partition columns, then its identifier columns; a leaf, a folder or a table stating none refuses a merge naming `$.merge_by` before the source is pulled | the match key, a stored column or a computed term per projection; overwrite and append refuse it |
| `num_threads` | the destination's own answer: an Iceberg table's `write.parallelism`, else its `read.parallelism`, else every thread the host offers | the most parts a write of several parts runs at once - an Iceberg commit's [partition groups](iceberg.md#write) - while a leaf of one file is written on the thread that writes it and reads nothing from it; `0` is refused naming `$.num_threads` before the source is pulled |

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_row_size(1024)
        .with_num_threads(4);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), "row");
    assert!(options.select().is_all());
    assert!(options.filter().is_always_true());
    assert_eq!(options.batch_row_size(), Some(1024));
    assert_eq!(options.num_threads(), Some(4));
    assert_eq!(options.stable_hash(), options.clone().stable_hash());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.parquet")
    options.field = schema
    options.batch_row_size = 1024
    options.commit_batch_num = 10
    options.num_threads = 4

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.name == "row"
    assert [child.name for child in options.field.dtype] == ["id"]
    assert options.select.is_all
    assert options.filter.is_always_true
    assert options.batch_row_size == 1024
    assert options.commit_batch_num == 10
    assert options.num_threads == 4

    # A setting one encoding has reads as None on an encoding that has none.
    assert options.max_row_group_size == 1_048_576
    stream = RecordOptions("trades.arrows")
    assert str(stream.mime_type) == "application/vnd.apache.arrow.stream"
    assert stream.max_row_group_size is None

    # `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert stream.level == 9
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const options = RecordOptions.from('trades.parquet')
      .withField(schema)
      .withBatchRowSize(1024)
      .withNumThreads(4)

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.equal(options.name, 'row')
    assert.ok(options.field.equals(schema))
    assert.ok(options.select.isAll)
    assert.ok(options.filter.isAlwaysTrue)
    assert.equal(options.batchRowSize, 1024)
    assert.equal(options.numThreads, 4)

    // A setting one encoding has reads as null on an encoding that has none.
    assert.equal(options.maxRowGroupSize, 1_048_576)
    const stream = RecordOptions.from('trades.arrows')
    assert.equal(stream.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    assert.equal(stream.maxRowGroupSize, null)

    // `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert.equal(stream.level, 9)

    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(false).safe, false)
    assert.equal(options.safe, true)
    ```

### Settings by name

Every record read and write takes `options` and, beside it, the option properties by name - keywords in Python, a plain object in JavaScript (alone in the options position, or after an options value) - each set on a copy of the options by that property's own setter, so a value is checked exactly as an assignment is and the options passed in never change. A value not given (`...` in Python, `undefined` in JavaScript) is skipped; `None`/`null` is a value and clears. A name no setter of that options class owns is not an error: it is skipped with an `UnknownPropertyWarning` naming it and the closest property there is (within a third of its length), so a typo is heard without failing the call - a `warnings` category in Python that a filter can escalate, a process warning (`code: 'YGGDRYL_UNKNOWN_PROPERTY'`) in JavaScript, heard once per process per message. A read-only name (`mime_type`) and another encoding's setting (`rowheader` on CSV) name no settable property; a known name the encoding cannot honour is the setter's own refusal. The same holds for `TextOptions`, `TextLine`, the Iceberg calls and `IcebergOptions`, and an HTTP session's `HttpOptions` properties. Rust sets each property through its typed setter, and `HttpOptions::is_property` and `S3Options::is_property` answer, from the readers themselves, which names their property doors take, the readers `Holder::from_url` hands a location's properties to. A property spelled as text - a [plan target's](../expression/plans.md) `with (...)`, a URL's query, a catalog's property bag - is read by the one reader of its type, never a parse of its own: a flag by the [boolean table](../types/numeric/boolean.md#the-one-text-reader) (`true`, `yes`, `on`, `1` and their opposites, in any case), a count by the integer grammar, a duration and a byte size as an [HTTP session's](../holder/index.md#durations-counts-sizes-and-flags) are; a text no reader reads is refused naming the property - `$.with.safe` on a plan target. Inference is narrower than any of them: a [CSV](csv.md) column is typed `boolean` only by `true` and `false`.

=== "Rust"

    ```rust
    use yggdryl::http::HttpOptions;
    use yggdryl::media::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::CSV)?;
    options.set_csv_separator(b';')?;
    assert_eq!(options.csv_separator(), Some(b';'));

    // The by-name door a URL's query and a catalog's properties reach.
    assert!(HttpOptions::is_property("timeout"));
    assert!(!HttpOptions::is_property("timout"));
    ```

=== "Python"

    ```python
    import tempfile
    import warnings
    from pathlib import Path

    from yggdryl import IOBase, RecordOptions, UnknownPropertyWarning

    handle = IOBase(Path(tempfile.mkdtemp()) / "trades.csv")
    handle.write_bytes(b"symbol;price\nAAPL;187\n")

    options = RecordOptions("text/csv")
    table = handle.read_arrow_reader(options=options, separator=";").read_all()
    assert table.column_names == ["symbol", "price"]
    assert options.separator == ","  # set on a copy

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        handle.read_arrow_field(seperator=";")
    assert issubclass(caught[0].category, UnknownPropertyWarning)
    assert "did you mean 'separator'?" in str(caught[0].message)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, RecordOptions } = require('yggdryl')

    const handle = new IOBase(path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'csv-')), 'trades.csv'))
    handle.writeText('symbol;price\nAAPL;187\n')

    const options = new RecordOptions('text/csv')
    const table = handle.readArrowReader(options, { separator: ';' }).intoTable()
    assert.equal(table.numCols, 2)
    assert.equal(options.separator, ',') // set on a copy

    const heard = []
    const listener = (warning) => heard.push(warning.message)
    process.on('warning', listener)
    handle.readArrowField({ seperator: ';' })
    setImmediate(() => {
      process.off('warning', listener)
      assert.match(heard[0], /did you mean 'separator'\?/)
    })
    ```
