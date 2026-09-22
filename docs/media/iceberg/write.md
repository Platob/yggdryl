# Iceberg writes

Rows into a table - as native scalars, as Arrow batches - with the intent in the method name and one commit per call.

## Contract

| Key | Rule |
| --- | --- |
| Native rows | `overwrite_records`, `append_records`, `merge_records` over the table or its folder; Python and JavaScript hand plain rows straight to `Table.overwrite` / `append` / `merge`, typed against the stored schema |
| Owns | `append`, `overwrite`, `merge` commits; `compact`; `IcebergOptions`; `data_mime_type`; the commit gate; branches and tags |
| Commit | Every record call, `compact`, and ref change is one commit through one retry gate, publishing one new [metadata document](metadata.md); a failed commit leaves no visible change |
| Reads | A table folder reads through the current snapshot; a replaced or uncommitted file is never read |
| Target size | `write.target-file-size-bytes`, then the root's `ICEBERG:write.target-file-size-bytes`, then 512 MiB; a partition group is cut into files of about the target, measured as its Arrow in-memory bytes per row |
| Keys | The identity partition columns lead every merge key, once each, then `merge_by`; a merge naming no key replaces the partitions its rows fall in; a merge reads and rewrites only the files of the partitions its rows fall in |
| Sort | Every data file holds one partition, sorted by the table's default sort order: [`SortOrder::for_spec`](#sorted-data-files) - the spec's source columns ascending, nulls first - unless `create_sorted` declared another; order 0 is unsorted |
| Parallel writes | Partition groups are written on `write.parallelism` threads (default: the resolved `read.parallelism`); the manifest lists files in group order; a failing group fails the commit before any metadata is written |
| Staging | `write.staging` = `off` or a local folder (default: the platform temporary folder for a remote root, `off` for a local one); every data file, manifest and manifest list is encoded into a staging file under a directory of the commit's own and uploaded once - multipart above the store's threshold - with its statistics read from the staged copy; the directory goes when the commit ends, and a failed commit removes every file it published |
| Remote calls | Over an object store an append of one partition is 9 requests, an upsert into one partition of three 14, a full scan of four files 7, a pruned scan of one file 3: the metadata chain, one `GET` per data file, one upload per written file, and the one listing that claims the version - never a listing of `data/`, a `HEAD` for a size, or a footer read back from the store ([Object stores](../../holder/backends/s3.md#what-an-iceberg-table-costs)) |
| Options | Explicit handle option, then the table property of the same name (or the `ICEBERG:` root protocol property), then the documented default; `set_options` sets the handle layer |
| Retry defaults | `commit_retries` 4, `commit_min_backoff_ms` 100, `commit_total_timeout_ms` `1_800_000`; a commit resolves only the four `commit.retry.*` keys |
| Data format | `write.format.default`; Parquet by default, Avro writable; ORC and Puffin metadata are preserved but refused on write |
| Feature flag | `parquet iceberg` |
| Bindings | Python takes `options=` and never the generic [`RecordOptions`](../options.md); JavaScript takes a trailing `IcebergOptions`; Python and JavaScript tables keep their own scan and commit vocabulary, so the folder handle is their generic route |

## Use

Native rows first - a tuple, a mapping, a plain object - typed against the schema the table already stores, so no row carries an Arrow holder or a schema of its own. Each call is one commit.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, StructType};

    struct Quote(i64, &'static str);

    impl From<Quote> for Scalar {
        fn from(row: Quote) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
        }
    }

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-native-write");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema, spec)?;

    // The table's stored schema declares the rows, so a native row is an ordered
    // sequence under it and carries no schema of its own.
    let base = table.record_options()?;
    let options = base.clone().with_field(table.read_arrow_field(&base)?);
    table.overwrite_records([Quote(1, "XNAS"), Quote(2, "XNYS")], &options)?;
    table.append_records([Quote(3, "XLON")], &options)?;

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    let merging = options.clone().with_merge_by("id")?;
    table.merge_records([Quote(2, "XNYS"), Quote(9, "XLON")], &merging)?;

    let total: usize = table
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);

    // Each call was one commit, and the table value followed them without
    // reopening anything.
    assert_eq!(table.metadata().snapshots().len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])

    path = pathlib.Path(tempfile.mkdtemp()) / "trades"
    table = Table.create(IOBase(path), columns, ["venue"])

    # Plain rows type against the table's stored schema, so they need no Arrow
    # holder and no schema of their own.
    table.overwrite([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": "XNYS"}])
    table.append([{"id": 3, "venue": "XLON"}])

    # A match key upserts: `2` is stored and updates, `9` is new and appends.
    table.merge([{"id": 2, "venue": "XNYS"}, {"id": 9, "venue": "XLON"}], ["id"])

    assert table.scan().read_all().num_rows == 4

    # Each call was one commit, and the read went through the last one.
    assert len(table.snapshots) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8?')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.Table.create(root, schema, ['venue'])

    // Plain objects are rows, typed against the table's stored schema.
    table.overwrite([{ id: 1n, venue: 'XNAS' }, { id: 2n, venue: 'XNYS' }])
    table.append([{ id: 3n, venue: 'XLON' }])

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    table.merge([{ id: 2n, venue: 'XNYS' }, { id: 9n, venue: 'XLON' }], ['id'])

    assert.equal(table.scan().intoTable().numRows, 4)

    // Each call was one commit, and the read went through the last one.
    assert.equal(table.snapshots.length, 3)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Rows as Arrow batches

The folder *is* the table, so the shared [record surface](../../holder/iobase/records.md) reaches it and each call is one commit.

=== "Rust"

    ```rust
    use yggdryl::media::IORecordOptions;
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-records");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

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
    let mut folder = LocalFolder::new(&path)?;
    let options = folder.record_options()?;
    folder.overwrite_arrow_reader(rows(vec![1, 2], vec!["XNAS", "XNYS"]), &options)?;
    folder.append_arrow_reader(rows(vec![3], vec!["XLON"]), &options)?;

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    let merging = options.clone().with_merge_by("id")?;
    folder.merge_arrow_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;

    let total: usize = folder
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);

    // Each call was one commit, and the read went through the last one.
    let table = Table::open(LocalFolder::new(&path)?)?;
    assert_eq!(table.metadata().snapshots().len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

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
    merging.merge_by = ["id"]
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
      options.withMergeBy(['id']),
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
| `merge_arrow_reader` | Keys the rows by their partition, then `merge_by`; groups them by partition, reads only that partition's files whose recorded key bounds overlap the group's keys, and carries every other file with the same location, statistics, and commit order; without `merge_by` a partitioned table replaces the partitions the rows fall in |
| `append_arrow_reader` | Writes new data files and keeps every manifest of the last snapshot; nothing stored is read or rewritten |

### The table value as a handle

A `Table` value is itself a handle, answering from metadata it already holds; the folder route probes the location on every call. A handle on one `column=value` directory addresses that partition, with its files taken from the manifest rather than a directory listing.

Rust only.

```rust
use yggdryl::media::IORecordOptions;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::{IOBase, IOMedia, StructType};
use yggdryl::local::LocalFolder;
use yggdryl::{arrow, DataType, MimeType};

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

let mut schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-table-handle");
let _ = std::fs::remove_dir_all(&path);
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

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

// A partition directory is a handle too, reading the files the manifest names.
let partition = LocalFolder::new(path.join("data").join("venue=XNYS"))?;
let partition_rows: usize = partition
    .read_arrow_reader(&partition.record_options()?)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(partition_rows, 1);

let merging = options.clone().with_merge_by("id")?;
table.merge_arrow_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;
assert_eq!(table.metadata().snapshots().len(), 2);
assert_eq!(table.current_snapshot().unwrap().operation(), "overwrite");

// A partition filter is answered by the scan plan, so the other partitions'
// files are never opened.
let filtered = options.clone().with_filter("venue = 'XNYS'")?;
let matching: usize = table
    .read_arrow_reader(&filtered)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(matching, 1);
```

## Partition keys are the primary keys

A merge joins on the identity partition columns first and the caller's key after them, each named once, so a row can only ever update a row of its own partition. The incoming rows are grouped by partition tuple - the grouping an append lays files out by - and each group joins with its own partition's files alone: the plan opens the manifests and files of those partitions, the key bounds narrow them further, and everything else is carried into the new snapshot under its own path. What is in memory at once is one group's rows and the files it selected. Within one write, the last of the rows arriving with one key wins. A merge that names no key is keyed by the partition alone and replaces the partitions its rows fall in; an unpartitioned table has nothing to match on then and refuses by name. The spec those columns come from is on [Partitions](partitions.md).

=== "Rust"

    ```rust
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType, Selector};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-primary-keys");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.into_arrow_schema()?;
    let rows = |ids: Vec<i64>, symbols: Vec<&'static str>, venues: Vec<&'static str>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
                Arc::new(StringArray::from(venues)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };
    for (id, symbol, venue) in [(1, "AAPL", "XNAS"), (2, "MSFT", "XNYS"), (3, "VOD", "XLON")] {
        table.commit_append(rows(vec![id], vec![symbol], vec![venue]))?;
    }
    let paths = |table: &Table<LocalFolder>| -> BTreeSet<String> {
        table.data_files().expect("the files list").into_iter().map(|(file, _)| file.file_path.to_string()).collect()
    };
    let before = paths(&table);

    // The key is (venue, id): id 1 updates inside XNAS, id 4 appends there,
    // and the two other partitions' files are carried under their own paths.
    table.commit_merge(
        rows(vec![1, 4], vec!["AAPL.O", "NVDA"], vec!["XNAS", "XNAS"]),
        &Selector::from_columns(["id"]),
        true,
    )?;
    let after = paths(&table);
    assert_eq!(before.intersection(&after).count(), 2);

    // The same id in another partition is another row, never an update.
    table.commit_merge(rows(vec![1], vec!["AAPL.L"], vec!["XLON"]), &Selector::from_columns(["id"]), true)?;
    assert_eq!(table.scan_where(&[("id", "1")], None)?.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 2);

    // No key: the partition is the key, so XNYS is replaced by the rows sent to it.
    table.commit_merge(rows(vec![9], vec!["MSFT.N"], vec!["XNYS"]), &Selector::all(), true)?;
    assert_eq!(table.scan_where(&[("venue", "XNYS")], None)?.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 1);
    assert_eq!(table.scan(None)?.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 5);

    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
        pa.field("venue", pa.string()),
    ])
    rows = lambda ids, symbols, venues: pa.record_batch(
        {"id": ids, "symbol": symbols, "venue": venues}, schema=columns
    )
    path = pathlib.Path(tempfile.mkdtemp()) / "trades"
    table = Table.create(IOBase(path), columns, ["venue"])
    for id_, symbol, venue in [(1, "AAPL", "XNAS"), (2, "MSFT", "XNYS"), (3, "VOD", "XLON")]:
        table.append(rows([id_], [symbol], [venue]))
    before = {file.path for file, _ in table.data_files()}

    # The key is (venue, id): id 1 updates inside XNAS, id 4 appends there,
    # and the two other partitions' files are carried under their own paths.
    table.merge(rows([1, 4], ["AAPL.O", "NVDA"], ["XNAS", "XNAS"]), ["id"])
    after = {file.path for file, _ in table.data_files()}
    assert len(before & after) == 2

    # The same id in another partition is another row, never an update.
    table.merge(rows([1], ["AAPL.L"], ["XLON"]), ["id"])
    assert table.scan_where({"id": "1"}).read_all().num_rows == 2

    # No key: the partition is the key, so XNYS is replaced by the rows sent to it.
    table.merge(rows([9], ["MSFT.N"], ["XNYS"]), [])
    assert table.scan_where({"venue": "XNYS"}).read_all().num_rows == 1
    assert table.scan().read_all().num_rows == 5
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8?'), Field.from('venue: utf8?')],
      { nullable: false },
    )
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.Table.create(root, schema, ['venue'])
    const rows = (ids, symbols, venues) =>
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray(ids, new arrow.Int64()),
          symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
          venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
        }),
      )
    for (const [id, symbol, venue] of [[1n, 'AAPL', 'XNAS'], [2n, 'MSFT', 'XNYS'], [3n, 'VOD', 'XLON']]) {
      table.append(rows([id], [symbol], [venue]))
    }
    const paths = () => new Set(table.dataFiles().map((file) => file.filePath))
    const before = paths()

    // The key is (venue, id): id 1 updates inside XNAS, id 4 appends there,
    // and the two other partitions' files are carried under their own paths.
    table.merge(rows([1n, 4n], ['AAPL.O', 'NVDA'], ['XNAS', 'XNAS']), ['id'])
    const after = paths()
    assert.equal([...before].filter((file) => after.has(file)).length, 2)

    // The same id in another partition is another row, never an update.
    table.merge(rows([1n], ['AAPL.L'], ['XLON']), ['id'])
    assert.equal(table.scanWhere({ id: '1' }).intoTable().numRows, 2)

    // No key: the partition is the key, so XNYS is replaced by the rows sent to it.
    table.merge(rows([9n], ['MSFT.N'], ['XNYS']), [])
    assert.equal(table.scanWhere({ venue: 'XNYS' }).intoTable().numRows, 1)
    assert.equal(table.scan().intoTable().numRows, 5)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Sorted data files

