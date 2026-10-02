# Iceberg

Apache Iceberg tables in a folder: `metadata/` and `data/`, no catalog required, every file reached through the same handles as any other medium - metadata as JSON, manifests as Avro, data files as Parquet by default.

## Overview

| | |
| --- | --- |
| Declared by | a table folder - `metadata/` and `data/` beneath it |
| Build | the `iceberg` feature, which implies `parquet` |
| Rust | `yggdryl::iceberg`: `Table` (`create`, `open`, `open_or_create`, `scan`, `scan_matching`, `plan_matching`, `commit_append`, `commit_overwrite`, `update_schema`), `PartitionSpec`, `PartitionField`, `Transform`, `SortOrder`, `SchemaUpdate`, `IcebergOptions` and the `Catalog` over folders of tables |
| Python | `yggdryl.iceberg`: `Table` (`create`, `open`, `scan`, `scan_matching`, `plan_matching`, `append`, `overwrite`, `update_schema`), `PartitionSpec`, `SchemaUpdate`, `IcebergOptions`, `Catalog` |
| JavaScript | `iceberg`: `Table` (`create`, `open`, `scan`, `scanMatching`, `planMatching`, `append`, `overwrite`, `merge`, `updateSchema`), `PartitionSpec`, `IcebergOptions`, `Catalog` |
| Settings | `IcebergOptions`, each resolved from the call, then the table property, then the default: `read.parallelism`, `write.parallelism`, `write.target-file-size-bytes` and the commit retries among them |

A table lives in one folder: `metadata/` and `data/`, no catalog required.
Iceberg's type strings - `timestamptz`, `fixed[16]`, `list<fixed[16]>`, the
`struct<1: a: optional long>` its reference implementations render - are
[datatype spellings](../types/datatype.md): each primitive reads as the
datatype the table reader maps it to, and a struct member keeps its id and
nullability. A list's or map's string carries no element id or element
nullability, so its child is the grammar's nullable `item`. A v3 `variant` and a v3 `unknown` both read as [`variant`](../types/variant.md); only a table's schema declares a column `unknown`, which it keeps out of its data files and reads back as nulls. A v1 or v2 table refuses both.

## Read

A table is opened from its folder - its newest metadata document, through the version hint where one is written and by listing `metadata/` where another writer left none - with no catalog in between. A scan plans snapshot, manifest list, manifests, then data files, from the metadata and never by walking `data/`: the manifest list's partition summaries skip whole manifests, a manifest's partition tuples and column statistics skip files, and the rest of the predicate filters the rows of the files kept. A scan decodes its files side by side once two of at least 64 KiB qualify (`read.parallel.min-files`, `read.parallel.min-file-size-bytes`), and the files in flight share `read.parallelism` with the columns inside them; it answers in plan order, so it differs from a sequential scan only in speed.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array as _, Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{arrow, DataType, StructType};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-read");
    let _ = std::fs::remove_dir_all(&path);

    // Four commits into a venue-partitioned table: one manifest each.
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON"), (4, "XLON")] {
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema()?,
            vec![Arc::new(Int64Array::from(vec![id])), Arc::new(StringArray::from(vec![venue]))],
        )?;
        table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // A table is opened from its folder, with no catalog in between.
    let table = Table::open(LocalFolder::new(&path)?)?;

    // The manifest list's partition summaries settle a predicate on the
    // partition column before a manifest is opened.
    let plan = table.plan_matching("venue = 'XNYS'")?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.manifests_skipped(), 3);

    // The partition prunes the files, and the rest of the filter the rows.
    let mut ids = Vec::new();
    for batch in table.scan_matching("id >= 4 and venue = 'XLON'", None)? {
        let batch = batch?;
        let column = batch.column(0).as_any().downcast_ref::<Int64Array>().expect("the ids");
        ids.extend(column.values().iter().copied());
    }
    assert_eq!(ids, [4]);
    let _ = std::fs::remove_dir_all(&path);
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

    # Four commits into a venue-partitioned table: one manifest each.
    created = Table.create(root, schema, ["venue"])
    for identifier, venue in ((1, "XNAS"), (2, "XNYS"), (3, "XLON"), (4, "XLON")):
        created.append(pa.record_batch({"id": [identifier], "venue": [venue]}, schema=schema))

    # A table is opened from its folder, with no catalog in between.
    table = Table.open(root)

    # The manifest list's partition summaries settle a predicate on the
    # partition column before a manifest is opened.
    plan = table.plan_matching("venue = 'XNYS'")
    assert (plan["tasks"], plan["manifests_skipped"]) == (1, 3)

    # The partition prunes the files, and the rest of the filter the rows.
    rows = table.scan_matching("id >= 4 and venue = 'XLON'").read_all()
    assert rows.column("id").to_pylist() == [4]
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

    // Four commits into a venue-partitioned table: one manifest each.
    const created = iceberg.Table.create(root, schema, ['venue'])
    for (const [id, venue] of [[1n, 'XNAS'], [2n, 'XNYS'], [3n, 'XLON'], [4n, 'XLON']]) {
      created.append(
        new arrow.Table({
          id: arrow.vectorFromArray([id], new arrow.Int64()),
          venue: arrow.vectorFromArray([venue], new arrow.Utf8()),
        }),
      )
    }

    // A table is opened from its folder, with no catalog in between.
    const table = iceberg.Table.open(root)

    // The manifest list's partition summaries settle a predicate on the
    // partition column before a manifest is opened.
    const plan = table.planMatching("venue = 'XNYS'")
    assert.equal(plan.tasks, 1)
    assert.equal(plan.manifestsSkipped, 3)

    // The partition prunes the files, and the rest of the filter the rows.
    const rows = table.scanMatching("id >= 4 and venue = 'XLON'").intoTable()
    assert.deepEqual([...rows.getChild('id')], [4n])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Write

