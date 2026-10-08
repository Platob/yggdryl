# yggdryl-warehouse in Rust

Every name is at the crate root: `use yggdryl::{Warehouse, FolderCatalog,
MediaTable, ...};`. The traits must be in scope for their verbs -
`ObjectValue` (`name`, `path`, `kind`, `properties`), `NamespaceValue`
(`children`, `get`), `CatalogValue` (`namespace_levels`), `TableValue`
(`field`, `storage`), `IOMedia` (`row_size`, `read_serie`,
`read_arrow_reader`). Everything is a default-feature build; a table laid out
as a table format reads its rows under `iceberg`.

## Register a folder and resolve a path

A folder is a catalog: its folders are namespaces (one level by default), its
tabular leaves tables. Nothing is listed until a path is resolved, and
resolving is one listing per level and no read.

```rust
use yggdryl::holder::Holder;
use yggdryl::{FolderCatalog, IOMedia, ObjectValue, TableValue, Warehouse};

let root = std::env::temp_dir().join(format!("ygg-skill-warehouse-folder-{}", std::process::id()));
std::fs::create_dir_all(root.join("eu"))?;
std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,187.5\nMSFT,410.25\n")?;

let mut warehouse = Warehouse::new();
warehouse.register(FolderCatalog::bound("market", Holder::folder(&root)?))?;

let trades = warehouse.table("market.eu.trades")?;
assert_eq!(trades.path(), ["market", "eu", "trades"]);
assert_eq!(trades.storage(), "text/csv");
assert_eq!(trades.field()?.field_len(), 2);
assert_eq!(trades.row_size()?, 2);

// Every record verb is the table's: a `StreamChunkedSerie` under its own options.
let mut rows = 0;
for serie in trades.read_serie(None)?.into_chunked_stream(None, None)?.into_chunks() {
    rows += serie?.len();
}
assert_eq!(rows, 2);
std::fs::remove_dir_all(&root)?;
```

## Register a table at any location

A `MediaTable` is a table over any location a record medium reads; its memory
catalog and namespace are built along the path. A declared field is renamed
after the table and answered before any read, and stated properties open the
handle (`media_type` types an extensionless location).

```rust
use yggdryl::{DataType, IOKind, MediaTable, ObjectValue, Properties, StructType, TableValue, Url, Warehouse};

let root = std::env::temp_dir().join(format!("ygg-skill-warehouse-media-{}", std::process::id()));
std::fs::create_dir_all(&root)?;
std::fs::write(root.join("blob"), "symbol,price\nAAPL,187.5\n")?;

let row = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("symbol"),
    DataType::Float64.nullable_field("price"),
])?)
.required_field("anything");
let table = MediaTable::new("lake.raw.trades", Url::from_path(root.join("blob"))?)?
    .with_field(row)
    .with_properties(Properties::new().with_property("media_type", "text/csv"));

let mut warehouse = Warehouse::new();
warehouse.register(table)?;
assert_eq!(warehouse.get("lake")?.kind(), IOKind::Catalog);
assert_eq!(warehouse.get("lake.raw")?.kind(), IOKind::Namespace);
let trades = warehouse.table("lake.raw.trades")?;
assert_eq!(trades.field()?.name(), "trades", "the declared field is the table's");
assert_eq!(trades.properties()?.get("media_type"), Some("text/csv"));

// A taken name conflicts; `replace` swaps and answers what was there.
let error = warehouse.register(MediaTable::new("lake.raw.trades", Url::from_str("file:///x.csv")?)?).unwrap_err();
assert!(error.is_conflict());
assert!(warehouse.replace(MediaTable::new("lake.raw.trades", Url::from_str("file:///x.csv")?)?)?.is_some());
assert_eq!(warehouse.unregister("lake.raw.trades")?.kind(), IOKind::Table);
std::fs::remove_dir_all(&root)?;
```

## Walk one level with the views

`namespaces()` and `tables()` are lazy views; `children()` yields every child
as an `Object`. Names may be dotted and descend.

```rust
use yggdryl::holder::Holder;
use yggdryl::{Catalog, CatalogValue, FolderCatalog, NamespaceValue, Object, ObjectValue, TableValue};

let root = std::env::temp_dir().join(format!("ygg-skill-warehouse-views-{}", std::process::id()));
for leaf in ["trades.csv", "eu/fills.csv", "eu/lake/part-0.csv", "README.md"] {
    let path = root.join(leaf);
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    std::fs::write(&path, "symbol,price\nAAPL,187.5\n")?;
}
let market = Catalog::from(FolderCatalog::bound("market", Holder::folder(&root)?));
assert_eq!(market.namespace_levels(), Some(1));

let names = |names: yggdryl::Names| names.map(|name| name.map(|name| name.to_string())).collect::<yggdryl::Result<Vec<_>>>();
assert_eq!(names(market.namespaces().iter())?, ["eu"]);
assert_eq!(names(market.tables().iter())?, ["trades"], "the markdown is no table");
assert!(market.tables().contains("eu.fills")?);
assert_eq!(market.namespace("eu")?.tables().len()?, 2);
assert_eq!(market.tables().get("eu.lake")?.storage(), "directory", "a folder under a namespace is a table");

for child in market.children() {
    match child? {
        Object::Namespace(namespace) => assert_eq!(namespace.name(), "eu"),
        Object::Table(table) => assert_eq!(table.name(), "trades"),
        other => panic!("no catalog under a catalog: {other}"),
    }
}
std::fs::remove_dir_all(&root)?;
```

## Write through a view