Every data file a commit writes holds one partition group with its rows in the table's default sort order, so the bounds it records on the sorted columns are tight and a filter on them prunes files instead of reading them. The default order a table takes at `create` is `SortOrder::for_spec`: the partition spec's source columns in spec order, ascending, nulls first, recorded in the metadata as sort order 1 and made the default; an unpartitioned spec derives from nothing, so its default stays the unsorted order 0. `Table::create_sorted` declares another - or `SortOrder::unsorted()`, which keeps rows in the order they arrived and records no order on the files. A sort field is honoured through its source column: identity, truncation and the calendar transforms order exactly as their source does, and a bucket orders by its source value rather than its hash.

Rust only; the bindings read the table's sort orders off its metadata as every other engine does.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::iceberg::{
    FormatVersion, IcebergOptions, PartitionSpec, SortField, SortOrder, Table, Transform,
    assign_field_ids,
};
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, StructType, arrow};

let mut schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("row");
assign_field_ids(&mut schema, 1)?;
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;

// The default order is the partition's source column, ascending, nulls first.
let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-sorted");
let _ = std::fs::remove_dir_all(&path);
let table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec.clone())?;
let order = table.metadata().default_sort_order()?;
assert_eq!(table.metadata().default_sort_order_id(), 1);
assert_eq!(order.fields[0].source_id, 3);
assert_eq!(order.fields[0].transform, Transform::Identity);
assert_eq!((order.fields[0].direction.as_str(), order.fields[0].null_order.as_str()), ("asc", "nulls-first"));

