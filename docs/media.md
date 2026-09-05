# Record media

Record media stream Arrow batches through one holder and one options contract.

## Media: a record encoding over a handle


`Media` is to a specialized record encoding what `Holder` is to a handle.
`Media::open` reads the handle's declared media type and binds the IPC,
Parquet, or Avro implementation it names. Plain text needs no wrapper: every
`IOBase` reaches it through ordinary `IOMedia` dispatch and
[`RecordOptions`](media.md#plain-text-records). Nothing is read to decide.

```rust
use yggdryl::holder::Holder;
use yggdryl::media::Media;
use yggdryl::holder::Buffer;
use yggdryl::Url;

fn named(name: &str) -> Result<Holder, Box<dyn std::error::Error>> {
    let url = Url::from_str(&format!("file:///{name}"))?;
    Ok(Holder::buffer(Buffer::new().with_media_type(url.media_type())))
}

assert!(matches!(Media::open(named("trades.arrows")?)?, Media::Ipc(_)));
assert!(matches!(Media::open(named("trades.parquet")?)?, Media::Parquet(_)));
```

Choosing the encoding is the only thing that changes. Every variant implements
`IOMedia`: `record_options` returns its held defaults, `read_arrow_field` and
`read_arrow_reader` answer its shape and batches, and the three explicit write
methods consume an [`arrow::BatchReader`](arrow.md). Their signatures and
validation rules are documented once in
[holder.md](holder.md#canonical-record-write-signatures).

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::holder::Holder;
use yggdryl::media::Media;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let url = Url::from_str("file:///trades.arrows")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_field(schema.clone());
let options = media.record_options()?;

media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
assert_eq!(media.read_arrow_field(&options)?, schema);

// A Media is also the bytes it encodes: an Arrow IPC stream opens with its
// continuation marker.
assert_eq!(media.read_range_bytes(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

#### Measured generic media redirection

`io_write_stateful/media_ipc` exercises the generic enum over its IPC variant with the same
4,096-row, four-column fixture as the concrete media pages. Criterion prepares the stored side for
append and keyed merge outside the timer, so the result covers the generic redirection and the
selected operation, not fixture construction.

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them with
`cargo bench -p yggdryl --bench io --all-features -- io_write_stateful/media_ipc`. Sub-millisecond
point estimates include allocator variance and are regression anchors, not a claim that enum
dispatch makes encoding faster; the enum redirects to the same IPC implementation.

The content coding is the handle's business, not the encoding's. A name that declares both gives the same calls and different bytes underneath.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::holder::Holder;
use yggdryl::media::Media;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![9]))],
)?;

let url = Url::from_str("file:///trades.arrows.gz")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));
let mut media = Media::open(handle)?.with_field(schema.clone());
let options = media.record_options()?;

media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);

// Still an Arrow IPC stream, now behind gzip framing.
assert_eq!(media.read_range_bytes(0, 2)?, [0x1F, 0x8B]);
```

An encoding with no implementation in this build is reported, never guessed at. The error names the media type that was found and the ones that would have worked.

```rust
use yggdryl::holder::Holder;
use yggdryl::media::Media;
use yggdryl::holder::Buffer;
use yggdryl::Url;

let url = Url::from_str("file:///trades.csv")?;
let handle = Holder::buffer(Buffer::new().with_media_type(url.media_type()));

let message = Media::open(handle).unwrap_err().to_string();
assert!(message.contains("text/csv"), "{message}");
```

`Media::ipc` and `Media::parquet` name a variant directly when the encoding is already known, and `Media::open_as` takes an explicit `MimeType` when the handle's own name cannot be trusted.


## RecordOptions: every encoding's settings


!!! note "All three"
    Python and JavaScript expose this as `RecordOptions`, derived from a media
    type and carrying every encoding's settings on one value; the encoding-
    specific structs behind it stay in Rust.

Reading rows out of an Arrow IPC stream and out of a Parquet file need the same handful of answers: what to call the root, what datatype and metadata it declares, how strict a cast may be, how many rows per batch, how hard to compress. `IORecordOptions` is that shared surface; `RecordOptions` is the enum naming every encoding's options.

The declared root is three parts, and `field` is built from them on every ask:

| part | default | declares |
| --- | --- | --- |
| `name` | `media::DEFAULT_ROOT_NAME` - `"row"` | the root Field name, of a declared field and of an inferred one alike |
| `dtype` | none | the root datatype; without one nothing is declared and the shape is inferred |
| `metadata` | empty | the root metadata; it reaches a read or write only through the field a `dtype` builds |

`field()` answers the non-null Struct root those parts spell, or nothing when no `dtype` is
declared, so a part changed after the last ask is never stale against it. `set_field` and
`with_field` decompose a `Field` into the three parts; its nullability and dictionary options are
not part of a declaration and are dropped. `take_field` returns the build and clears `dtype` and
`metadata`, keeping `name`. Because `name` and `metadata` always have one stored form, two
options declaring the same root compare and hash equal however they were declared:
`with_field(f)` equals `with_dtype(f.dtype().clone())` when `f` is named `"row"` and carries no
metadata.

`batch_row_size` is the rows-per-batch bound. It counts rows, which is what its name says; the
`batch_size` of [`pstream_bytes`](holder.md) counts bytes and keeps that name.

`RecordOptions` is also a complete Rust value: it implements `Clone`, `Eq`,
`Ord`, and `Hash`, including the encoding variant in its identity.
`stable_hash()` is deterministic across runs and redirects to that variant's
full configuration. The `lines_identity/stable_hash/record_options` Criterion
case measures this path with setup outside the timed loop.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_row_size(1024);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), "row");
    assert_eq!(options.dtype(), Some(schema.dtype()));
    assert!(options.metadata().is_empty());
    assert_eq!(options.batch_row_size(), Some(1024));
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
    options.commit_row_size = 10_000

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.name == "row"
    assert [child.name for child in options.dtype] == ["id"]
    assert options.metadata == {}
    assert options.field is not None
    assert options.batch_row_size == 1024
    assert options.commit_row_size == 10_000

    # A setting one encoding has reads as None on an encoding that has none.
    assert options.max_row_group_size == 1_048_576
    assert RecordOptions("trades.arrows").max_row_group_size is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const options = RecordOptions.from('trades.parquet')
      .withField(schema)
      .withBatchRowSize(1024)

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.equal(options.name, 'row')
    assert.ok(options.dtype.equals(schema.dtype))
    assert.deepEqual(options.metadata, [])
    assert.ok(options.field.equals(schema))
    assert.equal(options.batchRowSize, 1024)

    // A setting one encoding has reads as null on an encoding that has none.
    assert.equal(options.maxRowGroupSize, 1_048_576)
    assert.equal(RecordOptions.from('trades.arrows').maxRowGroupSize, null)
    ```

Each part changes alone, and the next `field` reflects it:

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, Metadata, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let mut options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema.clone());

    // One stored form: declaring only the datatype is the same declaration.
    let by_dtype = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_dtype(schema.dtype().clone());
    assert_eq!(options, by_dtype);
    assert_eq!(options.stable_hash(), by_dtype.stable_hash());

    options.set_name("trade".into());
    assert_eq!(options.field().unwrap().name(), "trade");
    assert_eq!(options.field().unwrap().dtype(), schema.dtype());

    options.set_metadata(Metadata::from_entries([("source", "exchange")])?);
    assert_eq!(options.field().unwrap().get_metadata("source"), Some("exchange"));

    let widened = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?;
    options.set_dtype(Some(widened.clone()));
    let built = options.field().unwrap();
    assert_eq!(built.name(), "trade");
    assert_eq!(built.dtype(), &widened);
    assert_eq!(built.get_metadata("source"), Some("exchange"));
    assert!(!built.is_nullable());

    // Taking the field clears the datatype and metadata; the name stays.
    assert_eq!(options.take_field(), Some(built));
    assert!(options.field().is_none());
    assert_eq!(options.name(), "trade");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, RecordOptions

    schema = Field("row", DataType.from_fields([Field("id", "int64", nullable=False)]), nullable=False)
    options = RecordOptions("trades.arrows")
    options.field = schema

    # One stored form: declaring only the datatype is the same declaration.
    by_dtype = RecordOptions("trades.arrows")
    by_dtype.dtype = schema.dtype
    assert options == by_dtype
    assert options.stable_hash() == by_dtype.stable_hash()

    options.name = "trade"
    assert options.field.name == "trade"
    assert options.field.dtype == schema.dtype

    options.metadata = {"source": "exchange"}
    assert options.field.metadata["source"] == "exchange"

    # The setter takes a datatype expression as readily as a DataType.
    options.dtype = "struct<id: int64, venue: utf8>"
    built = options.field
    assert built.name == "trade"
    assert [child.name for child in built.dtype] == ["id", "venue"]
    assert built.metadata["source"] == "exchange"
    assert not built.nullable

    # None clears a part; the name stays.
    options.dtype = None
    options.metadata = None
    assert options.field is None
    assert options.metadata == {}
    assert options.name == "trade"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const options = new RecordOptions('trades.arrows')
    options.field = schema

    // One stored form: declaring only the datatype is the same declaration.
    const byDtype = new RecordOptions('trades.arrows').withDtype(schema.dtype)
    assert.ok(options.equals(byDtype))
    assert.equal(options.stableHash(), byDtype.stableHash())

    options.name = 'trade'
    assert.equal(options.field.name, 'trade')
    assert.ok(options.field.dtype.equals(schema.dtype))

    // Entries, a plain object, or a Map declare the metadata alike.
    options.metadata = { source: 'exchange' }
    assert.deepEqual(options.metadata, [{ key: 'source', value: 'exchange' }])
    assert.equal(options.field.get('source'), 'exchange')

    // The setter takes a datatype expression as readily as a DataType.
    options.dtype = 'struct<id: int64, venue: utf8>'
    const built = options.field
    assert.equal(built.name, 'trade')
    assert.deepEqual([...built.dtype].map((child) => child.name), ['id', 'venue'])
    assert.equal(built.get('source'), 'exchange')
    assert.equal(built.nullable, false)

    // null clears a part; the name stays.
    options.dtype = null
    options.metadata = []
    assert.equal(options.field, null)
    assert.deepEqual(options.metadata, [])
    assert.equal(options.name, 'trade')
    ```

The options are also where option-driven casting is *defined*, once. `cast_arrow_batch` - and its
streaming sibling `cast_arrow_reader` - applies three layers in order: the declared schema says
what the rows are meant to be, `select_by_names` narrows and orders the columns, and the optional
`existing` root - a holder's stored shape - is what the rows are finally cast onto, always safely,
so a value that will not convert into a stored column becomes null rather than redefining that
column for every reader. Every write path routes through this one definition, which is why a
declared schema, a selection, and a stored shape can never disagree about what a cast means.

```rust
use arrow_array::RecordBatch;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, MimeType};

let declared = DataType::from_fields([
    DataType::Utf8.required_field("symbol"),
    DataType::Int64.required_field("price"),
])?
.required_field("row");

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
    .with_field(declared.clone())
    .with_select_by_names(["price"]);

// One call is the whole pipeline: the declared cast, then the selection.
// Passing a stored root as the second argument adds the completion layer.
let batch = RecordBatch::new_empty(declared.into_arrow_schema()?);
let cast = options.cast_arrow_batch(batch, None)?;
assert_eq!(cast.num_columns(), 1);
```

There is no shared settings struct threaded through the encodings. Each one stores the shared settings as its own flat public fields - `name`, `dtype`, `metadata`, `safe`, `batch_row_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, `level`, `merge_by_names`, `select_by_names`, `filter_partitions` - and implements `IORecordOptions` over them, so a concrete options value takes the same builders the enum does and converts into it. `commit_row_size` is the optional publication cadence shared by every encoding: unset publishes once, while non-zero `N` publishes complete `N`-row prefixes and the final remainder. A setting an encoding has no use for is still there and still ignored: [`ParquetOptions::level`](media.md) is unused, because Parquet compresses pages inside the file and an outer content coding would produce something no Parquet reader can open.

```rust
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::media::ipc::IpcOptions;
use yggdryl::MimeType;

let options: RecordOptions = IpcOptions::new()
    .with_name("trade")
    .with_safe(false)
    .with_commit_row_size(10_000)
    .into();

assert_eq!(options.mime_type(), MimeType::ARROW_STREAM);
assert_eq!(options.name(), "trade");
assert!(!options.safe());
assert_eq!(options.commit_row_size(), Some(10_000));
```

A datatype is the one part with no default. `require_field` is what a write calls, and it fails by naming the builders that declare one rather than inventing a schema from the first batch.

```rust
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::MimeType;

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?;
assert!(options.field().is_none());

let message = options.require_field().unwrap_err().to_string();
assert!(message.contains("with_field"), "{message}");
assert!(message.contains("with_dtype"), "{message}");
```

Content codings are ignored when deriving options: `for_media_type` looks only at the base type, because the coding belongs to the handle. This is the same derivation [`IOMedia::record_options`](holder.md) performs, which is how a record call on a bare handle knows its encoding without a format argument.


## Arrow IPC


`yggdryl::media::ipc` reads and writes Arrow IPC streams over any byte handle.

At handle level, overwrite, append, and keyed merge use the shared
[canonical record-write signatures](holder.md#canonical-record-write-signatures).
The free `ipc::overwrite_arrow_reader` below is the one complete-stream encoder
those intents ultimately publish through.

!!! note "All three"
    Python and JavaScript reach the encoding through [`IOBase`](holder.md)'s record
    methods rather than through the free functions. Python exchanges batches as
    `pyarrow.RecordBatchReader`, and JavaScript as Apache Arrow JS values over
    the copied Arrow IPC boundary described in
    [javascript.md](extensions/javascript.md).

### Arrow batch reads and writes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, venues: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
            ],
        )
    };

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![1, 2], vec![Some("XNAS"), Some("XNYS")])?],
        ),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![3], vec![Some("XLON")])?],
        ),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![2, 4], vec![Some("XPAR"), None])?],
        ),
        &options.clone().with_merge_by_names(["id"]),
    )?;

    let rows = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 4);
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
    batch = lambda ids, venues: pa.record_batch(
        {"id": ids, "venue": venues}, schema=schema
    )

    # The name says Arrow IPC, so no call names an encoding.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by_names = ["id"]
    handle.merge_arrow_batch(batch([2, 4], ["XPAR", None]), options=merging)

    assert handle.read_arrow_field().name == "row"
    assert handle.read_arrow_reader().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, venues) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
    })

    // The name says Arrow IPC, so no call names an encoding.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))
    handle.overwriteArrowTable(rows([1, 2], ['XNAS', 'XNYS']))
    handle.appendArrowTable(rows([3], ['XLON']))
    handle.mergeArrowTable(
      rows([2, 4], ['XPAR', null]),
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowField().name, 'row')
    assert.equal(handle.readArrowReader().intoTable().numRows, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

The methods above are the shared [`IOMedia`](holder.md#canonical-record-write-signatures)
surface. Their names make intent authoritative: append retains stored rows, while keyed
merge updates matching `id` values and inserts misses. `merge_by_names` supplies identity;
it never selects merge implicitly.

IPC itself stays one batch-native seam: `read_field`, `read_batch_reader`, and
`overwrite_arrow_reader` operate over any `IOBase` handle and `IpcOptions`. Runtime row,
table, and record-batch adapters widen into the same reader before encoding. Python crosses
that seam through Arrow C Stream; JavaScript uses the documented copied IPC boundary.

#### Measured batch operations

The read fixture contains 65,536 rows and four columns. The write fixture contains 4,096 rows;
Criterion prepares the stored side for append and keyed merge outside the timer. Keyed merge is
the upsert operation: matching `id` rows are updated and misses are inserted.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 4.33 ms | 15.1M rows/s |
| `overwrite_arrow_reader` | 4,096 | 181 us | 22.6M rows/s |
| `append_arrow_reader` | 4,096 | 615 us | 6.66M rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 5.44 ms | 754k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them on the deployment host with
`io_dimensions/ipc/read_rows` and `io_write_stateful/ipc`; the longer PyArrow comparison remains
in [Against PyArrow](#against-pyarrow).

#### Dimensions and opened sessions

`row_size` counts IPC message metadata while skipping dictionary and record-batch bodies;
`column_size` reads the canonical Struct field. They describe the whole stream, ignoring selection,
partition filters, and read limits. Closed calls read fresh metadata; `open` retains the inferred
IPC media wrapper and caches schema and dimensions until `close`. Writes invalidate the cache.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Holder;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;
    let mut handle = Holder::buffer(Buffer::new().with_media_type(MimeType::ARROW_STREAM.into()));
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &options)?;

    handle.open()?;
    assert_eq!(handle.read_arrow_field(&options)?, field);
    assert_eq!((handle.row_size()?, handle.column_size()?), (2, 1));
    handle.close()?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "dimensions.arrows")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2]}))
    with handle:
        assert (handle.row_size, handle.column_size) == (2, 1)
        assert handle.read_arrow_field().name == "row"
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
    const handle = new IOBase(path.join(root, 'dimensions.arrows'))
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2] }))
    handle.open()
    assert.deepEqual([handle.rowSize, handle.columnSize], [2, 1])
    assert.equal(handle.readArrowField().name, 'row')
    handle.close()
    fs.rmSync(root, { recursive: true, force: true })
    ```

The same 65,536-row fixture measured fresh/opened `row_size` at 2.51 us/6.63 ns and
fresh/opened `column_size` at 6.25 us/7.04 ns. Regenerate with
`cargo bench -p yggdryl --bench io --all-features -- io_dimensions/ipc`.

### Reading and writing are both readers

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;
    let batches = (0..3)
        .map(|start| {
            RecordBatch::try_new(
                arrow_schema.clone(),
                vec![Arc::new(Int64Array::from(vec![start, start + 1]))],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = IpcOptions::new();

    // `batch_reader` turns whatever is already in hand - a Vec, an array, an
    // iterator - into the one shape a write takes.
    ipc::overwrite_arrow_reader(
        &mut handle,
        arrow::batch_reader(arrow_schema, batches),
        &options,
    )?;

    let reader = ipc::read_batch_reader(&handle, None, &options)?;
    // The schema is known before a single batch is decoded.
    assert_eq!(reader.schema().fields().len(), 1);

    let mut rows = 0;
    for batch in reader {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 6);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batches = [
        pa.record_batch({"id": [start, start + 1]}, schema=schema)
        for start in range(0, 6, 2)
    ]

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # The primitive write consumes exactly one RecordBatchReader.
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, batches))

    reader = handle.read_arrow_reader()
    # The schema is known before a single batch is decoded.
    assert reader.schema.names == ["id"]

    rows = sum(batch.num_rows for batch in reader)
    assert rows == 6
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const batches = [0, 2, 4].map(
      (start) =>
        new arrow.Table({
          id: arrow.vectorFromArray([BigInt(start), BigInt(start + 1)], new arrow.Int64()),
        }).batches[0],
    )

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM

    // An Arrow JS Table, one RecordBatch, an array of them, or Arrow IPC bytes:
    // `BatchReader.from` turns whatever is in hand into the shape a write takes.
    handle.overwriteArrowReader(BatchReader.from(batches))

    const reader = handle.readArrowReader()
    // The schema is known before a single batch is decoded.
    assert.deepEqual([...reader.field.dtype].map((child) => child.name), ['id'])

    let rows = 0
    for (const batch of reader) rows += batch.numRows
    assert.equal(rows, 6)
    ```

`ipc::read_batch_reader` returns [`arrow::BatchReader`](arrow.md), a boxed `RecordBatchReader`.
It is an iterator, so batches arrive one at a time and only the current one is alive; the
stream's Arrow schema is available from the reader itself, ahead of the first batch.

Batches come back exactly as they were written. `ipc::read_batch_reader` does not cast them to
a declared schema, so what goes in is what comes out, block boundaries included.

The write side is the same type facing the other way: `ipc::overwrite_arrow_reader` consumes a
`BatchReader` and encodes each batch as it pulls it, so a reader that computes its batches
lazily is never materialized.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());

    // Nothing is materialized: each batch is built as the writer asks for it.
    let produced = (0..4).map({
        let arrow_schema = arrow_schema.clone();
        move |start| {
            RecordBatch::try_new(
                arrow_schema.clone(),
                vec![Arc::new(Int64Array::from(vec![start]))],
            )
            .expect("batch")
        }
    });
    ipc::overwrite_arrow_reader(
        &mut handle,
        arrow::batch_reader(arrow_schema, produced),
        &IpcOptions::new(),
    )?;

    assert_eq!(
        ipc::read_batch_reader(&handle, None, &IpcOptions::new())?.count(),
        4
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # Nothing is materialized: each batch is built as the writer asks for it.
    produced = (
        pa.record_batch({"id": [start]}, schema=schema) for start in range(4)
    )
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, produced))

    assert sum(1 for _ in handle.read_arrow_reader()) == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM

    // Apache Arrow JS owns the encoding of what a caller already holds, so the
    // four batches cross the boundary once, as one Arrow IPC stream.
    const produced = [0, 1, 2, 3].map(
      (start) =>
        new arrow.Table({ id: arrow.vectorFromArray([BigInt(start)], new arrow.Int64()) })
          .batches[0],
    )
    handle.overwriteArrowReader(BatchReader.from(produced))

    assert.equal([...handle.readArrowReader()].length, 4)
    ```

### Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
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
    let options = IpcOptions::new();
    ipc::overwrite_arrow_reader(&mut handle, arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // One of the three columns, named by a root Field of its own.
    let wanted = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let projected = ipc::read_batch_reader(&handle, Some(&wanted), &options)?;
    assert_eq!(projected.schema().fields().len(), 1);
    let batches = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(batches[0].num_columns(), 1);

    // The stream itself is unchanged: it still carries all three.
    assert_eq!(ipc::read_field(&handle, &options)?.field_len(), 3);
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

    # One of the three columns, declared through the centralized options field.
    options = handle.record_options()
    options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    projected = handle.read_arrow_reader(options=options)
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The stream itself is unchanged: it still carries all three.
    assert len(handle.read_arrow_field().dtype) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, IOBase, MimeType, fields } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const projected = handle.readArrowReader(handle.recordOptions().withField(wanted))
    assert.deepEqual([...projected.field.dtype].map((child) => child.name), ['id'])
    assert.equal(projected.intoTable().numCols, 1)

    // The stream itself is unchanged: it still carries all three.
    assert.equal(handle.readArrowField().dtype.length, 3)
    ```

The `field` argument to `ipc::read_batch_reader` is a column pushdown and nothing else. A
non-null struct root naming a subset of the stored columns becomes the projection the Arrow
IPC decoder takes, so the columns it leaves out are never turned into arrays. Be precise
about what that saves: an IPC record batch is one contiguous message, so its body is still
read off the handle whole - the projection removes the decode and the allocation, not the
bytes. [media.md](media.md), whose column chunks are separately addressable, is where a
projection also removes reading.

A root naming every stored column, or naming one the stream does not carry, reads
everything: a projection can only drop columns, never invent them. The selection keeps the
stored order and the stored types. The handle-level `read_arrow_reader` in
[holder.md](holder.md) is the one that also casts: it declares this schema, gets the projection out
of it, and then reshapes what comes back.

Arrow's own projected `StreamReader` reports the whole stream's schema while yielding
projected batches. The reader returned here reports the projected schema, so what it says
and what it yields agree.

### One stream, one configuration

!!! note "Rust only"
    Python and JavaScript reach the encoding through the handle itself; the
    stateful wrapper is a Rust type.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::media::ipc::Ipc;
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
let mut media = Ipc::new(handle).with_field(schema.clone());
let options = media.record_options()?;

// One options value carries the schema, root name, and coding.
media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
assert_eq!(media.read_arrow_field(&options)?, schema);

