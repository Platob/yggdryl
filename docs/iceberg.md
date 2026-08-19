# Apache Iceberg

Read and write Apache Iceberg tables through one [`IOBase`](io.md) handle.

!!! note "All three"
    Python has the table - create, open, scan, append, overwrite, evolve, and
    the metadata a commit produced - as `yggdryl.iceberg`, and JavaScript has
    the same surface as the `iceberg` namespace of `yggdryl`. The
    standalone document readers and writers stay in Rust, and each section below
    says so.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-lead");
    let _ = std::fs::remove_dir_all(&path);

    // A table is created in a folder, and a folder is all it ever touches.
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // A table that has never been written to has no current snapshot.
    assert!(table.current_snapshot().is_none());
    assert_eq!(table.scan(None)?.count(), 0);

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

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
    from yggdryl.iceberg import Table

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
    reopened = Table.open(IOBase(root.url.to_path()))
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
    assert.equal(table.scan().toTable().numRows, 0)

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
    assert.equal(reopened.scan().toTable().numRows, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

**An Iceberg table is a folder.** `metadata/` holds the JSON documents and the Avro manifests,
`data/` holds the Parquet files, and everything here is reached with
[`IOBase::child_by_path`](io.md) and [`IOBase::ls`](io.md) against the handle the table was constructed
from. Nothing in this module opens a path or calls the file system, so the same code works over a
local directory today and over an object store the moment a backend for one exists.

The vocabulary is the crate's own. A schema is a non-null struct [`Field`](field.md) whose children
carry `PARQUET:field_id`; a metadata document is a [`Value`](json.md) read by the crate's own JSON
parser; a data file is whatever [parquet](parquet.md) wrote plus the statistics it reported; a scan
is a `BatchReader` with the same [column pushdown](io.md) every other read gets. No dependency is
added for the table format itself - even the Avro container the manifests live in is implemented
here, because it is a header and some blocks.

## The `iceberg` feature

`iceberg` is not a default feature:

```toml
[dependencies]
yggdryl = { version = "0.1", features = ["iceberg"] }
```

It implies `parquet`, which implies `arrow`. A table format sits on top of the record encodings, so
a consumer that only needs schemas never compiles it.

## What a table writes

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-layout");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

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
    from yggdryl.iceberg import Table

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
`*.metadata.json` - because that is the only way to find a table without a catalog.

## Table metadata, v1 through v3

!!! note "Rust only"
    A metadata document is read and written from Rust. The bindings read the
    version a table declares as its `format_version`.

```rust
use yggdryl::iceberg::{FormatVersion, PartitionSpec, TableMetadata};
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
let document = v1.to_json()?;
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
assert!(v2.to_json()?.contains_key("last-sequence-number"));

// v3 adds row lineage.
let v3 = TableMetadata::new(
    FormatVersion::V3,
    "file:///lake/trades",
    schema,
    PartitionSpec::unpartitioned(),
)?;
assert_eq!(v3.next_row_id, Some(0));
assert!(v3.to_json()?.contains_key("next-row-id"));

// Every version reads back as itself.
for original in [v1, v2, v3] {
    let read = TableMetadata::from_json(&original.to_json()?)?;
    assert_eq!(read.format_version, original.format_version);
    assert!(read.current_snapshot().is_none());
}
```

Reading normalizes: a v1 document's `schema` becomes a one-element `schemas`, and its bare
`partition-spec` array becomes a spec with id zero, so nothing downstream has to ask which version
it is looking at. Writing emits exactly what the declared version requires.

The v3 additions this module implements are `next-row-id` on the table, `first-row-id` and
`added-rows` on each snapshot, the nanosecond temporals `timestamp_ns` and `timestamptz_ns`, the
`unknown` type, and the `initial-default` / `write-default` column values, which travel as reserved
`iceberg:*` [Field metadata](field.md).

## Snapshots and the current snapshot

!!! note "Rust only"
    The bindings read the same values off a table - its current snapshot and
    its snapshots - rather than off a metadata document.

```rust
use yggdryl::iceberg::{FormatVersion, PartitionSpec, TableMetadata};
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
let document = metadata.to_json()?.with_key("current-snapshot-id", -1_i64)?;
let read = TableMetadata::from_json(&document)?;
assert!(read.current_snapshot_id.is_none());
assert!(read.current_snapshot().is_none());
```

A snapshot is one complete version of the table: an identifier, the manifest list naming every
manifest alive at that moment, and a summary of what the commit did. The *current* snapshot is a
pointer, which is why `current_snapshot` returns an `Option` and why reading a table without one
must yield no rows rather than fail.

## Manifest lists and manifests

=== "Rust"

    ```rust
    use yggdryl::iceberg::{
        EntryStatus, FileFormat, FormatVersion, PartitionSpec, Table, assign_field_ids, read_manifest,
        read_manifest_spec,
    };
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-manifests");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec.clone())?;

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNAS")])),
        ],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

    // A snapshot names one manifest list; each of its rows is a manifest.
    let manifests = table.manifests()?;
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].added_files_count, 1);
    assert_eq!(manifests[0].added_rows_count, 2);

    // A manifest is self-describing: its Avro header carries the schema and the spec.
    let name = manifests[0].manifest_path.rsplit('/').next().unwrap().to_owned();
    let handle = Folder::new(&path)?.child_by_path(&format!("metadata/{name}"))?;
    assert_eq!(read_manifest_spec(&handle)?, spec);

    let entries = read_manifest(&handle)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, EntryStatus::Added);
    assert_eq!(entries[0].data_file.file_format, FileFormat::Parquet);
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

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

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
    assert file.file_format == "PARQUET"
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
    const { Field, fields, iceberg } = require('yggdryl')

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
    assert.equal(file.fileFormat, 'PARQUET')
    assert.equal(file.recordCount, 2)
    assert.deepEqual(file.partitionNames, ['venue'])

    // Statistics are keyed by field id, which is what lets a planner skip a file.
    assert.ok(file.valueCounts.some((entry) => entry.fieldId === 1 && entry.count === 2))
    assert.ok(file.columnSizes.some((entry) => entry.fieldId === 1))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

Iceberg puts two levels of indirection between a snapshot and its rows, and both are Avro,
implemented by the [`avro`](avro.md) codec module: a container is a header naming a writer schema,
then blocks of records separated by a synchronization marker. Manifest rows cross the boundary as
the same [`Value`](json.md) the JSON parser produces, so a manifest row and a metadata document are
read with one vocabulary. The writer is deterministic - the marker is derived from the content - so
two writers given the same entries produce the same bytes, which is what lets a conformance check
diff manifests instead of only comparing what they mean.

Two readers serve two needs. `read_manifest` decodes every field, and is what any path that may
*carry an entry forward* - an overwrite, a merge, a compaction - must use, because a carried entry
keeps its statistics. `read_manifest_for_plan` is the read-only planning fast path: it decodes
through a compiled schema-resolution plan that keeps only what pruning consults - file identity, the
partition tuple, sizes, and (for a filtered plan) the counts and bounds - and skips every other
statistics map as raw bytes. On wide manifests it decodes several times faster than the full read;
[the benchmarks](benchmarks.md) put numbers and outside baselines on that claim. A table's scans use
it automatically; the function is public for callers walking manifests themselves.

Statistics come from the Parquet footer the write just produced. Counts and sizes are emitted for
every top-level column; *bounds* are emitted only for the types whose Parquet statistic bytes are
byte-for-byte the Iceberg single-value encoding. A decimal is the case that differs - Parquet stores
it big-endian in a fixed width, Iceberg stores the minimal two's-complement big-endian - so a decimal
column gets counts but no bounds, rather than bounds that mean something else.

## Partition specs and the Hive layout

!!! note "Rust only"
    The bindings build the identity spec a table is created with, and read
    one back off the table; the transform vocabulary and the path rendering
    are Rust.

```rust
use yggdryl::iceberg::{PartitionSpec, Transform, assign_field_ids};
use yggdryl::{DataType, Value};

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
assert_eq!(spec.partition_path(&[Value::from("XNAS")])?, "venue=XNAS");
assert_eq!(spec.partition_path(&[Value::Null])?, "venue=null");

// A partition value is nullable even when its source column is not.
let partition = spec.partition_field(&schema)?;
assert!(partition.fields()[0].is_nullable());

// Only the invertible transforms can place a row.
assert!(Transform::Identity.is_invertible());
assert!(!Transform::from_str("bucket[16]")?.is_invertible());
let mut hashed = spec.clone();
hashed.fields[0].transform = Transform::Bucket(16);
assert!(hashed.require_writable().unwrap_err().to_string().contains("bucket[16]"));
```

Iceberg writes partition directories in exactly the `column=value` shape
[`Url::hive_partitions`](uri.md) already reads, so a table this module writes is also a lake the rest
of the crate can walk with [`IOBase::children_where`](io.md). It is the same shape because it is the
same renderer: `partition_path` spells a value through
[`io::partition::partition_text`](io.md#partition-columns-in-the-data), which is what a partitioned
folder write applies to a whole column, so a date is `day=2024-01-01` in a table and in a lake alike.
Unlike Hive, an Iceberg data file still stores its partition columns, so a scan needs no restoration
step in the normal case.

A spec and a schema say the same thing, so neither has to be spelled twice. The partition tuple
carries what produced each of its columns - the transform, the source column, and the partition
marker every path-borne column carries - and a schema can carry the marks itself:

```rust
use yggdryl::iceberg::{PartitionSpec, assign_field_ids};
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
assert_eq!(partition.iceberg().get("spec-id"), Some("1"));
let venue = partition.get_field_by_name("venue").expect("the partition column");
assert!(venue.is_partition());
assert_eq!(venue.iceberg().get("transform"), Some("identity"));
assert_eq!(PartitionSpec::from_field(&partition)?, spec);

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
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-null-partition");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), None])),
        ],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

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
    from yggdryl.iceberg import Table

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

