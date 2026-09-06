# Iceberg

Read and write Apache Iceberg tables through one [`IOBase`](../../holder/index.md) handle, with no catalog between a folder and its table.

## Contract

| Key | Value |
| --- | --- |
| Owns | `yggdryl::media::iceberg`: `Table`, `TableMetadata`, `Snapshot`, `PartitionSpec`, `Transform`, `Catalog`, manifest readers and writers |
| Feature flag | `iceberg`, off by default; needs Rust 1.94 (default and schema-only builds keep 1.85); enables Parquet/Arrow 59 and official Iceberg 0.10.1 |
| Delegated | Metadata and schema mutation, validation, property parsing, manifest and list reads: official Iceberg 0.10.1; no Arrow 58 value crosses any API |
| Kept local | `IOBase` publication, the [`Field`](../../types/field.md)/Arrow 59 boundary, data-file writes, deterministic manifest and list writers, planning, scans |
| Layout | One `IOBase` container: `metadata/` holds documents, manifest lists, and manifests; `data/` holds record files |
| Open | `metadata/version-hint.text`, else the highest-numbered `*.metadata.json`; a foreign filename is kept for the next `metadata-log` entry |
| Commit | A new `v{version}.metadata.json` plus the hint; nothing is mutated in place, so earlier snapshots stay readable |
| Versions | v1: singular `schema` and `partition-spec`, no sequence numbers. v2: plural keys, `last-sequence-number`. v3: `next-row-id`, row lineage |
| Bindings | Python `yggdryl.media.iceberg` and the JavaScript `iceberg` namespace own the table; `TableMetadata`, `read_manifest`, and transform application are Rust only |

## Use

Create in a folder, append, and reopen with no catalog in between.

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

## Pages

Data files are [Parquet](../parquet.md) and manifests are [Avro](../avro.md); the [Media](../index.md) overview lists the other encodings.

| Page | Owns |
| --- | --- |
| [Iceberg reads](read.md) | Column pushdown, planning from metadata, time travel and inspection tables, filtered reads, parallel multi-file reads |
| [Iceberg writes](write.md) | The three record methods, data-file size targets and compaction, `IcebergOptions`, commit retries, branches and tags |
| [Iceberg schema](schema.md) | Evolution and field ids, `SchemaUpdate`, schemas as JSON, primitive and nested type mappings |
| [Iceberg catalog](catalog.md) | `Catalog` over one folder, namespaces of tables, the Spark quickstart run locally |

## The `iceberg` feature

`iceberg` is not a default feature.

```toml
[dependencies]
yggdryl = { version = "0.1", features = ["iceberg"] }
```

