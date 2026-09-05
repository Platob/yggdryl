# Iceberg writes

This page owns committing rows to an Iceberg table: the record methods, data-file sizing and compaction, `IcebergOptions`, the data-file format, commit retries, and refs.

## Contract

| Key | Rule |
| --- | --- |
| Owns | `append`, `overwrite`, `merge` commits; `compact`; `IcebergOptions`; `data_mime_type`; the commit gate; branches and tags |
| Commit | Every record call, `compact`, and ref change is one commit through one retry gate; a failed commit leaves no visible change |
| Reads | A table folder reads through the current snapshot; a replaced or uncommitted file is never read |
| Target size | `write.target-file-size-bytes`, then the root's `iceberg:write.target-file-size-bytes`, then 512 MiB; a file rolls at the batch boundary, sized by Arrow in-memory bytes |
| Options | Explicit handle option, then the table property of the same name (or the `iceberg:` root protocol property), then the documented default; `set_options` sets the handle layer |
| Retry defaults | `commit_retries` 4, `commit_min_backoff_ms` 100, `commit_total_timeout_ms` `1_800_000`; a commit resolves only the four `commit.retry.*` keys |
| Data format | `write.format.default`; Parquet by default, Avro writable; ORC and Puffin metadata are preserved but refused on write |
| Feature flag | `parquet iceberg` |
| Bindings | Python takes `options=` and never the generic [`RecordOptions`](../options.md); JavaScript takes a trailing `IcebergOptions`; Python and JavaScript tables keep their own scan and commit vocabulary, so the folder handle is their generic route |

## Use

The folder *is* the table, so the shared [record surface](../../holder/iobase/records.md) reaches it and each call is one commit.

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

## The three record methods over a table

A handle on a table folder is not a folder of Parquet files: it is read through the current snapshot.

| Call | Behavior |
| --- | --- |
| `read_arrow_reader` | Scans the current snapshot, planned as on [Iceberg reads](read.md) |
| `overwrite_arrow_reader` | Replaces every row |
| `merge_arrow_reader` | Requires match keys; reads only the data files whose recorded key bounds overlap the incoming keys; carries the rest with the same location, statistics, and commit order |
| `append_arrow_reader` | Writes new data files and keeps every manifest of the last snapshot; nothing stored is read or rewritten |

### The table value as a handle

A `Table` value is itself a handle and answers from the metadata it already holds; the folder route probes the location on every call. Its `kind` is [`IOKind::Table`](../../holder/index.md) rather than `Directory`, because the files below a table are its storage, not its contents.

Rust only.

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

### A partition directory as a handle

A handle on one `column=value` directory addresses that partition, with its files taken from the manifest rather than a directory listing.

Rust only.

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

## Data files aim at a size

The bindings read the target as `target_file_size` / `targetFileSize`, and Parquet compression lands files under it rather than at it. A table that has accumulated small files rewrites them with `compact()`, which reports the same three numbers in each language's casing.

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

## One options value, three layers

Every knob a table honors lives on `IcebergOptions`, and every field resolves through the same three layers. The keys are Iceberg's own spellings, so a property another engine wrote configures this writer too:

- `commit.retry.num-retries`, `commit.retry.min-wait-ms`, `commit.retry.max-wait-ms`, `commit.retry.total-timeout-ms`
- `write.target-file-size-bytes`, `write.format.default`
- `read.parallelism`, `read.parallel.min-files`, `read.parallel.min-file-size-bytes`

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

The JavaScript constructor takes an object naming any of the ten fields, and every field is also a getter and a setter. A per-call value never mutates the passed object and never leaks into the handle's own override.

| Binding | Per-call layer | Handle-wide override |
| --- | --- | --- |
| Python | `options=` on each operation | `set_options` |
| JavaScript | Trailing argument of `scan(field, options)`, `scanAt(id, filters, field, options)`, `append(rows, options)`, `overwrite(rows, options)`, and the tables view's `append` / `overwrite` | `setOptions` |

## The data-file MIME type

`data_mime_type` / `dataMimeType` accepts a `MimeType` or anything its parser accepts, such as `parquet`, `.avro`, or a canonical MIME name. Each manifest stays authoritative, so a snapshot mixing [Parquet](../parquet.md) and [Avro](../avro.md) files scans as one table.

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

