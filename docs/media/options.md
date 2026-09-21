# RecordOptions

`IORecordOptions` is every encoding's settings surface; `RecordOptions` is the enum naming one encoding's options.

## Contract

| key | value |
| --- | --- |
| Owns | `IORecordOptions`, `RecordOptions`, each encoding's options struct |
| Sections | the split sections of one [plan](../expression/plans.md), stored apart: `field` (the `create` section; none = inferred), `filter` (`where`; always true), `select` (`select`; `*`), `merge_by` (`upsert by`; empty), `name` (`"row"`, `media::DEFAULT_ROOT_NAME`), and `max_row_size` (`limit`) |
| Plan | `plan()` composes the sections into one `Plan`; `set_plan` / `with_plan` split a `Plan`, a clause, a `Field`, or text back into them |
| Scalars | `set_select_scalar`, `set_filter_scalar`, `set_merge_by_scalar`, `set_plan_scalar` and their `with_` forms read the section from one `Scalar` as `from_scalar` does - text, a sequence, a mapping, null - the one door every binding's setter crosses |
| Properties | every record read and write takes the options and, beside them, properties by name (`**properties`, a plain object) set on a copy by their own setters; `...` / `undefined` skipped, `None` / `null` clear |
| Shared fields | `name`, `field`, `filter`, `select`, `merge_by`, `safe`, `batch_row_size`, `batch_byte_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, `level` |
| Pruning | `partition_pairs()` reads the equalities the filter pins, as partition paths spell them; a media prunes by them and lets the rest of the predicate run over the rows |
| Identity | `Clone`, `Eq`, `Ord`, `Hash` include the variant; `stable_hash()` is run-stable over that variant's full configuration |
| `batch_row_size` | rows per batch; [`pstream_bytes`](../holder/iobase/bytes.md) `batch_size` counts bytes |
| `batch_byte_size` | Arrow in-memory bytes per batch, whichever of it and `batch_row_size` binds first; a target rather than a ceiling, and a non-zero bound yields at least one row; Rust only |
| `commit_row_size` | unset publishes once; `N` publishes `N`-row prefixes plus the remainder |
| Derivation | `for_media_type` reads the base type only |
| Bindings | `RecordOptions` in Python and JavaScript; encoding structs stay in Rust |

## Use

The media type names the encoding, so no format argument is passed.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_row_size(1024);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), "row");
    assert!(options.select().is_all());
    assert!(options.filter().is_always_true());
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
    assert [child.name for child in options.field.dtype] == ["id"]
    assert options.select.is_all
    assert options.filter.is_always_true
    assert options.batch_row_size == 1024
    assert options.commit_row_size == 10_000

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

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.equal(options.name, 'row')
    assert.ok(options.field.equals(schema))
    assert.ok(options.select.isAll)
    assert.ok(options.filter.isAlwaysTrue)
    assert.equal(options.batchRowSize, 1024)

    // A setting one encoding has reads as null on an encoding that has none.
    assert.equal(options.maxRowGroupSize, 1_048_576)
    const stream = RecordOptions.from('trades.arrows')
    assert.equal(stream.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    assert.equal(stream.maxRowGroupSize, null)

    // `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert.equal(stream.level, 9)

    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(true).safe, true)
    assert.equal(options.safe, false)
    ```

## The sections of one plan

`plan()` spells the options as one [plan](../expression/plans.md), and `set_plan` reads one back into the sections.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, StructType};

    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("trade");
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
        .with_field(schema.clone())
        .with_filter("venue = 'XNAS' and id > 5")?
        .with_select("id")?
        .with_merge_by("id")?
        .with_max_row_size(10);

    // One plan: create, upsert by, select, where, limit.
    let plan = options.plan();
    assert_eq!(
        plan.to_string(),
        "create trade (id int64 not null, venue utf8 null) upsert by (id) select id \
         where venue = 'XNAS' and id > 5 limit 10",
    );
    assert_eq!(plan.field()?, Some(schema.clone()));

    // The equalities the filter pins are what prune a listing before anything is opened.
    assert_eq!(options.partition_pairs(), [("venue".to_owned(), "XNAS".to_owned())]);

    // The same plan splits back into the same sections.
    let read_back = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_plan(plan.to_string())?;
    assert_eq!(read_back, options);
    assert_eq!(read_back.field(), Some(schema));
    assert_eq!(read_back.max_row_size(), Some(10));

    // A renamed root is the same declaration under another name.
    let mut renamed = options.clone();
    renamed.set_name("row".into());
    assert_eq!(renamed.field().unwrap().name(), "row");
    ```