// An Ipc is also the bytes it encodes: a stream opens with its continuation marker.
assert_eq!(media.read_range_bytes(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

`Ipc<H>` holds the handle, its default options, and the opened metadata cache.
`record_options` returns those defaults as the `RecordOptions` value every
canonical `IOMedia` call accepts. `handle`, `handle_mut`, and `into_handle`
reach the wrapped handle; `options` and `options_mut` change future defaults.

`Ipc<H>` implements `IOBase` by delegating to the handle it owns, which is why
`read_range_bytes` above works on it directly. That is what lets a stream be copied, compressed,
or handed to another reader without unwrapping it first, and what lets an `Ipc` be held as
[`media::Media::Ipc`](media.md).

### The stream carries its schema

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::media::DEFAULT_ROOT_NAME;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from(vec![7]))],
    )?;

    let mut writer = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = writer.record_options()?;
    writer.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
    let bytes = writer.handle().as_slice().to_vec();

    // A reader that declares nothing recovers the schema from the bytes.
    let reader = Ipc::new(Buffer::from_bytes(bytes.clone()));
    let options = reader.record_options()?;
    assert_eq!(reader.read_arrow_field(&options)?, schema);
    assert_eq!(reader.read_arrow_field(&options)?.name(), DEFAULT_ROOT_NAME);

    // Arrow names columns, not the record; the root name is chosen on this side.
    let named = Ipc::new(Buffer::from_bytes(bytes)).with_name("trade");
    let options = named.record_options()?;
    let named_field = named.read_arrow_field(&options)?;
    assert_eq!(named_field.name(), "trade");
    assert_eq!(named_field.get_field_by_path("id"), schema.get_field_by_path("id"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(pa.record_batch({"id": [7]}, schema=schema))

    # A reader that declares nothing recovers the schema from the bytes.
    assert handle.read_arrow_field().name == "row"

    # Arrow names columns, not the record; the root name is chosen on this side.
    named = handle.record_options()
    named.name = "trade"
    assert handle.read_arrow_field(options=named).name == "trade"
    assert [child.name for child in handle.read_arrow_field().dtype] == ["id"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray([7n], new arrow.Int64()) }),
    )

    // A reader that declares nothing recovers the schema from the bytes.
    assert.equal(handle.readArrowField().name, 'row')

    // Arrow names columns, not the record; the root name is chosen on this side.
    const named = handle.recordOptions().withName('trade')
    assert.equal(handle.readArrowField(named).name, 'trade')
    assert.deepEqual([...handle.readArrowField().dtype].map((child) => child.name), ['id'])
    ```

An IPC stream is self-describing, so a declared schema is never required to read one. When
`IpcOptions::dtype` is set, `read_field` returns the field it builds without touching the handle;
when it is absent, the stream's Arrow schema is converted back to a `Field` and the struct root
is named `name`, which defaults to `media::DEFAULT_ROOT_NAME` - `"row"`. Arrow carries names
for the columns and none for the record, so that one name is the only thing inference
cannot recover.

### Content coding comes from the name

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::{DataType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;

    let mut sizes = Vec::new();
    for name in ["trades.arrows", "trades.arrows.gz", "trades.arrows.zst"] {
        let url = Url::from_str(&format!("file:///{name}"))?;
        let handle = Buffer::new().with_media_type(url.media_type());
        let mut media = Ipc::new(handle).with_field(schema.clone());

        let batch = RecordBatch::try_new(
            arrow_schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2]))],
        )?;
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(arrow_schema.clone(), [batch]),
            &options,
        )?;

        // Identical calls on both sides, whatever the coding is.
        assert_eq!(media.read_arrow_reader(&options)?.count(), 1, "{name}");
        sizes.push(media.handle().as_slice().to_vec());
    }

    // The bytes underneath are framed by the coding the name declared.
    assert_eq!(&sizes[1][..2], &[0x1F, 0x8B]);
    assert_eq!(&sizes[2][..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    assert_ne!(sizes[0], sizes[1]);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    written = []
    for name in ("trades.arrows", "trades.arrows.gz", "trades.arrows.zst"):
        handle = IOBase(root / name)
        handle.overwrite_arrow_batch(pa.record_batch({"id": [1, 2]}, schema=schema))

        # Identical calls on both sides, whatever the coding is.
        assert handle.read_arrow_reader().read_all().num_rows == 2, name
        written.append(handle.read_bytes())

    # The bytes underneath are framed by the coding the name declared.
    assert written[1][:2] == bytes.fromhex("1f8b")
    assert written[2][:4] == bytes.fromhex("28b52ffd")
    assert written[0] != written[1]
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
    const written = []
    for (const name of ['trades.arrows', 'trades.arrows.gz', 'trades.arrows.zst']) {
      const handle = new IOBase(path.join(root, name))
      handle.overwriteArrowTable(
        new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) }),
      )

      // Identical calls on both sides, whatever the coding is.
      assert.equal(handle.readArrowReader().intoTable().numRows, 2, name)
      written.push(handle.readBytes())
    }

    // The bytes underneath are framed by the coding the name declared.
    assert.deepEqual([...written[1].subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual([...written[2].subarray(0, 4)], [0x28, 0xb5, 0x2f, 0xfd])
    assert.notDeepEqual(written[0], written[1])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Compression is never an argument here. The handle reports a media type, `IOBase::codec`
reads the last content coding out of it, and the encoding applies it on write and strips
it on read. A handle named `trades.arrows.gz` round-trips through [gzip](coding.md), one
named `trades.arrows.zst` through [zstd](coding.md), and the calls above do not change.

`IpcOptions::level` is the only compression setting, and it reaches whichever coding the
handle declared. It does nothing when the handle declares none.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::{DataType, Level, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..512).collect::<Vec<i64>>()))],
    )?;

    let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
    let mut media = Ipc::new(handle)
        .with_field(schema.clone())
        .with_level(Level::BEST);
    let options = media.record_options()?;

    media.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
    // Still a gzip member, and smaller than the stream it encodes.
    assert_eq!(&media.handle().as_slice()[..2], &[0x1F, 0x8B]);
    assert!(media.handle().size() < 512 * 8);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows.gz")

    options = handle.record_options()
    options.level = 9
    handle.overwrite_arrow_batch(
        pa.record_batch({"id": list(range(512))}, schema=schema), options=options
    )

    assert handle.read_arrow_reader().read_all().num_rows == 512
    # Still a gzip member, and smaller than the stream it encodes.
    assert handle.read_bytes()[:2] == bytes.fromhex("1f8b")
    assert handle.size < 512 * 8
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
    const handle = new IOBase(path.join(root, 'trades.arrows.gz'))

    const ids = Array.from({ length: 512 }, (_, index) => BigInt(index))
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      handle.recordOptions().withLevel(9),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 512)
    // Still a gzip member, and smaller than the stream it encodes.
    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.ok(handle.size < 512 * 8)

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Options

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions, DEFAULT_ROOT_NAME};
    use yggdryl::media::ipc::IpcOptions;
    use yggdryl::{DataType, Level, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = IpcOptions::new()
        .with_field(schema.clone())
        .with_level(Level::BEST);

    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), DEFAULT_ROOT_NAME);
    assert_eq!(options.dtype(), Some(schema.dtype()));
    assert_eq!(options.level(), Level::BEST);

    // The fields are public, so a setting can also be written directly.
    let mut direct = IpcOptions::new();
    direct.batch_row_size = Some(1024);
    assert_eq!(direct.batch_row_size(), Some(1024));

    // It converts into the enum every encoding's settings share.
    let erased: RecordOptions = options.into();
    assert_eq!(erased.mime_type(), MimeType::ARROW_STREAM);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.arrows")
    options.field = schema
    options.level = 9

    assert options.field is not None
    assert options.name == "row"
    assert options.level == 9

    options.batch_row_size = 1024
    assert options.batch_row_size == 1024

    assert str(options.mime_type) == "application/vnd.apache.arrow.stream"
    # A setting another encoding has is absent rather than invented here.
    assert options.max_row_group_size is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    // The media type names the encoding, so there is no format argument.
    const options = new RecordOptions('trades.arrows')
    options.field = schema
    options.level = 9

    assert.ok(options.field.equals(schema))
    assert.equal(options.name, 'row')
    assert.equal(options.level, 9)

    options.batchRowSize = 1024
    assert.equal(options.batchRowSize, 1024)

    assert.equal(options.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(true).safe, true)
    assert.equal(options.safe, false)
    ```

`IpcOptions` stores the shared record settings as public fields: `name`, `dtype`, `metadata`,
`safe`, `batch_row_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, `level`,
`merge_by_names`, `select_by_names`, and `filter_partitions`. It implements
[`IORecordOptions`](types.md), which defines their accessors, the `field` built from the first
three, and the `with_*` builders once.
IPC adds no format-specific setting: the stream carries its schema and the handle carries
its optional outer coding.

The low-level `ipc::*` functions handle only the encoding seam. The shared
[`IOMedia`](holder.md#canonical-record-write-signatures) path applies casting, re-chunking,
selection, limits, partition filters, commit cadence, and write intent around that seam.
Consequently the same `RecordOptions` value has one meaning even when the caller does not
know which encoding is underneath.

`Ipc::with_options` replaces the whole settings value at once; `with_field` and `with_name`
reach through to the declared root. Every builder on `Ipc` drops the opened metadata cache,
`with_level` included: the cache is what the bytes say, and the options decide how they are read.

### Absence

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // A resource that does not exist yet holds no batches; it is not a parse failure.
    let missing = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = missing.record_options()?;
    let reader = missing.read_arrow_reader(&options)?;
    // The declared schema is what the empty reader reports.
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(reader.count(), 0);

    // Opening an absent stream succeeds and caches explicit zero dimensions.
    let mut empty = Ipc::new(Buffer::new());
    empty.open()?;
    assert!(empty.opened());
    assert_eq!(empty.row_size()?, 0);
    assert_eq!(empty.column_size()?, 0);

    // Writing no batches still writes the schema, so the stream exists and is readable.
    let mut written = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = written.record_options()?;
    written.overwrite_arrow_reader(
        arrow::batch_reader(
            schema.clone().into_arrow_schema()?,
            std::iter::empty::<RecordBatch>(),
        ),
        &options,
    )?;
    assert!(!written.handle().is_empty());
    assert_eq!(written.read_arrow_reader(&options)?.count(), 0);
    assert_eq!(written.read_arrow_field(&options)?, schema);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    # A resource that does not exist yet holds no batches; it is not a parse failure.
    missing = IOBase(root / "missing.arrows")
    assert not missing.exists()
    assert missing.read_arrow_reader().read_all().num_rows == 0

    # Writing no batches still writes the schema, so the stream exists and reads.
    written = IOBase(root / "empty.arrows")
    written.overwrite_arrow_table(pa.Table.from_batches([], schema=schema))
    assert written.size > 0
    assert written.read_arrow_reader().read_all().num_rows == 0
    assert written.read_arrow_field().name == "row"
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

    // A resource that does not exist yet holds no batches; it is not a parse failure.
    const missing = new IOBase(path.join(root, 'missing.arrows'))
    assert.ok(!missing.exists())
    assert.equal(missing.readArrowReader().intoTable().numRows, 0)

    // Writing no batches still writes the schema, so the stream exists and reads.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const written = new IOBase(path.join(root, 'empty.arrows'))
    written.overwriteArrowTable(new arrow.Table(schema))
    assert.ok(written.size > 0)
    assert.equal(written.readArrowReader().intoTable().numRows, 0)
    assert.equal(written.readArrowField().name, 'row')

    fs.rmSync(root, { recursive: true, force: true })
    ```

Reading follows the laziness rule the storage layer sets in [holder.md](holder.md): a location
that holds nothing yields nothing. An empty handle with a declared schema reports that
schema, and an empty handle without one reports an empty Arrow schema, so a caller can
probe a stream without an existence check first.

An empty stream and a stream of zero batches are different things, and the difference is
visible in the bytes: the second one was written, carries its schema, and answers
`schema` from the stream itself.

Anything that is not a stream fails on the spot rather than being guessed at.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};

    let handle = Buffer::from_bytes(b"definitely not an Arrow IPC stream".to_vec());
    assert!(ipc::read_field(&handle, &IpcOptions::new()).is_err());
    assert!(ipc::read_batch_reader(&handle, None, &IpcOptions::new()).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import IOBase

    handle = IOBase.from_bytes(b"definitely not an Arrow IPC stream")
    handle.media_type = "application/vnd.apache.arrow.stream"

    with pytest.raises(ValueError):
        handle.read_arrow_field()
    with pytest.raises(ValueError):
        handle.read_arrow_reader()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('definitely not an Arrow IPC stream'))
    handle.mediaType = MimeType.ARROW_STREAM

    assert.throws(() => handle.readArrowField(), /Arrow/)
    assert.throws(() => handle.readArrowReader(), /Arrow/)
    ```

The other record encoding in this build is [media.md](media.md), behind the non-default
`parquet` feature. It has the same `read_field`, `read_batch_reader`, and
`overwrite_arrow_reader` shape over the same shared settings, and adds the three a file
format needs that a stream does not. [`media::Media`](media.md) holds either one
without naming which.

### Against PyArrow

`python/benchmarks/media.py` carries a PyArrow IPC write baseline over the same batches and
the same sink. `records_io.py --min-time 0.1 --repeat 3`, one containerized x86_64 Linux run,
65,536 rows, 4 columns, 8 batches:

```text
ipc write reader                 1.133 ms   57.9M rows/s
PyArrow IPC write baseline       1.607 ms   40.8M rows/s
```

The IPC write outruns PyArrow's own writer on the same rows; the [parquet](media.md) page
carries that encoding's rows from the same run.

## Apache Parquet


Read and write Apache Parquet files, and their footer statistics, over any handle.

At handle level, overwrite, append, and keyed merge use the shared
[canonical record-write signatures](holder.md#canonical-record-write-signatures).
The free `parquet::overwrite_arrow_reader` function below remains the one
complete-file encoder those intents publish through; the stateful wrapper uses
`overwrite_arrow_reader` like every other media handle.

!!! note "All three"
    Python and JavaScript reach the encoding through [`IOBase`](holder.md)'s record
    methods, which cover reading, writing, column pushdown, row groups, and
    footer statistics. The stateful `Parquet` wrapper and encoding free
    functions stay Rust-only; the inferred handle surface is shared by all
    three languages.

### Arrow batch reads and writes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, symbols: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
    };

    // The name decides Parquet; the methods name the write intent.
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])?],
        ),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![3], vec![Some("GOOG")])?],
        ),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![2, 4], vec![Some("NVDA"), None])?],
        ),
        &options.clone().with_merge_by_names(["id"]),
    )?;

    let rows = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pyarrow.parquet as pq

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = lambda ids, symbols: pa.record_batch(
        {"id": ids, "symbol": symbols}, schema=schema
    )

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
    with IOBase(path) as handle:
        handle.overwrite_arrow_batch(batch([1, 2], ["AAPL", "MSFT"]))
        handle.append_arrow_batch(batch([3], ["GOOG"]))

        merging = handle.record_options()
        merging.merge_by_names = ["id"]
        handle.merge_arrow_batch(
            batch([2, 4], ["NVDA", None]), options=merging
        )

        assert handle.read_arrow_reader().read_all().num_rows == 4

    assert pq.read_table(path).num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, symbols) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(rows([1, 2], ['AAPL', 'MSFT']))
    handle.appendArrowTable(rows([3], ['GOOG']))
    handle.mergeArrowTable(
      rows([2, 4], ['NVDA', null]),
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 4)
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('PAR1')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Parquet answers the same explicit overwrite, append, and keyed merge intents
as every other encoding. Their [canonical signatures and streaming
rules](holder.md#canonical-record-write-signatures) apply without a format
argument: the handle's media type selects Parquet, while `merge_by_names`
supplies row-identity keys only.

#### Measured batch operations

The read fixture contains 65,536 rows and four columns. The write fixture contains 4,096 rows;
Criterion prepares the stored side for append and keyed merge outside the timer. Keyed merge is
the upsert operation: matching `id` rows are updated and misses are inserted.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 18.6 ms | 3.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 3.91 ms | 1.05M rows/s |
| `append_arrow_reader` | 4,096 | 8.06 ms | 508k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 9.03 ms | 453k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them on the deployment host with
`io_dimensions/parquet/read_rows` and `io_write_stateful/parquet`; the longer PyArrow comparison
remains in [Against PyArrow](#against-pyarrow).

#### Dimensions and opened sessions

`row_size` and `column_size` range-read only the eight-byte tail and footer; no row group or column
page is decoded. They describe the whole file, ignoring selection, filters, and limits. Closed calls
read a fresh footer; `open` retains the inferred Parquet wrapper and footer until `close`, and writes
invalidate it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Holder;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;
    let mut handle = Holder::buffer(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &options)?;

    handle.open()?;
    assert_eq!(handle.read_arrow_field(&options)?, field);
    assert_eq!((handle.row_size()?, handle.column_size()?), (2, 1));
    handle.close()?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "dimensions.parquet")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2]}))
    with handle:
        assert (handle.row_size, handle.column_size) == (2, 1)
        assert handle.read_arrow_field().name == "row"
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
    const handle = new IOBase(path.join(root, 'dimensions.parquet'))
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2] }))
    handle.open()
    assert.deepEqual([handle.rowSize, handle.columnSize], [2, 1])
    assert.equal(handle.readArrowField().name, 'row')
    handle.close()
    fs.rmSync(root, { recursive: true, force: true })
    ```

The same 65,536-row fixture measured fresh/opened `row_size` at 8.95 us/9.12 ns,
fresh/opened `column_size` at 18.1 us/6.86 ns, and fresh/opened `read_arrow_field` at
297 us/119 us. Regenerate with
`cargo bench -p yggdryl --bench io --all-features -- io_dimensions/parquet`.

Everything on this page is behind the non-default `parquet` feature. The codec is version-locked to
the pinned Arrow release and pulls in a thrift and compression stack a schema-only consumer never
touches, so it is opted into rather than carried. Without it the module does not exist, and
[`RecordOptions::for_mime_type`](types.md) reports `application/vnd.apache.parquet` as an encoding
this build does not implement instead of guessing one.

`Parquet<H>` binds one file to its handle, default settings, and opened footer
cache. `record_options` returns those defaults for the canonical `IOMedia`
calls: `overwrite_arrow_reader` consumes an [`arrow::BatchReader`](arrow.md),
while `read_arrow_reader` returns one and `read_arrow_field` returns its
canonical non-null struct root [`Field`](types.md). `arrow::batch_reader` turns
batches already in hand into the same streaming shape; every pulled batch is
checked against the reader's field and errors name its index.

The specialized `read_arrow_schema` returns the Arrow schema and
`read_statistics` returns the footer. The encoding seams also exist as free functions - `parquet::read_arrow_schema`,
`read_field`, `read_batch_reader`, `overwrite_arrow_reader`, `read_statistics` - taking the handle
explicitly, and a `&ParquetOptions` where the settings matter.

Both bindings call the same operations on the handle itself - Python's
`read_arrow_reader` / `overwrite_arrow_reader` and JavaScript's
`readArrowReader` / `overwriteArrowReader` - so a file this large never becomes a table on either side unless a caller asks
for one. Python exchanges `pyarrow.RecordBatchReader` values across the Arrow C Stream interface;
JavaScript exchanges Apache Arrow JS values over the copied Arrow IPC boundary, one batch per stream.
`record_options()`/`recordOptions()` is the Parquet settings value, carrying `compression`,
`max_row_group_size`, and `key_value_metadata` next to the shared settings.

### Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(Float64Array::from(vec![1.5, 2.5])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // Two of the four columns, named by a root Field of its own.
    let wanted = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?
    .required_field("row");

    let projected = media.read_arrow_reader(&options.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 2);
    let read = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(read[0].num_columns(), 2);

    // The file is unchanged: it still stores all four.
    assert_eq!(media.read_arrow_schema()?.fields().len(), 4);
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
        pa.field("price", pa.float64(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    rows = 4_096
    batch = pa.record_batch(
        {
            "id": list(range(rows)),
            "symbol": ["AAPL"] * rows,
            "price": [1.5] * rows,
            "venue": ["XNAS"] * rows,
        },
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # Two of the four columns, declared through the centralized options field.
    options = handle.record_options()
    options.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ])
    projected = handle.read_arrow_reader(options=options).read_all()
    assert projected.column_names == ["id", "price"]

    # Less is read, and the bytes say so rather than the clock.
    whole = handle.read_arrow_reader().read_all()
    assert projected.nbytes * 2 <= whole.nbytes

    # The file is unchanged: it still stores all four.
    assert len(handle.read_arrow_field().dtype) == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        price: arrow.vectorFromArray([1.5, 2.5], new arrow.Float64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // Two of the four columns, declared as this read's schema.
    const wanted = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('price: float64')],
      { nullable: false },
    )

    const options = handle.recordOptions().withField(wanted)
    const projected = handle.readArrowReader(options).intoTable()
    assert.equal(projected.numCols, 2)
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id', 'price'])

    // The file is unchanged: it still stores all four.
    assert.equal(handle.readArrowField().dtype.length, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

The `field` argument to `parquet::read_batch_reader` is a column pushdown and nothing else. A
non-null struct root naming a subset of the stored columns becomes a Parquet `ProjectionMask` over
the file's root columns, which is the format's own way of not reading a column: the chunks it leaves out are never
located, decompressed, or decoded. This is the encoding where a projection genuinely moves less data,
because a Parquet column chunk is separately addressable while an [Arrow IPC](media.md) record batch is
one contiguous message.

The mask is built from roots rather than leaves, so a nested column comes along with its whole
subtree. A root naming every stored column, or naming one the file does not store, reads everything:
a mask can only drop columns, never invent them. The selection keeps the stored order and the stored
types, so a caller wanting a different shape casts afterwards.

### Options

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Level, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..1_000).collect::<Vec<i64>>()))],
    )?;

    // Parquet's own settings and the shared ones are flat fields on one struct.
    let options = ParquetOptions::new()
        .with_max_row_group_size(4_096)
        .with_key_value("iceberg.schema-id", "7")
        .with_batch_row_size(256)
        .with_name("trade");

    assert_eq!(options.max_row_group_size, 4_096);
    assert_eq!(
        options.key_value_metadata,
        [("iceberg.schema-id".to_owned(), "7".to_owned())]
    );
    assert_eq!(options.batch_row_size(), Some(256));
    assert_eq!(options.name(), "trade");
    assert!(!options.safe());

    // Unused here: Parquet compresses pages itself.
    assert_eq!(options.level, Level::DEFAULT);

    let mut media =
        Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_options(options);
    let call_options = media.record_options()?;
    media.overwrite_arrow_reader(
        arrow::batch_reader(arrow_schema, [batch]),
        &call_options,
    )?;

    // batch_row_size bounds the reader, so no batch holds all 1,000 rows.
    let rows: Vec<usize> = media
        .read_arrow_reader(&call_options)?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .collect();
    assert_eq!(rows.iter().sum::<usize>(), 1_000);
    assert!(rows.iter().all(|count| *count <= 256), "{rows:?}");

    // The root name names the Field recovered from the footer.
    assert_eq!(media.read_arrow_field(&call_options)?.name(), "trade");

    // A declared schema is returned as-is, so an empty handle answers without a footer.
    let declared =
        Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_field(field.clone());
    let declared_options = declared.record_options()?;
    assert_eq!(declared.read_arrow_field(&declared_options)?, field);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batch = pa.record_batch({"id": list(range(1_000))}, schema=schema)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")

    # Parquet's own settings and the shared ones are properties of one value.
    options = handle.record_options()
    options.max_row_group_size = 4_096
    options.key_value_metadata = {"iceberg.schema-id": "7"}
    options.batch_row_size = 256
    options.name = "trade"

    assert options.max_row_group_size == 4_096
    assert options.key_value_metadata == {"iceberg.schema-id": "7"}
    assert options.batch_row_size == 256
    assert options.name == "trade"
    assert not options.safe

    handle.overwrite_arrow_batch(batch, options=options)

    # batch_row_size bounds the reader, so no batch holds all 1,000 rows.
    counts = [part.num_rows for part in handle.read_arrow_reader(options=options)]
    assert sum(counts) == 1_000
    assert all(count <= 256 for count in counts), counts

    # The root name names the Field recovered from the footer.
    assert handle.read_arrow_field(options=options).name == "trade"
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
    const handle = new IOBase(path.join(root, 'trades.parquet'))

    // Parquet's own settings and the shared ones are properties of one value.
    const options = handle
      .recordOptions()
      .withMaxRowGroupSize(4_096)
      .withKeyValue('iceberg.schema-id', '7')
      .withBatchRowSize(256)
      .withName('trade')

    assert.equal(options.maxRowGroupSize, 4_096)
    assert.deepEqual(options.keyValueMetadata, [{ key: 'iceberg.schema-id', value: '7' }])
    assert.equal(options.batchRowSize, 256)
    assert.equal(options.name, 'trade')
    assert.equal(options.safe, false)

    const ids = Array.from({ length: 1_000 }, (_, index) => BigInt(index))
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      options,
    )

    // batchRowSize bounds the reader, so no batch holds all 1,000 rows.
    const counts = [...handle.readArrowReader(options)].map((batch) => batch.numRows)
    assert.equal(counts.reduce((total, count) => total + count, 0), 1_000)
    assert.ok(counts.every((count) => count <= 256), counts.join())

    // The root name names the Field recovered from the footer.
    assert.equal(handle.readArrowField(options).name, 'trade')

    fs.rmSync(root, { recursive: true, force: true })
    ```

`ParquetOptions` adds three settings of its own: `compression`, applied to pages inside the file;
`max_row_group_size`, the row bound that decides how many row groups the file gets; and
`key_value_metadata`, entries written into the footer next to the ones the writer adds itself. The
rest are the flat shared fields every record encoding stores under the same names, reached through
[`IORecordOptions`](types.md): `name`, `dtype`, `metadata`, `safe`, `batch_row_size`, `max_row_size`,
`max_byte_size`, `commit_row_size`, `level`, `merge_by_names`, `select_by_names`, and
`filter_partitions`.

`level` is the one that does nothing here. It is the compression level of a declared content coding,
and Parquet has no outer coding to apply it to; `compression` is the setting that decides how the
file compresses.

`Parquet::with_options` replaces the whole set, while `with_field` and `with_name` reach
through to the declared root. A declared `dtype` short-circuits `read_arrow_field` - the field it
builds is returned without reading the file - and `name` roots a declared field and a field
recovered from the footer alike.

### Compression

!!! note "All three"
    Page compression is a `parquet` crate value, so the bindings name it as the
    text the format's own parser accepts - `zstd(3)`, `snappy`, `uncompressed`.
    Whichever runtime wrote a file, every other one reads it, because the footer
    records the codec.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use parquet::basic::Compression;
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let ids: Vec<i64> = (0..4_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )?;

    let mut sizes = Vec::new();
    for compression in [
        Compression::UNCOMPRESSED,
        Compression::SNAPPY,
        Compression::ZSTD(Default::default()),
    ] {
        // One batch per read, so the comparison is not split by the default bound.
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(
                ParquetOptions::new()
                    .with_compression(compression)
                    .with_batch_row_size(batch.num_rows()),
            );
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
            &options,
        )?;

        // Nothing on the read side names the compression: the footer records it.
        let read = media
            .read_arrow_reader(&options)?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(read, [batch.clone()], "{compression:?}");
        sizes.push(media.handle().size());
    }

    assert!(sizes[0] > sizes[1] && sizes[0] > sizes[2], "{sizes:?}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    rows = 4_000
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    table = pa.table(
        {"id": list(range(rows)), "symbol": ["AAPL"] * rows}, schema=schema
    )

    sizes = []
    for compression in ("uncompressed", "snappy", "zstd(1)"):
        handle = IOBase(root / f"trades-{compression}.parquet")
        # One batch per read, so the comparison is not split by the default bound.
        options = handle.record_options()
        options.compression = compression
        options.batch_row_size = rows
        handle.overwrite_arrow_table(table, options=options)

        # Nothing on the read side names the compression: the footer records it.
        read = handle.read_arrow_reader(options=options).read_all()
        assert read.num_rows == rows, compression
        sizes.append(handle.size)

    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes
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
    const ids = Array.from({ length: 4_000 }, (_, index) => BigInt(index))
    const table = new arrow.Table({
      id: arrow.vectorFromArray(ids, new arrow.Int64()),
      symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
    })

    const sizes = []
    for (const compression of ['uncompressed', 'snappy', 'zstd(1)']) {
      const handle = new IOBase(path.join(root, `trades-${compression}.parquet`))
      // One batch per read, so the comparison is not split by the default bound.
      const options = handle
        .recordOptions()
        .withCompression(compression)
        .withBatchRowSize(table.numRows)
      handle.overwriteArrowTable(table, options)

      // Nothing on the read side names the compression: the footer records it.
      const read = handle.readArrowReader(options).intoTable()
      assert.equal(read.numRows, 4_000, compression)
      sizes.push(handle.size)
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    fs.rmSync(root, { recursive: true, force: true })
    ```

Compression is a write setting and never a read one. The codec each column chunk was written with is
recorded in the file's own metadata, so a reader recovers it from the footer and the same
`read_arrow_reader` call decodes any of these files. The default is Zstandard at its default level, with
1,048,576-row groups.