A write commits a snapshot: an append keeps every row the table holds, an overwrite replaces them, and a merge updates the rows its key matches and appends the rest. A partitioned write groups each batch by vectorized keys and computes a partition tuple once per distinct key, not once per row, writing one data file per partition the commit touches; a commit shares `write.parallelism` between its partitions and their columns. New data files are Parquet unless `data_mime_type` names another encoding, and a scan reads each file as its manifest entry records, so one table can mix them.

A streamed write with no `commit_batch_num` commits a snapshot each time the batches it holds reach the table's target file size (`write.target-file-size-bytes`, `IcebergOptions`' `target_file_size`) as `yggdryl::arrow::memory_size` measures them, then the remainder, so a stream of any length holds at most one target file of rows before each commit; `commit_batch_num = N` commits every `N` whole batches instead. An overwrite's first commit replaces and the rest append; an append or a merge keeps its intent in every commit.

A commit beaten by another writer is retried: an append and a metadata-only commit rebase onto the winner, and an overwrite, a merge or a compaction restores the state it read; a failed commit may leave files no metadata names.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
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

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-lead");
    let _ = std::fs::remove_dir_all(&path);

    // A table is created in a folder, and a folder is all it ever touches.
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

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
    let reopened = Table::open(LocalFolder::new(&path)?)?;
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

## Partition transforms

