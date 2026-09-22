# Iceberg

Read and write Apache Iceberg tables through one [`IOBase`](../../holder/index.md) handle, with no catalog between a folder and its table.

## Contract

| Key | Value |
| --- | --- |
| Owns | `yggdryl::iceberg`: `Table`, `TableMetadata`, `Snapshot`, `PartitionSpec`, `Transform`, `Catalog`, manifest readers and writers |
| Feature flag | `iceberg`, off by default; needs Rust 1.94 (default and schema-only builds keep 1.85); enables Parquet/Arrow 59 and official Iceberg 0.10.1 |
| Delegated | Metadata and schema mutation, validation, property parsing, manifest and list reads: official Iceberg 0.10.1; no Arrow 58 value crosses any API |
| Kept local | `IOBase` publication, the [`Field`](../../types/field.md)/Arrow 59 boundary, data-file writes, deterministic manifest and list writers, planning, scans |
| Layout | One `IOBase` container: `metadata/` holds documents, manifest lists, and manifests; `data/` holds record files |
| Open | `metadata/version-hint.text`, else the highest-numbered `*.metadata.json`; a foreign filename is kept for the next `metadata-log` entry |
| Commit | A new `v{version}.metadata.json` plus the hint; nothing is mutated in place, so earlier snapshots stay readable |
| Versions | v1: singular `schema` and `partition-spec`, no sequence numbers. v2: plural keys, `last-sequence-number`. v3: `next-row-id`, row lineage |
| Bindings | Python `yggdryl.iceberg` and the JavaScript `iceberg` namespace own the table; `TableMetadata`, `read_manifest`, and transform application are Rust only |

## Use

Create in a folder, append, and reopen with no catalog in between.

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

## Pages

Iceberg answers the two surfaces every medium answers - rows as native scalars and rows as Arrow batches - and both sit on the direction pages, so one page holds every spelling of a read and one holds every spelling of a write. Data files are [Parquet](../parquet/index.md) and manifests are [Avro](../avro/index.md); the [Media](../index.md) overview lists the other encodings.

| Page | Owns |
| --- | --- |
| [Read](read.md) | rows out: a scan as native scalars, then as Arrow batches; column pushdown, planning, time travel and the inspection tables, filtered reads, parallel multi-file reads |
| [Write](write.md) | rows in: the three record methods as native scalars, then as Arrow batches; data-file sizing and compaction, `IcebergOptions`, staged commits, commit retries, branches and tags |
| [Metadata](metadata.md) | what a commit writes: the documents v1 through v3, snapshots, manifest lists and manifests |
| [Partitions](partitions.md) | the partition spec, its transforms, and the `column=value` layout it writes |
| [Schema](schema.md) | evolution and field ids, `SchemaUpdate`, schemas as JSON, primitive and nested type mappings |
| [Catalog](catalog.md) | `Catalog` over one folder, namespaces of tables, the Spark quickstart run locally |

## The `iceberg` feature

`iceberg` is not a default feature.

```toml
[dependencies]
yggdryl = { version = "0.1", features = ["iceberg"] }
```

The boundary follows the [Iceberg specification](https://iceberg.apache.org/spec/) and the official [`TableMetadataBuilder`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableMetadataBuilder.html), [`TableProperties`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.TableProperties.html), and [`ManifestList`](https://docs.rs/iceberg/0.10.1/iceberg/spec/struct.ManifestList.html) contracts.

## Interoperability

The two format exchanges run in both directions and fail when a half is missing, rather than pass quietly. The S3 Tables run is one direction - PyIceberg commits through the service, this crate reads the warehouse - and it is a benchmark rather than a check, so it reports `SKIPPED` and succeeds where it cannot run.

| Exchange | Driver | Covers |
| --- | --- | --- |
| [PyIceberg](https://py.iceberg.apache.org/) | `python scripts/check_iceberg_interop.py` (needs `pyiceberg`); the Rust half is the `iceberg::` module of the `interop` test | A partitioned v2 table read as a `StaticTable`; a PyIceberg table with other file names, manifest field order, and deflate Avro |
| Apache Spark | `python scripts/setup_spark_interop.py`, then `pytest -m spark_interop` (deselected by default; needs Java) | Field ids, primitive and nested types with nulls, transforms, time travel and refs, evolution, properties, mixed Parquet and Avro, compaction, metadata tables, statistics |
| Amazon S3 Tables | `python python/benchmarks/media/s3tables.py` (needs `pyiceberg`, `boto3`, and a table bucket ARN in `YGGDRYL_S3TABLES_ARN`; reports `SKIPPED` otherwise) | PyIceberg creates, partitions, fills and drops a table through the service's catalog; this crate opens the same table at the warehouse `s3:` location the catalog answers, signing with the credentials it vended, and the rows both read are compared before either is timed |

## Edges

- Renamed column -> resolved by field id, so a pre-rename file's column is renamed on read and pushed down under its own name.
- `uuid`, `fixed`, `time` in Spark -> no DDL spelling, so the exchange covers only the direction that exists.
- Remote catalog -> none; `Catalog` is an `IOBase` warehouse view, and commits publish through the supplied handle.
- Amazon S3 Tables -> no catalog client either; the table bucket ARN and the `s3tables:` locator are [identifiers](../../uri/arn.md), and a table is read at the warehouse `s3:` location its catalog answers, which is what `python/benchmarks/media/s3tables.py` times beside PyIceberg.
- Writing delete files, and applying deletes on read -> not implemented.
- Live position or equality delete manifests -> scans return a typed unsupported error, never undeleted rows; proven-inert manifests pass.
- Branch other than `main` -> no writes, since a commit's parent is always the current snapshot; read it with `scan_ref` and move it with `fast_forward`.
- Concurrent writers -> the commit gate re-checks the version and retries, but `IOBase` cannot make check-then-write atomic; exact only where storage serializes writers.
- Documents, snapshots and manifests -> [Metadata](metadata.md); specs and transforms -> [Partitions](partitions.md); field ids and evolution -> [Schema](schema.md); reads -> [Read](read.md); commits -> [Write](write.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::tables
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::table_metadata
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::partition_specs
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- snapshot::references snapshot::branches snapshot::expiration snapshot::validation
    cargo test --features "iceberg internals parquet" -p yggdryl --test iceberg -- mod_::interop_regressions
    cargo test --features "parquet iceberg" -p yggdryl --test interop iceberg::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^metadata/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^manifest/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^partition/'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^identity/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_iceberg.py
    python/.venv/bin/python -m pytest python/tests/test_spark_interop.py -m spark_interop
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/iceberg.test.js
    ```

## Performance

Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1. The fastavro and PyIceberg baseline over the same manifest reads sits on [Apache Avro](../avro/index.md).

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

### Iceberg over S3

The same table over the in-process S3 the object backend's own suites run on, every request counted: the `s3` group builds a fresh venue-partitioned table per measured commit, scans one of eight partitions, reads the bridge's own `.log` as one object and writes the FIX rows it holds back into a table on the store. Release Criterion `--quick`, sample size 10, on a containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores, 15 GiB; rustc 1.94.1) shared with another build at the time, so the medians are noisier than the request counts, which are exact and pinned in `accounting::iceberg` in `rust/tests/s3/mod_.rs`. The `.log` read is untouched by this work and keeps its six requests; the gap between its two medians is the noise floor of that host, and the FIX row is parsing and enrichment first, remote calls second.

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