## Concurrent writers and commit retries

[`IOBase`](../../holder/index.md) offers positional reads and writes and no compare-and-swap, so the commit gate re-checks the current version before writing. Each newer version it finds counts as being beaten once, and it retries with jittered exponential backoff up to `commit.retry.num-retries` times within `commit.retry.total-timeout-ms`.

| Operation | When beaten |
| --- | --- |
| `append`, `commit_metadata_changes` | Rebases: reloads the winner's document and re-applies the intent; the data files and the manifest of added entries are written once, only the manifest list and the document are rebuilt |
| `overwrite`, `merge`, `compact` | Never rebases: waits, looks again, and after the retries restores the in-memory state and returns `CommitConflict` |

Rust only.

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

## Branches and tags

A tag is a name that never moves; a branch is a name meant to. Creating one is a metadata-only commit, reading one is an ordinary scan, and every ref keeps the snapshot it names retained past any expiry.

| Rust | Python | JavaScript |
| --- | --- | --- |
| `create_tag` | `create_tag` | `createTag` |
| `create_branch` | `create_branch` | `createBranch` |
| `scan_ref`, `snapshot_by_ref` | `scan_ref`, `snapshot_by_ref` | `scanRef`, `snapshotByRef` |
| `fast_forward_branch` | `fast_forward` | `fastForward` |
| `remove_snapshot_ref` | `remove_ref` | `removeRef` |
| `expire_snapshots` | `expire_snapshots` | `expireSnapshots` |

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

## Edges

- `merge_arrow_reader` without match keys -> refused; a merge requires `merge_by_names`.
- A partition filter naming a column the schema does not declare, on a `Table` handle -> error; a folder of leaves ignores a column its batches do not carry.
- A stray file nobody committed, or a file an overwrite replaced -> never read.
- `compact()` -> touches only partition groups holding at least two files with one under the target; every other file is carried untouched, and the prior snapshot still time-travels.
- `compact()` with nothing to do -> no-op that commits nothing; all three numbers are zero.
- A table property that is present but does not parse -> typed error naming the key and the value, never a silent default; an explicit option shadows it, so it can be repaired through the same handle.
- An unparseable `read.*` property -> cannot stop the metadata-only commit that fixes it, because a commit resolves only the four `commit.retry.*` keys.
- `new iceberg.IcebergOptions({ targetFileSize: 0 })` -> refused at the boundary, naming the value.
- `data_mime_type` of ORC or Puffin -> the write fails before consuming rows.
- Beaten `overwrite`, `merge`, or `compact` after the retries -> `CommitConflict` naming what happened: `expected to commit version 4, got beaten 5 times; last saw version 8`; re-plan against the table as it now is.
- The check-then-write pair is not atomic -> on plain storage a writer landing between them goes undetected; retries shrink the window, storage that serializes writers (an object store's atomic PUT, a catalog's swap) closes it.
- Two processes truncating one memory-mapped file at the same instant -> the documented SIGBUS hazard of [`yggdryl::holder::local`](../../holder/backends/local.md).
- A failed commit -> no visible change; at worst orphan data files no snapshot names.
- A branch fast-forward -> only along its own ancestry; the target must reach the branch's head by parent ids.
- Removing a ref -> removes the name only, the snapshots stay retained; a second removal is refused naming the ref (Python `ValueError`, JavaScript throws) rather than committing nothing.
- `expire_snapshots` with cutoff or retain count omitted -> resolved from `history.expire.max-snapshot-age-ms` and `history.expire.min-snapshots-to-keep`; per-ref settings override them.
- Explicit snapshot ids -> join age selection but cannot remove retained heads; `main` never expires; recent unreferenced snapshots survive until the cutoff.
- `gc.enabled=false` -> the atomic expiry update is refused.
- Expired snapshots -> lose their statistics descriptors; physical file cleanup is separate.
- Ref changes -> the same retry gate as writes.
- Writing to a non-`main` branch directly -> not supported; read it with `scan_ref` and move it with `fast_forward`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::handles
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::concurrency_and_compaction
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::data_mime_type
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::line_projection
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^compact/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^merge/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^commit/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_iceberg.py python/tests/media/test_iceberg_planning.py
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/iceberg.test.js
    YGGDRYL_BENCH_FILTER=iceberg/append npm run --prefix node bench:media
    ```
