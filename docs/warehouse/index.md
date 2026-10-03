# Warehouse

One abstraction for every place that answers "which tables are there, and how do I read one": a catalog of namespaces of tables, each object a description that resolves its storage once, and a registry a dotted path - `lake.eu.trades` - resolves against.

## Contract

| | |
| --- | --- |
| Owns | the four traits `ObjectValue`, `NamespaceValue`, `CatalogValue`, `TableValue` and the four enums `Object`, `Catalog`, `Namespace`, `Table`; `Properties`; `IntoObjectPath`; the lazy views `Namespaces`, `Tables`, `Names`, `Objects`; `Warehouse` and `SystemWarehouse`; the generic implementations `MemoryCatalog`, `MemoryNamespace`, `FolderCatalog`, `FolderNamespace`, `MediaTable` with `FolderLayout`; `Handle`, the handle an object opens once on first use under its effective properties and a clone rebuilds; the `Holder::Catalog`, `Holder::Namespace` and `Holder::Table` [handle variants](../holder/index.md#variants). The `iceberg` feature adds the `Iceberg` variant of each enum, [`IcebergCatalog`, `IcebergNamespace` and `IcebergTable`](../media/iceberg.md#catalog) |
| Rust | `yggdryl::warehouse`, every name re-exported as `yggdryl::<Name>` |
| Python | `yggdryl.warehouse` (also `yggdryl`): `Catalog`, `Namespace`, `Table` are `IOBase` subclasses and `type(object)` is the implementation - `MemoryCatalog`, `FolderCatalog`, `MemoryNamespace`, `FolderNamespace`, `MediaTable`, and `yggdryl.iceberg`'s `IcebergCatalog`, `IcebergNamespace`, `IcebergTable`; `Namespaces` and `Tables` are mappings; `Warehouse`, `SystemWarehouse` |
| JavaScript | the frozen `warehouse` namespace: one class per kind - `Catalog`, `Namespace`, `Table` - with a static constructor per implementation (`Catalog.memory`, `Catalog.folder`, `Catalog.fromUrl`, `Namespace.memory`, `Namespace.folder`, `Table.media`) and `implementation` naming it; `MemoryCatalog`, `FolderCatalog`, `MemoryNamespace`, `FolderNamespace`, `MediaTable` as constructors over those statics, `iceberg.IcebergCatalog`, `iceberg.IcebergNamespace` and `iceberg.IcebergTable` the Iceberg ones, each with `from(object)` and `intoCatalog`/`intoNamespace`/`intoTable` to cross; `Namespaces`, `Tables` are Map-like; `Warehouse`, `SystemWarehouse`; `IOBase.from(object)` holds an object as the handle it is |
| Validated | a path at its intake, through the plan's [location grammar](../expression/plans.md#locations-and-targets); a namespace path of at least two parts and a table path of at least one; a registration exactly one level below its parent, under a name free at that level; a `type` property of `memory`, `folder` or, under `iceberg`, `hadoop` |
| Lazy | construction touches nothing; an object's handle is opened on the first verb that needs it; a folder is listed when it is asked, so a table written a moment ago is found on the next ask; `children`, `Names`, `Namespaces` and `Tables` walk as they are drained |
| Cached | nothing but the resolved handle, kept for the object's life; a clone starts unresolved and rebuilds from its location |
| Refused | a URL or a `with (...)` clause where a path is expected, at `$.path`; creating under a memory or a folder object, by implementation name - an Iceberg catalog creates; registering under an object that lists its own store; the byte verbs of a catalog or a namespace (`NotAtomic`) and its record verbs (name a table under it); a `type` this build has no catalog for |
| Build | default; a table laid out as a table format needs `iceberg` to read its rows, and is `Table::Iceberg` there |

## Use

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::{FolderCatalog, IOKind, IOMedia, MediaTable, ObjectValue, TableValue, Url, Warehouse};

    let root = std::env::temp_dir().join(format!("yggdryl-docs-warehouse-use-{}", std::process::id()));
    std::fs::create_dir_all(root.join("eu"))?;
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,187.5\nMSFT,410.25\n")?;

    // A folder is a catalog: its folders are namespaces, its tabular leaves tables.
    let mut warehouse = Warehouse::new();
    warehouse.register(FolderCatalog::bound("market", Holder::folder(&root)?))?;

    // A dotted path resolves to the table, one listing per level and no read.
    let trades = warehouse.table("market.eu.trades")?;
    assert_eq!(trades.to_string(), "market.eu.trades");
    assert_eq!(trades.storage(), "text/csv");
    assert_eq!(trades.field()?.field_len(), 2);

    // The table is a handle: every record verb reads its rows.
    let mut rows = 0;
    for serie in trades.read_serie(None)? {
        rows += serie?.len();
    }
    assert_eq!(rows, 2);

    // A table at any location registers at a path of its own, the memory
    // catalog and namespace along the path built as needed.
    let csv = Url::from_path(root.join("eu/trades.csv"))?;
    warehouse.register(MediaTable::new("lake.raw.trades", csv)?)?;
    assert_eq!(warehouse.get("lake.raw")?.kind(), IOKind::Namespace);
    assert_eq!(warehouse.table("lake.raw.trades")?.row_size()?, 2);
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.warehouse import FolderCatalog, MediaTable, MemoryNamespace, Warehouse

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "eu").mkdir()
    (root / "eu" / "trades.csv").write_text("symbol,price\nAAPL,187.5\nMSFT,410.25\n")

    # A folder is a catalog: its folders are namespaces, its tabular leaves tables.
    warehouse = Warehouse()
    warehouse.register(FolderCatalog("market", root))

    # A dotted path resolves to the table, which is a handle like any other.
    trades = warehouse.table("market.eu.trades")
    assert type(trades) is MediaTable
    assert str(trades) == "market.eu.trades"
    assert trades.storage == "text/csv"
    assert trades.field().name == "trades"
    assert trades.read_arrow_reader().read_all().num_rows == 2

    # A table at any location registers at a path of its own, the memory
    # catalog and namespace along the path built as needed.
    warehouse.register(MediaTable("lake.raw.trades", root / "eu" / "trades.csv"))
    assert type(warehouse.namespace("lake.raw")) is MemoryNamespace
    assert warehouse.table("lake.raw.trades").row_size() == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    fs.mkdirSync(path.join(root, 'eu'))
    fs.writeFileSync(path.join(root, 'eu', 'trades.csv'), 'symbol,price\nAAPL,187.5\nMSFT,410.25\n')

    // A folder is a catalog: its folders are namespaces, its tabular leaves tables.
    const registry = new warehouse.Warehouse()
    registry.register(warehouse.Catalog.folder('market', root))

    // A dotted path resolves to the table; `IOBase.from` holds it as the handle it is.
    const trades = registry.table('market.eu.trades')
    assert.equal(trades.implementation, 'MediaTable')
    assert.equal(String(trades), 'market.eu.trades')
    assert.equal(trades.storage, 'text/csv')
    assert.equal(trades.field().name, 'trades')
    assert.equal(IOBase.from(trades).readArrowReader().intoTable().numRows, 2)

    // A table at any location registers at a path of its own, the memory
    // catalog and namespace along the path built as needed.
    registry.register(warehouse.Table.media('lake.raw.trades', path.join(root, 'eu', 'trades.csv')))
    assert.equal(registry.namespace('lake.raw').implementation, 'MemoryNamespace')
    assert.equal(IOBase.from(registry.table('lake.raw.trades')).rowSize(), 2)
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Catalogs, namespaces and tables