A partition spec names one transform per field, `Transform` in Rust. The specification's own are read and written as it spells them; three more - `minutes[n]`, `week` and `quarter` - are this crate's own, and no other implementation knows them: Apache Iceberg's Java implementation and PyIceberg read a name they do not know as `unknown` and prune nothing by it, as the specification says of an unknown transform, while iceberg-rust 0.10 refuses a metadata or manifest document naming one, so a table partitioned or sorted by one does not open there. Every time transform is the expression grammar's [epoch function](../expression/functions.md#calendar-parts-and-epoch-periods) of the same name, computed by one rule and floored, so an instant before 1970 lands in its own period - and because a file's tuple names the period every row's source falls in, a filter on the source column prunes files and manifests by it, with no partition column named in the filter, a column two fields read pruning by the tighter of them. A writer that truncated an instant before 1970 toward zero filed it one period late, which Java's reader allows for; a scan here allows for it the same way, so a period at or below zero of the specification's `year`, `month`, `day` and `hour` (of a date, `year` and `month`) also keeps the instants of the period before it. A `bucket` or a `truncate` prunes nothing. A timestamp source of a time transform is counted in microseconds or nanoseconds, the two units Iceberg spells, and one in seconds or milliseconds is refused by all seven alike. Spark's DDL plurals - `years`, `months`, `days`, `hours`, `weeks`, `quarters` - are intake spellings, read and written singular. `minutes[n]` takes its step in brackets as `bucket[n]` does - `minutes[15]` the quarter hour, `minutes[30]` the half hour, `minutes[60]` the hour - with `n` from 1 to 2147483645, and has that one spelling: `minutes[0]`, `minutes(15)`, `minutes[+15]` and a bare `minutes` are refused by name.

| Transform | Also read | Source | Partition value | Grammar function | Whose |
| --- | --- | --- | --- | --- | --- |
| `identity` | | any primitive | the value | | specification |
| `bucket[n]` | `bucket(n)` | int, long, decimal, date, time, timestamp, string, uuid, fixed, binary | `int32` hash bucket | | specification |
| `truncate[w]` | `truncate(w)` | int, long, decimal, string, binary | the value shortened | | specification |
| `year` | `years` | date, timestamp | `int32` years since 1970 | `years(x)` | specification |
| `month` | `months` | date, timestamp | `int32` months since 1970-01 | `months(x)` | specification |
| `day` | `days` | date, timestamp | `date32` the UTC day | `days(x)` | specification |
| `hour` | `hours` | timestamp | `int32` hours since the epoch | `hours(x)` | specification |
| `minutes[n]` | | timestamp | `int32` periods of `n` minutes since the epoch | `minutes(x, n)` | this crate |
| `week` | `weeks` | date, timestamp | `int32` Monday-start weeks since Monday 1969-12-29 | `weeks(x)` | this crate |
| `quarter` | `quarters` | date, timestamp | `int32` quarters since 1970-Q1 | `quarters(x)` | this crate |
| `void` | | any | null | | specification |
| `unknown` | | any | not computed; a spec holding one is not written to | | specification |

In the table's metadata the three cross the Apache Iceberg model this crate validates with as reserved bucket counts above `i32::MAX` - `minutes[n]` as `bucket[2147483648 + n]`, `quarter` as `bucket[4294967294]`, `week` as `bucket[4294967295]` - and come back as themselves; the metadata and manifest files on disk spell the names above, never the bucket, so another writer of this crate reads them. A `bucket[n]` above `i32::MAX`, built in code or stated by a metadata document or a manifest header, is refused by its count rather than read as one of the three, and because a bucket binds to sources a period cannot read, the source of each of the three is judged by the period's own rule - `minutes[n]` over an `int64` or a date is refused naming the transform and the type - wherever a spec or a sort order meets a schema: a table created or read, a spec or an order added. `Transform::from_term` and `Transform::into_term` map a grammar call to its transform and back - `minutes(ts, 15)` is `minutes[15]` - for a partition declaration spelled as an expression; `Transform::function` / `Transform::from_function` are the parameter-free half of that mapping (Rust-only).

A table partitioned by `minutes[15]` writes one data file per quarter hour its rows fall in, and a filter on the timestamp skips the files whose period cannot hold it:

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionField, PartitionSpec, Table, Transform, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, Scalar, StructType, TimeUnit, Timezone, arrow};

    use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }.required_field("ts"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    // `minutes[15]` of `ts` (field id 2): one partition per quarter hour since the epoch.
    let spec = PartitionSpec {
        spec_id: 0,
        fields: vec![PartitionField {
            source_id: 2,
            field_id: 1000,
            name: "ts_minutes".into(),
            transform: Transform::from_str("minutes[15]")?,
        }],
    };
    assert!(Transform::from_str("minutes(15)").is_err(), "one spelling");

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-minutes");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01: three quarter hours.
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2, 3, 4])),
            Arc::new(TimestampMicrosecondArray::from(vec![0_i64, 899_000_000, 900_000_000, 3_600_000_000])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let mut periods: Vec<Scalar> = table.data_files()?.into_iter().map(|(file, _)| file.partition[0].clone()).collect();
    periods.sort();
    assert_eq!(periods, vec![Scalar::from(0), Scalar::from(1), Scalar::from(4)]);

    // A filter on `ts` itself skips the files whose period cannot hold it.
    let early = table.plan_matching("ts < '1970-01-01T00:15:00'")?;
    assert_eq!(early.tasks.len(), 1);
    assert_eq!(early.files_skipped(), 2);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import PartitionSpec, Table

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("ts", pa.timestamp("us"), nullable=False),
    ])
    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "ticks")

    # `minutes[15]` of `ts`, the second column (field id 2): one partition per
    # quarter hour since the epoch.
    spec = PartitionSpec.from_json({
        "spec-id": 0,
        "fields": [{"name": "ts_minutes", "transform": "minutes[15]", "source-id": 2, "field-id": 1000}],
    })
    table = Table.create(root, schema, spec)

    # 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01: three quarter hours.
    table.append(pa.record_batch(
        {"id": [1, 2, 3, 4], "ts": pa.array([0, 899_000_000, 900_000_000, 3_600_000_000], pa.timestamp("us"))},
        schema=schema,
    ))
    assert sorted(file.partition for file, _ in table.data_files()) == [(0,), (1,), (4,)]
    assert table.scan().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('ts: timestamp(us)')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'ticks')

    // `minutes[15]` of `ts`, the second column (field id 2): one partition per
    // quarter hour since the epoch.
    const spec = iceberg.PartitionSpec.fromJSON({
      'spec-id': 0,
      fields: [{ name: 'ts_minutes', transform: 'minutes[15]', 'source-id': 2, 'field-id': 1000 }],
    })
    const table = iceberg.Table.create(root, schema, spec)

    // 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01, as Arrow JS's milliseconds.
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n, 3n, 4n], new arrow.Int64()),
        ts: arrow.vectorFromArray([0, 899_000, 900_000, 3_600_000], new arrow.TimestampMicrosecond()),
      }),
    )
    const periods = table
      .dataFiles()
      .map((file) => file.partition[0].asJs())
      .sort((left, right) => left - right)
    assert.deepEqual(periods, [0, 1, 4])
    assert.equal(table.scan().intoTable().numRows, 4)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Declared partitioning and sort order