### Coded handles are rejected

=== "Rust"

    ```rust
    use arrow_array::RecordBatch;
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // The name declares gzip over the Parquet file.
    let url = Url::from_str("file:///trades.parquet.gz")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(url.media_type()));

    let empty = arrow::batch_reader(field.into_arrow_schema()?, std::iter::empty::<RecordBatch>());
    let options = media.record_options()?;
    let message = media
        .overwrite_arrow_reader(empty, &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("parquet compresses"), "{message}");
    assert!(message.contains("ParquetOptions::compression"), "{message}");

    // Nothing was published.
    assert!(media.handle().is_empty());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The name declares gzip over the Parquet file.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet.gz")

    with pytest.raises(ValueError, match="parquet compresses"):
        handle.overwrite_arrow_batch(pa.record_batch({"id": [1]}, schema=schema))

    # Nothing was published.
    assert handle.size == 0
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

    // The name declares gzip over the Parquet file.
    const handle = new IOBase(path.join(root, 'trades.parquet.gz'))

    assert.throws(
      () =>
        handle.overwriteArrowTable(
          new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }),
        ),
      /parquet compresses/,
    )

    // Nothing was published.
    assert.equal(handle.size, 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

Every other encoding treats the content coding as the handle's business: name a file
`trades.arrows.gz` and [media.md](media.md) writes an Arrow stream through gzip without being told. This
module is the exception. Parquet is a footer-first container that compresses its own pages, and a
coding wrapped around the whole file moves the footer out of reach - the result is bytes no Parquet
reader can open. So a handle whose media type declares any coding other than identity is rejected on
both reads and writes, and the error names `ParquetOptions::compression` as the setting that was
meant instead. The rejection happens before anything is encoded, so a refused write leaves the
handle untouched.

### Field identifiers

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id").with_parquet_field_id(1),
        DataType::Utf8.nullable_field("symbol").with_parquet_field_id(2),
    ])?
    .required_field("row");

    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec![Some("AAPL")])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // The ids went into the file, so the Arrow schema carries them back.
    let schema = media.read_arrow_schema()?;
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );

    // And the recovered Field answers by id rather than by position.
    let recovered = media.read_arrow_field(&options)?;
    assert_eq!(recovered.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(recovered.fields()[1].parquet_field_id()?, Some(2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False, metadata={"PARQUET:field_id": "1"}),
        pa.field("symbol", pa.string(), metadata={"PARQUET:field_id": "2"}),
    ])
    batch = pa.record_batch({"id": [1], "symbol": ["AAPL"]}, schema=schema)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # The ids went into the file, so the recovered Field answers by id rather
    # than by position.
    recovered = handle.read_arrow_field()
    assert [child.parquet_field_id for child in recovered.dtype] == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    // Arrow JS carries the identifiers the same way Arrow does anywhere else:
    // as field metadata under the exact `PARQUET:field_id` key.
    const rows = new arrow.Table({
      id: arrow.vectorFromArray([1n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
    })
    const schema = new arrow.Schema(
      rows.schema.fields.map(
        (child, index) =>
          new arrow.Field(
            child.name,
            child.type,
            child.nullable,
            new Map([['PARQUET:field_id', String(index + 1)]]),
          ),
      ),
    )

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema, rows.batches[0].data))

    // The ids went into the file, so the recovered Field answers by id rather
    // than by position.
    const recovered = handle.readArrowField()
    assert.deepEqual([...recovered.dtype].map((child) => child.parquetFieldId), [1, 2])
    assert.equal(recovered.dtype.getFieldAt(0).get('PARQUET:field_id'), '1')

    fs.rmSync(root, { recursive: true, force: true })
    ```

[`Field::with_parquet_field_id`](types.md) stores an identifier under the `PARQUET:field_id` metadata key, which is
exactly the key the Parquet writer reads when it assigns ids in the file's own schema. Projecting the
root to Arrow before building the write's reader is what carries them across, and reading reverses
it. That round trip is
the whole reason a downstream [Iceberg](media.md) or Delta layer can resolve a column after it has
been renamed or moved: the id is in the data file, not just in the catalog.

### Footer statistics

The inferred handle methods validate that the leaf is Parquet, range-read its footer, and decode no
rows. Rust receives the typed `FileStatistics`; Python and JavaScript receive the same shape through
the shared `Scalar` conversion, so integers, byte bounds, nulls, lists, and records become native
language values without binding-side DTO logic.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType, Scalar};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?.required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT"), None])),
        ],
    )?;
    let mut media = Parquet::new(
        Buffer::new().with_media_type(MimeType::PARQUET.into()),
    ).with_options(
        ParquetOptions::new()
            .with_max_row_group_size(2)
            .with_key_value("writer", "rust"),
    );
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = IOMedia::read_parquet_statistics(&media)?;
    assert_eq!(statistics.num_rows, 4);
    assert_eq!(statistics.row_groups.len(), 2);
    assert_eq!(statistics.null_count("symbol"), Some(2));
    let native = Scalar::from(statistics);
    assert_eq!(native.get_key_str("num_rows").and_then(Scalar::as_i64), Some(4));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    scratch = tempfile.TemporaryDirectory()
    handle = IOBase(pathlib.Path(scratch.name) / "trades.parquet")
    options = handle.record_options()
    options.max_row_group_size = 2
    options.key_value_metadata = {"writer": "python"}
    handle.overwrite_arrow_table(pa.table({"id": [1, 2, 3, 4]}), options=options)

    statistics = handle.read_parquet_statistics()
    assert statistics["num_rows"] == 4
    assert len(statistics["row_groups"]) == 2
    assert next(
        entry for entry in statistics["key_value_metadata"] if entry["key"] == "writer"
    ) == {
        "key": "writer",
        "value": "python",
    }
    assert isinstance(statistics["row_groups"][0]["columns"][0]["min_bytes"], bytes)
    scratch.cleanup()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-parquet-stats-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    const options = handle
      .recordOptions()
      .withMaxRowGroupSize(2)
      .withKeyValue('writer', 'javascript')
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2, 3, 4] }), options)

    const statistics = handle.readParquetStatistics()
    assert.equal(statistics.num_rows, 4)
    assert.equal(statistics.row_groups.length, 2)
    assert.deepEqual(statistics.key_value_metadata.find(({ key }) => key === 'writer'), {
      key: 'writer',
      value: 'javascript',
    })
    assert.ok(Buffer.isBuffer(statistics.row_groups[0].columns[0].min_bytes))
    fs.rmSync(root, { recursive: true, force: true })
    ```

`FileStatistics` carries the whole-file row count, writer, ordered footer key/value entries, and
row groups in file order. A row group carries counts, sizes, an optional split offset, and one column
entry per leaf path (`address.zip` for a nested leaf). Key/value metadata stays an entry list because
Parquet permits duplicate keys.

Bounds and null counts are optional because they are optional in the format: a writer records them
per column chunk, and `min_bytes` and `max_bytes` are the encoded values as Parquet stored them, not
decoded scalars. `null_count` sums one column's counts across row groups and returns `None` when no
row group recorded any, which is what distinguishes "no nulls" from "nobody counted".

Rust's `null_count` and `split_offsets` aggregate the same footer data Iceberg manifests use to skip
files. In every language `min_bytes` and `max_bytes` remain Parquet's encoded bounds, not guessed
scalars; missing bounds and counts remain null rather than becoming zero.

A local release boundary spot-check measured footer-to-native-record conversion at 504 us in
Python for 65,536 rows and 728 us in JavaScript for 10,000 rows. The fixtures differ, so these are
per-runtime regression anchors, not a language comparison. Regenerate with the
`parquet read statistics` filter in `python/benchmarks/media.py` and
`records/read_parquet_statistics` in `npm run --prefix node bench:records`.

### Geospatial and variant columns

Footer geospatial data crosses with the rest of `read_parquet_statistics`. A fresh projected scan is
`read_parquet_geospatial_statistics(column)` in Python and
`readParquetGeospatialStatistics(column)` in JavaScript; both return native records with
`bounding_box` and `geometry_types` through the same shared `Scalar` shape.

A column whose schema declares [geometry or geography](types.md) writes Parquet's own `GEOMETRY`
or `GEOGRAPHY` logical type over `BYTE_ARRAY` WKB, from the schema's own declaration: the CRS and,
for a geography, the edge algorithm ride along, and the defaults - `OGC:CRS84`, `spherical` - fold
to the format's absent spellings, so a bare declaration writes the bare logical type. A variant
field writes its metadata/value storage struct with the `VARIANT` logical type attached, at the
schema level only: a variant *value* cannot cross an Arrow array boundary yet - the variant binary
encoding lands with the Iceberg v3 layer - so variant columns stay schema-level until it does.

A geospatial column's sort order is undefined, so the writer never records min/max value bounds for
it - a bound would be a lie - while sibling columns keep theirs, and a min/max a foreign writer
recorded anyway is ignored on read rather than surfaced. What a geometry records instead is the
format's own geospatial statistics: the WKB bounding box and the sorted ISO geometry type codes
present, in the footer, readable from `ColumnStatistics::geospatial` and recomputable by scanning
the stored WKB through Rust's `read_geospatial_statistics` (the inferred handle methods above in the
bindings), which also answers for files whose writer recorded none. A geography records no box at
all: its bounds are edge-algorithm-aware, and a planar fold of the vertices would under-cover them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::geometry(None)?.nullable_field("shape"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(BinaryArray::from_opt_vec(vec![
                Some(&wkb_point(1.0, 2.0)[..]),
                None,
                Some(&wkb_point(-3.0, 7.0)[..]),
            ])),
        ],
    )?;
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = media.read_statistics()?;
    let columns = &statistics.row_groups[0].columns;
    let id = columns.iter().find(|column| column.path == "id").unwrap();
    let shape = columns.iter().find(|column| column.path == "shape").unwrap();
    assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
    assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
    assert_eq!(shape.null_count, Some(1));

    let geospatial = shape.geospatial.as_ref().unwrap();
    let bounds = geospatial.bounding_box.unwrap();
    assert_eq!(
        (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
        (-3.0, 1.0, 2.0, 7.0)
    );
    assert_eq!(geospatial.geometry_types, vec![1]);
    assert_eq!(
        IOMedia::read_parquet_geospatial_statistics(&media, "shape")?,
        *geospatial,
    );
    ```

=== "Python"

    ```python
    import pathlib
    import struct
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    def point(x: float, y: float) -> bytes:
        return b"\x01\x01\x00\x00\x00" + struct.pack("<dd", x, y)

    schema = pa.schema([
        pa.field(
            "shape",
            pa.binary(),
            metadata={
                b"ARROW:extension:name": b"geoarrow.wkb",
                b"ARROW:extension:metadata": b'{"crs":"OGC:CRS84"}',
            },
        )
    ])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "shapes.parquet")
    handle.overwrite_arrow_table(
        pa.table({"shape": [point(1, 2), None, point(-3, 7)]}, schema=schema)
    )

    scanned = handle.read_parquet_geospatial_statistics("shape")
    footer = handle.read_parquet_statistics()["row_groups"][0]["columns"][0][
        "geospatial"
    ]
    assert scanned == footer
    assert scanned["geometry_types"] == [1]
    assert scanned["bounding_box"]["xmin"] == -3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const point = (x, y) => {
      const bytes = Buffer.allocUnsafe(21)
      bytes.writeUInt8(1, 0)
      bytes.writeUInt32LE(1, 1)
      bytes.writeDoubleLE(x, 5)
      bytes.writeDoubleLE(y, 13)
      return bytes
    }

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-geo-'))
    const handle = new IOBase(path.join(root, 'shapes.parquet'))
    handle.overwriteArrowTable(new arrow.Table({
      shape: arrow.vectorFromArray(
        [point(1, 2), null, point(-3, 7)],
        new arrow.Binary(),
      ),
    }))

    const scanned = handle.readParquetGeospatialStatistics('shape')
    assert.deepEqual(scanned.geometry_types, [1])
    assert.equal(scanned.bounding_box.xmin, -3)
    assert.equal(scanned.bounding_box.ymax, 7)
    fs.rmSync(root, { recursive: true, force: true })
    ```

One read-side limit is named rather than hidden: a *foreign* file whose columns carry
`GEOMETRY`/`GEOGRAPHY`/`VARIANT` surfaces plain `Binary`/`Struct` Arrow types without extension
metadata, because the pinned parquet crate only maps those logical types to Arrow extensions behind
crate features that pull new dependencies; files written here round-trip their extension identity
through the embedded Arrow schema. And GeoArrow's own documents say the specification is not
finalized, so the `geoarrow.wkb` spelling the writer reads is revisitable if it changes.

The projected WKB-to-native-record boundary measured 2.64 ms in Python for 8,192 rows and
3.26 ms in JavaScript for 10,000 rows in the same local release spot-check. Regenerate with the
`parquet read geospatial stats` filter in `python/benchmarks/media.py` and
`records/read_parquet_geospatial_statistics` in `npm run --prefix node bench:records`.

### The handle underneath

!!! note "Rust encoding seam"
    The named `Parquet<H>` wrapper and free functions are Rust's typed encoding
    seam. Python and JavaScript infer and retain the same wrapper inside an
    opened `IOBase`; callers keep one generic handle surface.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::media::parquet::{self, Parquet, ParquetOptions};
use yggdryl::{DataType, MimeType};

let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = field.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

// The free functions take a handle and options; nothing is bound.
let options = ParquetOptions::new();
let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
parquet::overwrite_arrow_reader(
    &mut handle,
    arrow::batch_reader(arrow_schema, [batch]),
    &options,
)?;

assert_eq!(parquet::read_arrow_schema(&handle)?.fields().len(), 1);
assert_eq!(parquet::read_field(&handle, &options)?.name(), "row");
assert_eq!(parquet::read_batch_reader(&handle, None, &options)?.count(), 1);
assert_eq!(parquet::read_statistics(&handle)?.num_rows, 2);

// A Parquet is also the bytes it encodes, magic bytes included.
let mut media = Parquet::new(handle);
assert_eq!(media.read_range_bytes(0, 4)?, *b"PAR1");

// open caches the footer, close releases it.
assert!(!media.opened());
media.open()?;
assert!(media.opened());
assert_eq!(media.read_statistics()?.num_rows, 2);
assert_eq!(media.row_size()?, 2);
assert_eq!(media.column_size()?, 1);
media.close()?;
assert!(!media.opened());
```

`Parquet<H>` is itself an [`IOBase`](holder.md) over the handle it owns, so the encoded file is reachable
without unwrapping anything - to copy it, upload it, or hand it to another reader. It forwards every
byte method to the handle and keeps `open`, `opened`, and `close` for itself: `open` parses the
footer once and caches it, so repeated statistics reads do not re-parse it, `close` drops it, and any
write invalidates it.

When the encoding is decided at run time rather than written into the type,
[`Media::parquet`](types.md) names this variant and [`IOMedia::record_options`](holder.md) derives
`ParquetOptions` from a handle's own media type, so a file named `trades.parquet` is read as Parquet
without a format argument.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // Nothing has been written, so there is nothing to read.
    let empty = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
        .with_field(field.clone());
    let options = empty.record_options()?;
    let reader = empty.read_arrow_reader(&options)?;
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(reader.count(), 0);

    // An empty write still publishes a readable file with the schema in its footer.
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(
        arrow::batch_reader(
            field.into_arrow_schema()?,
            std::iter::empty::<RecordBatch>(),
        ),
        &options,
    )?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 0);
    assert_eq!(media.read_arrow_schema()?.fields().len(), 1);
    assert_eq!(media.read_statistics()?.num_rows, 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    # Nothing has been written, so there is nothing to read.
    empty = IOBase(root / "absent.parquet")
    assert empty.read_arrow_reader().read_all().num_rows == 0

    # An empty write still publishes a readable file with the schema in its footer.
    handle = IOBase(root / "written.parquet")
    handle.overwrite_arrow_table(pa.Table.from_batches([], schema=schema))
    assert handle.size > 0
    assert handle.read_arrow_reader().read_all().num_rows == 0
    assert len(handle.read_arrow_field().dtype) == 1
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

    // Nothing has been written, so there is nothing to read.
    const empty = new IOBase(path.join(root, 'absent.parquet'))
    assert.equal(empty.readArrowReader().intoTable().numRows, 0)

    // An empty write still publishes a readable file with the schema in its footer.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const handle = new IOBase(path.join(root, 'written.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema))
    assert.ok(handle.size > 0)
    assert.equal(handle.readArrowReader().intoTable().numRows, 0)
    assert.equal(handle.readArrowField().dtype.length, 1)

    fs.rmSync(root, { recursive: true, force: true })
    ```

An absent file holds no batches rather than failing on a missing footer, which is the laziness
contract every handle follows: constructing touches nothing, reading something absent yields nothing,
writing creates. With no file to read a schema from, the declared one is what the empty reader
reports. Writing no batches is a different thing entirely - it publishes a real file, so the schema
and the statistics are there with no rows behind them.

### Against PyArrow

`python/benchmarks/media.py` carries PyArrow Parquet baselines over the same rows.
`records_io.py --min-time 0.1 --repeat 3`, one containerized x86_64 Linux run, 65,536 rows,
4 columns, 8 batches:

```text
parquet write reader             6.932 ms    9.5M rows/s
PyArrow parquet write baseline   6.495 ms   10.1M rows/s
parquet read whole               2.620 ms   25.0M rows/s
PyArrow parquet read baseline    2.195 ms   29.9M rows/s
```

Both directions sit within ~15% of PyArrow's own writer and reader - the encoding dominates and
both sides drive the same `parquet` machinery. The [ipc](media.md) page carries that encoding's
rows from the same run.

## Apache Avro


Read and write Avro as streamed Arrow batches first, then use its more flexible
schema, object-container, and single-value operations over the shared
[`Scalar`](types.md). Every path works over any [`IOBase`](holder.md) handle, with
no Avro crate underneath.

!!! note "Two surfaces"
    The handle-level Arrow record surface below is available in Rust, Python,
    and JavaScript. Both bindings also expose the native `Schema`, whole
    object-container, single-object, and lazy compressed-block operations
    through their natural values. The explicit compiled `Resolution` type is
    Rust-only; binding `reader_schema` options compile and reuse it internally.

### Arrow batch reads and writes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, venues: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
            ],
        )
    };

    // The name decides Avro; the methods name the write intent.
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.avro")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![1, 2], vec![Some("XNAS"), Some("XNYS")])?]),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![3], vec![Some("XLON")])?]),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![2, 4], vec![Some("XPAR"), None])?]),
        &options.clone().with_merge_by_names(["id"]),
    )?;

    let rows = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 4);
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
    batch = lambda ids, venues: pa.record_batch(
        {"id": ids, "venue": venues}, schema=schema
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by_names = ["id"]
    handle.merge_arrow_batch(batch([2, 4], ["XPAR", None]), options=merging)

    assert handle.read_arrow_reader().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, venues) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.avro'))
    handle.overwriteArrowTable(rows([1, 2], ['XNAS', 'XNYS']))
    handle.appendArrowTable(rows([3], ['XLON']))
    handle.mergeArrowTable(
      rows([2, 4], ['XPAR', null]),
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 4)
    fs.rmSync(root, { recursive: true, force: true })
    ```

Avro answers the same read and explicit overwrite, append, and merge intents as
every other encoding. Their [canonical signatures and streaming
rules](holder.md#canonical-record-write-signatures) apply to a handle whose media
type says `avro`, with no format argument anywhere. Decoding is columnar - one builder per leaf, appended per
record, with no intermediate `Scalar` tree on that path - and a declared schema
becomes the encoding's own projection: an unselected top-level column's bytes
are *skipped*, not decoded, so a projection saves decode, allocation, and
those bytes. What it cannot save is reading the row, because Avro interleaves
columns per record; Parquet, whose column chunks are separately addressable,
is where a projection also skips reading.

#### Measured batch operations

The read fixture contains 65,536 rows and four columns. The write fixture contains 4,096 rows;
Criterion prepares the stored side for append and keyed merge outside the timer. Keyed merge is
the upsert operation: matching `id` rows are updated and misses are inserted.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 26.0 ms | 2.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 6.10 ms | 671k rows/s |
| `append_arrow_reader` | 4,096 | 12.9 ms | 318k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 11.4 ms | 358k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them on the deployment host with
`io_dimensions/avro/read_rows` and `io_write_stateful/avro`; the format-level comparisons remain
in [Benchmarks](#benchmarks).

#### Dimensions and opened sessions

`row_size` walks block counts and encoded lengths, jumps each payload positionally, and validates
its synchronization marker without allocating, decompressing, or decoding rows. `column_size`
reads only the header schema. Both describe the whole container, ignoring selections, filters, and
limits. Closed calls derive fresh metadata; `open` retains the inferred Avro wrapper and caches its
schema and dimensions until `close`. Writes invalidate the cache.

The 65,536-row fixture measured fresh/opened `row_size` at 20.6 us/83.6 ns,
fresh/opened `column_size` at 22.3 us/87.8 ns, and fresh/opened `read_arrow_field` at
101 us/72.5 us. Regenerate with
`cargo bench -p yggdryl --bench io --all-features -- io_dimensions/avro`.

`avro::Avro` is the stateful form - handle, options, and a metadata cache that
[`IOBase::open`](holder.md) fills and `close` releases - and `avro::AvroOptions`
adds two settings to the shared surface: the block codec name and an optional
fixed synchronization marker for byte-reproducible writes. A union wider than
`null` plus one branch, a recursive schema, or a datatype Avro cannot spell is
refused by name on this surface; the `Scalar`-level functions below have no
such limits. Avro compresses inside its blocks, so - like Parquet and unlike
IPC - a handle declaring an outer content coding such as `trades.avro.gz` is
rejected rather than double-compressed.

#### Block encoding options

The generic `RecordOptions` exposes those Avro-only settings without downcasting.
Codec names are validated through the same core vocabulary the writer dispatches
before a row source is pulled: `null`, `deflate`, `zstandard`, and `snappy` when
the build includes its compression support. A fixed marker is either absent or
exactly 16 bytes; absence generates a fresh marker for each write. Setting either
property on options for another encoding is a typed record error.

=== "Rust"

    ```rust
    use yggdryl::media::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::AVRO)?;
    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("zstandard")?;
    options.set_avro_sync_marker(Some(b"0123456789abcdef"))?;
    assert_eq!(options.avro_sync_marker(), Some(b"0123456789abcdef"));
    ```

=== "Python"

    ```python
    from yggdryl import RecordOptions

    options = RecordOptions("trades.avro")
    assert options.block_codec == "deflate"
    assert options.sync_marker is None

    options.block_codec = "zstandard"
    options.sync_marker = b"0123456789abcdef"
    assert options.sync_marker == b"0123456789abcdef"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { RecordOptions } = require('yggdryl')

    const options = RecordOptions.from('trades.avro')
      .withBlockCodec('zstandard')
      .withSyncMarker(Buffer.from('0123456789abcdef'))

    assert.equal(options.blockCodec, 'zstandard')
    assert.deepEqual(options.syncMarker, Buffer.from('0123456789abcdef'))
    ```

The generic enum redirects these operations without downcasting or allocation. A local Windows
x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23) measured a codec
read at 9.65 ns and setting both codec and fixed marker at 49.8 ns. These are Criterion point
estimates; regenerate them on the deployment host with:

```console
cargo bench -p yggdryl --bench io --all-features -- "io_dimensions/avro/options"
```

### Flexible Scalar containers and schema methods

!!! note "Native Scalar surface"
    Python exposes this layer as `avro.Schema`, `loads` / `dumps`, and
    `loads_single` / `dumps_single`; JavaScript uses `Schema`, `loads` /
    `dumps`, and `loadsSingle` / `dumpsSingle`. Their natural host values cross
    through the same core [`Scalar`](types.md). Their `blocks` iterator keeps
    compressed blocks lazy; the explicit `Resolution` object remains Rust-only.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"quantity","type":"long"}]}
        "#,
    )?;
    let rows = [
        json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?,
        json::from_utf8(r#"{"symbol":"MSFT","quantity":25}"#)?,
    ];
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[("source", "docs")], &rows)?;

    let decoded = avro::read_container(&handle)?;
    assert_eq!(decoded.get("source"), Some("docs"));
    assert_eq!(decoded.rows.len(), 2);
    assert_eq!(
        decoded.rows[0].get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    schema = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "quantity", "type": "long"},
        ],
    }
    encoded = avro.dumps(
        [{"symbol": "AAPL", "quantity": 100}, {"symbol": "MSFT", "quantity": 25}],
        schema,
        metadata={"source": "docs"},
    )
    decoded = avro.loads(encoded)

    assert decoded.metadata == {"source": "docs"}
    assert decoded.rows[0] == {"quantity": 100, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'quantity', type: 'long' },
      ],
    }
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', quantity: 100 }, { symbol: 'MSFT', quantity: 25 }],
      schema,
      { source: 'docs' },
    )
    const decoded = avro.loads(encoded)

    assert.deepEqual(decoded.metadata, { source: 'docs' })
    assert.deepEqual(decoded.rows[0], { quantity: 100, symbol: 'AAPL' })
    ```

An Avro object container is a self-describing file: its header carries the
writer's schema as JSON, so reading one needs nothing but the bytes.
`read_container` hands back the schema, the header metadata, and every row;
`write_container` takes the schema as its JSON [`Scalar`](types.md), writes
that JSON into the header verbatim - so attributes this implementation does not
model, such as Iceberg's `field-id`, survive byte for byte - and encodes the
rows against it.

Rows cross the boundary in the JSON parser's vocabulary: a record is a mapping,
an array is a sequence, and a union carries the branch value directly - an
optional field reads as the value or `Null`, never as a wrapper naming the
branch.

