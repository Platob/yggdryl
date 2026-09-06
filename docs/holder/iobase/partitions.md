# Partitions

This page owns globbing and Hive partitions over a folder: lazy listings, pruning and filtering, and partition columns in the path.

## Contract

| Item | Rule |
| --- | --- |
| Owns | `ls`, `glob`, `rglob`, `children_where`, `children_matching`, `filter_partitions`, partition columns in folder records |
| Listing | Lazy until the first `next`; items are `Result`; fused after the first failure; deterministic order |
| Pattern location | `kind` is `IOKind::Directory` before any backend call; `ls` expands from the fixed root; syntax in [Patterns](../../uri/patterns.md) |
| `children_where` | Leaves only, carrying every pair; what a folder-addressed record method resolves through; sugar over `children_matching` with `&holder.partition['column'] = 'value'` |
| `filter_partitions` | `(column, value)` pairs as paths spell them; a pruned leaf is never listed or decoded, a carried column is filtered row by row, same answer either way |
| Layout authority | Leaves spelling `column=value`, else partition-marked schema fields, else one leaf named after the encoding |
| Restored values | Declared type with a schema; text without |
| Retries | Listings and whole-leaf rewrites retry a bounded number of times with a growing pause; an append never retries |
| Routing | Batch by batch; the first batch to reach a leaf performs the operation, later ones append |
| Bindings | Python listings are `pathlib`-style iterators (`iterdir`, `glob`, `rglob`); JavaScript listings are iterables |

## Use

