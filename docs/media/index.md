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
| [Iceberg](iceberg.md) | a table folder, claimed as the `iceberg` table format | `iceberg` feature |
| [HTTP messages](http.md) | `message/http`, `.http` | `http` feature |
| [Compression](compression.md) | `.gz`, `.zz`, `.zst` suffix | default |
| [Charsets](charsets.md) | `;charset=` parameter | default |

A record medium is claimed on the register of media under the MIME types it declares: the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis, CSV and Excel - before the register answers anything, and a crate that brings another claims it from its `install()` ([Registering a medium](#registering-a-medium)). Iceberg is a table format claimed on its own register ([a table format](iceberg.md#registering-a-table-format)).

## Read

A read's primitive is `read_serie` / `readSerie`, returning the generic
`Serie`: a held value, a native row or chunk stream, or a key kind. The
media series of the Rust core - `IpcSerie`, `CSVSerie` and `TextSerie` for
its three media, `GenericMediaSerie` for every other medium - XML for Analysis,
Parquet, Avro, a workbook - are built over a medium by `new(media)` and hold no
options of their own: the medium holds the options, stating or inferring them
with their defaults, and answers the whole field its origin holds
(`read_origin_field`, served from its [metadata cache](#the-metadata-cache));
a serie keeps only the root the medium answered when it bound - its declared
field, else its origin's - and reads under the medium's options plus the
clauses its own verbs state (`with_filter`, `with_key`, `with_select`,
`with_row_range`), which prune source keys before payloads are decoded. The
clauses compose once, whatever the door: the early half of the `where` and the
columns the `select` reads reach the medium - a declared field is handed to it
narrowed to those columns, so a full declaration under a `select` decodes the
selected columns alone, and the row bounds go down where the medium takes them
natively - and what the medium did not do runs once over what it answered: the
`where` (a medium prunes by its early half and filters no row), the `select`
and the exact bounds. A conjunct the unit's location proves for every
row, a Hive path's `venue=XNAS` under `venue = 'XNAS'`, is dropped from both
halves. `read_arrow_reader`, the bindings' `where=` and `select=` properties
and `Plan::execute` are the same composition at their own doors. A serie
refuses the options of a medium it does not read, naming both.
`MediaSerieValue<T: IOMedia>` implements their shared accessors and snapshot
mutations. Explicit writes publish changes under the medium's options.
`read_arrow_reader` is an adapter over that same primitive. Native record mapping
adapters are Python `read_records` and JavaScript `readRecords`.

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

Every write states its intent: `overwrite_*` replaces the stored rows, `append_*` keeps them and adds its own after them - into an Iceberg table stating its own key, only the rows whose key it lacks ([Appending to a keyed table](iceberg.md#appending-to-a-keyed-table)) - and `merge_*` updates the rows whose key matches - the options' `merge_by`, else the destination's own ([an Iceberg table's](iceberg.md#the-merge-key)) - and appends the rest, rewriting nothing where no row changes ([Append and merge](../holder/index.md#append-and-merge)). `overwrite_records`, `append_records` and `merge_records` write native rows; the `*_arrow_reader` and `*_arrow_batch` twins - and `*_arrow_table` in the bindings - write Arrow batches, streamed and never collected; and `overwrite_serie`, `append_serie` and `merge_serie` - `write_serie` with the mode named - write a held `Serie`, a `ChunkedSerie` or a `StreamChunkedSerie` as the batches it already is ([Writing a serie to a handle](../types/serie.md#writing-a-serie-to-a-handle)), absent options being the handle's own. Every one of them answers an `IOResult` - the rows it read, wrote and skipped ([Write results](../holder/index.md#write-results)).

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
    for records in handle.read_serie(Some(&options.clone().with_field(field.clone())))?.into_chunked_stream(None, None)?.into_chunks() {
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

One `RecordOptions` drives every encoding: the root `field`, `select`, `filter`, `batch_row_size`, `merge_by`, `safe`, `level`, the row bounds `row_offset`, `max_row_size` and `max_byte_size` ([Limits](../holder/index.md#limits)), the publication cadence `commit_batch_num` ([Commit cadence](../holder/index.md#commit-cadence)), the write's `num_threads` and the metadata cache's `cache_ttl` ([below](#the-metadata-cache)), plus the settings one encoding owns, which that encoding's own options struct holds ([A medium's own settings](#a-mediums-own-settings)). A JSON, YAML, TOML or XML document is one value rather than a stream of batches, so it has no `RecordOptions` of its own: it reads through `read_serie` or `read_scalar` and writes, whole, through `overwrite_serie` or `write_scalar`, taking of a record encoding's options only the declared `field`. A read's schema and the columns it decodes follow one rule whichever medium holds the data: `read_arrow_field` answers the declared root, else the origin's, narrowed by the `select` - a `SORT:by` the root declares is kept only where the selection publishes every column its keys read unchanged - and what the medium decodes is the declared children, else the origin's, intersected with the columns the `select` and the early half of the `filter` read (a `*` reads every column, a late alias none), so a declared column outside the selection is never asked for. The declared `field` and the field a write completes onto are both declarations and cast by [one rule](../types/cast.md#required-columns): a nullable column takes a value it cannot convert as null while `safe` (the default) holds, and a not-null column refuses that value, a null and a missing column by name rather than storing its canonical default. A `TRANSFORM:`, `PARTITION:` or `DIGEST:` declaration on either field is metadata the cast carries, never a column a read or write fills: a derived or holder column the rows do not carry lands null where nullable and is refused by path where required, and the caller fills it first through the field's `transform` or `digest` view ([Applying a schema](../types/field.md#applying-a-schema)). The options' `apply_arrow_batch` and `apply_arrow_reader` are this shaping at its `RecordBatch` and `BatchReader` face - the declared field's cast, the `where` and `select` sections, then the cast onto `existing`, the stored field - each cast the [`ArrowCastPlan`](../types/cast.md#eager-and-lazy) `Serie` runs, compiled once per call, so a stream is shaped by `apply_arrow_reader`; `limit_arrow_reader` applies the row bounds last. Rust and Python bind the three; JavaScript binds none.

| Setting | Unset | Set |
| --- | --- | --- |
| `commit_batch_num` | the destination's own cadence: a leaf, a plain folder and an Iceberg table publish once, when the source ends - the table holding every partition's rows under the process [spill bound](../types/serie.md#spilling-to-disk) until then, so an overwrite of any length is one atomic snapshot | a publication every `N` whole batches, then the remainder; `0` is refused before the source is pulled ([Commit cadence](../holder/index.md#commit-cadence)) |
| `merge_by` | on a merge, the destination's own key: an Iceberg table's identity partition columns, then its identifier columns; a leaf, a folder or a table stating none refuses a merge naming `$.merge_by` before the source is pulled. `True` in Python - a boolean `true` through `IORecordOptions::set_merge_by_scalar` in Rust - spells this state outright: it stores the empty key `None` stores, so options set with it equal fresh ones and an overwrite or append under them is not refused; `False` is refused naming `$.merge_by`, the key the options held kept. JavaScript takes no boolean: `mergeBy` left out or `null` | the match key, a stored column or a computed term per projection; overwrite and append refuse it |
| `num_threads` | the destination's own answer: an Iceberg table's `write.parallelism`, else its `read.parallelism`, else every thread the host offers | the most parts a write of several parts runs at once - an Iceberg commit's [partition groups](iceberg.md#write) - while a leaf of one file is written on the thread that writes it and reads nothing from it; `0` is refused naming `$.num_threads` before the source is pulled |
| `cache_ttl` | `0`, realtime: a closed handle reads its metadata afresh on every call | milliseconds a closed handle serves a cached entry younger than it ([The metadata cache](#the-metadata-cache)); a whole number, a negative or fractional one refused naming `$.cache_ttl`; outside the options' identity. Python's `buffered(ttl=)` is seconds |

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, Scalar, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_row_size(1024)
        .with_num_threads(4)
        .with_cache_ttl(1000_u64);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), "row");
    assert!(options.select().is_all());
    assert!(options.filter().is_always_true());
    assert_eq!(options.batch_row_size(), Some(1024));
    assert_eq!(options.num_threads(), Some(4));
    assert_eq!(options.stable_hash(), options.clone().stable_hash());

    // `cache_ttl` is milliseconds, and outside the options' identity: another
    // time-to-live is the same options.
    assert_eq!(options.cache_ttl().millis(), 1000);
    assert_eq!(options.clone().with_cache_ttl(0_u64), options);
    assert_eq!(options.clone().with_cache_ttl(0_u64).stable_hash(), options.stable_hash());

    // `true` is the destination's own key: the empty key the options start with.
    let keyed = options.clone().with_merge_by("id")?;
    assert_eq!(keyed.with_merge_by_scalar(&Scalar::from(true))?, options);
    assert!(options.clone().with_merge_by_scalar(&Scalar::from(false)).is_err());
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
    options.cache_ttl = 1000

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.name == "row"
    assert [child.name for child in options.field.dtype] == ["id"]
    assert options.select.is_all
    assert options.filter.is_always_true
    assert options.batch_row_size == 1024
    assert options.commit_batch_num == 10
    assert options.num_threads == 4

    # `cache_ttl` is milliseconds, and outside the options' identity.
    assert options.cache_ttl == 1000
    assert RecordOptions("trades.parquet", cache_ttl=0) == RecordOptions("trades.parquet", cache_ttl=1000)

    # A setting one encoding has reads as None on an encoding that has none.
    assert options.max_row_group_size == 1_048_576
    stream = RecordOptions("trades.arrows")
    assert str(stream.mime_type) == "application/vnd.apache.arrow.stream"
    assert stream.max_row_group_size is None

    # `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert stream.level == 9

    # `merge_by=True` is the destination's own key: the empty key options start with.
    assert RecordOptions("trades.parquet", merge_by=True) == RecordOptions("trades.parquet")
    try:
        RecordOptions("trades.parquet", merge_by=False)
    except ValueError as refusal:
        assert "$.merge_by" in str(refusal)
    else:
        raise AssertionError("False names no key")
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

### The metadata cache

Every media wrapper - Arrow IPC, Parquet, Avro, CSV, plain text, XML for Analysis and Excel - keeps what it learned of its origin in one cache entry: the whole root the origin states (`read_origin_field`, its metadata whole), the row and column counts, and the medium's own object - a Parquet footer, an Avro container's dimensions, a CSV's inferred dialect and field, a workbook. `read_arrow_field`, `row_size`, `column_size` and the decode that needs a footer are all answered from it. A folder, a catalog and a namespace cache nothing, and an Iceberg table keeps its own document under its commit protocol ([Iceberg](iceberg.md)).

| Handle | `cache_ttl` | A read |
| --- | --- | --- |
| open (`open()`, Python's `with`) | any | served from the entry until `close`, which drops it |
| closed | `0`, the default | realtime: the store is asked on every call |
| closed | `n` above `0` | served while the entry is younger than `n` milliseconds, then read afresh |

A write through the wrapper keeps the entry true instead of throwing it away. Where what is written is the origin - Arrow IPC, Parquet, Avro - a write sets the origin to the root it published, the rows to the rows the value now holds and the medium's object to what its encoder returned - Parquet's footer, so the write is answered with no read of the file's end; an append and a merge rewrite the whole value, so they set it the same way. Where the origin is a reading of what was written - a CSV's header and sample, an XMLA rowset, a workbook's cells, text's lines - a write drops the entry and the next ask reads afresh. `clear` sets a leaf's empty entry, `remove` ends the session, and `pwrite`, `truncate`, `create_bytes`, `set_media_type` and the borrowed `handle_mut` change the store without saying what it holds, so they drop it. A closed write stores an entry only under a `cache_ttl` above zero, and a byte length (`size`) is served from the entry while open alone, since a byte write offsets by it. A time-to-live is a statement that a change another writer makes may go unseen for that long, so it belongs on a resource this process writes, or one read often and changed rarely. Rust and Python; JavaScript has no `cache_ttl`.

### A medium's own settings

`RecordOptions` holds the core's three media - Arrow IPC, plain text and CSV - as variants of their own and every other medium - Parquet, Avro, XML for Analysis, a workbook, any medium a crate claims - as `Registered`: the medium's whole options struct, shared sections included, behind one box, so its hash and its order are the struct's. Rust reaches a medium's settings by type. `settings::<T>()` and `settings_mut::<T>()` answer the struct, `None` for another medium's options; `require_settings::<T>()` and `require_settings_mut::<T>(path, setting)` refuse them at `path`, naming both media. `codec()` is the medium the options drive. The CSV dialect stays on `RecordOptions` as `csv_separator` and the other `csv_*` readers and setters, beside `header`, which CSV and Excel share; each reads `None` on another medium's options and each setter refuses it. Python and JavaScript read a setting as a property of the options - `None`/`null` on an encoding that has none - and setting it there is refused by name.

=== "Rust"

    ```rust
    use yggdryl::avro::AvroOptions;
    use yggdryl::media::RecordOptions;
    use yggdryl::parquet::ParquetOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::PARQUET)?;
    assert_eq!(options.codec().name(), "parquet");

    // The medium's own struct answers by type, and no other medium's does.
    assert_eq!(options.settings::<ParquetOptions>().map(|parquet| parquet.max_row_group_size), Some(1_048_576));
    assert!(options.settings::<AvroOptions>().is_none());

    options
        .require_settings_mut::<ParquetOptions>("$.max_row_group_size", "a row-group size")?
        .set_max_row_group_size(250);
    assert_eq!(options.settings::<ParquetOptions>().map(|parquet| parquet.max_row_group_size), Some(250));

    // Another medium's setting is refused at its own path, naming both media.
    let refusal = options
        .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
        .unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "invalid record value at $.block_codec: expected Avro options to set a block codec, \
         got application/vnd.apache.parquet options"
    );
    ```

=== "Python"

    ```python
    from yggdryl import RecordOptions

    options = RecordOptions("trades.parquet")
    options.max_row_group_size = 250
    assert options.max_row_group_size == 250

    # Another medium's setting reads None, and setting it is refused by name.
    assert options.block_codec is None
    try:
        options.block_codec = "deflate"
    except ValueError as refusal:
        assert "expected Avro options to set a block codec" in str(refusal)
    else:
        raise AssertionError("Parquet options hold no block codec")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { RecordOptions } = require('yggdryl')

    const options = RecordOptions.from('trades.parquet').withMaxRowGroupSize(250)
    assert.equal(options.maxRowGroupSize, 250)

    // Another medium's setting reads null, and setting it is refused by name.
    assert.equal(options.blockCodec, null)
    assert.throws(() => options.withBlockCodec('deflate'), /expected Avro options to set a block codec/)
    ```

### Settings by name

Every record read and write takes `options` and, beside it, the option properties by name - keywords in Python, a plain object in JavaScript (alone in the options position, or after an options value) - each set on a copy of the options by that property's own setter, so a value is checked exactly as an assignment is and the options passed in never change. A value not given (`...` in Python, `undefined` in JavaScript) is skipped; `None`/`null` is a value and clears. A name no setter of that options class owns is not an error: it is skipped with an `UnknownPropertyWarning` naming it and the closest property there is (within a third of its length), so a typo is heard without failing the call - a `warnings` category in Python that a filter can escalate, a process warning (`code: 'YGGDRYL_UNKNOWN_PROPERTY'`) in JavaScript, heard once per process per message. A read-only name (`mime_type`) and another encoding's setting (`rowheader` on CSV) name no settable property; a known name the encoding cannot honour is the setter's own refusal. The same holds for `TextOptions`, `TextLine`, the Iceberg calls and `IcebergOptions`, and an HTTP session's `HttpOptions` properties. Rust sets each property through its typed setter - a medium's own through the struct its options hold ([A medium's own settings](#a-mediums-own-settings)) - and `HttpOptions::is_property` and `S3Options::is_property` answer, from the readers themselves, which names their property doors take, the readers `Holder::from_url` hands a location's properties to. A property spelled as text - a [plan target's](../expression/plans.md) `with (...)`, a URL's query, a catalog's property bag - is read by the one reader of its type, never a parse of its own: a flag by the [boolean table](../types/numeric/boolean.md#the-one-text-reader) (`true`, `yes`, `on`, `1` and their opposites, in any case), a count by the integer grammar, a duration and a byte size as an [HTTP session's](../holder/index.md#durations-counts-sizes-and-flags) are; a text no reader reads is refused naming the property - `$.with.safe` on a plan target. Inference is narrower than any of them: a [CSV](csv.md) column is typed `boolean` only by `true` and `false`.

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

## Registering a medium

Rust only. A medium states itself once, as a `static` in its own file, and is claimed once; every intake door reads the claim and a value in hand carries its medium, so no leaf, batch or row read looks one up. Python and JavaScript read a registered medium exactly as they read the others - by the handle's name - and gain no door.

```text
pub trait MediaCodec: Debug + Send + Sync + 'static {
    fn name(&self) -> &'static str;          // "parquet": the tag the options hash under
    fn title(&self) -> &'static str;         // "Parquet": how a refusal names the medium
    fn rank(&self) -> u8;                    // where its options sort; EXTERNAL_RANK (32) or more outside the core
    fn mime_types(&self) -> &'static [MimeType];            // every type it answers, the first canonical
    fn default_options(&self, base: &MimeType) -> RecordOptions;
    fn read_batch_reader(&self, handle: &dyn IOBase, declared: Option<&Field>, options: &RecordOptions) -> Result<BatchReader>;
    fn row_size(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<u64>;
    fn read_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Field>;
    fn overwrite_arrow_reader(&self, handle: &mut dyn IOBase, batches: BatchReader, options: &RecordOptions) -> Result<()>;
    fn open(&self, handle: Holder) -> Media;                // the stateful wrapper, as Media::Registered
    // provided: compresses_internally, has_row_identity, stated_field, read_stream
}
```

| A crate states | Where |
| --- | --- |
| a `<Name>Codec` unit struct and its `<NAME>_CODEC` static, implementing `MediaCodec` | the medium's own file |
| `MediumSettings` on its options struct - `medium()` returns the static; `mime_type`, `header` and `file_threads` where the medium has them - beside `IORecordOptions`, and `impl From<XOptions> for RecordOptions` | the options struct's file |
| `MediaWrapper` on its wrapper, an `IOBase` that names its medium, its byte handle and takes a field; its metadata in one [`MediaCache`](#the-metadata-cache), as the core's wrappers hold theirs | the wrapper's file |
| `media::codec::claim(&X_CODEC, "my-crate")` | the crate's `install()` |

A leaf door receives the leaf as `&dyn IOBase` and the options whole, and reads its own struct back with `options.require_settings::<XOptions>()?`; `compresses_internally` says an outer content coding names a file no reader of the medium opens (Parquet, a workbook), `has_row_identity` that a stored row can be matched on (plain text lines cannot), and `read_stream` is the medium's native row stream where it has one. A complete medium that stores Arrow IPC is the test medium of `rust/tests/media_register.rs`.

A claim takes every MIME type the codec names, all or none, once for the life of the process. A type claimed already is refused as `Error::Conflict` naming the first claimant; a claim in the core's own name, a codec naming no MIME type and a rank below `EXTERNAL_RANK` - the positions the core's seven hold - are refused at `$.encoding`. `codec_for` is the intake lookup, which `RecordOptions::for_mime_type`, `Media::open` and `Holder::into_media` ask once; `codecs()` lists the claims in rank order; past intake `RecordOptions::codec()` answers the medium. A medium that pushes a `where` down splits it with `expression::filter_phases`, beside `Bounds` and `Residual`: the early phase over the stored columns, the late one over the rows the `select` publishes. A MIME type no claim answers is refused as `expected a record encoding this build implements (<every claimed type>; install the crate that claims it and call its `install()`), got <type>`. The core claims its own seven media before the register answers anything, until they move to the crates that hold them.

```rust
use yggdryl::media::{codec, codec_for, codecs, EXTERNAL_RANK, RecordOptions};
use yggdryl::MimeType;

// The core's media are claimed before the register answers anything, in rank order.
let parquet = codec_for(&MimeType::PARQUET)?;
assert_eq!((parquet.name(), parquet.title(), parquet.rank()), ("parquet", "Parquet", 1));
assert!(codecs().windows(2).all(|pair| pair[0].rank() <= pair[1].rank()));

// Options reach their medium through the claim; past intake they carry it.
assert_eq!(RecordOptions::for_mime_type(&MimeType::PARQUET)?.codec().name(), "parquet");

// A type no claim answers is refused naming the crate to install.
let refusal = RecordOptions::for_mime_type(&MimeType::ORC).unwrap_err().to_string();
assert!(refusal.contains("install the crate that claims it and call its `install()`"), "{refusal}");
assert!(refusal.contains("got application/vnd.apache.orc"), "{refusal}");

// The core's ranks and names are never another crate's.
assert_eq!(EXTERNAL_RANK, 32);
let refusal = codec::claim(&yggdryl::ipc::IPC_CODEC, "my-crate").unwrap_err().to_string();
assert!(refusal.contains("ranks at or above 32, got 0 for `ipc`"), "{refusal}");
```
