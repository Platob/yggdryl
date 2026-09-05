# RecordOptions

`IORecordOptions` is every encoding's settings surface; `RecordOptions` is the enum naming one encoding's options.

## Contract

| key | value |
| --- | --- |
| Owns | `IORecordOptions`, `RecordOptions`, each encoding's options struct |
| Root parts | `name` (`"row"`, `media::DEFAULT_ROOT_NAME`), `dtype` (none), `metadata` (empty) |
| Shared fields | `name`, `dtype`, `metadata`, `safe`, `batch_row_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, `level`, `merge_by_names`, `select_by_names`, `filter_partitions` |
| Identity | `Clone`, `Eq`, `Ord`, `Hash` include the variant; `stable_hash()` is run-stable |
| `batch_row_size` | rows per batch; [`pstream_bytes`](../holder/iobase/bytes.md) `batch_size` counts bytes |
| `commit_row_size` | unset publishes once; `N` publishes `N`-row prefixes plus the remainder |
| Derivation | `for_media_type` reads the base type only |
| Bindings | `RecordOptions` in Python and JavaScript; encoding structs stay in Rust |

## Use

The media type names the encoding, so no format argument is passed.

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

## Declared root

`field()` builds the non-null Struct root on every ask, so no part is ever stale against it.

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

## Casting

`cast_arrow_batch` and `cast_arrow_reader` apply the declared schema, then `select_by_names`, then the optional `existing` root. Every write path routes through this definition.

Rust only.

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

## Shared settings

Each encoding stores the settings as flat public fields and implements `IORecordOptions` over them.

Rust only.

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

## Requiring a datatype

`require_field` is what a write calls, and a datatype is the one part with no default.

Rust only.

```rust
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::MimeType;

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?;
assert!(options.field().is_none());

let message = options.require_field().unwrap_err().to_string();
assert!(message.contains("with_field"), "{message}");
assert!(message.contains("with_dtype"), "{message}");
```

## Edges

- No `dtype` -> `field()` is nothing; `require_field` errors naming `with_field` and `with_dtype`.
- `metadata` without a `dtype` -> never reaches a read or write.
- `set_field` / `with_field` -> nullability and dictionary options dropped.
- `take_field` -> clears `dtype` and `metadata`, keeps `name`.
- `existing` root -> the cast is always safe; an unconvertible value becomes null.
- Unused setting -> still there, still ignored, like [`ParquetOptions::level`](parquet.md).
- Content coding -> ignored, the derivation [`IOMedia::record_options`](index.md) also performs.
- `max_row_group_size` on `trades.arrows` -> `None` in Python, `null` in JavaScript.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::options::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib media::inference::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- 'io_dimensions/.*/record_options'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_records
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_mode_dispatch
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_commit_rows
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_io_records.py python/tests/media/test_commit_row_size.py
    python/.venv/bin/python python/benchmarks/media.py --filter "record options"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    YGGDRYL_BENCH_FILTER=records/record_options npm run --prefix node bench:media
    ```