A fixed prefix is descended, not listed and filtered.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-lake");
    let _ = std::fs::remove_dir_all(&root);
    for year in ["2024", "2025"] {
        let leaf = root.join(format!("year={year}")).join("month=01");
        std::fs::create_dir_all(&leaf)?;
        std::fs::write(leaf.join("part-0.parquet"), b"parquet")?;
    }

    let lake = Folder::new(&root)?;

    // A fixed prefix is descended, not listed and filtered.
    assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.count(), 1);
    assert_eq!(lake.glob("**/*.parquet", false)?.count(), 2);

    // Partition filters select the leaves to overwrite or upsert.
    let selected: Vec<_> = lake
        .children_where(&[("year", "2024")], false)?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].partitions(), vec![
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ]);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    for year in ("2024", "2025"):
        leaf = root / f"year={year}" / "month=01"
        leaf.mkdir(parents=True)
        (leaf / "part-0.parquet").write_bytes(b"parquet")

    lake = IOBase(root)

    assert len(list(lake.glob("year=2024/**/*.parquet"))) == 1
    assert len(list(lake.rglob("*.parquet"))) == 2

    selected = list(lake.children_where({"year": "2024"}))
    assert len(selected) == 1
    assert selected[0].partitions == (("year", "2024"), ("month", "01"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'lake')
    for (const year of ['2024', '2025']) {
      const leaf = path.join(root, `year=${year}`, 'month=01')
      fs.mkdirSync(leaf, { recursive: true })
      fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
    }

    const lake = new IOBase(root)

    // A fixed prefix is descended, not listed and filtered.
    assert.equal([...lake.glob('year=2024/**/*.parquet')].length, 1)
    assert.equal([...lake.rglob('*.parquet')].length, 2)

    // Partition filters select the leaves to overwrite or upsert.
    const selected = [...lake.childrenWhere({ year: '2024' })]
    assert.equal(selected.length, 1)
    assert.deepEqual(selected[0].partitions, [
      { column: 'year', value: '2024' },
      { column: 'month', value: '01' },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## A listing is an iterator

Every listing yields one entry at a time, so a folder with a million leaves lists like one with three.

| Guarantee | Meaning |
| --- | --- |
| Lazy | Building costs nothing; a glob whose fixed prefix names nothing reads no directory |
| Fails at the entry | The error names it; earlier items stand; `.collect::<Result<Vec<_>>>()` gives a vector |
| Deterministic | Directory entries are sorted; a walk yields a container immediately before its subtree |
| Frontier | A recursive walk holds one cursor per open depth, never its results |
| Owned | Bounded by the act is owned: `partitions` (one URL's path depth), an operation's report (expiry snapshot ids, compaction counts); unbounded by the resource is a `Listing` |
| Backend | [Local](../backends/local.md) reads one directory at a time; a native Arrow [filesystem](../backends/filesystems.md) walk loads one directory before yielding, and a foreign eager result is sorted once |

## Partition pruning and filtering

Both halves are one bound [expression](../../expression/holder.md): `&holder.partition['year'] = '2024'` against the path, `year = 2024` against the rows.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::MimeType;

    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
        .with_filter_partitions([("year", "2024"), ("month", "01")]);
    // handle.read_arrow_reader(&options)? now reads only the January
    // 2024 leaves, and only their matching rows.
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    IOBase(root / "year=2024" / "month=01" / "trades.arrows").overwrite_arrow_table(
        pa.table({"id": [1, 2]})
    )
    IOBase(root / "year=2024" / "month=02" / "trades.arrows").overwrite_arrow_table(
        pa.table({"id": [3]})
    )

    lake = IOBase(root)
    options = lake.record_options()
    options.filter_partitions = [("year", "2024"), ("month", "01")]
    reader = lake.read_arrow_reader(options=options)
    assert reader.read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const lake = path.join(root, 'lake')
    new IOBase(path.join(lake, 'year=2024', 'month=01', 'trades.arrows'))
      .overwriteRecords([{ id: 1n }, { id: 2n }])
    new IOBase(path.join(lake, 'year=2024', 'month=02', 'trades.arrows'))
      .overwriteRecords([{ id: 3n }])

    const handle = new IOBase(lake)
    const options = handle.recordOptions().withFilterPartitions([
      ['year', '2024'],
      ['month', '01'],
    ])
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 2)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Partition columns in the data

Addressing the folder restores the columns its directories spell and routes each written row to its leaf.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, MimeType};

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-partitioned");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("year=2024").join("month=01"))?;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("month"),
    ])?
    .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
            std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
            std::sync::Arc::new(arrow_array::StringArray::from(vec!["01", "01"])),
        ],
    )?;

    // The rows carry every column; the write drops the two the path spells out.
    let mut lake = Holder::folder(&root)?;
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema.clone());
    lake.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // Only `price` reached the leaf; the other two are the directory names.
    let leaf = lake.child_by_path("year=2024/month=01/part-0.arrows")?;
    assert_eq!(
        leaf.read_arrow_field(&RecordOptions::for_media_type(leaf.media_type())?)?.field_len(),
        1
    );

    // Reading the folder restores them with their declared types.
    let restored = lake
        .read_arrow_reader(&options)?
        .next()
        .expect("one batch")?;
    assert_eq!(restored.num_columns(), 3);
    assert_eq!(restored.schema().field(1).data_type(), &arrow_schema::DataType::Int32);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, RecordOptions

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "year=2024" / "month=01").mkdir(parents=True)

    schema = pa.schema([
        pa.field("price", pa.int64(), nullable=False),
        pa.field("year", pa.int32(), nullable=False),
        pa.field("month", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"price": [10, 20], "year": [2024, 2024], "month": ["01", "01"]},
        schema=schema,
    )

    # The rows carry every column; the write drops the two the path spells out.
    lake = IOBase(root)
    options = RecordOptions("part.arrows")
    options.field = schema
    lake.overwrite_arrow_batch(batch, options=options)

    # Only `price` reached the leaf; the other two are the directory names.
    leaf = lake / "year=2024" / "month=01" / "part-0.arrows"
    assert len(leaf.read_arrow_field().dtype) == 1

    # Reading the folder restores them with their declared types.
    restored = lake.read_arrow_reader(options=options).read_all()
    assert restored.column_names == ["price", "year", "month"]
    assert restored.schema.field("year").type == pa.int32()

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    fs.mkdirSync(path.join(root, 'year=2024', 'month=01'), { recursive: true })

    const schema = fields.struct(
      'row',
      [Field.from('price: int64'), Field.from('year: int32'), Field.from('month: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      price: arrow.vectorFromArray([10n, 20n], new arrow.Int64()),
      year: arrow.vectorFromArray([2024, 2024], new arrow.Int32()),
      month: arrow.vectorFromArray(['01', '01'], new arrow.Utf8()),
    })

    // The rows carry every column; the write drops the two the path spells out.
    const lake = new IOBase(root)
    const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withField(schema)
    lake.overwriteArrowReader(BatchReader.from(table), options)

    // Only `price` reached the leaf; the other two are the directory names.
    const leaf = lake.joinpath('year=2024').joinpath('month=01').joinpath('part-0.arrows')
    assert.equal(leaf.readArrowField().dtype.length, 1)

    // Reading the folder restores them with their declared types.
    const restored = lake.readArrowReader(options).intoTable()
    assert.equal(restored.numCols, 3)
    assert.equal(restored.schema.fields[1].type.toString(), 'Int32')
    assert.deepEqual(restored.getChild('month').toArray(), ['01', '01'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

A folder that spells nothing takes its layout from the schema's [partition-marked fields](../../types/protocol.md). Rust only.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, MimeType};

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-declared-layout");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;

    // Nothing is on disk, so nothing spells a layout. The schema does.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
    ])?
    .required_field("row")
    .with_partition_fields(&["year"])?;
    assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year"]);

    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
            std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
        ],
    )?;

    let mut lake = Holder::folder(&root)?;
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema);
    lake.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // The directory came from the declaration, and the leaf stores what the
    // path does not carry.
    assert!(root.join("year=2024").is_dir());

    // Reading it back reports the layout without being told it.
    let derived = lake.read_arrow_field(
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?,
    )?;
    assert_eq!(derived.partition_field_names().collect::<Vec<_>>(), ["year"]);

    let _ = std::fs::remove_dir_all(&root);
    ```

## Edges

- A range, null test, or `in` list -> `children_matching`, which takes the whole [expression](../../expression/holder.md) language.
- `len(list(listing))` in Python -> pays for the whole walk.
- A pair on an `int32` column -> an integer comparison, the text read through the column's datatype.
- `("price", "null")` in `filter_partitions` -> `price is null`, not text.
- A declared schema contradicting the stored layout -> refused, naming both.
- A column the data already carries -> left alone; the mismatch stays visible.
- Directory value `null` on a nullable declared column -> read back as null.
- Creating a tree -> address one partition directly, or declare the partition columns on the schema.
- An append racing another writer -> fails; a replayed append would duplicate rows.
- A partition touched by five batches -> rewritten five times.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::records
    cargo test --features "parquet iceberg" -p yggdryl --lib media::partition
    cargo bench --bench holder --features parquet -- io_listing
    cargo bench --bench media --features parquet -- io_pushdown
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py -k "Partitions"
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/holder/io.test.js"
    ```