// An explicit order sorts every file it writes: under a one-byte target each
// row is one file, and the files' symbol bounds march with the sort.
let sorted_path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-sorted-by-symbol");
let _ = std::fs::remove_dir_all(&sorted_path);
let by_symbol = SortOrder {
    order_id: 1,
    fields: vec![SortField {
        source_id: 2,
        transform: Transform::Identity,
        direction: "asc".into(),
        null_order: "nulls-last".into(),
    }],
};
let mut table = Table::create_sorted(LocalFolder::new(&sorted_path)?, FormatVersion::V2, schema.clone(), spec, by_symbol)?;
table.set_options(IcebergOptions::new().try_with_target_file_size_bytes(1)?);
let batch = RecordBatch::try_new(
    schema.into_arrow_schema()?,
    vec![
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Arc::new(StringArray::from(vec!["c", "a", "b"])),
        Arc::new(StringArray::from(vec!["X", "X", "X"])),
    ],
)?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
let lower_bounds: Vec<String> = table
    .data_files()?
    .into_iter()
    .map(|(file, _)| {
        assert_eq!(file.sort_order_id, Some(1));
        let (_, bytes) = file.lower_bounds.iter().find(|(id, _)| *id == 2).expect("a symbol bound");
        String::from_utf8(bytes.clone()).expect("utf-8")
    })
    .collect();