### Schemas, canonical form, and fingerprints

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;

    let schema = Schema::from_str(
        r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2}
        ]}"#,
    )?;

    assert!(!schema.clone().into_canonical_form().contains("doc"));
    assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);
    let text = String::from_utf8(yggdryl::text::json::into_bytes(&schema.into_json())?)?;
    assert!(text.contains("field-id"));
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    document = {
        "type": "record",
        "name": "trade",
        "doc": "one fill",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2},
        ],
    }
    schema = avro.Schema(document)

    assert "doc" not in schema.into_canonical_form()
    assert schema.fingerprint().to_bytes(8, "little")[0] == 0xF5
    assert schema.into_json()["fields"][1]["field-id"] == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'trade',
      doc: 'one fill',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'long', 'field-id': 2 },
      ],
    })

    assert.ok(!schema.canonicalForm.includes('doc'))
    assert.equal(Number(schema.fingerprint & 0xffn), 0xf5)
    assert.equal(schema.intoJSON().fields[1]['field-id'], 2)
    ```

A [`Schema`] resolves namespaces, aliases, defaults, and recursive references
at parse time; a reference to a named type stays a reference, which is what
lets a recursive schema stay finite. `Schema::fingerprint` hashes the Parsing
Canonical Form with CRC-64-AVRO, so two schemas that differ only in
whitespace, attribute order, docs, or unknown attributes carry the same
fingerprint.

A fingerprint is a wire-layout identifier, not the schema object's equality
identity. Parsing Canonical Form deliberately strips logical annotations,
aliases, and defaults even though those change the native value produced or
reader resolution. Schema equality, total ordering, and `stable_hash` therefore
use the complete retained JSON document. The bindings redirect their
`equals`/comparison/hash protocols to that core identity; two schemas may have
the same Avro fingerprint while remaining distinct Yggdryl schema values.

### Logical types decode as what they mean

=== "Rust"

    ```rust
    use yggdryl::TimeUnit;
    use yggdryl::holder::Buffer;
    use yggdryl::{Timezone, Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}}
        ]}"#,
    )?;
    let row = Scalar::from_record([
        (
            "day",
            Scalar::date32_in(19_782, TimeUnit::Day, Timezone::NAIVE)?,
        ),
        ("at", Scalar::datetime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )?),
        ("price", Scalar::d128(18_750, 2)),
    ])?;

    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
    assert_eq!(avro::read_container(&handle)?.rows[0], row);
    ```

=== "Python"

    ```python
    from datetime import date, datetime, timezone
    from decimal import Decimal

    from yggdryl.media import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}},
        ],
    }
    row = {
        "day": date(2024, 2, 29),
        "at": datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc),
        "price": Decimal("187.50"),
    }

    decoded = avro.loads(avro.dumps([row], schema)).rows[0]
    assert decoded == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, avro } = require('yggdryl')

    const decimal = {
      type: 'bytes',
      logicalType: 'decimal',
      precision: 10,
      scale: 2,
    }
    const value = Scalar.decimal(18750n, 2)
    const decoded = avro.loadsSingle(avro.dumpsSingle(value, decimal), decimal)

    assert.ok(decoded instanceof Scalar)
    assert.equal(decoded.kind, 'd64')
    assert.equal(decoded.unscaled, 18750n)
    assert.equal(decoded.scale, 2)
    ```

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`,
`local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes
and fixed, and `duration` are modeled, because the value model is typed: a
date is `Date32`, a timestamp is `DateTime64` with `UTC`, and a decimal keeps
its exact coefficient and scale. An annotation this implementation does not know -
or one whose attributes are invalid for its underlying type - degrades to the
underlying type, as the specification requires, never to an error. A decimal
wider than the encoding's supported 38 digits keeps its raw bytes; an Avro
`duration` also keeps its twelve bytes because it is a three-part
month/day/millisecond interval, not one elapsed count.

### Reading with a different schema

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let writer = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"qty","type":"int"},
            {"name":"venue","type":"string"}]}"#,
    )?;
    let reader = Schema::from_str(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"quantity","aliases":["qty"],"type":"long"},
            {"name":"note","type":"string","default":"none"}]}"#,
    )?;
    let row = json::from_utf8(r#"{"symbol":"AAPL","qty":100,"venue":"XNAS"}"#)?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &writer, &[], &[row])?;

    let decoded = avro::read_container_resolved(&handle, &reader)?;
    assert_eq!(
        decoded.rows[0].get_key_str("quantity").and_then(Scalar::as_i64),
        Some(100),
    );
    assert_eq!(decoded.rows[0].len(), 2, "unwanted writer fields are skipped");
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    writer = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "int"},
            {"name": "venue", "type": "string"},
        ],
    }
    reader = avro.Schema({
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "quantity", "aliases": ["qty"], "type": "long"},
            {"name": "note", "type": "string", "default": "none"},
        ],
    })
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100, "venue": "XNAS"}], writer)

    assert avro.loads(encoded, reader_schema=reader).rows == [
        {"note": "none", "quantity": 100}
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const writer = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'int' },
        { name: 'venue', type: 'string' },
      ],
    }
    const reader = new avro.Schema({
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'quantity', aliases: ['qty'], type: 'long' },
        { name: 'note', type: 'string', default: 'none' },
      ],
    })
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', qty: 100, venue: 'XNAS' }],
      writer,
    )

    assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [
      { note: 'none', quantity: 100 },
    ])
    ```

`read_container_resolved` compiles the specification's resolution matrix into
a [`Resolution`] once per (writer, reader) pair and executes it per row:
fields match by name or reader alias in any order, `int` promotes to `long`,
`float`, or `double` (and `long` and `float` upward likewise), `string` and
`bytes` interchange, enum symbols map with the reader's default as fallback,
and unions resolve branch by branch. A writer field the reader does not name
is skipped without being decoded - length-prefixed values jump by their
prefix, and an array or map block written in the size-carrying form jumps as
one seek - which is what makes projection cheap. An illegal resolution is
refused when the plan is built, naming both sides and the field path; a union
branch the reader cannot accept fails only when a datum actually takes it,
which is the specification's rule.

### Streaming a large container

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(r#"{"type":"record","name":"row","fields":[
        {"name":"id","type":"long"}]}"#)?;
    let rows: Vec<Scalar> = (0..3)
        .map(|id| Scalar::from_record([("id", Scalar::from(id))]))
        .collect::<Result<_, _>>()?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &rows)?;

    let mut blocks = avro::read_blocks(&handle)?;
    assert_eq!(blocks.schema().kind(), "record");
    while let Some(block) = blocks.next_block()? {
        assert_eq!(block.rows()?.len() as u64, block.count());
    }
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [{"name": "id", "type": "long"}],
    }
    stream = avro.blocks(avro.dumps([{"id": 1}, {"id": 2}, {"id": 3}], schema))

    assert stream.schema.kind == "record"
    block = next(stream)
    assert block.count == len(block.rows())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = {
      type: 'record',
      name: 'row',
      fields: [{ name: 'id', type: 'long' }],
    }
    const stream = avro.blocks(avro.dumps([{ id: 1 }, { id: 2 }, { id: 3 }], schema))

    assert.equal(stream.schema.kind, 'record')
    const block = stream.next().value
    assert.equal(block.count, BigInt(block.rows().length))
    ```

`read_blocks` iterates a container over nothing but `pread`, so it works on
any handle without holding the file in memory. Python and JavaScript accept an
already-held byte value and copy it into the owning native handle once; block
payloads remain compressed and row decoding stays lazy. Their `avro.blocks`
iterator is fused after its first error. Each `Block` arrives still
compressed with its declared row count; `rows` decompresses and decodes it,
`rows_resolved` does the same through a [`Resolution`], and not calling either
skips the block entirely. `read_container` stays the fast case for small
self-describing files - an Iceberg manifest describes files, not rows, so it
is small by construction.

### Single-object encoding

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;
    use yggdryl::{Scalar};
    use yggdryl::media::avro;

    let schema = Schema::from_str(r#"{"type":"record","name":"tick","fields":[
        {"name":"price","type":"double"}]}"#)?;
    let value = Scalar::from_record([("price", Scalar::from(187.5))])?;
    let framed = avro::into_single_object_vec(&schema, &value)?;

    assert_eq!(&framed[..2], &[0xC3, 0x01]);
    assert_eq!(avro::from_single_object_slice(&framed, &schema)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    schema = avro.Schema({
        "type": "record",
        "name": "tick",
        "fields": [{"name": "price", "type": "double"}],
    })
    framed = avro.dumps_single({"price": 187.5}, schema)

    assert framed[:2] == b"\xc3\x01"
    assert avro.loads_single(framed, schema) == {"price": 187.5}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'tick',
      fields: [{ name: 'price', type: 'double' }],
    })
    const framed = avro.dumpsSingle({ price: 187.5 }, schema)

    assert.deepEqual(framed.subarray(0, 2), Buffer.from([0xc3, 0x01]))
    assert.deepEqual(avro.loadsSingle(framed, schema), { price: 187.5 })
    ```

A message system that cannot afford a container header per record frames each
datum as `C3 01`, the writer schema's Rabin fingerprint in little-endian
order, and the body. The fingerprint is how a receiver picks the writer schema
out of a store - and the natural key for caching a [`Resolution`].

### Codecs and limits

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Limits, Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(r#""long""#)?;
    let mut bytes = Buffer::new();
    avro::write_container(&mut bytes, &schema, &[], &[Scalar::from(7_i64)])?;

    let limits = Limits::new(8, 1_024, 8, 1);
    assert_eq!(
        avro::read_container_with_limits(&bytes, limits)?.rows,
        [Scalar::from(7_i64)]
    );
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

    encoded = avro.dumps([7], '"long"')
    decoded = avro.loads(
        encoded,
        max_depth=8,
        max_input_bytes=1_024,
        max_nodes=8,
    )

    assert decoded.rows == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const encoded = avro.dumps([7], '"long"')
    const decoded = avro.loads(encoded, {
      maxDepth: 8,
      maxInputBytes: 1024,
      maxNodes: 8,
    })

    assert.deepEqual(decoded.rows, [7])
    ```

Blocks are decompressed with the codec the header names: `null`, `deflate`,
and `zstandard` map onto the crate's own [`Codec`](types.md) implementations,
and `snappy` - raw Snappy followed by a big-endian CRC-32 of the uncompressed
block - decodes in builds carrying the `parquet` feature, which is what
already compiles the Snappy code. Any other name, `bzip2` and `xz` among
them, is refused naming it and listing what this build implements.

Every Rust reading entry point has a `_with_limits` form taking the crate's
[`Limits`](text.md); Python uses matching snake-case keywords and JavaScript
uses the camel-case options shown above. Input bytes bound the container and each decompressed
block, depth bounds schema and datum nesting, and the node budget bounds rows and what one datum
may allocate. Opening the mandatory header does not consume that row budget; its structure keeps
the core safety floor while the caller's byte and depth bounds still apply. Lazy block iteration
therefore reports a low node limit at the first row that exceeds it, after the header has opened.
A hostile container is a typed error, never an allocation the process dies of. Malformed input
carries the byte position at or immediately after the failure, in the same shape every other codec
in the crate reports.

[`Schema`]: #schemas-canonical-form-and-fingerprints
[`Resolution`]: #reading-with-a-different-schema

### Benchmarks

The JavaScript raw-codec path below uses one 4,580-byte container of 1,000
three-column rows, with fixtures outside the loops. From `npm run bench:codec`
on Node 24.18.0, an x86-64 Windows release build (AMD Ryzen 5 150):

| operation | ms/op |
| --- | ---: |
| schema parse / canonical form | 0.136 / 0.003 |
| container decode / resolve / encode | 13.075 / 10.549 / 18.667 |
| first compressed block / decode / resolve | 0.054 / 13.833 / 11.979 |
| single-object decode / encode | 0.018 / 0.087 |

The first-block number includes header parsing and the first lazy `next`, but
not row decompression. The block decode and resolution rows measure that work
separately.

#### Against fastavro and PyIceberg, on identical bytes

`python scripts/bench_avro_baseline.py` has the Rust half write one deterministic
ten-thousand-entry Iceberg manifest (112,246 bytes, statistics included) and then times three
implementations over that exact file - so the rows below are readers of identical bytes, not of
similar fixtures. From one containerized x86_64 Linux run (rustc stable **release** build,
CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1):

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

`full` is `read_manifest`, decoding every field the way the other two readers do.
`plan_stats` is `read_manifest_for_plan(handle, true)` - what a *filtered* scan runs - which keeps
the value counts, null counts, and bounds that pruning consults and skips the rest as bytes.
`plan_identity` is the unfiltered planning read, which keeps only file identity, partition tuple,
and sizes. The gap between the three is the measured worth of the skip routines: on this manifest
the planning path is 2.1x the full decode with statistics kept and 4.4x without, and the ratios
hold at 1,000 and 100,000 entries (`manifest/decode_full`, `manifest/decode_plan_with_stats`,
`manifest/decode_plan_identity_only` in the `iceberg` target).

From the same machine, the `avro` target's own groups
(`cargo bench --bench avro --features "parquet iceberg"`):

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode
  at ~3.5M rows/s; two-string rows at ~3.3M rows/s decode; 18-digit decimals at ~6.7M rows/s;
  the deeply nested family (array of records of maps) at ~620K rows/s. The single-object varint
  floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is
  nearly flat from 1,024 to 65,536 rows per block for every codec - the sweet spot is "anything
  above ~1,000 rows"; below that the per-block header and sync overhead starts to show. On this
  payload snappy decodes at ~20 MiB/s of encoded bytes, deflate at ~12, zstandard at ~9.6, and
  the null codec at ~38 (its bytes are bigger, so the row rate is what to compare).
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible):
  reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. Avro
  interleaves columns per record, so a projection can never skip *reading* a row - the saving is
  the decode and allocation of the 37 skipped columns, jumped by their length prefixes.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once;
  executing it per row is *cheaper* than the direct decode on this shape (4.12 ms against 4.74 ms
  for 10,000 rows) because the plan skips two writer columns the reader never wanted - the
  per-record cost of resolving is not just near zero, it can be negative.

## Plain-text records


A plain-text row starts with this schema:

| column | datatype | value |
| --- | --- | --- |
| `url` | `utf8` | source URL, or an empty string for an unlocated buffer |
| `rownum` | `int64` | present only when `with_rownum` is set; first value is exactly that setting |
| `body` | `binary` | line bytes without the record terminator |

Use `TextOptions` with the ordinary `read_arrow_reader` / `readArrowReader` or
`read_records` / `readRecords` methods. `Text` retains those options for
generic handles; it adds no line iterator, schema builder, or read/write
vocabulary.

`TextOptions` is flat and converts into the text variant of `RecordOptions` at
the generic dispatch boundary:

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per line; named captures append nullable columns |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `with_rownum` / `withRownum` | optional signed 64-bit first row number; unset omits the column |
| `autotype` | infer capture datatypes from regex syntax before reading; default `true` |
| `timezone` | zone applied when autotyping offset-free timestamps |

When `rowheader` matches, its complete match is removed from `body`. Edge
stripping runs afterward. A line without a match keeps its body and receives
null capture values.

[`DataType::from_regex`](types.md#regular-expression-captures) recognizes
captures constrained to booleans, signed 64-bit integers, finite floats, ISO
dates, times, and datetimes. Broad captures such as `\S+` stay UTF-8. Set
`autotype = false` to keep every capture as UTF-8. Because this examines the
expression rather than sampled rows, an empty or unopened resource answers the
same complete schema as a populated one.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, BinaryArray, Int64Array};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::media::text::TextOptions;
    use yggdryl::Url;

    let text_source = Buffer::from_bytes(
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\n".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.with_rownum = Some(1);
    text_options.set_rowheader(Some(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"))?;
    text_options.set_lstrip(Some(r"^\s+"))?;
    text_options.set_rstrip(Some(r"\s+$"))?;
    let text_source = text_source.into_text_with(text_options);
    let record_options = text_source.record_options()?;

    let text_batch = text_source
        .read_arrow_reader(&record_options)?
        .next()
        .unwrap()?;
    assert_eq!(text_batch.schema().fields().len(), 5);
    assert_eq!(
        text_batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[1, 2],
    );
    assert_eq!(
        text_batch
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"first",
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(b"  [INFO] id=7 first  \r\n[WARN] id=9 second\n")

        options = TextOptions()
        options.with_rownum = 1
        options.rowheader = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"
        options.lstrip = r"^\s+"
        options.rstrip = r"\s+$"

        handle = IOBase(source).into_text(options)
        rows = list(handle.read_records())
        assert [row["rownum"] for row in rows] == [1, 2]
        assert [row["body"] for row in rows] == [b"first", b"second"]
        assert [row["id"] for row in rows] == [7, 9]

        target = IOBase(pathlib.Path(directory) / "copy.txt")
        target.overwrite_records(
            ({"body": row["body"]} for row in rows),
            options=TextOptions(),
        )
        assert target.read_bytes() == b"first\nsecond\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const textRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const textSource = path.join(textRoot, 'app.log')
    fs.writeFileSync(textSource, '  [INFO] id=7 first  \r\n[WARN] id=9 second\n')

    const textOptions = new TextOptions()
    textOptions.withRownum = 1n
    textOptions.rowheader = '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)'
    textOptions.lstrip = '^\\s+'
    textOptions.rstrip = '\\s+$'

    const textHandle = new IOBase(textSource).intoText(textOptions)
    const textRows = [...textHandle.readRecords()]
    assert.deepEqual(textRows.map((row) => row.rownum), [1n, 2n])
    assert.deepEqual(
      textRows.map((row) => Buffer.from(row.body).toString()),
      ['first', 'second'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    const textTarget = new IOBase(path.join(textRoot, 'copy.txt'))
    textTarget.overwriteRecords(
      textRows.map((row) => ({ body: row.body })),
      new TextOptions(),
    )
    assert.equal(textTarget.readBytes().toString(), 'first\nsecond\n')

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

Writes consume the non-null Binary `body` column and append the configured
terminator. A body containing that terminator is refused. Overwrite and append
use the generic media methods; keyed merge remains unsupported for plain text.

Content codings belong to the handle. Thus `app.log.gz` and a folder mixing
plain and gzip leaves use the same options and stream decoded rows without
retaining prior pages. The line splitter retains only the unfinished fragment
needed across byte chunks.

#### Measuring the boundary

The three benchmark targets use the same generic record methods. Python also
includes an equivalent `re` plus PyArrow baseline; JavaScript numbers include
the copied IPC crossing required by Arrow JS.

```console
cargo bench -p yggdryl --bench text
cd python
.venv/Scripts/python benchmarks/media/text.py --min-time 0.05 --repeat 3
cd ..
npm run --prefix node bench:text -- --records 5000 --iterations 3
```


## Apache Iceberg


Read and write Apache Iceberg tables through one [`IOBase`](holder.md) handle.

!!! note "All three"
    Python has the table - create, open, scan, append, overwrite, evolve, and
    the metadata a commit produced - as `yggdryl.media.iceberg`, and JavaScript has
    the same surface as the `iceberg` namespace of `yggdryl`. The
    standalone document readers and writers stay in Rust, and each section below
    says so.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-lead");
    let _ = std::fs::remove_dir_all(&path);

    // A table is created in a folder, and a folder is all it ever touches.
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // A table that has never been written to has no current snapshot.
    assert!(table.current_snapshot().is_none());
    assert_eq!(table.scan(None)?.count(), 0);

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let snapshot = table.current_snapshot().expect("a snapshot");
    assert_eq!(snapshot.operation(), "append");
    assert_eq!(table.data_files()?.len(), 2, "one file per venue");

    // Reopening finds the table again, with no catalog in between.
    let reopened = Table::open(Folder::new(&path)?)?;
    let rows: usize = reopened.scan(None)?.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")

    # A table is created in a folder, and a folder is all it ever touches.
    table = Table.create(root, schema, ["venue"])

    # A table that has never been written to has no current snapshot.
    assert table.current_snapshot is None
    assert table.scan().read_all().num_rows == 0

    table.append(
        pa.record_batch(
            {"id": [1, 2], "venue": ["XNAS", "XNYS"]},
            schema=pa.schema([
                pa.field("id", pa.int64(), nullable=False),
                pa.field("venue", pa.string()),
            ]),
        )
    )

    assert table.current_snapshot is not None
    assert table.current_snapshot.operation == "append"
    assert len(table.data_files()) == 2, "one file per venue"

    # Reopening finds the table again, with no catalog in between.
    reopened = Table.open(IOBase(root.url.into_path()))
    assert reopened.scan().read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    })

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    // A table is created in a folder, and a folder is all it ever touches.
    const table = iceberg.Table.create(root, schema, ['venue'])

    // A table that has never been written to has no current snapshot.
    assert.equal(table.currentSnapshot, null)
    assert.equal(table.scan().intoTable().numRows, 0)

    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
      }),
    )

    assert.equal(table.currentSnapshot.operation, 'append')
    assert.equal(table.dataFiles().length, 2, 'one file per venue')

    // Reopening finds the table again, with no catalog in between.
    const reopened = iceberg.Table.open(root)
    assert.equal(reopened.scan().intoTable().numRows, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

The `iceberg` feature delegates metadata/schema mutation, validation, property
parsing, and manifest/list reads to official Iceberg 0.10.1. Yggdryl owns
[`IOBase`](holder.md) publication, the public [`Field`](types.md)/Arrow 59 boundary,
data-file writes, deterministic manifest/list writers, planning, and scans. The
local writers remain because the official 0.10.1 writers use an async `FileIO`
and Arrow 58 boundary outside `IOBase`, produce non-deterministic Avro bytes,
and encode Iceberg UUID partitions as Avro strings instead of `fixed[16]`.
The official crate's Arrow 58 types stay private.

An Iceberg table is one `IOBase` container: metadata and manifests live under
`metadata/`, and record files live under `data/`.

### The `iceberg` feature

`iceberg` is not a default feature:

```toml
[dependencies]
yggdryl = { version = "0.1", features = ["iceberg"] }
```

It enables Yggdryl's Parquet/Arrow 59 stack and official Iceberg 0.10.1.
Iceberg-enabled builds require Rust 1.94; default and schema-only builds retain
the workspace's Rust 1.85 baseline. No Arrow 58 value crosses the Rust, Python,
or JavaScript API. The boundary follows the
[Iceberg specification](https://iceberg.apache.org/spec/) and the official
[`TableMetadataBuilder`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableMetadataBuilder.html),
[`TableProperties`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableProperties.html),
and [`ManifestList`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.ManifestList.html)
contracts.

### What a table writes

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-layout");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let names: Vec<String> = Folder::new(&path)?
        .ls(true, false)
        .collect::<yggdryl::Result<Vec<_>>>()?
        .iter()
        .filter(|entry| !entry.is_container())
        .filter_map(|entry| entry.url().and_then(|url| url.file_name().map(str::to_owned)))
        .collect();

    // One Parquet data file, one manifest, one manifest list, two metadata
    // documents (create, then commit), and the version hint that finds them.
    assert!(names.iter().any(|name| name.ends_with(".parquet")));
    assert!(names.iter().any(|name| name.starts_with("snap-") && name.ends_with(".avro")));
    assert!(names.iter().any(|name| name.ends_with("-m0.avro")));
    assert!(names.contains(&"v1.metadata.json".to_owned()));
    assert!(names.contains(&"v2.metadata.json".to_owned()));
    assert!(names.contains(&"version-hint.text".to_owned()));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")

    table = Table.create(root, schema)
    table.append(
        pa.record_batch(
            {"id": [1]}, schema=pa.schema([pa.field("id", pa.int64(), nullable=False)])
        )
    )

    # `table.root` is the folder handle the table reads and writes through.
    names = [
        entry.name
        for entry in table.root.ls(recursive=True)
        if entry.is_file()
    ]

    # One Parquet data file, one manifest, one manifest list, two metadata
    # documents (create, then commit), and the version hint that finds them.
    assert any(name.endswith(".parquet") for name in names)
    assert any(name.startswith("snap-") and name.endswith(".avro") for name in names)
    assert any(name.endswith("-m0.avro") for name in names)
    assert "v1.metadata.json" in names
    assert "v2.metadata.json" in names
    assert "version-hint.text" in names
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema)
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }))

    // `table.root` is the folder handle the table reads and writes through.
    const names = [...table.root.ls(true)]
      .filter((entry) => entry.isFile())
      .map((entry) => entry.name)

    // One Parquet data file, one manifest, one manifest list, two metadata
    // documents (create, then commit), and the version hint that finds them.
    assert.ok(names.some((name) => name.endsWith('.parquet')))
    assert.ok(names.some((name) => name.startsWith('snap-') && name.endsWith('.avro')))
    assert.ok(names.some((name) => name.endsWith('-m0.avro')))
    assert.ok(names.includes('v1.metadata.json'))
    assert.ok(names.includes('v2.metadata.json'))
    assert.ok(names.includes('version-hint.text'))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

Committing means writing a new metadata document; nothing is mutated in place, which is what makes
the previous snapshot still readable afterwards. `Table::open` finds the current document the way
`HadoopTables` does - `metadata/version-hint.text`, falling back to the highest-numbered
`*.metadata.json` - because that is the only way to find a table without a catalog. A commit
publishes under the one name that hint resolves, `v{version}.metadata.json`, so every other
catalog-free reader finds it too; the unique `00003-<uuid>` name a commit writes first is how it
claims the version against a concurrent writer, and it is removed once the commit is published.
A table opened from somebody else's writer keeps whatever filename was discovered, official
`00003-<uuid>` names included, so the next `metadata-log` entry is exact. Metadata is gzip-decoded
by magic bytes; setting `write.metadata.compression-codec` to `gzip` makes later commits write
`.gz.metadata.json`.
Apache Iceberg's property parser rejects unsupported codecs before publication.

Every location a document records is read back relative to the table's own, so a table moves
between storage systems by rewriting its locations rather than its code. Two spellings of one
place resolve to it: `file:/warehouse` and `file:///warehouse` name the same folder, because a
Java writer's URI normalizer drops the empty authority these URLs keep, and a table written here
and committed into by Spark carries both at once.

### Table metadata, v1 through v3

!!! note "Rust only"
    A metadata document is read and written from Rust. The bindings read the
    version a table declares as its `format_version`.

```rust
use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, TableMetadata};
use yggdryl::DataType;

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
    .required_field("row");

// v1 keeps the singular `schema` and `partition-spec` keys and has no
// sequence numbers.
let v1 = TableMetadata::new(
    FormatVersion::V1,
    "file:///lake/trades",
    schema.clone(),
    PartitionSpec::unpartitioned(),
)?;
let document = v1.clone().into_json()?;
assert!(document.contains_key("schema"));
assert!(document.contains_key("partition-spec"));
assert!(!document.contains_key("last-sequence-number"));

// v2 makes the plural keys the authority and numbers every commit.
let v2 = TableMetadata::new(
    FormatVersion::V2,
    "file:///lake/trades",
    schema.clone(),
    PartitionSpec::unpartitioned(),
)?;
assert!(v2.clone().into_json()?.contains_key("last-sequence-number"));

// v3 adds row lineage.
let v3 = TableMetadata::new(
    FormatVersion::V3,
    "file:///lake/trades",
    schema,
    PartitionSpec::unpartitioned(),
)?;
assert_eq!(v3.next_row_id(), Some(0));
assert!(v3.clone().into_json()?.contains_key("next-row-id"));

// Every version reads back as itself.
for original in [v1, v2, v3] {
    let read = TableMetadata::from_json(&original.clone().into_json()?)?;
    assert_eq!(read.format_version(), original.format_version());
    assert!(read.current_snapshot().is_none());
}
```

`TableMetadata::from_json` parses and normalizes versions 1 through 3 through
the official crate. `into_json` renders Yggdryl's deterministic public view,
then validates the complete document with the official model.

`TableMetadata` has canonical value identity: schemas, partition specs, sort orders, snapshots,
properties, and refs compare as keyed collections independent of document order, while snapshot
and metadata logs retain their meaningful order. `Eq`, `Ord`, `Hash`, and `stable_hash` all use
that same identity. The Iceberg identity benchmark measures `stable_hash` over representative
metadata.

Official validation covers the version-specific shape. Round trips retain
table and partition statistics, encryption keys, snapshot key and row-lineage
fields, nanosecond temporals, `unknown`, and column defaults.
Rust mutates `statistics`, `partition-statistics`, and v3 `encryption-keys`
through the corresponding `TableMetadata` methods, all backed by the official
metadata builder.

Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1:

| Metadata operation | Median | Throughput |
| --- | ---: | ---: |
| Parse 100 snapshots and three 50-column schemas | 12.168 ms | 2.8613 MiB/s |
| Expire 99 of 100 snapshots | 9.5145 ms | 3.6592 MiB/s |
| Stable hash of the same metadata | 61.634 us | - |

### Snapshots and the current snapshot

!!! note "Rust only"
    The bindings read the same values off a table - its current snapshot and
    its snapshots - rather than off a metadata document.

```rust
use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, TableMetadata};
use yggdryl::DataType;

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
    .required_field("row");
let metadata = TableMetadata::new(
    FormatVersion::V2,
    "file:///lake/trades",
    schema,
    PartitionSpec::unpartitioned(),
)?;

// A table can have snapshots and still have no current one; that is a
// freshly created table, and a rolled-back one.
assert!(metadata.current_snapshot().is_none());

// `-1` is the other way a document spells "no current snapshot".
let document = metadata.into_json()?.with_key("current-snapshot-id", -1_i64)?;
let read = TableMetadata::from_json(&document)?;
assert!(read.current_snapshot_id().is_none());
assert!(read.current_snapshot().is_none());
```

A snapshot is one complete version of the table: an identifier, its manifests,
and a commit summary. Current snapshots use `manifest_list`; v1 metadata
may instead carry `manifests` directly. The latter is preserved through
official metadata updates, exposed as `Snapshot.manifests`, and synthesized
into conservative `ManifestFile` rows so scans and time travel use the same
planner. The *current* snapshot is a pointer, so a table without one reads as
zero rows.

### Manifest lists and manifests

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{
        EntryStatus, FormatVersion, PartitionSpec, Table, assign_field_ids, read_manifest,
        read_manifest_spec,
    };
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType, MimeType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-manifests");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec.clone())?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNAS")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    // A snapshot names one manifest list; each of its rows is a manifest.
    let manifests = table.manifests()?;
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].added_files_count, Some(1));
    assert_eq!(manifests[0].added_rows_count, Some(2));

    // A manifest is self-describing: its Avro header carries the schema and the spec.
    let name = manifests[0].manifest_path.rsplit('/').next().unwrap().to_owned();
    let handle = Folder::new(&path)?.child_by_path(&format!("metadata/{name}"))?;
    assert_eq!(read_manifest_spec(&handle)?, spec);

    let entries = read_manifest(&handle)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, EntryStatus::Added);
    assert_eq!(entries[0].data_file.mime_type, MimeType::PARQUET);
    assert_eq!(entries[0].data_file.record_count, 2);

    // Statistics are keyed by field id, which is what lets a planner skip a file.
    assert!(entries[0].data_file.value_counts.iter().any(|(id, count)| *id == 1 && *count == 2));
    assert!(entries[0].data_file.column_sizes.iter().any(|(id, _)| *id == 1));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, MimeType
    from yggdryl.media.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema, ["venue"])
    table.append(
        pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNAS"]}, schema=columns)
    )

    # A snapshot names one manifest list; each of its rows is a manifest.
    manifests = table.manifests()
    assert len(manifests) == 1
    assert manifests[0].is_data()
    assert manifests[0].added_files_count == 1
    assert manifests[0].added_rows_count == 2

    # Each manifest row is a data file plus what the writer measured about it.
    (file, spec), = table.data_files()
    assert file.mime_type == MimeType.PARQUET
    assert file.record_count == 2
    assert spec.fields[0].name == "venue"

    # Statistics are keyed by field id, which is what lets a planner skip a file.
    assert file.value_counts[1] == 2
    assert 1 in file.column_sizes
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, MimeType, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema, ['venue'])
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // A snapshot names one manifest list; each of its rows is a manifest.
    const manifests = table.manifests()
    assert.equal(manifests.length, 1)
    assert.equal(manifests[0].content, 'data')
    assert.equal(manifests[0].addedFilesCount, 1)
    assert.equal(manifests[0].addedRowsCount, 2)

    // Each manifest row is a data file plus what the writer measured about it.
    const [file] = table.dataFiles()
    assert.ok(file.mimeType.equals(MimeType.PARQUET))
    assert.equal(file.recordCount, 2)
    assert.deepEqual(file.partitionNames, ['venue'])

    // Statistics are keyed by field id, which is what lets a planner skip a file.
    assert.ok(file.valueCounts.some((entry) => entry.fieldId === 1 && entry.count === 2))
    assert.ok(file.columnSizes.some((entry) => entry.fieldId === 1))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

