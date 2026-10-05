# Iceberg

Apache Iceberg tables in a folder: `metadata/` and `data/`, no catalog required, every file reached through the same handles as any other medium - metadata as JSON, manifests as Avro, data files as Parquet by default.

## Overview

| | |
| --- | --- |
| Declared by | a table folder - `metadata/` and `data/` beneath it |
| Build | the `iceberg` feature, which implies `parquet` |
| Rust | `yggdryl::iceberg`: `IcebergTable` (`create`, `open`, `open_or_create`, and by location alone `from_url`, `create_from_url`, `open_or_create_from_url`, `scan`, `scan_matching`, `plan_matching`, `commit_append`, `commit_overwrite`, `commit_merge`, `update_schema`, `compact`), `PartitionSpec`, `PartitionField`, `Transform`, `SortOrder`, `SchemaUpdate`, `IcebergOptions`; `IcebergCatalog` and `IcebergNamespace`, the [warehouse](../warehouse/index.md) implementations over a folder of namespaces of tables, `IcebergTable` the `Table` they answer |
| Python | `yggdryl.iceberg`: `IcebergTable` (`create`, `open_or_create`, the constructor `IcebergTable(root, **properties)` that opens one - `root` a container handle or the table's location - `scan`, `scan_matching`, `plan_matching`, `append`, `overwrite`, `merge`, `update_schema`, `compact`), `PartitionSpec`, `SchemaUpdate`, `IcebergOptions`; `IcebergCatalog`, `IcebergNamespace` and `IcebergTable` the `Catalog`, `Namespace` and `Table` subclasses a warehouse folder answers |
| JavaScript | `iceberg`: `IcebergTable` (`create`, `open`, `openOrCreate` - `root` a container handle or the table's location, `properties` beside a location - `scan`, `scanMatching`, `planMatching`, `append`, `overwrite`, `merge`, `updateSchema`, `compact`), `PartitionSpec`, `IcebergOptions`; `IcebergCatalog` and `IcebergNamespace` over a warehouse folder, `IcebergTable.from(table)` the Iceberg table a `warehouse.Table` holds |
| Settings | `IcebergOptions`, each resolved from the call, then the table property, then the default - a call's options shadow the table's explicit override for that call alone, `IcebergTable::with_call_options` in Rust and the `options` argument of every binding door: `read.parallelism`, `write.parallelism`, `write.target-file-size-bytes` and the commit retries among them; a write's `RecordOptions` add `commit_batch_num` and `num_threads`. A count or a byte size held as a table property reads as the one integer grammar reads a whole number - a sign and the digits, the surrounding blanks not part of it, no unit suffix - and a property that does not read is refused naming its key |

A table lives in one folder: `metadata/` and `data/`, no catalog required.
Iceberg's type strings - `timestamptz`, `fixed[16]`, `list<fixed[16]>`, the
`struct<1: a: optional long>` its reference implementations render - are
[datatype spellings](../types/datatype.md): each primitive reads as the
datatype the table reader maps it to, and a struct member keeps its id and
nullability. A list's or map's string carries no element id or element
nullability, so its child is the grammar's nullable `item`. A v3 `variant` and a v3 `unknown` both read as [`variant`](../types/variant.md); only a table's schema declares a column `unknown`, which it keeps out of its data files and reads back as nulls. A v1 or v2 table refuses both.

A column's `description` is its Iceberg `doc`, both ways and at every depth, so a catalog over the table - a table bucket's console, Athena, Spark - shows it. A `doc` read back has each control character another writer left in it read as a space, and an empty one is none. `SchemaUpdate`'s `update_doc` sets it, an empty text clearing it; no `ICEBERG:` property holds a doc.

## Read

A table is opened from its folder - its newest metadata document, through the version hint where one is written and by listing `metadata/` where another writer left none - with no catalog in between. A scan plans snapshot, manifest list, manifests, then data files, from the metadata and never by walking `data/`: the manifest list's partition summaries skip whole manifests, a manifest's partition tuples and column statistics skip files, and the rest of the predicate filters the rows of the files kept. A scan decodes its files side by side once two of at least 64 KiB qualify (`read.parallel.min-files`, `read.parallel.min-file-size-bytes`), and the files in flight share `read.parallelism` with the columns inside them; it answers in plan order, so it differs from a sequential scan only in speed. The scan doors - `scan`, `scan_matching`, `scan_at` - answer files in that plan order, the manifest list's then each manifest's, and state no row order across files.

A record read - `read_serie`, `read_arrow_reader` and every door over them, a folder addressing the table included - answers the table in order:

- **Partitions** come in ascending partition-tuple order, a null where the sort key naming that identity column places it, else last. A partition is decoded only once the one before it is yielded, so a row limit opens no file of a later partition.
- **Rows** come in the table's default sort order within each partition. Where a key varies inside a partition, its files - decoded side by side, as a scan's are - land in one [`ChunkedSerie`](../types/chunked-serie.md) under the process [spill bound](../types/serie.md#spilling-to-disk) and are sorted out of core unless already in order; at most one partition is held at once. Files open by where their leading sort key starts, the bound each manifest entry carries, so a partition whose commits each cover a later stretch is read as its files, never merged. A key the read's columns leave out is not sorted on, nor any after it.
- **Nothing is held** for a table unsorted, or sorted only by its identity partition columns: its files stream in tuple order, and `read_arrow_reader` is their scan, each batch reconciled and never landed.
- **The root declares** the order proven - the table's, as far as the root holds its keys and the `select` publishes them unchanged - where tuple order is the order's own leading keys (every spec field the identity of a non-floating column, the order opening with those columns ascending in spec order, or the table unpartitioned) and the metadata holds no spec but the default; `read_arrow_field` declares the same from the metadata alone. A transform key is the last declared, since a partition is sorted on the column `truncate(ts, 10)` or `days(ts)` reads, not on the key after it. An identity or transform key is declared only where the root reads its column as the table stores it; from a key read as another datatype on, nothing is. Otherwise each partition is still sorted and nothing is declared, and a plan holding a file of another spec reads in plan order.
- Behind a `select`, a `where` naming a column only the `select` publishes, or a row bound, `read_serie` lands the shaped stream once and carries the declaration as proven, not read again.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array as _, Int64Array, RecordBatch, StringArray};
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, IcebergTable, assign_field_ids};
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
    let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON"), (4, "XLON")] {
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema()?,
            vec![Arc::new(Int64Array::from(vec![id])), Arc::new(StringArray::from(vec![venue]))],
        )?;
        table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // A table is opened from its folder, with no catalog in between.
    let table = IcebergTable::open(LocalFolder::new(&path)?)?;

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
    from yggdryl.iceberg import IcebergTable

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")

    # Four commits into a venue-partitioned table: one manifest each.
    created = IcebergTable.create(root, schema, ["venue"])
    for identifier, venue in ((1, "XNAS"), (2, "XNYS"), (3, "XLON"), (4, "XLON")):
        created.append(pa.record_batch({"id": [identifier], "venue": [venue]}, schema=schema))

    # A table is opened from its folder, with no catalog in between.
    table = IcebergTable(root)

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
    const created = iceberg.IcebergTable.create(root, schema, ['venue'])
    for (const [id, venue] of [[1n, 'XNAS'], [2n, 'XNYS'], [3n, 'XLON'], [4n, 'XLON']]) {
      created.append(
        new arrow.Table({
          id: arrow.vectorFromArray([id], new arrow.Int64()),
          venue: arrow.vectorFromArray([venue], new arrow.Utf8()),
        }),
      )
    }

    // A table is opened from its folder, with no catalog in between.
    const table = iceberg.IcebergTable.open(root)

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