assert_eq!(lower_bounds, ["a", "b", "c"]);

let _ = std::fs::remove_dir_all(&path);
let _ = std::fs::remove_dir_all(&sorted_path);
```

## Parallel partition writes

The partition groups of one commit are independent - each writes its own files under its own directory - so they are written on worker threads, at most `write.parallelism` at once. The option resolves like every other: the explicit value, then the table property, then the default, which is the resolved `read.parallelism`; 1 writes the groups one after another on the calling thread. Whatever the parallelism, the manifest lists the files in partition-group order, never in completion order, so a commit's manifest bytes do not depend on scheduling, and a group that fails fails the commit before any metadata is written.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::{FormatVersion, IcebergOptions, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-parallel-writes");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // The default is the read parallelism; a table property or an explicit
    // option overrides it, and zero is refused naming the key.
    assert_eq!(table.options()?.write_parallelism(), table.options()?.read_parallelism());
    assert!(IcebergOptions::new().set_write_parallelism(0).is_err());
    table.set_options(IcebergOptions::new().try_with_write_parallelism(4)?);
    assert_eq!(table.options()?.write_parallelism(), 4);

    // Eight partitions on four threads: the manifest lists v1 through v8.
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from((1..=8).collect::<Vec<i64>>())),
            Arc::new(StringArray::from((1..=8).map(|n| format!("v{n}")).collect::<Vec<_>>())),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    let venues: Vec<String> = table
        .data_files()?
        .into_iter()
        .map(|(file, _)| file.partition[0].as_str().expect("a venue").to_owned())
        .collect();
    assert_eq!(venues, ["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"]);

    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergOptions, Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    path = pathlib.Path(tempfile.mkdtemp()) / "trades"
    table = Table.create(IOBase(path), columns, ["venue"])

    # The default is the read parallelism; the property and the explicit
    # option override it, and zero is refused naming the key.
    assert table.options().write_parallelism == table.options().read_parallelism
    table.update_properties({"write.parallelism": "3"})
    assert table.options().write_parallelism == 3
    try:
        IcebergOptions(write_parallelism=0)
    except ValueError as error:
        assert "write.parallelism" in str(error)

    # Eight partitions on four threads: the manifest lists v1 through v8.
    rows = pa.record_batch(
        {"id": list(range(1, 9)), "venue": [f"v{n}" for n in range(1, 9)]}, schema=columns
    )
    table.append(rows, options=IcebergOptions(write_parallelism=4))
    assert [file.partition[0] for file, _ in table.data_files()] == [f"v{n}" for n in range(1, 9)]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8?')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.Table.create(root, schema, ['venue'])

    // The default is the read parallelism; the property and the explicit
    // option override it, and zero is refused naming the key.
    assert.equal(table.options().writeParallelism, table.options().readParallelism)
    table.updateProperties({ 'write.parallelism': '3' })
    assert.equal(table.options().writeParallelism, 3)
    assert.throws(() => new iceberg.IcebergOptions({ writeParallelism: 0 }), /write\.parallelism/)

    // Eight partitions on four threads: the manifest lists v1 through v8.
    const ids = [1n, 2n, 3n, 4n, 5n, 6n, 7n, 8n]
    const rows = new arrow.Table({
      id: arrow.vectorFromArray(ids, new arrow.Int64()),
      venue: arrow.vectorFromArray(ids.map((n) => `v${n}`), new arrow.Utf8()),
    })
    table.append(rows, new iceberg.IcebergOptions({ writeParallelism: 4 }))
    assert.deepEqual(
      table.dataFiles().map((file) => file.partition[0].asJs()),
      ids.map((n) => `v${n}`),
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Staged commits

A commit into a remote table is a set of uploads: every data file, the manifest, the manifest list, then the document that names them. With `write.staging` on, each of those files is encoded into a local staging file first - under a directory of the commit's own, so two commits into one table never see each other's files - its statistics are read back from that copy rather than from the store, and it goes out as one upload, multipart above the store's threshold. The upload streams the staged file: above the threshold one part at a time, so memory holds one part of one file per writer thread, and below it the whole file, read once; the staged copy is removed as soon as its upload ends, so the disk holds each writer thread's current file and nothing else.

The staging is a transaction whose point of no return is the versioned metadata document. Until that document is durable, a failure - a refused upload, a commit beaten out of retries, a refused document write - removes every file the commit published, the attempt document included, so nothing is left that the metadata does not name, on the store or on the local disk; the directory goes when the commit ends however it ends. Once the document is durable the files are the table's whatever a later step reports, because every fresh handle resolves the version to that document: a hint write the store publishes and then reports as failed leaves a table that reads whole. A commit beaten on write publishes its manifest list again for the snapshot it rebases onto and removes the list of the attempt it replaces, one request, so a successful commit leaves nothing the metadata does not name either. An upload the store refuses is not followed by a removal: a refused `PUT` stored nothing, and an abandoned multipart upload is aborted.

The option resolves like every other: the explicit value, then the table property, then the default, which is the root's own - the platform temporary folder when the root is remote, `off` when it is local, where a staging file would be a second copy of a file already on the same disk. `off` writes every file straight to the table; a folder must be local, and a remote one is refused naming the key.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table, WriteStaging, assign_field_ids,
    };
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-staging");
    let stage = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-staging-folder");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // A local root stages nothing by default; the property and the explicit
    // option override it, and a remote folder is refused naming the key.
    assert_eq!(table.write_staging()?, WriteStaging::Off);
    assert_eq!(IcebergOptions::new().write_staging(), None);
    let folder = WriteStaging::from_str(&stage.to_string_lossy())?;
    assert!(WriteStaging::from_str("s3://trades/stage").is_err());
    table.commit_metadata_changes(|metadata| {
        metadata.set_property(IcebergOptions::WRITE_STAGING_KEY, folder.to_string())?;
        Ok(())
    })?;
    assert_eq!(table.write_staging()?, folder);
    table.set_options(IcebergOptions::new().try_with_write_staging(WriteStaging::Off)?);
    assert_eq!(table.write_staging()?, WriteStaging::Off);

    // A staged commit reads back as any other, and the staging folder holds
    // nothing once it is done.
    table.set_options(IcebergOptions::new().try_with_write_staging(folder)?);
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    assert_eq!(table.data_files()?.len(), 2);
    assert!(!stage.exists() || std::fs::read_dir(&stage)?.next().is_none());

    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_dir_all(&stage);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergOptions, Table

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    path = pathlib.Path(tempfile.mkdtemp()) / "trades"
    stage = pathlib.Path(tempfile.mkdtemp()) / "stage"
    table = Table.create(IOBase(path), columns, ["venue"])

    # A local root stages nothing by default; the property and the explicit
    # option override it, and a remote folder is refused naming the key.
    assert IcebergOptions().write_staging is None
    assert table.options().write_staging is None
    table.update_properties({"write.staging": str(stage)})
    assert table.options().write_staging.startswith("file:")
    try:
        IcebergOptions(write_staging="s3://trades/stage")
    except ValueError as error:
        assert "write.staging" in str(error)

    # A staged commit reads back as any other, and the staging folder holds
    # nothing once it is done.
    rows = pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns)
    table.append(rows, options=IcebergOptions(write_staging=str(stage)))
    assert len(table.data_files()) == 2
    assert not stage.exists() or not any(stage.iterdir())
    assert IcebergOptions(write_staging="off").write_staging == "off"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8?')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const stage = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'stage')
    const table = iceberg.Table.create(root, schema, ['venue'])

    // A local root stages nothing by default; the property and the explicit
    // option override it, and a remote folder is refused naming the key.
    assert.equal(new iceberg.IcebergOptions().writeStaging, null)
    assert.equal(table.options().writeStaging, null)
    table.updateProperties({ 'write.staging': stage })
    assert.ok(table.options().writeStaging.startsWith('file:'))
    assert.throws(() => new iceberg.IcebergOptions({ writeStaging: 's3://trades/stage' }), /write\.staging/)

    // A staged commit reads back as any other, and the staging folder holds
    // nothing once it is done.
    const rows = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
    })
    table.append(rows, new iceberg.IcebergOptions({ writeStaging: stage }))
    assert.equal(table.dataFiles().length, 2)
    assert.ok(!fs.existsSync(stage) || fs.readdirSync(stage).length === 0)
    assert.equal(new iceberg.IcebergOptions({ writeStaging: 'off' }).writeStaging, 'off')

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    fs.rmSync(path.dirname(stage), { recursive: true, force: true })
    ```

