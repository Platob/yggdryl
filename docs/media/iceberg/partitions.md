# Iceberg partitions

The spec a table is created with, the `column=value` directories it writes, and what a scan gets back out of them.

## Contract

| Key | Value |
| --- | --- |
| Owns | `PartitionSpec`, `Transform`, `partition_path`, `partition_field`, `mark_partitions`, `from_schema`, `from_partition_field`, `require_writable` |
| Layout | one directory level per partition field, `column=value`, under `data/`; the same renderer a partitioned folder write uses |
| Grouping key | the typed scalar tuple, so text or binary delimiter bytes cannot merge two partitions |
| Vocabulary | `field.as_iceberg()` and `as_iceberg_mut()` type the `ICEBERG:` properties `spec_id`, `partition_source_id` and `transform`; `is_partition` stays on the [`Field`](../../types/field.md) |
| Validated | `PartitionSpec::identity` resolves each name against the schema and numbers from 1000; `require_writable` checks every transform before a write pulls a row |
| Restored | identity values only, read off the manifest; every other transform leaves its source column to the data file, which still stores it |
| Refused | `Transform::Unknown` on any write, named by `require_writable`; reading a spec that carries one is fine |
| Bindings | Python and JavaScript create identity specs by column name and preserve every transform name when reading metadata; `PartitionSpec`, `Transform` and `partition_path` are Rust only |

## Use

A create names the identity partition columns, and every commit after it lays its data files out under one directory per partition value.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{StructType, arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-partition-use");
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

    // One data file per partition value, each under its own directory.
    let files = table.data_files()?;
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|(file, _)| file.file_path.contains("venue=XNAS")));
    assert!(files.iter().any(|(file, _)| file.file_path.contains("venue=XNYS")));
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

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, columns, ["venue"])
    table.append(pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns))

    # One data file per partition value, each under its own directory.
    files = table.data_files()
    assert len(files) == 2
    assert any("venue=XNAS" in file.path for file, _ in files)
    assert any("venue=XNYS" in file.path for file, _ in files)
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
        venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
      }),
    )

    // One data file per partition value, each under its own directory.
    const files = table.dataFiles()
    assert.equal(files.length, 2)
    assert.ok(files.some((file) => file.filePath.includes('venue=XNAS')))
    assert.ok(files.some((file) => file.filePath.includes('venue=XNYS')))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Partition specs and the Hive layout

Rust only. The bindings build identity specs and preserve every transform name when reading metadata.

A field carries its own Iceberg vocabulary: `field.as_iceberg()` and `as_iceberg_mut()` answer `IcebergField` and `IcebergFieldMut`, typing the `ICEBERG:` properties `schema_id`, `identifier_field_ids`, `doc`, `initial_default`, `write_default`, `spec_id`, `partition_source_id`, and `transform`. `is_partition` stays on the [`Field`](../../types/field.md), and the view borrows the whole field and dereferences to it.

```rust
use yggdryl::iceberg::{PartitionSpec, Transform, assign_field_ids};
use yggdryl::{DataType, Scalar, StructType};

let mut schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("venue"),
])?)
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

// The tuple describes itself, so the spec reads back off it; the view is the
// field, so the name and the property come off one value.
assert_eq!(partition.as_iceberg().spec_id()?, Some(1));
let venue = partition.get_field_by_path("venue").expect("the partition column");
assert!(venue.is_partition());
assert_eq!(venue.as_iceberg().transform()?, Some(Transform::Identity));
assert_eq!(venue.as_iceberg().get("transform"), Some("identity"));
assert_eq!(venue.as_iceberg().name(), "venue");
assert_eq!(PartitionSpec::from_partition_field(&partition)?, spec);

// And a schema that marks its own partition columns needs no column list.
let marked = spec.mark_partitions(&schema)?;
assert_eq!(marked.partition_field_names().collect::<Vec<_>>(), ["venue"]);
assert_eq!(PartitionSpec::from_schema(1, &marked)?, spec);

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

## A null partition value

A table marks its stored schema on create and on open, so `Table::schema` reports the layout from either end. The mark is core `Field` metadata, not an Iceberg document key, so it survives into Arrow and Parquet. A null partition value spells `null` in the data file's path, and the manifest keeps it null.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::{StructType, arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
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

## Edges

- `Transform::Unknown` in a spec -> readable metadata; `require_writable` and every write reject it.
- `venue=null` in a path -> the manifest is the authority; a path cannot separate the string `"null"` from an absent value.
- `days(at)` or `bucket(4, id)` partition -> restores no column; only `identity` values come from the manifest.
- A partition value -> nullable even when its source column is not, because a transform may answer null where the source cannot.
- A data file written under an earlier spec -> carried as it stands by a read; a [merge](write.md) that cannot exclude it by key bounds refuses it by name.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::partition_specs
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^partition/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_iceberg.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/iceberg.test.js
    ```
