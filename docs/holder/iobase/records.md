# Records

Every handle answers one Arrow batch read and three explicit write intents, and this page owns that record surface.

## Contract

| Item | Value |
| --- | --- |
| Owns | `read_arrow_reader`, `read_arrow_field`, `read_records`, `row_size`, `column_size`, and the `overwrite_*` / `append_*` / `merge_*` triplets |
| Encoding | `record_options()` derives it from the handle's media type; a coding such as `.zst` lives there too, so no call takes a format or coding argument |
| Write intents | `overwrite`, `append`, `merge`; `write_*` takes an explicit `IOMode`; no default mode and no untyped Rust `write` |
| Argument order | input, then mode when present, then options; Rust requires `&RecordOptions`, a binding resolves one default with `record_options()` |
| Merge keys | `merge_by_names` supplies row identity only; merge requires it, overwrite and append refuse it |
| Shaping order | declared field, then `select_by_names`, then completion cast, then partition filter, then `max_row_size` / `max_byte_size` last |
| Commits | `commit_row_size` unset publishes once when the source ends; `N` publishes every `N` rows plus the remainder; `0` is rejected before any input is pulled |
| Lazy | reads stream one batch at a time; append chains stored then incoming batches; merge indexes only the stored side |
| Feature flag | default `arrow` carries Arrow IPC, Avro, and text; `parquet` adds Parquet; an encoding the build lacks is named in the `record_options` error |
| Bindings | Python takes keyword-only `options=`; JavaScript takes a trailing `options?`; `scan_polars` / `scan_arrow` are Python only |

## Use

The handle's media type picks the encoding, and batches arrive one at a time.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, Url};

    // A non-null struct Field is the schema.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    // The handle's own media type picks the encoding; no format argument is passed.
    let mut handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?;

    // Overwrite takes a batch reader; the method name fixes the write intent.
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
    assert_eq!(handle.read_arrow_field(&options)?, schema);
    assert_eq!((handle.row_size()?, handle.column_size()?), (2, 2));

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&options)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    # A PyArrow schema is the schema; the binding imports it once at the boundary.
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = pa.record_batch({"id": [1, 2], "symbol": ["AAPL", None]}, schema=schema)

    # The handle's own media type picks the encoding; no format argument is passed.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()

    # The write path takes a batch reader and nothing else.
    handle.overwrite_arrow_batch(batch, options=options)
    assert handle.read_arrow_field(options=options).name == "row"

    # The read path returns one. Batches arrive one at a time, never as a vector.
    rows = sum(part.num_rows for part in handle.read_arrow_reader(options=options))
    assert rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    // A non-null struct Field is the schema.
    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The handle's own media type picks the encoding; no format argument is passed.
    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions()

    // The write path takes a batch reader and nothing else.
    handle.overwriteArrowReader(BatchReader.from(table), options)
    assert.ok(handle.readArrowField(options).equals(schema))

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let rows = 0
    for (const batch of handle.readArrowReader(options)) {
      rows += batch.numRows
    }
    assert.equal(rows, 2)
    ```

## Dimensions

`row_size` and `column_size` describe the whole logical media, so projection, filter, and limit settings never change them. Schema-bearing encodings read headers or footers only; text streams its multiline extractor; a folder sums matching leaves; a table answers from metadata.

| accessor | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| row count | `row_size(&self) -> Result<u64>` | `row_size` property | `rowSize` getter |
| column count | `column_size(&self) -> Result<usize>` | `column_size` property | `columnSize` getter |
| storage role | `IOKind` | `kind` property: `memory`, `file`, `directory`, `table`, `namespace`, `catalog`, `unknown` | `kind` getter, same names |

## Write intents

Each typed shape has three explicit methods and one `IOMode` dispatcher that validates intent before touching its input. Every adapter converges on the three reader primitives, where [BatchReader](../../arrow/readers.md) is the only place an encoding is decoded and encoded.

=== "Rust"

    ```text
    overwrite_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>
    append_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>
    merge_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>

    overwrite_arrow_batch(&mut self, batch: RecordBatch, options: &RecordOptions) -> Result<()>
    append_arrow_batch(&mut self, batch: RecordBatch, options: &RecordOptions) -> Result<()>
    merge_arrow_batch(&mut self, batch: RecordBatch, options: &RecordOptions) -> Result<()>

    overwrite_records(&mut self, records, options: &RecordOptions) -> Result<()>
    append_records(&mut self, records, options: &RecordOptions) -> Result<()>
    merge_records(&mut self, records, options: &RecordOptions) -> Result<()>

    write_arrow_reader(&mut self, reader: BatchReader, mode: IOMode, options: &RecordOptions) -> Result<()>
    write_arrow_batch(&mut self, batch: RecordBatch, mode: IOMode, options: &RecordOptions) -> Result<()>
    write_records(&mut self, records, mode: IOMode, options: &RecordOptions) -> Result<()>
    ```

=== "Python"

    ```text
    read_arrow_reader(*, options=None) -> pyarrow.RecordBatchReader
    read_records(cls=None, *, options=None) -> Iterator[dict | dataclass]
    overwrite|append|merge_arrow_reader(reader, *, options=None) -> None
    overwrite|append|merge_arrow_table(table, *, options=None) -> None
    overwrite|append|merge_arrow_batch(batch, *, options=None) -> None
    overwrite|append|merge_records(records, *, options=None) -> None
    write_arrow_reader(reader, mode, *, options=None) -> None
    write_arrow_table(table, mode, *, options=None) -> None
    write_arrow_batch(batch, mode, *, options=None) -> None
    write_records(records, mode, *, options=None) -> None
    ```

=== "JavaScript"

    ```text
    readArrowReader(options?) -> BatchReader
    readRecords(options?) -> Iterable<object>
    readRecords(cls, options?) -> Iterable<object>
    overwrite|append|mergeArrowReader(reader, options?) -> void
    overwrite|append|mergeArrowTable(table, options?) -> void
    overwrite|append|mergeArrowBatch(batch, options?) -> void
    overwrite|append|mergeRecords(records, options?) -> void | Promise<void>
    writeArrowReader(reader, mode, options?) -> void
    writeArrowTable(table, mode, options?) -> void
    writeArrowBatch(batch, mode, options?) -> void
    writeRecords(records, mode, options?) -> void | Promise<void>
    ```

`overwrite_arrow_reader` is the one publication hook an implementor must supply. Default append and merge cast to `options.field` once, apply selection and limits once, then delegate with the field removed so the hook never casts twice.

### Native rows

The Rust `*_records` triplet takes any iterator whose row implements `TryInto<Scalar>`: an ordered `Scalar::Sequence` under `options.field`, or a sorted `Scalar::Record` resolved to that order. Rust only.

```rust
use yggdryl::media::IORecordOptions;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, MimeType, Scalar};

