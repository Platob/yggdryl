# Iceberg reads

Scans, plans, time travel, the inspection readers, the filtered writes that share the planner, and parallel decoding.

## Contract

| Item | Behavior |
| --- | --- |
| Projection | `scan(Some(&field))` gives each file its own projection mask; `None` reads every column |
| Filter | `(column, value)` pairs, text parsed through the column's datatype; `*_matching` takes a whole [expression](../../expression/index.md) |
| `ScanPlan` | `record_count`, `files_planned`, `files_skipped`, `manifests_read`, `manifests_skipped`, decided before any data file opens |
| Bindings | Python and JavaScript plans keep only those five counts; JavaScript `equals`, `compare`, `stableHash`, `clone` cover that tuple, never a path |
| Rust identity | `ScanTask` and `ScanPlan` are immutable; `stable_hash` covers tasks, exclusions, skips, counters, no handle |
| Time travel | `scan_at` / `plan_at` read a snapshot under the schema current when it was written; `scan_ref` reads a branch or tag |
| Refs | `snapshot_by_ref` resolves a branch or tag; every commit moves `main` |
| Inspection | `inspect_history`, `inspect_snapshots`, `inspect_files`: ordinary readers under PyIceberg's column names |
| Writes | Rust `commit_overwrite_where` / `commit_merge_where`; bindings `overwrite_where` / `merge_where` with the table's `IcebergOptions`; filter `None` selects every row |
| Parallel read | needs `read.parallelism` >= 2 and `read.parallel.min-files` (default 16) files of `read.parallel.min-file-size-bytes` (default 4 MiB); default parallelism is the host's, clamped to 1..=8; plan order |

## Use