Iceberg puts two Avro levels between a snapshot and its rows. Full manifest and
manifest-list reads use the official parser after bounded input checks, then
map complete entries into Yggdryl values. This preserves encryption,
delete-file, split, bound, and v3 row-lineage fields. Writes use the core
[`avro`](media.md) codec through `IOBase`.

`read_manifest_spec` reads only the bounded Avro header and delegates its
metadata to the official parser; manifest entries are never decoded.

For UUID partitions, only the official parser's Avro-conversion failure on a
manifest declaring `fixed[16]` triggers a bounded compatibility view. That view
removes the unsupported UUID annotation without changing the 16 physical bytes,
then retries the official parser; other failures are returned unchanged.

Both readers validate through the official manifest parser. `read_manifest`
keeps every field for paths that may carry entries forward, such as overwrite,
merge, and compaction. `read_manifest_for_plan` then projects that validated
view to the file identity, partition, size, counts, and bounds used by pruning.
Scans select the planning view automatically.

All six manifest file/row counts are optional because the Iceberg wire format
permits null. Callers can distinguish an unreported count from zero.

For v3, the manifest-list writer follows the official row-id cursor rules:
existing assignments are preserved, new manifest ranges are contiguous, and
scans inherit missing data-file ids in manifest order. A first post-upgrade
commit assigns retained v2 files as well as new files, as required by
[first-row-id inheritance](https://iceberg.apache.org/spec/#first-row-id-inheritance).

Release Criterion on the machine above:

| Manifest operation, 100,000 entries | Median | Throughput |
| --- | ---: | ---: |
| Full official-validated decode | 5.8718 s | 17.031 K entries/s |
| Spec/header only; entries untouched | 190.02 us | 526.26 M nominal entries/s |

Statistics come from the Parquet footer the write just produced. Counts and sizes are emitted for
every top-level column; *bounds* are emitted only for the types whose Parquet statistic bytes are
byte-for-byte the Iceberg single-value encoding. A decimal is the case that differs - Parquet stores
it big-endian in a fixed width, Iceberg stores the minimal two's-complement big-endian - so a decimal
column gets counts but no bounds, rather than bounds that mean something else.

### Partition specs and the Hive layout

!!! note "Rust only"
    Rust applies transforms and renders partition paths. The bindings build
    identity specs and preserve every transform name when reading metadata.

```rust
use yggdryl::media::iceberg::{PartitionSpec, Transform, assign_field_ids};
use yggdryl::{DataType, Scalar};

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
assert_eq!(spec.fields[0].source_id, 2);
assert_eq!(spec.fields[0].field_id, 1000);
assert_eq!(spec.fields[0].transform, Transform::Identity);

// The directory chain is the `column=value` shape the crate's Hive reader knows.
assert_eq!(spec.partition_path(&[Scalar::from("XNAS")])?, "venue=XNAS");
assert_eq!(spec.partition_path(&[Scalar::Null])?, "venue=null");

// A partition value is nullable even when its source column is not.
let partition = spec.partition_field(&schema)?;
assert!(partition.fields()[0].is_nullable());

// Invertibility controls restoration, not write support.
assert!(Transform::Identity.is_invertible());
assert!(!Transform::from_str("bucket[16]")?.is_invertible());
assert!(!Transform::Unknown.is_invertible());
assert_eq!(Transform::Bucket(u32::MAX).to_string(), "bucket[4294967295]");
let mut hashed = spec.clone();
hashed.fields[0].name = "venue_bucket".into();
hashed.fields[0].transform = Transform::Bucket(16);
assert!(hashed.require_writable().is_ok());
hashed.fields[0].transform = Transform::Unknown;
assert!(hashed.require_writable().is_err());
```

Writes compute bucket, truncate, year, month, day, hour, identity, and void
values with the official scalar transform implementation. Typed scalar tuples
are grouping keys, so text or binary delimiter bytes cannot merge partitions.
Unknown transforms remain readable metadata but are rejected for writes.

Iceberg writes partition directories in exactly the `column=value` shape
[`Url::hive_partitions`](uri.md) already reads, so a table this module writes is also a lake the rest
of the crate can walk with [`IOBase::children_where`](holder.md). It is the same shape because it is the
same renderer: `partition_path` spells a value through
[`media::partition::partition_text`](holder.md#partition-columns-in-the-data), which is what a partitioned
folder write applies to a whole column, so a date is `day=2024-01-01` in a table and in a lake alike.
Unlike Hive, an Iceberg data file still stores its partition columns, so a scan needs no restoration
step in the normal case.

#### A field carries its own Iceberg vocabulary

A spec and a schema say the same thing, so neither has to be spelled twice. The partition tuple
carries what produced each of its columns - the transform, the source column, and the partition
marker every path-borne column carries - and a schema can carry the marks itself.

Those `iceberg:` properties are typed on the field's Iceberg view. `field.as_iceberg()` and
`field.as_iceberg_mut()` answer `IcebergField` and `IcebergFieldMut`, which parse and canonicalize
`schema_id`, `identifier_field_ids`, `doc`, `initial_default`, `write_default`, `spec_id`,
`partition_source_id` and `transform` on the way in and out. They live here rather than on
[`Field`](types.md#one-protocol-at-a-time) because they are Iceberg's vocabulary, not a field's own
state; the partition *mark* is a field's own state, which is why `is_partition` stays on the field
itself. The view borrows the whole field and dereferences to it, so one value answers both.

```rust
use yggdryl::media::iceberg::{PartitionSpec, Transform, assign_field_ids};
use yggdryl::DataType;

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;

// The tuple describes itself, so the spec reads back off it.
let partition = spec.partition_field(&schema)?;
assert_eq!(partition.as_iceberg().spec_id()?, Some(1));
let venue = partition.get_field_by_path("venue").expect("the partition column");
assert!(venue.is_partition());
assert_eq!(venue.as_iceberg().transform()?, Some(Transform::Identity));
assert_eq!(venue.as_iceberg().get("transform"), Some("identity"));

// The view is the field, so the name and the property come off one value.
assert_eq!(venue.as_iceberg().name(), "venue");
assert_eq!(PartitionSpec::from_partition_field(&partition)?, spec);

// And a schema that marks its own partition columns needs no column list.
let marked = spec.mark_partitions(&schema)?;
assert_eq!(marked.partition_field_names().collect::<Vec<_>>(), ["venue"]);
assert_eq!(PartitionSpec::from_schema(1, &marked)?, spec);
```

A table marks its stored schema this way when it is created and again when it is opened, so
`Table::schema` reports the layout whichever end you came in from - and the mark is core Field
metadata, not an Iceberg document key, so it survives into Arrow and Parquet without the table
metadata beside it.

**The manifest is the authority on a partition value, not the path.** A null value is spelled `null`
in a directory name, and a path cannot say whether that is the string `"null"` or the absence of a
value:

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-null-partition");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), None])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let files = table.data_files()?;
    assert_eq!(files.len(), 2);
    let (null_file, _) = files.iter().find(|(file, _)| file.partition[0].is_null()).unwrap();
    assert!(null_file.file_path.contains("venue=null"), "the path spells it");
    assert!(null_file.partition[0].is_null(), "the manifest means it");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema, ["venue"])
    table.append(pa.record_batch({"id": [1, 2], "venue": ["XNAS", None]}, schema=columns))

    files = table.data_files()
    assert len(files) == 2
    null_file, _ = next(pair for pair in files if pair[0].partition[0] is None)
    assert "venue=null" in null_file.path, "the path spells it"
    assert null_file.partition[0] is None, "the manifest means it"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema, ['venue'])
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        venue: arrow.vectorFromArray(['XNAS', null], new arrow.Utf8()),
      }),
    )

    const files = table.dataFiles()
    assert.equal(files.length, 2)
    const absent = files.find((file) => file.partition[0].asJs() === null)
    assert.ok(absent.filePath.includes('venue=null'), 'the path spells it')
    assert.equal(absent.partition[0].asJs(), null, 'the manifest means it')

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### Reading with column pushdown

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use std::sync::Arc;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-pushdown");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    // The target names the columns to keep; each file's Parquet reader gets it as
    // its own projection mask, so the dropped column chunk is never decoded.
    let wanted = schema.without_fields(&["symbol"])?;
    let reader = table.scan(Some(&wanted))?;
    assert_eq!(reader.schema().fields().len(), 1);
    for batch in reader {
        assert_eq!(batch?.num_columns(), 1);
    }

    // No target reads everything.
    assert_eq!(table.scan(None)?.schema().fields().len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema)
    table.append(
        pa.record_batch({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}, schema=columns)
    )

    # The target names the columns to keep; each file's Parquet reader gets it as
    # its own projection mask, so the dropped column chunk is never decoded.
    wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    reader = table.scan(wanted)
    assert reader.schema.names == ["id"]
    assert reader.read_all().num_rows == 2

    # No target reads everything.
    assert table.scan().schema.names == ["id", "symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('symbol: utf8')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema)
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      }),
    )

    // The target names the columns to keep; each file's Parquet reader gets it as
    // its own projection mask, so the dropped column chunk is never decoded.
    const wanted = fields.struct('row', [schema.dtype.getFieldAt(0)], { nullable: false })
    const projected = table.scan(wanted).intoTable()
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id'])
    assert.equal(projected.numRows, 2)

    // No target reads everything.
    assert.equal(table.scan().intoTable().numCols, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

`Table::scan` hands its optional `Field` to each data file as the schema
[`IOMedia::read_arrow_reader`](holder.md) reads under, minus the partition columns the file does not
store, then casts what comes back to the scan's own root. The pushdown is what makes a projected scan
cheap; the cast is what makes a table whose schema evolved readable as one shape.

### Planning a scan from the metadata

!!! note "All three"
    The planner is Rust, and both bindings report what it decided: `plan` and
    `plan_at` answer a `ScanPlan` in each language.
    [Filtered reads and filtered writes](#filtered-reads-and-filtered-writes)
    shows the same numbers from Python and JavaScript.

    The Rust plan retains its scan tasks because writes need them. The Python
    and JavaScript views deliberately retain only the bounded report
    `(record_count, files_planned, files_skipped, manifests_read,
    manifests_skipped)`. Those five counts are the complete binding value
    identity. JavaScript exposes `equals`, `compare`, `stableHash`, and `clone`
    over the camel-cased form of exactly that tuple; physical paths never enter
    its equality, order, or hash.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-plan");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // One commit per venue, so the manifest list has three rows to prune.
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON")] {
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema()?,
            vec![
                Arc::new(Int64Array::from(vec![id])),
                Arc::new(StringArray::from(vec![Some(venue)])),
            ],
        )?;
        table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // Nothing is listed: the snapshot names the manifest list, whose per-partition
    // summaries exclude two manifests before either Avro file is opened.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count()?, 1);
    assert_eq!(plan.manifests_read, 1);
    assert_eq!(plan.manifests_skipped(), 2);

    // A filter on a column the spec does not partition on prunes on the file's
    // own statistics instead, and then filters the rows the survivors hold.
    let bounded = table.plan(&[("id", "3")])?;
    assert_eq!(bounded.tasks.len(), 1);
    assert_eq!(bounded.files_skipped(), 2);

    let rows: usize = table
        .scan_where(&[("id", "3")], None)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 1);
    ```

A scan is planned entirely from the metadata, and every level of it prunes:

| Level | What it carries | What it skips |
| --- | --- | --- |
| Snapshot | the manifest list | every file an earlier snapshot named |
| Manifest list row | one `FieldSummary` per partition field | a whole manifest, unopened |
| Manifest entry | the file's partition tuple | one data file, unopened |
| Data file | per-column bounds and null counts | one data file, unopened |

A filter is an [expression](expression.md), and it is the same expression that filters a lake, a
batch, and a row. Every level of the chain answers it from the statistics it carries: a file's
partition tuple becomes a minimum equal to its maximum, so a conjunct the tuple *proves* is dropped
rather than re-tested per row, and a file's own path answers every free `&holder.*` attribute, so
`&holder.partition['venue'] = 'XNYS'` skips manifests before a byte is read. What no level settles is
filtered row by row afterwards - because a statistic bounds a *file* and does not select a row.

`scan_matching` and `plan_matching` take the whole language; `scan_where` and `plan` keep the
`(column, value)` pairs and build an expression from them, with the text read through the column's
own datatype. `ScanPlan` reports what was skipped at each level, so "a filtered read touches only the
files the metadata says it must" is something a caller can assert on rather than believe.

`ScanTask` and `ScanPlan` are complete immutable plan snapshots. Their `stable_hash` methods cover
the full ordered task/exclusion/skip state and counters; no live reader or table handle enters the
identity. Both have representative Criterion cases in the Iceberg identity benchmark group.

=== "Rust"

    ```{ .rust .ignore }
    // Ranges, null tests, and questions about the file, in one predicate.
    let reader = table.scan_matching(
        "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'",
        None,
    )?;
    ```

=== "Python"

    ```{ .python .ignore }
    reader = table.scan_matching(
        "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'"
    )
    ```

=== "JavaScript"

    ```{ .javascript .ignore }
    const reader = table.scanMatching(
      "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'",
    )
    ```

### Time travel and the inspection tables

!!! note "All three"
    `scan_at` / `scanAt`, `snapshot_by_ref` / `snapshotByRef`, and the three
    inspection readers cross into both bindings as ordinary record-batch
    readers.

Nothing a commit writes is mutated in place, so every retained snapshot is still a complete table.
Reading one is an ordinary scan with the snapshot named:

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-time-travel");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.into_arrow_schema()?;
    let one = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![1]))],
    )?;
    table.commit_append(yggdryl::arrow::batch_reader(std::sync::Arc::clone(&arrow_schema), [one]))?;
    let past = table.current_snapshot().expect("one commit").snapshot_id;

    let nine = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![9]))],
    )?;
    table.commit_overwrite(yggdryl::arrow::batch_reader(arrow_schema, [nine]))?;

    // The present shows the overwrite; the retained snapshot shows what was.
    assert_eq!(table.scan(None)?.count(), 1);
    let history = table.scan_at(past, &[], None)?.next().expect("one batch")?;
    assert_eq!(history.num_rows(), 1);

    // Planning history prunes exactly as planning the present does.
    assert_eq!(table.plan_at(past, &[])?.tasks.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"

    table = Table.create(IOBase(root), columns)
    table.append(pa.record_batch({"id": [1]}, schema=columns))
    past = table.current_snapshot.snapshot_id
    table.overwrite(pa.record_batch({"id": [9]}, schema=columns))

    # The present shows the overwrite; the retained snapshot shows what was.
    assert table.scan().read_all().column("id").to_pylist() == [9]
    assert table.scan_at(past).read_all().column("id").to_pylist() == [1]

    # A branch or tag resolves by name, and every commit moves `main`.
    assert table.snapshot_by_ref("main").snapshot_id == table.current_snapshot.snapshot_id

    # The inspection readers render the table's own record as record batches.
    assert table.inspect_history().read_all().num_rows == 2
    assert sorted(table.inspect_snapshots().read_all().column("operation").to_pylist()) == [
        "append",
        "overwrite",
    ]
    assert table.inspect_files().read_all().num_rows == 1

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')

    const table = iceberg.Table.create(root, schema)
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }))
    const past = table.currentSnapshot.snapshotId
    table.overwrite(new arrow.Table({ id: arrow.vectorFromArray([9n], new arrow.Int64()) }))

    // The present shows the overwrite; the retained snapshot shows what was.
    assert.deepEqual(table.scan().intoTable().getChild('id').toArray(), BigInt64Array.from([9n]))
    assert.deepEqual(table.scanAt(past).intoTable().getChild('id').toArray(), BigInt64Array.from([1n]))

    // A branch or tag resolves by name, and every commit moves `main`.
    assert.equal(table.snapshotByRef('main').snapshotId, table.currentSnapshot.snapshotId)

    // The inspection readers render the table's own record as record batches.
    assert.equal(table.inspectHistory().intoTable().numRows, 2)
    assert.deepEqual(
      Array.from(table.inspectSnapshots().intoTable().getChild('operation')).sort(),
      ['append', 'overwrite'],
    )
    assert.equal(table.inspectFiles().intoTable().numRows, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A snapshot is read as the schema that was current when it was written, so a column added later does
not appear and a column dropped later still does. A branch or tag resolves with `snapshot_by_ref`,
and a metadata-only change - a property, a new ref, an evolved schema - commits through
`commit_metadata_changes`, which writes one new metadata document and leaves the table untouched when the
change or the write fails.

The table also renders its own record as record batches, under the column names PyIceberg's
inspection tables use: `inspect_history` (when each snapshot became current, and whether it is on
the current ancestry chain), `inspect_snapshots` (operation, manifest list, and the summary map per
retained snapshot), and `inspect_files` (path, format, spec, rendered `column=value` partition
chain, row count, and size per live data file). They are ordinary readers, so the same collect that
drains a scan drains them.

### Filtered reads and filtered writes

!!! note "All three"
    The whole surface crosses. Python spells it `plan`, `plan_at`,
    `scan_where`, `overwrite_where`, `merge`, and `merge_where`; JavaScript
    spells the same six `plan`, `planAt`, `scanWhere`, `overwriteWhere`,
    `merge`, and `mergeWhere`; and `ScanPlan` reports the same five numbers in
    each language's casing.

    Rust says `commit_overwrite_where` and `commit_merge_where` for the two
    writes, because [`Table` is also an `IOMedia`](holder.md#arrow-batches) and a
    bare `overwrite` would sit beside the trait's own `overwrite_arrow_reader`
    with no way to tell which configuration a call resolves. The bindings keep
    the short verb and pass the table's own `IcebergOptions`; a filter of
    `None` selects every row in each language.

A filter is a column name and a value as text - the vocabulary
[`IOBase::children_where`](holder.md) filters a lake with - and it crosses as a mapping or as a
sequence of `(column, value)` pairs. `plan` reports what a read *would* open without opening
anything; `scan_where` reads the rows that match; `overwrite_where` replaces only what matches;
`merge` and `merge_where` upsert on a match key. They belong in one section because they are one
mechanism: each decides what to touch through
[the metadata chain the planning section walks](#planning-a-scan-from-the-metadata), and only then
opens a data file. `plan_at` and `scan_at` do the same over a retained snapshot, and `scan_ref`
over the snapshot a branch or tag names.

=== "Rust"

    ```rust
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::iceberg::{DataFile, FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("qty"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-filtered-writes");
    let _ = std::fs::remove_dir_all(&root);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&root)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.into_arrow_schema()?;
    let rows = |ids: Vec<i64>, venues: Vec<&'static str>, quantities: Vec<i64>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
                Arc::new(Int64Array::from(quantities)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };

    // One commit per venue, so the manifest list has three rows to prune.
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON")] {
        table.commit_append(rows(vec![id], vec![venue], vec![10]))?;
    }
    let inserted = table.current_snapshot().expect("three commits").snapshot_id;

    // Nothing is listed and no data file is opened: the manifest list's
    // per-partition summaries exclude two manifests before either is read.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count()?, 1);
    assert_eq!(plan.manifests_read, 1);
    assert_eq!(plan.manifests_skipped(), 2);
    assert_eq!(table.scan_where(&[("venue", "XNYS")], None)?.count(), 1);

    // A filter on a column the spec does not partition on prunes on the file's
    // own recorded bounds instead, then filters the rows the survivors hold.
    assert_eq!(table.plan(&[("id", "3")])?.files_skipped(), 2);

    // A filtered overwrite replaces the files the filter selects and carries
    // every other file into the new snapshot at its own path, statistics and all.
    let paths = |files: Vec<(DataFile, PartitionSpec)>| -> BTreeSet<String> {
        files.into_iter().map(|(file, _)| file.file_path.to_string()).collect()
    };
    let before = paths(table.data_files()?);
    table.commit_overwrite_where(&[("venue", "XNYS")], rows(vec![2], vec!["XNYS"], vec![99]))?;
    let after = paths(table.data_files()?);
    assert_eq!(before.difference(&after).count(), 1, "one partition was rewritten");
    assert_eq!(before.intersection(&after).count(), 2, "the others were carried");

    // A merge upserts on the key: 3 is stored and updates, 4 is new and appends.
    table.commit_merge(rows(vec![3, 4], vec!["XLON", "XLON"], vec![7, 8]), &["id".to_owned()], true)?;
    let total: usize = table
        .scan(None)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(total, 4);

    // Narrowed first: a merge into one partition can read no other partition.
    table.commit_merge_where(
        &[("venue", "XNAS")],
        rows(vec![1], vec!["XNAS"], vec![42]),
        &["id".to_owned()],
        true,
    )?;

    // History plans the same way: the snapshot before the overwrite still
    // selects one file for that partition.
    assert_eq!(table.plan_at(inserted, &[("venue", "XNYS")])?.tasks.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
        pa.field("qty", pa.int64()),
    ])
    rows = lambda ids, venues, quantities: pa.record_batch(
        {"id": ids, "venue": venues, "qty": quantities}, schema=columns
    )

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns, ["venue"])

    # One commit per venue, so the manifest list has three rows to prune.
    for identifier, venue in [(1, "XNAS"), (2, "XNYS"), (3, "XLON")]:
        table.append(rows([identifier], [venue], [10]))
    inserted = table.current_snapshot.snapshot_id

    # Nothing is listed and no data file is opened: the manifest list's
    # per-partition summaries exclude two manifests before either is read.
    plan = table.plan({"venue": "XNYS"})
    assert (plan.files_planned, plan.record_count) == (1, 1)
    assert (plan.manifests_read, plan.manifests_skipped) == (1, 2)
    assert table.scan_where({"venue": "XNYS"}).read_all().num_rows == 1

    # A filter on a column the spec does not partition on prunes on the file's
    # own recorded bounds instead, then filters the rows the survivors hold.
    assert table.plan([("id", "3")]).files_skipped == 2

    # A filtered overwrite replaces the files the filter selects and carries
    # every other file into the new snapshot at its own path, statistics and all.
    before = {file.path for file, _ in table.data_files()}
    table.overwrite_where({"venue": "XNYS"}, rows([2], ["XNYS"], [99]))
    after = {file.path for file, _ in table.data_files()}
    assert len(before - after) == 1, "one partition was rewritten"
    assert len(before & after) == 2, "the other two were carried, not rewritten"

    # A merge upserts on the key: 3 is stored and updates, 4 is new and appends.
    table.merge(rows([3, 4], ["XLON", "XLON"], [7, 8]), ["id"])
    merged = table.scan().read_all().sort_by("id").to_pydict()
    assert merged["id"] == [1, 2, 3, 4]
    assert merged["qty"] == [10, 99, 7, 8]

    # Narrowed first: a merge into one partition can read no other partition.
    table.merge_where({"venue": "XNAS"}, rows([1], ["XNAS"], [42]), ["id"])
    assert table.scan_where({"venue": "XNAS"}).read_all().column("qty").to_pylist() == [42]

    # History plans the same way: the snapshot before the overwrite still
    # selects one file for that partition, and it is the file that held 10.
    assert table.plan_at(inserted, {"venue": "XNYS"}).files_planned == 1
    assert table.scan_at(inserted, {"venue": "XNYS"}).read_all().column(
        "qty"
    ).to_pylist() == [10]

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('venue: utf8'), Field.from('qty: int64')],
      { nullable: false },
    )
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, schema, ['venue'])

    const rows = (ids, venues, quantities) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
        qty: arrow.vectorFromArray(quantities, new arrow.Int64()),
      })

    // One commit per venue, so the manifest list has three rows to prune.
    for (const [id, venue] of [[1n, 'XNAS'], [2n, 'XNYS'], [3n, 'XLON']]) {
      table.append(rows([id], [venue], [10n]))
    }
    const inserted = table.currentSnapshot.snapshotId

    // Nothing is listed and no data file is opened: the manifest list's
    // per-partition summaries exclude two manifests before either is read.
    const plan = table.plan({ venue: 'XNYS' })
    assert.equal(plan.filesPlanned, 1)
    assert.equal(plan.recordCount, 1)
    assert.equal(plan.manifestsRead, 1)
    assert.equal(plan.manifestsSkipped, 2)
    assert.equal(table.scanWhere({ venue: 'XNYS' }).intoTable().numRows, 1)

    // A filter on a column the spec does not partition on prunes on the file's
    // own recorded bounds instead, then filters the rows the survivors hold.
    assert.equal(table.plan([{ column: 'id', value: '3' }]).filesSkipped, 2)

    // A filtered overwrite replaces the files the filter selects and carries
    // every other file into the new snapshot at its own path, statistics and all.
    const paths = () => new Set(table.dataFiles().map((file) => file.filePath))
    const before = paths()
    table.overwriteWhere({ venue: 'XNYS' }, rows([2n], ['XNYS'], [99n]))
    const after = paths()
    assert.equal([...before].filter((file) => !after.has(file)).length, 1)
    assert.equal([...before].filter((file) => after.has(file)).length, 2)

    // A merge upserts on the key: 3 is stored and updates, 4 is new and appends.
    table.merge(rows([3n, 4n], ['XLON', 'XLON'], [7n, 8n]), ['id'])
    const merged = new Map(table.scan().intoTable().toArray().map((row) => [row.id, row.qty]))
    assert.deepEqual([...merged.keys()].sort(), [1n, 2n, 3n, 4n])
    assert.equal(merged.get(2n), 99n)
    assert.equal(merged.get(4n), 8n)

    // Narrowed first: a merge into one partition can read no other partition.
    table.mergeWhere({ venue: 'XNAS' }, rows([1n], ['XNAS'], [42n]), ['id'])
    assert.equal(table.scanWhere({ venue: 'XNAS' }).intoTable().getChild('qty').get(0), 42n)

    // History plans the same way: the snapshot before the overwrite still
    // selects one file for that partition, and it is the file that held 10.
    assert.equal(table.planAt(inserted, { venue: 'XNYS' }).filesPlanned, 1)
    assert.equal(
      table.scanAt(inserted, { venue: 'XNYS' }).intoTable().getChild('qty').get(0),
      10n,
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

**A filtered overwrite rewrites one partition, not the table.** The plan decides which data files
the filter selects; every other file is carried into the new snapshot as its manifest entry
already stands - same path, same statistics, same commit order - so nothing outside the selection
is read, decoded, or re-encoded. Replacing one partition of a thousand costs one partition, and the
carried files stay byte-identical, which is what lets the snapshot before the overwrite still be
read: the rewrite wrote new files beside the old ones rather than over them. A delete is the same
call with nothing incoming, which is why the [Spark quickstart](#the-spark-quickstart-locally)
spells `DELETE FROM ... WHERE vendor_id = 1` as an `overwrite_where`: the selected partition is
replaced by no rows, and the other partition's file is carried into the new snapshot untouched.

**A merge reads the files whose statistics could hold an incoming key.** For each stored file the
merge asks one question - could any incoming match key fall inside this file's recorded lower and
upper bounds for the key columns? - and reads only the files that answer yes. The rest are carried
forward unread. Correctness does not depend on how tight those bounds are: a file that is not read
keeps every row it had, so coarse statistics make a merge read more files, never the wrong ones.
That is what makes an upsert cost the files it can actually change rather than the table, and
`merge_where` narrows the candidates once more before the bounds are consulted at all.

Both are worth measuring rather than believing, which is what `ScanPlan` is for: `record_count`,
`files_planned`, `files_skipped`, `manifests_read`, and `manifests_skipped` are what the metadata
alone decided, reported before a single data file is opened. A plan that skips nothing says the
filter is not one the layout can answer - a filter on a non-partition column can only prune on
per-file bounds, and bounds on a column whose values are scattered across every file exclude
nothing. Neither `overwrite_where` nor `merge` rebases after
[a lost commit](#concurrent-writers-and-commit-retries): each planned against files the winner may
have replaced, so both raise and leave the caller to re-plan.

### The three record methods over a table

=== "Rust"

    ```rust
    use yggdryl::media::IORecordOptions;
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-records");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.into_arrow_schema()?;
    let rows = |ids: Vec<i64>, venues: Vec<&'static str>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };

    // The folder *is* the table, so the ordinary record surface reaches it. Its
    // options come from the metadata, before a single data file exists.
    let mut folder = Folder::new(&path)?;
    let options = folder.record_options()?;
    folder.overwrite_arrow_reader(rows(vec![1, 2], vec!["XNAS", "XNYS"]), &options)?;
    folder.append_arrow_reader(rows(vec![3], vec!["XLON"]), &options)?;

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    let merging = options.clone().with_merge_by_names(["id"]);
    folder.merge_arrow_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;

    let total: usize = folder
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);

    // Each call was one commit, and the read went through the last one.
    let table = Table::open(Folder::new(&path)?)?;
    assert_eq!(table.metadata().snapshots().len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = columns
    rows = lambda ids, venues: pa.record_batch(
        {"id": ids, "venue": venues}, schema=columns
    )

    path = pathlib.Path(tempfile.mkdtemp()) / "trades"
    Table.create(IOBase(path), schema, ["venue"])

    # The folder *is* the table, so the ordinary record surface reaches it. Its
    # options come from the metadata, before a single data file exists.
    folder = IOBase(path)
    options = folder.record_options()
    folder.overwrite_arrow_batch(rows([1, 2], ["XNAS", "XNYS"]), options=options)
    folder.append_arrow_batch(rows([3], ["XLON"]), options=options)

    # A match key upserts: `2` is stored and updates, `9` is new and appends.
    merging = folder.record_options()
    merging.merge_by_names = ["id"]
    folder.merge_arrow_batch(rows([2, 9], ["XNYS", "XLON"]), options=merging)

    assert folder.read_arrow_reader(options=options).read_all().num_rows == 4

    # Each call was one commit, and the read went through the last one.
    assert len(Table.open(IOBase(path)).snapshots) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8?')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    iceberg.Table.create(root, schema, ['venue'])

    const rows = (ids, venues) =>
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray(ids, new arrow.Int64()),
          venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
        }),
      )

    // The folder *is* the table, so the ordinary record surface reaches it. Its
    // options come from the metadata, before a single data file exists.
    const folder = IOBase.from(root)
    const options = folder.recordOptions()
    folder.overwriteArrowReader(rows([1n, 2n], ['XNAS', 'XNYS']), options)
    folder.appendArrowReader(rows([3n], ['XLON']), options)

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    folder.mergeArrowReader(
      rows([2n, 9n], ['XNYS', 'XLON']),
      options.withMergeByNames(['id']),
    )

    assert.equal(folder.readArrowReader(options).intoTable().numRows, 4)

    // Each call was one commit, and the read went through the last one.
    assert.equal(iceberg.Table.open(root).snapshots.length, 3)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A handle addressing a table's folder is not read as a folder of Parquet files: it
is read through the current snapshot, so a file an overwrite replaced is never
read back and a stray file nobody committed is never read at all. The shared
[record-write contract](holder.md#canonical-record-write-signatures) keeps each
intent explicit, and each call is a single commit:

- `read_arrow_reader` scans the current snapshot, planning as above.
- `overwrite_arrow_reader` replaces every row.
- `merge_arrow_reader` requires match keys and reads only the data files whose recorded bounds for the key columns
  overlap the incoming keys, carrying the rest into the new snapshot untouched
  - same location, same statistics, same commit order. That is what makes an
  upsert cost the files it can actually change, and it stays correct however
  coarse the statistics are, because a file that is not read keeps every row.
- `append_arrow_reader` writes new data files and keeps every manifest the
  last snapshot had, so nothing stored is read or rewritten.

The relationship runs the other way too: a `Table` value is itself a handle, so
the same primitives work on it directly. The folder route above probes the
location for a table on every call; the `Table` implementation answers from the
metadata the value already holds, and each answer is the better one.
`record_options` names the data files' encoding before the first file exists,
`read_arrow_field` is the stored schema with its field identifiers rather than a
shape lifted off decoded batches, a `filter_partitions` pair prunes data files
through the scan plan instead of filtering rows after they were decoded, and a
write is one commit the value reports immediately - `current_snapshot` and
`version` stay current without reopening anything. One deliberate difference: a
filter naming a column the schema does not declare is an error, because a
table's schema is authoritative, where a folder of leaves ignores a column its
batches do not carry. (The Python and JavaScript tables keep their own scan and
commit vocabulary; there, the folder handle above is the generic route.)

It says what it is, too: `IOBase::kind` on a `Table` is [`IOKind::Table`](types.md)
rather than the `Directory` its root folder would answer, because the files below
a table are its storage and not its contents, and `is_tabular` is `true` without
touching storage at all. The folder route reaches the same shape by probing the
location for a metadata document; holding the table skips the probe, exactly as it
skips it everywhere else.

```rust
use yggdryl::media::IORecordOptions;
use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::local::Folder;
use yggdryl::{arrow, DataType, MimeType};

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-table-handle");
let _ = std::fs::remove_dir_all(&path);
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

// The role is the table's own, and it costs nothing to say so.
assert_eq!(IOBase::kind(&table), yggdryl::IOKind::Table);
assert!(table.is_container());
assert!(table.is_tabular());
assert!(!table.is_atomic());

// The record surface answers before a single data file exists: the encoding
// from the metadata, the schema with its field identifiers.
let options = table.record_options()?;
assert_eq!(options.mime_type(), MimeType::PARQUET);
assert_eq!(
    table.read_arrow_field(&options)?.fields()[0].parquet_field_id()?,
    Some(1),
);

let arrow_schema = schema.into_arrow_schema()?;
let rows = |ids: Vec<i64>, venues: Vec<&'static str>| {
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(venues)),
        ],
    )
    .expect("a batch matching the root");
    arrow::batch_reader(batch.schema(), [batch])
};