A table is also created from what its schema declares. `PartitionSpec::from_schema` reads the root's [`PARTITION:by`](../types/protocol.md#partition-columns) - a bare column an identity field, an epoch function over a column its transform, `truncate(col, w)` a truncation, each named by its alias or by the convention (`ts_minutes`, `name_truncate`), anything else refused by name - and `Table::create` reads the root's [`SORT:by`](../types/protocol.md#sort-order) as the default sort order, `SortOrder::for_spec` where it declares none. The declarations are written on the root rather than through `with_partition_by`, because a derived partition value lives in the manifest and not in the rows. `Table::schema()` reports both keys back, `mark_partitions` writing the spec's fields the grammar can spell (a `bucket` has no spelling and is left out) and the default order its keys, so a reopened table says how it partitions and sorts. A partition group whose rows already arrive in the table's order is written as it arrived.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, SortOrder, Table, Transform, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType, TimeUnit, Timezone};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
        DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }.required_field("ts"),
    ])?)
    .required_field("row");
    schema.as_partition_mut().set_by_texts(["venue", "minutes(ts, 15)"])?;
    schema.as_sort_mut().set_by_texts(["ts desc", "id"])?;
    assign_field_ids(&mut schema, 1)?;

    let spec = PartitionSpec::from_schema(1, &schema)?;
    assert_eq!(spec.fields[1].transform, Transform::Minutes(15));
    assert_eq!(spec.fields[1].name, "ts_minutes");
    assert_eq!(SortOrder::from_schema(1, &schema)?.fields.len(), 2);

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-declared");
    let _ = std::fs::remove_dir_all(&path);
    let table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema, spec)?;
    assert_eq!(table.metadata().default_sort_order()?.fields[0].direction, "desc");
    assert_eq!(table.schema()?.get_metadata("PARTITION:by"), Some(r#"["venue","minutes(ts, 15)"]"#));
    assert_eq!(table.schema()?.get_metadata("SORT:by"), Some(r#"["ts desc","id"]"#));
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import DataType, Field, IOBase
    from yggdryl.iceberg import Table

    schema = Field(
        "row",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field("venue", "utf8", nullable=False),
            Field("ts", "timestamp(us)", nullable=False),
        ]),
        nullable=False,
    )
    schema.partition.by = ["venue", "minutes(ts, 15)"]
    schema.sort.by = ["ts desc", "id"]
    root = pathlib.Path(tempfile.mkdtemp())

    # Omitted, the table partitions and sorts as its schema declares.
    table = Table.create(IOBase(root / "declared"), schema)
    assert [(field.name, field.transform) for field in table.spec.fields] == [
        ("venue", "identity"),
        ("ts_minutes", "minutes[15]"),
    ]
    assert table.schema.partition.by == ["venue", "minutes(ts, 15)"]
    assert table.schema.sort.by == ["ts desc", "id"]

    # Stated, the entries are read by the same rule and replace the declaration.
    stated = Table.create(IOBase(root / "stated"), schema, ["days(ts)", "truncate(venue, 4) as prefix"])
    assert [(field.name, field.transform) for field in stated.spec.fields] == [
        ("ts_day", "day"),
        ("prefix", "truncate[4]"),
    ]

    # `None` partitions nothing, whatever the schema declares.
    assert Table.create(IOBase(root / "flat"), schema, None).spec.is_unpartitioned()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { DataType, Field, iceberg } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('venue', 'utf8', false),
        new Field('ts', 'timestamp(us)', false),
      ]),
      false,
    )
    schema.partition.by = ['venue', 'minutes(ts, 15)']
    schema.sort.by = ['ts desc', 'id']
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // Omitted, the table partitions and sorts as its schema declares.
    const table = iceberg.Table.create(path.join(root, 'declared'), schema)
    assert.deepEqual(
      table.spec.fields.map((field) => [field.name, field.transform]),
      [['venue', 'identity'], ['ts_minutes', 'minutes[15]']],
    )
    assert.deepEqual(table.schema.partition.by, ['venue', 'minutes(ts, 15)'])
    assert.deepEqual(table.schema.sort.by, ['ts desc', 'id'])

    // Stated, the entries are read by the same rule and replace the declaration.
    const stated = iceberg.Table.create(path.join(root, 'stated'), schema, ['days(ts)', 'truncate(venue, 4) as prefix'])
    assert.deepEqual(
      stated.spec.fields.map((field) => [field.name, field.transform]),
      [['ts_day', 'day'], ['prefix', 'truncate[4]']],
    )

    // `null` partitions nothing, whatever the schema declares.
    assert.equal(iceberg.Table.create(path.join(root, 'flat'), schema, null).spec.isUnpartitioned(), true)

    fs.rmSync(root, { recursive: true, force: true })
    ```

`Table.create` and `open_or_create` take the partitioning as a `PartitionSpec` or as `PARTITION:by` entries - Python's `partition_by`, JavaScript's `partitionBy`, each entry its text, in Python a `Term` or a `(term, alias)` pair too - read by `PartitionSpec::from_schema`'s rule, so a refusal names the entry. Omitted, the schema's own `PARTITION:by` is read; `None` in Python and `null` in JavaScript - or an empty list - partition nothing whatever the schema declares. The default sort order is the schema's `SORT:by` either way.

## Schema evolution

A `SchemaUpdate` records column operations - add, rename, drop, promote - and one commit replays them onto the schema that commit attempt reads, so a commit beaten by another writer rebases onto the winner's schema rather than overwriting it. Field IDs are kept and a dropped one is never reused; promotions are `int32 -> int64`, `float32 -> float64`, same-scale decimal widening and, on a v3 table, an `unknown` column to any type, which clears its [`unknown` declaration](../types/variant.md#edges); any other change into or out of `unknown`, `variant` or `binary` is refused. The commit answers the schema id it made current, and an update that recorded nothing writes nothing and answers the current one.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch};
    use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::local::LocalFolder;
    use yggdryl::{arrow, DataType, StructType};

    let mut schema = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
        .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-evolve");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
    let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int32Array::from(vec![1]))])?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.add_column("", DataType::utf8().nullable_field("note"));
    update.update_type("id", DataType::Int64);
    assert_eq!(table.update_schema(&update)?, 1);

    // Nothing recorded, nothing written: the current id comes back.
    let unchanged = SchemaUpdate::from_metadata(table.metadata())?;
    assert_eq!(table.update_schema(&unchanged)?, 1);

    let first = table.scan(None)?.next().expect("one batch")?;
    assert_eq!(first.column(0).data_type(), &arrow_schema::DataType::Int64);
    assert_eq!(first.column(1).null_count(), 1);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import Field, IOBase
    from yggdryl.iceberg import Table

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, pa.schema([pa.field("id", pa.int32(), nullable=False)]))
    table.append(pa.table({"id": pa.array([1], pa.int32())}))

    schema_id = table.update_schema().add_column("", Field("note", "utf8")).update_type("id", "int64").commit()
    assert schema_id == 1

    # Nothing recorded, nothing written: the current id comes back.
    assert table.update_schema().commit() == 1

    assert [child.name for child in table.schema.dtype] == ["id", "note"]
    assert table.scan().read_all().column("note").to_pylist() == [None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.Table.create(root, fields.struct('row', [new Field('id', 'int32', false)], { nullable: false }))
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1], new arrow.Int32()) }))

    const schemaId = table.updateSchema().addColumn('', Field.from('note: utf8')).updateType('id', 'int64').commit()
    assert.equal(schemaId, 1)

    // Nothing recorded, nothing written: the current id comes back.
    assert.equal(table.updateSchema().commit(), 1)

    assert.deepEqual([...table.schema.dtype].map((child) => child.name), ['id', 'note'])
    assert.deepEqual([...table.scan().intoTable().getChild('note')], [null])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Performance

Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1. The fastavro and PyIceberg baseline over the same manifest reads sits on [Avro](avro.md#performance).

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

### Against PyIceberg

One run of `python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5` beside PyIceberg 0.11.1 with its SQLite catalog, on one local warehouse: Intel Xeon @ 2.10 GHz, 4 cores, a shared virtual host; rustc 1.97.0, CPython 3.11.15, PyArrow 25.0.1, release wheel. Each append writes 1,048,576 six-column rows, one second apart so they span fourteen UTC days, into a fresh table in three layouts: unpartitioned, eight partitions by `symbol`, fourteen by `day(ts)`. Both readers then read the table PyIceberg wrote, so they decode the same files, and what both read is compared before anything is timed. The ratio is PyIceberg's median over this crate's, so above one is in this crate's favor. On this host the median of an unchanged scan moved by up to a quarter between runs, so a ratio that close to one is a tie.

| append | this crate | PyIceberg | ratio |
| --- | ---: | ---: | ---: |
| unpartitioned | 136.04 ms | 246.92 ms | 1.82 |
| 8 partitions by `symbol` | 280.12 ms | 320.94 ms | 1.15 |
| 14 partitions by `day(ts)` | 209.32 ms | 189.34 ms | 0.90 |

Each read cell is this crate's median, then PyIceberg's, then the ratio.

| read | unpartitioned | 8 partitions by `symbol` | 14 partitions by `day(ts)` |
| --- | ---: | ---: | ---: |
| open | 0.99 / 0.65 ms, 0.66 | 1.11 / 0.69 ms, 0.62 | 1.46 / 0.70 ms, 0.48 |
| scan everything | 29.52 / 56.49 ms, 1.91 | 33.12 / 45.26 ms, 1.37 | 37.98 / 53.26 ms, 1.40 |
| scan `symbol = 'AAPL'` | 36.77 / 62.10 ms, 1.69 | 8.33 / 24.92 ms, 2.99 | 40.88 / 63.06 ms, 1.54 |
| scan `price > 900` | 31.57 / 62.98 ms, 2.00 | 39.58 / 54.21 ms, 1.37 | 38.41 / 63.90 ms, 1.66 |
| scan `id, price` | 24.65 / 25.83 ms, 1.05 | 21.36 / 27.33 ms, 1.28 | 26.99 / 38.43 ms, 1.42 |
| scan one day of `ts` | 33.52 / 77.15 ms, 2.30 | 31.99 / 51.38 ms, 1.61 | 7.91 / 20.32 ms, 2.57 |

The appends pay one thing PyIceberg's do not: every file published on local storage is flushed to the device before the metadata that names it, so a crash cannot leave the table pointing at a file the disk never received. A file costs about 2 ms of that here, which is why the gap narrows as the files multiply. An unpartitioned file also encodes its columns on every thread, while a partitioned append writes its groups one file per thread. A group's rows are sliced where they are one run of a batch, as every day of these rows is, and gathered where partitions interleave, as `symbol` does. A group already in the table's sort order is neither joined nor copied.

Opening is the one row behind. PyIceberg is handed the metadata location, while this crate finds it - a table PyIceberg's catalog wrote has no version hint, so the metadata folder is listed - and parses the document twice, once as a value and once through the official crate's validating reader.

The script ends with the same rows appended into a table partitioned by `minutes(ts, 15)`, a transform PyIceberg cannot write. That append is 1,166 data files, and a scan of one hour of `ts` keeps four of them and skips 1,162.

| `minutes(ts, 15)`, this crate alone | median |
| --- | ---: |
| append | 993.95 ms |
| scan one hour of `ts` | 97.62 ms |

About 90 ms of that scan is planning: the one manifest's 1,166 entries are decoded to keep four, at the full-decode rate the manifest table above states.

```bash
python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
```

### Iceberg over S3

The same table over the in-process S3 the S3 backend's own suites run on, every request counted: the `s3` group builds a fresh venue-partitioned table per measured commit, scans one of eight partitions, reads the bridge's own `.log` as one object and writes the FIX rows it holds back into a table on the store. Release Criterion `--quick`, sample size 10, on a containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores, 15 GiB; rustc 1.94.1) shared with another build at the time, so the medians are noisier than the request counts, which are exact and pinned in `accounting::iceberg` in `rust/tests/s3/mod_.rs`. The `.log` read is untouched by this work and keeps its six requests; the gap between its two medians is the noise floor of that host, and the FIX row is parsing and enrichment first, remote calls second.

| operation | requests before | requests after | median before | median after |
| --- | ---: | ---: | ---: | ---: |
| append, one partition, 5,000 rows | 25 | 9 | 7.81 ms | 7.41 ms |
| append, eight partitions, 40,000 rows | 67 | 16 | 56.7 ms | 35.0 ms |
| upsert of 10 rows into one partition of eight | 37 | 13 | 16.6 ms | 8.93 ms |
| full scan, eight files | 30 | 10 | 8.70 ms | 5.42 ms |
| pruned scan, one file of eight | 9 | 3 | 4.11 ms | 2.40 ms |
| `.log` object read as text, 2,304 lines | 6 | 6 | 36.2 ms | 25.0 ms |
| FIX rows parsed, enriched and written back | 61 | 20 | 3.99 s | 2.81 s |

Every request left is the metadata chain - the hint, the manifest list, one manifest per commit that survives the summaries, one `GET` per data file - one upload per file a commit writes, and the one listing that claims a version; the loopback timing only shows that nothing else hides between them. On a real store each request is a round trip of 1-20 ms, which is what the counts are worth.

```bash
cargo bench --features "iceberg s3" -p yggdryl --bench media -- 's3/' --quick
```

### Iceberg on Amazon S3 Tables

`python/benchmarks/media/s3tables.py` is the same question against the real service, beside PyIceberg. It takes a table bucket ARN in `YGGDRYL_S3TABLES_ARN`, has PyIceberg create a table there partitioned by `symbol`, append 65,536 rows in four partitions through the service's catalog - the only door a commit to S3 Tables has - and then opens the same table both ways: PyIceberg through the catalog's REST load, this crate through the warehouse `s3:` location that load answers, with the region the ARN carries and the credentials the catalog vended. Opening the table, a full scan to Arrow, and a scan pruned to one partition of four are each timed on both sides, after the rows both read have been compared; the table is dropped afterwards. The ratio column is PyIceberg's median over this crate's, so above one is in this crate's favor. No table is published here: the run needs an account's own table bucket, and the numbers are those of a network round trip to it, which is why the request counts pinned above are the part that travels.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2 --repeat 5
```