struct Quote(i32, &'static str);

impl From<Quote> for Scalar {
    fn from(row: Quote) -> Self {
        Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
    }
}

let field = DataType::from_fields([
    DataType::Int32.required_field("id"),
    DataType::Utf8.required_field("symbol"),
])?
.required_field("quote");
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?.with_field(field);

handle.overwrite_records([Quote(1, "AAPL"), Quote(2, "MSFT")], &options)?;
handle.append_records([Quote(3, "AMD")], &options)?;
handle.merge_records(
    [Quote(2, "MSFT.O")],
    &options.clone().with_merge_by_names(["id"]),
)?;
assert_eq!(
    handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum::<usize>(),
    3,
);
```

### Commit cadence

`options.commit_row_size` is the one streamed-write publication boundary; shaping happens once before splitting, and the splitter slices batches as views.

| setting or intent | publication |
| --- | --- |
| unset | once, when the source ends; a leaf or table publishes natively, so nothing is visible until the replacement completes |
| `N > 0` | every complete group of `N` incoming rows, then the final remainder |
| `0` | rejected before the input is pulled |
| overwrite | overwrite on the first commit, append on every later one |
| append, merge | keep their intent on every commit |
| empty input | overwrite still publishes its shaped field; append and merge are no-ops |
| plain folder | no cross-leaf transaction: each routed leaf publishes independently even when the cadence is unset |
| Iceberg folder | redirected first and published through its [snapshot commit](../../media/iceberg/write.md) |

A successful prefix stays visible when conversion, decoding, encoding, or publication fails later; the cadence only bounds how many rows each visible prefix holds.

### Absent resources and unknown encodings

An absent resource reads as empty, and an encoding the build does not implement is named rather than guessed.

=== "Rust"

    ```rust
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::MimeType;

    // An absent resource holds no batches rather than failing to parse.
    let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    assert_eq!(
        empty.read_arrow_reader(&empty.record_options()?)?.count(),
        0
    );

    // An encoding this build does not implement is named rather than guessed.
    let csv = Buffer::new().with_media_type(MimeType::CSV.into());
    let message = csv.record_options().unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # An absent resource holds no batches rather than failing to parse.
    empty = IOBase(root / "absent.arrows")
    assert empty.read_arrow_reader().read_all().num_rows == 0

    # An encoding this build does not implement is named rather than guessed.
    csv = IOBase(root / "trades.csv")
    with pytest.raises(ValueError, match="text/csv"):
        csv.record_options()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    // An absent resource holds no batches rather than failing to parse.
    const empty = IOBase.fromBytes()
    empty.mediaType = MimeType.ARROW_STREAM
    assert.equal([...empty.readArrowReader()].length, 0)

    // An encoding this build does not implement is named rather than guessed.
    const csv = IOBase.fromBytes()
    csv.mediaType = MimeType.CSV
    assert.throws(() => csv.recordOptions(), /text\/csv/)
    ```

The shared settings (field, root name, cast strictness, limits, cadence, compression level, merge keys, selection, partition filters) are defined once on [RecordOptions](../../media/options.md).

## Rows

`readRecords` yields plain objects or instances of the constructor it is given; the `*Records` writers widen rows into the matching reader intent. Python exposes the same lazy `read_records` mapping or dataclass view and all three write intents; Rust accepts `TryInto<Scalar>` rows on writes only. JavaScript only.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
const handle = new IOBase(path.join(root, 'trades.arrows'))
// Plain objects are rows; `overwriteRecords` widens them into the Arrow stream.
handle.overwriteRecords([
  { id: 1n, venue: 'XNAS' },
  { id: 2n, venue: null },
])

// Plain objects out, streamed batch by batch ...
assert.deepEqual([...handle.readRecords()].map((row) => row.id), [1n, 2n])

// ... or instances of any class whose constructor takes the plain row.
class Trade {
  constructor(row) {
    Object.assign(this, row)
  }
}
const trades = [...handle.readRecords(Trade)]
assert.ok(trades.every((t) => t instanceof Trade))

// An absent resource yields no records rather than raising.
assert.deepEqual([...new IOBase(path.join(root, 'absent.arrows')).readRecords()], [])

fs.rmSync(root, { recursive: true, force: true })
```

## Lazy scans

`scan_polars` returns a `polars.LazyFrame` and `scan_arrow` a `pyarrow.dataset.Scanner`; a plain local Parquet leaf is scanned natively, everything else streams through the native reader into the same lazy shape. Python only.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
IOBase(target).overwrite_arrow_table(pa.table({"symbol": ["AAPL", "MSFT"], "price": [187.23, 402.11]}))

# A local Parquet leaf becomes the real lazy scan - projection and
# predicate pushdown belong to the engine, and the handle publishes its
# bytes at their exact length first so the foreign reader sees a whole file.
lazy = IOBase(target).scan_polars()
assert lazy.select("symbol").head(10).collect().height == 2

# The pyarrow spelling of the same idea, as a dataset Scanner.
scanner = IOBase(target).scan_arrow()
assert scanner.to_table().num_rows == 2

# Anything a foreign scanner cannot mmap - an in-memory buffer, a
# compressed name, an Arrow stream - streams through the native reader
# instead, so both calls answer for every holder.
memory = IOBase.from_bytes()
memory.media_type = "application/vnd.apache.arrow.stream"
memory.overwrite_arrow_table(pa.table({"symbol": ["AAPL"]}))
assert memory.scan_arrow().to_table().num_rows == 1
```

## Column pushdown

The field on the options selects and casts in one pass: stored columns it names become the encoding's own projection, so omitted columns are skipped rather than read and discarded. The cast then reorders, converts, and fills columns the resource lacks, batch by batch as each is pulled.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let plain = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

    // One of the three columns, declared as this read's schema.
    let wanted = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let projected = handle.read_arrow_reader(&plain.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 1);
    assert_eq!(projected.map(|batch| batch.unwrap().num_columns()).sum::<usize>(), 1);

    // The resource is unchanged: it still holds all three.
    assert_eq!(handle.read_arrow_field(&plain)?.field_len(), 3);

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    let invented = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("nowhere"),
    ])?
    .required_field("row");
    let widened = handle.read_arrow_reader(&plain.with_field(invented))?;
    assert_eq!(widened.schema().fields().len(), 2);
    assert_eq!(widened.schema().field(1).name(), "nowhere");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    stored = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch)

    # One of the three columns, declared as this read's schema.
    options = handle.record_options()
    options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    projected = handle.read_arrow_reader(options=options)
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The resource is unchanged: it still holds all three.
    assert len(handle.read_arrow_field().dtype) == 3

    # A column it does not hold cannot be projected out of it, so the encoding
    # reads everything and the cast supplies that column as nulls.
    options.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("nowhere", pa.string()),
    ])
    widened = handle.read_arrow_reader(options=options)
    assert widened.schema.names == ["id", "nowhere"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
    })

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(BatchReader.from(table))

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const options = handle.recordOptions()

    const projected = handle.readArrowReader(options.withField(wanted))
    assert.equal(projected.field.dtype.length, 1)
    assert.equal(projected.intoTable().numCols, 1)

    // The resource is unchanged: it still holds all three.
    assert.equal(handle.readArrowField().dtype.length, 3)

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    const invented = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('nowhere: utf8?')],
      { nullable: false },
    )
    const widened = handle.readArrowReader(options.withField(invented))
    assert.equal(widened.field.dtype.length, 2)
    ```

| encoding | what a projection saves |
| --- | --- |
| [Parquet](../../media/parquet.md) | the decode and the bytes, because a column chunk is separately addressable |
| [Arrow IPC](../../media/ipc.md) | the decode and the allocation; the message body is still read |

## Limits

`max_row_size` counts result rows and `max_byte_size` counts their uncompressed Arrow bytes; both apply last, after the shaping order, and a satisfied limit stops pulling.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from_iter_values(0..1_000))],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let plain = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

    // Ten result rows, exactly: the batch the bound lands inside is sliced.
    let first = handle.read_arrow_reader(&plain.clone().with_max_row_size(10))?;
    assert_eq!(first.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 10);

    // Zero is a valid ask: the shaped schema answers, and no batch flows.
    let mut none = handle.read_arrow_reader(&plain.clone().with_max_row_size(0))?;
    assert_eq!(none.schema().fields().len(), 1);
    assert!(none.next().is_none());

    // A non-zero byte bound always yields at least one row.
    let narrow = handle.read_arrow_reader(&plain.clone().with_max_byte_size(1))?;
    assert_eq!(narrow.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 1);

    // A limited write truncates the data the caller offered: three rows land,
    // and what the bound cut off is never pulled from the reader.
    let mut copy = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    copy.overwrite_arrow_reader(
        handle.read_arrow_reader(&plain)?,
        &plain.clone().with_max_row_size(3),
    )?;
    let kept = copy.read_arrow_reader(&plain)?;
    assert_eq!(kept.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.media_type = "application/vnd.apache.arrow.stream"
    handle.overwrite_arrow_table(pa.table({"id": list(range(1_000))}))

    # Ten result rows, exactly: the batch the bound lands inside is sliced.
    ten = handle.record_options()
    ten.max_row_size = 10
    assert handle.read_arrow_reader(options=ten).read_all().num_rows == 10

    # Zero is a valid ask: the shaped schema answers, and no batch flows.
    zero = handle.record_options()
    zero.max_row_size = 0
    empty = handle.read_arrow_reader(options=zero)
    assert empty.schema.names == ["id"]
    assert empty.read_all().num_rows == 0

    # A non-zero byte bound always yields at least one row.
    one_byte = handle.record_options()
    one_byte.max_byte_size = 1
    assert handle.read_arrow_reader(options=one_byte).read_all().num_rows == 1

    # A limited write truncates the data the caller offered: three rows land,
    # and what the bound cut off is never pulled from the reader.
    copy = IOBase.from_bytes()
    copy.media_type = "application/vnd.apache.arrow.stream"
    first_three = copy.record_options()
    first_three.max_row_size = 3
    copy.overwrite_arrow_reader(handle.read_arrow_reader(), options=first_three)
    assert copy.read_arrow_reader().read_all().num_rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray(
        Array.from({ length: 1000 }, (_, index) => BigInt(index)),
        new arrow.Int64(),
      ),
    })

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(BatchReader.from(table))
    const options = handle.recordOptions()

    // Ten result rows, exactly: the batch the bound lands inside is sliced.
    assert.equal(handle.readArrowReader(options.withMaxRowSize(10)).intoTable().numRows, 10)

    // Zero is a valid ask: the shaped schema answers, and no batch flows.
    const empty = handle.readArrowReader(options.withMaxRowSize(0))
    assert.equal(empty.field.dtype.length, 1)
    assert.equal(empty.intoTable().numRows, 0)

    // A non-zero byte bound always yields at least one row.
    assert.equal(handle.readArrowReader(options.withMaxByteSize(1)).intoTable().numRows, 1)

    // A limited write truncates the data the caller offered: three rows land,
    // and what the bound cut off is never pulled from the reader.
    const copy = IOBase.fromBytes()
    copy.mediaType = MimeType.ARROW_STREAM
    copy.overwriteArrowReader(handle.readArrowReader(), options.withMaxRowSize(3))
    assert.equal(copy.readArrowReader().intoTable().numRows, 3)
    ```

## Append and merge

Overwrite replaces the resource, append retains stored rows, and merge updates matching keys while adding new ones; the called method is always the authority.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, Url};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let rows = |ids: Vec<i64>, symbols: Vec<&'static str>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?.with_field(schema.clone());

    // Overwrite replaces the resource.
    handle.overwrite_arrow_reader(rows(vec![1, 2], vec!["AAPL", "MSFT"]), &options)?;

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_reader(rows(vec![3], vec!["NVDA"]), &options)?;
    let total: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 3);

    // Merge requires a match key: `2` updates and `9` appends.
    let merging = options.clone().with_merge_by_names(["id"]);
    handle.merge_arrow_reader(rows(vec![2, 9], vec!["MSFT.O", "AMD"]), &merging)?;
    let total: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    rows = lambda ids, symbols: pa.record_batch(
        {"id": ids, "symbol": symbols}, schema=schema
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()
    options.field = schema

    # No match key: the resource is replaced.
    handle.overwrite_arrow_batch(rows([1, 2], ["AAPL", "MSFT"]), options=options)

    # Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_batch(rows([3], ["NVDA"]), options=options)
    assert handle.read_arrow_reader(options=options).read_all().num_rows == 3

    # A match key merges: `2` is already stored and updates, `9` is new and appends.
    merging = handle.record_options()
    merging.field = schema
    merging.merge_by_names = ["id"]
    handle.merge_arrow_batch(rows([2, 9], ["MSFT.O", "AMD"]), options=merging)
    assert handle.read_arrow_reader(options=options).read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8?')],
      { nullable: false },
    )
    const rows = (ids, symbols) =>
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray(ids, new arrow.Int64()),
          symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
        }),
      )

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions().withField(schema)

    // No match key: the resource is replaced.
    handle.overwriteArrowReader(rows([1n, 2n], ['AAPL', 'MSFT']), options)

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.appendArrowReader(rows([3n], ['NVDA']), options)
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 3)

    // A match key merges: `2` is already stored and updates, `9` is new and appends.
    const merging = options.withMergeByNames(['id'])
    handle.mergeArrowReader(rows([2n, 9n], ['MSFT.O', 'AMD']), merging)
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 4)
    ```