// Each generic write is one commit, and the value's metadata follows it
// without reopening anything.
table.append_arrow_reader(rows(vec![1, 2], vec!["XNAS", "XNYS"]), &options)?;
let merging = options.clone().with_merge_by_names(["id"]);
table.merge_arrow_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;
assert_eq!(table.metadata().snapshots().len(), 2);
assert_eq!(table.current_snapshot().unwrap().operation(), "overwrite");

// A partition filter is answered by the scan plan, so the other partitions'
// files are never opened.
let filtered = options.clone().with_filter_partitions([("venue", "XNYS")]);
let matching: usize = table
    .read_arrow_reader(&filtered)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(matching, 1);
```

A handle addressing one of the table's `column=value` directories addresses that
partition of it, exactly as it would in a plain Hive lake - the difference is that
the files come from the manifest rather than from a directory listing:

```rust
use yggdryl::media::IORecordOptions;
use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::local::Folder;
use yggdryl::{arrow, DataType};

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-partition");
let _ = std::fs::remove_dir_all(&path);
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

let batch = RecordBatch::try_new(
    schema.into_arrow_schema()?,
    vec![
        Arc::new(Int64Array::from(vec![1_i64, 2])),
        Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
    ],
)?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

let partition = Folder::new(path.join("data").join("venue=XNYS"))?;
let options = partition.record_options()?;
let rows: usize = partition
    .read_arrow_reader(&options)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(rows, 1);
```

### A warehouse of tables

!!! note "All three"
    The catalog crosses whole: Python has it as `yggdryl.media.iceberg.Catalog` and
    JavaScript as `iceberg.Catalog`, over the same warehouse folder and the
    same dotted names.

A caller who has rows and a dotted name should need nothing else. `Catalog` is that surface: one
warehouse folder, namespaces as nested folders, and a table per name - `HadoopCatalog`'s layout,
reached through [`IOBase`](holder.md) and nothing else.

Storage sees three indistinguishable folders there, so each value says which role it plays:
`Catalog::kind` is [`IOKind::Catalog`](types.md), `Namespace::kind` is `IOKind::Namespace`, and a
`Table` answers `IOKind::Table` through `IOBase::kind`. The framing is what tells them apart, so it
is the framing that answers - never a listing, and never a guess. (`IOKind` is Rust-only, as it is
everywhere else; the bindings ask the questions rather than name the kinds.)

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::iceberg::Catalog;
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let warehouse = Folder::temporary()?.path()?.join("yggdryl-doc-warehouse");
    let _ = std::fs::remove_dir_all(&warehouse);
    let catalog = Catalog::new(Folder::new(&warehouse)?);

    // Rows and a name are enough: the first append creates the table with the
    // schema the rows carry, and the second appends to it.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row")
    .with_partition_fields(&["venue"])?;
    let arrow_schema = schema.into_arrow_schema()?;
    let rows = |ids: &[i64], venues: &[&str]| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(venues.to_vec())),
            ],
        )
    };
    let first = rows(&[1, 2], &["XNAS", "XNYS"])?;
    let table = catalog
        .tables()
        .append_arrow_reader("nyc.trades", yggdryl::arrow::batch_reader(first.schema(), [first]))?;
    let rows_read: usize = table.scan(None)?.map(|batch| batch.map(|b| b.num_rows())).sum::<Result<usize, _>>()?;
    assert_eq!(rows_read, 2);

    let second = rows(&[3], &["XNAS"])?;
    catalog
        .tables()
        .append_arrow_reader("nyc.trades", yggdryl::arrow::batch_reader(second.schema(), [second]))?;

    // The partition marks the schema carried became the table's spec.
    let reopened = catalog.table("nyc.trades")?;
    assert_eq!(reopened.metadata().default_spec()?.fields[0].name, "venue");
    assert!(catalog.tables().contains("nyc.trades")?);
    let namespaces: Vec<String> =
        catalog.namespaces().iter().collect::<yggdryl::Result<_>>()?;
    assert_eq!(namespaces, ["nyc"]);
    let tables: Vec<String> = catalog
        .namespaces()
        .get("nyc")?
        .tables()
        .iter()
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(tables, ["trades"]);

    let _ = std::fs::remove_dir_all(&warehouse);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import DataType, Field
    from yggdryl.media.iceberg import Catalog

    warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "warehouse"
    catalog = Catalog(warehouse)

    # Rows and a name are enough: the first append creates the table with the
    # schema the rows carry, and the second appends to it.
    marked = Field(
        "row",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field("venue", "string"),
        ]),
        nullable=False,
    ).with_partition_fields(["venue"])
    columns = pa.schema([child.into_arrow() for child in marked.dtype])

    table = catalog.append(
        "nyc.trades", pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns)
    )
    assert table.scan().read_all().num_rows == 2

    catalog.append("nyc.trades", pa.table({"id": [3], "venue": ["XNAS"]}, schema=columns))

    # The partition marks the schema carried became the table's spec.
    reopened = catalog.table("nyc.trades")
    assert [field.name for field in reopened.spec.fields] == ["venue"]
    assert reopened.scan().read_all().num_rows == 3
    assert "nyc.trades" in catalog.tables
    assert list(catalog.namespaces) == ["nyc"]
    assert list(catalog.namespaces["nyc"].tables) == ["trades"]

    shutil.rmtree(warehouse.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const warehouse = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    const catalog = new iceberg.Catalog(warehouse)

    // The explicit spelling: the schema is numbered here, and its partition
    // marks become the identity spec.
    const marked = fields
      .struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], { nullable: false })
      .withPartitionFields(['venue'])
    catalog.tables.create('nyc.trades', marked)

    const rows = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    const table = catalog.append('nyc.trades', rows([1n, 2n], ['XNAS', 'XNYS']))
    assert.equal(table.scan().intoTable().numRows, 2)
    assert.equal(catalog.append('nyc.trades', rows([3n], ['XNAS'])).scan().intoTable().numRows, 3)

    // The dotted name is the folder nyc/trades, and the marks became the spec.
    assert.ok(catalog.tables.has('nyc.trades'))
    assert.deepEqual(catalog.table('nyc.trades').spec.fields.map((field) => field.name), ['venue'])
    assert.deepEqual(catalog.namespaces.names(), ['nyc'])
    assert.deepEqual(catalog.namespaces.get('nyc').tables.names(), ['trades'])

    fs.rmSync(warehouse, { recursive: true, force: true })
    ```

`tables().create` is the explicit spelling - it numbers an unnumbered schema, derives the identity
spec from the schema's own [partition marks](types.md#a-field-can-be-a-partition-column), and
refuses a name that already has a table with a typed conflict. `append` and `overwrite` are
create-or-write; Rust spells the two `append_arrow_reader` and `overwrite_arrow_reader`, and takes
the per-call settings through `append_arrow_reader_with_options` and its overwrite twin.

Every write here - on a table, on the tables view, on the catalog - reads its rows through the same
inference point [the record surface uses](holder.md#arrow-batches), with the table's stored schema as
the declared field. Python therefore takes what `append_records` takes: a `RecordBatchReader`,
`Table`, `RecordBatch`, `Dataset` or `Scanner`, a `pandas` or `polars` frame including a
`LazyFrame`, an iterable of any of those, or an iterable of mappings and dataclass rows.
JavaScript takes what `BatchReader.from` takes plus the plain objects and field-class instances
`appendRecords` accepts. A create-on-write table has no schema to declare yet, and the rows are
then what name one. In every language the collections are the one spelling and the catalog keeps
exactly two dotted entry points - `table` and `namespace` - because a dotted identifier is a real
Iceberg spelling and deserves one call; there is no flat `create_table`/`has_table` surface beside
the views, because two spellings of one operation is the disease and a one-line delegate is still
a second spelling.

What is deliberately not here: `drop_table` and `rename_table`, because the storage contract has no
delete or move primitive, and a catalog must not emulate either by leaving a half-erased table
behind; and no catalog *service* client, because the module holds no network code. A REST catalog
is future work behind an HTTP storage backend.

#### The object model: namespaces of tables

A catalog is namespaces of tables, and each collection is its own type: `catalog.namespaces` is a
lazy view of the namespaces, indexing it answers a `Namespace`, and `namespace.tables` is the same
shape one level down, indexing to a `Table`. A nested namespace is reached through its parent's
`namespaces` view, so access chains - `catalog.namespaces["sales"].tables["orders"]` - and every
collection operation has exactly one home. The views are cheap handles, not caches: constructing
one performs no I/O, membership and iteration consult storage at the moment they are asked, two
views over the same catalog observe each other's writes, and a missing name is a `KeyError`
carrying the core's own absence message. JavaScript has no indexing hook a native class can
answer - no operator sugar exists - so the Map verbs are the spelling there: `get`, `has`, `size`,
`keys`, `values`, `entries`, `create`, `openOrCreate` over the same views, and `for...of` walks a
view's names lazily. Dotted names are resolved in the collections themselves -
`namespaces.get("sales.eu")` and `tables.get("sales.eu.orders")` descend - so the resolution rule
lives in one place, and `catalog.tables` is the same view at the warehouse root, where a fully
dotted name reaches any table in one lookup.

Iterating a collection is lazy in all three languages: the names arrive one at a time, `values` /
`items` / `entries` open one resource per step, and `len` / `size` drain the listing, so they cost
the full level. In Rust `get` returns `Result` and nothing implements `Index`: panic-on-missing is
normal for an in-memory child lookup and is not normal for a storage lookup - Python and
JavaScript get the map spelling their readers expect instead, and there is no `__delitem__`
anywhere because removal is deliberately absent from the hierarchy: the storage contract's
`remove` deletes a leaf or an empty container, and dropping a table is maintenance work, not a
`del`.

A catalog and a namespace each carry properties too, in one small metadata document apiece -
`metadata/catalog.json` under the warehouse, `metadata/namespace.json` under the namespace folder -
written through the shared JSON codec. Absent means empty properties, never an error; writing the
namespace document is also what makes an *empty* namespace durable, and what creates its ancestry.
The `iceberg:` property prefix is reserved for the format and refused by name. Above the warehouse
sits `Catalogs`, the same collection shape over a folder of warehouses, so
`catalogs.get("lake")?.namespaces()` addresses a lake without a caller-side convention (Rust-only
for now).

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::Catalog;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-views");
    let _ = std::fs::remove_dir_all(&root);
    let catalog = Catalog::new(Folder::new(&root)?);

    // Constructing the views touches nothing; every answer is storage's.
    let namespaces = catalog.namespaces();
    assert_eq!(namespaces.iter().count(), 0);
    let sales = namespaces.open_or_create("sales")?;
    assert!(!sales.tables().contains("orders")?);
    assert!(namespaces.contains("sales")?);

    // The namespace document is what makes the empty namespace durable, and
    // it is where its properties live.
    sales.update_properties([("region".to_owned(), "eu".to_owned())], [])?;
    assert_eq!(
        sales.properties()?.get("region").map(String::from),
        Some("eu".to_owned())
    );

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl.media.iceberg import Catalog

    warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "warehouse"
    catalog = Catalog(warehouse)

    # The views are lazy: an empty warehouse answers empty, touching nothing.
    assert len(catalog.namespaces) == 0
    sales = catalog.namespaces.open_or_create("sales")

    # The write conveniences create a table on first write, from the rows'
    # own schema; indexing chains a catalog to a namespace to a table.
    sales.tables.append("orders", pa.table({"id": [1, 2], "qty": [5.0, 2.5]}))
    assert "orders" in sales.tables
    assert list(sales.tables) == ["orders"]

    table = catalog.namespaces["sales"].tables["orders"]
    assert table.scan().read_all().num_rows == 2

    # The mapping surface: keys, values, and items, exactly as a dict's -
    # values and items are lazy iterators that open one table per step.
    # len drains the listing, so it costs the full level.
    assert list(sales.tables.keys()) == ["orders"]
    assert [name for name, _ in sales.tables.items()] == ["orders"]
    assert next(sales.tables.values()).scan().read_all().num_rows == 2
    assert len(sales.tables) == 1
    # There is no __delitem__: removal is absent from the whole hierarchy,
    # because the storage contract has no delete primitive to build it on.
    assert not hasattr(sales.tables, "__delitem__")

    # A catalog and a namespace each carry properties, in one small document.
    sales.update_properties({"region": "eu"})
    assert sales.properties == {"region": "eu"}

    shutil.rmtree(warehouse.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { iceberg } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    const catalog = new iceberg.Catalog(path.join(root, 'warehouse'))

    // The views are lazy: an empty warehouse answers empty, touching nothing.
    assert.equal(catalog.namespaces.size(), 0)
    const sales = catalog.namespaces.openOrCreate('sales')

    // The write conveniences create a table on first write, from the rows'
    // own schema; the views chain a catalog to a namespace to a table.
    sales.tables.append(
      'orders',
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        qty: arrow.vectorFromArray([5, 2.5], new arrow.Float64()),
      }),
    )
    assert.ok(sales.tables.has('orders'))
    assert.deepEqual(sales.tables.names(), ['orders'])
    // No operator sugar exists - JavaScript gives a native class no indexing
    // hook - so the Map verbs are the spelling: has, size, keys, values,
    // entries, and for...of. values and entries open one table per step.
    assert.deepEqual([...sales.tables.keys()], ['orders'])
    assert.deepEqual([...sales.tables], ['orders'])
    assert.deepEqual([...sales.tables.entries()].map(([name]) => name), ['orders'])
    assert.equal(sales.tables.size(), 1)

    const table = catalog.namespaces.get('sales').tables.get('orders')
    assert.equal(table.scan().intoTable().numRows, 2)

    // A catalog and a namespace each carry properties, in one small document.
    sales.updateProperties({ region: 'eu' })
    assert.deepEqual(sales.properties(), { region: 'eu' })

    // A nested namespace is reached through its parent's own view.
    sales.namespaces.create('eu')
    assert.deepEqual(catalog.namespaces.get('sales').namespaces.names(), ['eu'])

    // A missing name is refused naming it, never answered as an empty table.
    assert.throws(() => catalog.namespaces.get('marketing'), /marketing/)

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Data files aim at a size