The helpers open the table and write through its own record surface, creating
it from the reader's schema only where the parent creates tables - a memory
or a folder level does not, so register the table first.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::{Catalog, DataType, IOMedia, MediaTable, MemoryCatalog, NamespaceValue, Properties, StructType, Url};

let root = std::env::temp_dir().join(format!("ygg-skill-warehouse-write-{}", std::process::id()));
std::fs::create_dir_all(&root)?;
let catalog = Catalog::from(
    MemoryCatalog::new("lake").with_object(MediaTable::new("lake.ticks", Url::from_path(root.join("ticks.arrows"))?)?)?,
);
let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
let batch = RecordBatch::try_new(field.clone().into_arrow_schema()?, vec![Arc::new(Int64Array::from(vec![1_i64, 2]))])?;

let tables = catalog.tables();
assert_eq!(tables.append_arrow_reader("ticks", yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?.row_size()?, 2);
assert_eq!(tables.append_arrow_reader("ticks", yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?.row_size()?, 4);
assert_eq!(tables.overwrite_arrow_reader("ticks", yggdryl::arrow::batch_reader(batch.schema(), [batch]))?.row_size()?, 2);

let error = tables.open_or_create("fills", &field, &Properties::new()).unwrap_err();
assert_eq!(error.to_string(), "filesystem \"MemoryCatalog\" does not support creating a table");
std::fs::remove_dir_all(&root)?;
```

## State properties once

A child's effective properties are its parent's, then its own, a nearer
statement winning by name; every handle under an object opens with them.

```rust
use yggdryl::{Catalog, MediaTable, MemoryCatalog, MemoryNamespace, ObjectValue, Properties, Url};

let trades = MediaTable::new("lake.eu.trades", Url::from_str("file:///lake/eu/trades.csv")?)?
    .with_properties(Properties::new().with_property("codec", "gzip"));
let eu = MemoryNamespace::new("lake.eu")?
    .with_properties(Properties::new().with_property("region", "eu-west-1"))
    .with_object(trades)?;
let lake = Catalog::from(
    MemoryCatalog::new("lake")
        .with_properties(Properties::new().with_property("token", "t").with_property("region", "global"))
        .with_object(eu)?,
);
assert_eq!(
    lake.table("eu.trades")?.properties()?.iter().collect::<Vec<_>>(),
    [("token", "t"), ("region", "eu-west-1"), ("codec", "gzip")],
);
assert_eq!(lake.namespace("eu")?.properties()?.get("region"), Some("eu-west-1"));
```

## Hold an object as a handle

`Holder::from(object)` is `Holder::Catalog`, `Holder::Namespace` or
`Holder::Table`. A container object lists its children as handles and holds
no bytes; a table answers every record verb through its own handle.

```rust
use yggdryl::holder::Holder;
use yggdryl::{Catalog, Error, FolderCatalog, IOBase, IOKind, IOMedia};

let root = std::env::temp_dir().join(format!("ygg-skill-warehouse-handle-{}", std::process::id()));
std::fs::create_dir_all(root.join("eu"))?;
std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,187.5\n")?;

let held = Holder::from(Catalog::from(FolderCatalog::bound("market", Holder::folder(&root)?)));
assert_eq!(held.kind(), IOKind::Catalog);
assert_eq!(held.ls(true, false).count(), 2);
let trades = held.child_by_path("eu/trades")?;
assert!(matches!(trades, Holder::Table(_)));
assert_eq!(trades.kind(), IOKind::File, "a leaf table's kind is its storage's");
assert_eq!(trades.row_size()?, 1);
assert!(matches!(held.read_all_bytes(), Err(Error::NotAtomic { .. })));
assert!(held.record_options().unwrap_err().to_string().ends_with("name a table under it"));
std::fs::remove_dir_all(&root)?;
```

## The system warehouse

`SystemWarehouse` is the process's one registry, under a lock taken per
call. Use a name unique to the process and unregister it.

```rust
use yggdryl::{IOKind, MediaTable, ObjectValue, SystemWarehouse, TableValue, Url};

assert_eq!(SystemWarehouse::catalog("local")?.kind(), IOKind::Catalog);
let catalog = format!("docs_skill_{}", std::process::id());
let path = [catalog.as_str(), "raw", "trades"];
SystemWarehouse::register(MediaTable::new(path, Url::from_str("file:///lake/raw/trades.csv")?)?)?;
assert_eq!(SystemWarehouse::table(path)?.storage(), "text/csv");
assert_eq!(SystemWarehouse::unregister([catalog.as_str()])?.kind(), IOKind::Catalog);
```

## Gotchas in Rust

- `Catalog`, `Namespace` and `Table` implement both `IOBase` and
  `ObjectValue`, which both define `kind`, `url` and (`mtime`/`modified`)
  readings: with both traits in scope, write `ObjectValue::kind(&table)` or
  `IOBase::kind(&table)`. `Object` and `Holder` implement one each, so a
  verb on them is never ambiguous.
- `IntoObjectPath` takes `&str` (dotted grammar), `String`, `&[&str]`,
  `[&str; N]`, `Vec<&str>`, `Vec<SmolStr>`, `&[SmolStr]` and a `Location`;
  a `&str` with a dot inside one part must be quoted.
- `Tables::open_or_create` opens an existing table as it is: the `field`
  describes only what the call would create.
- `Namespaces::len`/`Tables::len` drain the listing; `is_empty` stops at
  the first entry of that kind.
- `FolderCatalog::bound`/`MediaTable::bound` keep the handle given, and a
  clone rebuilds from its location; a `Holder::buffer` has none, so a clone
  of an object bound to one refuses its next verb by name.
- `update_properties` is refused by every implementation here; stated
  properties are never written into a document.