The boundary follows the [Iceberg specification](https://iceberg.apache.org/spec/) and the official [`TableMetadataBuilder`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableMetadataBuilder.html), [`TableProperties`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableProperties.html), and [`ManifestList`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.ManifestList.html) contracts.

## What a table writes

A commit adds one metadata document under `metadata/`; the earlier documents and their snapshots stay readable.

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

A commit first claims its version under a unique `00003-<uuid>` name, then publishes as `v{version}.metadata.json` and removes the claim. Every recorded location resolves relative to the table's own, so `file:/warehouse` and `file:///warehouse` name one folder and a table moves by rewriting locations.

## Table metadata, v1 through v3

Rust only. The bindings read the version a table declares as its `format_version`.

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

| Call | Behavior |
| --- | --- |
| `TableMetadata::from_json` | Parses and normalizes v1 through v3 through the official crate |
| `into_json` | Renders the deterministic public view, then validates the complete document with the official model |
| `Eq`, `Ord`, `Hash`, `stable_hash` | One canonical identity: keyed collections ignore document order; snapshot and metadata logs keep theirs |
| `statistics`, `partition-statistics`, v3 `encryption-keys` | Mutated through `TableMetadata` methods over the official builder |
| Round trip | Retains table and partition statistics, encryption keys, snapshot key and row-lineage fields, nanosecond temporals, `unknown`, column defaults |

## Snapshots and the current snapshot

Rust only. The bindings read the current snapshot and the snapshot list off a table, not off a document.

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

A snapshot is one complete table version: an identifier, its manifests, and a commit summary. Current snapshots use `manifest_list`; a v1 `manifests` list is preserved, exposed as `Snapshot.manifests`, and synthesized into `ManifestFile` rows for the same planner.

## Manifest lists and manifests

Two Avro levels sit between a snapshot and its rows: the manifest list, then each manifest.

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

| Reader or writer | Behavior |
| --- | --- |
| `read_manifest`, manifest-list read | Official parser after bounded input checks; keeps encryption, delete-file, split, bound, and v3 row-lineage fields |
| `read_manifest_spec` | Reads the bounded Avro header only; entries are never decoded |
| `read_manifest_for_plan` | Projects the validated view to file identity, partition, size, counts, and bounds; scans select it automatically |
| Writers | The core [`avro`](../avro.md) codec through `IOBase`; v3 follows the official row-id cursor rules, and a first post-upgrade commit assigns retained v2 files too |
| Statistics | From the Parquet footer just written; counts and sizes for every top-level column, bounds only where Parquet bytes equal the Iceberg encoding |

## Partition specs and the Hive layout

Rust only. The bindings build identity specs and preserve every transform name when reading metadata.

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

| Rule | Behavior |
| --- | --- |
| Write transforms | bucket, truncate, year, month, day, hour, identity, void, computed by the official scalar implementation |
| Grouping key | The typed scalar tuple, so text or binary delimiter bytes cannot merge partitions |
| Path | `partition_path` shares the renderer of a partitioned folder write, so [`Url::hive_partitions`](../../uri/patterns.md) and [`IOBase::children_where`](../../holder/iobase/partitions.md) walk a table as a lake |
| Data file | Still stores its partition columns, so a scan needs no restoration step |

### A field carries its own Iceberg vocabulary

`field.as_iceberg()` and `as_iceberg_mut()` answer `IcebergField` and `IcebergFieldMut`, typing the `iceberg:` properties `schema_id`, `identifier_field_ids`, `doc`, `initial_default`, `write_default`, `spec_id`, `partition_source_id`, and `transform`. `is_partition` stays on the [`Field`](../../types/field.md), and the view borrows the whole field and dereferences to it.

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

A table marks its stored schema on create and on open, so `Table::schema` reports the layout from either end. The mark is core `Field` metadata, not an Iceberg document key, so it survives into Arrow and Parquet.

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

## Interoperability

Both exchanges run in both directions and skip themselves, naming what is missing, rather than pass quietly.

| Exchange | Driver | Covers |
| --- | --- | --- |
| [PyIceberg](https://py.iceberg.apache.org/) | `python scripts/check_iceberg_interop.py` (needs `pyiceberg`); the Rust half is the `iceberg::` module of the `interop` test | A partitioned v2 table read as a `StaticTable`; a PyIceberg table with other file names, manifest field order, and deflate Avro |
| Apache Spark | `python scripts/setup_spark_interop.py`, then `pytest -m spark_interop` (deselected by default; needs Java) | Field ids, primitive and nested types with nulls, transforms, time travel and refs, evolution, properties, mixed Parquet and Avro, compaction, metadata tables, statistics |

## Edges

- `write.metadata.compression-codec` = `gzip` -> later commits write `.gz.metadata.json`; metadata is decoded by magic bytes; other codecs are rejected before publication.
- `current-snapshot-id` = `-1`, or no snapshot yet -> no current snapshot; the table scans as zero rows.
- Manifest file and row counts -> all six are optional; `None` means unreported, not zero.
- v3 row ids -> existing assignments are preserved, new manifest ranges are contiguous, and scans inherit missing data-file ids in manifest order.
- Manifest declaring a `fixed[16]` UUID partition -> the annotation is stripped, the 16 bytes kept, and the official parser retried; other failures return unchanged.
- Decimal column -> counts and sizes but no bounds, because Parquet and Iceberg encode it differently.
- `Transform::Unknown` in a spec -> readable metadata; `require_writable` and every write reject it.
- `venue=null` in a path -> the manifest is the authority; a path cannot separate the string `"null"` from an absent value.
- Renamed column -> resolved by field id, so a pre-rename file's column is renamed on read and pushed down under its own name.
- `days(at)` or `bucket(4, id)` partition -> restores no column; only `identity` values come from the manifest.
- `uuid`, `fixed`, `time` in Spark -> no DDL spelling, so the exchange covers only the direction that exists.
- `uuid` -> preserved as `uuid` through a metadata round trip, never demoted to `fixed[16]`.
- Remote catalog -> none; `Catalog` is an `IOBase` warehouse view, and commits publish through the supplied handle.
- Writing delete files, and applying deletes on read -> not implemented.
- Live position or equality delete manifests -> scans return a typed unsupported error, never undeleted rows; proven-inert manifests pass.
- Branch other than `main` -> no writes, since a commit's parent is always the current snapshot; read it with `scan_ref` and move it with `fast_forward`.
- Concurrent writers -> the commit gate re-checks the version and retries, but `IOBase` cannot make check-then-write atomic; exact only where storage serializes writers.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::tables
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::table_metadata
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::partition_specs
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::snapshot::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::interop_regressions
    cargo test --features "parquet iceberg" -p yggdryl --test interop iceberg::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^metadata/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^manifest/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^partition/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^identity/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_iceberg.py
    python/.venv/bin/python -m pytest python/tests/media/test_spark_interop.py -m spark_interop
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/iceberg.test.js
    ```

## Performance

Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1. The fastavro and PyIceberg baseline over the same manifest reads sits on [Apache Avro](../avro.md).

| Metadata operation | Median | Throughput |
| --- | ---: | ---: |
| Parse 100 snapshots and three 50-column schemas | 12.168 ms | 2.8613 MiB/s |
| Expire 99 of 100 snapshots | 9.5145 ms | 3.6592 MiB/s |
| Stable hash of the same metadata | 61.634 us | - |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^metadata/'
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^identity/'
```

The manifest rows share that host and toolchain.

| Manifest operation, 100,000 entries | Median | Throughput |
| --- | ---: | ---: |
| Full official-validated decode | 5.8718 s | 17.031 K entries/s |
| Spec/header only; entries untouched | 190.02 us | 526.26 M nominal entries/s |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^manifest/'
```