## Reading with column pushdown

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use std::sync::Arc;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-pushdown");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

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
    from yggdryl.iceberg import Table

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
    const wanted = fields.struct('row', [schema.dataType.at(0)], { nullable: false })
    const projected = table.scan(wanted).toTable()
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id'])
    assert.equal(projected.numRows, 2)

    // No target reads everything.
    assert.equal(table.scan().toTable().numCols, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

`Table::scan` hands its optional `Field` to each data file as the schema
[`IOBase::read_arrow_batch_reader`](io.md) reads under, minus the partition columns the file does not
store, then casts what comes back to the scan's own root. The pushdown is what makes a projected scan
cheap; the cast is what makes a table whose schema evolved readable as one shape.

## Planning a scan from the metadata

!!! note "All three"
    The planner is Rust, and both bindings report what it decided: `plan` and
    `plan_at` answer a `ScanPlan` in each language.
    [Filtered reads and filtered writes](#filtered-reads-and-filtered-writes)
    shows the same numbers from Python and JavaScript.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-plan");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // One commit per venue, so the manifest list has three rows to prune.
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON")] {
        let batch = RecordBatch::try_new(
            schema.to_arrow_schema()?,
            vec![
                Arc::new(Int64Array::from(vec![id])),
                Arc::new(StringArray::from(vec![Some(venue)])),
            ],
        )?;
        table.append(arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // Nothing is listed: the snapshot names the manifest list, whose per-partition
    // summaries exclude two manifests before either Avro file is opened.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count(), 1);
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

## Time travel and the inspection tables

!!! note "All three"
    `scan_at` / `scanAt`, `snapshot_by_ref` / `snapshotByRef`, and the three
    inspection readers cross into both bindings as ordinary record-batch
    readers.

Nothing a commit writes is mutated in place, so every retained snapshot is still a complete table.
Reading one is an ordinary scan with the snapshot named:

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-time-travel");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.to_arrow_schema()?;
    let one = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![1]))],
    )?;
    table.append(yggdryl::arrow::batch_reader(std::sync::Arc::clone(&arrow_schema), [one]))?;
    let past = table.current_snapshot().expect("one commit").snapshot_id;

    let nine = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![9]))],
    )?;
    table.overwrite(yggdryl::arrow::batch_reader(arrow_schema, [nine]))?;

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
    from yggdryl.iceberg import Table

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
    assert table.inspect_snapshots().read_all().column("operation").to_pylist() == [
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
    assert.deepEqual(table.scan().toTable().getChild('id').toArray(), BigInt64Array.from([9n]))
    assert.deepEqual(table.scanAt(past).toTable().getChild('id').toArray(), BigInt64Array.from([1n]))

    // A branch or tag resolves by name, and every commit moves `main`.
    assert.equal(table.snapshotByRef('main').snapshotId, table.currentSnapshot.snapshotId)

    // The inspection readers render the table's own record as record batches.
    assert.equal(table.inspectHistory().toTable().numRows, 2)
    assert.equal(table.inspectSnapshots().toTable().getChild('operation').get(1), 'overwrite')
    assert.equal(table.inspectFiles().toTable().numRows, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A snapshot is read as the schema that was current when it was written, so a column added later does
not appear and a column dropped later still does. A branch or tag resolves with `snapshot_by_ref`,
and a metadata-only change - a property, a new ref, an evolved schema - commits through
`commit_changes`, which writes one new metadata document and leaves the table untouched when the
change or the write fails.

The table also renders its own record as record batches, under the column names PyIceberg's
inspection tables use: `inspect_history` (when each snapshot became current, and whether it is on
the current ancestry chain), `inspect_snapshots` (operation, manifest list, and the summary map per
retained snapshot), and `inspect_files` (path, format, spec, rendered `column=value` partition
chain, row count, and size per live data file). They are ordinary readers, so the same collect that
drains a scan drains them.

## Filtered reads and filtered writes

!!! note "All three"
    The whole surface crosses. Python spells it `plan`, `plan_at`,
    `scan_where`, `overwrite_where`, `merge`, and `merge_where`; JavaScript
    spells the same six `plan`, `planAt`, `scanWhere`, `overwriteWhere`,
    `merge`, and `mergeWhere`; and `ScanPlan` reports the same five numbers in
    each language's casing.

A filter is a column name and a value as text - the vocabulary
[`IOBase::children_where`](io.md) filters a lake with - and it crosses as a mapping or as a
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
    use yggdryl::iceberg::{DataFile, FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("qty"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let root = std::env::temp_dir().join("yggdryl-doc-filtered-writes");
    let _ = std::fs::remove_dir_all(&root);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&root)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.to_arrow_schema()?;
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
        table.append(rows(vec![id], vec![venue], vec![10]))?;
    }
    let inserted = table.current_snapshot().expect("three commits").snapshot_id;

    // Nothing is listed and no data file is opened: the manifest list's
    // per-partition summaries exclude two manifests before either is read.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count(), 1);
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
    table.overwrite_where(&[("venue", "XNYS")], rows(vec![2], vec!["XNYS"], vec![99]))?;
    let after = paths(table.data_files()?);
    assert_eq!(before.difference(&after).count(), 1, "one partition was rewritten");
    assert_eq!(before.intersection(&after).count(), 2, "the others were carried");

    // A merge upserts on the key: 3 is stored and updates, 4 is new and appends.
    table.merge(rows(vec![3, 4], vec!["XLON", "XLON"], vec![7, 8]), &["id".to_owned()], true)?;
    let total: usize = table
        .scan(None)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(total, 4);

    // Narrowed first: a merge into one partition can read no other partition.
    table.merge_where(
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
    from yggdryl.iceberg import Table

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
    assert.equal(table.scanWhere({ venue: 'XNYS' }).toTable().numRows, 1)

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
    const merged = new Map(table.scan().toTable().toArray().map((row) => [row.id, row.qty]))
    assert.deepEqual([...merged.keys()].sort(), [1n, 2n, 3n, 4n])
    assert.equal(merged.get(2n), 99n)
    assert.equal(merged.get(4n), 8n)

    // Narrowed first: a merge into one partition can read no other partition.
    table.mergeWhere({ venue: 'XNAS' }, rows([1n], ['XNAS'], [42n]), ['id'])
    assert.equal(table.scanWhere({ venue: 'XNAS' }).toTable().getChild('qty').get(0), 42n)

    // History plans the same way: the snapshot before the overwrite still
    // selects one file for that partition, and it is the file that held 10.
    assert.equal(table.planAt(inserted, { venue: 'XNYS' }).filesPlanned, 1)
    assert.equal(
      table.scanAt(inserted, { venue: 'XNYS' }).toTable().getChild('qty').get(0),
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

## The three record methods over a table

=== "Rust"

    ```rust
    use yggdryl::generic::IORecordOptions;
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-records");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.to_arrow_schema()?;
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
    folder.write_arrow_batch_reader(rows(vec![1, 2], vec!["XNAS", "XNYS"]), &options)?;
    folder.append_arrow_batch_reader(rows(vec![3], vec!["XLON"]), &options)?;

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    let merging = options.clone().with_merge_by_names(["id"]);
    folder.write_arrow_batch_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;

    let total: usize = folder
        .read_arrow_batch_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);

    // Each call was one commit, and the read went through the last one.
    let table = Table::open(Folder::new(&path)?)?;
    assert_eq!(table.metadata().snapshots.len(), 3);
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
    folder.write_arrow_batch_reader(rows([1, 2], ["XNAS", "XNYS"]), options=options)
    folder.append_arrow_batch_reader(rows([3], ["XLON"]), options=options)

    # A match key upserts: `2` is stored and updates, `9` is new and appends.
    merging = folder.record_options()
    merging.merge_by_names = ["id"]
    folder.write_arrow_batch_reader(rows([2, 9], ["XNYS", "XLON"]), options=merging)

    assert folder.read_arrow_batch_reader(options=options).read_all().num_rows == 4

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
    folder.writeArrowBatchReader(rows([1n, 2n], ['XNAS', 'XNYS']), options)
    folder.appendArrowBatchReader(rows([3n], ['XLON']), options)

    // A match key upserts: `2` is stored and updates, `9` is new and appends.
    folder.writeArrowBatchReader(
      rows([2n, 9n], ['XNYS', 'XLON']),
      options.withMergeByNames(['id']),
    )

    assert.equal(folder.readArrowBatchReader(options).toTable().numRows, 4)

    // Each call was one commit, and the read went through the last one.
    assert.equal(iceberg.Table.open(root).snapshots.length, 3)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A handle addressing a table's folder is not read as a folder of Parquet files: it
is read through the current snapshot, so a file an overwrite replaced is never
read back and a stray file nobody committed is never read at all. The three
methods keep their meanings, and each one is a single commit:

- `read_arrow_batch_reader` scans the current snapshot, planning as above.
- `write_arrow_batch_reader` with no match key replaces every row; with one it
  merges, reading only the data files whose recorded bounds for the key columns
  overlap the incoming keys and carrying the rest into the new snapshot untouched
  - same location, same statistics, same commit order. That is what makes an
  upsert cost the files it can actually change, and it stays correct however
  coarse the statistics are, because a file that is not read keeps every row.
- `append_arrow_batch_reader` writes new data files and keeps every manifest the
  last snapshot had, so nothing stored is read or rewritten.

The relationship runs the other way too: a `Table` value is itself a handle, so
the same three methods work on it directly. The folder route above probes the
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

It says what it is, too: `IOBase::kind` on a `Table` is [`IOKind::Table`](enums.md)
rather than the `Directory` its root folder would answer, because the files below
a table are its storage and not its contents, and `is_tabular` is `true` without
touching storage at all. The folder route reaches the same shape by probing the
location for a metadata document; holding the table skips the probe, exactly as it
skips it everywhere else.

```rust
use yggdryl::generic::IORecordOptions;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::io::IOBase;
use yggdryl::local::Folder;
use yggdryl::{arrow, DataType, MimeType};

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let path = std::env::temp_dir().join("yggdryl-docs-iceberg-table-handle");
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

let arrow_schema = schema.to_arrow_schema()?;
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
table.append_arrow_batch_reader(rows(vec![1, 2], vec!["XNAS", "XNYS"]), &options)?;
let merging = options.clone().with_merge_by_names(["id"]);
table.write_arrow_batch_reader(rows(vec![2, 9], vec!["XNYS", "XLON"]), &merging)?;
assert_eq!(table.metadata().snapshots.len(), 2);
assert_eq!(table.current_snapshot().unwrap().operation(), "overwrite");

// A partition filter is answered by the scan plan, so the other partitions'
// files are never opened.
let filtered = options.clone().with_filter_partitions([("venue", "XNYS")]);
let matching: usize = table
    .read_arrow_batch_reader(&filtered)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(matching, 1);
```

A handle addressing one of the table's `column=value` directories addresses that
partition of it, exactly as it would in a plain Hive lake - the difference is that
the files come from the manifest rather than from a directory listing:

```rust
use yggdryl::generic::IORecordOptions;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::io::IOBase;
use yggdryl::local::Folder;
use yggdryl::{arrow, DataType};

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

let mut schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("venue"),
])?
.required_field("row");
assign_field_ids(&mut schema, 1)?;

let path = std::env::temp_dir().join("yggdryl-docs-iceberg-partition");
let _ = std::fs::remove_dir_all(&path);
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

let batch = RecordBatch::try_new(
    schema.to_arrow_schema()?,
    vec![
        Arc::new(Int64Array::from(vec![1_i64, 2])),
        Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
    ],
)?;
table.append(arrow::batch_reader(batch.schema(), [batch]))?;

let partition = Folder::new(path.join("data").join("venue=XNYS"))?;
let options = partition.record_options()?;
let rows: usize = partition
    .read_arrow_batch_reader(&options)?
    .map(|batch| batch.unwrap().num_rows())
    .sum();
assert_eq!(rows, 1);
```

## A warehouse of tables

!!! note "All three"
    The catalog crosses whole: Python has it as `yggdryl.iceberg.Catalog` and
    JavaScript as `iceberg.Catalog`, over the same warehouse folder and the
    same dotted names.

A caller who has rows and a dotted name should need nothing else. `Catalog` is that surface: one
warehouse folder, namespaces as nested folders, and a table per name - `HadoopCatalog`'s layout,
reached through [`IOBase`](io.md) and nothing else.

Storage sees three indistinguishable folders there, so each value says which role it plays:
`Catalog::kind` is [`IOKind::Catalog`](enums.md), `Namespace::kind` is `IOKind::Namespace`, and a
`Table` answers `IOKind::Table` through `IOBase::kind`. The framing is what tells them apart, so it
is the framing that answers - never a listing, and never a guess. (`IOKind` is Rust-only, as it is
everywhere else; the bindings ask the questions rather than name the kinds.)

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::Catalog;
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let warehouse = std::env::temp_dir().join("yggdryl-doc-warehouse");
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
    let arrow_schema = schema.to_arrow_schema()?;
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
        .append("nyc.trades", yggdryl::arrow::batch_reader(first.schema(), [first]))?;
    let rows_read: usize = table.scan(None)?.map(|batch| batch.map(|b| b.num_rows())).sum::<Result<usize, _>>()?;
    assert_eq!(rows_read, 2);

    let second = rows(&[3], &["XNAS"])?;
    catalog
        .tables()
        .append("nyc.trades", yggdryl::arrow::batch_reader(second.schema(), [second]))?;

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
    from yggdryl.iceberg import Catalog

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
    columns = pa.schema([child.to_arrow() for child in marked.data_type])

    table = catalog.append(
        "nyc.trades", pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns)
    )
    assert table.scan().read_all().num_rows == 2

    catalog.append("nyc.trades", pa.table({"id": [3], "venue": ["XNAS"]}, schema=columns))

    # The partition marks the schema carried became the table's spec.
    reopened = catalog.table("nyc.trades")
    assert [field.name for field in reopened.spec.fields] == ["venue"]
    assert reopened.scan().read_all().num_rows == 3
    assert catalog.has_table("nyc.trades")
    assert catalog.list_namespaces() == ["nyc"]
    assert catalog.list_tables("nyc") == ["nyc.trades"]

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
    catalog.createTable('nyc.trades', marked)

    const rows = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    const table = catalog.append('nyc.trades', rows([1n, 2n], ['XNAS', 'XNYS']))
    assert.equal(table.scan().toTable().numRows, 2)
    assert.equal(catalog.append('nyc.trades', rows([3n], ['XNAS'])).scan().toTable().numRows, 3)

    // The dotted name is the folder nyc/trades, and the marks became the spec.
    assert.ok(catalog.hasTable('nyc.trades'))
    assert.deepEqual(catalog.table('nyc.trades').spec.fields.map((field) => field.name), ['venue'])
    assert.deepEqual(catalog.listNamespaces(), ['nyc'])
    assert.deepEqual(catalog.listTables('nyc'), ['nyc.trades'])

    fs.rmSync(warehouse, { recursive: true, force: true })
    ```

`tables().create` is the explicit spelling - it numbers an unnumbered schema, derives the identity
spec from the schema's own [partition marks](field.md#a-field-can-be-a-partition-column), and
refuses a name that already has a table with a typed conflict. `append` and `overwrite` are
create-or-write. In Rust the collections are the one implementation and the catalog keeps exactly
two dotted entry points - `Catalog::table` and `Catalog::namespace` - because a dotted identifier is
a real Iceberg spelling and deserves one call; the bindings keep their flat conveniences as thin
delegates over the same views.

What is deliberately not here: `drop_table` and `rename_table`, because the storage contract has no
delete or move primitive, and a catalog must not emulate either by leaving a half-erased table
behind; and no catalog *service* client, because the module holds no network code. A REST catalog
is future work behind an HTTP storage backend.

### The object model: namespaces of tables

A catalog is namespaces of tables, and each collection is its own type: `catalog.namespaces` is a
lazy view of the namespaces, indexing it answers a `Namespace`, and `namespace.tables` is the same
shape one level down, indexing to a `Table`. A nested namespace is reached through its parent's
`namespaces` view, so access chains - `catalog.namespaces["sales"].tables["orders"]` - and every
collection operation has exactly one home. The views are cheap handles, not caches: constructing
one performs no I/O, membership and iteration consult storage at the moment they are asked, two
views over the same catalog observe each other's writes, and a missing name is a `KeyError` naming
it. JavaScript has no indexing hook a native class can answer, so the same questions are spelled
out there - `get`, `has`, `keys`, `names`, `size`, `create`, `openOrCreate` - over the same views,
and `for...of` walks a view's names lazily. Dotted names are resolved in the collections
themselves - `namespaces.get("sales.eu")` and `tables.get("sales.eu.orders")` descend - so the
resolution rule lives in one place.

Iterating a collection is lazy in all three languages: the names arrive one at a time, and `len` /
`size` drain the listing, so they cost the full level. In Rust `get` returns `Result` and nothing
implements `Index`: panic-on-missing is normal for an in-memory child lookup and is not normal for
a storage lookup - Python and JavaScript get the map spelling their readers expect instead, and
there is no `__delitem__` anywhere because removal is deliberately absent from the hierarchy.

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
    use yggdryl::iceberg::Catalog;
    use yggdryl::local::Folder;

    let root = std::env::temp_dir().join("yggdryl-doc-views");
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

    from yggdryl.iceberg import Catalog

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

    # The mapping surface: keys, values, and items, exactly as a dict's.
    assert list(sales.tables.keys()) == ["orders"]
    assert [name for name, _ in sales.tables.items()] == ["orders"]

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
    // The Map-like surface: lazy keys, and for...of walks them.
    assert.deepEqual([...sales.tables.keys()], ['orders'])
    assert.deepEqual([...sales.tables], ['orders'])

    const table = catalog.namespaces.get('sales').tables.get('orders')
    assert.equal(table.scan().toTable().numRows, 2)

    // A nested namespace is reached through its parent's own view.
    sales.namespaces.create('eu')
    assert.deepEqual(catalog.namespaces.get('sales').namespaces.names(), ['eu'])

    // A missing name is refused naming it, never answered as an empty table.
    assert.throws(() => catalog.namespaces.get('marketing'), /marketing/)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Data files aim at a size

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
    use yggdryl::iceberg::{Catalog, FormatVersion};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let warehouse = std::env::temp_dir().join("yggdryl-doc-compaction");
    let _ = std::fs::remove_dir_all(&warehouse);
    let catalog = Catalog::new(Folder::new(&warehouse)?);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
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
        table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
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
    table = catalog.create_table("tiny.rows", columns)
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
    const table = catalog.createTable('tiny.rows', [Field.from('id: int64')])
    assert.equal(table.targetFileSize, 512 * 1024 * 1024)

    // Five appends, five snapshots, five small files.
    for (const value of [0n, 1n, 2n, 3n, 4n]) {
      table.append(new arrow.Table({ id: arrow.vectorFromArray([value], new arrow.Int64()) }))
    }
    assert.equal(table.inspectFiles().toTable().numRows, 5)

    // Compaction rewrites the small groups as one replace commit and reports it.
    const compaction = table.compact()
    assert.equal(compaction.filesBefore, 5)
    assert.equal(compaction.filesAfter, 1)
    assert.ok(compaction.bytesRewritten > 0)
    assert.equal(table.scan().toTable().numRows, 5)

    // Nothing to do is a no-op that commits nothing.
    assert.deepEqual(table.compact(), { filesBefore: 0, filesAfter: 0, bytesRewritten: 0 })

    fs.rmSync(warehouse, { recursive: true, force: true })
    ```

Compaction groups live files by partition, touches only groups holding at least two files with one
under the target, and carries every other file into the new snapshot exactly as a merge carries the
files it never read. The snapshot before the compaction still time-travels: rewriting the present
never rewrites history.

## One options value, three layers

!!! note "All three"
    `IcebergOptions` is the same value in every language. Python takes it as
    `options=` and each field as its own keyword; JavaScript builds it from a
    plain object and passes it as the trailing argument of the calls that
    honour one.

Every knob a table honors lives on one value, `IcebergOptions`, and every field of it resolves
the same way: an explicit option set on the handle, then the table property of the same name
(falling back to the schema root's `iceberg:`-prefixed protocol property), then the documented
default. The keys are Iceberg's own spellings - `commit.retry.num-retries`,
`commit.retry.min-wait-ms`, `commit.retry.max-wait-ms`, `write.target-file-size-bytes`,
`write.format.default`, `read.parallelism`, `read.parallel.min-files`,
`read.parallel.min-file-size-bytes` - so a property another engine wrote configures this reader
too:

=== "Rust"

    ```rust
    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-options");
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
    assert_eq!(table.options()?.target_file_size_bytes(), 512 * 1024 * 1024);

    // The property layer is the table's own metadata, one commit away.
    table.commit_changes(|metadata| {
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
    import pytest

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergOptions, Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns)

    # Nothing set: every field answers its documented default.
    assert table.options().commit_retries == 4
    assert table.options().target_file_size == 512 * 1024 * 1024

    # The property layer is the table's own metadata, one commit away.
    table.update_properties({"commit.retry.num-retries": "9"})
    assert table.options().commit_retries == 9

    # An explicit override shadows the property on this handle alone; nothing
    # is written, and an unset field still resolves the other layers.
    table.set_options(IcebergOptions(commit_retries=2))
    assert table.options().commit_retries == 2
    assert table.options().commit_min_backoff_ms == 100

    # A keyword is the per-call layer: it configures this write and no later one.
    table.append(pa.record_batch({"id": [1]}, schema=columns), target_file_size=1 << 20)
    assert table.options().target_file_size == 512 * 1024 * 1024

    # A misspelled keyword is a TypeError naming it, not a knob doing nothing.
    with pytest.raises(TypeError, match="target_file_sze"):
        table.append(pa.record_batch({"id": [2]}, schema=columns), target_file_sze=1)

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
also scoped to what each operation consults: a commit resolves only the three `commit.retry.*`
keys, so an unparseable `read.*` property cannot stop the metadata-only commit that fixes it.

In Python, `IcebergOptions` is the same value, and every Iceberg method that takes it also takes
each field as its own keyword - `table.append(rows, target_file_size=..., commit_retries=...)` -
resolved by one rule: the `options=` argument (or the handle's stored override) is the base, an
explicit keyword wins over the same field on it, the passed object is never mutated, and a
misspelled keyword is a `TypeError` naming it. The generic `RecordOptions` is never accepted here:
Iceberg is a table format over the record encodings, and its configuration is its own.

JavaScript has no keyword arguments, so the value carries the whole surface instead: the
constructor takes an object naming any of the nine fields, every field is also a getter and a
setter, and a call that honours options takes one as its last argument -
`table.scan(field, options)`, `table.scanAt(id, filters, field, options)`,
`table.append(rows, options)`, `table.overwrite(rows, options)`, and the same trailing argument on
the tables view's `append` and `overwrite`. A per-call value is put back after the call, so it
never leaks into the handle's own override; `setOptions` is what changes that.

## The data file format

`write.format.default` - the spec's own property key - names the format new data files are written
in: `parquet`, the default, or `avro`. Like every option it resolves per call, per handle, or per
table, so one call can drop Avro files into a Parquet table; the manifest records the format each
file was *actually* written in, and a scan decodes each file as its manifest entry says, so a
table whose files mix formats still reads as one shape. A format the build cannot encode - `orc` -
is a typed error naming the key and the format before anything is written, never a silent fall
back to Parquet.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::iceberg::{FileFormat, FormatVersion, IcebergOptions, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-data-format");
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
        schema.to_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;

    // One Parquet append, then one Avro append via the explicit option.
    table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?;
    table.set_options(IcebergOptions::new().with_data_format(FileFormat::Avro));
    table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    // The manifest records what was written, and the mixed table scans whole.
    let mut formats: Vec<FileFormat> = table
        .data_files()?
        .into_iter()
        .map(|(file, _)| file.file_format)
        .collect();
    formats.sort();
    assert_eq!(formats, [FileFormat::Parquet, FileFormat::Avro]);
    assert_eq!(table.scan(None)?.count(), 2);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergOptions, Table, assign_field_ids

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = Table.create(
        IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"),
        assign_field_ids(schema),
    )

    # One Parquet append, then one Avro append - the option is one keyword.
    table.append(pa.table({"id": [1]}, schema=schema))
    table.append(pa.table({"id": [2]}, schema=schema), data_format="avro")

    formats = sorted(file.file_format for file, _ in table.data_files())
    assert formats == ["AVRO", "PARQUET"]
    assert table.scan().read_all().num_rows == 2

    # Stored per table, the spec's own key configures every writer.
    table.update_properties({"write.format.default": "avro"})
    assert table.options().data_format == "AVRO"
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

    // One Parquet append, then one Avro append - the option is the trailing
    // argument every write already takes.
    table.append(rows(1n))
    table.append(rows(2n), new iceberg.IcebergOptions({ dataFormat: 'avro' }))

    const formats = table.dataFiles().map((file) => file.fileFormat).sort()
    assert.deepEqual(formats, ['AVRO', 'PARQUET'])
    assert.equal(table.scan().toTable().numRows, 2)

    // Stored per table, the spec's own key configures every writer.
    table.updateProperties({ 'write.format.default': 'avro' })
    assert.equal(table.options().dataFormat, 'AVRO')

    // A format the build cannot encode is named before anything is written.
    assert.throws(
      () => table.append(rows(3n), new iceberg.IcebergOptions({ dataFormat: 'orc' })),
      /ORC/,
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Concurrent writers and commit retries

!!! note "Rust only"
    The commit gate is the core's, so every binding's writes retry through it
    and every binding sets the three `commit.retry.*` keys - as
    `commit_retries` and its two backoff neighbours, or as `commitRetries` and
    theirs. The race itself is shown once, in Rust, because staging it needs
    two handles and no rows.

Two writers holding the same table race the moment both commit, and what this module can promise
depends on what [`IOBase`](io.md) offers: positional reads and writes, no compare-and-swap. So the
one commit gate every write goes through re-checks the current version before writing, counts each
newer version it finds as being *beaten* once, and retries with jittered exponential backoff up to
`commit.retry.num-retries` times. What a retry does depends on the operation. An `append` and every
metadata-only `commit_changes` **rebase**: they reload the winner's document and re-apply their
intent on it - the data files and the manifest of added entries are written once and reused, only
the manifest list and the document are rebuilt - so both writers' rows survive in one line of
history:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-concurrency");
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

    let arrow_schema = schema.to_arrow_schema()?;
    let one = |id: i64| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )
    };

    let batch = one(1)?;
    left.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    // The right handle is now stale; its commit observes the winner,
    // rebases onto it, and lands as the next version.
    let batch = one(2)?;
    right.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

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
it; `yggdryl::local`'s memory mapping does not, and two processes truncating one mapped file at
the same instant is the documented SIGBUS hazard of that backend. A failed commit leaves no
visible change: at worst it leaves orphan data files no snapshot names.

## Branches and tags

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
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-branching");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.to_arrow_schema()?;
    let one = |id: i64| {
        RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )
    };

    let batch = one(1)?;
    table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    let audited = table.current_snapshot().expect("one commit").snapshot_id;

    // The tag pins the audited state; the table keeps moving.
    table.create_tag("audit-2026", audited)?;
    let batch = one(2)?;
    table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    table.create_branch("review", audited)?;

    // Every ref reads as the complete table it names.
    assert_eq!(table.scan_ref("audit-2026", &[], None)?.count(), 1);
    assert_eq!(table.scan_ref("review", &[], None)?.count(), 1);
    assert_eq!(table.scan(None)?.count(), 2);

    // A branch fast-forwards only along its own ancestry: the target must
    // reach the branch's head by parent ids, so no history can be lost.
    let head = table.current_snapshot().expect("two commits").snapshot_id;
    table.fast_forward("review", head)?;
    assert_eq!(table.snapshot_by_ref("review")?.snapshot_id, head);

    // Removing a ref removes the name; the snapshots stay retained.
    let removed = table.remove_ref("review")?;
    assert_eq!(removed.snapshot_id, head);

    // Expiry honors every ref's retention: the tagged snapshot survives
    // a cutoff that would otherwise expire everything old.
    assert!(table.expire_snapshots(i64::MAX)?.is_empty());

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
    assert.equal(table.scanRef('audit-2026').toTable().numRows, 1)
    assert.equal(table.scanRef('review').toTable().numRows, 1)
    assert.equal(table.scan().toTable().numRows, 2)

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

Each ref carries its own retention - `min-snapshots-to-keep` and `max-snapshot-age-ms` for a
branch's history, `max-ref-age-ms` for the ref itself - and `expire_snapshots` applies them before
its own age cutoff; `main` itself never expires. A branch or tag commits through the same retrying
gate as everything else, so two writers tagging at once behave like two writers appending at once.
Writing *to* a branch other than `main` remains future work: a commit's parent is always the
current snapshot, so today a branch is read with `scan_ref` and moved with `fast_forward`.

## Reading many files at once

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
    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-parallel-read");
    let _ = std::fs::remove_dir_all(&root);

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let arrow_schema = schema.to_arrow_schema()?;
    for id in 0..3 {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )?;
        table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
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
as fast as one; the numbers live in `rust/benchmarks/iceberg.rs` under `read/`.

## The Spark quickstart, locally

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
use yggdryl::generic::Holder;
use yggdryl::iceberg::Table;
use yggdryl::local::Folder;
use yggdryl::DataType;

let root = std::env::temp_dir().join("yggdryl-doc-nyc-taxis");
let _ = std::fs::remove_dir_all(&root);
let catalog = yggdryl::iceberg::Catalog::new(Folder::new(&root)?);

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
let arrow_schema = schema.to_arrow_schema()?;
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
table.append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))?;

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
table.merge(
    yggdryl::arrow::batch_reader(update.schema(), [update]),
    &["trip_id".to_owned()],
    true,
)?;
assert_eq!(fares(&table)?[0], (1_000_371, 16.32));
assert_eq!(fares(&table)?.len(), 4);

// DELETE FROM nyc.taxis WHERE vendor_id = 1
// A delete is a filtered overwrite with nothing incoming: the selected
// partition is replaced by no rows, and every other file is carried over.
table.overwrite_where(
    &[("vendor_id", "1")],
    yggdryl::arrow::batch_reader(Arc::clone(&arrow_schema), []),
)?;
assert_eq!(
    fares(&table)?,
    [(1_000_372, 22.15), (1_000_373, 9.01)],
);

// ALTER TABLE nyc.taxis ADD COLUMN fare_per_distance float
let mut update = yggdryl::iceberg::SchemaUpdate::for_metadata(table.metadata())?;
update.add_column("", DataType::Float32.nullable_field("fare_per_distance"));
let evolved = update.apply()?;
table.commit_changes(|metadata| {
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

## Schema evolution and field ids

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let path = std::env::temp_dir().join("yggdryl-docs-iceberg-evolution");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.to_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.append(arrow::batch_reader(batch.schema(), [batch]))?;

    // Add a column. Numbering continues above `last-column-id`, so the new column
    // can never be confused with a dropped one.
    let evolved = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Int64.nullable_field("quantity"),
    ])?
    .required_field("row");
    assert_eq!(table.evolve_schema(evolved)?, 1, "the new schema's id");

    // The old schema is retained, so the snapshot written under it still reads.
    assert_eq!(table.metadata().schemas.len(), 2);
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
    from yggdryl.iceberg import Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema)
    table.append(pa.record_batch({"id": [1]}, schema=columns))

    # Add a column. Numbering continues above `last-column-id`, so the new column
    # can never be confused with a dropped one.
    evolved = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("quantity", pa.int64()),
    ])
    assert table.evolve_schema(evolved) == 1, "the new schema's id"

    # The old schema is retained, so the snapshot written under it still reads.
    assert len(table.schemas) == 2
    assert len(table.schemas[0].data_type) == 1

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
    const evolved = fields.struct('row', [Field.from('id: int64'), Field.from('quantity: int64')], {
      nullable: false,
    })
    assert.equal(table.evolveSchema(evolved), 1, "the new schema's id")

    // The old schema is retained, so the snapshot written under it still reads.
    assert.equal(table.schemas.length, 2)
    assert.equal(table.schemas[0].dataType.length, 1)

    // And the file written before the column existed reads it as null.
    const rows = table.scan().toTable()
    assert.deepEqual(rows.schema.fields.map((child) => child.name), ['id', 'quantity'])
    assert.equal(rows.getChild('quantity').nullCount, rows.numRows)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