## Data files aim at a size

The bindings read the target as `target_file_size` / `targetFileSize`, and Parquet compression lands files under it rather than at it. A table that has accumulated small files rewrites them with `compact()`, which reports the same three numbers in each language's casing.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::iceberg::{Catalog, FormatVersion};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType};

    let warehouse = LocalFolder::temporary()?.path()?.join("yggdryl-doc-compaction");
    let _ = std::fs::remove_dir_all(&warehouse);
    let catalog = Catalog::new(LocalFolder::new(&warehouse)?);

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
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
    assert_eq!(table.compact()?, yggdryl::iceberg::Compaction::default());

    let _ = std::fs::remove_dir_all(&warehouse);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl.iceberg import Catalog

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
- `write.parallelism`, defaulting to the resolved `read.parallelism`
- `write.staging`, `off` or a local folder, defaulting to the temporary folder for a remote root and `off` for a local one

=== "Rust"

    ```rust
    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-options");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let mut table = Table::create(
        LocalFolder::new(&root)?,
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
    from yggdryl.iceberg import IcebergOptions, Table

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

The JavaScript constructor takes an object naming any of the eleven fields, and every field is also a getter and a setter. A per-call value never mutates the passed object and never leaks into the handle's own override.

| Binding | Per-call layer | Handle-wide override |
| --- | --- | --- |
| Python | `options=` on each operation | `set_options` |
| JavaScript | Trailing argument of `scan(field, options)`, `scanAt(id, filters, field, options)`, `append(rows, options)`, `overwrite(rows, options)`, and the tables view's `append` / `overwrite` | `setOptions` |

## The data-file MIME type

`data_mime_type` / `dataMimeType` accepts a `MimeType` or anything its parser accepts, such as `parquet`, `.avro`, or a canonical MIME name. Each manifest stays authoritative, so a snapshot mixing [Parquet](../parquet/index.md) and [Avro](../avro/index.md) files scans as one table.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::iceberg::{FormatVersion, IcebergOptions, PartitionSpec, Table};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, MimeType, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-data-format");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let mut table = Table::create(
        LocalFolder::new(&root)?,
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
    from yggdryl.iceberg import IcebergOptions, Table, assign_field_ids

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
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-concurrency");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    Table::create(
        LocalFolder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    // Two handles opened at the same version, each unaware of the other.
    let mut left = Table::open(LocalFolder::new(&root)?)?;
    let mut right = Table::open(LocalFolder::new(&root)?)?;

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
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-branching");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let mut table = Table::create(
        LocalFolder::new(&root)?,
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
    from yggdryl.iceberg import Table

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

- `merge_arrow_reader` on a `Table` without match keys -> the partition columns are the key, so the partitions the rows fall in are replaced; on an unpartitioned table, and through a plain folder handle, a merge still requires `merge_by`.
- Merge key naming a partition column -> named once; the partition columns always lead the key.
- A live data file written under another partition spec that the key bounds cannot exclude -> the merge is refused naming the file and both spec ids; it belongs to no partition of the current spec, so rewrite it first.
- Duplicate keys inside one incoming write -> the last row wins, per partition group, before the join.
- Unknown (`DataType::Null`) column -> never written into a data file, as the spec requires; the scan restores it as null.
- A sort field over a bucket transform -> the file's rows order by the source value; every other transform orders exactly as its source.
- A partition writer thread that fails -> the commit fails before any manifest or metadata document is written; the files it wrote are orphans no snapshot names.
- A partition filter naming an undeclared column, on a `Table` handle -> error.
- The same filter on a folder of leaves -> ignored, because its batches do not carry that column.
- A stray file nobody committed, or a file an overwrite replaced -> never read.
- `record_options` / `read_arrow_field` on a table -> answered from the metadata before any data file exists; the schema keeps its field identifiers.
- `IOBase::kind` on a `Table` -> [`IOKind::Table`](../../holder/index.md), not the root folder's `Directory`; `is_tabular` is true without touching storage.
- A write through a `Table` value -> one commit; `current_snapshot` and `version` stay current without reopening.
- A `filter` on a `Table` -> prunes data files through the scan plan, not rows after decoding.
- `compact()` -> touches only partition groups holding at least two files with one under the target.
- Every file `compact()` does not touch -> carried into the new snapshot untouched; the prior snapshot still time-travels.
- `compact()` with nothing to do -> no-op that commits nothing; all three numbers are zero.
- A table property that is present but does not parse -> typed error naming the key and the value, never a silent default.
- A broken stored property -> shadowed by an explicit option, which never reads it, so it can be repaired through the same handle.
- An unparseable `read.*` property -> cannot stop the metadata-only commit that fixes it, because a commit resolves only the four `commit.retry.*` keys.
- `new iceberg.IcebergOptions({ targetFileSize: 0 })` -> refused at the boundary, naming the value.
- `data_mime_type` of ORC or Puffin -> the write fails before consuming rows.
- Beaten `overwrite`, `merge`, or `compact` after the retries -> `CommitConflict`: `expected to commit version 4, got beaten 5 times; last saw version 8`.
- After a `CommitConflict` -> the in-memory state is restored; re-plan against the table as it now is.
- The check-then-write pair is not atomic -> on plain storage a writer landing between them goes undetected.
- Retries shrink that window but cannot close it; storage that serializes writers (an object store's atomic PUT, a catalog's swap) closes it.
- [`yggdryl::local`](../../holder/backends/local.md) memory mapping -> does not close it; two processes truncating one mapped file at the same instant is its documented SIGBUS hazard.
- A failed commit -> no visible change; at worst orphan data files no snapshot names.
- A branch fast-forward -> only along its own ancestry; the target must reach the branch's head by parent ids.
- Removing a ref -> removes the name only; the snapshots stay retained.
- A second removal of the same ref -> refused naming the ref (Python `ValueError`, JavaScript throws), not a commit of nothing.
- `expire_snapshots` with cutoff or retain count omitted -> resolved from `history.expire.max-snapshot-age-ms` and `history.expire.min-snapshots-to-keep`; per-ref settings override them.
- Explicit snapshot ids -> join age selection but cannot remove retained heads; `main` never expires; recent unreferenced snapshots survive until the cutoff.
- `gc.enabled=false` -> the atomic expiry update is refused.
- Expired snapshots -> lose their statistics descriptors; physical file cleanup is separate.
- Ref changes -> the same retry gate as writes.
- Writing to a non-`main` branch directly -> not supported; read it with `scan_ref` and move it with `fast_forward`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::handles
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::concurrency_and_compaction
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::data_mime_type
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::line_projection
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::isolation
    cargo test --features "parquet iceberg" -p yggdryl --test iceberg -- partition::iceberg scan::iceberg staging::iceberg table::iceberg types::iceberg
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^compact/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^merge/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^commit/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_iceberg.py
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/iceberg.test.js
    YGGDRYL_BENCH_FILTER=iceberg/append npm run --prefix node bench:media
    ```