!!! note "All three"
    The bindings read the target as `target_file_size` / `targetFileSize` and
    rewrite with `compact()`, which reports the same three numbers in each
    language's casing.

One key names the target: the table property `write.target-file-size-bytes`, falling back to the
schema root's `iceberg:write.target-file-size-bytes` protocol property, then Iceberg's 512 MiB
default. A partition's stream rolls to a new data file at the batch boundary that reaches the
target - sized by Arrow in-memory bytes, so Parquet's compression lands files under the target
rather than at it - and a table that has accumulated small files rewrites them:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{Catalog, FormatVersion};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let warehouse = Folder::temporary()?.path()?.join("yggdryl-doc-compaction");
    let _ = std::fs::remove_dir_all(&warehouse);
    let catalog = Catalog::new(Folder::new(&warehouse)?);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let one = |id: i64| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )
    };

    // Five appends, five snapshots, five small files.
    let mut table = catalog.tables().create("tiny.rows", schema)?;
    for id in 0..5 {
        let batch = one(id)?;
        table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    }
    assert_eq!(table.inspect_files()?.next().expect("one batch")?.num_rows(), 5);

    // Compaction rewrites the small groups as one replace commit and reports it.
    let compaction = table.compact()?;
    assert_eq!(compaction.files_before, 5);
    assert_eq!(compaction.files_after, 1);
    assert_eq!(table.scan(None)?.map(|batch| batch.map(|b| b.num_rows())).sum::<Result<usize, _>>()?, 5);

    // Nothing to do is a no-op that commits nothing.
    assert_eq!(table.compact()?, yggdryl::media::iceberg::Compaction::default());

    let _ = std::fs::remove_dir_all(&warehouse);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl.media.iceberg import Catalog

    warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "warehouse"
    catalog = Catalog(warehouse)

    # The default target is Iceberg's own 512 MiB.
    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = catalog.tables.create("tiny.rows", columns)
    assert table.target_file_size == 512 * 1024 * 1024

    # Five appends, five snapshots, five small files.
    for value in range(5):
        table.append(pa.record_batch({"id": [value]}, schema=columns))
    assert table.inspect_files().read_all().num_rows == 5

    # Compaction rewrites the small groups as one replace commit and reports it.
    compaction = table.compact()
    assert compaction.files_before == 5
    assert compaction.files_after == 1
    assert compaction.bytes_rewritten > 0
    assert table.scan().read_all().num_rows == 5

    # Nothing to do is a no-op that commits nothing.
    done = table.compact()
    assert (done.files_before, done.files_after, done.bytes_rewritten) == (0, 0, 0)

    shutil.rmtree(warehouse.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, iceberg } = require('yggdryl')

    const warehouse = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    const catalog = new iceberg.Catalog(warehouse)

    // The default target is Iceberg's own 512 MiB.
    const table = catalog.tables.create('tiny.rows', [Field.from('id: int64')])
    assert.equal(table.targetFileSize, 512 * 1024 * 1024)

    // Five appends, five snapshots, five small files.
    for (const value of [0n, 1n, 2n, 3n, 4n]) {
      table.append(new arrow.Table({ id: arrow.vectorFromArray([value], new arrow.Int64()) }))
    }
    assert.equal(table.inspectFiles().intoTable().numRows, 5)

    // Compaction rewrites the small groups as one replace commit and reports it.
    const compaction = table.compact()
    assert.equal(compaction.filesBefore, 5)
    assert.equal(compaction.filesAfter, 1)
    assert.ok(compaction.bytesRewritten > 0)
    assert.equal(table.scan().intoTable().numRows, 5)

    // Nothing to do is a no-op that commits nothing.
    const done = table.compact()
    assert.equal(done.filesBefore, 0)
    assert.equal(done.filesAfter, 0)
    assert.equal(done.bytesRewritten, 0)

    fs.rmSync(warehouse, { recursive: true, force: true })
    ```

Compaction groups live files by partition, touches only groups holding at least two files with one
under the target, and carries every other file into the new snapshot exactly as a merge carries the
files it never read. The snapshot before the compaction still time-travels: rewriting the present
never rewrites history.

### One options value, three layers

!!! note "All three"
    `IcebergOptions` is the same value in every language. Python passes it as
    `options=`; JavaScript builds it from a plain object and passes it as the
    trailing argument of calls that honour one.

Every knob a table honors lives on one value, `IcebergOptions`, and every field of it resolves
the same way: an explicit option set on the handle, then the table property of the same name
(falling back to the schema root's `iceberg:`-prefixed protocol property), then the documented
default. The keys are Iceberg's own spellings - `commit.retry.num-retries`,
`commit.retry.min-wait-ms`, `commit.retry.max-wait-ms`,
`commit.retry.total-timeout-ms`, `write.target-file-size-bytes`, `write.format.default`,
`read.parallelism`, `read.parallel.min-files`,
`read.parallel.min-file-size-bytes` - so a property another engine wrote configures this reader
too:

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-options");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;

    // Nothing set: every field answers its documented default.
    assert_eq!(table.options()?.commit_retries(), 4);
    assert_eq!(table.options()?.commit_total_timeout_ms(), 1_800_000);
    assert_eq!(table.options()?.target_file_size_bytes(), 512 * 1024 * 1024);

    // The property layer is the table's own metadata, one commit away.
    table.commit_metadata_changes(|metadata| {
        metadata.set_property(IcebergOptions::COMMIT_RETRIES_KEY, "9")?;
        Ok(())
    })?;
    assert_eq!(table.options()?.commit_retries(), 9);

    // An explicit override shadows the property on this handle alone;
    // nothing is written, and an unset field still resolves the other layers.
    table.set_options(IcebergOptions::new().with_commit_retries(2));
    assert_eq!(table.options()?.commit_retries(), 2);
    assert_eq!(table.options()?.commit_min_backoff_ms(), 100);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa
    from yggdryl import IOBase
    from yggdryl.media.iceberg import IcebergOptions, Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns)

    # Nothing set: every field answers its documented default.
    assert table.options().commit_retries == 4
    assert table.options().commit_total_timeout_ms == 1_800_000
    assert table.options().target_file_size == 512 * 1024 * 1024

    # The property layer is the table's own metadata, one commit away.
    table.update_properties({"commit.retry.num-retries": "9"})
    assert table.options().commit_retries == 9

    # An explicit override shadows the property on this handle alone; nothing
    # is written, and an unset field still resolves the other layers.
    table.set_options(IcebergOptions(commit_retries=2))
    assert table.options().commit_retries == 2
    assert table.options().commit_min_backoff_ms == 100

    # One options value configures this write and no later one.
    table.append(
        pa.record_batch({"id": [1]}, schema=columns),
        options=IcebergOptions(target_file_size=1 << 20),
    )
    assert table.options().target_file_size == 512 * 1024 * 1024

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, schema)

    // Nothing set: every field answers its documented default.
    assert.equal(table.options().commitRetries, 4)
    assert.equal(table.options().commitTotalTimeoutMs, 1_800_000)
    assert.equal(table.options().targetFileSize, 512 * 1024 * 1024)

    // The property layer is the table's own metadata, one commit away.
    table.updateProperties({ 'commit.retry.num-retries': '9' })
    assert.equal(table.options().commitRetries, 9)

    // An explicit override shadows the property on this handle alone; nothing
    // is written, and an unset field still resolves the other layers.
    table.setOptions(new iceberg.IcebergOptions({ commitRetries: 2 }))
    assert.equal(table.options().commitRetries, 2)
    assert.equal(table.options().commitMinBackoffMs, 100)

    // The trailing argument is the per-call layer: this write alone is sized.
    const rows = new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) })
    table.append(rows, new iceberg.IcebergOptions({ targetFileSize: 1 << 20 }))
    assert.equal(table.options().targetFileSize, 512 * 1024 * 1024)

    // A value the core refuses is refused at the boundary, naming it.
    assert.throws(() => new iceberg.IcebergOptions({ targetFileSize: 0 }))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A property that is present but does not parse is a typed error naming the key and the value, never
a silent default - and because an explicit option never reads the property it shadows, a broken
stored value can be shadowed first and repaired after, through the same handle. The resolvers are
also scoped to what each operation consults: a commit resolves only the four `commit.retry.*`
keys, so an unparseable `read.*` property cannot stop the metadata-only commit that fixes it.

In Python, `IcebergOptions` carries the whole surface. Each operation accepts one `options=`
value, and `set_options` changes the handle-wide override. A per-call value never mutates the
passed object or leaks into the handle. The generic `RecordOptions` is never accepted here:
Iceberg is a table format over the record encodings, and its configuration is its own.

JavaScript has no keyword arguments, so the value carries the whole surface instead: the
constructor takes an object naming any of the ten fields, every field is also a getter and a
setter, and a call that honours options takes one as its last argument -
`table.scan(field, options)`, `table.scanAt(id, filters, field, options)`,
`table.append(rows, options)`, `table.overwrite(rows, options)`, and the same trailing argument on
the tables view's `append` and `overwrite`. A per-call value is put back after the call, so it
never leaks into the handle's own override; `setOptions` is what changes that.

### The data-file MIME type

`data_mime_type` / `dataMimeType` accepts the generic `MimeType` or anything its
parser accepts, including `parquet`, `.avro`, and canonical MIME names.
`write.format.default` remains Iceberg's stored property key. Parquet is the
default; Avro is also writable. ORC and Puffin metadata are preserved, but a
write using either fails before consuming rows. Each manifest remains
authoritative, so mixed Parquet/Avro snapshots scan as one table.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{FormatVersion, IcebergOptions, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, MimeType};

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-data-format");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;

    // One Parquet append, then one Avro append via the explicit option.
    table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?;
    table.set_options(
        IcebergOptions::new().try_with_data_mime_type(MimeType::AVRO)?,
    );
    table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    // The manifest records what was written, and the mixed table scans whole.
    let mut formats: Vec<MimeType> = table
        .data_files()?
        .into_iter()
        .map(|(file, _)| file.mime_type)
        .collect();
    formats.sort();
    assert_eq!(formats, [MimeType::AVRO, MimeType::PARQUET]);
    assert_eq!(table.scan(None)?.count(), 2);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, MimeType
    from yggdryl.media.iceberg import IcebergOptions, Table, assign_field_ids

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = Table.create(
        IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"),
        assign_field_ids(schema),
    )

    # One Parquet append, then one Avro append.
    table.append(pa.table({"id": [1]}, schema=schema))
    table.append(
        pa.table({"id": [2]}, schema=schema),
        options=IcebergOptions(data_mime_type=MimeType.AVRO),
    )

    formats = sorted(file.mime_type for file, _ in table.data_files())
    assert formats == [MimeType.AVRO, MimeType.PARQUET]
    assert table.scan().read_all().num_rows == 2

    # Stored per table, the spec's own key configures every writer.
    table.update_properties({"write.format.default": "avro"})
    assert table.options().data_mime_type == MimeType.AVRO
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, MimeType, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, schema)

    const rows = (id) =>
      new arrow.Table({ id: arrow.vectorFromArray([id], new arrow.Int64()) })

    // One Parquet append, then one Avro append - the option is the trailing
    // argument every write already takes.
    table.append(rows(1n))
    table.append(rows(2n), new iceberg.IcebergOptions({ dataMimeType: MimeType.AVRO }))

    const formats = table.dataFiles().map((file) => file.mimeType.toString()).sort()
    assert.deepEqual(formats, [MimeType.AVRO.toString(), MimeType.PARQUET.toString()])
    assert.equal(table.scan().intoTable().numRows, 2)

    // Stored per table, the spec's own key configures every writer.
    table.updateProperties({ 'write.format.default': 'avro' })
    assert.ok(table.options().dataMimeType.equals(MimeType.AVRO))

    // Formats the build cannot encode are named before anything is written.
    assert.throws(
      () => table.append(rows(3n), new iceberg.IcebergOptions({ dataMimeType: MimeType.ORC })),
      /orc/i,
    )
    assert.throws(
      () => table.append(rows(3n), new iceberg.IcebergOptions({ dataMimeType: MimeType.PUFFIN })),
      /puffin/i,
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### Concurrent writers and commit retries

!!! note "Rust only"
    The commit gate is the core's, so every binding's writes retry through it
    and every binding sets the four `commit.retry.*` keys - the retry count,
    two backoff bounds, and total timeout. The race itself is shown once, in
    Rust, because staging it needs two handles and no rows.

Two writers holding the same table race the moment both commit, and what this module can promise
depends on what [`IOBase`](holder.md) offers: positional reads and writes, no compare-and-swap. So the
one commit gate every write goes through re-checks the current version before writing, counts each
newer version it finds as being *beaten* once, and retries with jittered exponential backoff up to
`commit.retry.num-retries` times and within the cumulative backoff budget named by
`commit.retry.total-timeout-ms`. What a retry does depends on the operation. An `append` and every
metadata-only `commit_metadata_changes` **rebase**: they
reload the winner's document and re-apply their
intent on it - the data files and the manifest of added entries are written once and reused, only
the manifest list and the document are rebuilt - so both writers' rows survive in one line of
history:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-concurrency");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    // Two handles opened at the same version, each unaware of the other.
    let mut left = Table::open(Folder::new(&root)?)?;
    let mut right = Table::open(Folder::new(&root)?)?;

    let arrow_schema = schema.into_arrow_schema()?;
    let one = |id: i64| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )
    };

    let batch = one(1)?;
    left.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    // The right handle is now stale; its commit observes the winner,
    // rebases onto it, and lands as the next version.
    let batch = one(2)?;
    right.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    // Both rows survive, on one line of history, and the rebased handle
    // is current: no re-open needed to see the winner's row.
    let rows: usize = right
        .scan(None)?
        .map(|batch| batch.map(|b| b.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 2);
    assert_eq!(right.inspect_history()?.next().expect("one batch")?.num_rows(), 2);

    let _ = std::fs::remove_dir_all(&root);
    ```

`overwrite`, `merge`, and `compact` never rebase: they planned against files the winner may have
replaced, and their input rows are already consumed, so re-applying could resurrect deleted data.
Beaten, they only wait, look again, and after exhausting the retries restore the in-memory state
and return a `CommitConflict` naming what happened - `expected to commit version 4, got beaten 5
times; last saw version 8` - so the caller re-plans against the table as it now is.

Honesty about the window: the check-then-write pair is not atomic. On plain storage a writer
landing between the check and the write goes undetected - retries shrink the window, they cannot
close it. Storage that serializes writers (an object store's atomic PUT, a catalog's swap) closes
it; `yggdryl::holder::local`'s memory mapping does not, and two processes truncating one mapped file at
the same instant is the documented SIGBUS hazard of that backend. A failed commit leaves no
visible change: at worst it leaves orphan data files no snapshot names.

### Branches and tags

!!! note "All three"
    The refs cross whole: `create_branch` / `createBranch`, `create_tag` /
    `createTag`, `remove_ref` / `removeRef`, `fast_forward` / `fastForward`,
    and `expire_snapshots` / `expireSnapshots` are the same five calls in each
    language's casing.

Named references are part of the metadata document: a **tag** is a name that never moves, a
**branch** is a name meant to. Creating one is a metadata-only commit, reading one is an ordinary
scan, and every ref keeps the snapshot it names retained past any expiry:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-branching");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.into_arrow_schema()?;
    let one = |id: i64| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )
    };

    let batch = one(1)?;
    table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    let audited = table.current_snapshot().expect("one commit").snapshot_id;

    // The tag pins the audited state; the table keeps moving.
    table.create_tag("audit-2026", audited)?;
    let batch = one(2)?;
    table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    table.create_branch("review", audited)?;

    // Every ref reads as the complete table it names.
    assert_eq!(table.scan_ref("audit-2026", &[], None)?.count(), 1);
    assert_eq!(table.scan_ref("review", &[], None)?.count(), 1);
    assert_eq!(table.scan(None)?.count(), 2);

    // A branch fast-forwards only along its own ancestry: the target must
    // reach the branch's head by parent ids, so no history can be lost.
    let head = table.current_snapshot().expect("two commits").snapshot_id;
    table.fast_forward_branch("review", head)?;
    assert_eq!(table.snapshot_by_ref("review")?.snapshot_id, head);

    // Removing a ref removes the name; the snapshots stay retained.
    let removed = table.remove_snapshot_ref("review")?;
    assert_eq!(removed.snapshot_id, head);

    // Expiry honors every ref's retention: the tagged snapshot survives
    // a cutoff that would otherwise expire everything old.
    assert!(table
        .expire_snapshots(Some(i64::MAX), None, &[])?
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns)

    table.append(pa.record_batch({"id": [1]}, schema=columns))
    audited = table.current_snapshot.snapshot_id

    # The tag pins the audited state; the table keeps moving.
    table.create_tag("audit-2026", audited)
    table.append(pa.record_batch({"id": [2]}, schema=columns))
    table.create_branch("review", audited)

    # Every ref reads as the complete table it names.
    assert table.scan_ref("audit-2026").read_all().num_rows == 1
    assert table.scan_ref("review").read_all().num_rows == 1
    assert table.scan().read_all().num_rows == 2

    # A branch fast-forwards only along its own ancestry: the target must reach
    # the branch's head by parent ids, so no history can be lost.
    head = table.current_snapshot.snapshot_id
    table.fast_forward("review", head)
    assert table.snapshot_by_ref("review").snapshot_id == head

    # Removing a ref removes the name; the snapshots stay retained, and a
    # second removal is refused rather than committing nothing.
    table.remove_ref("review")
    with pytest.raises(ValueError, match="review"):
        table.remove_ref("review")

    # Expiry honors every ref's retention: the tagged snapshot survives a
    # cutoff that would otherwise expire everything old.
    assert table.expire_snapshots(2**62) == []
    assert len(table.snapshots) == 2

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, schema)

    const rows = (id) =>
      new arrow.Table({ id: arrow.vectorFromArray([id], new arrow.Int64()) })

    table.append(rows(1n))
    const audited = table.currentSnapshot.snapshotId

    // The tag pins the audited state; the table keeps moving.
    table.createTag('audit-2026', audited)
    table.append(rows(2n))
    table.createBranch('review', audited)

    // Every ref reads as the complete table it names.
    assert.equal(table.scanRef('audit-2026').intoTable().numRows, 1)
    assert.equal(table.scanRef('review').intoTable().numRows, 1)
    assert.equal(table.scan().intoTable().numRows, 2)

    // A branch fast-forwards only along its own ancestry: the target must reach
    // the branch's head by parent ids, so no history can be lost.
    const head = table.currentSnapshot.snapshotId
    table.fastForward('review', head)
    assert.equal(table.snapshotByRef('review').snapshotId, head)

    // Removing a ref reports what it pointed at; the snapshots stay retained,
    // and a second removal is refused rather than committing nothing.
    assert.equal(table.removeRef('review').snapshotId, head)
    assert.throws(() => table.removeRef('review'), /review/)

    // Expiry honors every ref's retention: the tagged snapshot survives a
    // cutoff that would otherwise expire everything old.
    assert.deepEqual(table.expireSnapshots(Number.MAX_SAFE_INTEGER), [])
    assert.equal(table.snapshots.length, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

Each ref carries its own retention. Omitted cutoff and retain count resolve
from `history.expire.max-snapshot-age-ms` and
`history.expire.min-snapshots-to-keep`; per-ref settings override them.
Explicit snapshot ids join age selection but cannot remove retained heads.
`main` never expires, recent unreferenced snapshots survive until the cutoff,
and `gc.enabled=false` refuses the atomic update. Expired snapshots lose their
statistics descriptors, but physical file cleanup is separate. Ref changes use
the same retry gate as writes. Non-`main` branches are currently read with
`scan_ref` and moved with `fast_forward`, not written directly.

### Reading many files at once

!!! note "Rust only"
    The fan-out is inside the core scan, so every binding's scan gets it, and
    the three thresholds are ordinary option fields there -
    `read_parallelism` / `readParallelism` and their two neighbours. The
    demonstration is Rust because what it asserts is that the fan-out changes
    nothing observable.

A scan over many files can decode them in parallel, and the decision is deliberately conservative:
the fan-out starts only when `read.parallelism` is at least 2 **and** at least
`read.parallel.min-files` planned files (default 16) carry a recorded size of at least
`read.parallel.min-file-size-bytes` (default 4 MiB). Small reads never pay for threads they cannot
use, and storage is never hammered with more than `read.parallelism` files in flight - the default
is the host's own parallelism, clamped to 1..=8. The order is the plan's order either way:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-parallel-read");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.into_arrow_schema()?;
    for id in 0..3 {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )?;
        table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // Three tiny files sit below both thresholds, so this table reads
    // sequentially by default; forcing the thresholds down demonstrates
    // that the fan-out changes nothing the caller can observe.
    let sequential: Vec<RecordBatch> = table.scan(None)?.collect::<Result<_, _>>()?;
    table.set_options(
        IcebergOptions::new()
            .try_with_read_parallelism(2)?
            .with_read_parallel_min_files(1)
            .with_read_parallel_min_file_size_bytes(0),
    );
    let fanned: Vec<RecordBatch> = table.scan(None)?.collect::<Result<_, _>>()?;
    assert_eq!(sequential, fanned);

    let _ = std::fs::remove_dir_all(&root);
    ```

Each worker decodes one file end to end - the cast, the partition restore, and the residual
filters run on the worker, not the consumer - and a reorder buffer releases batches strictly in
plan order, admitting the next file only as the cursor file drains. Pruning still happens first:
a filtered scan fans out over the files the statistics could not exclude, not over the table. On
the benchmark table - 32 files of 100k rows each - four workers read a full collect about twice
as fast as one; the numbers live in `rust/benchmarks/media/iceberg.rs` under `read/`.

### The Spark quickstart, locally

!!! note "All three"
    The same walk now runs from Python and JavaScript: the catalog, the writes,
    the schema evolution, and the look back each have their three-language form
    in the sections above, so the quickstart itself is shown once, in Rust.

The scenario the [Spark quickstart](https://iceberg.apache.org/spark-quickstart/) walks - create
`nyc.taxis`, insert, read, update, delete, evolve, look back - runs against this module with no
Spark, no JVM, and no catalog service. A local folder is the whole warehouse.

```rust
use std::sync::Arc;

use arrow_array::{Float32Array, Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::media::iceberg::Table;
use yggdryl::holder::local::Folder;
use yggdryl::DataType;

let root = Folder::temporary()?.path()?.join("yggdryl-doc-nyc-taxis");
let _ = std::fs::remove_dir_all(&root);
let catalog = yggdryl::media::iceberg::Catalog::new(Folder::new(&root)?);

// CREATE TABLE nyc.taxis (...) PARTITIONED BY (vendor_id)
// The partition mark on the schema is the whole PARTITIONED BY clause.
let schema = DataType::from_fields([
    DataType::Int64.required_field("vendor_id"),
    DataType::Int64.required_field("trip_id"),
    DataType::Float32.nullable_field("trip_distance"),
    DataType::Float64.nullable_field("fare_amount"),
    DataType::Utf8.nullable_field("store_and_fwd_flag"),
])?
.required_field("row")
.with_partition_fields(&["vendor_id"])?;
let mut table = catalog.tables().create("nyc.taxis", schema.clone())?;
let schema = table.schema()?.clone();

// INSERT INTO nyc.taxis VALUES (...)
let arrow_schema = schema.into_arrow_schema()?;
let taxis = |vendors: &[i64], trips: &[i64], distances: &[f32], fares: &[f64], flags: &[&str]| {
    RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vendors.to_vec())),
            Arc::new(Int64Array::from(trips.to_vec())),
            Arc::new(Float32Array::from(distances.to_vec())),
            Arc::new(Float64Array::from(fares.to_vec())),
            Arc::new(StringArray::from(flags.to_vec())),
        ],
    )
};
let rows = taxis(
    &[1, 2, 2, 1],
    &[1_000_371, 1_000_372, 1_000_373, 1_000_374],
    &[1.8, 2.5, 0.9, 8.4],
    &[15.32, 22.15, 9.01, 42.13],
    &["N", "N", "N", "Y"],
)?;
table.commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))?;

// SELECT * FROM nyc.taxis
let fares = |table: &Table<Holder>| -> Result<Vec<(i64, f64)>, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();
    for batch in table.scan(None)? {
        let batch = batch?;
        let trips = batch.column_by_name("trip_id").expect("the trip column");
        let fares = batch.column_by_name("fare_amount").expect("the fare column");
        let trips = trips.as_any().downcast_ref::<Int64Array>().expect("int64");
        let fares = fares.as_any().downcast_ref::<Float64Array>().expect("float64");
        for row in 0..batch.num_rows() {
            rows.push((trips.value(row), fares.value(row)));
        }
    }
    rows.sort_by_key(|(trip, _)| *trip);
    Ok(rows)
};
assert_eq!(fares(&table)?.len(), 4);
assert_eq!(fares(&table)?[0], (1_000_371, 15.32));
let before_changes = table.current_snapshot().expect("the insert").snapshot_id;

// UPDATE nyc.taxis SET fare_amount = 16.32 WHERE trip_id = 1000371
// An update is a merge: the incoming row matches on the key and replaces.
let update = taxis(&[1], &[1_000_371], &[1.8], &[16.32], &["N"])?;
table.commit_merge(
    yggdryl::arrow::batch_reader(update.schema(), [update]),
    &["trip_id".to_owned()],
    true,
)?;
assert_eq!(fares(&table)?[0], (1_000_371, 16.32));
assert_eq!(fares(&table)?.len(), 4);

// DELETE FROM nyc.taxis WHERE vendor_id = 1
// A delete is a filtered overwrite with nothing incoming: the selected
// partition is replaced by no rows, and every other file is carried over.
table.commit_overwrite_where(
    &[("vendor_id", "1")],
    yggdryl::arrow::batch_reader(Arc::clone(&arrow_schema), []),
)?;
assert_eq!(
    fares(&table)?,
    [(1_000_372, 22.15), (1_000_373, 9.01)],
);