An Iceberg schema is a struct with *numbered* fields, and the number is the identity: a column read
by id survives a rename, and a new column can never reuse a retired id.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{assign_field_ids, last_field_id};
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
    assert_eq!(last_field_id(&schema)?, 3, "what a table records as last-column-id");

    // The root is not a column, so it is not numbered.
    assert_eq!(schema.parquet_field_id()?, None);

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert_eq!(assign_field_ids(&mut schema, 100)?, 100);
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl.iceberg import assign_field_ids

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
    assert [child.parquet_field_id for child in schema.data_type] == [1, 2]
    assert schema.data_type[1].data_type[0].parquet_field_id == 3

    # The root is not a column, so it is not numbered.
    assert schema.parquet_field_id is None

    # A field that already carries an id keeps it, so a second pass changes nothing.
    assert [child.parquet_field_id for child in assign_field_ids(schema, 100).data_type] == [1, 2]
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
    assert.equal(plain.dataType.at(0).parquetFieldId, null)
    assert.equal(schema.dataType.at(0).parquetFieldId, 1)
    assert.equal(schema.dataType.at(1).parquetFieldId, 2)
    assert.equal(schema.dataType.at(1).dataType.at(0).parquetFieldId, 3)

    // The root is not a column, so it is not numbered.
    assert.equal(schema.parquetFieldId, null)

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert.equal(iceberg.assignFieldIds(schema, 100).dataType.at(0).parquetFieldId, 1)
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
    use yggdryl::iceberg::schema_to_json;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let message = schema_to_json(&schema).unwrap_err().to_string();
    assert!(message.contains("assign_field_ids"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

    # A plain PyArrow schema carries no ids; creating the table numbers it.
    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = Table.create(IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"), columns)

    assert [child.parquet_field_id for child in table.schema.data_type] == [1]
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
    assert.equal(table.schema.dataType.at(0).parquetFieldId, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Evolving a schema

!!! note "All three"
    Python records the chain on `update_schema()` as a context manager,
    JavaScript on a builder ending in `commit()`, and `can_promote` /
    `canPromote` answers the promotion list everywhere.

A column change is a new schema, and `SchemaUpdate` is how one is built from the current one:
record the operations, apply, and commit the result. Only the promotions Iceberg allows are
accepted, so a change that would reinterpret stored values is refused naming both sides.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{can_promote, FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::local::Folder;
    use yggdryl::DataType;

    let root = std::env::temp_dir().join("yggdryl-doc-evolution");
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
    let mut update = SchemaUpdate::for_metadata(table.metadata())?;
    update.update_type("id", DataType::Int64);
    update.rename_column("symbol", "ticker");
    update.add_column("", DataType::Utf8.nullable_field("venue"));
    let evolved = update.apply()?;

    table.commit_changes(|metadata| {
        let schema_id = metadata.add_schema(evolved.clone())?;
        metadata.set_current_schema(schema_id)
    })?;

    let current = table.schema()?;
    assert_eq!(current.get_field_by_name("id").expect("the column").data_type(), &DataType::Int64);
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert_eq!(current.get_field_by_name("ticker").expect("the column").parquet_field_id()?, Some(2));
    assert_eq!(current.get_field_by_name("venue").expect("the column").parquet_field_id()?, Some(3));

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
    from yggdryl.iceberg import Table, can_promote

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

    children = list(table.schema.data_type)
    assert [child.name for child in children] == ["id", "ticker", "venue"]
    assert str(children[0].data_type) == "int64"
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
    assert.deepEqual(Array.from(evolved.dataType, (child) => child.name), ['id', 'ticker', 'venue'])
    assert.equal(String(evolved.dataType.at(0).dataType), 'int64')
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert.deepEqual(Array.from(evolved.dataType, (child) => child.parquetFieldId), [1, 2, 3])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

`TableMetadata` carries the rest of the update vocabulary - `set_property`/`remove_property`,
`set_location`, `assign_uuid`, `upgrade_format_version`, `set_snapshot_ref`/`remove_snapshot_ref`,
`remove_snapshots`, `add_spec`/`set_default_spec`, `add_sort_order`/`set_default_sort_order` - and
every one of them commits through the same `commit_changes`, which validates the whole document
before a byte of it is written. Dropping a column never frees its identifier: `last-column-id` only
grows, so a reader of an old file can never mistake a retired column for a new one.

## Schemas as documents

!!! note "All three"
    Both bindings read and write the document under the same two names, and
    both take it as the mapping their own JSON decoder produces.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{schema_from_json, schema_to_json};
    use yggdryl::{json, DataType};

    let document = json::from_str(
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
    assert_eq!(schema.fields()[0].data_type(), &DataType::Int64);

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert!(!schema.fields()[0].is_nullable());
    assert!(schema.fields()[1].is_nullable());
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(schema.fields()[0].get_metadata("PARQUET:field_id"), Some("1"));

    // The same document comes back out.
    assert_eq!(schema_to_json(&schema)?, document);
    ```

=== "Python"

    ```python
    import json

    from yggdryl.iceberg import schema_from_json, schema_to_json

    document = json.loads("""{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"},
        {"id":2,"name":"symbol","required":false,"type":"string"}
    ]}""")

    # An Iceberg schema is a non-null struct field; its columns are the children.
    schema = schema_from_json("row", document)
    assert schema.data_type.kind == "struct"
    assert not schema.nullable
    assert len(schema.data_type) == 2
    assert str(schema.data_type[0].data_type) == "int64"

    # `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert not schema.data_type[0].nullable
    assert schema.data_type[1].nullable
    assert schema.data_type[0].parquet_field_id == 1
    assert schema.data_type[0].metadata["PARQUET:field_id"] == "1"

    # The same document comes back out.
    assert schema_to_json(schema) == document
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
    assert.equal(schema.dataType.kind, 'struct')
    assert.equal(schema.nullable, false)
    assert.equal(schema.dataType.length, 2)
    assert.equal(String(schema.dataType.at(0).dataType), 'int64')

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert.equal(schema.dataType.at(0).nullable, false)
    assert.equal(schema.dataType.at(1).nullable, true)
    assert.equal(schema.dataType.at(0).parquetFieldId, 1)
    assert.equal(schema.dataType.at(0).get('PARQUET:field_id'), '1')

    // The same document comes back out.
    assert.deepEqual(iceberg.schemaToJson(schema).asJs(), document)
    ```

There is no Iceberg schema type in this module. An Iceberg schema *is* a non-null struct
[`Field`](field.md) whose children carry `PARQUET:field_id`, so the two functions convert rather than
mirror: what comes back is a field the rest of the crate already reads, writes, casts, and projects
into [Arrow](arrow.md).

Documents are read and written by the crate's own [JSON](json.md) parser, so an Iceberg document is
an ordinary [`Value`](json.md) - the same value a YAML or TOML document decodes to - and no second
JSON model enters the crate through this module.

Three things the field model spells differently, all of which survive the round trip:

- The root takes the `name` you pass, because an Iceberg schema names its columns but not itself.
- Iceberg states requirement and the core states nullability, so `"required": true` reads back as
  `is_nullable() == false` and writes back as `!field.is_nullable()`.
- `schema-id` is kept as `iceberg:schema-id` metadata on the root, a column's `doc` as `iceberg:doc`,
  and the v3 defaults as `iceberg:initial-default` and `iceberg:write-default`, which is why
  re-emitting the document reproduces it instead of dropping fields the field model has no slot for.

## Primitive types

!!! note "Rust only"
    The type mapping is a Rust table. The bindings see its result in the
    schema a table reports.

```rust
use yggdryl::iceberg::PrimitiveType;
use yggdryl::{DataType, TimeUnit};

// Every Iceberg primitive name has exactly one physical datatype.
assert_eq!(PrimitiveType::from_str("long")?.to_data_type()?, DataType::Int64);
assert_eq!(PrimitiveType::from_str("string")?.to_data_type()?, DataType::Utf8);
assert_eq!(
    PrimitiveType::from_str("decimal(18, 4)")?.to_data_type()?,
    DataType::decimal(18, 4)?
);

// Iceberg fixed every temporal resolution at microseconds until v3 added the
// nanosecond pair.
assert_eq!(
    PrimitiveType::from_str("timestamp")?.to_data_type()?,
    DataType::Timestamp(TimeUnit::Microsecond, None)
);
assert_eq!(
    PrimitiveType::from_str("timestamp_ns")?.to_data_type()?,
    DataType::Timestamp(TimeUnit::Nanosecond, None)
);
assert_eq!(
    PrimitiveType::from_str("time")?.to_data_type()?,
    DataType::time(TimeUnit::Microsecond)?
);

// A v3 `unknown` column always reads as null, which Arrow spells exactly.
assert_eq!(PrimitiveType::from_str("unknown")?.to_data_type()?, DataType::Null);

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
| `timestamp` | `Timestamp(Microsecond, None)` | v1 |
| `timestamptz` | `Timestamp(Microsecond, Some("UTC"))` | v1 |
| `timestamp_ns` | `Timestamp(Nanosecond, None)` | v3 |
| `timestamptz_ns` | `Timestamp(Nanosecond, Some("UTC"))` | v3 |
| `string` | `Utf8` | v1 |
| `uuid` | `FixedSizeBinary(16)` | v1 |
| `fixed[n]` | `FixedSizeBinary(n)` | v1 |
| `binary` | `Binary` | v1 |
| `unknown` | `Null` | v3 |

`to_data_type` is total: every Iceberg type materializes without loss. `from_data_type` is not, and
that is the point - it names the datatype it refuses instead of widening it behind your back:

```rust
use yggdryl::iceberg::PrimitiveType;
use yggdryl::DataType;

// The variants that differ only in physical layout collapse onto one name.
assert_eq!(PrimitiveType::from_data_type(&DataType::Utf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_data_type(&DataType::LargeUtf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_data_type(&DataType::BinaryView)?, PrimitiveType::Binary);
assert_eq!(
    PrimitiveType::from_data_type(&DataType::decimal64(9, 2)?)?,
    PrimitiveType::Decimal { precision: 9, scale: 2 }
);

// A datatype Iceberg cannot express is reported, never approximated.
let message = PrimitiveType::from_data_type(&DataType::Int8).unwrap_err().to_string();
assert!(message.contains("int8"));
assert!(PrimitiveType::from_data_type(&DataType::Int16).is_err());

// A UUID is 16 bytes on the wire and nothing more, so it writes back as `fixed[16]`.
assert_eq!(
    PrimitiveType::from_data_type(&PrimitiveType::Uuid.to_data_type()?)?.to_string(),
    "fixed[16]"
);
```

`int8`, `uint32`, `interval`, `union`, `decimal256`, and any `time` or `timestamp` unit other than
microsecond and nanosecond have no Iceberg spelling, and this conversion refuses them rather than
widening them behind your back - the column type in the table has to be one you chose.

Choosing it is one call: `iceberg` is a
[schema-compatibility target](datatype.md#compatibility-rewriting) like `spark` and `polars`, so the
widenings that are lossless are named in one place and applied by the one recursive walker.

```rust
use yggdryl::iceberg::PrimitiveType;
use yggdryl::{DataType, Scheme};

// The narrow integers widen; the refusals stay refusals.
let widened = DataType::Int8.to_scheme_compat(&Scheme::ICEBERG)?;
assert_eq!(widened, DataType::Int32);
assert_eq!(PrimitiveType::from_data_type(&widened)?.to_string(), "int");
assert!(DataType::Interval(yggdryl::TimeUnit::YearMonth).to_scheme_compat(&Scheme::ICEBERG).is_err());
```

## Nested types

!!! note "Rust only"
    The type mapping is a Rust table. The bindings see its result in the
    schema a table reports.

```rust
use yggdryl::iceberg::{schema_from_json, schema_to_json};
use yggdryl::{json, DataType};

let document = json::from_str(
    r#"{"type":"struct","fields":[
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
let DataType::List(element) = legs.data_type() else { panic!("expected a list") };
assert_eq!(element.name(), "element");
assert_eq!(element.parquet_field_id()?, Some(2));
assert!(!element.is_nullable());
assert_eq!(element.fields()[0].name(), "price");

// A map becomes a `Map` over a non-null `entries` struct of `key` and `value`.
let tags = &schema.fields()[1];
let DataType::Map(map) = tags.data_type() else { panic!("expected a map") };
assert_eq!(map.entries().name(), "entries");
assert!(!map.entries().is_nullable());
assert!(!map.entries().fields()[0].is_nullable());
assert!(map.entries().fields()[1].is_nullable());
assert_eq!(map.entries().fields()[0].parquet_field_id()?, Some(5));

assert_eq!(schema_to_json(&schema)?, document);
```

`struct`, `list`, and `map` nest to any depth. The names `element`, `key`, `value`, and `entries`
are synthesized, because Iceberg numbers those positions instead of naming them: `element-id`,
`key-id`, and `value-id` become field ids on the fields the conversion builds. A map key is always
required, so only `element-required` and `value-required` read as nullability; both default to
required when absent.

## Into a data file

!!! note "Rust only"
    The bindings commit through a table's append and overwrite, which write
    the same files; the writer's own settings stay in Rust.

```rust
use arrow_array::RecordBatch;
use yggdryl::arrow;
use yggdryl::iceberg::schema_from_json;
use yggdryl::io::Buffer;
use yggdryl::json;
use yggdryl::parquet::Parquet;

let document = json::from_str(
    r#"{"type":"struct","fields":[
        {"id":7,"name":"id","required":true,"type":"long"},
        {"id":8,"name":"symbol","required":false,"type":"string"}
    ]}"#,
)?;
let schema = schema_from_json("row", &document)?;

let mut media = Parquet::new(Buffer::new());
media.write_batch_reader(arrow::batch_reader(
    schema.to_arrow_schema()?,
    std::iter::empty::<RecordBatch>(),
))?;

// The ids Iceberg assigned are the ids in the file.
let written = media.read_field()?;
assert_eq!(written.fields()[0].parquet_field_id()?, Some(7));
assert_eq!(written.fields()[1].parquet_field_id()?, Some(8));
assert!(!written.fields()[0].is_nullable());
```

[parquet](parquet.md) has no Iceberg-specific code path. It writes `PARQUET:field_id` into the file
schema and reads it back, and because that is the same metadata key the conversion here uses, an
Iceberg schema needs no translation step before it becomes a data file - which is what lets a reader
resolve columns by id rather than by position.

## Interoperating with another implementation

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
  under its old name; the scan renames decoded columns to the current schema's names wherever the
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

## What is not here

No catalog client and no network code: nothing here resolves a table name, lists a namespace, or
speaks to a catalog service. Committing a snapshot means writing metadata somewhere, and *where* is
the [`IOBase`](io.md) handle the caller supplies.

No deletes. This module writes and reads data files; position and equality delete files are read as
manifest content that a scan skips, not produced.

No writes to a branch other than `main`: a commit's parent is always the current snapshot, so a
branch is read with `scan_ref` and moved with `fast_forward` until commits learn to parent a
branch's head.

No compare-and-swap. The commit gate re-checks the current version and retries when beaten, but
`IOBase` cannot make check-then-write atomic, so the guarantee is honest best-effort on plain
storage and exact only where the storage itself serializes writers.

Two partition transforms can place a row: `identity` and `void`. A write against a spec using
`bucket`, `truncate`, or a calendar transform is refused by name rather than silently writing rows
into the wrong partition; *reading* such a table is unaffected, because a manifest already records
which partition each file belongs to.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/iceberg-rust.ipynb){ download },
[Python](notebooks/iceberg-python.ipynb){ download },
[JavaScript](notebooks/iceberg-javascript.ipynb){ download }.

<!-- /notebooks -->
