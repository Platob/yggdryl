# Iceberg metadata

What a commit writes under `metadata/`: the table document, the snapshots it retains, and the two Avro levels between a snapshot and its rows.

## Contract

| Key | Value |
| --- | --- |
| Owns | `TableMetadata`, `Snapshot`, `read_manifest`, `read_manifest_spec`, `read_manifest_for_plan`, and the deterministic manifest and manifest-list writers |
| Layout | `metadata/` holds the documents, the manifest lists, and the manifests; `data/` holds the record files |
| Open | `metadata/version-hint.text`, else the highest-numbered `*.metadata.json`; a foreign filename is kept for the next `metadata-log` entry |
| Commit | one new `v{version}.metadata.json` plus the hint, claimed under a unique name first; nothing is mutated in place, so earlier snapshots stay readable |
| Validated | the official Iceberg 0.10.1 model, on every document read and on every document rendered, and the official parser on every manifest and manifest list |
| Lazy | `read_manifest_spec` reads the bounded Avro header alone, so a spec costs no entry decode; `read_manifest_for_plan` projects to what a plan reads |
| Cached | nothing beyond the document a `Table` already holds; a commit replaces it, and reopening rereads the hint |
| Refused | a metadata compression codec other than `gzip`; a document or manifest the official model rejects; a live position or equality delete manifest, named rather than silently undeleted |
| Bindings | Python and JavaScript read `format_version`, the current snapshot, the snapshot list and the manifests off a table; `TableMetadata`, `read_manifest`, and `read_manifest_spec` are Rust only |

## Use

A commit adds one metadata document under `metadata/`; the earlier documents and their snapshots stay readable.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-layout");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        LocalFolder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let names: Vec<String> = LocalFolder::new(&path)?
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

A commit first claims its version under a unique `00003-<uuid>` name, then publishes as `v{version}.metadata.json` and removes the claim. Every recorded location resolves relative to the table's own, so `file:/warehouse` and `file:///warehouse` name one folder and a table moves by rewriting locations.

## Table metadata, v1 through v3

Rust only. The bindings read the version a table declares as its `format_version`.

```rust
use yggdryl::iceberg::{FormatVersion, PartitionSpec, TableMetadata};
use yggdryl::DataType;
use yggdryl::StructType;

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
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

// A fresh table has no current snapshot, and `-1` is the other way a document
// spells "no current snapshot".
assert!(v2.current_snapshot().is_none());
let document = v2.clone().into_json()?.with_key("current-snapshot-id", -1_i64)?;
let read = TableMetadata::from_json(&document)?;
assert!(read.current_snapshot_id().is_none());
assert!(read.current_snapshot().is_none());

// Every version reads back as itself, with no current snapshot yet.
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

A table can have snapshots and still have no current one: a freshly created table, and a rolled-back one. An absent `current-snapshot-id` and `-1` both read as none, as the [metadata example](#table-metadata-v1-through-v3) asserts; Rust only, because the bindings read the current snapshot and the snapshot list off a table, not off a document.

A snapshot is one complete table version: an identifier, its manifests, and a commit summary. Current snapshots use `manifest_list`; a v1 `manifests` list is preserved, exposed as `Snapshot.manifests`, and synthesized into `ManifestFile` rows for the same planner.

## Manifest lists and manifests

Two Avro levels sit between a snapshot and its rows: the manifest list, then each manifest.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{
        EntryStatus, FormatVersion, PartitionSpec, Table, assign_field_ids, read_manifest,
        read_manifest_spec,
    };
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType, MimeType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-manifests");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec.clone())?;

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
    let handle = LocalFolder::new(&path)?.child_by_path(&format!("metadata/{name}"))?;
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
| Writers | The core [`avro`](../avro/index.md) codec through `IOBase`; v3 follows the official row-id cursor rules, and a first post-upgrade commit assigns retained v2 files too |
| Statistics | From the Parquet footer just written; counts and sizes for every top-level column, bounds only where Parquet bytes equal the Iceberg encoding |

## Edges

- `write.metadata.compression-codec` = `gzip` -> later commits write `.gz.metadata.json`; metadata is decoded by magic bytes; other codecs are rejected before publication.
- `current-snapshot-id` = `-1`, or no snapshot yet -> no current snapshot; the table scans as zero rows.
- Manifest file and row counts -> all six are optional; `None` means unreported, not zero.
- v3 row ids -> existing assignments are preserved, new manifest ranges are contiguous, and scans inherit missing data-file ids in manifest order.
- Manifest declaring a `fixed[16]` UUID partition -> the annotation is stripped, the 16 bytes kept, and the official parser retried; other failures return unchanged.
- Decimal column -> counts and sizes but no bounds, because Parquet and Iceberg encode it differently.
- `uuid` -> preserved as `uuid` through a metadata round trip, never demoted to `fixed[16]`.
- `unknown` and `variant` columns -> spelled by the crate's own schema serde; the official model sees `binary` under their field ids and never the names, in documents and manifest headers alike.
- A manifest whose header spells `unknown` or `variant` -> re-encoded in memory for the official reader once per read; every other manifest is read as it is.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::table_metadata
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- snapshot::references snapshot::branches snapshot::expiration snapshot::validation
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::interop_regressions
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^metadata/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^manifest/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_iceberg.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/iceberg.test.js
    ```

The measured numbers for both are on the [overview](index.md#performance).