| key situation | result |
| --- | --- |
| null key | matches another null key exactly, through Arrow's own row format |
| composite key | compares column by column |
| key stored more than once | every occurrence is updated |
| key arriving more than once | the last arrival wins |

Append streams both sides after casting the incoming batches to the target shape, so data whose schema merely fits is accepted. Merge holds only the stored side, because a reader cannot rewind to a row it has already yielded.

### Selecting columns

`select_by_names` narrows both directions: a read yields exactly the named columns in the given order, and a write keeps exactly those columns of the incoming rows.

=== "Rust"

    ```rust
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{arrow, DataType, MimeType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(batch.schema(), [batch]), &options)?;

    // A read narrowed to one column yields one column.
    let selecting = options.with_select_by_names(["symbol"]);
    let first = handle.read_arrow_reader(&selecting)?.next().unwrap()?;
    assert_eq!(first.num_columns(), 1);
    assert_eq!(first.schema().field(0).name(), "symbol");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "orders.arrows")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}))

    # Record settings live on the one options object shared by every operation.
    options = handle.record_options()
    options.select_by_names = ["symbol"]
    narrowed = handle.read_arrow_reader(options=options).read_all()
    assert narrowed.column_names == ["symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'orders.arrows'))
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      }),
    )

    const narrowed = handle.recordOptions().withSelectByNames(['symbol'])
    const table = handle.readArrowReader(narrowed).intoTable()
    assert.deepEqual(table.schema.fields.map((field) => field.name), ['symbol'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Text records

`text/plain` uses the same record methods as IPC, Parquet, and Avro; each physical line becomes one row, and [Text records](../../media/text.md) defines the schema, parsing order, errors, and benchmarks.

| column | datatype | when |
| --- | --- | --- |
| `url` | `utf8` | always, first |
| `rownum` | `int64` | `TextOptions.with_rownum` / `withRownum`, which also supplies its first value |
| `body` | `binary` | always, last |

`into_text` / `intoText` keeps a flat `TextOptions` (regex `rowheader` captures, edge stripping, a fixed line separator, pre-read autotyping) and adds no line iterator or line-only method.

## Edges

- `record_options()` on an encoding this build lacks -> error naming the media type, for example `text/csv`.
- absent resource -> a reader with no batches; `readRecords` yields nothing; no existence guard is needed.
- empty `overwrite` -> publishes the shaped field; empty `append` / `merge` -> no-op.
- non-empty `merge_by_names` on overwrite or append -> refused, directing the caller to merge; empty on merge -> refused, directing to overwrite or append.
- empty row source without `options.field` -> refused; a non-empty source may infer the field from its decorated row class.
- `max_row_size = 0` -> the shaped schema and no batches, not an error.
- row bound -> exact; the batch it lands inside is sliced as a view over the same buffers, never copied.
- non-zero `max_byte_size` -> stops at the last row keeping the running total under the limit, and always yields at least one row.
- both bounds set -> whichever binds first wins.
- limit with non-empty `merge_by_names` -> refused naming both settings, because a truncated merge would silently drop unmatched keys.
- limited write -> truncates the offered rows exactly as a read; the cut-off rows are never pulled from the reader.
- declared field on an overwrite of a resource that stores one -> rows are cast to the stored field; clear the handle first to change it.
- `select_by_names` naming a column the rows lack -> error listing what is there; names match ASCII case-insensitively; an empty list selects everything.
- no declared field on a read -> the stored shape is preserved exactly and no cast runs.
- `read_arrow_field` -> the shape this read produces, so schema and batches never disagree.
- `row_size` / `column_size` on an explicitly opened media -> cached until `close`; a closed handle asks the resource afresh each time.
- `column_size` with a declared Struct field and no rows -> the declared field stays authoritative.
- native Rust row conversion -> bounded by the smaller of `batch_row_size` and `commit_row_size`, so a failure at row `N + 1` never erases the committed `N` rows.
- plain folder write failing on a later leaf -> already-published leaves stay visible; see [Partitions](partitions.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::records
    cargo bench --bench media --features parquet -- io_dimensions
    cargo bench --bench media --features parquet -- io_write_mode_dispatch
    cargo bench --bench media --features parquet -- io_write_commit_rows
    cargo bench --bench media --features parquet -- io_write_records
    cargo bench --bench media --features parquet -- io_pushdown
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py -k "Scans or Partitions or PathlibParity"
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/holder/io.test.js"
    npm run --prefix node bench:holder:io
    ```

## Performance

Reader, held-batch, and native-row dispatch under all three modes over 4,096 rows: one local Windows x86_64 release run, Criterion point estimates, regenerate on the deployment host.

| generic dispatcher | overwrite | append | merge |
| --- | ---: | ---: | ---: |
| `write_arrow_reader` | 255 us (16.1M rows/s) | 703 us (5.83M rows/s) | 3.27 ms (1.25M rows/s) |
| `write_arrow_batch` | 92.0 us (44.5M rows/s) | 637 us (6.43M rows/s) | 4.14 ms (989k rows/s) |
| `write_records` | 3.00 ms (1.36M rows/s) | 4.44 ms (922k rows/s) | 8.96 ms (457k rows/s) |

The mode branch is shared across a row; append re-encodes the stored side, merge indexes it by key, and native records also validate and materialize ordered `Scalar::Sequence` rows.

```bash
cargo bench --bench media --features parquet -- io_write_mode_dispatch
```
