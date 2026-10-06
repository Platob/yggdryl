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
| Declarations kept | an Iceberg schema states a column's identifier, name, type, nullability and `doc` and nothing else, so the table keeps a derived column's term as `yggdryl.transform.<column>` ([Declared partitioning](#declared-partitioning-and-sort-order)) and the columns whose integers are bits as `yggdryl.representation.bits` ([Integers stated as bits](#integers-stated-as-bits)), each declared back on the schema wherever the metadata is read |
| Settings | `IcebergOptions`, each resolved from the call, then the table property, then the default: `read.parallelism`, `write.parallelism`, `write.target-file-size-bytes` and the commit retries among them; both parallelisms default to every thread the host offers, and each thread costs a bounded piece of memory: a scan worker holds its file's projected compressed column chunks and at most sixteen refined batches ahead of the release cursor (`READ_AHEAD_BATCHES`, the Parquet reader's own read-ahead), never a file decoded whole; a commit writer holds one whole encoded data file - the Parquet encoder writes into memory and publishes the file in one write, so up to about `write.target-file-size-bytes` compressed - plus up to 32 MiB of Arrow input awaiting the column writers, the group's rows themselves spilled chunks under the process spill bound; a write's `RecordOptions` add `commit_batch_num` and `num_threads`. A count or a byte size held as a table property reads as the one integer grammar reads a whole number - a sign and the digits, the surrounding blanks not part of it, no unit suffix - and a property that does not read is refused naming its key |

A table lives in one folder: `metadata/` and `data/`, no catalog required.
Iceberg's type strings - `timestamptz`, `fixed[16]`, `list<fixed[16]>`, the
`struct<1: a: optional long>` its reference implementations render - are
[datatype spellings](../types/datatype.md): each primitive reads as the
datatype the table reader maps it to, and a struct member keeps its id and
nullability. A list's or map's string carries no element id or element
nullability, so its child is the grammar's nullable `item`. A v3 `variant` and a v3 `unknown` both read as [`variant`](../types/variant.md); only a table's schema declares a column `unknown`, which it keeps out of its data files and reads back as nulls. A v1 or v2 table refuses both.

A column's description is its Iceberg `doc`, both ways: a schema written from a field states each column's `description` as that column's `doc`, at every depth, so a catalog over the table - a table bucket's console, Athena, Spark - shows what the field says it holds; a schema read states the `doc` back as the description, a line break or any other control character another writer left in one read as a space and an empty one as none; and `SchemaUpdate`'s `update_doc` sets it, an empty text clearing it. No `ICEBERG:` property holds a doc.

## Read

A table is opened from its folder - its newest metadata document, with no catalog in between: the version hint says where to start, a listing of `metadata/` where another writer left none, and from there the next version's document is read in both spellings, `v{n}.metadata.json` and `v{n}.gz.metadata.json`, until neither is there, as `HadoopTableOperations` reads one - so an open sees every commit that has returned, a hint a commit has not replaced yet included, at the newest document's two spellings and two reads past it. A document still being written - one whose read fails, empty, or not yet whole - ends that walk before it, and a hint that cannot be read, is empty or names no version - one a writer outside the crate is rewriting in place, or one a store sizes before it reads it as it is replaced - is no hint: the listing answers where the walk starts. A scan plans snapshot, manifest list, manifests, then data files, from the metadata and never by walking `data/`: the manifest list's partition summaries skip whole manifests, the manifests they keep are read side by side on `read.parallelism` threads - every thread the host offers unless configured - their entries taken in plan order, so a plan is the sequential one at the same reads, a manifest's partition tuples and column statistics skip files, and the rest of the predicate filters the rows of the files kept. A scan decodes its files side by side once two of at least 64 KiB qualify (`read.parallel.min-files`, `read.parallel.min-file-size-bytes`), and the files in flight share `read.parallelism` with the columns inside them; it answers in plan order, so it differs from a sequential scan only in speed. The scan doors - `scan`, `scan_matching`, `scan_at` - answer the files in that plan order, the manifest list's then each manifest's, and their rows state no order across files.

A record read - `read_serie`, `read_arrow_reader` and every door over them, a folder addressing the table included - answers partition after partition, in ascending partition-tuple order (a null where the sort key naming that identity column places it, else last), each partition's rows in the table's default sort order. A partition is decoded only once the one before it has been yielded, so a row limit opens no file of the partitions after the one that satisfied it. Where the order names a column a partition does not hold constant, its files - decoded side by side as a scan's are - land in one [`ChunkedSerie`](../types/chunked-serie.md) under the process [spill bound](../types/serie.md#spilling-to-disk) and are sorted out of core, unless they already keep the order: at most one partition is held at once. Its files open by where their leading sort key starts, the bound each manifest entry already carries, so a partition whose commits each cover a later stretch is read as its files and never merged. A key the read's columns leave out is not sorted on, nor any after it. A table unsorted, or sorted only by its identity partition columns, holds nothing: its files stream in tuple order, and `read_arrow_reader` is their scan, each batch reconciled and never landed. The root declares the order this proves - the table's, as far as the root holds its keys and the `select` publishes them unchanged - where partitions in tuple order are the order's own leading keys: every spec field the identity of a column that is not floating, the order opening with those columns in spec order and ascending, or the table unpartitioned; and the metadata holds no spec besides the default, so `read_arrow_field` declares from the metadata alone exactly what the stream does. A transform key is the last one declared: a partition is sorted on the column `truncate(ts, 10)` or `days(ts)` reads, so the rows of one bucket follow that column rather than the key after it. An identity partition column and a transform are declared only where the root reads their column as the table stores it, because the stored values order them, so a declared field reading one as another datatype declares nothing from that key on. Anywhere else each partition is still sorted and nothing is declared, and a plan holding a file of a spec other than the default reads in plan order. Behind a `select`, a `where` naming a column only the `select` publishes, or a row bound, `read_serie` lands the stream Arrow shapes once and carries the declaration as proven rather than reading it again.

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

A write commits a snapshot: an append keeps every row the table holds and adds its own - on a table stating its own key, only the rows whose key it lacks ([Appending to a keyed table](#appending-to-a-keyed-table)) - an overwrite replaces the partitions its rows fall in, and a merge updates the rows [its key](#the-merge-key) matches and appends the rest, committing nothing where no row changes ([A merge that changes nothing](#a-merge-that-changes-nothing)). The partition is an overwrite's unit: every live file of a partition the incoming rows reach is dropped from the new snapshot and every other file is carried as it is - same location, same statistics, same row lineage, on every format version - so reprocessing one window of a table rewrites that window's partitions and touches no other, and a source with no row replaces nothing and commits nothing. An unpartitioned table is one partition, replaced whole. A `where` naming partition values replaces those partitions instead, whatever the rows reach; `commit_overwrite_where` with no filter, and `clear`, replace the whole table. A partitioned write groups each batch by vectorized keys and computes a partition tuple once per distinct key, not once per row, and cuts each partition's rows into data files of about the target file size (`write.target-file-size-bytes`) as `yggdryl::arrow::memory_size` measures them before encoding - one file for a partition under it; the target cuts files, never commits. A commit writes its partition groups on `num_threads` threads at once where the write's options state it, else on `write.parallelism`, else `read.parallelism`, else every thread the host offers, each group's file encoding its columns on its share of them. New data files are Parquet unless `data_mime_type` names another encoding, and a scan reads each file as its manifest entry records, so one table can mix them.

A write through the record doors - `write_serie` and its three intents, the `*_arrow_reader` family, a folder addressing the table - commits **once**, when its source ends: every partition's rows are held as the chunks they arrived in - a batch falling whole in one partition the batch itself, a run of its rows a slice, interleaved rows one take - settled under the process [spill bound](../types/serie.md#spilling-to-disk) as they arrive, so an overwrite of any length is one atomic snapshot and its memory is the bound, not the stream; beside it a merge holds the stored rows its key may hit, and a keyed append the keys of its partition ([Appending to a keyed table](#appending-to-a-keyed-table)). `commit_batch_num = N` commits every `N` whole batches instead, which paces a stream whose rows would outgrow the spill folder; an overwrite then replaces each partition on the first commit that reaches it and appends to it on every later one, and an append or a merge keeps its intent in every commit, a keyed append reading what the commits before it wrote as stored. No write compacts: `compact` is the one maintenance door, and the caller runs it. A data file's directory spells its partition tuple with ASCII letters, digits, `.`, `_`, `+` and `-` alone, any other character `_` - an instant's `:` among them, which no Windows path holds - because the manifest, never the path, is the authority on a partition value. Where the table declares a sort order, each partition's rows are sorted as a whole by it - stable, through [`ChunkedSerie::into_sort_by`](../types/chunked-serie.md#sorting-uniqueness-and-partitions), each chunk sorted on its own and the chunks merged - unless the group is already in that order: a stream whose root [declares](../types/serie.md#a-declared-order) it, proven as it lands, or a group read once chunk by chunk and edge by edge, is written as it arrived. The Python and JavaScript `IcebergTable.append` and `IcebergTable.overwrite` hold their rows under the same bound and commit once too.

A commit claims its version by creating the version's document, `metadata/v{n}.metadata.json`, with `IOBase::create_bytes`: the create writes only where no document is, so of writers racing for one version exactly one publishes it and every other is told - nothing listed, no attempt file written or removed - wherever the store's create is exclusive: local storage and the local filesystem (`O_EXCL`), Amazon S3, Azure Blob Storage and an HTTP resource (`If-None-Match: *`), Google Cloud Storage (`ifGenerationMatch=0`), a ZIP member written through one archive and a buffer. Where it is not, the claim is best-effort and the later document stands: a filesystem bridged from Python or JavaScript, which asks and then writes, an HTTP origin that ignores preconditions, and two mounts of one ZIP archive, which hold an index each - A version has two names, plain and gzip, and the create excludes one: a claim that lands reads its version's other spelling once before its staged files are committed, and where that spelling holds a byte withdraws its own document and is beaten as a refused create is - of two claims under two codecs at most one commits, both withdraw when they land before either checks, and the jittered retries settle the next. A reader that meets both spellings of one version whole refuses with an error naming both documents, and a commit that meets them waits as a beaten attempt and reports the fork once its budget is spent. Best-effort still: a reader in the instant between a second claim and its withdrawal is refused rather than answered; a third writer that read the first claim before that claim's own check, and built the next version on it, has built on a document the check then withdraws; and a withdrawal the store refuses leaves the version forked, reported with every file the document names kept. After the document, the version hint is written whole with one `IOBase::write_all_bytes` - a local file through a sibling renamed over it, so a reader that mapped the old hint keeps reading it, an object in one `PUT` - and a hint write that fails once the document is durable is logged at warn while the commit answers `Ok`: the document is the commit, and every reader walks past the version a hint names. A create lists `metadata/` once before it claims `v1.metadata.json` and refuses with a conflict where any metadata document is there - another catalog's `00001-{uuid}.metadata.json`, a first document spelled `v1.gz.metadata.json` - rather than hiding that table behind a hint of its own; two creates racing under two spellings stay best-effort. A commit whose claim is lost to a winner whose document cannot be read yet waits and looks again, as one beaten by a visible winner does. A keyless append keeps one pending attempt intact while that winner is unreadable, so a retry names the same snapshot and manifest list; a winner retaining that exact snapshot is the append already committed, adopted without adding its rows again. A commit beaten by another writer rebases where that is safe: an append to a table stating no key and a metadata-only commit reload the winner and re-apply their intent, with jittered backoff bounded by `commit_retries` and `commit_total_timeout_ms`. An overwrite, a merge, an append to a keyed table or a compaction cannot - it planned against files the winner may have replaced or written keys beside, and its input is already consumed - so after the same bounded waits it fails with `CommitConflict` naming both versions, the table left as the winner made it, and the caller re-reads and retries. A conflict after a refused create whose winner stays unreadable is an ambiguous outcome: the claim may already have durably written this commit's document. The handle restores its prior in-memory state and reports `CommitConflict` when the retry budget ends, keyed append included, but retains the staged data, manifest and manifest-list files because that document may name them. A later reopen may therefore show the commit despite the conflict; if another writer held the claim, table maintenance can collect the unreferenced files. A readable winner with a distinct snapshot follows the ordinary conflict path, with a safe blind append rebased and a keyed append refused.

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

### The merge key

A merge matches on the identity partition columns first, then its key: the options' `merge_by` where it names one, else the table's own - the columns the schema's `identifier-field-ids` names, a column below structs by its path. `IOMedia::merge_by` answers that whole key from the metadata alone, no data file opened, and every door resolves it once, before a row is pulled: `merge_serie`, `write_arrow_reader` under merge and the `merge_*` family, `merge_records`, `commit_merge` with an empty selector, a `Holder` or a warehouse `Table` holding the table, `upsert into t` stating no `by`, Python's `merge` and `merge_where` with `merge_by` left out, `None` or `True` - `True` spelling the destination's own key on every record door ([Options](index.md#options)) - and JavaScript's `merge` with `mergeBy` left out or `null`, JavaScript taking no boolean. With no identifier stated the partition is the key, so a merge naming none replaces the partitions its rows fall in - through the record doors too, which refused such a merge before; an unpartitioned table stating neither is refused naming `$.merge_by`, its source never pulled. The key is the table's: another engine that changes `identifier-field-ids` changes what a merge naming none matches on. A folder handle addressing the table states no key of its own and refuses a merge naming none; open it as a table. A keyed merge on format v3 is refused, the table's own key included, until rewritten rows keep their row ids.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOMedia, Scalar, Serie, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let mut schema = row.clone();
    assign_field_ids(&mut schema, 1)?;
    // The table's own key: `id`, named by the field id the numbering gave it.
    schema.as_iceberg_mut().set_identifier_field_ids(&[1])?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-merge-key");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = IcebergTable::create(
        LocalFolder::new(&path)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;
    assert_eq!(IOMedia::merge_by(&table)?.to_string(), "id");

    let trade = |id: i64, venue: &str| Scalar::from_sequence([Scalar::from(id), Scalar::from(venue)]);
    let trades = Serie::from_scalars(row.clone(), [trade(1, "XNAS"), trade(2, "XNYS")])?;
    table.append_serie(trades.into(), None)?;

    // No key named: the merge matches on `id`, updating 2 and adding 3.
    let incoming = Serie::from_scalars(row.clone(), [trade(2, "XLON"), trade(3, "XPAR")])?;
    table.merge_serie(incoming.into(), None)?;
    // `true` names the same key outright; `false` is refused.
    let own = table.record_options()?.with_merge_by_scalar(&Scalar::from(true))?;
    table.merge_serie(Serie::from_scalars(row, [trade(1, "XAMS")])?.into(), Some(&own))?;
    assert!(own.with_merge_by_scalar(&Scalar::from(false)).is_err());

    let mut rows: Vec<Scalar> = Vec::new();
    for batch in table.read_serie(None)? {
        rows.extend(batch?.rows().into_owned());
    }
    rows.sort();
    assert_eq!(rows, [trade(1, "XAMS"), trade(2, "XLON"), trade(3, "XPAR")]);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergTable, assign_field_ids

    rows = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = assign_field_ids(rows)
    # The table's own key: `id`, named by the field id the numbering gave it.
    schema.iceberg.update({"identifier-field-ids": "1"})

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = IcebergTable.create(root, schema, None)
    table.append(pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=rows))

    # No key named: the merge matches on `id`, updating 2 and adding 3.
    table.merge_serie(pa.record_batch({"id": [2, 3], "venue": ["XLON", "XPAR"]}, schema=rows))
    # `merge` with `merge_by` left out takes the same key.
    table.merge(pa.record_batch({"id": [1], "venue": ["XAMS"]}, schema=rows))
    # `True` names it outright, on `merge` and on every record door; `False` is refused.
    table.merge(pa.record_batch({"id": [3], "venue": ["XMIL"]}, schema=rows), True)
    table.merge_serie(pa.record_batch({"id": [4], "venue": ["XETR"]}, schema=rows), merge_by=True)
    try:
        table.merge(pa.record_batch({"id": [4], "venue": ["XPAR"]}, schema=rows), False)
    except ValueError as refusal:
        assert "$.merge_by" in str(refusal)
    else:
        raise AssertionError("False names no key")

    assert table.scan().read_all().sort_by("id").to_pydict() == {
        "id": [1, 2, 3, 4],
        "venue": ["XAMS", "XLON", "XMIL", "XETR"],
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = iceberg.assignFieldIds(
      fields.struct('row', [Field.from('id: int64 not null'), Field.from('venue: utf8')], {
        nullable: false,
      }),
    )
    // The table's own key: `id`, named by the field id the numbering gave it.
    schema.set('ICEBERG:identifier-field-ids', '1')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.IcebergTable.create(root, schema)
    const trades = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    table.append(trades([1n, 2n], ['XNAS', 'XNYS']))

    // No key named: the merge matches on `id`, updating 2 and adding 3.
    table.merge(trades([2n, 3n], ['XLON', 'XPAR']))

    const merged = table.scan().intoTable()
    const venues = new Map(
      [...merged.getChild('id')].map((id, index) => [id, merged.getChild('venue').get(index)]),
    )
    assert.deepEqual(
      venues,
      new Map([
        [1n, 'XNAS'],
        [2n, 'XLON'],
        [3n, 'XPAR'],
      ]),
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### Appending to a keyed table

An append to a table whose schema states `identifier-field-ids` writes only the rows whose key - the identity partition columns, then the identifier columns, the key [a merge naming none](#the-merge-key) matches on - is neither stored in their partition nor met earlier in the same write. The first arrival is kept, every other row is counted in `IOResult.skipped_rows` ([Write results](../holder/index.md#write-results)), no stored file is rewritten, and an append that keeps no row commits no snapshot - so loading the same rows twice leaves the table as the first load did. A table stating no identifier appends every row: the declaration is the only switch. A keyed append works on format v2 and v3 alike, where a keyed merge on v3 is refused. Every append door decides the same way - `commit_append`, `append_serie`, the `append_*` family, `write_serie` and `write_arrow_reader` under append, a `Holder` or a warehouse `Table` holding the table, Python's and JavaScript's `append` - and the doors that answer an `IOResult` count what they left out; `commit_append` and the bindings' `append` answer nothing.

Within each partition the incoming rows fall in, only the key columns of the files whose statistics may hold an incoming key are read. A live file of another partition spec is read the same way where its statistics may hold an incoming key - its keys and the columns the current spec places a row by, each row given to the partition its current tuple names - and is never refused. A write paced by [`commit_batch_num`](../holder/index.md#commit-cadence) reads what its earlier commits wrote as stored, while the later commits of an overwrite append every row they bring beside its first. What is held to decide is per partition group, on its writer thread: one set of key bytes - every stored key read from the files kept and every incoming key kept - and a mask over the group's rows, never a stored row's payload; the incoming rows are held as every commit holds them, spilled chunks under the process spill bound, and at most `num_threads` groups' sets are live at once. The keys of a file of another partition spec are the exception: no group owns such a file and it may hold any group's keys, so it is read once, on the committing thread before any writer starts, and the key columns of its rows are held for every group they fall in at once, each group's until its writer folds them into its set. The key set is not under the spill bound: a partition of many keys costs its keys in memory, as a merge's own key index does.

A keyed append decides what is absent against one snapshot, and a commit that beat it may hold the keys it kept, so it does not rebase: after the configured retries it fails with `CommitConflict`, as a merge does, and a re-run skips whatever the winner wrote ([Write](#write)).

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, IOMedia, IOResult, Scalar, Serie, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let mut schema = row.clone();
    assign_field_ids(&mut schema, 1)?;
    // The table's own key: `id`, named by the field id the numbering gave it.
    schema.as_iceberg_mut().set_identifier_field_ids(&[1])?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-keyed-append");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = IcebergTable::create(
        LocalFolder::new(&path)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;

    let trade = |id: i64, venue: &str| Scalar::from_sequence([Scalar::from(id), Scalar::from(venue)]);
    let trades = |rows: Vec<Scalar>| Serie::from_scalars(row.clone(), rows);
    let first = table.append_serie(trades(vec![trade(1, "XNAS"), trade(2, "XNYS")])?.into(), None)?;
    assert_eq!(first, IOResult::new(2, 2));

    // 2 is stored and 3 arrives twice: the first 3 is kept, the other two skipped.
    let appended = table.append_serie(
        trades(vec![trade(2, "XLON"), trade(3, "XPAR"), trade(3, "XAMS")])?.into(),
        None,
    )?;
    assert_eq!(appended, IOResult::new(3, 1));
    assert_eq!(appended.skipped_rows, 2);

    // A replay keeps no row, so it commits no snapshot.
    let snapshots = table.metadata()?.snapshots().len();
    let replayed = table.append_serie(trades(vec![trade(1, "XNAS"), trade(2, "XNYS")])?.into(), None)?;
    assert_eq!((replayed.written_rows, replayed.skipped_rows), (0, 2));
    assert_eq!(table.metadata()?.snapshots().len(), snapshots);

    let mut rows: Vec<Scalar> = Vec::new();
    for batch in table.read_serie(None)? {
        rows.extend(batch?.rows().into_owned());
    }
    rows.sort();
    assert_eq!(rows, [trade(1, "XNAS"), trade(2, "XNYS"), trade(3, "XPAR")]);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, IOResult
    from yggdryl.iceberg import IcebergTable, assign_field_ids

    rows = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = assign_field_ids(rows)
    # The table's own key: `id`, named by the field id the numbering gave it.
    schema.iceberg.update({"identifier-field-ids": "1"})

    table = IcebergTable.create(IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"), schema, None)
    trades = lambda ids, venues: pa.record_batch({"id": ids, "venue": venues}, schema=rows)
    assert table.append_serie(trades([1, 2], ["XNAS", "XNYS"])) == IOResult(2, 2, 0)

    # 2 is stored and 3 arrives twice: the first 3 is kept, the other two skipped.
    appended = table.append_serie(trades([2, 3, 3], ["XLON", "XPAR", "XAMS"]))
    assert appended == IOResult(3, 1, 2)
    assert appended.skipped_rows == 2

    # A replay keeps no row, so it commits no snapshot; `append` answers nothing.
    snapshots = len(table.snapshots)
    assert table.append(trades([1, 2], ["XNAS", "XNYS"])) is None
    assert len(table.snapshots) == snapshots

    assert table.scan().read_all().sort_by("id").to_pydict() == {
        "id": [1, 2, 3],
        "venue": ["XNAS", "XNYS", "XPAR"],
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = iceberg.assignFieldIds(
      fields.struct('row', [Field.from('id: int64 not null'), Field.from('venue: utf8')], {
        nullable: false,
      }),
    )
    // The table's own key: `id`, named by the field id the numbering gave it.
    schema.set('ICEBERG:identifier-field-ids', '1')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.IcebergTable.create(root, schema)
    const trades = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    table.append(trades([1n, 2n], ['XNAS', 'XNYS']))

    // 2 is stored and 3 arrives twice: only the first 3 is written.
    table.append(trades([2n, 3n, 3n], ['XLON', 'XPAR', 'XAMS']))

    // A replay keeps no row, so it commits no snapshot.
    const snapshots = table.snapshots.length
    table.append(trades([1n, 2n], ['XNAS', 'XNYS']))
    assert.equal(table.snapshots.length, snapshots)

    const stored = table.scan().intoTable()
    const venues = new Map(
      [...stored.getChild('id')].map((id, index) => [id, stored.getChild('venue').get(index)]),
    )
    assert.deepEqual(
      venues,
      new Map([
        [1n, 'XNAS'],
        [2n, 'XNYS'],
        [3n, 'XPAR'],
      ]),
    )

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### A merge that changes nothing

A merge replaces a stored row only where the last incoming row for its key - the last of every batch the merge pulls - differs from it, every column compared ([Append and merge](../holder/index.md#append-and-merge)). A partition group whose merge changes no stored row and adds no key carries the files it read under their exact paths; a group that adds a key or changes a row rewrites the files it read; and a merge in which no group changes commits no snapshot, the table left as it was - so replaying a merge adds no version. An overwrite, and a merge keyed by the partition alone - which replaces the partitions its rows fall in - always commit. The merge still answers every row it pulled as written, changed or not ([Write results](../holder/index.md#write-results)).

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, IOMedia, IOResult, Scalar, Serie, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let mut schema = row.clone();
    assign_field_ids(&mut schema, 1)?;
    schema.as_iceberg_mut().set_identifier_field_ids(&[1])?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-replay-merge");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = IcebergTable::create(
        LocalFolder::new(&path)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;

    let trade = |id: i64, venue: &str| Scalar::from_sequence([Scalar::from(id), Scalar::from(venue)]);
    let trades = |rows: Vec<Scalar>| Serie::from_scalars(row.clone(), rows);
    table.append_serie(trades(vec![trade(1, "XNAS"), trade(2, "XNYS")])?.into(), None)?;
    let version = table.metadata_version()?;
    let snapshots = table.metadata()?.snapshots().len();

    // Every row equals the row its key holds: no file is written and no
    // snapshot committed, and the merge still answers every row it pulled.
    let replayed = table.merge_serie(trades(vec![trade(1, "XNAS"), trade(2, "XNYS")])?.into(), None)?;
    assert_eq!(replayed, IOResult::new(2, 2));
    assert_eq!(table.metadata_version()?, version);
    assert_eq!(table.metadata()?.snapshots().len(), snapshots);

    // One row that differs is one snapshot.
    table.merge_serie(trades(vec![trade(2, "XLON")])?.into(), None)?;
    assert_eq!(table.metadata()?.snapshots().len(), snapshots + 1);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, IOResult
    from yggdryl.iceberg import IcebergTable, assign_field_ids

    rows = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    schema = assign_field_ids(rows)
    schema.iceberg.update({"identifier-field-ids": "1"})

    table = IcebergTable.create(IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"), schema, None)
    trades = lambda ids, venues: pa.record_batch({"id": ids, "venue": venues}, schema=rows)
    table.append(trades([1, 2], ["XNAS", "XNYS"]))
    before = (table.version, len(table.snapshots))

    # Every row equals the row its key holds: no file is written and no snapshot
    # committed, and the merge still answers every row it pulled.
    assert table.merge_serie(trades([1, 2], ["XNAS", "XNYS"])) == IOResult(2, 2, 0)
    table.merge(trades([2], ["XNYS"]))
    assert (table.version, len(table.snapshots)) == before

    # One row that differs is one snapshot.
    table.merge(trades([2], ["XLON"]))
    assert len(table.snapshots) == before[1] + 1
    assert table.scan().read_all().sort_by("id").to_pydict() == {
        "id": [1, 2],
        "venue": ["XNAS", "XLON"],
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = iceberg.assignFieldIds(
      fields.struct('row', [Field.from('id: int64 not null'), Field.from('venue: utf8')], {
        nullable: false,
      }),
    )
    schema.set('ICEBERG:identifier-field-ids', '1')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.IcebergTable.create(root, schema)
    const trades = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    table.append(trades([1n, 2n], ['XNAS', 'XNYS']))
    const version = table.version
    const snapshots = table.snapshots.length

    // Every row equals the row its key holds: no file, no snapshot.
    table.merge(trades([1n, 2n], ['XNAS', 'XNYS']))
    assert.equal(table.version, version)
    assert.equal(table.snapshots.length, snapshots)

    // One row that differs is one snapshot.
    table.merge(trades([2n], ['XLON']))
    assert.equal(table.snapshots.length, snapshots + 1)
    assert.equal(table.scan().intoTable().numRows, 2)

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

A table is also created from what its schema declares. `PartitionSpec::from_schema` reads the root's [`PARTITION:by`](../types/protocol.md#partition-columns) - a bare column an identity field, an epoch function over a column its transform, `truncate(col, w)` a truncation, each named by its alias or by the convention (`ts_minutes`, `name_truncate`), anything else refused by name - and `IcebergTable::create` reads the root's [`SORT:by`](../types/protocol.md#sort-order) as the default sort order, `SortOrder::for_spec` where it declares none. A transform's declaration is written on the root rather than through `with_partition_by`, because its partition value lives in the manifest and not in the rows. Any other derivation is a column: `with_partition_by(["time_bucket('15 minutes', ts) as part"])` materializes `part` as a [`TRANSFORM:`](../types/protocol.md#views) column of the schema, the spec partitions on it by identity - which every engine reads - and the table computes it for every row written, through every door, after the options' declared field and before their clauses, so a `where` may name it; a value the rows carry under its name is computed again, because the table owns the derivation and what it stores is what its schema says. That is what lets a read rely on it: a partition value of `time_bucket(width, x)` bounds `x` as the bucket's whole range, at the manifest summary and at the file, so a window on `x` alone - `ts >= ... and ts < ...` - rules out the manifests of every other bucket without opening them, exactly as a time transform's period does. An Iceberg schema has nowhere to state a term, so the table keeps each one as the property `yggdryl.transform.<column>` and declares it back on the column wherever its metadata is read. An identity partition on a nanosecond timestamp reads through a view of the manifest the official parser is handed with that column spelled `long`; the partition value keeps its type. `IcebergTable::schema()` reports both keys back, `mark_partitions` writing the spec's fields the grammar can spell (a `bucket` has no spelling and is left out) and the default order its keys, so a reopened table says how it partitions and sorts. A partition group whose rows already arrive in the table's order is written as it arrived. The order is how each data file is laid out; a record read keeps it across files and declares on its root - and on `read_arrow_field` - what that proves ([Read](#read)), the part a [`Serie`](../types/serie.md#a-declared-order) can rely on without checking it again, while a scan door reads the files in plan order and its rows declare none.

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

### Integers stated as bits

Iceberg has no unsigned integer, so [`into_scheme_compat`](../types/datatype.md#compatibility-rewriting) widens a `uint64` to `decimal(20, 0)`. A column stating [`FIELD:representation=bits`](../types/protocol.md#integers-stated-as-bits) - an XXH3-64 digest nobody does arithmetic on - is a `long` instead, carrying the digest's bits, and the table keeps the declaration: an Iceberg schema states nothing beside a type, so the table property `yggdryl.representation.bits` holds the sorted, comma-separated identifiers of every column, at any depth, that states it. It is written when the table is created, restated when a schema is added - over every schema the table holds, identifiers never being reused - and declared back on every schema wherever the metadata is read; a property that is no list of identifiers is refused naming it, and an identifier a schema does not hold declares nothing on it. The table's own field, which every write is cast onto, and every scan batch's Arrow field therefore state the bits, so a write of `uint64` digests is stored by its bits and a read under a field stating `uint64` takes them back, the buffer shared both ways and no option asked for. A read sorted by such a column lands the digests and orders what it landed, so the order it declares is the digests' own. A pushed-down filter compares the stored `long`: filter a digest past `i64::MAX` by its signed value, or after the cast.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, TableMetadata};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, IOMedia, Representation, Scalar, Scheme, Serie, StructType};

    let mut digest = DataType::UInt64.required_field("digest");
    digest.as_field_properties_mut().set_representation(Representation::Bits)?;
    let logical = DataType::from(StructType::from_fields([digest])?).required_field("row");
    let schema = logical.clone().into_scheme_compat(&Scheme::ICEBERG)?;
    assert_eq!(schema.fields()[0].dtype(), &DataType::Int64);

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-bits");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = IcebergTable::create(
        LocalFolder::new(&path)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;
    let rows = Serie::from_scalars(
        logical,
        [Scalar::from_sequence([Scalar::from(u64::MAX)])],
    )?;
    table.append_serie(rows.into(), None)?;

    // Reopened, the property declares the column again.
    let reopened = IcebergTable::open(LocalFolder::new(&path)?)?;
    let column = &reopened.schema()?.fields()[0];
    assert_eq!(column.as_field_properties().representation(), Representation::Bits);
    let id = column.parquet_field_id()?.expect("a column id").to_string();
    assert_eq!(
        reopened.metadata()?.property(TableMetadata::REPRESENTATION_BITS_PROPERTY),
        Some(id.as_str())
    );
    let batch = reopened.scan(None)?.next().expect("one batch")?;
    let stored = batch.column(0).as_any().downcast_ref::<arrow_array::Int64Array>().expect("a long");
    assert_eq!(stored.values(), &[-1]);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    from yggdryl import DataType, Field, IOBase
    from yggdryl.iceberg import IcebergTable, assign_field_ids

    digest = Field("digest", "uint64", nullable=False)
    digest.field_properties.representation = "bits"
    logical = Field("row", DataType.from_fields([digest]), nullable=False)
    stored = assign_field_ids(logical.into_scheme_compat("iceberg"))
    root = pathlib.Path(tempfile.mkdtemp())

    table = IcebergTable.create(IOBase(root / "digests"), stored)
    table.append(pa.record_batch({"digest": pa.array([2**64 - 1], type=pa.uint64())}))

    reopened = IcebergTable(IOBase(root / "digests"))
    assert reopened.schema.dtype["digest"].field_properties.representation == "bits"
    assert reopened.scan().read_all().column("digest").to_pylist() == [-1]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { fields, iceberg } = require('yggdryl')

    const digest = fields.uint64('digest', { nullable: false })
    digest.fieldProperties.representation = 'bits'
    const logical = fields.struct('row', [digest], { nullable: false })
    const stored = iceberg.assignFieldIds(logical.intoSchemeCompat('iceberg'))
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const location = path.join(root, 'digests')

    const table = iceberg.IcebergTable.create(location, stored, iceberg.PartitionSpec.unpartitioned())
    table.append(new arrow.Table({ digest: arrow.vectorFromArray([2n ** 64n - 1n], new arrow.Uint64()) }))

    const reopened = iceberg.IcebergTable.open(location)
    assert.equal(reopened.schema.dtype.getFieldAt(0).fieldProperties.representation, 'bits')
    assert.deepEqual([...reopened.scan().intoTable().getChild('digest')], [-1n])

    fs.rmSync(root, { recursive: true, force: true })
    ```

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

A listing classifies each entry with one listing of its `metadata/` and no read: a folder holding a `version-hint.text` or a `*.metadata.json` is a table, every other folder a namespace, the reserved `metadata` name skipped and refused as a name. A table answered is described at its folder and reads its current document on the first verb that needs it. `create_namespace` writes the namespace document; `create_table` is `IcebergTable::create` under `PartitionSpec::from_schema` at the format version the create's `format-version` property states - else the lowest that states the schema, 3 where it holds a nanosecond timestamp, a variant or an unknown column and 2 otherwise - over the schema as Iceberg expresses it (`into_scheme_compat`): a dictionary layout is stored as the string it encodes and the rows cast to it on the way in, `float16` is widened to `float`, and a type Iceberg lacks - an interval - is refused by path with nothing created. A create descends through existing namespaces only: `tables().create("sales.eu.orders", ..)` under a missing `sales` is the absence of `sales`, never a namespace made on the way, and the view's `append` and `overwrite` create the table from the rows' own schema under the same rule.

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
| the table's first `metadata()` | the version hint, the document it names in both its spellings and the next version's two spellings, which find nothing - `file_info=1 open_input_stream=5`; every later one `none` |
| a missing table | the levels above it, then one presence answer and nothing listed or read for it: `file_info=2 list=1 open_input_stream=2` |
| `tables().create("a.b.c.orders", ..)` | three levels at that cost, the namespace's document for what the table inherits, then the create: one presence answer, one listing of `metadata/`, which finds no document under any name, the exclusive create of `v1.metadata.json` with the missing parent repaired once, one read of `v1.gz.metadata.json`, which finds no claim beside it, and the hint written whole in one output stream: `create_dir=1 create_file=2 file_info=4 list=4 open_input_stream=5 open_output_stream=1`; nothing walks the ancestry twice |
| `tables().open_or_create(..)` | absent, the get to the missing child then the create, `list=3` where the create alone lists four; present, exactly what `get` costs, because it is the same attempt |

### A table a catalog service names

A table laid out as `HadoopTables` lays one out names its current document itself - `metadata/version-hint.text`, else the highest number a listing of `metadata/` shows, then every next version's document there is - and commits by creating the next one there, exclusively. A table a catalog service keeps is named by the service instead, and its folder need take neither a listing nor a delete. `MetadataPointer` is that service reduced to its two questions - `current()`, the document named now and the token a publication is conditioned on, and `publish(token, location)`, which names the next document on condition the pointer still stands at the token - beside `remove()`, which drops the table from the catalog and which a pointer answers only where its catalog drops one; and `IcebergTable::open_pointed` and `IcebergTable::create_pointed` are the doors beside `open` and `create`.

```text
trait MetadataPointer: Debug + Send + Sync {
    fn current(&self) -> Result<PointerState>;                               // its location - none before the first document - and its token
    fn publish(&self, token: &str, location: &Url) -> Result<PointerState>;  // a moved pointer is a conflict
    fn remove(&self) -> Result<()> { .. }                                    // provided: refuses; a catalog that drops a table answers it
}
IcebergTable::open_pointed(root, pointer) -> Result<IcebergTable<H>>         // one answer, one read of the document it names
IcebergTable::create_pointed(root, format_version, schema, spec, pointer)    // version 0, published under the token read
```

A pointed table reads the one document the pointer names, its location taken relative to the root's, and writes each next one as `metadata/{version:05}-{uuid}.metadata.json` - the name Iceberg's own catalogs write, the first `00000` - which it publishes under the token the last reading or publication answered, so a commit is its files, one document and one publication, with nothing read first. The pointer's token is the condition a publication is made under, as the exclusive create of the version's document is in a folder: a publication refused because the pointer moved reads it again, one answer and one document, and an append or a metadata-only commit applies again onto the winner under the retry budget the folder contract keeps, while an overwrite, a merge or a compaction is a `CommitConflict` at once - unless the pointer still names the document it planned against, a token moved by a change that wrote no document, which it publishes again under the new token. A publication that fails otherwise is in doubt, so the pointer is read once more and a pointer naming the attempt is the commit made. No read and no commit lists the folder, writes or reads a hint, or removes a file: a failed commit's files and its document stay, unreferenced. The table's own `ls` is refused as `Error::Unsupported`, touching nothing, and its `remove` is the pointer's - whatever `recursive` says, since a catalog drops a table whole - because the catalog that keeps the pointer keeps the table: it is dropped through that catalog, never by deleting the files the catalog's document names, and a pointer whose catalog drops nothing refuses as `Error::Unsupported`, which the table restates naming its location, as its listing's refusal does - any other failure of the drop, the catalog's own refusal or the transport's, reaching the caller as it came. A drop that took forgets the document the value had read, so what it is asked next is asked of the pointer again. `metadata_version` is the number the document's name states, and the table's path, equality and hash are what they are under the folder contract - the pointer is not its identity. Rust only: a binding reaches a pointed table as the [Amazon S3 Tables](#iceberg-on-amazon-s3-tables) table its [location](#a-table-by-its-location) names.

### Iceberg on Amazon S3 Tables

An Amazon S3 Tables table bucket is a catalog on the [warehouse](../warehouse/index.md) abstraction, behind the `s3tables` feature (which implies `s3` and `iceberg`): `S3TablesCatalog` is the bucket, its namespaces one level below it (`namespace_levels` is `Some(1)`), each an `S3TablesNamespace` holding Iceberg tables, and each table the `IcebergTable` a generic `Table::Iceberg` holds, rooted on a `Handle` on the warehouse `s3:` location the service chose (`s3://<id>--table-s3`), opened through the [S3 backend](../holder/index.md#object-stores) under the catalog's session, in the bucket's region and under the catalog's properties - so the store's own names (`s3.endpoint`, `s3.region`, ...) stated on the catalog reach every table's files. The service names a table's current document, so every table is [pointed](#a-table-a-catalog-service-names) at it: `GetTableMetadataLocation` is `current()` and `UpdateTableMetadataLocation` is `publish`, whose `409 ConflictException` - a version token the table moved past - is the commit conflict. The warehouse location takes `PutObject` and `GetObject`, and nothing here asks it for more: no listing, no hint, no delete - a failed commit's files are the bucket's unreferenced-file removal's to collect, and a table's `remove` deletes none of its files: it is one `DeleteTable`, the bucket dropping the table it keeps.

`create_table` runs the steps `IcebergCatalog`'s runs, against the control plane: the schema as Iceberg states it (`into_scheme_compat`), numbered above the highest identifier it carries, partitioned by `PartitionSpec::from_schema` - a derived `PARTITION:by` entry included - and sorted by its `SORT:by`; then `CreateTable` registers the table with no schema, `GetTableMetadataLocation` answers its warehouse and token, and its first document is written and published under that token, so the document the table keeps is the crate's own rather than one the service wrote. A first document that is not published removes the registration again under its token, which a publication that took has moved past. Both catalogs create at the format version the create's `format-version` property states, else the lowest that states the schema: 3 where it holds a nanosecond timestamp, a variant or an unknown column, 2 otherwise. The service keeps no properties for a bucket or a namespace, so theirs are what was stated; a table's ride its metadata, as on any Iceberg table.

`Catalog::from_url` answers one for the table bucket's ARN, or for the `s3tables://<bucket>` location the ARN locates. Who signs is `Session::from_properties` over the properties, PyIceberg's `s3tables.`-prefixed names (`s3tables.profile-name`, `s3tables.access-key-id`, ...) read after the bare ones; `s3tables.region` and `s3tables.endpoint` are the client's region and endpoint. The bucket's ARN is the one the location was given as - read once, its region and account kept - else the `s3tables.warehouse` or `warehouse` property where one names it, else built from the `account_id` property and the client's region, else found by name among the caller's own table buckets in that region by one `ListTableBuckets` on first use, since a bare `s3tables://<bucket>` states neither. `Catalog::from_url` is the bucket's door alone - a location naming a namespace or a table below a bucket is refused at `$.url` there, and named by [the doors that take one](#a-table-by-its-location) - a `warehouse` ARN naming another bucket, or another ARN than the one the location was given as, at `$.with.warehouse`; creating a namespace under a namespace, or a table directly under the catalog, is refused by implementation name. The catalog keeps, and hands every namespace and table under it, the properties less the ones the session read: who signs is the session from there on, so `properties` lists no credential.

| Operation | Requests |
| --- | --- |
| building the catalog, a namespace or a table description | none |
| who signs a table's store requests | the catalog's session, stated in the bucket's region once per catalog and shared by every table's store client: one credential walk, one signer cache, one list of refused keys for the bucket |
| the bucket's ARN, named by `s3tables://<bucket>` alone | 1 `ListTableBuckets` per page, once |
| `children()` of the catalog | 1 `ListNamespaces` per page |
| `get` of a namespace | 1 `GetNamespace` |
| `create_namespace` | 1 `CreateNamespace` |
| `children()` of a namespace | 1 `ListTables` per page, 1 `GetTableMetadataLocation` per table as its turn comes |
| `get` of a table - `catalog.table("desk.quotes")` too, the namespace descended by description and proven by the table's own answer | 1 `GetTableMetadataLocation`, which primes the pointer |
| `create_table` | `CreateTable`, `GetTableMetadataLocation`, 1 `PutObject` and `UpdateTableMetadataLocation` |
| a table's `remove` | 1 `DeleteTable` |
| a table's first read | 1 `GetObject` of the document the request that found the table named; a pointer asked again - after a publication, a refused one - 1 `GetTableMetadataLocation` first |
| a commit | its files' `PutObject`s and 1 `UpdateTableMetadataLocation`; a refused one 1 `GetTableMetadataLocation` and 1 `GetObject` more |

The counts are pinned in `rust/tests/s3tables/catalog.rs` against the fake control plane, each table's warehouse a bucket of the fake object store the `s3` suites run on, whose log shows no listing and no delete; the pointer's own suite, over a pointer in memory and a counting filesystem, is `rust/tests/iceberg/pointer.rs`.

### The capture pipeline on Amazon S3 Tables

`python/tests/medallion.py` is the capture pipeline on two table buckets - the capture under a glob to `bronze.log_messages`, the parse to `bronze.fix_messages`, the lifecycle to `silver.fix_messages`, the books every quarter of an hour to `silver.books`, and the books read once into the three event tables. The historical comparison below was measured live on 2026-10-06 against Amazon S3 Tables in `eu-central-1` under an IAM user, over one day of a capture of two 124 KB log objects, the second of two runs in one process - every namespace and table there, every table rewritten:

| per run | before (main `671d2df32`) | after |
| --- | ---: | ---: |
| control plane (`s3tables.eu-central-1.amazonaws.com`) | 66: a `GetNamespace` and a `GetTableMetadataLocation` per table opened, a second `GetTableMetadataLocation` on its first use, 21 for the report | 6: one `UpdateTableMetadataLocation` per table written |
| the capture's bucket, two objects under a glob | 9: a table-format probe, a kind probe per object, a windowed read and a read past the end | 3: one listing, one `GET` per object |
| the books' warehouse | 353: 84 files read three times, once per event table | 175: 84 files written and read once |
| the other six warehouses | 93 | 69 |
| the report of every table | 28 | 0 |
| credential walks | 21 | 5 per process |

The books stay 84 files because the capture holds a book in 84 of the day's quarter hours and the quarter hour is the partition a window reruns whole; a tick holding a thousand books is still one file. The pipeline holds what it opened in a `Lake` - each namespace once, every table across stages, the books read once - and the rest is the crate's: the pointer primed by the request that located the table, a dotted path descended by description, one store session per bucket, and a text read of a listed object streamed through the one `GET` it owns.

=== "Rust"

    ```rust
    use yggdryl::aws::{Credentials, Session};
    use yggdryl::s3tables::{S3Tables, S3TablesCatalog};
    use yggdryl::{Arn, Catalog, CatalogValue, ObjectValue, Properties};

    // A session that states everything consults nothing: no file, no variable, no socket.
    let session = Session::new()
        .with_environment(false)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "a-secret"));
    let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;

    // The bucket is a catalog of namespaces of Iceberg tables; building one sends nothing.
    let catalog = Catalog::from(S3TablesCatalog::new("lake", S3Tables::new(session), lake.clone())?);
    assert_eq!(catalog.namespace_levels(), Some(1));
    assert_eq!(catalog.url().map(ToString::to_string).as_deref(), Some("s3tables://lake"));

    // Named by its ARN, the catalog keeps the region and the account the ARN
    // states: no listing asks for them, and the catalog keeps no credential.
    let properties = Properties::new()
        .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
        .with_property("secret_access_key", "a-secret");
    let Catalog::S3Tables(bucket) = Catalog::from_url(&lake, &properties)? else {
        unreachable!("a table bucket's ARN is an S3 Tables catalog");
    };
    assert_eq!(bucket.bucket_arn()?, &lake);
    assert!(!format!("{bucket:?}").contains("a-secret"));
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

An ignored test runs a table's life in the catalog against the live service, in a table bucket the operator names: a namespace, a table from a schema with a nanosecond instant, a UUID and a quarter-hour partition, an append, an overwrite and a read back through the catalog; it removes the table and the namespace however it ended, and leaves the files to the bucket's own removal.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> cargo test -p yggdryl --features s3tables --test s3tables catalog::live -- --ignored --nocapture
```

#### A table by its location

A location alone reaches what a table bucket keeps, with no catalog built first. `s3tables://<bucket>[/<namespace>[/<table>]]` names the bucket's catalog, one of its namespaces or one of its tables: a trailing slash names nothing, and more than a namespace and a table below the bucket - or a name the service has no namespace or table of - is refused at `$.url`, before any request. A table bucket's ARN names the catalog, and a table's ARN, `arn:<partition>:s3tables:<region>:<account>:bucket/<name>/table/<id>`, the table that identifier is, wherever a rename has moved it. That ARN is read as the ARN it is: the location it locates (`Arn::locator`, `s3tables://<bucket>/<id>`) spells the identifier where a namespace goes, so every door reads the identifier before it lowers it. One reading answers all of them:

| Door | Answers |
| --- | --- |
| `Holder::from_url(location, properties)` - Python `IOBase(location)`, JavaScript `new IOBase(location)`, under no properties | the catalog, the namespace or the table, as the handle it is, resolved when called: `kind()` says which, and Python answers the `Catalog`, `Namespace` or `IcebergTable` class. A table bucket's table costs its requests at construction, and an absent one is refused there |
| an identifier used as a handle - `Holder::from(arn)`, a `Uri` as `IOBase`; Rust only | the same, resolved under no properties - who signs is the environment's - on the first operation that needs it, and kept for the value's life |
| `IcebergTable::from_url(location, properties)` - Python `IcebergTable(location, **properties)`, JavaScript `IcebergTable.open(location, properties)` | the table: a table bucket's described, its document read on the first verb that needs it; a folder's opened, its document read as `open` reads it. A bucket or a namespace is refused by its kind at `$.url` |
| `IcebergTable::create_from_url(location, properties, version, schema, spec)` - Python `IcebergTable.create(location, schema, partition_by, format_version=None, **properties)`, JavaScript `IcebergTable.create(location, schema, partitionBy, version, properties)` | the table registered in its bucket and its first document published, as [`create_table`](#iceberg-on-amazon-s3-tables) does it. A namespace the bucket does not hold is made on the way, which is this door's alone; a table's ARN names a table that exists, and is refused |
| `IcebergTable::open_or_create_from_url` - Python `IcebergTable.open_or_create`, JavaScript `IcebergTable.openOrCreate` | the table opened, and on absence created - the location read and the bucket's catalog built once for both. A table's ARN is opened or absent, never created: an identifier names nothing a create could make |

The same three doors open, create, and open or create a table in a folder any backend holds - `file:`, an object store, a path - rooted on the handle `Holder::from_url` opens for the location under the properties, resolved on first use and again by every clone. Either way a `version` left out is the `format-version` property, else the lowest version that states the schema - 3 for a nanosecond timestamp, a variant or an unknown column, else 2 - a `spec` left out is the one the schema declares, and the schema is stored as Iceberg expresses it, numbered above the highest identifier it carries. In the bindings a `root` that is a handle keeps what it did - the folder it addresses, reopened by that location alone and under the environment, format version 2 unless stated - and properties beside a handle are refused by name: they are read beside a location only.

The properties are read as `Catalog::from_url` reads them - who signs, `s3tables.region`, `s3tables.endpoint`, the bucket's ARN from `s3tables.warehouse` or `warehouse` or from `account_id` - and the catalog is called what the `name` property says, else what the bucket is. A table reached this way states nothing of its own and inherits the properties less the ones the session read, so it lists and prints no credential. A folder table keeps the same rule without a session: the handle it is rooted on opens under every property it was given, and again by every clone, while the table states them less the ones that store's reader took - who signs, where the store is, how it is addressed - so a credential stated to open an object-store folder reaches the storage and no `properties` listing or `Debug`; a local path reads none, and a table on one states everything it was given. Dropping a table bucket's table is its `remove`: one `DeleteTable` under no version token, a table already gone being dropped, after which the value has forgotten the document it read - the handle answers that nothing is there (`Holder::exists`), and a later verb is the table's absence rather than a commit to a table that is gone; a folder table's `remove(recursive)` removes its folder, as it always did.

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
| the first read after an open | 1 `GetObject`: the request that located the table primed its pointer, so the first verb reads the document that request named - the version current when the table was located, as PyIceberg's `load_table` reads - and the service is asked again only after a publication or a refused one |

The counts are pinned in `rust/tests/s3tables/catalog.rs` against the two fakes, and an identifier used as a handle - under the environment it resolves by - in `rust/tests/s3tables_handle.rs`. A folder door costs what `open`, `create` and `open_or_create` cost on the handle `Holder::from_url` opens: `from_url` the open's five reads - the version hint, the current document in both its spellings and the next version's two spellings, which find nothing - which `rust/tests/iceberg/table.rs` (`mod located`) pins against the fake object store beside the doors' behaviour.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::IcebergTable;
    use yggdryl::{Arn, DataType, IOBase, IOKind, Properties, StructType, Url};

    // An identity stated in full: nothing below is sent.
    let properties = Properties::new()
        .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
        .with_property("secret_access_key", "a-secret");

    // The bucket is its catalog and a segment below it a namespace, by its
    // location or by its ARN: a description each, and no request.
    let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
    assert_eq!(Holder::from_url(&lake, &properties)?.kind(), IOKind::Catalog);
    let bucket = Url::from_str("s3tables://lake")?;
    assert_eq!(Holder::from_url(&bucket, &properties)?.kind(), IOKind::Catalog);
    let desk = Url::from_str("s3tables://lake/desk/")?;
    assert_eq!(Holder::from_url(&desk, &properties)?.kind(), IOKind::Namespace);

    // A location names at most a namespace and a table, and a table door
    // takes a table: both are refused where the location is read.
    let deep = Url::from_str("s3tables://lake/a/b/c")?;
    let refused = IcebergTable::from_url(&deep, &properties).unwrap_err();
    assert!(refused.to_string().contains("$.url"));
    let refused = IcebergTable::from_url(&desk, &properties).unwrap_err();
    assert!(refused.to_string().contains("got the namespace"));

    // The same doors over a folder any backend holds: created, reopened and
    // dropped by its location, the version and the spec the schema's own.
    let folder = std::env::temp_dir().join(format!("yggdryl-located-doc-{}", std::process::id()));
    let location = Url::from_path(&folder)?;
    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let created = IcebergTable::create_from_url(&location, &Properties::new(), None, schema, None)?;
    let mut table = IcebergTable::from_url(&location, &Properties::new())?;
    assert_eq!(table.metadata_file_name()?, created.metadata_file_name()?);
    assert!(table.current_snapshot()?.is_none());
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
    desk = IOBase("s3tables://lake/desk/")
    assert isinstance(desk, Namespace)
    assert str(desk) == "lake.desk"

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

`yggdryl::s3tables::S3Tables` is the client of that catalog: the control plane of a table bucket, which holds namespaces, each holding Iceberg tables. It creates, describes, lists and removes the three levels, and it moves a table's metadata location - which is what a commit to one of these tables is, since the service names the current metadata file rather than a version hint beside it. A table's files stay the [S3 backend's](../holder/index.md#object-stores) to read and write, at the warehouse `s3:` location the catalog answers. The client is behind the `s3tables` feature, which implies `s3` and `iceberg`. Every request it sends is an `http::Request` signed by [`Request::with_sigv4`](../holder/index.md#aws-identity) for the `s3tables` service as an `aws::Session` answers, to `Session::service_endpoint("s3tables", region)` unless the client states its own. The control plane is reached at the configured endpoint when one is configured - `AWS_ENDPOINT_URL_S3TABLES`, `AWS_ENDPOINT_URL`, the profile's `[services]` entry `s3tables`, its `endpoint_url` ([Where each service is reached](../holder/index.md#where-each-service-is-reached)) - else at the region's published host; a stated or configured endpoint is used whatever the session's FIPS and dual-stack switches say, which choose among the published hosts, as botocore uses one. The service takes no anonymous request, so a session that answers no credential set is refused before anything is sent. Rust only.

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

The values are descriptions rather than handles - `TableDescription` is what `GetTable` answers, `NamespaceSummary` and `TableSummary` the model's own names for a namespace reading and a listing's entries - so none of them shadows the [warehouse's](../holder/index.md) `Table` and `Namespace`.

Every verb is one request and a listing is one per page of 250, asked for when the page before it is drained; building a client or a listing sends nothing. A read or a removal that meets a `5xx`, a `429`, an error type botocore reads as throttling (`ThrottlingException` under a `400`) or a transport failure is sent again under the HTTP client's attempts; a `PUT` that creates, renames or commits is sent once, because the service may have acted on the one it did not answer - a throttled one included, where botocore would send it again. A request the service refuses because the key that signed it lapsed or is not recognized (`ExpiredTokenException`, `UnrecognizedClientException`) is signed and sent once more, only when the session then answers another set.

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

A table bucket is addressed by its ARN, which also names the region a request is signed for and sent to. Refused before any request: a name the service's model refuses - a table bucket is 3 to 63 of `0-9`, `a-z` and `-`, a namespace and a table 1 to 255 of `0-9`, `a-z` and `_` - an ARN that names no table bucket - or, for `get_table_by_arn`, no table of one - an empty version token, a rename that names no new namespace and no new name, a region that is no host label (it names the host the signed request goes to), and what the session refuses of a configured endpoint - a value naming none, a `[services]` section nobody wrote. An endpoint carrying user information, a query or a fragment is refused where it is read, and no refusal and no `Debug` repeats the user information. `create_table` sends what the schema declares beside its columns: its `PARTITION:by` as the table's `partitionSpec` and its `SORT:by` as its `writeOrder`. The service's `NotFoundException` is `Error::Absent` from a `get_*` verb and success from a `remove_*` verb; its `ConflictException` is `Error::Conflict` from a `create_*` verb; every other refusal is `Error::Remote` with the service's own status, error type and message, located at the endpoint, the region and where that region came from (the client, the bucket's ARN or the session), a commit under a version token the table has moved past (`409 ConflictException`) among them.

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
    assert_eq!(region, "eu-west-3");
    assert_eq!(tables.endpoint_url(&region)?, "https://s3tables.eu-west-3.amazonaws.com");

    // What the service's own model refuses costs no request: a table bucket's
    // name holds no upper case, and an Amazon S3 bucket is not a table bucket.
    assert!(tables.create_table_bucket("Lake").is_err());
    let objects = Arn::from_str("arn:aws:s3:::lake")?;
    assert!(tables.get_table_bucket(&objects).is_err());

    // A listing is built without a request, and that refusal is its one item.
    let mut namespaces = tables.namespaces(&objects);
    assert!(matches!(namespaces.next(), Some(Err(_))));
    assert!(namespaces.next().is_none());

    // An endpoint the caller states is where every request goes, FIPS or not:
    // the switches choose among the published hosts.
    let gateway = S3Tables::new(session.with_use_fips_endpoint(true))
        .try_with_endpoint_url("http://localhost:4566")?;
    assert_eq!(gateway.endpoint_url(&region)?, "http://localhost:4566");

    // One the session configures for the service is where it is reached.
    let configured = S3Tables::new(
        Session::new()
            .with_variables([("AWS_ENDPOINT_URL_S3TABLES", "http://localhost:4566/tables/")])
            .with_directory("/nonexistent/.aws"),
    );
    assert_eq!(configured.endpoint_url(&region)?, "http://localhost:4566/tables");
    ```

=== "Python"

    ```python
    # Rust only: no binding reaches the S3 Tables catalog.
    ```

=== "JavaScript"

    ```javascript
    // Rust only: no binding reaches the S3 Tables catalog.
    ```

An ignored test runs a table's whole life - a bucket, a namespace read and listed, a table with a schema read and listed, a commit, a rename, and every removal - against the live service, for an operator who names a signed-in profile; it removes whatever it made before it reports, and reports a bucket it could not look for.

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

### Merges and keyed appends

The `merge` group measures the key path. Three functions share an unpartitioned table of fifty single-row data files, one id each, so every file's id bounds are as tight as bounds can be: `upsert_into_50_files` merges ten stored ids whose venue flips on every iteration - a settling merge first folds their ten files into one, so each measured upsert reads that file, rewrites it and carries the other forty; `replay_into_50_files` merges the ten rows the table already holds, which reads and compares that file, writes nothing and commits no snapshot; and `append_held_keys_into_50_files` appends the same ten ids to a twin table keyed by `id` through its `identifier-field-ids`, which reads the key column of the ten files their bounds keep and commits nothing. `one_partition_of_64` merges ten rows into one `venue` partition of 64 whose every column is the key, so each measured merge reads that partition's one file, writes nothing and commits nothing - the partition alone keeps it from reading the other 63. Each setup asserts what it claims - the files folded or carried, the snapshots unchanged - before the timer starts. No result is published here yet: the table is regenerated by a release run of the command below on the machine it names.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- "^merge/"
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

Every request left is the metadata chain - the hint, the manifest list, one manifest per commit that survives the summaries, one `GET` per data file - and one upload per file a commit writes; the loopback timing only shows that nothing else hides between them. The counts were measured before a commit claimed its version with one conditional `PUT` in place of an attempt document, a listing and a delete, so the `requests after` column overstates a commit by three until the release run regenerates the table. On a real store each request is a round trip of 1-20 ms, which is what the counts are worth.

```bash
cargo bench --features "iceberg s3" -p yggdryl --bench media -- 's3/' --quick
```

### Against PyIceberg on Amazon S3 Tables

`python/benchmarks/media/s3tables.py` is the same question against the real service, beside PyIceberg. It takes a table bucket ARN in `YGGDRYL_S3TABLES_ARN`, has PyIceberg create a table there partitioned by `symbol`, append 65,536 rows in four partitions through the service's catalog - the only door a commit to S3 Tables has - and then opens the same table both ways: PyIceberg through the catalog's REST load, this crate through the warehouse `s3:` location that load answers, with the region the ARN carries and the credentials the catalog vended. Opening the table, a full scan to Arrow, and a scan pruned to one partition of four are each timed on both sides, after the rows both read have been compared; the table is dropped afterwards. The ratio column is PyIceberg's median over this crate's, so above one is in this crate's favor. A write from this crate commits through [`S3TablesCatalog`](#iceberg-on-amazon-s3-tables); the run times reads. No table is published here: the run needs an account's own table bucket, and the numbers are those of a network round trip to it, which is why the request counts pinned above are the part that travels.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2 --repeat 5
```