A write commits a snapshot: an append keeps every row the table holds, an overwrite replaces the partitions its rows fall in, and a merge updates the rows its key matches and appends the rest. The partition is an overwrite's unit: each live file of a partition the rows reach is dropped, and every other file is carried as it is - location, statistics and row lineage, on every format version - so reprocessing one window rewrites that window's partitions alone, and a source with no row commits nothing. An unpartitioned table is one partition. A `where` naming partition values replaces those partitions instead, whatever the rows reach; `commit_overwrite_where` with no filter, and `clear`, replace the whole table. A partitioned write groups each batch by vectorized keys and computes a partition tuple once per distinct key, not once per row, and cuts each partition's rows into data files of about the target file size (`write.target-file-size-bytes`) as `yggdryl::arrow::memory_size` measures them before encoding - one file for a partition under it; the target cuts files, never commits. A commit writes its partition groups on `num_threads` threads at once where the write's options state it, else on `write.parallelism`, else `read.parallelism`, else every thread the host offers, each group's file encoding its columns on its share of them. New data files are Parquet unless `data_mime_type` names another encoding, and a scan reads each file as its manifest entry records, so one table can mix them.

A write through the record doors - `write_serie` and its three intents, the `*_arrow_reader` family, a folder addressing the table - commits **once**, when its source ends: every partition's rows are held as the chunks they arrived in - a batch falling whole in one partition the batch itself, a run of its rows a slice, interleaved rows one take - settled under the process [spill bound](../types/serie.md#spilling-to-disk) as they arrive, so an overwrite of any length is one atomic snapshot and its memory is the bound, not the stream. `commit_batch_num = N` commits every `N` whole batches instead, which paces a stream whose rows would outgrow the spill folder; an overwrite then replaces each partition on the first commit that reaches it and appends to it on every later one, and an append or a merge keeps its intent in every commit. No write compacts: `compact` is the one maintenance door, and the caller runs it. A data file's directory spells its partition tuple in ASCII letters, digits, `.`, `_`, `+` and `-`, any other character - an instant's `:`, which no Windows path holds - as `_`: the manifest, never the path, is the authority on a partition value. Where the table declares a sort order, each partition's rows are sorted as a whole by it - stable, through [`ChunkedSerie::into_sort_by`](../types/chunked-serie.md#sorting-uniqueness-and-partitions), each chunk sorted on its own and the chunks merged - unless the group is already in that order: a stream whose root [declares](../types/serie.md#a-declared-order) it, proven as it lands, or a group read once chunk by chunk and edge by edge, is written as it arrived. The Python and JavaScript `IcebergTable.append` and `IcebergTable.overwrite` hold their rows under the same bound and commit once too.

A commit beaten by another writer rebases where that is safe: an append and a metadata-only commit reload the winner and re-apply their intent, with jittered backoff bounded by `commit_retries` and `commit_total_timeout_ms`. An overwrite, a merge or a compaction cannot - it planned against files the winner may have replaced, and its input is already consumed - so after the same bounded waits it fails with `CommitConflict` naming both versions, the table left as the winner made it, and the caller re-reads and retries. A failed commit changes nothing a reader sees; at worst it leaves data files no snapshot names.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, IcebergTable, assign_field_ids};
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
    let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // A table that has never been written to has no current snapshot.
    assert!(table.current_snapshot()?.is_none());
    assert_eq!(table.scan(None)?.count(), 0);

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let snapshot = table.current_snapshot()?.expect("a snapshot");
    assert_eq!(snapshot.operation(), "append");
    assert_eq!(table.data_files()?.len(), 2, "one file per venue");

    // Reopening finds the table again, with no catalog in between.
    let reopened = IcebergTable::open(LocalFolder::new(&path)?)?;
    let rows: usize = reopened.scan(None)?.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergTable

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")

    # A table is created in a folder, and a folder is all it ever touches.
    table = IcebergTable.create(root, schema, ["venue"])

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
    reopened = IcebergTable(IOBase(root.url.into_path()))
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
    const table = iceberg.IcebergTable.create(root, schema, ['venue'])

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
    const reopened = iceberg.IcebergTable.open(root)
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
    use yggdryl::iceberg::{FormatVersion, PartitionField, PartitionSpec, IcebergTable, Transform, assign_field_ids};
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
    let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

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
    from yggdryl.iceberg import PartitionSpec, IcebergTable

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
    table = IcebergTable.create(root, schema, spec)

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
    const table = iceberg.IcebergTable.create(root, schema, spec)

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