The target names the columns to keep; the cast to the scan's root reads an evolved table as one shape.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use std::sync::Arc;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-pushdown");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

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
    from yggdryl.media.iceberg import Table

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
    const wanted = fields.struct('row', [schema.dtype.getFieldAt(0)], { nullable: false })
    const projected = table.scan(wanted).intoTable()
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id'])
    assert.equal(projected.numRows, 2)

    // No target reads everything.
    assert.equal(table.scan().intoTable().numCols, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

[`IOMedia::read_arrow_reader`](../../holder/iobase/records.md) reads each file under the target, minus the partition columns the file does not store.

## Planning a scan from the metadata

Rust only; [Filtered reads and filtered writes](#filtered-reads-and-filtered-writes) asserts the same counts from Python and JavaScript.

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

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-plan");
    let _ = std::fs::remove_dir_all(&path);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // One commit per venue, so the manifest list has three rows to prune.
    for (id, venue) in [(1_i64, "XNAS"), (2, "XNYS"), (3, "XLON")] {
        let batch = RecordBatch::try_new(
            schema.clone().into_arrow_schema()?,
            vec![
                Arc::new(Int64Array::from(vec![id])),
                Arc::new(StringArray::from(vec![Some(venue)])),
            ],
        )?;
        table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
    }

    // Nothing is listed: the snapshot names the manifest list, whose per-partition
    // summaries exclude two manifests before either Avro file is opened.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count()?, 1);
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

Every level prunes:

| Level | What it carries | What it skips |
| --- | --- | --- |
| Snapshot | the manifest list | every file an earlier snapshot named |
| Manifest list row | one `FieldSummary` per partition field | a whole manifest, unopened |
| Manifest entry | the file's partition tuple | one data file, unopened |
| Data file | per-column bounds and null counts | one data file, unopened |

Each level answers the [expression](../../expression/index.md) from its own statistics; a file's path answers every free [`&holder.*`](../../expression/holder.md) attribute before a byte is read. `scan_where` and `plan` build the expression from pairs; `scan_matching` and `plan_matching` take the whole language.

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

Nothing a commit writes is mutated in place, so a retained snapshot is read by an ordinary scan with the snapshot named.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-time-travel");
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
    let one = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![1]))],
    )?;
    table.commit_append(yggdryl::arrow::batch_reader(std::sync::Arc::clone(&arrow_schema), [one]))?;
    let past = table.current_snapshot().expect("one commit").snapshot_id;

    let nine = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![std::sync::Arc::new(arrow_array::Int64Array::from(vec![9]))],
    )?;
    table.commit_overwrite(yggdryl::arrow::batch_reader(arrow_schema, [nine]))?;

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
    from yggdryl.media.iceberg import Table

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
    assert sorted(table.inspect_snapshots().read_all().column("operation").to_pylist()) == [
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
    assert.deepEqual(table.scan().intoTable().getChild('id').toArray(), BigInt64Array.from([9n]))
    assert.deepEqual(table.scanAt(past).intoTable().getChild('id').toArray(), BigInt64Array.from([1n]))

    // A branch or tag resolves by name, and every commit moves `main`.
    assert.equal(table.snapshotByRef('main').snapshotId, table.currentSnapshot.snapshotId)

    // The inspection readers render the table's own record as record batches.
    assert.equal(table.inspectHistory().intoTable().numRows, 2)
    assert.deepEqual(
      Array.from(table.inspectSnapshots().intoTable().getChild('operation')).sort(),
      ['append', 'overwrite'],
    )
    assert.equal(table.inspectFiles().intoTable().numRows, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

The collect that drains a scan drains the inspection readers.

| Reader | Columns |
| --- | --- |
| `inspect_history` | when each snapshot became current; whether it is on the current ancestry chain |
| `inspect_snapshots` | operation, manifest list, summary map per retained snapshot |
| `inspect_files` | path, format, spec, rendered `column=value` partition chain, row count, size per live data file |

## Filtered reads and filtered writes

The filter is the vocabulary [`IOBase::children_where`](../../holder/iobase/partitions.md) uses, as a mapping or a sequence of pairs. `plan` reports, `scan_where` reads, `overwrite_where` replaces, and `merge` / `merge_where` upsert on a key, each through the same metadata chain.

=== "Rust"

    ```rust
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::iceberg::{DataFile, FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("qty"),
    ])?
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-filtered-writes");
    let _ = std::fs::remove_dir_all(&root);
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(Folder::new(&root)?, FormatVersion::V2, schema.clone(), spec)?;

    let arrow_schema = schema.into_arrow_schema()?;
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
        table.commit_append(rows(vec![id], vec![venue], vec![10]))?;
    }
    let inserted = table.current_snapshot().expect("three commits").snapshot_id;

    // Nothing is listed and no data file is opened: the manifest list's
    // per-partition summaries exclude two manifests before either is read.
    let plan = table.plan(&[("venue", "XNYS")])?;
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.record_count()?, 1);
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
    table.commit_overwrite_where(&[("venue", "XNYS")], rows(vec![2], vec!["XNYS"], vec![99]))?;
    let after = paths(table.data_files()?);
    assert_eq!(before.difference(&after).count(), 1, "one partition was rewritten");
    assert_eq!(before.intersection(&after).count(), 2, "the others were carried");

    // A merge upserts on the key: 3 is stored and updates, 4 is new and appends.
    table.commit_merge(rows(vec![3, 4], vec!["XLON", "XLON"], vec![7, 8]), &["id".to_owned()], true)?;
    let total: usize = table
        .scan(None)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(total, 4);

    // Narrowed first: a merge into one partition can read no other partition.
    table.commit_merge_where(
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
    from yggdryl.media.iceberg import Table

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
    assert.equal(table.scanWhere({ venue: 'XNYS' }).intoTable().numRows, 1)

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
    const merged = new Map(table.scan().intoTable().toArray().map((row) => [row.id, row.qty]))
    assert.deepEqual([...merged.keys()].sort(), [1n, 2n, 3n, 4n])
    assert.equal(merged.get(2n), 99n)
    assert.equal(merged.get(4n), 8n)

    // Narrowed first: a merge into one partition can read no other partition.
    table.mergeWhere({ venue: 'XNAS' }, rows([1n], ['XNAS'], [42n]), ['id'])
    assert.equal(table.scanWhere({ venue: 'XNAS' }).intoTable().getChild('qty').get(0), 42n)

    // History plans the same way: the snapshot before the overwrite still
    // selects one file for that partition, and it is the file that held 10.
    assert.equal(table.planAt(inserted, { venue: 'XNYS' }).filesPlanned, 1)
    assert.equal(
      table.scanAt(inserted, { venue: 'XNYS' }).intoTable().getChild('qty').get(0),
      10n,
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A filtered overwrite carries every unselected manifest entry as it stands: same path, statistics and commit order, nothing re-read or re-encoded. A merge reads only the files whose recorded key bounds could hold an incoming key; `merge_where` narrows the candidates first.

## Reading many files at once

Rust only; the fan-out is inside the core scan, so every binding gets it through `read_parallelism` / `readParallelism` and its two neighbours.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::media::iceberg::{
        FormatVersion, IcebergOptions, PartitionSpec, Table,
    };
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-parallel-read");
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
    for id in 0..3 {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![Arc::new(Int64Array::from(vec![id]))],
        )?;
        table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
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

Each worker decodes one file end to end, filters included; a reorder buffer releases batches in plan order. On the benchmark table, 32 files of 100k rows, four workers collect about twice as fast as one (`read/` group, `rust/benchmarks/media/iceberg.rs`).

## Edges

- Conjunct a file's partition tuple proves -> dropped, never re-tested per row; what no level settles is filtered row by row.
- Filter on a non-partition column -> prunes on per-file bounds only; scattered values exclude nothing, and a plan that skips nothing says so.
- `scan_at` -> a column added later is absent; a column dropped later is still present.
- `commit_metadata_changes` -> one new metadata document; a failed change or write leaves the table untouched.
- `overwrite_where` with nothing incoming -> a delete, as the [Spark quickstart](catalog.md) spells `DELETE FROM ... WHERE`.
- Coarse key bounds -> a merge reads more files, never the wrong ones.
- `overwrite_where` or `merge` after a lost commit -> no rebase; both raise, the caller re-plans. See [Iceberg writes](write.md).
- Table below a parallel threshold -> sequential read, identical batches; never more than `read.parallelism` files in flight.
- Filtered parallel scan -> fans out over the surviving files, not over the table.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::planning
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::manifest_planning
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^plan/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^read/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_iceberg_planning.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/iceberg.test.js
    YGGDRYL_BENCH_FILTER=iceberg/scan npm run --prefix node bench:media
    ```