Four traits say what every object a path reaches answers, and four enums say which implementation answers it, the way `Holder` does for storage and `Media` for encodings. An object is a description - its path, what it states, where its storage is - never a view borrowed from its parent, so any object sits in an enum, is registered, and crosses a binding boundary.

```text
ObjectValue:    name, path, kind, description, url, modified, properties, update_properties
NamespaceValue: children, get, create_namespace, create_table      // + ObjectValue
CatalogValue:   namespace_levels                                   // + NamespaceValue
TableValue:     field, storage                                     // + ObjectValue + IOBase

Object    { Catalog, Namespace, Table }   as_catalog, as_namespace, as_table, into_table, into_namespace, into_holder
Catalog   { Memory, Folder, Iceberg }     from_url, resolve, table, namespace, namespaces(), tables()
Namespace { Memory, Folder, Iceberg }     resolve, namespaces(), tables()
Table     { Media, Iceberg }              every IOBase and IOMedia verb, delegated
```

| Kind | `IOKind` | Enum | Python class | JavaScript |
| --- | --- | --- | --- | --- |
| the first namespace layer, registered by name | `Catalog` (`catalog`) | `Catalog` | `Catalog`, as `MemoryCatalog`, `FolderCatalog` or `IcebergCatalog` | `warehouse.Catalog`, `implementation` `'MemoryCatalog'`, `'FolderCatalog'` or `'IcebergCatalog'` |
| a container of namespaces and tables | `Namespace` (`namespace`) | `Namespace` | `Namespace`, as `MemoryNamespace`, `FolderNamespace` or `IcebergNamespace` | `warehouse.Namespace` |
| rows every record read and write reaches | `Table` (`table`) | `Table` | `Table`, as `MediaTable` or `IcebergTable` | `warehouse.Table` |

An object displays as its dotted path, quoted only where the grammar needs it; equality and hash are the description - path, location, stated properties - never the store's contents.

=== "Rust"

    ```rust
    use yggdryl::{Catalog, CatalogValue, IOKind, MediaTable, MemoryCatalog, MemoryNamespace, Namespace, NamespaceValue, Object, ObjectValue, Url};

    let fills = MediaTable::new(["lake", "eu west", "fills"], Url::from_str("file:///lake/eu/fills.csv")?)?;
    let eu = MemoryNamespace::new(["lake", "eu west"])?.with_object(fills)?;
    let lake = Catalog::from(MemoryCatalog::new("lake").with_description("the lake").with_object(eu)?);

    assert_eq!(lake.kind(), IOKind::Catalog);
    assert_eq!(lake.description(), Some("the lake"));
    assert_eq!(lake.namespace_levels(), None, "memory namespaces nest to any depth");

    // A path descends one `get` per part; a part the grammar must quote is quoted.
    let table = lake.table("\"eu west\".fills")?;
    assert_eq!(table.path(), ["lake", "eu west", "fills"]);
    assert_eq!(table.name(), "fills");
    assert_eq!(table.to_string(), "lake.\"eu west\".fills");

    // `resolve` answers whichever kind is there; the enums narrow it.
    match lake.resolve(["eu west"])? {
        Object::Namespace(Namespace::Memory(namespace)) => assert_eq!(namespace.registered().len(), 1),
        other => panic!("expected a namespace, got {other}"),
    }
    assert!(lake.resolve(["eu west"])?.as_namespace().is_some());
    assert!(lake.table(["eu west"]).unwrap_err().is_absent(), "a namespace where a table is asked for is absence");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.warehouse import Catalog, MediaTable, MemoryCatalog, MemoryNamespace, Namespace

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "fills.csv").write_text("symbol,qty\nAAPL,10\n")

    fills = MediaTable(["lake", "eu west", "fills"], root / "fills.csv")
    eu = MemoryNamespace(["lake", "eu west"], objects=[fills])
    lake = MemoryCatalog("lake", description="the lake", objects=[eu])

    # Every object is a handle; its class is the implementation, its base the kind.
    assert isinstance(lake, Catalog) and isinstance(lake, IOBase)
    assert lake.kind() == "catalog"
    assert lake.description == "the lake"
    assert lake.namespace_levels is None

    # A path descends one level per part; a part the grammar must quote is quoted.
    table = lake.table('"eu west".fills')
    assert table.path == ("lake", "eu west", "fills")
    assert table.name == "fills"
    assert str(table) == 'lake."eu west".fills'
    assert repr(table) == """MediaTable('lake."eu west".fills')"""

    # `resolve` answers whichever kind is there.
    assert isinstance(lake.resolve(["eu west"]), Namespace)
    assert lake.resolve(["eu west"]) == eu
    try:
        lake.table(["eu west"])
    except ValueError as error:
        assert str(error) == 'expected a table at "lake.\\"eu west\\"", got nothing'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    fs.writeFileSync(path.join(root, 'fills.csv'), 'symbol,qty\nAAPL,10\n')

    const fills = warehouse.Table.media(['lake', 'eu west', 'fills'], path.join(root, 'fills.csv'))
    const eu = warehouse.Namespace.memory(['lake', 'eu west'], { objects: [fills] })
    const lake = warehouse.Catalog.memory('lake', { description: 'the lake', objects: [eu] })

    // One class per kind; `implementation` names what answers.
    assert.ok(lake instanceof warehouse.Catalog)
    assert.equal(lake.kind, 'catalog')
    assert.equal(lake.implementation, 'MemoryCatalog')
    assert.equal(lake.description, 'the lake')
    assert.equal(lake.namespaceLevels, null)

    // A path descends one level per part; a part the grammar must quote is quoted.
    const table = lake.table('"eu west".fills')
    assert.deepEqual(table.path, ['lake', 'eu west', 'fills'])
    assert.equal(table.name, 'fills')
    assert.equal(String(table), 'lake."eu west".fills')

    // `resolve` answers whichever kind is there.
    assert.ok(lake.resolve(['eu west']) instanceof warehouse.Namespace)
    assert.ok(lake.resolve(['eu west']).equals(eu))
    assert.throws(() => lake.table(['eu west']), /expected a table at "lake\.\\"eu west\\"", got nothing/)
    fs.rmSync(root, { recursive: true, force: true })
    ```

### The views