A table is also created from what its schema declares. `PartitionSpec::from_schema` reads the root's [`PARTITION:by`](../types/protocol.md#partition-columns) - a bare column an identity field, an epoch function over a column its transform, `truncate(col, w)` a truncation, each named by its alias or by the convention (`ts_minutes`, `name_truncate`), anything else refused by name - and `IcebergTable::create` reads the root's [`SORT:by`](../types/protocol.md#sort-order) as the default sort order, `SortOrder::for_spec` where it declares none. A transform's declaration is written on the root rather than through `with_partition_by`, because its partition value lives in the manifest, not in the rows. Any other derivation is a column: `with_partition_by(["time_bucket('15 minutes', ts) as part"])` materializes `part` as a [`TRANSFORM:`](../types/protocol.md#views) column, the spec partitions on it by identity - which every engine reads - and the table computes it for every row written through every door, after the options' declared field and before their clauses, so a `where` may name it. A value the rows carry under its name is computed again: the table owns the derivation. A `time_bucket(width, x)` partition value bounds `x` to the bucket's whole range, at the manifest summary and at the file, so a window on `x` alone - `ts >= ... and ts < ...` - skips every other bucket's manifests unopened, as a time transform's period does. An Iceberg schema cannot state a term, so the table keeps each as the property `yggdryl.transform.<column>` and declares it back on the column wherever its metadata is read. An identity partition on a nanosecond timestamp reaches the official manifest parser through a view spelling that column `long`; the partition value keeps its type. `IcebergTable::schema()` reports both keys back, `mark_partitions` writing the spec's fields the grammar can spell (a `bucket` has no spelling and is left out) and the default order its keys, so a reopened table says how it partitions and sorts. A record read keeps the order across files and declares on its root, and on `read_arrow_field`, what that proves ([Read](#read)); a scan door's rows declare none.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, SortOrder, IcebergTable, Transform, assign_field_ids};
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
    let table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema, spec)?;
    assert_eq!(table.metadata()?.default_sort_order()?.fields[0].direction, "desc");
    assert_eq!(table.schema()?.get_metadata("PARTITION:by"), Some(r#"["venue","minutes(ts, 15)"]"#));
    assert_eq!(table.schema()?.get_metadata("SORT:by"), Some(r#"["ts desc","id"]"#));
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import DataType, Field, IOBase
    from yggdryl.iceberg import IcebergTable

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
    table = IcebergTable.create(IOBase(root / "declared"), schema)
    assert [(field.name, field.transform) for field in table.spec.fields] == [
        ("venue", "identity"),
        ("ts_minutes", "minutes[15]"),
    ]
    assert table.schema.partition.by == ["venue", "minutes(ts, 15)"]
    assert table.schema.sort.by == ["ts desc", "id"]

    # Stated, the entries are read by the same rule and replace the declaration.
    stated = IcebergTable.create(IOBase(root / "stated"), schema, ["days(ts)", "truncate(venue, 4) as prefix"])
    assert [(field.name, field.transform) for field in stated.spec.fields] == [
        ("ts_day", "day"),
        ("prefix", "truncate[4]"),
    ]

    # `None` partitions nothing, whatever the schema declares.
    assert IcebergTable.create(IOBase(root / "flat"), schema, None).spec.is_unpartitioned()
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
    const table = iceberg.IcebergTable.create(path.join(root, 'declared'), schema)
    assert.deepEqual(
      table.spec.fields.map((field) => [field.name, field.transform]),
      [['venue', 'identity'], ['ts_minutes', 'minutes[15]']],
    )
    assert.deepEqual(table.schema.partition.by, ['venue', 'minutes(ts, 15)'])
    assert.deepEqual(table.schema.sort.by, ['ts desc', 'id'])

    // Stated, the entries are read by the same rule and replace the declaration.
    const stated = iceberg.IcebergTable.create(path.join(root, 'stated'), schema, ['days(ts)', 'truncate(venue, 4) as prefix'])
    assert.deepEqual(
      stated.spec.fields.map((field) => [field.name, field.transform]),
      [['ts_day', 'day'], ['prefix', 'truncate[4]']],
    )

    // `null` partitions nothing, whatever the schema declares.
    assert.equal(iceberg.IcebergTable.create(path.join(root, 'flat'), schema, null).spec.isUnpartitioned(), true)

    fs.rmSync(root, { recursive: true, force: true })
    ```

`IcebergTable.create` and `open_or_create` take the partitioning as a `PartitionSpec` or as `PARTITION:by` entries - Python's `partition_by`, JavaScript's `partitionBy`, each entry its text, in Python a `Term` or a `(term, alias)` pair too - read by `PartitionSpec::from_schema`'s rule, so a refusal names the entry. Omitted, the schema's own `PARTITION:by` is read; `None` in Python and `null` in JavaScript - or an empty list - partition nothing whatever the schema declares. The default sort order is the schema's `SORT:by` either way.

## Schema evolution

A `SchemaUpdate` records column operations - add, rename, drop, promote - and one commit replays them onto the schema that commit attempt reads, so a commit beaten by another writer rebases onto the winner's schema rather than overwriting it. Field IDs are kept and a dropped one is never reused; promotions are `int32 -> int64`, `float32 -> float64`, same-scale decimal widening and, on a v3 table, an `unknown` column to any type, which clears its [`unknown` declaration](../types/variant.md#edges); any other change into or out of `unknown`, `variant` or `binary` is refused. The commit answers the schema id it made current, and an update that recorded nothing writes nothing and answers the current one. A column is named by a [field path](../types/paths.md) of struct column names, read once by the one path parser: `quote.price` is `price` inside the struct column `quote`, a column whose name holds a dot is the quoted `"a.b"` and never two levels, and the empty parent `""` of `add_column` is the root; a path reaching anything but a struct child, or carrying `as`, is refused naming it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch};
    use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, SchemaUpdate, IcebergTable};
    use yggdryl::local::LocalFolder;
    use yggdryl::{arrow, DataType, StructType};

    let mut schema = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
        .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-evolve");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
    let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int32Array::from(vec![1]))])?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let mut update = SchemaUpdate::from_metadata(table.metadata()?)?;
    update.add_column("", DataType::utf8().nullable_field("note"));
    update.update_type("id", DataType::Int64);
    assert_eq!(table.update_schema(&update)?, 1);

    // Nothing recorded, nothing written: the current id comes back.
    let unchanged = SchemaUpdate::from_metadata(table.metadata()?)?;
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
    from yggdryl.iceberg import IcebergTable

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = IcebergTable.create(root, pa.schema([pa.field("id", pa.int32(), nullable=False)]))
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
    const table = iceberg.IcebergTable.create(root, fields.struct('row', [new Field('id', 'int32', false)], { nullable: false }))
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1], new arrow.Int32()) }))

    const schemaId = table.updateSchema().addColumn('', Field.from('note: utf8')).updateType('id', 'int64').commit()
    assert.equal(schemaId, 1)

    // Nothing recorded, nothing written: the current id comes back.
    assert.equal(table.updateSchema().commit(), 1)

    assert.deepEqual([...table.schema.dtype].map((child) => child.name), ['id', 'note'])
    assert.deepEqual([...table.scan().intoTable().getChild('note')], [null])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Catalog

A warehouse folder is a catalog on the [warehouse](../warehouse/index.md) abstraction, laid out the way `HadoopCatalog` lays one out: `IcebergCatalog` is the `Catalog` implementation, a folder under the warehouse an `IcebergNamespace`, a folder laid out as a table the `IcebergTable` a generic `Table` holds, so `lake.nyc.taxis` is the folder `nyc/taxis` under the warehouse registered as `lake`. The views, the dotted descent, the registry and a plan's `from lake.nyc.taxis` are the generic ones; what the implementation adds is the storage. Namespaces nest to any depth, so a catalog states no `namespace_levels`. Each level keeps its stored properties in its own document - `metadata/catalog.json` under the warehouse, `metadata/namespace.json` under a namespace - read beneath what was stated and written by `update_properties`, which refuses the reserved `ICEBERG:` prefix; a table's properties ride its metadata document, so its `update_properties` is refused in favour of `commit_metadata_changes`, and what was stated for it at creation is answered over them and written nowhere. Constructing any of the three touches nothing, and every verb runs against the folder when it is asked, so two catalogs over one folder see the same tables. `Catalog::from_url` answers one under `type = hadoop`, PyIceberg's spelling.

A listing classifies each entry with one listing of its `metadata/` and no read: a folder holding a `version-hint.text` or a `*.metadata.json` is a table, every other folder a namespace, the reserved `metadata` name skipped and refused as a name. A table answered is described at its folder and reads its current document on the first verb that needs it. `create_namespace` writes the namespace document; `create_table` is `IcebergTable::create` under `PartitionSpec::from_schema` at the create's `format-version` property, else the lowest version that states the schema - 3 for a nanosecond timestamp, a variant or an unknown column, else 2 - over the schema as Iceberg expresses it (`into_scheme_compat`): a dictionary layout is stored as the string it encodes and the rows cast to it on the way in, `float16` is widened to `float`, and a type Iceberg lacks - an interval - is refused by path with nothing created. A create descends through existing namespaces only: `tables().create("sales.eu.orders", ..)` under a missing `sales` is the absence of `sales`, never a namespace made on the way, and the view's `append` and `overwrite` create the table from the rows' own schema under the same rule.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::IcebergCatalog;
    use yggdryl::local::LocalFolder;
    use yggdryl::{
        arrow, Catalog, DataType, IOMedia, ObjectValue, Properties, StructType, Table, TableValue, Warehouse,
    };

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-docs-iceberg-catalog-{}", std::process::id()));
    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue").with_partition(true),
    ])?)
    .required_field("row");

    // A warehouse folder is a catalog; constructing one touches nothing, and
    // what it states reaches every object under it without being written.
    let catalog = Catalog::from(
        IcebergCatalog::bound("lake", Holder::folder(&root)?)
            .with_properties(Properties::new().with_property("owner", "ops")),
    );
    assert!(!root.exists());

    // A create descends through existing namespaces only, so the namespace
    // is made before the table under it.
    let nyc = catalog.namespaces().create("nyc", &Properties::new())?;
    let mut taxis = catalog.tables().create("nyc.taxis", &schema, &Properties::new())?;
    assert_eq!(taxis.to_string(), "lake.nyc.taxis");
    assert_eq!(taxis.storage(), "table");
    assert!(matches!(taxis, Table::Iceberg(_)));
    assert_eq!(nyc.properties()?.get("owner"), Some("ops"));
    assert_eq!(taxis.properties()?.get("owner"), Some("ops"));

    // A table answers every record verb, partitioned as its schema declares.
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    taxis.append_arrow_reader(arrow::batch_reader(batch.schema(), [batch]), &taxis.record_options()?)?;
    assert_eq!(taxis.row_size()?, 2);

    // The same folder resolves by dotted path, one listing per level and no
    // read, through a registry or the catalog itself.
    let mut warehouse = Warehouse::new();
    warehouse.register(catalog)?;
    assert_eq!(warehouse.table("lake.nyc.taxis")?.path(), taxis.path());
    let names = warehouse.catalog("lake")?.namespaces().iter().collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(names, ["nyc"]);
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import Catalog, Warehouse
    from yggdryl.iceberg import IcebergCatalog, IcebergNamespace, IcebergTable

    root = pathlib.Path(tempfile.mkdtemp()) / "warehouse"
    schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("venue", pa.string())])

    # A warehouse folder is a catalog; constructing one touches nothing, and
    # what it states reaches every object under it without being written.
    catalog = IcebergCatalog("lake", root, owner="ops")
    assert isinstance(catalog, Catalog) and not root.exists()

    # A create descends through existing namespaces only, so the namespace is
    # made before the table under it; each object is the Iceberg subclass of
    # its kind.
    nyc = catalog.namespaces.create("nyc")
    taxis = nyc.tables.create("taxis", schema)
    assert type(nyc) is IcebergNamespace and type(taxis) is IcebergTable
    assert str(taxis) == "lake.nyc.taxis" and taxis.storage == "table"
    assert taxis.properties == {"owner": "ops"}

    # The view's append creates on first write, from the rows' own schema; a
    # table answers every record verb.
    catalog.tables.append("nyc.zones", pa.table({"id": [1, 2], "zone": ["a", "b"]}))
    assert sorted(nyc.tables) == ["taxis", "zones"]
    assert catalog.table("nyc.zones").read_arrow_reader().read_all().num_rows == 2

    # The same folder resolves by dotted path through a registry.
    warehouse = Warehouse()
    warehouse.register(catalog)
    assert warehouse.table("lake.nyc.taxis") == taxis
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg, warehouse } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'warehouse')
    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    })

    // A warehouse folder is a catalog; constructing one touches nothing, and
    // what it states reaches every object under it without being written.
    const catalog = new iceberg.IcebergCatalog('lake', root, { properties: { owner: 'ops' } })
    assert.ok(!fs.existsSync(root))

    // A create descends through existing namespaces only, so the namespace is
    // made before the table under it.
    const nyc = catalog.namespaces().create('nyc')
    const taxis = catalog.tables().create('nyc.taxis', schema)
    assert.equal(String(taxis), 'lake.nyc.taxis')
    assert.equal(taxis.implementation, 'IcebergTable')
    assert.deepEqual(taxis.properties, { owner: 'ops' })

    // The view's append creates on first write, from the rows' own schema;
    // `IcebergTable.from` is the Iceberg table a generic one holds.
    const zones = catalog.tables().append(
      'nyc.zones',
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        zone: arrow.vectorFromArray(['a', 'b'], new arrow.Utf8()),
      }),
    )
    assert.equal(iceberg.IcebergTable.from(zones).scan().intoTable().numRows, 2)
    assert.deepEqual([...nyc.tables().keys()], ['taxis', 'zones'])

    // The same folder resolves by dotted path through a registry.
    const registry = new warehouse.Warehouse()
    registry.register(catalog.intoCatalog())
    assert.ok(registry.table('lake.nyc.taxis').equals(taxis))

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

The cost is stated in store calls and pinned in `rust/tests/iceberg/catalog/mod_.rs` (`mod call_counts`) on a counted Arrow filesystem:

| Operation | Calls |
| --- | --- |
| `tables().get("sales.orders")` | per level one presence answer, one listing of the entry's `metadata/` to tell a table from a namespace, and one read of the level's own document for what its child inherits; the table's own document is not read: `file_info=2 list=2 open_input_stream=2` |
| the table's first `metadata()` | the version hint and the document it names, `file_info=1 open_input_stream=2`; every later one `none` |
| a missing table | the levels above it, then one presence answer and nothing listed or read for it: `file_info=2 list=1 open_input_stream=2` |
| `tables().create("a.b.c.orders", ..)` | three levels at that cost, the namespace's document for what the table inherits, then the create: one presence answer, one listing of `metadata/`, and the writes with the missing parent repaired once - `create_dir=1 delete_file=1 file_info=4 list=4 open_input_stream=4 open_output_stream=4`; nothing walks the ancestry twice |
| `tables().open_or_create(..)` | absent, the get to the missing child then the create, `list=3` where the create alone lists four; present, exactly what `get` costs, because it is the same attempt |

### A table a catalog service names

A folder table names its current document itself - `metadata/version-hint.text`, else the highest number a listing of `metadata/` shows - and commits by writing the next one. A table a catalog service keeps is named by the service, so its folder need take neither a listing nor a delete. `MetadataPointer` is that service reduced to a compare-and-swap, and `open_pointed` and `create_pointed` are the doors beside `open` and `create`:

```text
trait MetadataPointer: Debug + Send + Sync {
    fn current(&self) -> Result<PointerState>;                               // the document named now - none before the first - and its token
    fn publish(&self, token: &str, location: &Url) -> Result<PointerState>;  // names the next while the pointer stands at token; moved, a conflict
    fn remove(&self) -> Result<()> { .. }                                    // provided: refuses; a catalog that drops a table answers it
}
IcebergTable::open_pointed(root, pointer) -> Result<IcebergTable<H>>         // one answer, one read of the document it names
IcebergTable::create_pointed(root, format_version, schema, spec, pointer)    // version 0, published under the token read
```

A pointed table reads the one document the pointer names, its location relative to the root's, and writes each next one as `metadata/{version:05}-{uuid}.metadata.json` - the name Iceberg's own catalogs write, the first `00000` - published under the token the last reading or publication answered. A commit is its files, one document and one publication, with nothing read first; nothing lists the folder, writes or reads a hint, or removes a file, so a failed commit's files and document stay, unreferenced.

- **A moved pointer** is read again, one answer and one document. An append or a metadata-only commit applies again onto the winner under the folder contract's retry budget; an overwrite, a merge or a compaction is a `CommitConflict` at once, unless the pointer still names the document it planned against - a token moved by a change that wrote no document - and then publishes again under the new token.
- **A publication in doubt** - one failing otherwise - reads the pointer once more, and a pointer naming the attempt is the commit made.
- **`ls`** is refused as `Error::Unsupported`, touching nothing. **`remove`** is the pointer's whatever `recursive` says: the catalog that keeps the pointer keeps the table and drops it whole, never by deleting its files. A pointer whose catalog drops nothing refuses as `Error::Unsupported`, restated naming the table's location as `ls`'s refusal is; any other failure of the drop reaches the caller as it came. A drop that took forgets the document read, so the next question goes to the pointer again.
- `metadata_version` is the number the document's name states; the table's path, equality and hash are the folder contract's - the pointer is not its identity.

Rust only: a binding reaches a pointed table as the [Amazon S3 Tables](#iceberg-on-amazon-s3-tables) table its [location](#a-table-by-its-location) names.

### Iceberg on Amazon S3 Tables

An Amazon S3 Tables table bucket is a catalog on the [warehouse](../warehouse/index.md) abstraction, behind the `s3tables` feature (which implies `s3` and `iceberg`): `S3TablesCatalog` is the bucket, its namespaces one level below (`namespace_levels` is `Some(1)`), each an `S3TablesNamespace` of Iceberg tables. Each table is the `IcebergTable` a generic `Table::Iceberg` holds, rooted on a `Handle` on the warehouse `s3:` location the service chose (`s3://<id>--table-s3`) and opened through the [S3 backend](../holder/index.md#object-stores) under the catalog's session, region and properties - so `s3.endpoint`, `s3.region` and the store's other names stated on the catalog reach every table's files. The service names a table's current document, so every table is [pointed](#a-table-a-catalog-service-names): `GetTableMetadataLocation` is `current()` and `UpdateTableMetadataLocation` is `publish`, whose `409 ConflictException` - a version token the table moved past - is the commit conflict. The warehouse location is asked only `PutObject` and `GetObject`: a failed commit's files are the bucket's unreferenced-file removal's to collect, and a table's `remove` is one `DeleteTable`, deleting none of its files.

`create_table` runs `IcebergCatalog`'s steps against the control plane: the schema as Iceberg states it (`into_scheme_compat`), numbered above the highest identifier it carries, partitioned by `PartitionSpec::from_schema` - a derived `PARTITION:by` entry included - sorted by its `SORT:by`, at the [format version a folder catalog picks](#catalog). `CreateTable` registers the table with no schema, `GetTableMetadataLocation` answers its warehouse and token, and the crate's own first document is written and published under that token; one not published removes the registration again under that token, which a publication that took has moved past. The service keeps no properties for a bucket or a namespace, so theirs are what was stated; a table's ride its metadata.

`Catalog::from_url` answers one for a table bucket's ARN or the `s3tables://<bucket>` location it locates, and for nothing below: a namespace or a table there is refused at `$.url`, and reached by [the doors that take one](#a-table-by-its-location). Who signs is `Session::from_properties` over the properties, PyIceberg's `s3tables.`-prefixed names (`s3tables.profile-name`, `s3tables.access-key-id`, ...) read after the bare ones; `s3tables.region` and `s3tables.endpoint` are the client's region and endpoint. The bucket's ARN is the one the location was given as, its region and account kept; else the `s3tables.warehouse` or `warehouse` property; else one built from the `account_id` property and the client's region; else, since a bare `s3tables://<bucket>` states neither, the caller's table bucket of that name in that region, found by one `ListTableBuckets` on first use. A `warehouse` ARN naming another bucket, or another ARN than the location's, is refused at `$.with.warehouse`; a namespace under a namespace, or a table directly under the catalog, is refused by implementation name. The catalog hands every namespace and table under it the properties less those the session read, so `properties` lists no credential.

| Operation | Requests |
| --- | --- |
| building the catalog, a namespace or a table description | none |
| the bucket's ARN, named by `s3tables://<bucket>` alone | 1 `ListTableBuckets` per page, once |
| `children()` of the catalog | 1 `ListNamespaces` per page |
| `get` of a namespace | 1 `GetNamespace` |
| `create_namespace` | 1 `CreateNamespace` |
| `children()` of a namespace | 1 `ListTables` per page, 1 `GetTableMetadataLocation` per table as its turn comes |
| `get` of a table - `catalog.table("desk.quotes")` adds the namespace's `GetNamespace` | 1 `GetTableMetadataLocation` |
| `create_table` | `CreateTable`, `GetTableMetadataLocation`, 1 `PutObject` and `UpdateTableMetadataLocation` |
| a table's `remove` | 1 `DeleteTable` |
| a table's first read | 1 `GetTableMetadataLocation` and 1 `GetObject` |
| a commit | its files' `PutObject`s and 1 `UpdateTableMetadataLocation`; a refused one 1 `GetTableMetadataLocation` and 1 `GetObject` more |

The counts are pinned in `rust/tests/s3tables/catalog.rs` against the fake control plane, each table's warehouse a bucket of the fake object store the `s3` suites run on, whose log shows no listing and no delete; the pointer's own suite, over a pointer in memory and a counting filesystem, is `rust/tests/iceberg/pointer.rs`.

=== "Rust"

    ```rust
    use yggdryl::{Arn, Catalog, CatalogValue, ObjectValue, Properties};

    // A table bucket's ARN is its catalog: the region and the account it
    // states are kept, so no listing of the caller's own buckets asks for them.
    let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
    let properties = Properties::new()
        .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
        .with_property("secret_access_key", "a-secret");
    let catalog = Catalog::from_url(&lake, &properties)?;
    assert_eq!(catalog.name(), "lake");
    assert_eq!(catalog.namespace_levels(), Some(1));
    assert!(!format!("{catalog:?}").contains("a-secret"), "the credential stays out of Debug");
    ```

=== "Python"

    ```python
    from yggdryl import Catalog

    # A table bucket's ARN is its catalog: the region and the account it
    # states are kept, so no listing of the caller's own buckets asks for them.
    lake = "arn:aws:s3tables:eu-west-3:123456789012:bucket/lake"
    catalog = Catalog.from_url(lake)
    assert type(catalog) is Catalog
    assert catalog.name == "lake"
    assert catalog.namespace_levels == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { warehouse } = require('yggdryl')

    // A table bucket's ARN is its catalog: the region and the account it
    // states are kept, so no listing of the caller's own buckets asks for them.
    const lake = 'arn:aws:s3tables:eu-west-3:123456789012:bucket/lake'
    const catalog = warehouse.Catalog.fromUrl(lake)
    assert.equal(catalog.implementation, 'Catalog')
    assert.equal(catalog.name, 'lake')
    assert.equal(catalog.namespaceLevels, 1)
    ```

An ignored test runs a table's life through the catalog against the live service, in a table bucket the operator names - a namespace, a table with a nanosecond instant, a UUID and a quarter-hour partition, an append, an overwrite, a read back - and removes the table and the namespace however it ended.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> cargo test -p yggdryl --features s3tables --test s3tables catalog::live -- --ignored --nocapture
```

#### A table by its location

A location alone reaches what a table bucket keeps, with no catalog built first. `s3tables://<bucket>[/<namespace>[/<table>]]` names the bucket's catalog, a namespace or a table, a trailing slash naming nothing more; a deeper location, or a name the service has no namespace or table of, is refused at `$.url` before any request. A table bucket's ARN names the catalog, and a table's ARN - `arn:<partition>:s3tables:<region>:<account>:bucket/<name>/table/<id>` - the table that identifier is, wherever a rename moved it. Its location (`Arn::locator`, `s3tables://<bucket>/<id>`) spells the identifier where a namespace goes, so every door reads the ARN as an ARN before lowering it. One reading answers every door:

| Door | Answers |
| --- | --- |
| `Holder::from_url(location, properties)` - Python `IOBase(location)`, JavaScript `new IOBase(location)`, under no properties | the catalog, the namespace or the table, as the handle it is, resolved when called: `kind()` says which, and Python answers the `Catalog`, `Namespace` or `IcebergTable` class. A table bucket's table costs its requests at construction, and an absent one is refused there |
| an identifier used as a handle - `Holder::from(arn)`, a `Uri` as `IOBase`; Rust only | the same, resolved under no properties - who signs is the environment's - on the first operation that needs it, and kept for the value's life |
| `IcebergTable::from_url(location, properties)` - Python `IcebergTable(location, **properties)`, JavaScript `IcebergTable.open(location, properties)` | the table: a table bucket's described, its document read on the first verb that needs it; a folder's opened, its document read as `open` reads it. A bucket or a namespace is refused by its kind at `$.url` |
| `IcebergTable::create_from_url(location, properties, version, schema, spec)` - Python `IcebergTable.create(location, schema, partition_by, format_version=None, **properties)`, JavaScript `IcebergTable.create(location, schema, partitionBy, version, properties)` | the table registered in its bucket and its first document published, as [`create_table`](#iceberg-on-amazon-s3-tables) does. A namespace the bucket lacks is made on the way - this door's alone; a table's ARN names a table that exists, and is refused |
| `IcebergTable::open_or_create_from_url` - Python `IcebergTable.open_or_create`, JavaScript `IcebergTable.openOrCreate` | the table opened, else created - the location read and the bucket's catalog built once for both. A table's ARN is opened or absent, never created |

The same three doors open, create, and open or create a table in a folder any backend holds - `file:`, an object store, a path - rooted on the handle `Holder::from_url` opens for the location under the properties, resolved on first use and again by every clone. Either way a `version` left out is the one [a folder catalog's create picks](#catalog), a `spec` left out the one the schema declares, and the schema is stored as Iceberg expresses it, numbered above the highest identifier it carries. In the bindings a handle as `root` keeps what it did - the folder it addresses, reopened by that location alone under the environment, format version 2 unless stated - and properties beside a handle are refused by name.

The properties are read as [`Catalog::from_url`](#iceberg-on-amazon-s3-tables) reads them, and the catalog is named by the `name` property, else the bucket. A table reached this way inherits the properties less those the session read, so it lists and prints no credential. A folder table keeps that rule without a session: its handle opens under every property given, again by every clone, while the table states them less those the store's reader took - who signs, where the store is, how it is addressed - so a credential reaches the storage and no `properties` listing or `Debug`; a local path reads none, so a table on one states everything. A table bucket's table's `remove` is one `DeleteTable` under no version token, a table already gone counting as dropped; the value then forgets the document it read, so the handle answers that nothing is there (`Holder::exists`) and a later verb meets the table's absence, never a commit to a gone table. A folder table's `remove(recursive)` removes its folder.

| Operation | Requests |
| --- | --- |
| a bucket's or a namespace's location, a bucket's ARN | none |
| a table's location, the bucket's ARN known - a `warehouse` property or `account_id` | 1 `GetTableMetadataLocation`, and no `GetNamespace` |
| a table's location alone | that, and the 1 `ListTableBuckets` per page a catalog named so pays, once |
| a table's ARN | 1 `GetTable`, addressed by `tableArn` |
| `create_from_url` | `CreateTable`, `GetTableMetadataLocation`, 1 `PutObject` and `UpdateTableMetadataLocation` |
| `create_from_url` under a namespace the bucket does not hold | the refused `CreateTable`, 1 `CreateNamespace`, then those four |
| `open_or_create_from_url` | the open; absent, its one refused request, then the create - the bucket's ARN resolved once for both, so a location alone lists once, and a bucket that listing finds none of is absent before the open; a table's ARN the one `GetTable`, opened or absent. Under a stated ARN or `account_id`, a bucket that is not there is the open's miss, then the service's refusal of the create |
| an identifier used as a handle | what its location costs above, on the first operation that needs it: kept for the value's life when the table is there, and paid again by every operation and every accessor while it is absent, a failed resolution being kept nowhere |
| Python `IOBase(location)`, JavaScript `new IOBase(location)` | what its location costs above, once, at construction - a location alone lists the buckets, since the constructor states no ARN and no account |
| `remove` | 1 `DeleteTable` |
| the first read after an open | 1 `GetTableMetadataLocation` and 1 `GetObject`: the request that located the table primes nothing, since an object may be read long after it is built |

The counts are pinned in `rust/tests/s3tables/catalog.rs` against the two fakes, and an identifier used as a handle in `rust/tests/s3tables_handle.rs`. A folder door costs what `open`, `create` and `open_or_create` cost on the handle `Holder::from_url` opens - `from_url` the version hint and the current document - pinned in `rust/tests/iceberg/table.rs` (`mod located`).

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::IcebergTable;
    use yggdryl::{Arn, DataType, IOBase, IOKind, Properties, StructType, Url};

    // The bucket is its catalog and a segment below it a namespace, by its
    // location or by its ARN: a description each, so nothing is sent.
    let none = Properties::new();
    let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
    assert_eq!(Holder::from_url(&lake, &none)?.kind(), IOKind::Catalog);
    assert_eq!(Holder::from_url(Url::from_str("s3tables://lake")?, &none)?.kind(), IOKind::Catalog);
    assert_eq!(Holder::from_url(Url::from_str("s3tables://lake/desk/")?, &none)?.kind(), IOKind::Namespace);

    // A location names at most a namespace and a table, and a table door
    // takes a table: both are refused where the location is read.
    for location in ["s3tables://lake/a/b/c", "s3tables://lake/desk"] {
        let refused = IcebergTable::from_url(Url::from_str(location)?, &none).unwrap_err();
        assert!(refused.to_string().contains("$.url"));
    }

    // The same doors over a folder any backend holds: created, reopened and
    // dropped by its location, the version and the spec the schema's own.
    let folder = std::env::temp_dir().join(format!("yggdryl-located-doc-{}", std::process::id()));
    let location = Url::from_path(&folder)?;
    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let created = IcebergTable::create_from_url(&location, &none, None, schema, None)?;
    let mut table = IcebergTable::from_url(&location, &none)?;
    assert_eq!(table.metadata_file_name()?, created.metadata_file_name()?);
    table.remove(true)?;
    assert!(!folder.exists());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import Catalog, IOBase, Namespace
    from yggdryl.iceberg import IcebergTable

    # The bucket is its catalog and a segment below it a namespace, by its
    # location or by its ARN: a description each, so nothing is sent.
    assert isinstance(IOBase("s3tables://lake"), Catalog)
    assert isinstance(IOBase("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake"), Catalog)
    assert isinstance(IOBase("s3tables://lake/desk/"), Namespace)

    # A location names at most a namespace and a table, and a table door takes
    # a table: both are refused where the location is read.
    for location in ("s3tables://lake/a/b/c", "s3tables://lake/desk"):
        try:
            IcebergTable(location)
        except ValueError as refused:
            assert "$.url" in str(refused)
        else:
            raise AssertionError("expected a refusal")

    # The same doors over a folder any backend holds: created, reopened and
    # dropped by its location, the version and the spec the schema's own.
    with tempfile.TemporaryDirectory() as root:
        folder = pathlib.Path(root) / "trades"
        schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
        created = IcebergTable.create(folder, schema)
        table = IcebergTable(str(folder))
        assert table.table_uuid == created.table_uuid
        assert table.format_version == 2
        table.remove(recursive=True)
        assert not folder.exists()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, IOBase, fields, iceberg } = require('yggdryl')

    // The bucket is its catalog and a segment below it a namespace, by its
    // location or by its ARN: a description each, so nothing is sent.
    assert.equal(new IOBase('s3tables://lake').kind(), 'catalog')
    assert.equal(new IOBase('arn:aws:s3tables:eu-west-3:123456789012:bucket/lake').kind(), 'catalog')
    assert.equal(new IOBase('s3tables://lake/desk/').kind(), 'namespace')

    // A location names at most a namespace and a table, and a table door
    // takes a table: both are refused where the location is read.
    assert.throws(() => iceberg.IcebergTable.open('s3tables://lake/a/b/c'), /\$\.url/)
    assert.throws(() => iceberg.IcebergTable.open('s3tables://lake/desk'), /\$\.url/)

    // The same doors over a folder any backend holds: created, reopened and
    // dropped by its location, the version and the spec the schema's own.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-located-'))
    const folder = path.join(root, 'trades')
    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const created = iceberg.IcebergTable.create(folder, schema)
    const table = iceberg.IcebergTable.open(folder)
    assert.equal(table.tableUuid, created.tableUuid)
    assert.equal(table.formatVersion, 2)
    IOBase.from(table.intoTable()).remove(true)
    assert.equal(fs.existsSync(folder), false)
    fs.rmSync(root, { recursive: true, force: true })
    ```

#### The table bucket's catalog

`yggdryl::s3tables::S3Tables` is the client of a table bucket's control plane, behind the `s3tables` feature (which implies `s3` and `iceberg`) and Rust only. It creates, describes, lists and removes buckets, namespaces and tables, and moves a table's metadata location - a commit, since the service names the current metadata file. A table's files stay the [S3 backend's](../holder/index.md#object-stores), at the warehouse `s3:` location the catalog answers. Each request is an `http::Request` signed by [`Request::with_sigv4`](../holder/index.md#aws-identity) for the `s3tables` service as an `aws::Session` answers, sent to the endpoint the client states, else `Session::service_endpoint("s3tables", region)`: the configured one - `AWS_ENDPOINT_URL_S3TABLES`, `AWS_ENDPOINT_URL`, the profile's `[services]` entry `s3tables`, its `endpoint_url` ([Where each service is reached](../holder/index.md#where-each-service-is-reached)) - else the region's published host. The session's FIPS and dual-stack switches choose among published hosts only, as botocore's do. The service takes no anonymous request, so a session answering no credentials is refused before anything is sent.

```text
S3Tables::new(session)                                    // reads nothing, reaches nothing
tables.with_region(region)                                // over the ARN's and the session's
tables.try_with_endpoint_url(url) -> Result<S3Tables>     // over the session's, read once
tables.region_of(Some(&bucket)) -> Result<String>         // stated, else the ARN's, else the session's
tables.endpoint_url(region) -> Result<String>             // stated, configured, else https://s3tables[-fips].{region}.{suffix}

tables.create_table_bucket(name) -> Result<Arn>
tables.get_table_bucket(&bucket) -> Result<TableBucket>
tables.table_buckets() -> TableBuckets                    // lazy: Iterator<Item = Result<TableBucket>>
tables.remove_table_bucket(&bucket) -> Result<()>

tables.create_namespace(&bucket, namespace) -> Result<()>
tables.get_namespace(&bucket, namespace) -> Result<NamespaceSummary>
tables.namespaces(&bucket) -> NamespaceSummaries          // lazy: Iterator<Item = Result<NamespaceSummary>>
tables.remove_namespace(&bucket, namespace) -> Result<()>

tables.create_table(&bucket, namespace, name, Some(&schema)) -> Result<TableVersion>
tables.get_table(&bucket, namespace, name) -> Result<TableDescription>
tables.get_table_by_arn(&table) -> Result<TableDescription> // by the ARN alone: its namespace and name answered
tables.tables(&bucket, Some(namespace)) -> TableSummaries // lazy: Iterator<Item = Result<TableSummary>>
tables.rename_table(&bucket, namespace, name, new_namespace, new_name, version_token) -> Result<()>
tables.remove_table(&bucket, namespace, name, version_token) -> Result<()>

tables.get_table_metadata_location(&bucket, namespace, name) -> Result<TableMetadataLocation>
tables.update_table_metadata_location(&bucket, namespace, name, version_token, &location)
    -> Result<TableVersion>                               // the commit, under the token last read
```

The values are descriptions, not handles - `TableDescription` what `GetTable` answers, `NamespaceSummary` and `TableSummary` the model's names for a namespace and a listing entry - so none shadows the [warehouse's](../holder/index.md) `Table` and `Namespace`.

Every verb is one request, a listing one per page of 250 asked for as the page before it drains, and building a client or a listing sends nothing. A read or a removal meeting a `5xx`, a `429`, a throttling error type (`ThrottlingException` under a `400`) or a transport failure is retried under the HTTP client's attempts; a `PUT` that creates, renames or commits is sent once, throttled or not - where botocore would retry - since the service may have acted on an unanswered one. A request refused for a lapsed or unrecognized key (`ExpiredTokenException`, `UnrecognizedClientException`) is signed and sent once more, only when the session then answers another set.

| Verb | Request |
| --- | --- |
| `create_table_bucket` | `PUT /buckets` |
| `get_table_bucket` | `GET /buckets/{arn}` |
| `table_buckets` | `GET /buckets`, one per page |
| `remove_table_bucket` | `DELETE /buckets/{arn}` |
| `create_namespace` | `PUT /namespaces/{arn}` |
| `get_namespace` | `GET /namespaces/{arn}/{namespace}` |
| `namespaces` | `GET /namespaces/{arn}`, one per page |
| `remove_namespace` | `DELETE /namespaces/{arn}/{namespace}` |
| `create_table` | `PUT /tables/{arn}/{namespace}` |
| `get_table`, `get_table_by_arn` | `GET /get-table` |
| `tables` | `GET /tables/{arn}`, one per page |
| `rename_table` | `PUT /tables/{arn}/{namespace}/{name}/rename` |
| `remove_table` | `DELETE /tables/{arn}/{namespace}/{name}` |
| `get_table_metadata_location` | `GET /tables/{arn}/{namespace}/{name}/metadata-location` |
| `update_table_metadata_location` | `PUT /tables/{arn}/{namespace}/{name}/metadata-location` |

A table bucket is addressed by its ARN, which also names the region a request is signed for and sent to. Refused before any request - a listing's refusal as its one item:

- a name the service's model refuses - a table bucket 3 to 63 of `0-9`, `a-z` and `-`, a namespace or a table 1 to 255 of `0-9`, `a-z` and `_`;
- an ARN naming no table bucket, or for `get_table_by_arn` no table of one;
- an empty version token, and a rename naming no new namespace and no new name;
- a region that is no host label, since it names the host;
- an endpoint carrying user information, a query or a fragment, refused where it is read, and a configured one the session refuses - a value naming none, a `[services]` section nobody wrote; no refusal and no `Debug` repeats the user information.

`create_table` sends the schema's `PARTITION:by` as the table's `partitionSpec` and its `SORT:by` as its `writeOrder`. The service's `NotFoundException` is `Error::Absent` from a `get_*` verb and success from a `remove_*`; its `ConflictException` is `Error::Conflict` from a `create_*`; every other refusal is `Error::Remote` with the service's status, error type and message - a commit under a moved version token (`409 ConflictException`) among them.

=== "Rust"

    ```rust
    use yggdryl::Arn;
    use yggdryl::aws::{Credentials, Session};
    use yggdryl::s3tables::S3Tables;

    // A session that states everything consults nothing: no file, no variable, no socket.
    let session = Session::new()
        .with_environment(false)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "a-secret"));
    let tables = S3Tables::new(session.clone());

    // A table bucket is where its ARN says it is, and the region names the host.
    let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
    let region = tables.region_of(Some(&lake))?;
    assert_eq!(tables.endpoint_url(&region)?, "https://s3tables.eu-west-3.amazonaws.com");

    // What the service's model refuses costs no request: an upper-case bucket
    // name, an Amazon S3 bucket's ARN.
    assert!(tables.create_table_bucket("Lake").is_err());
    assert!(tables.get_table_bucket(&Arn::from_str("arn:aws:s3:::lake")?).is_err());

    // An endpoint the caller states is where every request goes, FIPS or not.
    let gateway = S3Tables::new(session.with_use_fips_endpoint(true))
        .try_with_endpoint_url("http://localhost:4566")?;
    assert_eq!(gateway.endpoint_url(&region)?, "http://localhost:4566");
    ```

=== "Python"

    ```python
    # Rust only: no binding reaches the S3 Tables catalog.
    ```

=== "JavaScript"

    ```javascript
    // Rust only: no binding reaches the S3 Tables catalog.
    ```

An ignored test runs a table's whole life against the live service under a signed-in profile the operator names - a bucket, a namespace and a table each read and listed, a commit, a rename, every removal - and removes whatever it made before it reports, naming any bucket it could not look for.

```bash
YGGDRYL_S3TABLES_PROFILE=<profile> cargo test -p yggdryl --features s3tables --test s3tables live -- --ignored --nocapture
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

### One commit per write

The `commit` group streams eight batches through `append_arrow_reader` into a fresh table partitioned by `venue` into 32 partitions and sorted by `id` ascending, every batch interleaving every partition and descending within each, so no group arrives in the table's order: one commit of sorted partition files on one thread (`num_threads = 1`), the same on four (`num_threads = 4`), and the same stream paced to a commit every two batches (`commit_batch_num = 2`). Each measured write asserts the snapshots it made - one, or four under the cadence. No result is published here yet: the table is regenerated by a release run of the command below on the machine it names.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- "^commit/"
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

### Against PyIceberg on Amazon S3 Tables

`python/benchmarks/media/s3tables.py` is the same question against the real service, beside PyIceberg. It takes a table bucket ARN in `YGGDRYL_S3TABLES_ARN`, has PyIceberg create a table there partitioned by `symbol`, append 65,536 rows in four partitions through the service's catalog - the only door a commit to S3 Tables has - and then opens the same table both ways: PyIceberg through the catalog's REST load, this crate through the warehouse `s3:` location that load answers, with the region the ARN carries and the credentials the catalog vended. Opening the table, a full scan to Arrow, and a scan pruned to one partition of four are each timed on both sides, after the rows both read have been compared; the table is dropped afterwards. The ratio column is PyIceberg's median over this crate's, so above one is in this crate's favor. A write from this crate commits through [`S3TablesCatalog`](#iceberg-on-amazon-s3-tables); the run times reads. No table is published here: the run needs an account's own table bucket, and the numbers are those of a network round trip to it, which is why the request counts pinned above are the part that travels.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2 --repeat 5
```