// ALTER TABLE nyc.taxis ADD COLUMN fare_per_distance float
let mut update = yggdryl::media::iceberg::SchemaUpdate::from_metadata(table.metadata())?;
update.add_column("", DataType::Float32.nullable_field("fare_per_distance"));
let evolved = update.into_field()?;
table.commit_metadata_changes(|metadata| {
    // The new column got the next unused id; a retired id is never reused.
    let schema_id = metadata.add_schema(evolved.clone())?;
    metadata.set_current_schema(schema_id)
})?;
let widened = table.scan(None)?.next().expect("one batch")?;
assert_eq!(widened.schema().fields().len(), 6);
assert_eq!(widened.column_by_name("fare_per_distance").expect("the new column").null_count(), 2);

// Time travel: the table before the update and the delete is still there.
assert_eq!(table.scan_at(before_changes, &[], None)?.map(|batch| batch.map(|b| b.num_rows())).sum::<Result<usize, _>>()?, 4);

// SELECT * FROM nyc.taxis.history / .snapshots / .files
let history = table.inspect_history()?.next().expect("one batch")?;
assert_eq!(history.num_rows(), 3);
let files = table.inspect_files()?.next().expect("one batch")?;
assert_eq!(files.num_rows(), 1);

let _ = std::fs::remove_dir_all(&root);
```

Every data-moving step above is one commit, so the history table ends with three rows - the insert,
the merge, the filtered overwrite - and the metadata-only schema change never appears there, because
it moved no data. The delete really is an overwrite: `vendor_id` is a partition column, so the plan selects one
partition's file, replaces it with nothing, and carries the other file into the new snapshot
untouched. And the time-travel read at the end sees the four original fares, because nothing a
commit writes is ever mutated in place.

### Schema evolution and field ids

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-evolution");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    // Add a column. Numbering continues above `last-column-id`, so the new column
    // can never be confused with a dropped one.
    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.add_column("", DataType::Int64.nullable_field("quantity"));
    let evolved = update.into_field()?;
    assert_eq!(table.evolve_schema(evolved)?, 1, "the new schema's id");

    // The old schema is retained, so the snapshot written under it still reads.
    assert_eq!(table.metadata().schemas().len(), 2);
    assert_eq!(table.metadata().schema_by_id(0).unwrap().field_len(), 1);

    // And the file written before the column existed reads it as null.
    for batch in table.scan(None)? {
        let batch = batch?;
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.column_by_name("quantity").unwrap().null_count(), batch.num_rows());
    }
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema)
    table.append(pa.record_batch({"id": [1]}, schema=columns))

    # Add a column. Numbering continues above `last-column-id`, so the new column
    # can never be confused with a dropped one.
    with table.update_schema() as update:
        update.add_column("", "quantity: int64")

    # The old schema is retained, so the snapshot written under it still reads.
    assert len(table.schemas) == 2
    assert len(table.schemas[0].dtype) == 1

    # And the file written before the column existed reads it as null.
    rows = table.scan().read_all()
    assert rows.column_names == ["id", "quantity"]
    assert rows.column("quantity").null_count == rows.num_rows
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema)
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }))

    // Add a column. Numbering continues above `last-column-id`, so the new column
    // can never be confused with a dropped one.
    const schemaId = table.updateSchema().addColumn('', 'quantity: int64').commit()
    assert.equal(schemaId, 1, "the new schema's id")

    // The old schema is retained, so the snapshot written under it still reads.
    assert.equal(table.schemas.length, 2)
    assert.equal(table.schemas[0].dtype.length, 1)

    // And the file written before the column existed reads it as null.
    const rows = table.scan().intoTable()
    assert.deepEqual(rows.schema.fields.map((child) => child.name), ['id', 'quantity'])
    assert.equal(rows.getChild('quantity').nullCount, rows.numRows)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

An Iceberg schema is a struct with *numbered* fields, and the number is the identity: a column read
by id survives a rename, and a new column can never reuse a retired id.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{assign_field_ids, last_column_id};
    use yggdryl::DataType;

    let leg = DataType::from_fields([DataType::decimal(18, 4)?.required_field("price")])?;
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        leg.nullable_field("leg"),
    ])?
    .required_field("row");

    // Depth first from `start`; the return value is the first id it did not use.
    assert_eq!(assign_field_ids(&mut schema, 1)?, 4);
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(schema.fields()[1].parquet_field_id()?, Some(2));
    assert_eq!(schema.fields()[1].fields()[0].parquet_field_id()?, Some(3));
    assert_eq!(last_column_id(&schema)?, 3, "what a table records as last-column-id");

    // The root is not a column, so it is not numbered.
    assert_eq!(schema.parquet_field_id()?, None);

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert_eq!(assign_field_ids(&mut schema, 100)?, 100);
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl.media.iceberg import assign_field_ids

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field(
            "leg",
            pa.struct([pa.field("price", pa.decimal128(18, 4), nullable=False)]),
        ),
    ])

    # Depth first from `start`; the numbered schema is what comes back, so the
    # schema handed in is left as it was.
    schema = assign_field_ids(columns, 1)
    assert [child.parquet_field_id for child in schema.dtype] == [1, 2]
    assert schema.dtype[1].dtype[0].parquet_field_id == 3

    # The root is not a column, so it is not numbered.
    assert schema.parquet_field_id is None

    # A field that already carries an id keeps it, so a second pass changes nothing.
    assert [child.parquet_field_id for child in assign_field_ids(schema, 100).dtype] == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, iceberg } = require('yggdryl')

    const leg = fields.struct('leg', [Field.from('price: decimal(18, 4)')])
    const plain = fields.struct('row', [Field.from('id: int64'), leg], { nullable: false })

    // Depth first from `start`; the numbered schema is what comes back, so the
    // schema handed in is left as it was.
    const schema = iceberg.assignFieldIds(plain)
    assert.equal(plain.dtype.getFieldAt(0).parquetFieldId, null)
    assert.equal(schema.dtype.getFieldAt(0).parquetFieldId, 1)
    assert.equal(schema.dtype.getFieldAt(1).parquetFieldId, 2)
    assert.equal(schema.dtype.getFieldAt(1).dtype.getFieldAt(0).parquetFieldId, 3)

    // The root is not a column, so it is not numbered.
    assert.equal(schema.parquetFieldId, null)

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert.equal(iceberg.assignFieldIds(schema, 100).dtype.getFieldAt(0).parquetFieldId, 1)
    ```

Creating and evolving a table numbers whatever arrives unnumbered, continuing above the highest id
already present, so the common path never spells numbering out. `assign_field_ids` remains for the
caller who needs the ids *before* the table exists - building a `PartitionSpec` by hand, or emitting
a schema document for another system. Because an existing id is preserved, the same call also fills
the gaps in a tree you extended, and the returned id is where the next call starts.

Emitting a schema *document* from a tree whose columns were never numbered still fails, because the
document's ids are the table's identity and inventing them silently would bind that identity to
chance; creating a table numbers first, which is why the same schema is fine there:

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::schema_into_json;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let message = schema_into_json(&schema).unwrap_err().to_string();
    assert!(message.contains("assign_field_ids"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    # A plain PyArrow schema carries no ids; creating the table numbers it.
    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = Table.create(IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"), columns)

    assert [child.parquet_field_id for child in table.schema.dtype] == [1]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, iceberg } = require('yggdryl')

    // A plain schema carries no ids; creating the table numbers it.
    const unnumbered = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, unnumbered)
    assert.equal(table.schema.dtype.getFieldAt(0).parquetFieldId, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### Evolving a schema

!!! note "All three"
    Python records the chain on `update_schema()` as a context manager,
    JavaScript on a builder ending in `commit()`, and `can_promote` /
    `canPromote` answers the promotion list everywhere.

A column change is a new schema, and `SchemaUpdate` is how one is built from the current one:
record the operations, apply, and commit the result. Only the promotions Iceberg allows are
accepted, so a change that would reinterpret stored values is refused naming both sides.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{can_promote, FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-evolution");
    let _ = std::fs::remove_dir_all(&root);
    let schema = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;

    // Legal promotions pass; anything else is refused naming both sides.
    assert!(can_promote(&DataType::Int32, &DataType::Int64).is_ok());
    assert!(can_promote(&DataType::decimal(10, 2)?, &DataType::decimal(18, 2)?).is_ok());
    let message = can_promote(&DataType::Int64, &DataType::Int32).unwrap_err().to_string();
    assert!(message.contains("int64") && message.contains("int32"));

    // Widen id, rename symbol, add venue - one evolved schema, one commit.
    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.update_type("id", DataType::Int64);
    update.rename_column("symbol", "ticker");
    update.add_column("", DataType::Utf8.nullable_field("venue"));
    let evolved = update.into_field()?;

    table.commit_metadata_changes(|metadata| {
        let schema_id = metadata.add_schema(evolved.clone())?;
        metadata.set_current_schema(schema_id)
    })?;

    let current = table.schema()?;
    assert_eq!(current.get_field_by_path("id").expect("the column").dtype(), &DataType::Int64);
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert_eq!(current.get_field_by_path("ticker").expect("the column").parquet_field_id()?, Some(2));
    assert_eq!(current.get_field_by_path("venue").expect("the column").parquet_field_id()?, Some(3));

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table, can_promote

    # Legal promotions pass; anything else is refused naming both sides.
    assert can_promote("int32", "int64") is None
    assert can_promote("decimal128(10, 2)", "decimal128(18, 2)") is None
    with pytest.raises(ValueError, match="int64 to int32"):
        can_promote("int64", "int32")

    columns = pa.schema([
        pa.field("id", pa.int32(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns)

    # Widen id, rename symbol, add venue - one evolved schema, one commit.
    with table.update_schema() as update:
        update.update_type("id", "int64").rename_column("symbol", "ticker")
        update.add_column("", "venue: string")

    children = list(table.schema.dtype)
    assert [child.name for child in children] == ["id", "ticker", "venue"]
    assert str(children[0].dtype) == "int64"
    # A renamed column keeps its identifier: the name is a label, the id is the column.
    assert [child.parquet_field_id for child in children] == [1, 2, 3]

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, iceberg } = require('yggdryl')

    // Legal promotions pass; anything else is refused naming both sides.
    iceberg.canPromote('int32', 'int64')
    iceberg.canPromote('decimal128(10, 2)', 'decimal128(18, 2)')
    assert.throws(() => iceberg.canPromote('int64', 'int32'), /int64 to int32/)

    const declared = fields.struct('row', [Field.from('id: int32'), Field.from('symbol: utf8')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, declared)

    // Widen id, rename symbol, add venue - one evolved schema, one commit.
    const schemaId = table
      .updateSchema()
      .updateType('id', 'int64')
      .renameColumn('symbol', 'ticker')
      .addColumn('', 'venue: utf8')
      .commit()
    assert.equal(schemaId, 1)

    const evolved = table.schema
    assert.deepEqual(Array.from(evolved.dtype, (child) => child.name), ['id', 'ticker', 'venue'])
    assert.equal(String(evolved.dtype.getFieldAt(0).dtype), 'int64')
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert.deepEqual(Array.from(evolved.dtype, (child) => child.parquetFieldId), [1, 2, 3])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

`TableMetadata` carries the rest of the update vocabulary - `set_property`/`remove_property`,
`set_location`, `assign_uuid`, `upgrade_format_version`, `set_snapshot_ref`/`remove_snapshot_ref`,
`remove_snapshots`, `add_spec`/`set_default_spec`, `add_sort_order`/`set_default_sort_order`.
Each operation goes through the official metadata builder before `commit_metadata_changes`
publishes it. Equivalent schemas, specs, and orders reuse the builder's canonical
identifier; conflicting requested identifiers are reassigned. Dropping a column
never frees its identifier.

### Schemas as documents

!!! note "All three"
    Both bindings read and write the document under the same two names, and
    both take it as the mapping their own JSON decoder produces.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{schema_from_json, schema_into_json};
    use yggdryl::{DataType};
    use yggdryl::text::json;

    let document = json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"symbol","required":false,"type":"string"}
        ]}"#,
    )?;

    // An Iceberg schema is a non-null struct field; its columns are the children.
    let schema = schema_from_json("row", &document)?;
    assert!(schema.is_struct());
    assert!(!schema.is_nullable());
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.fields()[0].dtype(), &DataType::Int64);

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert!(!schema.fields()[0].is_nullable());
    assert!(schema.fields()[1].is_nullable());
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(schema.fields()[0].get_metadata("PARQUET:field_id"), Some("1"));

    // The same document comes back out.
    assert_eq!(schema_into_json(&schema)?, document);
    ```

=== "Python"

    ```python
    import json

    from yggdryl.media.iceberg import schema_from_json, schema_into_json

    document = json.loads("""{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"},
        {"id":2,"name":"symbol","required":false,"type":"string"}
    ]}""")

    # An Iceberg schema is a non-null struct field; its columns are the children.
    schema = schema_from_json("row", document)
    assert schema.dtype.kind == "nested"
    assert not schema.nullable
    assert len(schema.dtype) == 2
    assert str(schema.dtype[0].dtype) == "int64"

    # `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert not schema.dtype[0].nullable
    assert schema.dtype[1].nullable
    assert schema.dtype[0].parquet_field_id == 1
    assert schema.dtype[0].metadata["PARQUET:field_id"] == "1"

    # The same document comes back out.
    assert schema_into_json(schema) == document
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { iceberg, json } = require('yggdryl')

    const document = json.loads(
      Buffer.from(`{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"},
        {"id":2,"name":"symbol","required":false,"type":"string"}
      ]}`),
    )

    // An Iceberg schema is a non-null struct field; its columns are the children.
    const schema = iceberg.schemaFromJson('row', document)
    assert.equal(schema.dtype.kind, 'nested')
    assert.equal(schema.nullable, false)
    assert.equal(schema.dtype.length, 2)
    assert.equal(String(schema.dtype.getFieldAt(0).dtype), 'int64')

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert.equal(schema.dtype.getFieldAt(0).nullable, false)
    assert.equal(schema.dtype.getFieldAt(1).nullable, true)
    assert.equal(schema.dtype.getFieldAt(0).parquetFieldId, 1)
    assert.equal(schema.dtype.getFieldAt(0).get('PARQUET:field_id'), '1')

    // The same document comes back out.
    assert.deepEqual(iceberg.schemaIntoJson(schema).asJs(), document)
    ```

There is no Iceberg schema type in this module. An Iceberg schema *is* a non-null struct
[`Field`](types.md) whose children carry `PARQUET:field_id`, so the two functions convert rather than
mirror: what comes back is a field the rest of the crate already reads, writes, casts, and projects
into [Arrow](arrow.md).

Documents enter and leave through the core [JSON](text.md) codec as ordinary
[`Scalar`](text.md) values. The official Iceberg model validates and
normalizes that document before Yggdryl projects it into `Field`.

Three things the field model spells differently, all of which survive the round trip:

- The root takes the `name` you pass, because an Iceberg schema names its columns but not itself.
- Iceberg states requirement and the core states nullability, so `"required": true` reads back as
  `is_nullable() == false` and writes back as `!field.is_nullable()`.
- `schema-id` is kept as `iceberg:schema-id` metadata on the root, a column's `doc` as `iceberg:doc`,
  and the v3 defaults as `iceberg:initial-default` and `iceberg:write-default`, which is why
  re-emitting the document reproduces it instead of dropping fields the field model has no slot for.

### Primitive types

!!! note "Rust only"
    The type mapping is a Rust table. The bindings see its result in the
    schema a table reports.

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::{DataType, TimeUnit, Timezone};

// Every Iceberg primitive name has exactly one physical datatype.
assert_eq!(PrimitiveType::from_str("long")?.into_dtype()?, DataType::Int64);
assert_eq!(PrimitiveType::from_str("string")?.into_dtype()?, DataType::Utf8);
assert_eq!(
    PrimitiveType::from_str("decimal(18, 4)")?.into_dtype()?,
    DataType::decimal(18, 4)?
);

// Iceberg fixed every temporal resolution at microseconds until v3 added the
// nanosecond pair.
assert_eq!(
    PrimitiveType::from_str("timestamp")?.into_dtype()?,
    DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }
);
assert_eq!(
    PrimitiveType::from_str("timestamp_ns")?.into_dtype()?,
    DataType::DateTime64 { unit: TimeUnit::Nanosecond, timezone: Timezone::NAIVE }
);
assert_eq!(
    PrimitiveType::from_str("time")?.into_dtype()?,
    DataType::time(TimeUnit::Microsecond)?
);

// A v3 `unknown` column always reads as null, which Arrow spells exactly.
assert_eq!(PrimitiveType::from_str("unknown")?.into_dtype()?, DataType::Null);

// A name round trips through `Display`.
assert_eq!(PrimitiveType::from_str("fixed[16]")?.to_string(), "fixed[16]");
```

`PrimitiveType` is the whole Iceberg type vocabulary, parsed from the spelling that appears in table
metadata JSON:

| Iceberg | `DataType` | Version |
| --- | --- | --- |
| `boolean` | `Boolean` | v1 |
| `int` | `Int32` | v1 |
| `long` | `Int64` | v1 |
| `float` | `Float32` | v1 |
| `double` | `Float64` | v1 |
| `decimal(p, s)` | `Decimal128 { precision: p, scale: s }` | v1 |
| `date` | `Date32` | v1 |
| `time` | `Time64(Microsecond)` | v1 |
| `timestamp` | `DateTime64 { unit: Microsecond, timezone: NAIVE }` | v1 |
| `timestamptz` | `DateTime64 { unit: Microsecond, timezone: UTC }` | v1 |
| `timestamp_ns` | `DateTime64 { unit: Nanosecond, timezone: NAIVE }` | v3 |
| `timestamptz_ns` | `DateTime64 { unit: Nanosecond, timezone: UTC }` | v3 |
| `string` | `Utf8` | v1 |
| `uuid` | `FixedSizeBinary(16)` | v1 |
| `fixed[n]` | `FixedSizeBinary(n)` | v1 |
| `binary` | `Binary` | v1 |
| `unknown` | `Null` | v3 |

`into_dtype` is total: every Iceberg type materializes without loss. `from_dtype` is not, and
that is the point - it names the datatype it refuses instead of widening it behind your back:

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::DataType;

// The variants that differ only in physical layout collapse onto one name.
assert_eq!(PrimitiveType::from_dtype(&DataType::Utf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_dtype(&DataType::LargeUtf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_dtype(&DataType::BinaryView)?, PrimitiveType::Binary);
assert_eq!(
    PrimitiveType::from_dtype(&DataType::decimal64(9, 2)?)?,
    PrimitiveType::Decimal { precision: 9, scale: 2 }
);

// A datatype Iceberg cannot express is reported, never approximated.
let message = PrimitiveType::from_dtype(&DataType::Int8).unwrap_err().to_string();
assert!(message.contains("int8"));
assert!(PrimitiveType::from_dtype(&DataType::Int16).is_err());

// A UUID is the core's own `uuid`, so the spelling survives the round trip
// in the datatype rather than in a marker beside the column.
assert_eq!(PrimitiveType::Uuid.into_dtype()?, DataType::Uuid);
assert_eq!(
    PrimitiveType::from_dtype(&PrimitiveType::Uuid.into_dtype()?)?.to_string(),
    "uuid"
);
```

`int8`, `uint32`, `interval`, `union`, `decimal256`, and any `time` or `timestamp` unit other than
microsecond and nanosecond have no Iceberg spelling, and this conversion refuses them rather than
widening them behind your back - the column type in the table has to be one you chose.

Choosing it is one call: `iceberg` is a
[schema-compatibility target](types.md#compatibility-rewriting) like `spark` and `polars`, so the
widenings that are lossless are named in one place and applied by the one recursive walker.

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::{DataType, Scheme};

// The narrow integers widen; the refusals stay refusals.
let widened = DataType::Int8.into_scheme_compat(&Scheme::ICEBERG)?;
assert_eq!(widened, DataType::Int32);
assert_eq!(PrimitiveType::from_dtype(&widened)?.to_string(), "int");
assert!(DataType::Interval(yggdryl::TimeUnit::YearMonth).into_scheme_compat(&Scheme::ICEBERG).is_err());
```

### Nested types

!!! note "Rust only"
    The type mapping is a Rust table. The bindings see its result in the
    schema a table reports.

```rust
use yggdryl::media::iceberg::{schema_from_json, schema_into_json};
use yggdryl::{DataType};
use yggdryl::text::json;

let document = json::from_utf8(
    r#"{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"legs","required":false,"type":{
            "type":"list","element-id":2,"element":{
                "type":"struct","fields":[
                    {"id":3,"name":"price","required":true,"type":"decimal(18, 4)"}
                ]
            },"element-required":true
        }},
        {"id":4,"name":"tags","required":false,"type":{
            "type":"map","key-id":5,"key":"string","value-id":6,"value":"int",
            "value-required":false
        }}
    ]}"#,
)?;

let schema = schema_from_json("row", &document)?;

// A list becomes a `List` whose item field is named `element` and carries `element-id`.
let legs = &schema.fields()[0];
let DataType::List(element) = legs.dtype() else { panic!("expected a list") };
assert_eq!(element.name(), "element");
assert_eq!(element.parquet_field_id()?, Some(2));
assert!(!element.is_nullable());
assert_eq!(element.fields()[0].name(), "price");

// A map becomes a `Map` over a non-null `entries` struct of `key` and `value`.
let tags = &schema.fields()[1];
let DataType::Map(map) = tags.dtype() else { panic!("expected a map") };
assert_eq!(map.entries().name(), "entries");
assert!(!map.entries().is_nullable());
assert!(!map.entries().fields()[0].is_nullable());
assert!(map.entries().fields()[1].is_nullable());
assert_eq!(map.entries().fields()[0].parquet_field_id()?, Some(5));

assert_eq!(schema_into_json(&schema)?, document);
```

`struct`, `list`, and `map` nest to any depth. The names `element`, `key`, `value`, and `entries`
are synthesized, because Iceberg numbers those positions instead of naming them: `element-id`,
`key-id`, and `value-id` become field ids on the fields the conversion builds. A map key is always
required, so only `element-required` and `value-required` read as nullability; both default to
required when absent.

### Into a data file

!!! note "Rust only"
    The bindings commit through a table's append and overwrite, which write
    the same files; the writer's own settings stay in Rust.

```rust
use arrow_array::RecordBatch;
use yggdryl::arrow;
use yggdryl::media::iceberg::schema_from_json;
use yggdryl::IOMedia;
use yggdryl::holder::Buffer;
use yggdryl::text::json;
use yggdryl::media::parquet::Parquet;

let document = json::from_utf8(
    r#"{"type":"struct","fields":[
        {"id":7,"name":"id","required":true,"type":"long"},
        {"id":8,"name":"symbol","required":false,"type":"string"}
    ]}"#,
)?;
let schema = schema_from_json("row", &document)?;

let mut media = Parquet::new(Buffer::new());
let options = media.record_options()?;
media.overwrite_arrow_reader(
    arrow::batch_reader(
        schema.into_arrow_schema()?,
        std::iter::empty::<RecordBatch>(),
    ),
    &options,
)?;

// The ids Iceberg assigned are the ids in the file.
let written = media.read_arrow_field(&options)?;
assert_eq!(written.fields()[0].parquet_field_id()?, Some(7));
assert_eq!(written.fields()[1].parquet_field_id()?, Some(8));
assert!(!written.fields()[0].is_nullable());
```

[parquet](media.md) has no Iceberg-specific code path. It writes `PARQUET:field_id` into the file
schema and reads it back, and because that is the same metadata key the conversion here uses, an
Iceberg schema needs no translation step before it becomes a data file - which is what lets a reader
resolve columns by id rather than by position.

### Interoperating with another implementation

A table format that only its own writer can read is not a table format. `python
scripts/check_iceberg_interop.py` runs the exchange in both directions against
[PyIceberg](https://py.iceberg.apache.org/): a partitioned v2 table written here is opened as a
PyIceberg `StaticTable` and compared column by column and row by row, and a table PyIceberg writes -
different metadata file names, different manifest field ordering, deflate-compressed Avro - is
opened by `Table::open` and compared the same way. `cargo test --features "parquet iceberg" --test
iceberg_interop` is the Rust half; run alone it says on stdout that it skipped the external table
rather than passing quietly.

Apache Spark, the format's reference implementation, gets the same treatment at a larger scale.
`python scripts/setup_spark_interop.py` provisions `pyspark` and the `iceberg-spark-runtime` jar,
and `pytest -m spark_interop` (in `python/tests/test_spark_interop.py`) then exchanges tables over
one shared Hadoop warehouse in both directions: creation and field ids, the primitive and nested
types with nulls, identity and transform partitioning, snapshots with time travel and refs, schema
evolution, table properties, Parquet and Avro data files including mixed-format tables, compaction,
the metadata tables, and the statistics renderings. The suite is deselected from the default test
run and skips itself, naming what is missing, when Java or Spark is absent.

Two behaviors here were arbitrated against the spec by that exchange and are deliberate:

- **Column resolution is by field id.** A data file written before a rename stores the column
  under its pre-rename name; the scan renames decoded columns to the current schema's names wherever the
  file's recorded field id matches, and a projected read pushes the file's *own* name down so the
  encoding still skips what it should. Names alone would silently null the column, which is what
  the spec's id-based resolution exists to prevent.
- **Transformed partition fields restore no column.** `days(at)` or `bucket(4, id)` store a
  derived value under a name that is not a schema column; the source column rides in the data file
  itself, exactly as Spark writes it, so only `identity` partition values are restored from the
  manifest.

Where Spark's SQL surface cannot express a spec type - `uuid`, `fixed`, `time` have no Spark DDL
spelling - the exchange covers the direction that exists, and the declared `uuid` spelling is
preserved through a metadata round trip rather than demoted to the physically identical
`fixed[16]`.

### What is not here

No remote catalog client or network transport. `yggdryl::media::iceberg::Catalog` is
an [`IOBase`](holder.md) warehouse view; commits publish through the supplied
handle.

Delete-file writing and row application are not implemented. Scans reject live
position/equality delete manifests with a typed unsupported error; they never
silently return undeleted rows. Manifests proven to contain no live delete
files are inert.

No writes to a branch other than `main`: a commit's parent is always the current snapshot, so a
branch is read with `scan_ref` and moved with `fast_forward` until commits learn to parent a
branch's head.

No compare-and-swap. The commit gate re-checks the current version and retries when beaten, but
`IOBase` cannot make check-then-write atomic, so the guarantee is honest best-effort on plain
storage and exact only where the storage itself serializes writers.