`namespaces()` and `tables()` are lazy views over one level: constructing one touches nothing, every question is asked of the store when it is asked, and a dotted name descends. Both answer `get`, `create`, `open_or_create`, `contains`, `iter`, `len` and `is_empty`; `Tables` adds `append_arrow_reader` and `overwrite_arrow_reader`, which open the table - creating it from the reader's schema where the parent creates tables - and write through its own record surface.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::{Catalog, DataType, IOMedia, MediaTable, MemoryCatalog, NamespaceValue, Properties, StructType, Url};

    let root = std::env::temp_dir().join(format!("yggdryl-docs-warehouse-views-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let ticks = MediaTable::new("lake.ticks", Url::from_path(root.join("ticks.arrows"))?)?;
    let catalog = Catalog::from(MemoryCatalog::new("lake").with_object(ticks)?);

    let tables = catalog.tables();
    assert!(tables.contains("ticks")?);
    assert!(!tables.contains("fills")?);
    assert_eq!(tables.len()?, 1);
    assert!(catalog.namespaces().is_empty()?);

    // The write helpers reach the table's own record surface.
    let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let batch = RecordBatch::try_new(
        field.clone().into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64, 2]))],
    )?;
    let written = tables.append_arrow_reader("ticks", yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?;
    assert_eq!(written.row_size()?, 2);
    let replaced = tables.overwrite_arrow_reader("ticks", yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    assert_eq!(replaced.row_size()?, 2);

    // A memory level lists what was registered and creates nothing.
    let error = tables.open_or_create("fills", &field, &Properties::new()).unwrap_err();
    assert_eq!(error.to_string(), "filesystem \"MemoryCatalog\" does not support creating a table");
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl.warehouse import MediaTable, MemoryCatalog, Tables

    root = pathlib.Path(tempfile.mkdtemp())
    catalog = MemoryCatalog("lake", objects=[MediaTable("lake.ticks", root / "ticks.arrows")])

    # A mapping over one level, asked of the store when asked.
    tables = catalog.tables
    assert isinstance(tables, Tables)
    assert list(tables) == ["ticks"]
    assert "ticks" in tables and "fills" not in tables
    assert len(tables) == 1
    assert len(catalog.namespaces) == 0
    assert tables.get("fills") is None

    # The write helpers take any shape the record surface takes.
    written = tables.append("ticks", pa.table({"id": [1, 2]}))
    assert str(written) == "lake.ticks"
    assert written.row_size() == 2
    tables.append("ticks", [{"id": 3}])
    assert tables["ticks"].row_size() == 3
    assert tables.overwrite("ticks", pa.table({"id": [7]})).row_size() == 1

    # A missing key carries the core's message; a memory level creates nothing.
    try:
        tables["fills"]
    except KeyError as error:
        assert 'expected a child at "lake.fills", got nothing' in str(error)
    try:
        tables.open_or_create("fills", "fills: struct<id: int64>")
    except ValueError as error:
        assert str(error) == 'filesystem "MemoryCatalog" does not support creating a table'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    const catalog = warehouse.Catalog.memory('lake', {
      objects: [warehouse.Table.media('lake.ticks', path.join(root, 'ticks.arrows'))],
    })

    // Map-like over one level, asked of the store when asked.
    const tables = catalog.tables()
    assert.ok(tables instanceof warehouse.Tables)
    assert.deepEqual([...tables.keys()], ['ticks'])
    assert.equal(tables.has('ticks'), true)
    assert.equal(tables.has('fills'), false)
    assert.equal(tables.size, 1)
    assert.equal(catalog.namespaces().size, 0)

    // The write helpers take anything `BatchReader.from` reads.
    const rows = new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) })
    const written = tables.append('ticks', rows)
    assert.equal(String(written), 'lake.ticks')
    assert.equal(IOBase.from(written).rowSize(), 2)
    assert.equal(IOBase.from(tables.overwrite('ticks', rows)).rowSize(), 2)

    // Absence names the path; a memory level creates nothing.
    assert.throws(() => tables.get('fills'), /expected a child at "lake.fills", got nothing/)
    assert.throws(
      () => tables.openOrCreate('fills', 'fills: struct<id: int64>'),
      /"MemoryCatalog" does not support creating a table/,
    )
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Properties

`Properties` is the one ordered name/value bag: a target's `with (...)` clause, what `Holder::from_url` and the backend option doors read, and what an object states or keeps. It holds one value per name - a later set replaces in place - and keeps every name as written. An object's effective properties are its parent's, then what its store keeps for it, then what was stated for it, a later entry replacing an earlier one by name; they propagate, so every child an object answers carries them and every handle under it opens with them - credentials stated once on a catalog reach every table's storage.

