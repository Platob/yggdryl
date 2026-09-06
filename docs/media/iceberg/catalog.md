# Iceberg catalog

One warehouse folder, namespaces as nested folders, and a table per dotted name.

## Contract

| key | value |
| --- | --- |
| Owns | `Catalog` over one folder, `HadoopCatalog`'s layout |
| Storage | [`IOBase`](../../holder/index.md), nothing else |
| Kinds | `IOKind::Catalog` / `Namespace` / `Table`, answered by the framing, never by a listing; `IOKind` is Rust-only |
| Dotted names | `table`, `namespace`; the collections descend them too, and `catalog.tables` reaches any table from the root in one lookup |
| Views | cheap handles, not caches; construction performs no I/O, and two views over one catalog see each other's writes |
| Iteration | lazy; membership and iteration consult storage when asked, `values` / `items` / `entries` open one resource per step |
| Properties | `metadata/catalog.json`, `metadata/namespace.json`, through the shared JSON codec |
| Absent | `drop_table`, `rename_table`, `__delitem__`, service client |
| Bindings | `yggdryl.media.iceberg.Catalog`, `iceberg.Catalog` |
| Rust-only | `Catalogs`, over a folder of warehouses |

## Use

A caller holding rows and a dotted name needs nothing else.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::iceberg::Catalog;
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let warehouse = Folder::temporary()?.path()?.join("yggdryl-doc-warehouse");
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
    let arrow_schema = schema.into_arrow_schema()?;
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
        .append_arrow_reader("nyc.trades", yggdryl::arrow::batch_reader(first.schema(), [first]))?;
    let rows_read: usize = table.scan(None)?.map(|batch| batch.map(|b| b.num_rows())).sum::<Result<usize, _>>()?;
    assert_eq!(rows_read, 2);

    let second = rows(&[3], &["XNAS"])?;
    catalog
        .tables()
        .append_arrow_reader("nyc.trades", yggdryl::arrow::batch_reader(second.schema(), [second]))?;

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
    from yggdryl.media.iceberg import Catalog

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
    columns = pa.schema([child.into_arrow() for child in marked.dtype])

    table = catalog.append(
        "nyc.trades", pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns)
    )
    assert table.scan().read_all().num_rows == 2

    catalog.append("nyc.trades", pa.table({"id": [3], "venue": ["XNAS"]}, schema=columns))

    # The partition marks the schema carried became the table's spec.
    reopened = catalog.table("nyc.trades")
    assert [field.name for field in reopened.spec.fields] == ["venue"]
    assert reopened.scan().read_all().num_rows == 3
    assert "nyc.trades" in catalog.tables
    assert list(catalog.namespaces) == ["nyc"]
    assert list(catalog.namespaces["nyc"].tables) == ["trades"]

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
    catalog.tables.create('nyc.trades', marked)

    const rows = (ids, venues) =>
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
      })
    const table = catalog.append('nyc.trades', rows([1n, 2n], ['XNAS', 'XNYS']))
    assert.equal(table.scan().intoTable().numRows, 2)
    assert.equal(catalog.append('nyc.trades', rows([3n], ['XNAS'])).scan().intoTable().numRows, 3)

    // The dotted name is the folder nyc/trades, and the marks became the spec.
    assert.ok(catalog.tables.has('nyc.trades'))
    assert.deepEqual(catalog.table('nyc.trades').spec.fields.map((field) => field.name), ['venue'])
    assert.deepEqual(catalog.namespaces.names(), ['nyc'])
    assert.deepEqual(catalog.namespaces.get('nyc').tables.names(), ['trades'])

    fs.rmSync(warehouse, { recursive: true, force: true })
    ```

## Creating a table

`create` is the explicit spelling; `append` and `overwrite` are create-or-write.

| call | behaviour |
| --- | --- |
| `tables().create` | numbers the schema; spec from its [partition marks](../../types/field.md) |
| `append` / `overwrite` | Rust: `append_arrow_reader`, `overwrite_arrow_reader` |
| `append_arrow_reader_with_options`, twin | per-call [options](../options.md) |
| rows | what [the record surface](../../holder/iobase/records.md) infers: Python takes `append_records` inputs, JavaScript `BatchReader.from` plus `appendRecords` inputs |
| declared field | the table's stored schema; a create-on-write table has none, so the rows name one |

## Namespaces of tables

`catalog.namespaces` indexes to a `Namespace`, `namespace.tables` to a `Table`.
A nested namespace comes from its parent's own view.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::Catalog;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-views");
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

    from yggdryl.media.iceberg import Catalog

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

    # The mapping surface: keys, values, and items, exactly as a dict's -
    # values and items are lazy iterators that open one table per step.
    # len drains the listing, so it costs the full level.
    assert list(sales.tables.keys()) == ["orders"]
    assert [name for name, _ in sales.tables.items()] == ["orders"]
    assert next(sales.tables.values()).scan().read_all().num_rows == 2
    assert len(sales.tables) == 1
    # There is no __delitem__: removal is absent from the whole hierarchy,
    # because the storage contract has no delete primitive to build it on.
    assert not hasattr(sales.tables, "__delitem__")

    # A catalog and a namespace each carry properties, in one small document.
    sales.update_properties({"region": "eu"})
    assert sales.properties == {"region": "eu"}

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
    // No operator sugar exists - JavaScript gives a native class no indexing
    // hook - so the Map verbs are the spelling: has, size, keys, values,
    // entries, and for...of. values and entries open one table per step.
    assert.deepEqual([...sales.tables.keys()], ['orders'])
    assert.deepEqual([...sales.tables], ['orders'])
    assert.deepEqual([...sales.tables.entries()].map(([name]) => name), ['orders'])
    assert.equal(sales.tables.size(), 1)

    const table = catalog.namespaces.get('sales').tables.get('orders')
    assert.equal(table.scan().intoTable().numRows, 2)

    // A catalog and a namespace each carry properties, in one small document.
    sales.updateProperties({ region: 'eu' })
    assert.deepEqual(sales.properties(), { region: 'eu' })

    // A nested namespace is reached through its parent's own view.
    sales.namespaces.create('eu')
    assert.deepEqual(catalog.namespaces.get('sales').namespaces.names(), ['eu'])

    // A missing name is refused naming it, never answered as an empty table.
    assert.throws(() => catalog.namespaces.get('marketing'), /marketing/)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## The Spark quickstart, locally

The walk runs here with no Spark, no JVM, and no catalog service.

Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Float32Array, Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::media::iceberg::Table;
use yggdryl::holder::local::Folder;
use yggdryl::DataType;

let root = Folder::temporary()?.path()?.join("yggdryl-doc-nyc-taxis");
let _ = std::fs::remove_dir_all(&root);
let catalog = yggdryl::media::iceberg::Catalog::new(Folder::new(&root)?);

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
let arrow_schema = schema.into_arrow_schema()?;
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
table.commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))?;

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
table.commit_merge(
    yggdryl::arrow::batch_reader(update.schema(), [update]),
    &["trip_id".to_owned()],
    true,
)?;
assert_eq!(fares(&table)?[0], (1_000_371, 16.32));
assert_eq!(fares(&table)?.len(), 4);

// DELETE FROM nyc.taxis WHERE vendor_id = 1
// A delete is a filtered overwrite with nothing incoming: the selected
// partition is replaced by no rows, and every other file is carried over.
table.commit_overwrite_where(
    &[("vendor_id", "1")],
    yggdryl::arrow::batch_reader(Arc::clone(&arrow_schema), []),
)?;
assert_eq!(
    fares(&table)?,
    [(1_000_372, 22.15), (1_000_373, 9.01)],
);

// ALTER TABLE nyc.taxis ADD COLUMN fare_per_distance float
let mut update = yggdryl::media::iceberg::SchemaUpdate::from_metadata(table.metadata())?;
update.add_column("", DataType::Float32.nullable_field("fare_per_distance"));
let evolved = update.into_field()?;
table.commit_metadata_changes(|metadata| {
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

- Three commits, three history rows; the schema change moved no data.
- The delete replaces one partition's file with nothing, carrying the other.
- The look back reads the original fares; a commit mutates nothing in place.

## Edges

- `tables().create` on a taken name -> a typed conflict.
- A missing name -> `KeyError`, a throw, or an `Err`.
- Rust `get` -> a `Result`; nothing implements `Index`.
- No JavaScript indexing hook -> Map verbs: `get`, `has`, `size`, `keys`, `values`, `entries`, `create`, `openOrCreate`.
- `len` / `size` -> drains the listing, costing the level.
- No properties document -> empty properties, never an error.
- A key prefixed `iceberg:` -> refused; reserved for the format.
- `update_properties` on an empty namespace -> durable, plus ancestry.
- Removal anywhere -> absent; storage has no delete or move primitive.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::catalog::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^catalog_resolve/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_iceberg.py
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/iceberg.test.js
    ```