=== "Python"

    ```python
    from yggdryl import Field, RecordOptions, Selector

    schema = Field("trade", "struct<id: int64 not null, venue: utf8>", nullable=False)
    options = RecordOptions("trades.arrows")
    options.field = schema
    options.filter = "venue = 'XNAS' and id > 5"
    options.select = ["id"]
    options.merge_by = ["id"]
    options.max_row_size = 10

    # One plan: create, upsert by, select, where, limit.
    plan = options.plan
    assert str(plan) == (
        "create trade (id int64 not null, venue utf8 null) upsert by (id) select id "
        "where venue = 'XNAS' and id > 5 limit 10"
    )
    assert plan.field() == schema
    assert plan.merge_by == Selector("id")

    # The equalities the filter pins are what prune a listing before anything is opened.
    assert options.partition_pairs() == [("venue", "XNAS")]

    # The same plan splits back into the same sections.
    read_back = RecordOptions("trades.arrows")
    read_back.plan = str(plan)
    assert read_back == options
    assert read_back.field == schema
    assert read_back.max_row_size == 10

    # A renamed root is the same declaration under another name; None clears it.
    options.name = "row"
    assert options.field.name == "row"
    options.field = None
    assert options.field is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions } = require('yggdryl')

    const schema = Field.from('trade: struct<id: int64 not null, venue: utf8> not null')
    const options = new RecordOptions('trades.arrows')
      .withField(schema)
      .withFilter("venue = 'XNAS' and id > 5")
      .withSelect(['id'])
      .withMergeBy(['id'])
      .withMaxRowSize(10)

    // One plan: create, upsert by, select, where, limit.
    const plan = options.plan
    assert.equal(
      plan.toString(),
      'create trade (id int64 not null, venue utf8 null) upsert by (id) select id ' +
        "where venue = 'XNAS' and id > 5 limit 10",
    )
    assert.ok(plan.field().equals(schema))
    assert.ok(plan.mergeBy.equals('id'))

    // The equalities the filter pins are what prune a listing before anything is opened.
    assert.deepEqual(options.partitionPairs(), [['venue', 'XNAS']])

    // The same plan splits back into the same sections.
    const readBack = new RecordOptions('trades.arrows').withPlan(plan.toString())
    assert.ok(readBack.equals(options))
    assert.ok(readBack.field.equals(schema))
    assert.equal(readBack.maxRowSize, 10)

    // A renamed root is the same declaration under another name; null clears it.
    options.name = 'row'
    assert.equal(options.field.name, 'row')
    options.field = null
    assert.equal(options.field, null)
    ```

## Shaping

`apply_arrow_batch` and `apply_arrow_reader` [apply](../types/field.md#applying-a-schemas-declarations) the declared schema, then run the `where` and `select` sections, then apply the optional `existing` root.

A field shapes rows by applying, not by casting: a declaration is the cast *and* the `TRANSFORM:` and `DIGEST:` columns it derives, so a declared derived column arrives written rather than arriving as the default nothing filled. The selection after it only narrows, because deriving there would restore the columns it was asked to drop. A root declaring no derivation applies as the cast alone, at the safety `safe` names; the `existing` completion is always safe.

Rust; Python binds the same `RecordOptions.apply_arrow_batch` / `apply_arrow_reader`, and JavaScript binds neither.

```rust
use arrow_array::RecordBatch;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, MimeType, StructType};

let declared = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("symbol"),
    DataType::Int64.required_field("price"),
])?)
.required_field("row");

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
    .with_field(declared.clone())
    .with_select("price")?;

// One call is the whole pipeline: the declared apply, then the selection.
// Passing a stored root as the second argument adds the completion layer.
let batch = RecordBatch::new_empty(declared.into_arrow_schema()?);
let shaped = options.apply_arrow_batch(batch, None)?;
assert_eq!(shaped.num_columns(), 1);
```

## Shared settings

Each encoding stores the settings as flat public fields and implements `IORecordOptions` over them.

Rust only.

```rust
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::ipc::IpcOptions;
use yggdryl::{Level, MimeType};

let mut ipc = IpcOptions::new()
    .with_name("trade")
    .with_safe(false)
    .with_level(Level::BEST);
// The fields are public, so a setting can also be written directly.
ipc.commit_row_size = Some(10_000);
ipc.batch_byte_size = Some(1 << 20);
assert_eq!(ipc.level(), Level::BEST);

// It converts into the enum every encoding's settings share.
let options: RecordOptions = ipc.into();
assert_eq!(options.mime_type(), MimeType::ARROW_STREAM);
assert_eq!(options.name(), "trade");
assert!(!options.safe());
assert_eq!(options.level(), Level::BEST);
assert_eq!(options.commit_row_size(), Some(10_000));
assert_eq!(options.batch_byte_size(), Some(1 << 20));
```

## Requiring a datatype

`require_field` is what a write calls, and a datatype is the one part with no default.

Rust; Python binds the same `RecordOptions.require_field`, and JavaScript does not.

```rust
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::MimeType;

let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?;
assert!(options.field().is_none());

let message = options.require_field().unwrap_err().to_string();
assert!(message.contains("with_field"), "{message}");
```

## Edges

- No `field` -> the shape is inferred, `plan()` has no `create` section, and `require_field` errors naming `with_field`.
- `set_field` / `with_field` -> nullability and dictionary options dropped; `set_name` renames the declared field in place.
- `take_field` -> clears the declaration, keeps `name`.
- `with_plan(f)` for a `Field` `f` -> equal, and hash-equal, to `with_field(f)`.
- `set_plan` -> replaces every section the plan spells and clears the ones it does not; a `limit` sets `max_row_size`, and the plan's targets and source are not read, because the handle these options are given to is both.
- `merge_by` naming a column twice -> refused naming it; `max_row_size` with `merge_by` -> refused naming both.
- `select` naming a column the stored root lacks -> the encoding reads everything and the cast supplies it as nulls.
- `existing` root -> the cast is always safe; an unconvertible value becomes null.
- Every read and write path -> routes through `apply_arrow_batch` / `apply_arrow_reader`, so declaration, derivation, selection, and stored shape agree.
- Unused setting -> still there, still ignored, like [`ParquetOptions::level`](parquet/compression.md).
- Content coding -> ignored, the derivation [`IOMedia::record_options`](index.md) also performs.
- `max_row_group_size` on `trades.arrows` -> `None` in Python, `null` in JavaScript.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::options::tests
    cargo test --features "parquet iceberg" -p yggdryl --test media -- inference::
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