=== "Rust"

    ```rust
    use yggdryl::{Catalog, MediaTable, MemoryCatalog, MemoryNamespace, ObjectValue, Properties, Url, Warehouse};

    // One value per name, replaced in place; a knob read through the one reader
    // of its type; the clause it displays as.
    let bag = Properties::new()
        .with_property("region", "eu-west-1")
        .with_property("batch_row_size", "1024")
        .with_property("region", "us-east-1");
    assert_eq!(bag.iter().collect::<Vec<_>>(), [("region", "us-east-1"), ("batch_row_size", "1024")]);
    assert_eq!(bag.knob_count::<u64>("batch_row_size")?, Some(1024));
    assert_eq!(bag.knob_bool("safe")?, None);
    assert_eq!(bag.to_string(), "region = 'us-east-1', batch_row_size = '1024'");

    // A child inherits its parent's bag, its own values winning, the parent's order kept.
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

    // Nothing here keeps properties in a store, so persisting is refused by name.
    let error = lake.update_properties(&Properties::new().with_property("owner", "ops"), &[]).unwrap_err();
    assert_eq!(error.to_string(), "filesystem \"MemoryCatalog\" does not support updating the properties it keeps");

    // A warehouse answers the properties of the deepest registered object whose
    // URL holds a location, on a path boundary.
    let mut warehouse = Warehouse::new();
    warehouse.register(lake)?;
    let below = Url::from_str("file:///lake/eu/trades.csv/part-0")?;
    assert_eq!(warehouse.properties_for(&below).get("codec"), Some("gzip"));
    assert!(warehouse.properties_for(&Url::from_str("file:///lake/eu/trades.csvx")?).is_empty());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.warehouse import MediaTable, MemoryCatalog, MemoryNamespace, Warehouse

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")

    # A mapping, keyword properties merged after it; every value its text.
    lake = MemoryCatalog("lake", properties={"token": "t", "region": "global"}, retries=3)
    assert lake.properties == {"token": "t", "region": "global", "retries": "3"}
    assert list(lake.properties) == ["token", "region", "retries"]

    # A child inherits its parent's bag, its own values winning, the parent's order kept.
    trades = MediaTable("lake.eu.trades", root / "trades.csv", codec="gzip")
    eu = MemoryNamespace("lake.eu", region="eu-west-1", objects=[trades])
    lake = MemoryCatalog("lake", token="t", region="global", objects=[eu])
    assert lake.table("eu.trades").properties == {"token": "t", "region": "eu-west-1", "codec": "gzip"}
    assert list(lake.table("eu.trades").properties) == ["token", "region", "codec"]

    # Nothing here keeps properties in a store, so persisting is refused by name.
    try:
        lake.update_properties({"owner": "ops"})
    except ValueError as error:
        assert "does not support updating the properties it keeps" in str(error)

    # A warehouse answers the properties of the deepest registered object whose
    # URL holds a location, on a path boundary.
    warehouse = Warehouse()
    warehouse.register(lake)
    assert warehouse.properties_for(root / "trades.csv") == {"codec": "gzip"}
    assert warehouse.properties_for(root / "trades.csvx") == {}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    fs.writeFileSync(path.join(root, 'trades.csv'), 'symbol,price\nAAPL,187.5\n')

    // An ordered plain object; numbers and booleans are spelled as text.
    const stated = warehouse.Catalog.memory('lake', { properties: { token: 't', region: 'global', retries: 3 } })
    assert.deepEqual(stated.properties, { token: 't', region: 'global', retries: '3' })
    assert.deepEqual(Object.keys(stated.properties), ['token', 'region', 'retries'])

    // A child inherits its parent's bag, its own values winning, the parent's order kept.
    const trades = warehouse.Table.media('lake.eu.trades', path.join(root, 'trades.csv'), { properties: { codec: 'gzip' } })
    const eu = warehouse.Namespace.memory('lake.eu', { properties: { region: 'eu-west-1' }, objects: [trades] })
    const lake = warehouse.Catalog.memory('lake', { properties: { token: 't', region: 'global' }, objects: [eu] })
    assert.deepEqual(lake.table('eu.trades').properties, { token: 't', region: 'eu-west-1', codec: 'gzip' })

    // Nothing here keeps properties in a store, so persisting is refused by name.
    assert.throws(() => lake.updateProperties({ owner: 'ops' }), /does not support updating the properties it keeps/)

    // A warehouse answers the properties of the deepest registered object whose
    // URL holds a location, on a path boundary.
    const registry = new warehouse.Warehouse()
    registry.register(lake)
    assert.deepEqual(registry.propertiesFor(path.join(root, 'trades.csv')), { codec: 'gzip' })
    assert.deepEqual(registry.propertiesFor(path.join(root, 'trades.csvx')), {})
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Paths

`IntoObjectPath` is the one intake for a path into a warehouse. Dotted text is read through the plan's [location grammar](../expression/plans.md#locations-and-targets), so quoting is the grammar's - `lake."eu west".fills`, backticks or `[...]` - the empty text is the root, and parts given as a list arrive as they are, a dot inside one being no separator. A URL or a `with (...)` clause where a path is expected is refused by name at `$.path`, and nothing past the intake splits a path on `.`.

=== "Rust"

    ```rust
    use yggdryl::IntoObjectPath;

    let parts = |text: &str| -> yggdryl::Result<Vec<String>> {
        Ok(text.into_object_path()?.iter().map(ToString::to_string).collect())
    };
    assert_eq!(parts("lake.eu.trades")?, ["lake", "eu", "trades"]);
    assert_eq!(parts("lake.\"eu west\".fills")?, ["lake", "eu west", "fills"]);
    assert_eq!(parts("lake.`eu west`.fills")?, ["lake", "eu west", "fills"]);
    assert_eq!(parts("lake.[eu west].fills")?, ["lake", "eu west", "fills"]);
    assert_eq!(parts("\"a.b\".c")?, ["a.b", "c"], "a quoted dot is no separator");
    assert!(parts("")?.is_empty(), "the empty text is the root");

    // Parts arrive as they are, however they would have to be quoted.
    assert_eq!(["lake", "eu west"].into_object_path()?, ["lake", "eu west"]);
    assert_eq!(vec!["a.b", "c"].into_object_path()?, ["a.b", "c"]);

    let error = "'file:///lake/trades.parquet'".into_object_path().unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $.path: expected a path of parts, got the URL file:///lake/trades.parquet",
    );
    let error = "lake.eu with (media_type = 'text/csv')".into_object_path().unwrap_err();
    assert!(error.to_string().ends_with("got a `with (...)` clause on `lake.eu`"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.warehouse import MediaTable

    root = pathlib.Path(tempfile.mkdtemp())
    location = root / "fills.csv"

    # Dotted text through the grammar, or the parts as they are.
    assert MediaTable("lake.eu.fills", location).path == ("lake", "eu", "fills")
    assert MediaTable('lake."eu west".fills', location).path == ("lake", "eu west", "fills")
    assert MediaTable("lake.`eu west`.fills", location).path == ("lake", "eu west", "fills")
    assert MediaTable('"a.b".c', location).path == ("a.b", "c")
    assert MediaTable(["lake", "eu west", "fills"], location) == MediaTable('lake."eu west".fills', location)
    assert MediaTable(["a.b", "c"], location).path == ("a.b", "c")
    assert str(MediaTable(["lake", "eu west"], location)) == 'lake."eu west"'

    for text in ("", "'file:///lake/trades.parquet'", "lake.eu with (media_type = 'text/csv')"):
        try:
            MediaTable(text, location)
        except ValueError as error:
            assert "$.path" in str(error)
        else:
            raise AssertionError(text)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    const location = path.join(root, 'fills.csv')

    // Dotted text through the grammar, or the parts as they are.
    assert.deepEqual(warehouse.Table.media('lake.eu.fills', location).path, ['lake', 'eu', 'fills'])
    assert.deepEqual(warehouse.Table.media('lake."eu west".fills', location).path, ['lake', 'eu west', 'fills'])
    assert.deepEqual(warehouse.Table.media('lake.`eu west`.fills', location).path, ['lake', 'eu west', 'fills'])
    assert.deepEqual(warehouse.Table.media('"a.b".c', location).path, ['a.b', 'c'])
    assert.ok(warehouse.Table.media(['lake', 'eu west', 'fills'], location).equals(warehouse.Table.media('lake."eu west".fills', location)))
    assert.deepEqual(warehouse.Table.media(['a.b', 'c'], location).path, ['a.b', 'c'])
    assert.equal(String(warehouse.Table.media(['lake', 'eu west'], location)), 'lake."eu west"')

    for (const text of ['', "'file:///lake/trades.parquet'", "lake.eu with (media_type = 'text/csv')"]) {
      assert.throws(() => warehouse.Table.media(text, location), /\$\.path/)
    }
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Folders

`FolderCatalog` reads a container as a catalog and `FolderNamespace` a folder as a namespace; both list the store when asked, cache nothing but the container handle, and create nothing. The rules, parametrized by how many namespace `levels` sit under the catalog - one by default, so a path reads `catalog.schema.table`; zero under a standalone namespace:

- a folder at a depth under the levels is a namespace; deeper, it is a table read as the rows beneath it (`FolderLayout::Folder`, storage `directory`);
- a folder laid out as a table format - the store answers `IOKind::Table`, or one listing of its `metadata/` shows a version hint or a metadata document, never a read - is a table at any depth (`FolderLayout::Format`, storage `table`);
- a leaf is a table when a record medium of this build reads its media type, which its name states with no read (`FolderLayout::Leaf`, storage its media type); any other leaf is skipped, and so is a private entry - a dot-prefixed name - at every level;
- a table is named by its file name less every extension a media type claims (`trades.arrows.gz` is `trades`, `2024.report.parquet` is `2024.report`), a folder by its name, URI escapes decoded exactly once, a ZIP member by its last segment;
- two entries of one name at one level are both listed, and asking for the name is a conflict.

`Catalog::from_url` is the door a plan or a binding opens a catalog by: the `type` property decides first - `memory`, or `folder` - and otherwise every location a byte backend holds is a folder catalog over the container it names; the catalog is called what the `name` property says, else the location's last segment, and every property travels on to every handle under it.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::{Catalog, CatalogValue, FolderCatalog, IOMedia, NamespaceValue, ObjectValue, Properties, TableValue, Url};

    let root = std::env::temp_dir().join(format!("yggdryl-docs-warehouse-folders-{}", std::process::id()));
    for leaf in ["trades.csv", "README.md", "eu/fills.csv", "eu/quotes.csv", "eu/quotes.arrows", "eu/lake/part-0.csv", "eu/.staging/draft.csv"] {
        let path = root.join(leaf);
        std::fs::create_dir_all(path.parent().expect("a parent"))?;
        std::fs::write(&path, "symbol,price\nAAPL,187.5\n")?;
    }
    std::fs::create_dir_all(root.join("warm/metadata"))?;
    std::fs::write(root.join("warm/metadata/version-hint.text"), "1")?;
    let names = |names: yggdryl::Names| names.map(|name| name.map(|name| name.to_string())).collect::<yggdryl::Result<Vec<_>>>();

    // One level by default: `eu` is a namespace, `warm` a table format, the
    // markdown no table, and the folder under `eu` a table of the rows beneath it.
    let market = Catalog::from(FolderCatalog::bound("market", Holder::folder(&root)?));
    assert_eq!(market.namespace_levels(), Some(1));
    assert_eq!(names(market.namespaces().iter())?, ["eu"]);
    assert_eq!(names(market.tables().iter())?, ["trades", "warm"]);
    assert_eq!(market.table("warm")?.storage(), "table");
    assert_eq!(market.table("eu.lake")?.storage(), "directory");
    assert_eq!(market.table("eu.lake")?.row_size()?, 1);

    // Names come off their extensions; a private entry is no table; two leaves
    // of one name are both listed and conflict when asked for.
    assert!(market.table(["trades.csv"]).unwrap_err().is_absent());
    assert!(market.table(["eu", ".staging"]).unwrap_err().is_absent());
    assert_eq!(names(market.namespace("eu")?.tables().iter())?, ["fills", "lake", "quotes", "quotes"]);
    assert!(market.table("eu.quotes").unwrap_err().is_conflict());

    // No level: every folder under the catalog is a table.
    let flat = Catalog::from(FolderCatalog::bound("market", Holder::folder(&root)?).with_levels(0));
    assert!(flat.namespaces().is_empty()?);
    assert_eq!(flat.table("eu")?.storage(), "directory");

    // A folder catalog lists its store and creates nothing.
    let error = market.create_namespace("asia", &Properties::new()).unwrap_err();
    assert_eq!(error.to_string(), "filesystem \"FolderCatalog\" does not support creating a namespace");

    // From a URL: the name from the properties or the last segment, the rest travelling on.
    let url = Url::from_path(&root)?;
    let properties = Properties::new().with_property("name", "market").with_property("region", "eu-west-1");
    let catalog = Catalog::from_url(&url, &properties)?;
    assert!(matches!(catalog, Catalog::Folder(_)));
    assert_eq!(catalog.name(), "market");
    assert_eq!(catalog.table("eu.fills")?.properties()?.get("region"), Some("eu-west-1"));
    assert!(Catalog::from_url(&url, &Properties::new())?.name().starts_with("yggdryl-docs-warehouse-folders"));
    let error = Catalog::from_url(&url, &Properties::new().with_property("type", "rest")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid record value at $.with.type: expected `memory`, `folder` or `hadoop`, got `rest`; this build has no catalog of that type",
    );
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.warehouse import Catalog, FolderCatalog, MemoryCatalog

    root = pathlib.Path(tempfile.mkdtemp())
    for leaf in ("trades.csv", "README.md", "eu/fills.csv", "eu/quotes.csv", "eu/quotes.arrows", "eu/lake/part-0.csv", "eu/.staging/draft.csv"):
        (root / leaf).parent.mkdir(parents=True, exist_ok=True)
        (root / leaf).write_text("symbol,price\nAAPL,187.5\n")
    (root / "warm" / "metadata").mkdir(parents=True)
    (root / "warm" / "metadata" / "version-hint.text").write_text("1")

    # One level by default: `eu` is a namespace, `warm` a table format, the
    # markdown no table, and the folder under `eu` a table of the rows beneath it.
    market = FolderCatalog("market", root)
    assert market.namespace_levels == 1
    assert list(market.namespaces) == ["eu"]
    assert list(market.tables) == ["trades", "warm"]
    assert market.tables["warm"].storage == "table"
    assert market.tables["eu.lake"].storage == "directory"
    assert market.tables["eu.lake"].row_size() == 1

    # Names come off their extensions; a private entry is no table; two leaves
    # of one name are both listed and conflict when asked for.
    assert market.tables.get('"trades.csv"') is None
    assert list(market.namespaces["eu"].tables) == ["fills", "lake", "quotes", "quotes"]
    try:
        market.tables["eu.quotes"]
    except ValueError as error:
        assert "got an existing several leaves of that name" in str(error)

    # No level: every folder under the catalog is a table.
    flat = FolderCatalog("market", root, levels=0)
    assert len(flat.namespaces) == 0
    assert flat.tables["eu"].storage == "directory"

    # A folder catalog lists its store and creates nothing.
    try:
        market.namespaces.create("asia")
    except ValueError as error:
        assert str(error) == 'filesystem "FolderCatalog" does not support creating a namespace'

    # From a URL: the name from the properties or the last segment, the rest travelling on.
    catalog = Catalog.from_url(root, name="market", region="eu-west-1")
    assert type(catalog) is FolderCatalog and catalog.name == "market"
    assert catalog.tables["eu.fills"].properties["region"] == "eu-west-1"
    assert Catalog.from_url(root).name == root.name
    assert type(Catalog.from_url(root, type="memory", name="scratch")) is MemoryCatalog
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    for (const leaf of ['trades.csv', 'README.md', 'eu/fills.csv', 'eu/quotes.csv', 'eu/quotes.arrows', 'eu/lake/part-0.csv', 'eu/.staging/draft.csv']) {
      fs.mkdirSync(path.dirname(path.join(root, leaf)), { recursive: true })
      fs.writeFileSync(path.join(root, leaf), 'symbol,price\nAAPL,187.5\n')
    }
    fs.mkdirSync(path.join(root, 'warm', 'metadata'), { recursive: true })
    fs.writeFileSync(path.join(root, 'warm', 'metadata', 'version-hint.text'), '1')

    // One level by default: `eu` is a namespace, `warm` a table format, the
    // markdown no table, and the folder under `eu` a table of the rows beneath it.
    const market = warehouse.Catalog.folder('market', root)
    assert.equal(market.namespaceLevels, 1)
    assert.deepEqual([...market.namespaces().keys()], ['eu'])
    assert.deepEqual([...market.tables().keys()], ['trades', 'warm'])
    assert.equal(market.table('warm').storage, 'table')
    assert.equal(market.table('eu.lake').storage, 'directory')
    assert.equal(IOBase.from(market.table('eu.lake')).rowSize(), 1)

    // Names come off their extensions; a private entry is no table; two leaves
    // of one name are both listed and conflict when asked for.
    assert.equal(market.tables().has('"trades.csv"'), false)
    assert.deepEqual([...market.namespace('eu').tables().keys()], ['fills', 'lake', 'quotes', 'quotes'])
    assert.throws(() => market.table('eu.quotes'), /got an existing several leaves of that name/)

    // No level: every folder under the catalog is a table.
    const flat = warehouse.Catalog.folder('market', root, { levels: 0 })
    assert.equal(flat.namespaces().size, 0)
    assert.equal(flat.table('eu').storage, 'directory')

    // A folder catalog lists its store and creates nothing.
    assert.throws(() => market.createNamespace('asia'), /"FolderCatalog" does not support creating a namespace/)

    // From a URL: the name from the properties or the last segment, the rest travelling on.
    const catalog = warehouse.Catalog.fromUrl(root, { name: 'market', region: 'eu-west-1' })
    assert.equal(catalog.implementation, 'FolderCatalog')
    assert.equal(catalog.name, 'market')
    assert.equal(catalog.table('eu.fills').properties.region, 'eu-west-1')
    assert.equal(warehouse.Catalog.fromUrl(root).name, path.basename(root))
    assert.equal(warehouse.Catalog.fromUrl(root, { type: 'memory', name: 'scratch' }).implementation, 'MemoryCatalog')
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Memory

`MemoryCatalog` and `MemoryNamespace` keep registered objects of any implementation in registration order, with no storage behind them: what a `Warehouse` builds along a registered path. A catalog registers by name; a namespace or a table registers at its path, the memory levels along it created as needed, and only memory levels extend - a folder catalog lists its own store, so registering under one is refused by name. `replace` swaps the object of a name and answers the one it replaced; `unregister` removes one and answers it.

=== "Rust"

    ```rust
    use yggdryl::{Catalog, IOKind, MediaTable, MemoryCatalog, Namespace, ObjectValue, Url, Warehouse};

    let table = |path: &str, url: &str| -> yggdryl::Result<MediaTable> { MediaTable::new(path, Url::from_str(url)?) };
    let mut warehouse = Warehouse::new();
    warehouse.register(table("lake.eu.trades", "file:///lake/eu/trades.csv")?)?;

    // The catalog and the namespace along the path were built.
    assert!(matches!(warehouse.catalog("lake")?, Catalog::Memory(_)));
    assert!(matches!(warehouse.namespace("lake.eu")?, Namespace::Memory(_)));
    warehouse.register(table("lake.eu.fills", "file:///lake/eu/fills.csv")?)?;
    let names: Vec<String> = warehouse
        .namespace("lake.eu")?
        .tables()
        .iter()
        .map(|name| name.map(|name| name.to_string()))
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(names, ["trades", "fills"], "registration order");

    // A taken name conflicts; `replace` swaps and answers what was there.
    let error = warehouse.register(table("lake.eu.trades", "file:///elsewhere.csv")?).unwrap_err();
    assert_eq!(error.to_string(), "expected to create a table at \"lake.eu.trades\", got an existing table");
    let replaced = warehouse.replace(table("lake.eu.trades", "file:///lake/eu/trades.parquet")?)?;
    assert_eq!(replaced.and_then(|old| old.url().map(ToString::to_string)).as_deref(), Some("file:///lake/eu/trades.csv"));
    assert_eq!(warehouse.unregister("lake.eu.fills")?.kind(), IOKind::Table);
    assert!(warehouse.table("lake.eu.fills").unwrap_err().is_absent());

    // A catalog registers by name, described or not.
    warehouse.register(MemoryCatalog::new("scratch").with_description("ad hoc"))?;
    assert_eq!(warehouse.get("scratch")?.description(), Some("ad hoc"));
    assert_eq!(warehouse.catalogs().len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import Url
    from yggdryl.warehouse import MediaTable, MemoryCatalog, MemoryNamespace, Warehouse

    root = pathlib.Path(tempfile.mkdtemp())
    warehouse = Warehouse()
    warehouse.register(MediaTable("lake.eu.trades", root / "trades.csv"))

    # The catalog and the namespace along the path were built.
    assert type(warehouse.catalog("lake")) is MemoryCatalog
    assert type(warehouse.namespace("lake.eu")) is MemoryNamespace
    warehouse.register(MediaTable("lake.eu.fills", root / "fills.csv"))
    assert list(warehouse.namespace("lake.eu").tables) == ["trades", "fills"]  # registration order

    # A taken name conflicts; `replace` swaps and answers what was there.
    try:
        warehouse.register(MediaTable("lake.eu.trades", root / "elsewhere.csv"))
    except ValueError as error:
        assert str(error) == 'expected to create a table at "lake.eu.trades", got an existing table'
    replaced = warehouse.replace(MediaTable("lake.eu.trades", root / "trades.parquet"))
    assert replaced.url == Url.from_path(root / "trades.csv")
    assert str(warehouse.unregister("lake.eu.fills")) == "lake.eu.fills"
    assert list(warehouse.namespace("lake.eu").tables) == ["trades"]

    # A catalog registers by name, described or not.
    warehouse.register(MemoryCatalog("scratch", description="ad hoc"))
    assert warehouse.get("scratch").description == "ad hoc"
    assert [catalog.name for catalog in warehouse.catalogs] == ["lake", "scratch"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Url, warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    const registry = new warehouse.Warehouse()
    registry.register(warehouse.Table.media('lake.eu.trades', path.join(root, 'trades.csv')))

    // The catalog and the namespace along the path were built.
    assert.equal(registry.catalog('lake').implementation, 'MemoryCatalog')
    assert.equal(registry.namespace('lake.eu').implementation, 'MemoryNamespace')
    registry.register(warehouse.Table.media('lake.eu.fills', path.join(root, 'fills.csv')))
    assert.deepEqual([...registry.namespace('lake.eu').tables().keys()], ['trades', 'fills']) // registration order

    // A taken name conflicts; `replace` swaps and answers what was there.
    assert.throws(
      () => registry.register(warehouse.Table.media('lake.eu.trades', path.join(root, 'elsewhere.csv'))),
      /expected to create a table at "lake.eu.trades", got an existing table/,
    )
    const replaced = registry.replace(warehouse.Table.media('lake.eu.trades', path.join(root, 'trades.parquet')))
    assert.equal(replaced.url.toString(), Url.fromPath(path.join(root, 'trades.csv')).toString())
    assert.equal(String(registry.unregister('lake.eu.fills')), 'lake.eu.fills')
    assert.deepEqual([...registry.namespace('lake.eu').tables().keys()], ['trades'])

    // A catalog registers by name, described or not.
    registry.register(warehouse.Catalog.memory('scratch', { description: 'ad hoc' }))
    assert.equal(registry.get('scratch').description, 'ad hoc')
    assert.deepEqual(registry.catalogs.map((catalog) => catalog.name), ['lake', 'scratch'])
    fs.rmSync(root, { recursive: true, force: true })
    ```

## The system warehouse

`SystemWarehouse` is the process's one `Warehouse`, which a plan's `from catalog.namespace.table` resolves against when it is given no other - `Plan::execute` reads it, `execute_in` names another ([Sources](../expression/plans.md#sources)). It starts with the memory catalog `local`, holding the folder namespaces `temporary`, `home` and `config` over the platform's temporary directory, the user's home and its `.config` - a root that cannot be resolved is left out. Every door takes the lock for its own call and nothing re-enters it, so it is the same verbs as static functions.

=== "Rust"

    ```rust
    use yggdryl::{CatalogValue, IOKind, MediaTable, ObjectValue, SystemWarehouse, TableValue, Url};

    let local = SystemWarehouse::catalog("local")?;
    assert_eq!(local.kind(), IOKind::Catalog);
    assert_eq!(local.namespace_levels(), None);
    assert_eq!(SystemWarehouse::get("local.temporary")?.kind(), IOKind::Namespace);

    // Registration is the same verb, under the lock for the call; the name is
    // the process's, because the registry is.
    let catalog = format!("docs_page_{}", std::process::id());
    let path = [catalog.as_str(), "raw", "trades"];
    SystemWarehouse::register(MediaTable::new(path, Url::from_str("file:///lake/raw/trades.csv")?)?)?;
    assert_eq!(SystemWarehouse::table(path)?.storage(), "text/csv");
    assert_eq!(SystemWarehouse::get([catalog.as_str(), "raw"])?.kind(), IOKind::Namespace);
    assert_eq!(SystemWarehouse::unregister([catalog.as_str()])?.kind(), IOKind::Catalog);
    assert!(SystemWarehouse::catalog(&catalog).unwrap_err().is_absent());
    ```

=== "Python"

    ```python
    import os
    import pathlib
    import tempfile

    from yggdryl.warehouse import FolderNamespace, MediaTable, MemoryCatalog, SystemWarehouse

    local = SystemWarehouse.catalog("local")
    assert type(local) is MemoryCatalog
    assert type(SystemWarehouse.namespace("local.temporary")) is FolderNamespace
    assert "local" in [catalog.name for catalog in SystemWarehouse.catalogs()]

    # Registration is the same verb, under the lock for the call; the name is
    # the process's, because the registry is.
    root = pathlib.Path(tempfile.mkdtemp())
    (root / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")
    catalog = f"docs_page_{os.getpid()}"
    SystemWarehouse.register(MediaTable(f"{catalog}.raw.trades", root / "trades.csv"))
    try:
        assert SystemWarehouse.table(f"{catalog}.raw.trades").row_size() == 1
        assert SystemWarehouse.get([catalog, "raw"]).kind() == "namespace"
    finally:
        assert str(SystemWarehouse.unregister(catalog)) == catalog
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, warehouse } = require('yggdryl')

    const local = warehouse.SystemWarehouse.catalog('local')
    assert.equal(local.implementation, 'MemoryCatalog')
    assert.equal(warehouse.SystemWarehouse.namespace('local.temporary').implementation, 'FolderNamespace')
    assert.ok(warehouse.SystemWarehouse.catalogs().some((catalog) => catalog.name === 'local'))

    // Registration is the same verb, under the lock for the call; the name is
    // the process's, because the registry is.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    fs.writeFileSync(path.join(root, 'trades.csv'), 'symbol,price\nAAPL,187.5\n')
    const catalog = `docs_page_${process.pid}`
    warehouse.SystemWarehouse.register(warehouse.Table.media([catalog, 'raw', 'trades'], path.join(root, 'trades.csv')))
    try {
      assert.equal(IOBase.from(warehouse.SystemWarehouse.table([catalog, 'raw', 'trades'])).rowSize(), 1)
      assert.equal(warehouse.SystemWarehouse.get([catalog, 'raw']).kind, 'namespace')
    } finally {
      assert.equal(String(warehouse.SystemWarehouse.unregister([catalog])), catalog)
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

## Objects as handles

Every object is a [handle](../holder/index.md#handles): `Object::into_holder` and `Holder::from` hold it as `Holder::Catalog`, `Holder::Namespace` or `Holder::Table`, Python's `Catalog`, `Namespace` and `Table` are `IOBase` subclasses, and JavaScript's `IOBase.from(object)` holds one. A catalog or a namespace is a container: `ls` yields its children as handles, recursively when asked, `child_by_path("a/b")` resolves parts, and it holds no bytes - every byte verb is `NotAtomic` and every record verb names a table under it. A table is its rows: every record verb reaches them, and its `kind` is its storage's - `file` for a leaf, `directory` for a folder, `table` for a table format.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::{Catalog, Error, FolderCatalog, IOBase, IOKind, IOMedia, MediaTable, Object, Url};

    let root = std::env::temp_dir().join(format!("yggdryl-docs-warehouse-handles-{}", std::process::id()));
    std::fs::create_dir_all(root.join("eu"))?;
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,187.5\n")?;

    let held = Holder::from(Catalog::from(FolderCatalog::bound("market", Holder::folder(&root)?)));
    assert!(matches!(held, Holder::Catalog(_)));
    assert_eq!(held.kind(), IOKind::Catalog);
    assert!(held.is_container() && !held.is_atomic() && !held.is_tabular());

    // The children as handles, and a path of parts resolved below.
    let children: Vec<IOKind> = held.ls(false, false).map(|child| child.map(|child| child.kind())).collect::<yggdryl::Result<_>>()?;
    assert_eq!(children, [IOKind::Namespace]);
    assert_eq!(held.ls(true, false).count(), 2, "a recursive listing descends the namespace");
    let trades = held.child_by_path("eu/trades")?;
    assert!(matches!(trades, Holder::Table(_)));
    assert_eq!(trades.kind(), IOKind::File, "a leaf table answers its storage's kind");
    assert!(trades.is_tabular());
    assert_eq!(trades.row_size()?, 1);

    // A container object holds no bytes and is no table.
    assert!(matches!(held.read_all_bytes(), Err(Error::NotAtomic { .. })));
    let error = held.record_options().unwrap_err();
    assert_eq!(error.to_string(), "invalid record value at $.market: expected a table, got the catalog `market`; name a table under it");

    // Any object is a handle, a registered table included.
    let object = Object::from(MediaTable::new("lake.raw.trades", Url::from_path(root.join("eu/trades.csv"))?)?);
    assert!(matches!(object.into_holder(), Holder::Table(_)));
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.warehouse import FolderCatalog, MediaTable, Namespace

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "eu").mkdir()
    (root / "eu" / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")

    market = FolderCatalog("market", root)
    assert isinstance(market, IOBase)
    assert market.kind() == "catalog" and market.is_dir()

    # The children as handles, and a path of parts resolved below.
    assert all(isinstance(child, Namespace) for child in market.iterdir())
    assert sorted(str(child) for child in market.ls(recursive=True)) == ["market.eu", "market.eu.trades"]
    eu = market / "eu"
    assert str(eu) == "market.eu" and eu.kind() == "namespace"
    trades = market.joinpath("eu/trades")
    assert type(trades) is MediaTable
    assert trades.kind() == "file"  # a leaf table answers its storage's kind
    assert trades.row_size() == 1

    # A container object holds no bytes and is no table.
    try:
        eu.read_bytes()
    except ValueError as error:
        assert "got a namespace" in str(error)
    try:
        eu.read_arrow_reader()
    except ValueError as error:
        assert "name a table under it" in str(error)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, warehouse } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-warehouse-'))
    fs.mkdirSync(path.join(root, 'eu'))
    fs.writeFileSync(path.join(root, 'eu', 'trades.csv'), 'symbol,price\nAAPL,187.5\n')

    const market = warehouse.Catalog.folder('market', root)
    const held = IOBase.from(market)
    assert.equal(held.kind(), 'catalog')
    assert.equal(held.isDir(), true)

    // The children as handles, and a path of parts resolved below.
    assert.deepEqual([...held.ls()].map((child) => child.kind()), ['namespace'])
    assert.equal([...held.ls(true)].length, 2)
    const trades = held.joinpath('eu/trades')
    assert.equal(trades.kind(), 'file') // a leaf table answers its storage's kind
    assert.equal(trades.rowSize(), 1)

    // A container object holds no bytes and is no table.
    assert.throws(() => held.readBytes(), /got a catalog/)
    assert.throws(() => held.readArrowReader(), /name a table under it/)
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- The empty text is the root; a path given where an object is named - a registration, a table, a namespace - must have the parts its kind needs, a namespace at least two, a table at least one, and is refused at `$.path` otherwise.
- Absence names the path as the grammar spells it, a quoted part's quotes escaped inside the message's own: `expected a table at "lake.eu.trades", got nothing`, `expected a table at "lake.\"eu west\"", got nothing`; a namespace or a catalog where a table is asked for is absence, and a table met before the last part is the absence of the namespace asked for below it.
- A memory level names an `object` in its absence, a folder level a `table`, whichever view asked.
- A taken name is a conflict naming what is there: `expected to create a table at "lake.eu", got an existing namespace`; two store entries of one name are both listed and asking for the name conflicts.
- Registering under a folder catalog, a folder namespace or a table is refused by name: ``filesystem "FolderCatalog `market`" does not support registering under an object that lists its own store``.
- Memory and folder objects create nothing - an [Iceberg catalog](../media/iceberg.md#catalog) creates namespaces and tables, through existing namespaces only: `filesystem "MemoryCatalog" does not support creating a table`; `open_or_create`, the append and overwrite helpers refuse the same way for a name nothing holds, and `open_or_create` opens an existing table as it is - `field` describes only the table the call would create.
- `update_properties` persists only where the store keeps something; the memory, folder and media implementations keep nothing and refuse by name, an Iceberg catalog and namespace keep theirs in their own document, and an Iceberg table's ride its metadata, written through `commit_metadata_changes`. Stated properties - credentials included - are never written into a document.
- A clone starts unresolved and rebuilds its handle from its location; a clone of an object bound to a handle with no location - an in-memory `Buffer` - refuses its next verb naming the path: ``expected a located handle to rebuild `memory.trades` from, got one with no URL``.
- A table laid out as a table format is listed in every build and read only under `iceberg`, where a folder catalog answers it as `Table::Iceberg`; without it, its record verbs are refused at `$.encoding` naming the feature.
- A catalog's or a namespace's `clear` and `remove` are refused: an object is unregistered or its store changed, never emptied through its handle; a table's reach its storage.
- `Catalog::from_url` refuses `rest` and `xmla` - and `hadoop` without the `iceberg` feature - as types this build has no catalog for, any other unknown type at `$.with.type`, an `s3tables://` location, and a URL with no segment to name the catalog by when no `name` property is stated.
- `Names::len` and the views' `len` drain a listing; `is_empty` costs the listing up to the first entry of that kind.
- A name in a view that would have to be quoted - `eu west` - is quoted in dotted text or passed as one part; JavaScript's `values()` and `entries()` open each name as one part for that reason.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test warehouse
    cargo test -p yggdryl --test iobase_calls warehouse
    cargo bench -p yggdryl --bench holder -- warehouse
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_warehouse.py
    python/.venv/bin/python python/benchmarks/warehouse.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/warehouse.test.js
    npm run --prefix node bench:warehouse
    ```

## Performance

The cost of the warehouse is stated in store calls and pinned in `rust/tests/iobase_calls.rs` (`mod warehouse`), on a counted Arrow filesystem laid out as a catalog - a root leaf, a namespace of a leaf and a folder table, and an empty namespace - where `list` is one directory listing and `file_info` the backend's answer to a listed leaf's role:

| Operation | Calls |
| --- | --- |
| describing, registering and resolving a registered object; `properties_for` | none |
| a folder catalog's `children`, drained | one listing, then per folder entry its role and one listing of its `metadata/`: `file_info=1 list=3` for two folders and a leaf; `file_info=1 list=2` under a namespace of a leaf and a folder |
| the first child | one listing and one entry classified: `list=2` |
| a path | one listing per level descended, one listing of `metadata/` per folder passed through, the matched leaf's role once: `file_info=1 list=1` for `trades`, `file_info=1 list=3` for `eu.fills`, `list=3` to an absence under `eu` |
| a read through `Holder::Table` | exactly what the same read on the table's own handle costs: `file_info=1 open_input_stream=1`; the schema through a resolved path `file_info=2 list=3 open_input_stream=1`, which is the path and then the schema |
| a read through a clone | the clone rebuilds its handle from the leaf's location on its first verb, one role resolution more than the leaf in hand: `file_info=3 open_input_stream=1` |

The benchmark is `warehouse` in the `holder` target: `resolve/registered` at one and a thousand registered tables - the floor of the abstraction, a walk of the memory levels and the clone of the object answered, no store touched - and `first_table/folder` beside `drain/folder` over a folder of 10, 1 000 and 100 000 CSV leaves, time to the first table against the full drain, as the listing benchmarks measure a folder. No number is stated here until a release run on a named machine writes it; regenerate with:

```bash
cargo bench -p yggdryl --bench holder -- warehouse
python/.venv/bin/python python/benchmarks/warehouse.py --iterations 10000
npm run --prefix node bench:warehouse
```
